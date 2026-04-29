from __future__ import annotations

import asyncio
import json
import logging
import os
import re
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import discord
from discord.ext import commands, tasks

log = logging.getLogger(__name__)


AI_MODERATOR_CONFIG = {
    "SCAN_CHANNEL_IDS": [1289721245281292291],
    "MOD_REVIEW_CHANNEL_ID": 1315684135175716978,
    "LOG_CHANNEL_ID": 1374364800817303632,
    "AI_PROVIDER": "minimax",
    "AI_MODEL": "MiniMax-M2.7",
    "TIMEOUT_MINUTES": 1440,
    "RAGEBAIT_WINDOW_MINUTES": 120,
    "RAGEBAIT_ESCALATE_THRESHOLD": 4,
    "AUTO_DELETE_CONFIDENCE": 0.90,
    "PROPOSE_CONFIDENCE": 0.55,
    "CONTEXT_ESCALATE_BETWEEN": (0.35, 0.70),
    "CONTEXT_BACKFILL_MESSAGES": 25,
    "AUTO_DELETE_CATEGORIES": ["nsfw_explicit", "csam", "raping", "epstein_child"],
    "IGNORE_BOTS": True,
    "PER_USER_COOLDOWN_SECONDS": 2,
    "MAX_IMAGES_PER_CHECK": 4,
    "MAX_PROMPT_CHARS": 4000,
}

MODERATION_SYSTEM_PROMPT = """Du bist ein Discord-Moderations-Klassifikator fuer einen Gaming-Server zu Deadlock.
Deine Aufgabe: Bewerte die letzte Nachricht eines Users. Antworte ausschliesslich mit gueltigem JSON und ohne weiteren Text.

Regeln:
- NSFW explizit, Pornografie, sexuelle Gewalt, Raping, Child-Content, Minderjaehrigen-Sexualisierung oder Epstein-bezogene Child-Anspielungen:
  verdict="delete", category="nsfw_explicit"|"csam"|"raping"|"epstein_child", confidence hoch.
- Racism, Hate Speech, Slurs, gezieltes Mobbing oder wiederholte persoenliche Angriffe:
  verdict="propose", category="racism"|"harassment"|"hate_speech".
- Ragebait, Gaming-Flame, Trash Talk, "noob", "trash", "deine Mama" im Spass oder provokante Spiel-Diskussion:
  verdict="ok", category="ragebait_ok" oder "game_related_ok".
  WICHTIG: Ragebait ist grundsaetzlich erlaubt und soll NICHT eskaliert werden. Der Bot zaehlt das separat.
  Nur gezieltes oder wiederholtes Mobbing gegen konkrete Personen ist harassment.
- Normale Spielkritik, Balance-Beschwerden, Patchnotes, Taktik, Matchmaking, Diskussion ueber Deadlock:
  verdict="ok", category="game_related_ok".
- Wenn du ohne mehr Kontext unsicher bist:
  verdict="needs_context", needs_context=true.
- Bei Bildern/GIFs: Wenn unsicher oder nicht klar erkennbar, needs_context=true.

Erlaubte Kategorien:
nsfw_explicit, csam, raping, epstein_child, racism, harassment, hate_speech, ragebait_ok, game_related_ok, other

Output-Format strikt:
{"verdict":"ok|delete|propose|needs_context","category":"...","confidence":0.0,"reason":"1-2 Saetze Deutsch","needs_context":true}
"""

SCHEMA_SQL = """
CREATE TABLE IF NOT EXISTS ai_moderation_cases (
    case_id TEXT PRIMARY KEY,
    guild_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    user_tag TEXT,
    original_content TEXT,
    attachments_json TEXT,
    ai_category TEXT,
    ai_confidence REAL,
    ai_reason TEXT,
    ai_raw_json TEXT,
    escalated_with_context INTEGER DEFAULT 0,
    action TEXT,
    mod_id INTEGER,
    mod_action_at TIMESTAMP,
    mod_deny_reason TEXT,
    mod_review_message_id INTEGER,
    log_message_id INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_mod_cases_user ON ai_moderation_cases(user_id);
CREATE INDEX IF NOT EXISTS idx_mod_cases_created ON ai_moderation_cases(created_at);
CREATE TABLE IF NOT EXISTS ai_moderation_ragebait_hits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    content_preview TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_ragebait_user_time ON ai_moderation_ragebait_hits(user_id, created_at);
"""

CASE_ID_TEMPLATE = r"(?P<case_id>[A-Za-z0-9_-]+)"
JSON_RE = re.compile(r"\{.*\}", re.DOTALL)
MENTION_RE = re.compile(r"<[@#][!&]?\d+>")
WHITESPACE_RE = re.compile(r"\s+")
DISCORD_FIELD_LIMIT = 1024
DISCORD_MESSAGE_SAFE_LIMIT = 1800
PERSISTENT_CASE_PLACEHOLDER = "template"
RAGEBAITER_FREE_PROPOSE_CONFIDENCE = 0.40
RAGEBAITER_MOD_TAG_DURATION = timedelta(days=14)
RAGEBAITER_FREE_WARNING_WINDOW = timedelta(minutes=30)
ALLOWED_CATEGORIES = {
    "nsfw_explicit",
    "csam",
    "raping",
    "epstein_child",
    "racism",
    "harassment",
    "hate_speech",
    "ragebait_ok",
    "game_related_ok",
    "other",
}
INTERNAL_CATEGORIES = ALLOWED_CATEGORIES | {"persistent_ragebait"}
ALLOWED_VERDICTS = {"ok", "delete", "propose", "needs_context"}
CATEGORY_LABELS = {
    "nsfw_explicit": "NSFW",
    "csam": "CSAM",
    "raping": "Raping",
    "epstein_child": "Epstein/Child",
    "racism": "Racism",
    "harassment": "Harassment",
    "hate_speech": "Hate Speech",
    "ragebait_ok": "Ragebait OK",
    "game_related_ok": "Game Related OK",
    "persistent_ragebait": "Persistent Ragebait",
    "other": "Other",
}


def _normalize_text(value: str | None) -> str:
    return WHITESPACE_RE.sub(" ", (value or "").replace("\r", " ").replace("\n", " ")).strip()


def _strip_mentions(value: str | None) -> str:
    return _normalize_text(MENTION_RE.sub("", value or ""))


def _truncate(value: str | None, limit: int, *, suffix: str = "…") -> str:
    text = value or ""
    if len(text) <= limit:
        return text
    if limit <= len(suffix):
        return suffix[:limit]
    return text[: limit - len(suffix)].rstrip() + suffix


def _safe_title_fragment(value: str) -> str:
    return CATEGORY_LABELS.get(value, "Other")


