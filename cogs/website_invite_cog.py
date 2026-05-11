"""WebsiteInviteCog — verwaltet Permanent-Invites für Website-CTAs.

Was er tut:
- Beim Cog-Load: prüft ob pro Website-Subseite ein gespeicherter Invite-Code
  noch live ist. Falls nicht (oder nie erstellt): erstellt einen neuen,
  permanenten Invite (max_age=0, max_uses=0) im konfigurierten Welcome-Channel
  und persistiert den Code in central_db (key-value).
- /website-invite (Owner): Zeigt aktuellen Invite-Status (Code, URL, uses) je Subseite.
- /website-invite-recreate (Owner): Erstellt einen neuen Code für genau eine Subseite.
- /join-quellen (Owner): Aggregiert member_events der letzten N Tage und
  zeigt pro Quelle (Website / Vanity / Personal-Invite / etc.) wie viele
  Joins reinkamen.

Tracking-Logik basiert auf dem bereits existierenden user_activity_analyzer:
der speichert pro Member-Join `metadata.invite_code`. Wir labeln hier
zur Query-Zeit passende Website-Codes als "Website: <Subseite>".

Env-Override: WEBSITE_INVITE_CHANNEL_ID (default: RULES_CHANNEL_ID).
"""

from __future__ import annotations

import json
import logging
import os
from datetime import datetime, timedelta
from typing import TYPE_CHECKING

import discord
from discord import app_commands
from discord.ext import commands

from service import db as central_db

if TYPE_CHECKING:
    pass

log = logging.getLogger(__name__)

# Welcome-Channel (gleicher wie rules_channel.py default) — kann via Env überschrieben werden
DEFAULT_WELCOME_CHANNEL_ID = 1315684135175716975

KV_NAMESPACE = "website_invites"
KV_KEY_MAIN = "main"  # Speichert JSON: {"code": "...", "channel_id": ..., "created_at": "..."}

WEBSITE_SOURCE_LABEL = "Website"
WEBSITE_SUBPAGES: list[tuple[str, str]] = [
    ("landing", "Landing"),
    ("streamer", "Streamer"),
    ("mitspieler", "Mitspieler"),
    ("coaching", "Coaching"),
    ("helden", "Helden"),
    ("guides", "Guides"),
]


def _welcome_channel_id() -> int:
    raw = os.getenv("WEBSITE_INVITE_CHANNEL_ID", "").strip()
    if raw:
        try:
            return int(raw)
        except ValueError:
            log.warning("WEBSITE_INVITE_CHANNEL_ID=%r ist keine Zahl, fallback auf default", raw)
    return DEFAULT_WELCOME_CHANNEL_ID


def _is_owner_or_admin(interaction: discord.Interaction) -> bool:
    if interaction.user.id == getattr(interaction.guild, "owner_id", 0):
        return True
    perms = getattr(interaction.user, "guild_permissions", None)
    return bool(perms and perms.administrator)


def _subpage_label(slug: str) -> str:
    for subpage_slug, label in WEBSITE_SUBPAGES:
        if subpage_slug == slug:
            return label
    return slug


def _load_raw_invite(key: str) -> dict | None:
    raw = central_db.get_kv(KV_NAMESPACE, key)
    if not raw:
        return None
    try:
        data = json.loads(raw)
    except (TypeError, ValueError):
        return None
    if not isinstance(data, dict):
        return None
    code = str(data.get("code") or "").strip()
    if not code:
        return None
    return data


def _load_invite_for_subpage(slug: str) -> dict | None:
    stored = _load_raw_invite(slug)
    if stored is not None:
        return stored
    if slug == "landing":
        return _load_raw_invite(KV_KEY_MAIN)
    return None


def _save_invite_for_subpage(slug: str, code: str, channel_id: int) -> None:
    payload = {
        "code": code,
        "channel_id": channel_id,
        "created_at": datetime.utcnow().isoformat(timespec="seconds"),
    }
    payload_json = json.dumps(payload, separators=(",", ":"))
    central_db.set_kv(KV_NAMESPACE, slug, payload_json)
    if slug == "landing":
        central_db.set_kv(KV_NAMESPACE, KV_KEY_MAIN, payload_json)


def _load_all_subpage_invites() -> list[tuple[str, str, dict | None]]:
    return [(slug, label, _load_invite_for_subpage(slug)) for slug, label in WEBSITE_SUBPAGES]


