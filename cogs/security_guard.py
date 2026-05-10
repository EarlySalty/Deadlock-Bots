import asyncio
import json
import logging
import re
from collections import defaultdict, deque
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any

import discord
from discord.ext import commands

log = logging.getLogger(__name__)


SCAM_DETECTION_SYSTEM_PROMPT = (
    "You are a scam detector for a Discord gaming server. "
    "Decide if the message is financial spam or a scam (earnings promises, investment schemes, "
    "Telegram/contact requests, profit-sharing, referral schemes, or similar). "
    "Reply only with valid JSON, no other text: "
    '{"is_scam": true|false, "confidence": 0.0-1.0, "reason": "max one sentence"}'
)

SCAM_IMAGE_SYSTEM_PROMPT = (
    "You are a scam detector for a Discord gaming server. "
    "Analyze the images and determine if they show financial scam content: "
    "fake investment or trading profit screenshots, fake X/Twitter posts about earnings, "
    "crypto/forex/stock gain screenshots, testimonials about making money, "
    "or any get-rich-quick scheme visuals. "
    "Reply only with valid JSON, no other text: "
    '{"is_scam": true|false, "confidence": 0.0-1.0, "reason": "max one sentence"}'
)

_SCAM_JSON_RE = re.compile(r"\{.*?\}", re.DOTALL)


def _safe_log_value(value: Any) -> str:
    """Sanitize values before logging to prevent log injection attacks."""
    text = "" if value is None else str(value)
    return text.replace("\r", "\\r").replace("\n", "\\n")


# ---------------- Static Config (edit here, no ENV needed) ----------------
SECURITY_CONFIG: dict[str, object] = {
    # ID eines Textkanals, in den Beweise/Embeds gepostet werden.
    "REVIEW_CHANNEL_ID": 0,
    # ID des Mod-Kanals fuer Ban-Logs, Appeals und Unban-Button.
    "MOD_CHANNEL_ID": 1315684135175716978,
    # Aktion bei Treffer: "ban" oder "timeout".
    "PUNISHMENT": "ban",
    # Beobachtungsfenster fuer Burst-Detection (Sekunden).
    "WINDOW_SECONDS": 3600,  # 1h
    # Schwellen fuer Mehrkanal-Spam (z.B. 3 Nachrichten in 3 Channels).
    "CHANNEL_THRESHOLD": 3,
    "MESSAGE_THRESHOLD": 3,
    # Account muss juenger als X Stunden sein. Join-Alter spielt keine Rolle mehr.
    "ACCOUNT_MAX_AGE_HOURS": 720,  # 30 Tage
    # Ab diesem Account-Alter gilt ein Account als "etabliert" → Timeout statt Ban.
    "ESTABLISHED_ACCOUNT_MIN_AGE_HOURS": 720,  # 30 Tage
    "ESTABLISHED_MIN_JOIN_HOURS": 24,
    # AI-Scam-Erkennung fuer Einzelnachrichten mit Keyword-Treffer.
    "AI_SCAM_PROVIDER": "openai",
    "AI_SCAM_CONFIDENCE": 0.78,
    # AI-Bild-Scam-Erkennung (Multimodal, nur MiniMax unterstuetzt).
    "AI_IMAGE_PROVIDER": "minimax",
    "AI_IMAGE_CONFIDENCE": 0.75,
    # Mindestanzahl Channels mit Bildern um Bild-Scam-Check auszuloesen.
    "IMAGE_CHANNEL_THRESHOLD": 2,
    # Dauer des Timeouts in Minuten (Default: 24h).
    "TIMEOUT_MINUTES": 1440,
    # Timeout fuer Buttons (Sekunden).
    "VIEW_TIMEOUT_SECONDS": 86400,  # 24h
    # Appeal Textlaenge.
    "APPEAL_MIN_CHARS": 4,
    "APPEAL_MAX_CHARS": 800,
    # Attachment-Handling fuer Beweissicherung.
    "ATTACHMENT_FORWARD_LIMIT": 4,
    "ATTACHMENT_MAX_BYTES": 7_000_000,
    # Optional: Nur auf bestimmten Guilds aktivieren (leer = alle).
    "GUILD_IDS": [],
    # Schlagwort-Netz fuer typische Scam-Messages.
    "KEYWORDS": [
        "telegram",
        "dm me",
        "pm me",
        "friend request",
        "how to start earning",
        "100k",
        "usdt",
        "withdrawal",
        "payout",
        "profit",
        "woamax",
        "promo code",
        "bonus",
        "first 10 people",
        "earning $",
    ],
}


@dataclass
class RecentMessage:
    """Lightweight container for tracking a user's recent activity."""

    message: discord.Message
    channel_id: int
    created_at: datetime
    content: str
    attachments: list[discord.Attachment]


@dataclass
class IncidentCase:
    case_id: str
    guild_id: int
    user_id: int
    user_tag: str
    reason: str
    created_at: datetime
    action: str


class AppealModal(discord.ui.Modal):
    def __init__(
        self,
        cog: "SecurityGuard",
        case_id: str,
        *,
        min_chars: int,
        max_chars: int,
    ) -> None:
        super().__init__(title="Appeal")
        self.cog = cog
        self.case_id = case_id
        self.appeal_reason = discord.ui.TextInput(
            label="Appeal reason",
            style=discord.TextStyle.paragraph,
            min_length=min_chars,
            max_length=max_chars,
            placeholder="Explain why this ban should be reviewed.",
        )
        self.add_item(self.appeal_reason)

    async def on_submit(self, interaction: discord.Interaction) -> None:
        await self.cog.handle_appeal_submission(
            interaction,
            self.case_id,
            self.appeal_reason.value.strip(),
        )