def _case_jump_url(guild_id: int, channel_id: int, message_id: int) -> str:
    return f"https://discord.com/channels/{guild_id}/{channel_id}/{message_id}"


def _content_preview(value: str | None, *, limit: int = 180) -> str:
    cleaned = _strip_mentions(value or "")
    if not cleaned:
        return "[kein Text]"
    return _truncate(cleaned, limit)


def _safe_message_text(value: str | None, *, limit: int) -> str:
    text = discord.utils.escape_mentions(value or "")
    text = text.strip()
    if not text:
        return "[kein Text]"
    return _truncate(text, limit)


def _resolve_db_path() -> Path:
    env_db_path = os.getenv("DEADLOCK_DB_PATH")
    if env_db_path:
        return Path(env_db_path)
    env_db_dir = os.getenv("DEADLOCK_DB_DIR")
    if env_db_dir:
        return Path(env_db_dir) / "deadlock.sqlite3"
    return Path(__file__).resolve().parent.parent / "data" / "deadlock.sqlite3"


@dataclass(slots=True)
class AIVerdict:
    verdict: str
    category: str
    confidence: float
    reason: str
    needs_context: bool
    raw_json: str


@dataclass(slots=True)
class ModerationCase:
    case_id: str
    guild_id: int
    channel_id: int
    message_id: int
    user_id: int
    user_tag: str
    original_content: str
    attachments: list[dict[str, str]]
    ai_category: str
    ai_confidence: float
    ai_reason: str
    ai_raw_json: str
    escalated_with_context: bool
    action: str
    mod_id: int | None = None
    mod_deny_reason: str | None = None
    mod_review_message_id: int | None = None
    log_message_id: int | None = None
    mod_action_at: str | None = None

    @property
    def jump_url(self) -> str:
        return _case_jump_url(self.guild_id, self.channel_id, self.message_id)

    @classmethod
    def from_row(cls, row: sqlite3.Row) -> ModerationCase:
        attachments_raw = row["attachments_json"] or "[]"
        try:
            attachments = json.loads(attachments_raw)
        except json.JSONDecodeError:
            attachments = []
        return cls(
            case_id=str(row["case_id"]),
            guild_id=int(row["guild_id"]),
            channel_id=int(row["channel_id"]),
            message_id=int(row["message_id"]),
            user_id=int(row["user_id"]),
            user_tag=str(row["user_tag"] or f"User {row['user_id']}"),
            original_content=str(row["original_content"] or ""),
            attachments=attachments if isinstance(attachments, list) else [],
            ai_category=str(row["ai_category"] or "other"),
            ai_confidence=float(row["ai_confidence"] or 0.0),
            ai_reason=str(row["ai_reason"] or ""),
            ai_raw_json=str(row["ai_raw_json"] or ""),
            escalated_with_context=bool(row["escalated_with_context"]),
            action=str(row["action"] or ""),
            mod_id=int(row["mod_id"]) if row["mod_id"] is not None else None,
            mod_deny_reason=str(row["mod_deny_reason"]) if row["mod_deny_reason"] else None,
            mod_review_message_id=(
                int(row["mod_review_message_id"]) if row["mod_review_message_id"] else None
            ),
            log_message_id=int(row["log_message_id"]) if row["log_message_id"] else None,
            mod_action_at=str(row["mod_action_at"]) if row["mod_action_at"] else None,
        )


class DenyReasonModal(discord.ui.Modal):
    def __init__(self, cog: AIModeratorCog, case_id: str) -> None:
        super().__init__(title="Moderation ablehnen")
        self.cog = cog
        self.case_id = case_id
        self.reason = discord.ui.TextInput(
            label="Warum lehnst du ab?",
            style=discord.TextStyle.paragraph,
            required=True,
            min_length=4,
            max_length=500,
            placeholder="Kurze Begruendung fuer die Ablehnung.",
        )
        self.add_item(self.reason)

    async def on_submit(self, interaction: discord.Interaction) -> None:
        await self.cog.handle_deny_submit(interaction, self.case_id, self.reason.value.strip())


class AcceptModerationButton(
    discord.ui.DynamicItem[discord.ui.Button[discord.ui.View]],
    template=rf"aimod:accept:{CASE_ID_TEMPLATE}",
):
    def __init__(self, cog: AIModeratorCog, case_id: str | None = None) -> None:
        self.cog = cog
        self.case_id = case_id or PERSISTENT_CASE_PLACEHOLDER
        super().__init__(
            discord.ui.Button(
                label="Accept",
                style=discord.ButtonStyle.success,
                custom_id=f"aimod:accept:{self.case_id}",
            )
        )

    @classmethod
    async def from_custom_id(
        cls,
        interaction: discord.Interaction,
        item: discord.ui.Item[Any],
        match: re.Match[str],
    ) -> AcceptModerationButton:
        cog = interaction.client.get_cog("AIModeratorCog")
        if cog is None:
            raise RuntimeError("AIModeratorCog nicht geladen")
        return cls(cog, match.group("case_id"))

    async def callback(self, interaction: discord.Interaction) -> Any:
        await self.cog.handle_accept_interaction(interaction, self.case_id)


class DenyModerationButton(
    discord.ui.DynamicItem[discord.ui.Button[discord.ui.View]],
    template=rf"aimod:deny:{CASE_ID_TEMPLATE}",
):
    def __init__(self, cog: AIModeratorCog, case_id: str | None = None) -> None:
        self.cog = cog
        self.case_id = case_id or PERSISTENT_CASE_PLACEHOLDER
        super().__init__(
            discord.ui.Button(
                label="Deny",
                style=discord.ButtonStyle.danger,
                custom_id=f"aimod:deny:{self.case_id}",
            )
        )

    @classmethod
    async def from_custom_id(
        cls,
        interaction: discord.Interaction,
        item: discord.ui.Item[Any],
        match: re.Match[str],
    ) -> DenyModerationButton:
        cog = interaction.client.get_cog("AIModeratorCog")
        if cog is None:
            raise RuntimeError("AIModeratorCog nicht geladen")
        return cls(cog, match.group("case_id"))

    async def callback(self, interaction: discord.Interaction) -> Any:
        await self.cog.handle_deny_interaction(interaction, self.case_id)


class ModerationProposalView(discord.ui.View):
    def __init__(self, cog: AIModeratorCog, case_id: str | None) -> None:
        super().__init__(timeout=None)
        self.add_item(AcceptModerationButton(cog, case_id))
        self.add_item(DenyModerationButton(cog, case_id))