async def _create_permanent_invite(channel: discord.abc.GuildChannel) -> discord.Invite:
    """Erstellt einen permanenten, niemals ablaufenden Invite."""
    return await channel.create_invite(
        max_age=0,
        max_uses=0,
        unique=True,
        reason="WebsiteInviteCog: permanenter Invite für Website-CTAs",
    )


class WebsiteInviteCog(commands.Cog):
    """Verwaltet dedizierte Website-Invites und liefert Join-Quellen-Stats."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot

    async def cog_load(self) -> None:
        # Bot ist hier evtl. noch nicht ready — lazy mit Listener arbeiten
        self.bot.loop.create_task(self._ensure_invite_when_ready())

    async def _ensure_invite_when_ready(self) -> None:
        try:
            await self.bot.wait_until_ready()
            await self._ensure_invite()
        except Exception as exc:
            log.error("WebsiteInviteCog: Initialisierung fehlgeschlagen: %s", exc, exc_info=True)

    async def _resolve_target_guild(self) -> discord.Guild | None:
        # Im Code-Pfad gibt es nur eine Guild — primary-Guild aus settings.guild_id
        try:
            from service.config import settings

            guild_id = settings.guild_id
        except Exception:
            guild_id = 0

        if guild_id:
            guild = self.bot.get_guild(int(guild_id))
            if guild:
                return guild

        # Fallback: erste verfügbare Guild
        guilds = list(getattr(self.bot, "guilds", []))
        return guilds[0] if guilds else None

    async def _resolve_invite_channel(self, guild: discord.Guild) -> discord.abc.GuildChannel | None:
        channel_id = _welcome_channel_id()
        channel = guild.get_channel(channel_id)
        if channel:
            return channel
        log.warning(
            "WebsiteInviteCog: Channel %s nicht gefunden in Guild %s",
            channel_id,
            guild.id,
        )
        return None

    async def _verify_invite_alive(self, guild: discord.Guild, code: str) -> discord.Invite | None:
        try:
            invites = await guild.invites()
        except discord.Forbidden:
            log.warning("WebsiteInviteCog: Bot fehlt MANAGE_GUILD — kann Invites nicht lesen")
            return None
        except discord.HTTPException as exc:
            log.warning("WebsiteInviteCog: invites() fehlgeschlagen: %s", exc)
            return None
        for inv in invites:
            if str(getattr(inv, "code", "")) == code:
                return inv
        return None

    async def _ensure_invite(self) -> dict[str, dict[str, object]] | None:
        guild = await self._resolve_target_guild()
        if guild is None:
            log.warning("WebsiteInviteCog: keine Guild verfügbar — skip")
            return None

        channel = await self._resolve_invite_channel(guild)
        if channel is None:
            return None

        resolved: dict[str, dict[str, object]] = {}
        for slug, label in WEBSITE_SUBPAGES:
            stored = _load_invite_for_subpage(slug)
            if stored:
                existing = await self._verify_invite_alive(guild, stored["code"])
                if existing is not None:
                    if slug == "landing" and central_db.get_kv(KV_NAMESPACE, "landing") is None:
                        _save_invite_for_subpage(slug, stored["code"], int(stored["channel_id"]))
                    log.info(
                        "WebsiteInviteCog: existierender Invite ok — subpage=%s code=%s uses=%s",
                        slug,
                        stored["code"],
                        getattr(existing, "uses", 0),
                    )
                    resolved[slug] = stored
                    continue
                log.info(
                    "WebsiteInviteCog: gespeicherter Code %s fuer %s nicht mehr vorhanden — erstelle neu",
                    stored["code"],
                    slug,
                )

            try:
                invite = await _create_permanent_invite(channel)
            except discord.Forbidden:
                log.warning(
                    "WebsiteInviteCog: Bot darf in Channel %s keinen Invite erstellen",
                    channel.id,
                )
                return None
            except discord.HTTPException as exc:
                log.warning("WebsiteInviteCog: create_invite fehlgeschlagen: %s", exc)
                return None

            code = str(invite.code)
            _save_invite_for_subpage(slug, code, channel.id)
            stored = _load_invite_for_subpage(slug) or {"code": code, "channel_id": channel.id}
            resolved[slug] = stored
            log.info(
                "WebsiteInviteCog: neuer Permanent-Invite erstellt — subpage=%s label=%s code=%s channel=%s",
                slug,
                label,
                code,
                channel.id,
            )

        return resolved

    # ── Slash-Commands ────────────────────────────────────────────────────

    @app_commands.command(
        name="website-invite",
        description="Zeigt den aktuellen Website-Invite-Code und seine Nutzung.",
    )
    @app_commands.guild_only()
    async def cmd_show(self, interaction: discord.Interaction) -> None:
        if not _is_owner_or_admin(interaction):
            await interaction.response.send_message("❌ Nur für Server-Owner/Admins.", ephemeral=True)
            return

        await interaction.response.defer(ephemeral=True)
        stored_map = {slug: stored for slug, _, stored in _load_all_subpage_invites() if stored}
        if len(stored_map) < len(WEBSITE_SUBPAGES):
            ensured = await self._ensure_invite()
            if ensured:
                stored_map = ensured
        if not stored_map:
            await interaction.followup.send(
                "⚠️ Kein Website-Invite verfügbar. Prüfe Logs und Bot-Permissions (MANAGE_GUILD).",
                ephemeral=True,
            )
            return

        guild = interaction.guild
        embed = discord.Embed(
            title="Website-Invites",
            description="Aktuelle Invite-Codes pro Website-Subseite",
            color=0x40C4FF,
        )
        footer_parts: list[str] = []
        for slug, label in WEBSITE_SUBPAGES:
            stored = stored_map.get(slug)
            if not stored:
                embed.add_field(name=label, value="⚠️ Nicht konfiguriert", inline=False)
                continue

            code = str(stored["code"])
            invite_url = f"https://discord.gg/{code}"
            uses_text = "—"
            if guild is not None:
                existing = await self._verify_invite_alive(guild, code)
                if existing is not None:
                    uses_text = str(getattr(existing, "uses", "—") or 0)

            channel_id = stored.get("channel_id")
            channel_mention = f"<#{channel_id}>" if channel_id else "—"
            embed.add_field(
                name=label,
                value=(
                    f"**URL:** {invite_url}\n"
                    f"**Code:** `{code}`\n"
                    f"**Channel:** {channel_mention}\n"
                    f"**Joins:** {uses_text}"
                ),
                inline=False,
            )
            if stored.get("created_at"):
                footer_parts.append(f"{label}: {stored['created_at']}")

        if footer_parts:
            embed.set_footer(text=" | ".join(footer_parts[:3]))
        await interaction.followup.send(embed=embed, ephemeral=True)

    @app_commands.command(
        name="website-invite-recreate",
        description="Erstellt einen neuen Website-Invite-Code fuer eine Subseite.",
    )
    @app_commands.describe(subseite="Welche Website-Subseite soll einen neuen Code bekommen?")
    @app_commands.choices(
        subseite=[
            app_commands.Choice(name=label, value=slug) for slug, label in WEBSITE_SUBPAGES
        ]
    )
    @app_commands.guild_only()
    async def cmd_recreate(
        self,
        interaction: discord.Interaction,
        subseite: app_commands.Choice[str],
    ) -> None:
        if not _is_owner_or_admin(interaction):
            await interaction.response.send_message("❌ Nur für Server-Owner/Admins.", ephemeral=True)
            return

        await interaction.response.defer(ephemeral=True)
        guild = interaction.guild
        if guild is None:
            await interaction.followup.send("❌ Nur im Server nutzbar.", ephemeral=True)
            return

        slug = subseite.value
        label = _subpage_label(slug)

        stored = _load_invite_for_subpage(slug)
        if stored:
            old = await self._verify_invite_alive(guild, stored["code"])
            if old is not None:
                try:
                    await old.delete(reason="WebsiteInviteCog: Recreate")
                except discord.HTTPException as exc:
                    log.warning("WebsiteInviteCog: delete alter Invite fehlgeschlagen: %s", exc)

        channel = await self._resolve_invite_channel(guild)
        if channel is None:
            await interaction.followup.send(
                "❌ Welcome-Channel nicht gefunden. Setze WEBSITE_INVITE_CHANNEL_ID in der Env.",
                ephemeral=True,
            )
            return

        try:
            invite = await _create_permanent_invite(channel)
        except discord.HTTPException as exc:
            await interaction.followup.send(f"❌ Invite-Erstellung fehlgeschlagen: {exc}", ephemeral=True)
            return

        code = str(invite.code)
        _save_invite_for_subpage(slug, code, channel.id)
        await interaction.followup.send(
            f"✅ Neuer Website-Invite fuer {label}: https://discord.gg/{code}",
            ephemeral=True,
        )

    @app_commands.command(
        name="join-quellen",
        description="Zeigt Joins der letzten N Tage gruppiert nach Quelle.",
    )
    @app_commands.describe(tage="Anzahl Tage zurück (default 30, max 365)")
    @app_commands.guild_only()
    async def cmd_join_sources(self, interaction: discord.Interaction, tage: int = 30) -> None:
        if not _is_owner_or_admin(interaction):
            await interaction.response.send_message("❌ Nur für Server-Owner/Admins.", ephemeral=True)
            return

        tage = max(1, min(int(tage), 365))
        await interaction.response.defer(ephemeral=True)

        guild = interaction.guild
        guild_id = guild.id if guild else None
        since_iso = (datetime.utcnow() - timedelta(days=tage)).strftime("%Y-%m-%d %H:%M:%S")

        rows = central_db.query_all(
            """
            SELECT metadata
            FROM member_events
            WHERE event_type = 'join'
              AND timestamp >= ?
              AND (? IS NULL OR guild_id = ?)
            """,
            (since_iso, guild_id, guild_id),
        )

        website_codes: dict[str, str] = {}
        website_details: list[str] = []
        for slug, label, stored in _load_all_subpage_invites():
            if not stored:
                continue
            code = str(stored.get("code") or "").strip()
            if not code:
                continue
            website_codes[code.lower()] = label
            website_details.append(f"`{code}` → {label}")

        # Aggregation
        buckets: dict[str, int] = {}
        total = 0
        for row in rows:
            total += 1
            meta_raw = row["metadata"] if isinstance(row, dict) or hasattr(row, "keys") else None
            try:
                meta = json.loads(meta_raw) if meta_raw else {}
            except (TypeError, ValueError):
                meta = {}

            invite_code = str(meta.get("invite_code") or "").strip()
            kind = str(meta.get("join_source_kind") or "").strip()
            label_existing = str(meta.get("join_source_label") or "").strip()

            website_label = website_codes.get(invite_code.lower()) if invite_code else None
            if website_label:
                key = f"{WEBSITE_SOURCE_LABEL}: {website_label}"
            elif kind == "vanity":
                key = "Vanity-Link (Discord-Listings)"
            elif kind == "twitch_streamer":
                login = meta.get("twitch_streamer_login")
                key = f"Twitch: {login}" if login else "Twitch-Streamer"
            elif kind == "bot_invite":
                key = "Bot-Invite"
            elif kind == "invite_link":
                inviter = meta.get("inviter_name") or "?"
                key = f"Persönlicher Invite ({inviter})"
            elif kind == "server_discovery":
                key = "Server entdecken"
            else:
                key = label_existing or "Unbekannt"

            buckets[key] = buckets.get(key, 0) + 1

        if total == 0:
            await interaction.followup.send(
                f"Keine Join-Events in den letzten {tage} Tagen gefunden.",
                ephemeral=True,
            )
            return

        sorted_buckets = sorted(buckets.items(), key=lambda kv: (-kv[1], kv[0]))
        max_show = 12
        lines = []
        for key, count in sorted_buckets[:max_show]:
            pct = count / total * 100
            bar_units = max(1, round(pct / 5))  # 1 Block = 5%
            bar = "▰" * bar_units + "▱" * max(0, 4 - bar_units)
            lines.append(f"`{count:>4}` ({pct:5.1f}%) {bar}  {key}")
        if len(sorted_buckets) > max_show:
            rest = sum(c for _, c in sorted_buckets[max_show:])
            lines.append(f"`{rest:>4}` (rest) — {len(sorted_buckets) - max_show} weitere Quellen")

        embed = discord.Embed(
            title=f"Join-Quellen — letzte {tage} Tage",
            description="\n".join(lines),
            color=0x40C4FF,
        )
        embed.set_footer(text=f"Total: {total} Joins")
        if website_details:
            embed.add_field(
                name="Website-Codes",
                value="\n".join(website_details),
                inline=False,
            )
        else:
            embed.add_field(
                name="Website-Codes",
                value="⚠️ Noch nicht konfiguriert — `/website-invite` aufrufen.",
                inline=False,
            )
        await interaction.followup.send(embed=embed, ephemeral=True)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(WebsiteInviteCog(bot))