class AppealView(discord.ui.View):
    def __init__(self, cog: "SecurityGuard", case_id: str) -> None:
        super().__init__(timeout=cog.view_timeout_seconds)
        self.cog = cog
        self.case_id = case_id

    @discord.ui.button(label="Appeal", style=discord.ButtonStyle.primary)
    async def appeal_button(
        self, interaction: discord.Interaction, button: discord.ui.Button
    ) -> None:
        await interaction.response.send_modal(
            AppealModal(
                self.cog,
                self.case_id,
                min_chars=self.cog.appeal_min_chars,
                max_chars=self.cog.appeal_max_chars,
            )
        )


class UnbanView(discord.ui.View):
    def __init__(self, cog: "SecurityGuard", guild_id: int, user_id: int, case_id: str) -> None:
        super().__init__(timeout=cog.view_timeout_seconds)
        self.cog = cog
        self.guild_id = guild_id
        self.user_id = user_id
        self.case_id = case_id

    @discord.ui.button(label="Unban", style=discord.ButtonStyle.danger)
    async def unban_button(
        self, interaction: discord.Interaction, button: discord.ui.Button
    ) -> None:
        await self.cog.handle_unban_request(
            interaction, self.guild_id, self.user_id, self.case_id, button, self
        )


class ScamBanView(discord.ui.View):
    def __init__(self, cog: "SecurityGuard", guild_id: int, user_id: int, case_id: str) -> None:
        super().__init__(timeout=cog.view_timeout_seconds)
        self.cog = cog
        self.guild_id = guild_id
        self.user_id = user_id
        self.case_id = case_id

    def _check_perms(self, interaction: discord.Interaction) -> bool:
        perms = getattr(interaction.user, "guild_permissions", None)
        return bool(perms and (perms.ban_members or perms.administrator))

    def _disable_all(self) -> None:
        for item in self.children:
            item.disabled = True  # type: ignore[attr-defined]

    @discord.ui.button(label="Ban", style=discord.ButtonStyle.danger)
    async def ban_button(
        self, interaction: discord.Interaction, button: discord.ui.Button
    ) -> None:
        if not self._check_perms(interaction):
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
            return
        guild = self.cog.bot.get_guild(self.guild_id)
        if guild is None:
            await interaction.response.send_message("Server nicht gefunden.", ephemeral=True)
            return
        try:
            await guild.ban(
                discord.Object(id=self.user_id),
                reason=f"[SecurityGuard][case:{self.case_id}] Mod-escalation: scam (established account)",
            )
            self._disable_all()
            try:
                await interaction.message.edit(view=self)
            except discord.HTTPException:
                pass
            await interaction.response.send_message("Gebannt.", ephemeral=True)
        except discord.Forbidden:
            await interaction.response.send_message("Keine Ban-Berechtigung.", ephemeral=True)
        except discord.HTTPException as exc:
            await interaction.response.send_message(f"Ban fehlgeschlagen: {exc}", ephemeral=True)

    @discord.ui.button(label="Timeout aufheben", style=discord.ButtonStyle.secondary)
    async def remove_timeout_button(
        self, interaction: discord.Interaction, button: discord.ui.Button
    ) -> None:
        if not self._check_perms(interaction):
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
            return
        guild = self.cog.bot.get_guild(self.guild_id)
        if guild is None:
            await interaction.response.send_message("Server nicht gefunden.", ephemeral=True)
            return
        try:
            member = guild.get_member(self.user_id) or await guild.fetch_member(self.user_id)
            await member.edit(
                communication_disabled_until=None,
                reason=f"[SecurityGuard][case:{self.case_id}] Mod: false positive, timeout entfernt",
            )
            self._disable_all()
            try:
                await interaction.message.edit(view=self)
            except discord.HTTPException:
                pass
            await interaction.response.send_message("Timeout aufgehoben.", ephemeral=True)
        except discord.NotFound:
            await interaction.response.send_message("Member nicht gefunden.", ephemeral=True)
        except discord.Forbidden:
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
        except discord.HTTPException as exc:
            await interaction.response.send_message(f"Fehlgeschlagen: {exc}", ephemeral=True)