class AIModeratorCog(commands.Cog):
    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        cfg = AI_MODERATOR_CONFIG
        self.scan_channel_ids = {int(channel_id) for channel_id in cfg["SCAN_CHANNEL_IDS"]}
        self.mod_review_channel_id = int(cfg["MOD_REVIEW_CHANNEL_ID"])
        self.log_channel_id = int(cfg["LOG_CHANNEL_ID"])
        self.ai_provider = str(cfg["AI_PROVIDER"])
        self.ai_model = str(cfg["AI_MODEL"])
        self.timeout_minutes = int(cfg["TIMEOUT_MINUTES"])
        self.ragebait_window_minutes = int(cfg["RAGEBAIT_WINDOW_MINUTES"])
        self.ragebait_escalate_threshold = int(cfg["RAGEBAIT_ESCALATE_THRESHOLD"])
        self.auto_delete_confidence = float(cfg["AUTO_DELETE_CONFIDENCE"])
        self.propose_confidence = float(cfg["PROPOSE_CONFIDENCE"])
        lower, upper = cfg["CONTEXT_ESCALATE_BETWEEN"]
        self.context_escalate_lower = float(lower)
        self.context_escalate_upper = float(upper)
        self.context_backfill_messages = int(cfg["CONTEXT_BACKFILL_MESSAGES"])
        self.auto_delete_categories = {str(item) for item in cfg["AUTO_DELETE_CATEGORIES"]}
        self.ignore_bots = bool(cfg["IGNORE_BOTS"])
        self.per_user_cooldown_seconds = float(cfg["PER_USER_COOLDOWN_SECONDS"])
        self.max_images_per_check = int(cfg["MAX_IMAGES_PER_CHECK"])
        self.max_prompt_chars = int(cfg["MAX_PROMPT_CHARS"])
        self.db_path = _resolve_db_path()
        self._last_scan_ts: dict[int, float] = {}
        self._ragebaiter_free_warnings: dict[tuple[int, int], datetime] = {}

    async def cog_load(self) -> None:
        await asyncio.to_thread(self._ensure_schema_sync)
        self.db_path.parent.mkdir(parents=True, exist_ok=True)

        try:
            self.bot.add_dynamic_items(AcceptModerationButton, DenyModerationButton)
        except Exception as exc:
            log.debug("Dynamic items bereits registriert oder fehlgeschlagen: %s", exc)

        try:
            self.bot.add_view(ModerationProposalView(self, case_id=None))
        except Exception as exc:
            log.debug("Persistent moderation view bereits registriert oder fehlgeschlagen: %s", exc)

        for case in await asyncio.to_thread(self._fetch_open_proposals_sync):
            if case.mod_review_message_id is None:
                continue
            try:
                self.bot.add_view(
                    ModerationProposalView(self, case.case_id),
                    message_id=case.mod_review_message_id,
                )
            except Exception as exc:
                log.debug("Konnte Proposal-View fuer Case %s nicht registrieren: %s", case.case_id, exc)

        if not self.cleanup_ragebait_hits.is_running():
            self.cleanup_ragebait_hits.start()

    def cog_unload(self) -> None:
        if self.cleanup_ragebait_hits.is_running():
            self.cleanup_ragebait_hits.cancel()

    @tasks.loop(minutes=10)
    async def cleanup_ragebait_hits(self) -> None:
        deleted = await asyncio.to_thread(self._cleanup_ragebait_hits_sync)
        if deleted:
            log.debug("AI Moderator cleanup entfernte %s alte Ragebait-Hits", deleted)

    @cleanup_ragebait_hits.before_loop
    async def before_cleanup_ragebait_hits(self) -> None:
        await self.bot.wait_until_ready()

    @commands.Cog.listener()
    async def on_message(self, message: discord.Message) -> None:
        if message.guild is None:
            return
        if self.ignore_bots and message.author.bot:
            return
        if message.channel.id not in self.scan_channel_ids:
            return
        if message.webhook_id is not None or message.is_system():
            return
        if not isinstance(message.author, discord.Member):
            return
        perms = getattr(message.author, "guild_permissions", None)
        if perms and perms.manage_messages:
            return

        image_attachments = self._extract_image_attachments(message.attachments)
        if not _normalize_text(message.content) and not image_attachments:
            return

        now = asyncio.get_running_loop().time()
        last_ts = self._last_scan_ts.get(message.author.id, 0.0)
        if now - last_ts < self.per_user_cooldown_seconds:
            return
        self._last_scan_ts[message.author.id] = now

        required_tone_tag = await asyncio.to_thread(
            self._get_required_tone_tag_sync,
            message.channel.id,
        )
        proposal_threshold = self._get_proposal_threshold(required_tone_tag)
        verdict, escalated = await self._classify_message(message, image_attachments)
        if verdict.verdict == "needs_context":
            return

        if verdict.verdict == "ok" and verdict.category == "ragebait_ok":
            escalated_verdict = await self._handle_ragebait_hit(message, verdict)
            if escalated_verdict is not None:
                verdict = escalated_verdict
                await self._create_proposal_case(
                    message,
                    verdict,
                    escalated_with_context=escalated,
                    action="ragebait_escalated",
                )
                return
            if await self._maybe_handle_ragebaiter_free_warning(
                message,
                verdict,
                required_tone_tag=required_tone_tag,
                escalated_with_context=escalated,
            ):
                return
            return

        if verdict.verdict == "ok":
            return

        if (
            verdict.verdict == "delete"
            and verdict.category in self.auto_delete_categories
            and verdict.confidence >= self.auto_delete_confidence
        ):
            await self._auto_delete_case(message, verdict, escalated_with_context=escalated)
            return

        if verdict.confidence < proposal_threshold:
            return

        if verdict.verdict in {"delete", "propose"}:
            await self._create_proposal_case(
                message,
                verdict,
                escalated_with_context=escalated,
                action="proposed",
            )

    async def handle_accept_interaction(
        self, interaction: discord.Interaction, case_id: str
    ) -> None:
        if not self._has_review_permission(interaction):
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
            return

        await interaction.response.defer(ephemeral=True)
        case = await asyncio.to_thread(self._fetch_case_sync, case_id)
        if case is None:
            await interaction.followup.send("Case nicht gefunden.", ephemeral=True)
            return
        if case.action in {"accepted", "denied"}:
            await interaction.followup.send("Case wurde bereits bearbeitet.", ephemeral=True)
            return

        delete_ok, delete_note = await self._delete_case_message(case)
        timeout_ok, timeout_note = await self._timeout_case_member(case, interaction.guild)
        detail_bits = [note for note in (delete_note, timeout_note) if note]

        await asyncio.to_thread(
            self._mark_case_accepted_sync,
            case.case_id,
            interaction.user.id,
        )
        updated = await asyncio.to_thread(self._fetch_case_sync, case.case_id)
        if updated is None:
            updated = case
            updated.action = "accepted"
            updated.mod_id = interaction.user.id

        status = f"Accepted by {interaction.user.mention}"
        if detail_bits:
            status = f"{status} ({'; '.join(detail_bits)})"
        await self._edit_review_message(updated, status_text=status)

        detail = "; ".join(
            bit
            for bit in (
                None if delete_ok else "Delete fehlgeschlagen",
                None if timeout_ok else "Timeout fehlgeschlagen",
            )
            if bit
        )
        log_message_id = await self._post_action_log(
            updated,
            action="accepted",
            mod_user=interaction.user,
            detail=detail,
        )
        if log_message_id is not None:
            await asyncio.to_thread(self._update_log_message_id_sync, case.case_id, log_message_id)

        await interaction.followup.send("Moderationsvorschlag akzeptiert.", ephemeral=True)

    async def handle_deny_interaction(self, interaction: discord.Interaction, case_id: str) -> None:
        if not self._has_review_permission(interaction):
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
            return
        await interaction.response.send_modal(DenyReasonModal(self, case_id))

    async def handle_deny_submit(
        self, interaction: discord.Interaction, case_id: str, reason: str
    ) -> None:
        if not self._has_review_permission(interaction):
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
            return

        await interaction.response.defer(ephemeral=True)
        case = await asyncio.to_thread(self._fetch_case_sync, case_id)
        if case is None:
            await interaction.followup.send("Case nicht gefunden.", ephemeral=True)
            return
        if case.action in {"accepted", "denied"}:
            await interaction.followup.send("Case wurde bereits bearbeitet.", ephemeral=True)
            return

        await asyncio.to_thread(
            self._mark_case_denied_sync,
            case.case_id,
            interaction.user.id,
            reason,
        )
        updated = await asyncio.to_thread(self._fetch_case_sync, case.case_id)
        if updated is None:
            updated = case
            updated.action = "denied"
            updated.mod_id = interaction.user.id
            updated.mod_deny_reason = reason

        status = f"Denied by {interaction.user.mention}: {_truncate(reason, 200)}"
        await self._edit_review_message(updated, status_text=status)
        log_message_id = await self._post_action_log(
            updated,
            action="denied",
            mod_user=interaction.user,
            deny_reason=reason,
        )
        if log_message_id is not None:
            await asyncio.to_thread(self._update_log_message_id_sync, case.case_id, log_message_id)

        await interaction.followup.send("Moderationsvorschlag abgelehnt.", ephemeral=True)

    async def _classify_message(
        self,
        message: discord.Message,
        image_attachments: list[discord.Attachment],
    ) -> tuple[AIVerdict, bool]:
        minimal_context = await self._fetch_context_lines(message, limit=2)
        verdict = await self._run_moderation_call(
            message=message,
            image_attachments=image_attachments,
            context_lines=minimal_context,
            include_full_context=False,
        )
        needs_escalation = verdict.verdict == "needs_context" or (
            self.context_escalate_lower <= verdict.confidence < self.context_escalate_upper
        )
        if not needs_escalation:
            return verdict, False

        full_context = await self._fetch_context_lines(
            message,
            limit=self.context_backfill_messages,
        )
        escalated_verdict = await self._run_moderation_call(
            message=message,
            image_attachments=image_attachments,
            context_lines=full_context,
            include_full_context=True,
        )
        return escalated_verdict, True

    async def _run_moderation_call(
        self,
        *,
        message: discord.Message,
        image_attachments: list[discord.Attachment],
        context_lines: list[str],
        include_full_context: bool,
    ) -> AIVerdict:
        ai = self.bot.get_cog("AIConnector")
        if ai is None or not hasattr(ai, "generate_multimodal"):
            raw_json = json.dumps(
                {"error": "generate_multimodal_unavailable"},
                ensure_ascii=False,
            )
            return AIVerdict(
                verdict="needs_context",
                category="other",
                confidence=0.0,
                reason="parse_error",
                needs_context=True,
                raw_json=raw_json,
            )

        payload = self._build_prompt_payload(message, context_lines, len(image_attachments), include_full_context)
        prompt = json.dumps(payload, ensure_ascii=False)
        image_urls = [attachment.url for attachment in image_attachments]

        try:
            response_text, meta = await ai.generate_multimodal(
                provider=self.ai_provider,
                prompt=prompt,
                images=image_urls,
                system_prompt=MODERATION_SYSTEM_PROMPT,
                model=self.ai_model,
                max_output_tokens=300,
                temperature=0.2,
            )
        except Exception as exc:
            log.warning("AI moderation request fehlgeschlagen fuer Message %s: %s", message.id, exc)
            response_text = None
            meta = {"error": str(exc)}

        return self._parse_ai_verdict(response_text, meta)

    def _build_prompt_payload(
        self,
        message: discord.Message,
        context_lines: list[str],
        attachment_count: int,
        include_full_context: bool,
    ) -> dict[str, Any]:
        focus_message = _truncate(_strip_mentions(message.content), 1000)
        if not focus_message:
            focus_message = "[kein Text]"

        context_limit = 110 if include_full_context else 220
        trimmed_context = [_truncate(line, context_limit) for line in context_lines]
        payload = {
            "user_message": focus_message,
            "user_tag": _truncate(str(message.author), 80),
            "recent_context": trimmed_context,
            "attachment_count": attachment_count,
        }
        if include_full_context:
            payload["analysis_stage"] = "context_escalation"
            payload["focus_message_id"] = str(message.id)
        prompt = json.dumps(payload, ensure_ascii=False)
        if len(prompt) <= self.max_prompt_chars:
            return payload

        overflow = len(prompt) - self.max_prompt_chars
        if trimmed_context:
            reduced_context = trimmed_context[:]
            while reduced_context and overflow > 0:
                reduced_context.pop(0)
                payload["recent_context"] = reduced_context
                prompt = json.dumps(payload, ensure_ascii=False)
                overflow = len(prompt) - self.max_prompt_chars
            return payload

        payload["user_message"] = _truncate(payload["user_message"], max(200, 1000 - overflow))
        return payload

    def _parse_ai_verdict(self, raw_text: str | None, meta: dict[str, Any]) -> AIVerdict:
        envelope = {"response_text": raw_text, "meta": meta}
        if not raw_text:
            return AIVerdict(
                verdict="needs_context",
                category="other",
                confidence=0.0,
                reason="parse_error",
                needs_context=True,
                raw_json=json.dumps(envelope, ensure_ascii=False),
            )

        match = JSON_RE.search(raw_text)
        if not match:
            return AIVerdict(
                verdict="needs_context",
                category="other",
                confidence=0.0,
                reason="parse_error",
                needs_context=True,
                raw_json=json.dumps(envelope, ensure_ascii=False),
            )

        try:
            payload = json.loads(match.group(0))
        except json.JSONDecodeError:
            return AIVerdict(
                verdict="needs_context",
                category="other",
                confidence=0.0,
                reason="parse_error",
                needs_context=True,
                raw_json=json.dumps(envelope, ensure_ascii=False),
            )

        verdict = str(payload.get("verdict") or "needs_context").strip().lower()
        category = str(payload.get("category") or "other").strip().lower()
        confidence_raw = payload.get("confidence", 0.0)
        try:
            confidence = float(confidence_raw)
        except (TypeError, ValueError):
            confidence = 0.0
        confidence = max(0.0, min(1.0, confidence))
        reason = _truncate(_normalize_text(str(payload.get("reason") or "")), 500)
        if not reason:
            reason = "Keine Begruendung geliefert."
        needs_context = bool(payload.get("needs_context", verdict == "needs_context"))
        if verdict not in ALLOWED_VERDICTS:
            verdict = "needs_context"
            needs_context = True
        if category not in ALLOWED_CATEGORIES:
            category = "other"

        envelope["parsed"] = payload
        return AIVerdict(
            verdict=verdict,
            category=category,
            confidence=confidence,
            reason=reason,
            needs_context=needs_context,
            raw_json=json.dumps(envelope, ensure_ascii=False),
        )

    async def _handle_ragebait_hit(
        self, message: discord.Message, verdict: AIVerdict
    ) -> AIVerdict | None:
        count, hits = await asyncio.to_thread(
            self._insert_ragebait_hit_sync,
            message.guild.id,
            message.author.id,
            message.id,
            message.channel.id,
            _content_preview(message.content),
        )
        if count < self.ragebait_escalate_threshold:
            return None
        await self._apply_persistent_ragebait_tag(message.author.id)

        lines = ["Wiederholtes Ragebait innerhalb des Zeitfensters erkannt:"]
        for hit in hits[-self.ragebait_escalate_threshold :]:
            preview = _truncate(str(hit["content_preview"] or "[kein Text]"), 90)
            jump_url = _case_jump_url(
                int(hit["guild_id"]),
                int(hit["channel_id"]),
                int(hit["message_id"]),
            )
            lines.append(f"- {preview} ({jump_url})")

        combined_reason = f"{verdict.reason}\n" + "\n".join(lines)
        return AIVerdict(
            verdict="propose",
            category="persistent_ragebait",
            confidence=max(self._get_proposal_threshold(None), verdict.confidence),
            reason=_truncate(combined_reason, 900),
            needs_context=False,
            raw_json=verdict.raw_json,
        )

    async def _apply_persistent_ragebait_tag(self, user_id: int) -> None:
        tag_service = self.bot.get_cog("TagService")
        if tag_service is None or not hasattr(tag_service, "add_mod_tag"):
            return

        expires_at = self._now_utc() + RAGEBAITER_MOD_TAG_DURATION
        try:
            await tag_service.add_mod_tag(
                int(user_id),
                "ragebaiter",
                set_by=0,
                reason="auto: persistent_ragebait",
                expires_at=expires_at,
            )
        except Exception as exc:
            log.warning("Konnte persistent_ragebait Tag fuer User %s nicht setzen: %s", user_id, exc)

    async def _maybe_handle_ragebaiter_free_warning(
        self,
        message: discord.Message,
        verdict: AIVerdict,
        *,
        required_tone_tag: str | None,
        escalated_with_context: bool,
    ) -> bool:
        if required_tone_tag != "ragebaiter_free":
            return False
        if not (RAGEBAITER_FREE_PROPOSE_CONFIDENCE <= verdict.confidence < self.propose_confidence):
            return False

        user_id = int(message.author.id)
        channel_id = int(message.channel.id)
        now = self._now_utc()
        if self._has_recent_ragebaiter_free_warning(user_id, channel_id, now=now):
            repeated_reason = (
                f"{verdict.reason}\nHinweis fuer Ragebaiter-Free-Channel bereits innerhalb von 30 Minuten erfolgt."
            )
            proposal_verdict = AIVerdict(
                verdict="propose",
                category=verdict.category,
                confidence=max(RAGEBAITER_FREE_PROPOSE_CONFIDENCE, verdict.confidence),
                reason=_truncate(repeated_reason, 900),
                needs_context=False,
                raw_json=verdict.raw_json,
            )
            await self._create_proposal_case(
                message,
                proposal_verdict,
                escalated_with_context=escalated_with_context,
                action="proposed",
            )
            return True

        self._remember_ragebaiter_free_warning(user_id, channel_id, now=now)
        await self._send_ragebaiter_free_hint(message)
        return True

    async def _send_ragebaiter_free_hint(self, message: discord.Message) -> None:
        sender = getattr(message.author, "send", None)
        if sender is None:
            return
        try:
            await sender(
                "Hinweis: Dieser Voice ist als `Ragebaiter-Free` markiert, "
                "bitte Provokationen reduzieren."
            )
        except discord.HTTPException:
            pass

    def _has_recent_ragebaiter_free_warning(
        self,
        user_id: int,
        channel_id: int,
        *,
        now: datetime,
    ) -> bool:
        self._prune_ragebaiter_free_warnings(now=now)
        warned_at = self._ragebaiter_free_warnings.get((int(user_id), int(channel_id)))
        if warned_at is None:
            return False
        return now - warned_at <= RAGEBAITER_FREE_WARNING_WINDOW

    def _remember_ragebaiter_free_warning(
        self,
        user_id: int,
        channel_id: int,
        *,
        now: datetime,
    ) -> None:
        self._prune_ragebaiter_free_warnings(now=now)
        self._ragebaiter_free_warnings[(int(user_id), int(channel_id))] = now

    def _prune_ragebaiter_free_warnings(self, *, now: datetime) -> None:
        expired_keys = [
            key
            for key, warned_at in self._ragebaiter_free_warnings.items()
            if now - warned_at > RAGEBAITER_FREE_WARNING_WINDOW
        ]
        for key in expired_keys:
            self._ragebaiter_free_warnings.pop(key, None)

    def _get_proposal_threshold(self, required_tone_tag: str | None) -> float:
        if required_tone_tag == "ragebaiter_free":
            return RAGEBAITER_FREE_PROPOSE_CONFIDENCE
        return self.propose_confidence

    def _get_required_tone_tag_sync(self, channel_id: int) -> str | None:
        try:
            with self._connect_db() as connection:
                row = connection.execute(
                    """
                    SELECT required_tone_tag
                    FROM tempvoice_lane_tag_filter
                    WHERE channel_id = ?
                    """,
                    (int(channel_id),),
                ).fetchone()
        except sqlite3.OperationalError:
            return None

        tone_tag = row["required_tone_tag"] if row else None
        if tone_tag is None:
            return None
        normalized = str(tone_tag).strip().lower()
        return normalized or None

    def _now_utc(self) -> datetime:
        return datetime.now(UTC)

    async def _create_proposal_case(
        self,
        message: discord.Message,
        verdict: AIVerdict,
        *,
        escalated_with_context: bool,
        action: str,
    ) -> None:
        case = self._make_case(message, verdict, escalated_with_context=escalated_with_context, action=action)
        await asyncio.to_thread(self._insert_case_sync, case)

        review_message_id: int | None = None
        review_channel = await self._resolve_text_channel(self.mod_review_channel_id)
        if review_channel is not None:
            embed = self._build_review_embed(case, status_text="Offen")
            view = ModerationProposalView(self, case.case_id)
            try:
                review_message = await review_channel.send(embed=embed, view=view)
                review_message_id = review_message.id
                try:
                    self.bot.add_view(view, message_id=review_message.id)
                except Exception as exc:
                    log.debug(
                        "Konnte Proposal-View fuer Review-Message %s nicht registrieren: %s",
                        review_message.id,
                        exc,
                    )
            except discord.HTTPException as exc:
                log.warning("Konnte Moderationsvorschlag fuer Case %s nicht posten: %s", case.case_id, exc)

        log_message_id = await self._post_action_log(case, action=action)
        await asyncio.to_thread(
            self._update_case_message_refs_sync,
            case.case_id,
            review_message_id,
            log_message_id,
        )

    async def _auto_delete_case(
        self,
        message: discord.Message,
        verdict: AIVerdict,
        *,
        escalated_with_context: bool,
    ) -> None:
        case = self._make_case(
            message,
            verdict,
            escalated_with_context=escalated_with_context,
            action="auto_delete",
        )
        await asyncio.to_thread(self._insert_case_sync, case)

        delete_ok, delete_note = await self._delete_message(message, case.case_id)
        timeout_ok, timeout_note = await self._timeout_member(
            message.guild,
            message.author,
            case.case_id,
            reason_label="AutoDelete",
        )
        action = "auto_delete" if delete_ok and timeout_ok else "auto_delete_failed"
        detail = "; ".join(note for note in (delete_note, timeout_note) if note)

        log_message_id = await self._post_action_log(case, action=action, detail=detail or None)
        await asyncio.to_thread(
            self._mark_auto_delete_result_sync,
            case.case_id,
            action,
            log_message_id,
        )

    def _make_case(
        self,
        message: discord.Message,
        verdict: AIVerdict,
        *,
        escalated_with_context: bool,
        action: str,
    ) -> ModerationCase:
        attachments = [
            {
                "url": attachment.url,
                "content_type": attachment.content_type or "",
                "filename": attachment.filename,
            }
            for attachment in message.attachments
        ]
        return ModerationCase(
            case_id=str(uuid.uuid4()),
            guild_id=message.guild.id,
            channel_id=message.channel.id,
            message_id=message.id,
            user_id=message.author.id,
            user_tag=str(message.author),
            original_content=message.content or "",
            attachments=attachments,
            ai_category=verdict.category,
            ai_confidence=verdict.confidence,
            ai_reason=verdict.reason,
            ai_raw_json=verdict.raw_json,
            escalated_with_context=escalated_with_context,
            action=action,
        )

    async def _fetch_context_lines(self, message: discord.Message, *, limit: int) -> list[str]:
        lines: list[str] = []
        try:
            async for previous in message.channel.history(limit=limit + 1, before=message):
                preview = _strip_mentions(previous.content)
                if not preview and previous.attachments:
                    preview = "[Anhang]"
                if not preview:
                    continue
                lines.append(f"{previous.author.display_name}: {_truncate(preview, 160)}")
        except discord.HTTPException as exc:
            log.debug("Konnte Kontext fuer Message %s nicht laden: %s", message.id, exc)
        lines.reverse()
        return lines[-limit:]

    def _extract_image_attachments(
        self, attachments: list[discord.Attachment]
    ) -> list[discord.Attachment]:
        images: list[discord.Attachment] = []
        for attachment in attachments:
            content_type = (attachment.content_type or "").lower()
            if not content_type.startswith("image/"):
                continue
            images.append(attachment)
            if len(images) >= self.max_images_per_check:
                break
        return images

    def _build_review_embed(self, case: ModerationCase, *, status_text: str) -> discord.Embed:
        title = f"AI Moderationsvorschlag: {_safe_title_fragment(case.ai_category)}"
        embed = discord.Embed(
            title=title,
            colour=discord.Colour.orange(),
            timestamp=discord.utils.utcnow(),
        )
        embed.add_field(name="Author", value=f"<@{case.user_id}> (`{case.user_tag}`)", inline=False)
        embed.add_field(
            name="Channel",
            value=f"<#{case.channel_id}> | [Jump]({case.jump_url})",
            inline=False,
        )
        embed.add_field(name="Kategorie", value=_safe_title_fragment(case.ai_category), inline=True)
        embed.add_field(name="Confidence", value=f"{case.ai_confidence:.2f}", inline=True)
        embed.add_field(
            name="AI-Reason",
            value=_truncate(case.ai_reason, DISCORD_FIELD_LIMIT),
            inline=False,
        )
        embed.add_field(
            name="Original",
            value=_safe_message_text(case.original_content, limit=DISCORD_FIELD_LIMIT),
            inline=False,
        )
        embed.add_field(name="Status", value=_truncate(status_text, DISCORD_FIELD_LIMIT), inline=False)
        embed.add_field(name="Case-ID", value=case.case_id, inline=False)
        self._apply_attachment_rendering(embed, case.attachments)
        return embed

    def _build_log_embed(
        self,
        case: ModerationCase,
        *,
        action: str,
        mod_user: discord.abc.User | None = None,
        deny_reason: str | None = None,
        detail: str | None = None,
    ) -> discord.Embed:
        title_map = {
            "auto_delete": f":rotating_light: Auto-Delete: {_safe_title_fragment(case.ai_category)} ({case.ai_confidence:.2f})",
            "auto_delete_failed": f":warning: Auto-Delete Failed: {_safe_title_fragment(case.ai_category)} ({case.ai_confidence:.2f})",
            "proposed": f":memo: Proposed: {_safe_title_fragment(case.ai_category)} ({case.ai_confidence:.2f})",
            "ragebait_escalated": ":memo: Ragebait Escalated",
            "accepted": f":white_check_mark: Accepted by {mod_user.mention if mod_user else 'Mod'}",
            "denied": f":x: Denied by {mod_user.mention if mod_user else 'Mod'}",
        }
        colour_map = {
            "auto_delete": discord.Colour.red(),
            "auto_delete_failed": discord.Colour.dark_red(),
            "proposed": discord.Colour.orange(),
            "ragebait_escalated": discord.Colour.orange(),
            "accepted": discord.Colour.green(),
            "denied": discord.Colour.light_grey(),
        }
        embed = discord.Embed(
            title=title_map.get(action, ":memo: AI Moderation"),
            colour=colour_map.get(action, discord.Colour.blurple()),
            timestamp=discord.utils.utcnow(),
        )
        embed.add_field(name="Author", value=f"<@{case.user_id}> (`{case.user_tag}`)", inline=False)
        embed.add_field(
            name="Channel",
            value=f"<#{case.channel_id}> | [Jump]({case.jump_url})",
            inline=False,
        )
        embed.add_field(name="Kategorie", value=_safe_title_fragment(case.ai_category), inline=True)
        embed.add_field(name="Confidence", value=f"{case.ai_confidence:.2f}", inline=True)
        embed.add_field(
            name="AI-Reason",
            value=_truncate(case.ai_reason, DISCORD_FIELD_LIMIT),
            inline=False,
        )
        embed.add_field(name="Aktion", value=action, inline=True)
        embed.add_field(name="Case-ID", value=case.case_id, inline=True)
        if mod_user is not None:
            embed.add_field(name="Mod", value=mod_user.mention, inline=True)
        if deny_reason:
            embed.add_field(
                name="Deny-Reason",
                value=_truncate(deny_reason, DISCORD_FIELD_LIMIT),
                inline=False,
            )
        if detail:
            embed.add_field(name="Detail", value=_truncate(detail, DISCORD_FIELD_LIMIT), inline=False)
        self._apply_attachment_rendering(embed, case.attachments)
        return embed

    def _apply_attachment_rendering(
        self, embed: discord.Embed, attachments: list[dict[str, str]]
    ) -> None:
        if not attachments:
            return
        image_urls = [
            attachment["url"]
            for attachment in attachments
            if str(attachment.get("content_type") or "").lower().startswith("image/")
        ][: self.max_images_per_check]
        other_urls = [
            f"[{_truncate(attachment.get('filename') or 'attachment', 80)}]({attachment.get('url')})"
            for attachment in attachments
            if not str(attachment.get("content_type") or "").lower().startswith("image/")
        ]
        if image_urls:
            embed.set_image(url=image_urls[0])
        if len(image_urls) > 1:
            embed.add_field(
                name="Weitere Bilder",
                value="\n".join(image_urls[1:]),
                inline=False,
            )
        if other_urls:
            embed.add_field(
                name="Attachments",
                value=_truncate("\n".join(other_urls), DISCORD_FIELD_LIMIT),
                inline=False,
            )

    async def _post_action_log(
        self,
        case: ModerationCase,
        *,
        action: str,
        mod_user: discord.abc.User | None = None,
        deny_reason: str | None = None,
        detail: str | None = None,
    ) -> int | None:
        channel = await self._resolve_text_channel(self.log_channel_id)
        if channel is None:
            return None
        embed = self._build_log_embed(
            case,
            action=action,
            mod_user=mod_user,
            deny_reason=deny_reason,
            detail=detail,
        )
        try:
            log_message = await channel.send(embed=embed)
            original = _safe_message_text(case.original_content, limit=DISCORD_MESSAGE_SAFE_LIMIT)
            content_message = f">>> {original}"
            if len(case.original_content or "") > DISCORD_MESSAGE_SAFE_LIMIT:
                content_message += "\nNachricht gekuerzt"
            await channel.send(content_message, allowed_mentions=discord.AllowedMentions.none())
            return log_message.id
        except discord.HTTPException as exc:
            log.warning("Konnte Log fuer Case %s nicht posten: %s", case.case_id, exc)
            return None

    async def _edit_review_message(self, case: ModerationCase, *, status_text: str) -> None:
        if case.mod_review_message_id is None:
            return
        channel = await self._resolve_text_channel(self.mod_review_channel_id)
        if channel is None:
            return
        try:
            message = await channel.fetch_message(case.mod_review_message_id)
        except discord.HTTPException:
            return

        embed = self._build_review_embed(case, status_text=status_text)
        try:
            await message.edit(embed=embed, view=self._build_disabled_view(case.case_id))
        except discord.HTTPException as exc:
            log.warning("Konnte Review-Message fuer Case %s nicht editieren: %s", case.case_id, exc)

    def _build_disabled_view(self, case_id: str) -> ModerationProposalView:
        view = ModerationProposalView(self, case_id)
        for item in view.children:
            inner = getattr(item, "item", None)
            if inner is not None:
                inner.disabled = True
        return view

    async def _resolve_text_channel(self, channel_id: int) -> discord.TextChannel | None:
        channel = self.bot.get_channel(channel_id)
        if isinstance(channel, discord.TextChannel):
            return channel
        try:
            fetched = await self.bot.fetch_channel(channel_id)
        except discord.HTTPException:
            return None
        return fetched if isinstance(fetched, discord.TextChannel) else None

    def _has_review_permission(self, interaction: discord.Interaction) -> bool:
        perms = getattr(interaction.user, "guild_permissions", None)
        return bool(perms and perms.manage_messages)

    async def _delete_case_message(self, case: ModerationCase) -> tuple[bool, str | None]:
        channel = self.bot.get_channel(case.channel_id)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(case.channel_id)
            except discord.HTTPException:
                return False, "Channel nicht gefunden"
        if not isinstance(channel, discord.TextChannel):
            return False, "Channel ist kein TextChannel"
        try:
            message = await channel.fetch_message(case.message_id)
        except discord.NotFound:
            return False, "Nachricht bereits geloescht"
        except discord.HTTPException:
            return False, "Nachricht nicht abrufbar"
        return await self._delete_message(message, case.case_id)

    async def _delete_message(
        self, message: discord.Message, case_id: str
    ) -> tuple[bool, str | None]:
        try:
            await message.delete(reason=f"AI-Moderator Case {case_id}")
            return True, None
        except discord.NotFound:
            return False, "Nachricht bereits geloescht"
        except discord.Forbidden:
            return False, "Keine Berechtigung zum Loeschen"
        except discord.HTTPException as exc:
            log.warning("Loeschen fehlgeschlagen fuer Message %s: %s", message.id, exc)
            return False, "Loeschen fehlgeschlagen"

    async def _timeout_case_member(
        self, case: ModerationCase, guild: discord.Guild | None
    ) -> tuple[bool, str | None]:
        if guild is None:
            return False, "Guild nicht verfuegbar"
        member = guild.get_member(case.user_id)
        if member is None:
            try:
                member = await guild.fetch_member(case.user_id)
            except discord.NotFound:
                return False, "Member nicht gefunden"
            except discord.HTTPException:
                return False, "Member nicht abrufbar"
        return await self._timeout_member(guild, member, case.case_id)

    async def _timeout_member(
        self,
        guild: discord.Guild,
        member: discord.Member,
        case_id: str,
        *,
        reason_label: str = "Accept",
    ) -> tuple[bool, str | None]:
        me = guild.me
        if me is None:
            try:
                me = await guild.fetch_member(self.bot.user.id)
            except discord.HTTPException:
                return False, "Bot-Member nicht aufloesbar"
        if not me.guild_permissions.moderate_members:
            return False, "Bot hat kein Moderate Members"
        try:
            await member.timeout(
                timedelta(minutes=self.timeout_minutes),
                reason=f"AI-Moderator {reason_label} {case_id}",
            )
            return True, None
        except discord.NotFound:
            return False, "Member nicht gefunden"
        except discord.Forbidden:
            return False, "Timeout nicht erlaubt"
        except discord.HTTPException as exc:
            log.warning("Timeout fehlgeschlagen fuer Member %s: %s", member.id, exc)
            return False, "Timeout fehlgeschlagen"

    def _connect_db(self) -> sqlite3.Connection:
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        connection = sqlite3.connect(self.db_path)
        connection.row_factory = sqlite3.Row
        return connection

    def _ensure_schema_sync(self) -> None:
        with self._connect_db() as connection:
            connection.executescript(SCHEMA_SQL)
            connection.commit()

    def _insert_case_sync(self, case: ModerationCase) -> None:
        with self._connect_db() as connection:
            connection.execute(
                """
                INSERT OR REPLACE INTO ai_moderation_cases (
                    case_id, guild_id, channel_id, message_id, user_id, user_tag,
                    original_content, attachments_json, ai_category, ai_confidence,
                    ai_reason, ai_raw_json, escalated_with_context, action,
                    mod_id, mod_action_at, mod_deny_reason, mod_review_message_id,
                    log_message_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    case.case_id,
                    case.guild_id,
                    case.channel_id,
                    case.message_id,
                    case.user_id,
                    case.user_tag,
                    case.original_content,
                    json.dumps(case.attachments, ensure_ascii=False),
                    case.ai_category,
                    case.ai_confidence,
                    case.ai_reason,
                    case.ai_raw_json,
                    int(case.escalated_with_context),
                    case.action,
                    case.mod_id,
                    case.mod_action_at,
                    case.mod_deny_reason,
                    case.mod_review_message_id,
                    case.log_message_id,
                ),
            )
            connection.commit()

    def _fetch_case_sync(self, case_id: str) -> ModerationCase | None:
        with self._connect_db() as connection:
            row = connection.execute(
                "SELECT * FROM ai_moderation_cases WHERE case_id = ?",
                (case_id,),
            ).fetchone()
        return ModerationCase.from_row(row) if row else None

    def _fetch_open_proposals_sync(self) -> list[ModerationCase]:
        with self._connect_db() as connection:
            rows = connection.execute(
                """
                SELECT * FROM ai_moderation_cases
                WHERE action IN ('proposed', 'ragebait_escalated')
                  AND mod_review_message_id IS NOT NULL
                """
            ).fetchall()
        return [ModerationCase.from_row(row) for row in rows]

    def _update_case_message_refs_sync(
        self,
        case_id: str,
        review_message_id: int | None,
        log_message_id: int | None,
    ) -> None:
        with self._connect_db() as connection:
            connection.execute(
                """
                UPDATE ai_moderation_cases
                SET mod_review_message_id = COALESCE(?, mod_review_message_id),
                    log_message_id = COALESCE(?, log_message_id)
                WHERE case_id = ?
                """,
                (review_message_id, log_message_id, case_id),
            )
            connection.commit()

    def _mark_auto_delete_result_sync(
        self, case_id: str, action: str, log_message_id: int | None
    ) -> None:
        with self._connect_db() as connection:
            connection.execute(
                """
                UPDATE ai_moderation_cases
                SET action = ?, log_message_id = COALESCE(?, log_message_id)
                WHERE case_id = ?
                """,
                (action, log_message_id, case_id),
            )
            connection.commit()

    def _mark_case_accepted_sync(self, case_id: str, mod_id: int) -> None:
        with self._connect_db() as connection:
            connection.execute(
                """
                UPDATE ai_moderation_cases
                SET action = 'accepted',
                    mod_id = ?,
                    mod_action_at = CURRENT_TIMESTAMP
                WHERE case_id = ?
                """,
                (mod_id, case_id),
            )
            connection.commit()

    def _mark_case_denied_sync(self, case_id: str, mod_id: int, deny_reason: str) -> None:
        with self._connect_db() as connection:
            connection.execute(
                """
                UPDATE ai_moderation_cases
                SET action = 'denied',
                    mod_id = ?,
                    mod_action_at = CURRENT_TIMESTAMP,
                    mod_deny_reason = ?
                WHERE case_id = ?
                """,
                (mod_id, deny_reason, case_id),
            )
            connection.commit()

    def _update_log_message_id_sync(self, case_id: str, log_message_id: int) -> None:
        with self._connect_db() as connection:
            connection.execute(
                "UPDATE ai_moderation_cases SET log_message_id = ? WHERE case_id = ?",
                (log_message_id, case_id),
            )
            connection.commit()

    def _insert_ragebait_hit_sync(
        self,
        guild_id: int,
        user_id: int,
        message_id: int,
        channel_id: int,
        content_preview: str,
    ) -> tuple[int, list[sqlite3.Row]]:
        with self._connect_db() as connection:
            connection.execute(
                """
                INSERT INTO ai_moderation_ragebait_hits (
                    guild_id, user_id, message_id, channel_id, content_preview
                ) VALUES (?, ?, ?, ?, ?)
                """,
                (guild_id, user_id, message_id, channel_id, content_preview),
            )
            count = int(
                connection.execute(
                    """
                    SELECT COUNT(*) FROM ai_moderation_ragebait_hits
                    WHERE user_id = ?
                      AND guild_id = ?
                      AND created_at > datetime('now', ?)
                    """,
                    (user_id, guild_id, f"-{self.ragebait_window_minutes} minutes"),
                ).fetchone()[0]
            )
            hits = connection.execute(
                """
                SELECT guild_id, user_id, message_id, channel_id, content_preview
                FROM ai_moderation_ragebait_hits
                WHERE user_id = ?
                  AND guild_id = ?
                  AND created_at > datetime('now', ?)
                ORDER BY created_at DESC
                LIMIT ?
                """,
                (
                    user_id,
                    guild_id,
                    f"-{self.ragebait_window_minutes} minutes",
                    self.ragebait_escalate_threshold,
                ),
            ).fetchall()
            connection.commit()
        hits.reverse()
        return count, hits

    def _cleanup_ragebait_hits_sync(self) -> int:
        with self._connect_db() as connection:
            cursor = connection.execute(
                """
                DELETE FROM ai_moderation_ragebait_hits
                WHERE created_at < datetime('now', '-24 hours')
                """
            )
            connection.commit()
            return cursor.rowcount


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(AIModeratorCog(bot))
