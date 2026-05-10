"""Voice-Reaction-Sales-DM-Cog.

Pollt die Postgres-Tabelle `twitch_partner_outreach_conversations` des
Twitch-Bots auf Konversationen, bei denen der Conversation-Brain
`should_notify_human=true` gesetzt hat (Spalte `human_notify_pending_at`
ist gesetzt, `human_notify_sent_at` aber noch leer).

Für jede gefundene Konversation wird dem Bot-Owner eine DM mit einem
kompakten Embed geschickt (Streamer-Login, Stance/Confidence, Reasoning,
Konversations-Auszug). Danach wird `human_notify_sent_at` gesetzt, sodass
keine doppelten DMs entstehen.

Der DSN wird aus `TWITCH_ANALYTICS_DSN` gelesen — derselbe Wert, den der
Twitch-Bot nutzt.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
from datetime import datetime
from typing import Any, Iterable

import discord
from discord.ext import commands, tasks

from service.config import settings

log = logging.getLogger(__name__)

POLL_INTERVAL_SECONDS = 30
DSN_ENV = "TWITCH_ANALYTICS_DSN"
EMBED_COLOR = 0x5865F2
MAX_HISTORY_LINES = 8
MAX_HISTORY_LINE_CHARS = 200


def _is_enabled() -> bool:
    return str(os.getenv("VOICE_REACTION_DM_ENABLED") or "").strip().lower() in {
        "1",
        "true",
        "yes",
        "y",
        "on",
    }


def _resolve_dsn() -> str | None:
    raw = os.getenv(DSN_ENV) or ""
    raw = raw.strip()
    return raw or None


class VoiceReactionDM(commands.Cog):
    """Schickt dem Bot-Owner DMs, sobald der Twitch-Bot einen Sales-Lead meldet."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self._dsn = _resolve_dsn()
        self._psycopg = None  # lazy
        self._owner: discord.User | None = None
        self._poll_failed_logged = False
        self._poll_tick = 0
        self._last_pending_count: int | None = None

    async def cog_load(self) -> None:
        if not _is_enabled():
            log.info("VoiceReactionDM: deaktiviert (VOICE_REACTION_DM_ENABLED leer)")
            return
        if not self._dsn:
            log.warning(
                "VoiceReactionDM: kein %s gesetzt — Polling startet nicht", DSN_ENV
            )
            return
        try:
            import psycopg  # type: ignore
            from psycopg.rows import dict_row  # type: ignore
        except Exception:
            log.warning(
                "VoiceReactionDM: psycopg nicht verfügbar, Cog inaktiv", exc_info=True
            )
            return
        self._psycopg = psycopg
        self._dict_row = dict_row
        self._poll.start()
        log.info("VoiceReactionDM: aktiv, Polling alle %ds", POLL_INTERVAL_SECONDS)

    async def cog_unload(self) -> None:
        try:
            self._poll.cancel()
        except Exception:
            pass  # Poll cancellation is best-effort during unload.

    # ------------------------------------------------------------------
    # Poll-Loop
    # ------------------------------------------------------------------
    async def _resolve_owner_id(self) -> int:
        owner_id = int(getattr(settings, "owner_id", 0) or 0)
        if owner_id > 0:
            return owner_id
        env_id = (os.getenv("VOICE_REACTION_DM_OWNER_ID") or "").strip()
        if env_id.isdigit():
            return int(env_id)
        try:
            info = await self.bot.application_info()
        except Exception:
            return 0
        owner = getattr(info, "owner", None)
        return int(getattr(owner, "id", 0) or 0)

    @tasks.loop(seconds=POLL_INTERVAL_SECONDS)
    async def _poll(self) -> None:
        self._poll_tick += 1
        owner_id = await self._resolve_owner_id()
        if owner_id <= 0:
            if self._poll_tick == 1 or self._poll_tick % 60 == 0:
                log.warning(
                    "VoiceReactionDM: owner_id nicht gesetzt (settings.owner_id=%r, auch keine application_info-owner) — DMs deaktiviert",
                    getattr(settings, "owner_id", None),
                )
            return
        try:
            pending = await asyncio.to_thread(self._fetch_pending)
        except Exception:
            if not self._poll_failed_logged:
                log.exception("VoiceReactionDM: Poll-Query fehlgeschlagen")
                self._poll_failed_logged = True
            return
        self._poll_failed_logged = False

        count = len(pending)
        if self._poll_tick == 1 or count != self._last_pending_count:
            log.info(
                "VoiceReactionDM: Poll-Tick #%d, owner_id=%d, pending=%d",
                self._poll_tick,
                owner_id,
                count,
            )
        self._last_pending_count = count

        if not pending:
            return

        owner = await self._ensure_owner(owner_id)
        if owner is None:
            return

        for row in pending:
            login = str(row.get("streamer_login") or "")
            if not login:
                continue
            try:
                embed = self._build_embed(row)
                await owner.send(embed=embed)
                await asyncio.to_thread(self._mark_sent, login)
                log.info("VoiceReactionDM: Lead-DM gesendet für %s", login)
            except Exception:
                log.exception("VoiceReactionDM: DM-Versand für %s fehlgeschlagen", login)

    @_poll.before_loop
    async def _before_poll(self) -> None:
        await self.bot.wait_until_ready()

    # ------------------------------------------------------------------
    # DB
    # ------------------------------------------------------------------
    def _fetch_pending(self) -> list[dict[str, Any]]:
        assert self._psycopg is not None
        with self._psycopg.connect(self._dsn, autocommit=True) as conn:
            with conn.cursor(row_factory=self._dict_row) as cur:
                cur.execute(
                    """
                    SELECT streamer_login,
                           streamer_user_id,
                           source,
                           state,
                           messages_json,
                           last_stance,
                           last_confidence,
                           human_notify_pending_at,
                           human_notify_sent_at
                      FROM twitch_partner_outreach_conversations
                     WHERE human_notify_pending_at IS NOT NULL
                       AND human_notify_sent_at IS NULL
                     ORDER BY human_notify_pending_at ASC
                     LIMIT 20
                    """
                )
                return list(cur.fetchall() or [])

    def _mark_sent(self, login: str) -> None:
        assert self._psycopg is not None
        with self._psycopg.connect(self._dsn, autocommit=True) as conn:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    UPDATE twitch_partner_outreach_conversations
                       SET human_notify_sent_at = NOW()
                     WHERE streamer_login = %s
                       AND human_notify_sent_at IS NULL
                    """,
                    (login.lower(),),
                )

    # ------------------------------------------------------------------
    # Owner / Embed
    # ------------------------------------------------------------------
    async def _ensure_owner(self, owner_id: int) -> discord.User | None:
        if self._owner is not None and getattr(self._owner, "id", 0) == owner_id:
            return self._owner
        try:
            user = self.bot.get_user(owner_id) or await self.bot.fetch_user(owner_id)
        except Exception:
            log.exception("VoiceReactionDM: konnte Owner %s nicht laden", owner_id)
            return None
        self._owner = user
        return user

    def _build_embed(self, row: dict[str, Any]) -> discord.Embed:
        login = str(row.get("streamer_login") or "")
        stance = str(row.get("last_stance") or "—")
        confidence = row.get("last_confidence")
        confidence_str = (
            f"{float(confidence):.2f}" if isinstance(confidence, (int, float)) else "—"
        )
        source = str(row.get("source") or "—")
        history = _coerce_history(row.get("messages_json"))
        description = _format_history(history) or "_keine relevante History_"
        if len(description) > 3500:
            description = description[:3497].rstrip() + "…"

        embed = discord.Embed(
            title=f"Sales-Lead: {login}",
            url=f"https://twitch.tv/{login}",
            description=description,
            color=EMBED_COLOR,
            timestamp=datetime.utcnow(),
        )
        embed.add_field(name="Stance", value=stance, inline=True)
        embed.add_field(name="Confidence", value=confidence_str, inline=True)
        embed.add_field(name="Trigger", value=source, inline=True)
        return embed


def _coerce_history(value: Any) -> list[dict[str, Any]]:
    if isinstance(value, list):
        return [entry for entry in value if isinstance(entry, dict)]
    if isinstance(value, (bytes, bytearray)):
        try:
            value = value.decode("utf-8", "ignore")
        except Exception:
            return []
    if isinstance(value, str):
        try:
            parsed = json.loads(value)
        except Exception:
            return []
        if isinstance(parsed, list):
            return [entry for entry in parsed if isinstance(entry, dict)]
    return []


def _format_history(history: Iterable[dict[str, Any]]) -> str:
    lines: list[str] = []
    seq = list(history)[-MAX_HISTORY_LINES:]
    for entry in seq:
        role = str(entry.get("role") or "system")
        text = str(entry.get("text") or "").strip()
        if not text:
            continue
        prefix = {
            "voice": "voice",
            "streamer_chat": "streamer",
            "bot_chat": "bot",
        }.get(role, role)
        if len(text) > MAX_HISTORY_LINE_CHARS:
            text = text[: MAX_HISTORY_LINE_CHARS - 3].rstrip() + "…"
        lines.append(f"**{prefix}**: {text}")
    return "\n".join(lines)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(VoiceReactionDM(bot))