class SecurityGuard(commands.Cog):
    """
    Guards against brand new accounts that shotgun messages across channels.
    - Detects multi-channel bursts from accounts younger than X hours and fresh joins.
    - Deletes the burst, bans or times the member out, and mirrors evidence to a review channel.
    """

    def __init__(self, bot: commands.Bot):
        self.bot = bot

        cfg = SECURITY_CONFIG
        self.review_channel_id = int(cfg.get("REVIEW_CHANNEL_ID", 0) or 0)
        self.mod_channel_id = int(cfg.get("MOD_CHANNEL_ID", 0) or 0)
        self.punishment = str(cfg.get("PUNISHMENT", "ban") or "ban").strip().lower()
        self.window_seconds = int(cfg.get("WINDOW_SECONDS", 120) or 120)
        self.channel_threshold = max(2, int(cfg.get("CHANNEL_THRESHOLD", 3) or 3))
        self.message_threshold = max(2, int(cfg.get("MESSAGE_THRESHOLD", 3) or 3))
        self.account_max_age_hours = max(1, int(cfg.get("ACCOUNT_MAX_AGE_HOURS", 720) or 720))
        self.established_account_min_age_hours = max(
            1, int(cfg.get("ESTABLISHED_ACCOUNT_MIN_AGE_HOURS", 168) or 168)
        )
        self.established_min_join_hours = max(
            0, int(cfg.get("ESTABLISHED_MIN_JOIN_HOURS", 24) or 24)
        )
        self.ai_scam_provider = str(cfg.get("AI_SCAM_PROVIDER", "openai") or "openai").lower()
        self.ai_scam_confidence = max(
            0.5, min(1.0, float(cfg.get("AI_SCAM_CONFIDENCE", 0.78) or 0.78))
        )
        self.ai_image_provider = str(cfg.get("AI_IMAGE_PROVIDER", "minimax") or "minimax").lower()
        self.ai_image_confidence = max(
            0.5, min(1.0, float(cfg.get("AI_IMAGE_CONFIDENCE", 0.75) or 0.75))
        )
        self.image_channel_threshold = max(2, int(cfg.get("IMAGE_CHANNEL_THRESHOLD", 2) or 2))
        self.timeout_minutes = max(5, int(cfg.get("TIMEOUT_MINUTES", 1440) or 1440))
        self.view_timeout_seconds = max(60, int(cfg.get("VIEW_TIMEOUT_SECONDS", 86400) or 86400))
        self.appeal_min_chars = max(1, int(cfg.get("APPEAL_MIN_CHARS", 4) or 4))
        self.appeal_max_chars = max(
            self.appeal_min_chars, int(cfg.get("APPEAL_MAX_CHARS", 800) or 800)
        )
        self.attachment_forward_limit = max(0, int(cfg.get("ATTACHMENT_FORWARD_LIMIT", 4) or 4))
        self.attachment_max_bytes = max(
            1_000_000, int(cfg.get("ATTACHMENT_MAX_BYTES", 7_000_000) or 7_000_000)
        )

        if self.punishment not in ("ban", "timeout"):
            self.punishment = "ban"

        raw_guilds = cfg.get("GUILD_IDS", [])
        guild_ids: list[int] = []
        if isinstance(raw_guilds, (list, tuple, set)):
            for item in raw_guilds:
                try:
                    guild_ids.append(int(item))
                except Exception:  # noqa: S112
                    continue
        self.allowed_guild_ids: set[int] = set(guild_ids)

        self._message_history: dict[int, deque[RecentMessage]] = defaultdict(
            lambda: deque(maxlen=20)
        )
        self._active_cases: set[int] = set()
        self.case_cache_limit = 250
        self._cases: dict[str, IncidentCase] = {}
        self._case_order: deque[str] = deque()

        # Simple keyword net for common scam phrasing
        kw = cfg.get("KEYWORDS", [])
        kws: set[str] = set()
        if isinstance(kw, (list, tuple, set)):
            for item in kw:
                if isinstance(item, str):
                    kws.add(item.lower())
        self.suspicious_keywords = kws

    # ---------------- Events ----------------
    @commands.Cog.listener()
    async def on_message(self, message: discord.Message):
        if message.guild is None or message.author.bot:
            return
        if not isinstance(message.author, discord.Member):
            return
        member = message.author

        if self.allowed_guild_ids and message.guild.id not in self.allowed_guild_ids:
            return
        if member.guild_permissions.manage_messages or member.guild_permissions.manage_guild:
            return  # do not police staff

        now = discord.utils.utcnow()
        is_young = self._is_new_account(member, now)  # Account < 30 Tage

        # History fuer ALLE Accounts tracken (noetig fuer Bild-Scam-Detection)
        history = self._message_history[member.id]
        history.append(
            RecentMessage(
                message=message,
                channel_id=message.channel.id,
                created_at=message.created_at or now,
                content=message.content or "",
                attachments=list(message.attachments),
            )
        )
        self._prune_history(member.id, now)
        recent_msgs = list(history)

        # Pfad 1: Mehrkanal-Burst — nur fuer junge Accounts (<30 Tage), regelbasiert
        if is_young and member.id not in self._active_cases:
            triggered, reason, meta = self._should_trigger(member, recent_msgs, now)
            if triggered:
                self._active_cases.add(member.id)
                try:
                    await self._handle_incident(member, recent_msgs, reason, meta)
                finally:
                    self._message_history.pop(member.id, None)
                    self._active_cases.discard(member.id)
                return

        # Pfad 2: Bilder in mehreren Channels → AI-Bild-Scam-Check (alle Accounts)
        if (
            self._is_image_multi_channel(recent_msgs)
            and member.id not in self._active_cases
        ):
            self._active_cases.add(member.id)
            try:
                is_scam, confidence, ai_reason = await self._ai_check_image_scam(recent_msgs)
                if is_scam and confidence >= self.ai_image_confidence:
                    if self._is_established_account(member, now):
                        latest_img_msg = next(
                            (m.message for m in reversed(recent_msgs) if m.attachments), message
                        )
                        await self._handle_scam_proposal(member, latest_img_msg, ai_reason, confidence)
                    else:
                        img_channels = len({m.channel_id for m in recent_msgs if m.attachments})
                        img_count = sum(len(m.attachments) for m in recent_msgs)
                        reason_str = f"Bild-Scam in {img_channels} Channels (conf {confidence:.0%}): {ai_reason}"
                        scam_meta = {
                            "channel_count": img_channels,
                            "message_count": len(recent_msgs),
                            "attachment_count": img_count,
                            "keyword_hit": 0,
                        }
                        await self._handle_incident(member, recent_msgs, reason_str, scam_meta)
                    self._message_history.pop(member.id, None)
            finally:
                self._active_cases.discard(member.id)

        # Pfad 3: Keyword + AI — fuer alle Accounts (jung = Ban, etabliert = Timeout + DM)
        if (
            self._contains_suspicious_text(message.content)
            and member.id not in self._active_cases
        ):
            self._active_cases.add(member.id)
            try:
                is_scam, confidence, ai_reason = await self._ai_check_scam(message.content)
                if is_scam and confidence >= self.ai_scam_confidence:
                    if self._is_established_account(member, now):
                        await self._handle_scam_proposal(member, message, ai_reason, confidence)
                    else:
                        single = RecentMessage(
                            message=message,
                            channel_id=message.channel.id,
                            created_at=message.created_at or now,
                            content=message.content or "",
                            attachments=list(message.attachments),
                        )
                        ai_reason_short = f"AI-Scam (conf {confidence:.0%}): {ai_reason}"
                        scam_meta = {
                            "channel_count": 1,
                            "message_count": 1,
                            "attachment_count": len(message.attachments),
                            "keyword_hit": 1,
                        }
                        await self._handle_incident(member, [single], ai_reason_short, scam_meta)
                    self._message_history.pop(member.id, None)
            finally:
                self._active_cases.discard(member.id)

    # ---------------- Core logic ----------------
    def _is_new_account(self, member: discord.Member, now: datetime) -> bool:
        created = member.created_at
        if not created:
            return False
        age = now - created.replace(tzinfo=UTC)
        return age.total_seconds() <= self.account_max_age_hours * 3600

    def _is_established_account(self, member: discord.Member, now: datetime) -> bool:
        created = member.created_at
        joined = member.joined_at
        if not created or not joined:
            return False
        account_age_h = (now - created.replace(tzinfo=UTC)).total_seconds() / 3600
        join_age_h = (now - joined.replace(tzinfo=UTC)).total_seconds() / 3600
        return (
            account_age_h >= self.established_account_min_age_hours
            and join_age_h >= self.established_min_join_hours
        )

    async def _ai_check_scam(self, content: str) -> tuple[bool, float, str]:
        ai = self.bot.get_cog("AIConnector")
        if ai is None or not hasattr(ai, "generate_text"):
            return False, 0.0, "ai_unavailable"
        try:
            text, _ = await ai.generate_text(
                provider=self.ai_scam_provider,
                prompt=content[:2000],
                system_prompt=SCAM_DETECTION_SYSTEM_PROMPT,
                max_output_tokens=120,
                temperature=0.1,
            )
        except Exception as exc:
            log.warning("AI scam check fehlgeschlagen: %s", exc)
            return False, 0.0, "ai_error"
        if not text:
            return False, 0.0, "no_response"
        match = _SCAM_JSON_RE.search(text)
        if not match:
            return False, 0.0, "parse_error"
        try:
            data = json.loads(match.group(0))
        except json.JSONDecodeError:
            return False, 0.0, "json_error"
        is_scam = bool(data.get("is_scam", False))
        confidence = max(0.0, min(1.0, float(data.get("confidence", 0.0))))
        reason = str(data.get("reason", ""))[:300]
        return is_scam, confidence, reason

    def _is_image_multi_channel(self, msgs: list[RecentMessage]) -> bool:
        channels_with_images = {m.channel_id for m in msgs if m.attachments}
        return len(channels_with_images) >= self.image_channel_threshold

    async def _ai_check_image_scam(self, msgs: list[RecentMessage]) -> tuple[bool, float, str]:
        ai = self.bot.get_cog("AIConnector")
        if ai is None or not hasattr(ai, "generate_multimodal"):
            return False, 0.0, "ai_unavailable"

        image_urls: list[str] = []
        for msg in msgs:
            for att in msg.attachments:
                if (att.content_type or "").lower().startswith("image/"):
                    image_urls.append(att.url)
                if len(image_urls) >= 4:
                    break
            if len(image_urls) >= 4:
                break

        if not image_urls:
            return False, 0.0, "no_images"

        try:
            text, _ = await ai.generate_multimodal(
                provider=self.ai_image_provider,
                prompt="Analyze these images for scam content.",
                images=image_urls,
                system_prompt=SCAM_IMAGE_SYSTEM_PROMPT,
                max_output_tokens=120,
                temperature=0.1,
            )
        except Exception as exc:
            log.warning("AI Bild-Scam-Check fehlgeschlagen: %s", exc)
            return False, 0.0, "ai_error"

        if not text:
            return False, 0.0, "no_response"
        match = _SCAM_JSON_RE.search(text)
        if not match:
            return False, 0.0, "parse_error"
        try:
            data = json.loads(match.group(0))
        except json.JSONDecodeError:
            return False, 0.0, "json_error"

        is_scam = bool(data.get("is_scam", False))
        confidence = max(0.0, min(1.0, float(data.get("confidence", 0.0))))
        reason = str(data.get("reason", ""))[:300]
        return is_scam, confidence, reason

    def _prune_history(self, user_id: int, now: datetime) -> None:
        cutoff = now - timedelta(seconds=self.window_seconds)
        history = self._message_history.get(user_id)
        if not history:
            return
        while history and history[0].created_at < cutoff:
            history.popleft()

    def _make_case_id(self, member: discord.Member, now: datetime) -> str:
        return f"{member.guild.id}-{member.id}-{int(now.timestamp())}"

    def _remember_case(self, record: IncidentCase) -> None:
        self._cases[record.case_id] = record
        self._case_order.append(record.case_id)
        while len(self._case_order) > self.case_cache_limit:
            old_case = self._case_order.popleft()
            self._cases.pop(old_case, None)

    def _contains_suspicious_text(self, text: str) -> bool:
        lower = text.lower()
        return any(key in lower for key in self.suspicious_keywords)

    def _should_trigger(
        self,
        member: discord.Member,
        msgs: list[RecentMessage],
        now: datetime,
    ) -> tuple[bool, str, dict[str, int]]:
        if not msgs:
            return False, "", {}

        unique_channels = {m.channel_id for m in msgs}
        total_msgs = len(msgs)
        attachment_count = sum(1 for m in msgs if m.attachments)
        attachment_channels = {m.channel_id for m in msgs if m.attachments}
        keyword_hit = any(self._contains_suspicious_text(m.content) for m in msgs)

        multi_channel_burst = (
            len(unique_channels) >= self.channel_threshold and total_msgs >= self.message_threshold
        )
        two_channel_sus = (
            len(unique_channels) >= 2 and total_msgs >= 2 and (keyword_hit or attachment_count > 0)
        )
        attachment_multi_channel = len(attachment_channels) >= 2

        if multi_channel_burst or two_channel_sus or attachment_multi_channel:
            reason_bits = []
            if multi_channel_burst:
                reason_bits.append("multi-channel burst")
            if two_channel_sus and not multi_channel_burst:
                reason_bits.append("suspicious content across 2+ channels")
            if attachment_multi_channel and not multi_channel_burst:
                reason_bits.append("attachments across 2+ channels")
            if keyword_hit:
                reason_bits.append("keyword match")
            if attachment_count:
                reason_bits.append(f"{attachment_count} attachment(s)")
            reason = "; ".join(reason_bits) or "burst from new account"
            meta = {
                "channel_count": len(unique_channels),
                "message_count": total_msgs,
                "attachment_count": attachment_count,
                "keyword_hit": int(keyword_hit),
            }
            return True, reason, meta

        return False, "", {}

    async def _handle_incident(
        self,
        member: discord.Member,
        msgs: list[RecentMessage],
        reason: str,
        meta: dict[str, int],
    ) -> None:
        now = discord.utils.utcnow()
        case_id = self._make_case_id(member, now)
        record = IncidentCase(
            case_id=case_id,
            guild_id=member.guild.id,
            user_id=member.id,
            user_tag=str(member),
            reason=reason,
            created_at=now,
            action=self.punishment,
        )
        self._remember_case(record)

        # Copy attachments before deletion
        forwarded_files = await self._collect_attachments(msgs)

        dm_sent = await self._send_user_dm(member, reason, case_id)
        action, action_ok = await self._apply_action(member, reason, case_id)
        deleted = await self._delete_messages(msgs, reason)

        await self._log_incident(
            member,
            msgs,
            reason,
            meta,
            action,
            action_ok,
            deleted,
            forwarded_files,
            case_id,
            dm_sent,
        )

    async def _handle_scam_proposal(
        self,
        member: discord.Member,
        message: discord.Message,
        ai_reason: str,
        confidence: float,
    ) -> None:
        now = discord.utils.utcnow()
        case_id = self._make_case_id(member, now)

        # Nachricht löschen
        try:
            await message.delete(reason=f"[SecurityGuard][case:{case_id}] scam (established account)")
        except discord.HTTPException:
            pass

        # Sofort 24h-Timeout anwenden
        timeout_ok = False
        until = now + timedelta(minutes=self.timeout_minutes)
        try:
            me = member.guild.me or await member.guild.fetch_member(self.bot.user.id)
            if me.guild_permissions.moderate_members:
                await member.edit(
                    communication_disabled_until=until,
                    reason=f"[SecurityGuard][case:{case_id}] AI-Scam, etablierter Account (möglicherweise gehackt)",
                )
                timeout_ok = True
        except discord.HTTPException as exc:
            log.warning("Timeout fuer etablierten Scam-Account %s fehlgeschlagen: %s", member.id, exc)

        # DM an User
        if timeout_ok:
            try:
                dm_embed = discord.Embed(
                    title="Du wurdest vorübergehend stummgeschaltet",
                    color=0xE67E22,
                    timestamp=now,
                )
                dm_embed.add_field(name="Server", value=member.guild.name, inline=False)
                dm_embed.add_field(
                    name="Grund",
                    value="Auf deinem Account wurde eine verdächtige Scam-Nachricht erkannt. "
                          "Falls dein Account gehackt wurde, melde dich bitte beim Mod-Team, "
                          "sobald du ihn zurück hast.",
                    inline=False,
                )
                dm_embed.add_field(name="Dauer", value=f"{self.timeout_minutes // 60} Stunden", inline=True)
                dm_embed.add_field(name="Case ID", value=case_id, inline=True)
                dm_embed.set_footer(text="Wende dich an das Mod-Team, wenn dein Account wieder sicher ist.")
                await member.send(embed=dm_embed)
            except discord.HTTPException:
                pass

        # Mod-Channel: Info-Embed mit Aktions-Buttons
        mod_channel = await self._resolve_mod_channel(member.guild)
        if not mod_channel:
            log.warning("Kein Mod-Channel gesetzt; Scam-Proposal Case %s nur gelogt.", case_id)
            return

        snippet = (message.content or "")[:500].replace("`", "'")
        embed = discord.Embed(
            title="Scam erkannt: etablierter Account — Auto-Timeout",
            color=0xE67E22,
            timestamp=now,
        )
        embed.add_field(name="Member", value=f"{member.mention} ({member.id})", inline=False)
        embed.add_field(name="Case ID", value=case_id, inline=True)
        embed.add_field(name="Account-Alter", value=self._fmt_delta(now, member.created_at), inline=True)
        embed.add_field(name="Server-Mitglied seit", value=self._fmt_delta(now, member.joined_at), inline=True)
        embed.add_field(name="AI Confidence", value=f"{confidence:.0%}", inline=True)
        embed.add_field(name="Timeout", value="24h ✓" if timeout_ok else "fehlgeschlagen ✗", inline=True)
        embed.add_field(name="AI Reason", value=ai_reason or "—", inline=False)
        embed.add_field(
            name="Nachricht",
            value=f"```{snippet}```" if snippet else "(leer)",
            inline=False,
        )
        embed.set_footer(
            text="Nachricht gelöscht + 24h Timeout gesetzt. "
                 "Account möglicherweise gehackt — User wurde per DM informiert."
        )

        view = ScamBanView(self, member.guild.id, member.id, case_id)
        try:
            await mod_channel.send(embed=embed, view=view)
        except discord.HTTPException as exc:
            log.warning("Konnte Scam-Info nicht posten fuer Case %s: %s", case_id, exc)

    async def _send_user_dm(self, member: discord.Member, reason: str, case_id: str) -> bool:
        action_label = "banned" if self.punishment == "ban" else "timed out"
        action_title = "Ban" if self.punishment == "ban" else "Timeout"
        embed = discord.Embed(
            title=f"You were {action_label} by SecurityGuard",
            color=0xE74C3C,
            timestamp=discord.utils.utcnow(),
        )
        embed.add_field(name="Guild", value=member.guild.name, inline=False)
        embed.add_field(name="Reason", value=reason or "auto-detected burst", inline=False)
        embed.add_field(name="Case ID", value=case_id, inline=True)
        footer = (
            "If you believe this is a mistake, use the Appeal button."
            if self.punishment == "ban"
            else f"{action_title} duration: {self.timeout_minutes} minutes. Appeal is still available."
        )
        embed.set_footer(text=footer)
        try:
            await member.send(embed=embed, view=AppealView(self, case_id))
            return True
        except discord.Forbidden:
            return False
        except discord.HTTPException as exc:
            log.warning("Failed to DM member %s: %s", member.id, exc)
            return False

    async def _apply_action(
        self, member: discord.Member, reason: str, case_id: str
    ) -> tuple[str, bool]:
        if self.punishment == "ban":
            return "ban", await self._apply_ban(member, reason, case_id)
        return "timeout", await self._apply_timeout(member, reason, case_id)

    async def _apply_ban(self, member: discord.Member, reason: str, case_id: str) -> bool:
        guild = member.guild
        me = guild.me
        if me is None:
            try:
                me = await guild.fetch_member(self.bot.user.id)
            except discord.HTTPException:
                log.warning("Could not resolve self member for guild %s", guild.id)
                return False

        if not me.guild_permissions.ban_members:
            log.warning("Missing Ban Members permission to ban %s", member.id)
            return False

        ban_reason = f"[SecurityGuard][case:{case_id}] {reason}"
        try:
            try:
                await guild.ban(member, reason=ban_reason, delete_message_seconds=0)
            except TypeError:
                await guild.ban(member, reason=ban_reason, delete_message_days=0)
            return True
        except discord.Forbidden:
            log.warning("Forbidden to ban member %s", member.id)
        except discord.HTTPException as exc:
            log.warning("Failed to ban member %s: %s", member.id, exc)
        return False

    async def _apply_timeout(self, member: discord.Member, reason: str, case_id: str) -> bool:
        guild = member.guild
        me = guild.me
        if me is None:
            try:
                me = await guild.fetch_member(self.bot.user.id)
            except discord.HTTPException:
                log.warning("Could not resolve self member for guild %s", guild.id)
                return False

        perms = me.guild_permissions
        if not perms.moderate_members:
            log.warning("Missing Moderate Members permission to timeout %s", member.id)
            return False

        until = discord.utils.utcnow() + timedelta(minutes=self.timeout_minutes)
        timeout_reason = f"[SecurityGuard][case:{case_id}] {reason}"
        try:
            await member.edit(communication_disabled_until=until, reason=timeout_reason)
            return True
        except discord.Forbidden:
            log.warning("Forbidden to timeout member %s", member.id)
        except discord.HTTPException as exc:
            log.warning("Failed to timeout member %s: %s", member.id, exc)
        return False

    async def _delete_messages(self, msgs: list[RecentMessage], reason: str) -> int:
        deleted = 0
        seen: set[int] = set()
        for msg in msgs:
            if msg.message.id in seen:
                continue
            seen.add(msg.message.id)
            try:
                await msg.message.delete(reason=f"[SecurityGuard] {reason}")
                deleted += 1
            except discord.NotFound:
                log.debug("Message %s already removed before deletion step", msg.message.id)
            except discord.HTTPException as exc:
                log.warning("Failed to delete message %s: %s", msg.message.id, exc)
            await asyncio.sleep(0.2)
        return deleted

    async def _collect_attachments(self, msgs: list[RecentMessage]) -> list[discord.File]:
        files: list[discord.File] = []
        per_channel_taken: dict[int, int] = {}
        for msg in msgs:
            for att in msg.attachments:
                taken = per_channel_taken.get(msg.channel_id, 0)
                if taken >= 1:
                    continue  # nur ein Anhang pro Channel spiegeln
                if len(files) >= self.attachment_forward_limit:
                    return files
                if att.size and att.size > self.attachment_max_bytes:
                    log.info(
                        "Skip attachment %s (%s bytes) - too large for mirror",
                        att.filename,
                        att.size,
                    )
                    continue
                try:
                    files.append(await att.to_file())
                    per_channel_taken[msg.channel_id] = taken + 1
                except Exception as exc:
                    log.debug("Failed to mirror attachment %s: %s", att.filename, exc)
        return files

    async def _resolve_review_channel(self, guild: discord.Guild) -> discord.TextChannel | None:
        if not self.review_channel_id:
            return None
        ch = guild.get_channel(self.review_channel_id)
        if isinstance(ch, discord.TextChannel):
            return ch
        try:
            fetched = await guild.fetch_channel(self.review_channel_id)
            if isinstance(fetched, discord.TextChannel):
                return fetched
        except discord.HTTPException as exc:
            log.warning("Could not fetch review channel %s: %s", self.review_channel_id, exc)
        return None

    async def _resolve_mod_channel(self, guild: discord.Guild) -> discord.TextChannel | None:
        if not self.mod_channel_id:
            return None
        ch = guild.get_channel(self.mod_channel_id)
        if isinstance(ch, discord.TextChannel):
            return ch
        try:
            fetched = await guild.fetch_channel(self.mod_channel_id)
            if isinstance(fetched, discord.TextChannel):
                return fetched
        except discord.HTTPException as exc:
            log.warning("Could not fetch mod channel %s: %s", self.mod_channel_id, exc)
        return None

    async def handle_appeal_submission(
        self,
        interaction: discord.Interaction,
        case_id: str,
        appeal_text: str,
    ) -> None:
        case = self._cases.get(case_id)
        user = interaction.user
        guild = None
        guild_id = case.guild_id if case else None
        if guild_id is None:
            parts = case_id.split("-", 2)
            if parts and parts[0].isdigit():
                guild_id = int(parts[0])
        if guild_id:
            guild = self.bot.get_guild(guild_id)
            if guild is None:
                try:
                    guild = await self.bot.fetch_guild(guild_id)
                except discord.HTTPException:
                    guild = None

        mod_channel = await self._resolve_mod_channel(guild) if guild else None
        safe_appeal = appeal_text.replace("`", "'").strip()
        if not safe_appeal:
            safe_appeal = "(empty)"

        if mod_channel:
            embed = discord.Embed(
                title="Appeal submitted",
                color=0x3498DB,
                timestamp=discord.utils.utcnow(),
            )
            embed.add_field(name="Member", value=f"{user.mention} ({user.id})", inline=False)
            embed.add_field(name="Case ID", value=case_id, inline=True)
            if case:
                embed.add_field(name="Original reason", value=case.reason or "n/a", inline=False)
            embed.add_field(name="Appeal reason", value=safe_appeal[:1000], inline=False)
            try:
                await mod_channel.send(embed=embed)
            except discord.HTTPException as exc:
                log.warning("Failed to post appeal for case %s: %s", case_id, exc)
        else:
            log.warning("No mod channel set; appeal case %s logged to stdout.", case_id)
            log.info("Appeal %s by %s: %s", case_id, user.id, _safe_log_value(safe_appeal))

        try:
            await interaction.response.send_message("Your appeal was sent to the moderators.")
        except discord.HTTPException as exc:
            log.debug("Could not send appeal ack to user %s: %s", user.id, exc)

    async def handle_unban_request(
        self,
        interaction: discord.Interaction,
        guild_id: int,
        user_id: int,
        case_id: str,
        button: discord.ui.Button,
        view: discord.ui.View,
    ) -> None:
        perms = getattr(interaction.user, "guild_permissions", None)
        if not perms or not (perms.ban_members or perms.administrator):
            await interaction.response.send_message(
                "You do not have permission to unban.", ephemeral=True
            )
            return

        await interaction.response.defer(ephemeral=True)

        guild = interaction.guild
        if guild is None or guild.id != guild_id:
            guild = self.bot.get_guild(guild_id)
            if guild is None:
                try:
                    guild = await self.bot.fetch_guild(guild_id)
                except discord.HTTPException:
                    await interaction.followup.send("Guild not found.", ephemeral=True)
                    return

        try:
            await guild.unban(
                discord.Object(id=user_id),
                reason=f"[SecurityGuard][case:{case_id}] unban by {interaction.user} ({interaction.user.id})",
            )
            button.disabled = True
            try:
                await interaction.message.edit(view=view)
            except discord.HTTPException as exc:
                log.debug("Unable to update unban message view for case %s: %s", case_id, exc)
            await interaction.followup.send("Unban completed.", ephemeral=True)
        except discord.NotFound:
            await interaction.followup.send("User is not banned.", ephemeral=True)
        except discord.Forbidden:
            await interaction.followup.send("Bot lacks permission to unban.", ephemeral=True)
        except discord.HTTPException as exc:
            log.warning("Unban failed for case %s: %s", case_id, exc)
            await interaction.followup.send("Unban failed.", ephemeral=True)

    async def _log_incident(
        self,
        member: discord.Member,
        msgs: list[RecentMessage],
        reason: str,
        meta: dict[str, int],
        action: str,
        action_ok: bool,
        deleted: int,
        files: list[discord.File],
        case_id: str,
        dm_sent: bool,
    ) -> None:
        now = discord.utils.utcnow()
        channel_names = {
            m.channel_id: getattr(m.message.channel, "mention", f"#{m.channel_id}") for m in msgs
        }
        lines = []
        for idx, msg in enumerate(sorted(msgs, key=lambda m: m.created_at)):
            channel_display = channel_names.get(msg.channel_id, str(msg.channel_id))
            ts = msg.created_at.strftime("%H:%M:%S")
            snippet = (
                msg.content.replace("\r", "\\r").replace("\n", "\\n").strip().replace("`", "'")
            )
            if len(snippet) > 160:
                snippet = snippet[:157] + "..."
            attach_note = f" [attachments: {len(msg.attachments)}]" if msg.attachments else ""
            if not snippet:
                snippet = "(kein Text)"
            lines.append(f"{idx + 1}. {ts} {channel_display}: {snippet}{attach_note}")

        proof_blocks = self._chunk_lines(lines, 1900)
        review_channel = await self._resolve_review_channel(member.guild)
        mod_channel = await self._resolve_mod_channel(member.guild)

        action_label = "Ban" if action == "ban" else "Timeout"
        if action == "ban":
            action_text = f"Ban: {'yes' if action_ok else 'failed'}"
        else:
            action_text = f"Timeout {self.timeout_minutes}m: {'yes' if action_ok else 'failed'}"

        embed = discord.Embed(
            title=f"Auto-{action_label}: possible scam/spam burst",
            color=0xE74C3C,
            timestamp=now,
        )
        embed.add_field(name="Member", value=f"{member.mention} ({member.id})", inline=False)
        embed.add_field(name="Case ID", value=case_id, inline=True)
        embed.add_field(
            name="Account age",
            value=self._fmt_delta(now, member.created_at),
            inline=True,
        )
        embed.add_field(
            name="Time since join",
            value=self._fmt_delta(now, member.joined_at),
            inline=True,
        )
        embed.add_field(
            name="Activity window",
            value=f"{meta.get('message_count', 0)} msgs / {meta.get('channel_count', 0)} channels in {self.window_seconds}s",
            inline=False,
        )
        embed.add_field(
            name="Signals",
            value=f"Keywords: {bool(meta.get('keyword_hit'))} | Attachments: {meta.get('attachment_count', 0)}",
            inline=True,
        )
        embed.add_field(
            name="Actions",
            value=f"{action_text}\nDeleted: {deleted}\nDM sent: {'yes' if dm_sent else 'no'}",
            inline=True,
        )
        embed.add_field(name="Reason", value=reason or "auto-detected burst", inline=False)

        sent_any = False
        if mod_channel:
            try:
                view = (
                    UnbanView(self, member.guild.id, member.id, case_id)
                    if action == "ban" and action_ok
                    else None
                )
                await mod_channel.send(embed=embed, view=view)
                for block in proof_blocks:
                    await mod_channel.send(f"```{block}```")
                if files:
                    await mod_channel.send(
                        content="Mirrored attachments (capped).",
                        files=files,
                    )
                sent_any = True
            except discord.HTTPException as exc:
                log.warning("Failed to send mod log: %s", exc)

        if review_channel and (not mod_channel or review_channel.id != mod_channel.id):
            try:
                await review_channel.send(embed=embed)
                for block in proof_blocks:
                    await review_channel.send(f"```{block}```")
                sent_any = True
            except discord.HTTPException as exc:
                log.warning("Failed to send review log: %s", exc)

        if not sent_any:
            log.warning("No log channel set; incident logged to stdout.")
            log.info("Incident %s: %s", member.id, "\n".join(lines))

    def _chunk_lines(self, lines: Iterable[str], max_len: int) -> list[str]:
        chunks: list[str] = []
        buf = ""
        for line in lines:
            if len(buf) + len(line) + 1 > max_len:
                chunks.append(buf.rstrip())
                buf = ""
            buf += line + "\n"
        if buf:
            chunks.append(buf.rstrip())
        return chunks

    def _fmt_delta(self, now: datetime, past: datetime | None) -> str:
        if not past:
            return "n/a"
        delta = now - past.replace(tzinfo=UTC)
        days = delta.days
        hours, remainder = divmod(delta.seconds, 3600)
        minutes, _ = divmod(remainder, 60)
        if days > 0:
            return f"{days}d {hours}h"
        if hours > 0:
            return f"{hours}h {minutes}m"
        return f"{minutes}m"

    # ---------------- Commands ----------------
    @commands.command(name="security_diag", help="Zeigt die aktiven Spam-Guard Schwellen.")
    @commands.has_permissions(administrator=True)
    async def security_diag(self, ctx: commands.Context):
        review = f"<#{self.review_channel_id}>" if self.review_channel_id else "nicht gesetzt"
        mod = f"<#{self.mod_channel_id}>" if self.mod_channel_id else "nicht gesetzt"
        guilds = (
            ", ".join(str(g) for g in sorted(self.allowed_guild_ids))
            if self.allowed_guild_ids
            else "alle"
        )
        desc = (
            f"Fenster: {self.window_seconds}s | Kanaele: >= {self.channel_threshold} | "
            f"Nachrichten: >= {self.message_threshold}\n"
            f"Account-Alter: <= {self.account_max_age_hours}h (kein Join-Gate)\n"
            f"Etabliert ab: Account >= {self.established_account_min_age_hours}h & Join >= {self.established_min_join_hours}h → Mod-Vorschlag\n"
            f"AI-Scam: Provider={self.ai_scam_provider} | Confidence >= {self.ai_scam_confidence:.0%}\n"
            f"Aktion: {self.punishment} | Timeout: {self.timeout_minutes}m\n"
            f"Review-Channel: {review} | Mod-Channel: {mod}\n"
            f"Aktiv auf Guilds: {guilds}"
        )
        await ctx.reply(desc, mention_author=False)


async def setup(bot: commands.Bot):
    await bot.add_cog(SecurityGuard(bot))
