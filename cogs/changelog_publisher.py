from __future__ import annotations

import datetime
import logging
import os
from pathlib import Path
from typing import TYPE_CHECKING, Any

import aiohttp
import discord
from aiohttp import web
from discord import app_commands
from discord.ext import commands

if TYPE_CHECKING:
    pass

log = logging.getLogger(__name__)

DEV_UPDATES_CHANNEL_ID = 1492910851483504821
TWITCH_BOT_CHANNEL_ID = 1318329964713611385
SERVER_ALERT_CHANNEL_ID = 1374364800817303632  # Admin-Channel (Decision Logs)
SERVER_ALERT_PING_USER_ID = 662995601738170389
CHANGELOG_API_TOKEN = os.getenv("CHANGELOG_API_TOKEN", "changeme-local")
CHANGELOG_API_PORT = int(os.getenv("CHANGELOG_API_PORT", "8899"))
MAX_FILE_BYTES = 24 * 1024 * 1024

VALID_TARGETS = {"all", "twitch"}

ALERT_LEVELS = {
    "warn": ("⚠️", 0xFFA500),
    "crit": ("🚨", 0xED4245),
    "ok": ("✅", 0x57F287),
}

TWITCH_INTERNAL_API_BASE_URL = "http://127.0.0.1:8776"
SPAM_LEARNING_PATH = "/internal/twitch/v1/spam-learning"
INTERNAL_TOKEN_ENV_NAMES = (
    "TWITCH_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
)


def _resolve_internal_token() -> str:
    for env_name in INTERNAL_TOKEN_ENV_NAMES:
        token = (os.getenv(env_name) or "").strip()
        if token:
            return token
    return ""


def _trim_text(value: Any, limit: int) -> str:
    return str(value or "").replace("\r", " ").replace("\n", " ").strip()[:limit]


def _parse_spam_learning(raw: Any) -> dict[str, str] | None:
    if not isinstance(raw, dict):
        return None
    pattern = _trim_text(raw.get("pattern"), 200)
    if len(pattern) < 4:
        return None
    pattern_type = _trim_text(raw.get("pattern_type") or "phrase", 20)
    if pattern_type not in {"phrase", "fragment"}:
        pattern_type = "fragment"
    return {
        "pattern": pattern,
        "pattern_type": pattern_type,
        "source_message": _trim_text(raw.get("source_message"), 500),
        "source_channel": _trim_text(raw.get("source_channel"), 100),
        "reason": _trim_text(raw.get("reason"), 200),
    }


async def post_spam_learning(payload: dict[str, str], verdict: str) -> bool:
    token = _resolve_internal_token()
    if not token:
        log.error("spam-learning: kein interner API-Token konfiguriert")
        return False

    body = {
        "verdict": verdict,
        "pattern": payload["pattern"],
        "patternType": payload["pattern_type"],
        "sourceMessage": payload["source_message"],
        "sourceChannel": payload["source_channel"],
        "reason": payload["reason"],
    }
    headers = {"X-Internal-Token": token, "Content-Type": "application/json"}
    try:
        async with aiohttp.ClientSession() as session:
            async with session.post(
                f"{TWITCH_INTERNAL_API_BASE_URL}{SPAM_LEARNING_PATH}",
                json=body,
                headers=headers,
            ) as response:
                ok = 200 <= response.status < 300
                if not ok:
                    log.warning("spam-learning: API status=%s", response.status)
                return ok
    except Exception:
        log.exception("spam-learning: HTTP-Aufruf fehlgeschlagen")
        return False


class _SpamLearningButton(discord.ui.Button):
    def __init__(
        self,
        parent: Any,
        *,
        verdict: str,
        label: str,
        style: discord.ButtonStyle,
    ) -> None:
        super().__init__(label=label, style=style)
        self._parent = parent
        self._verdict = verdict

    async def callback(self, interaction: discord.Interaction) -> None:
        await self._parent.handle_click(interaction, self._verdict)


class SpamLearningView(discord.ui.View):
    def __init__(self, payload: dict[str, str]) -> None:
        super().__init__(timeout=24 * 3600)
        self.payload = payload
        self.add_item(
            _SpamLearningButton(
                self,
                verdict="spam",
                label="Spam lernen",
                style=discord.ButtonStyle.danger,
            )
        )
        self.add_item(
            _SpamLearningButton(
                self,
                verdict="safe",
                label="Harmlos lernen",
                style=discord.ButtonStyle.success,
            )
        )

    @staticmethod
    def _allowed(interaction: discord.Interaction) -> bool:
        perms = getattr(getattr(interaction, "user", None), "guild_permissions", None)
        return bool(
            getattr(perms, "manage_messages", False)
            or getattr(perms, "manage_guild", False)
            or getattr(perms, "administrator", False)
        )

    async def handle_click(self, interaction: discord.Interaction, verdict: str) -> None:
        if not self._allowed(interaction):
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
            return

        await interaction.response.defer(ephemeral=True)
        ok = await post_spam_learning(self.payload, verdict)
        if not ok:
            await interaction.followup.send("Lernen fehlgeschlagen.", ephemeral=True)
            return

        for child in self.children:
            child.disabled = True
            if isinstance(child, discord.ui.Button):
                child.label = "Gelernt"
                child.style = discord.ButtonStyle.secondary
        if interaction.message is not None:
            try:
                await interaction.message.edit(view=self)
            except Exception:
                log.debug("spam-learning: Button-View konnte nicht editiert werden", exc_info=True)
        await interaction.followup.send("Gelernt.", ephemeral=True)


class ChangelogPublisher(commands.Cog):
    """Posts changelog entries as Discord embeds and exposes an internal HTTP API."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self._runner: web.AppRunner | None = None
        self._site: web.TCPSite | None = None

    async def cog_load(self) -> None:
        app = web.Application()
        app.router.add_post("/changelog", self._http_handler)
        app.router.add_post("/changelog/rich", self._rich_changelog_handler)
        app.router.add_post("/highlight-clips", self._highlight_handler)
        app.router.add_post("/discord/messages", self._fetch_messages_handler)
        app.router.add_post("/alert", self._alert_handler)
        self._runner = web.AppRunner(app)
        await self._runner.setup()
        self._site = web.TCPSite(self._runner, "127.0.0.1", CHANGELOG_API_PORT)
        await self._site.start()
        log.info("Changelog HTTP API listening on 127.0.0.1:%s", CHANGELOG_API_PORT)

    async def cog_unload(self) -> None:
        if self._runner:
            await self._runner.cleanup()
            log.info("Changelog HTTP API stopped")

    async def _build_embed(self, title: str, content: str, target: str) -> discord.Embed:
        embed = discord.Embed(
            title=f"📋 {title}",
            description=content,
            color=0x5865F2,
            timestamp=datetime.datetime.now(datetime.UTC),
        )
        embed.set_footer(text="Twitch Bot" if target == "twitch" else "Deadlock Bots")
        return embed

    async def _send_changelog(self, title: str, content: str, target: str) -> int:
        channel_id = TWITCH_BOT_CHANNEL_ID if target == "twitch" else DEV_UPDATES_CHANNEL_ID
        channel = self.bot.get_channel(channel_id)
        if channel is None:
            raise RuntimeError(f"Channel {channel_id} not found (bot may not be ready)")
        embed = await self._build_embed(title, content, target)
        await channel.send(embed=embed)
        log.info("Changelog posted to channel %s: %s", channel_id, title)
        return channel_id

    async def _rich_changelog_handler(self, request: web.Request) -> web.Response:
        """Mehrseitiger Changelog: postet mehrere Embed-Karten in EINER Nachricht.

        Body: {token, target|channel_id, sections:[{title, content}],
               replace_message_id?, role_ping_id?}
        - sections: je Eintrag wird ein eigenes Embed (max. 10, Discord-Limit).
        - replace_message_id: vorherige Bot-Nachricht wird vorher gelöscht.
        - role_ping_id: pingt diese Rolle als Nachrichten-Content.
        """
        try:
            data = await request.json()
        except Exception:
            return web.json_response({"ok": False, "error": "invalid JSON"}, status=400)

        if data.get("token") != CHANGELOG_API_TOKEN:
            return web.json_response({"ok": False, "error": "unauthorized"}, status=401)

        sections = data.get("sections") or []
        if not isinstance(sections, list) or not sections:
            return web.json_response({"ok": False, "error": "sections required"}, status=400)
        if len(sections) > 10:
            return web.json_response({"ok": False, "error": "max 10 sections"}, status=400)

        target = (data.get("target") or "all").strip()
        raw_channel_id = data.get("channel_id")
        if raw_channel_id:
            try:
                channel_id = int(raw_channel_id)
            except (TypeError, ValueError):
                return web.json_response(
                    {"ok": False, "error": "channel_id must be an integer"}, status=400
                )
        else:
            channel_id = TWITCH_BOT_CHANNEL_ID if target == "twitch" else DEV_UPDATES_CHANNEL_ID

        channel = self.bot.get_channel(channel_id)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(channel_id)
            except Exception:
                log.debug(
                    "rich-changelog: Channel %s nicht per API gefunden", channel_id, exc_info=True
                )
        if channel is None or not hasattr(channel, "send"):
            return web.json_response(
                {"ok": False, "error": f"channel {channel_id} not found"}, status=404
            )

        # Vorherigen Post ersetzen (optional)
        replace_id = data.get("replace_message_id")
        if replace_id:
            try:
                old = await channel.fetch_message(int(replace_id))
                await old.delete()
            except Exception as e:
                log.warning(
                    "rich-changelog: konnte alte Nachricht %s nicht löschen: %s", replace_id, e
                )

        footer = "Twitch Bot" if target == "twitch" else "Deadlock Bots"
        embeds: list[discord.Embed] = []
        for i, sec in enumerate(sections):
            title = str(sec.get("title") or "").strip()
            content = str(sec.get("content") or "").strip()
            if not content:
                continue
            embed = discord.Embed(
                title=title or None,
                description=content[:4096],
                color=0x5865F2,
            )
            if i == len(sections) - 1:
                embed.set_footer(text=footer)
                embed.timestamp = datetime.datetime.now(datetime.UTC)
            embeds.append(embed)

        if not embeds:
            return web.json_response({"ok": False, "error": "no valid sections"}, status=400)

        content = None
        allowed = None
        role_ping_id = data.get("role_ping_id")
        if role_ping_id:
            content = f"<@&{int(role_ping_id)}>"
            allowed = discord.AllowedMentions(roles=True, everyone=False, users=False)

        try:
            sent = await channel.send(content=content, embeds=embeds, allowed_mentions=allowed)
            log.info("rich-changelog: %s Embeds in Kanal %s gepostet", len(embeds), channel_id)
            return web.json_response({"ok": True, "channel_id": channel_id, "message_id": sent.id})
        except Exception as e:
            log.warning("rich-changelog post failed: %s", e)
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    async def _highlight_handler(self, request: web.Request) -> web.Response:
        try:
            data = await request.json()
        except Exception:
            return web.json_response({"ok": False, "error": "invalid JSON"}, status=400)

        if data.get("token") != CHANGELOG_API_TOKEN:
            return web.json_response({"ok": False, "error": "unauthorized"}, status=401)

        channel_id = int(data.get("channel_id") or 0)
        streamer = str(data.get("streamer_login") or "").strip()
        match_id = int(data.get("match_id") or 0)
        events = data.get("events") or []
        clip_paths = data.get("clip_paths") or []

        if not channel_id or not streamer or not match_id:
            return web.json_response(
                {"ok": False, "error": "channel_id, streamer_login, match_id required"}, status=400
            )

        channel = self.bot.get_channel(channel_id)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(channel_id)
            except Exception:
                log.debug(
                    "highlight-clips: Channel %s nicht per API gefunden", channel_id, exc_info=True
                )
        if channel is None or not hasattr(channel, "send"):
            return web.json_response(
                {"ok": False, "error": f"channel {channel_id} not found"}, status=404
            )

        try:
            embed = discord.Embed(
                title=f"Highlights — {streamer} (Match #{match_id})",
                description=f"{len(clip_paths)} Clip(s)",
                color=discord.Color.orange(),
                timestamp=datetime.datetime.now(datetime.UTC),
            )
            await channel.send(embed=embed)

            for event, clip_path in zip(events, clip_paths, strict=False):
                path = Path(clip_path)
                if not path.exists() or path.stat().st_size > MAX_FILE_BYTES:
                    continue
                label = event.get("label") or event.get("event_type") or "Clip"
                await channel.send(
                    content=f"**{streamer}** — {label}",
                    file=discord.File(path, filename=path.name),
                )

            log.info(
                "highlight-clips: %s clips posted for %s match=%s",
                len(clip_paths),
                streamer,
                match_id,
            )
            return web.json_response({"ok": True, "clips_sent": len(clip_paths)})
        except Exception as e:
            log.warning("highlight-clips post failed: %s", e)
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    @staticmethod
    def _serialize_message(m: discord.Message) -> dict:
        return {
            "id": m.id,
            "created_at": m.created_at.isoformat(),
            "author": {"id": m.author.id, "name": str(m.author), "bot": m.author.bot},
            "content": m.content,
            "embeds": [e.to_dict() for e in m.embeds],
            "attachments": [{"filename": a.filename, "url": a.url} for a in m.attachments],
        }

    async def _fetch_messages_handler(self, request: web.Request) -> web.Response:
        """Local-only read API: zieht eine Discord-Message (oder die letzten N) als JSON.

        Body: {token, channel_id, message_id?} oder {token, channel_id, limit?}
        """
        try:
            data = await request.json()
        except Exception:
            return web.json_response({"ok": False, "error": "invalid JSON"}, status=400)

        if data.get("token") != CHANGELOG_API_TOKEN:
            return web.json_response({"ok": False, "error": "unauthorized"}, status=401)

        try:
            channel_id = int(data.get("channel_id") or 0)
        except (TypeError, ValueError):
            return web.json_response(
                {"ok": False, "error": "channel_id must be an integer"}, status=400
            )
        if not channel_id:
            return web.json_response({"ok": False, "error": "channel_id required"}, status=400)

        channel = self.bot.get_channel(channel_id)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(channel_id)
            except Exception as e:
                return web.json_response(
                    {"ok": False, "error": f"channel {channel_id}: {e}"}, status=404
                )
        if not hasattr(channel, "fetch_message"):
            return web.json_response(
                {"ok": False, "error": f"channel {channel_id} not readable"}, status=400
            )

        try:
            message_id = data.get("message_id")
            if message_id:
                msg = await channel.fetch_message(int(message_id))
                return web.json_response({"ok": True, "messages": [self._serialize_message(msg)]})

            limit = min(int(data.get("limit") or 1), 100)
            before_id = data.get("before_id")
            before_obj = discord.Object(id=int(before_id)) if before_id else None
            msgs = [
                self._serialize_message(m)
                async for m in channel.history(limit=limit, before=before_obj)
            ]
            return web.json_response({"ok": True, "messages": msgs})
        except Exception as e:
            log.warning("fetch-messages failed for channel %s: %s", channel_id, e)
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    async def _alert_handler(self, request: web.Request) -> web.Response:
        """Server-Health-Warnung in den Admin-Channel, mit User-Ping bei warn/crit.

        Body: {token, title, content, level: "warn"|"crit"|"ok"}
        """
        try:
            data = await request.json()
        except Exception:
            return web.json_response({"ok": False, "error": "invalid JSON"}, status=400)

        if data.get("token") != CHANGELOG_API_TOKEN:
            return web.json_response({"ok": False, "error": "unauthorized"}, status=401)

        title = (data.get("title") or "").strip()
        content = (data.get("content") or "").strip()
        level = (data.get("level") or "warn").strip()
        if level not in ALERT_LEVELS:
            return web.json_response(
                {"ok": False, "error": f"level must be one of {set(ALERT_LEVELS)}"}, status=400
            )
        if not title or not content:
            return web.json_response(
                {"ok": False, "error": "title and content required"}, status=400
            )

        channel = self.bot.get_channel(SERVER_ALERT_CHANNEL_ID)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(SERVER_ALERT_CHANNEL_ID)
            except Exception:
                log.debug(
                    "server-alert: Channel %s nicht per API gefunden",
                    SERVER_ALERT_CHANNEL_ID,
                    exc_info=True,
                )
        if channel is None or not hasattr(channel, "send"):
            return web.json_response(
                {"ok": False, "error": f"channel {SERVER_ALERT_CHANNEL_ID} not found"}, status=404
            )

        icon, color = ALERT_LEVELS[level]
        embed = discord.Embed(
            title=f"{icon} {title}",
            description=content[:4096],
            color=color,
            timestamp=datetime.datetime.now(datetime.UTC),
        )
        embed.set_footer(text="Server-Monitor")

        ping = None
        allowed = None
        if level in ("warn", "crit"):
            ping = f"<@{SERVER_ALERT_PING_USER_ID}>"
            allowed = discord.AllowedMentions(users=True, roles=False, everyone=False)

        try:
            await channel.send(content=ping, embed=embed, allowed_mentions=allowed)
            log.info("Server-Alert (%s) gepostet: %s", level, title)
            return web.json_response({"ok": True, "channel_id": SERVER_ALERT_CHANNEL_ID})
        except Exception as e:
            log.warning("Server-Alert post failed: %s", e)
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    async def _http_handler(self, request: web.Request) -> web.Response:
        try:
            data = await request.json()
        except Exception:
            return web.json_response({"ok": False, "error": "invalid JSON"}, status=400)

        if data.get("token") != CHANGELOG_API_TOKEN:
            return web.json_response({"ok": False, "error": "unauthorized"}, status=401)

        title = (data.get("title") or "").strip()
        content = (data.get("content") or "").strip()
        target = (data.get("target") or "all").strip()

        if not title or not content:
            return web.json_response(
                {"ok": False, "error": "title and content required"}, status=400
            )

        # Direkte channel_id hat Vorrang vor target
        raw_channel_id = data.get("channel_id")
        if raw_channel_id:
            try:
                direct_channel_id = int(raw_channel_id)
            except (TypeError, ValueError):
                return web.json_response(
                    {"ok": False, "error": "channel_id must be an integer"}, status=400
                )
            try:
                channel = self.bot.get_channel(direct_channel_id) or await self.bot.fetch_channel(
                    direct_channel_id
                )
                embed = await self._build_embed(title, content, target)
                learning = _parse_spam_learning(data.get("spam_learning"))
                view = SpamLearningView(learning) if learning is not None else None
                await channel.send(embed=embed, view=view)
                return web.json_response({"ok": True, "channel_id": direct_channel_id})
            except Exception as e:
                log.warning("Changelog HTTP post (direct channel) failed: %s", e)
                return web.json_response({"ok": False, "error": str(e)}, status=500)

        if target not in VALID_TARGETS:
            return web.json_response(
                {"ok": False, "error": f"target must be one of {VALID_TARGETS}"}, status=400
            )

        try:
            channel_id = await self._send_changelog(title, content, target)
            return web.json_response({"ok": True, "channel_id": channel_id})
        except Exception as e:
            log.warning("Changelog HTTP post failed: %s", e)
            return web.json_response({"ok": False, "error": str(e)}, status=500)

    changelog_group = app_commands.Group(
        name="changelog",
        description="Changelog-Eintrag in Discord posten",
        guild_only=True,
    )

    @changelog_group.command(name="post", description="Changelog-Eintrag in Discord posten")
    @app_commands.describe(
        title="Überschrift des Eintrags",
        content="Inhalt als Markdown (z. B. '- Feature A\\n- Bug gefixt')",
        target="Zielkanal: 'all' für alle Bots, 'twitch' für den Twitch-Bot",
    )
    @app_commands.choices(
        target=[
            app_commands.Choice(name="Alle Bots (Dev Updates)", value="all"),
            app_commands.Choice(name="Twitch Bot", value="twitch"),
        ]
    )
    async def changelog_post(
        self,
        interaction: discord.Interaction,
        title: str,
        content: str,
        target: str = "all",
    ) -> None:
        perms = interaction.user.guild_permissions  # type: ignore[union-attr]
        if not (perms.manage_guild or perms.administrator):
            await interaction.response.send_message("Keine Berechtigung.", ephemeral=True)
            return

        await interaction.response.defer(ephemeral=True)
        try:
            channel_id = await self._send_changelog(title, content, target)
            confirm = discord.Embed(
                description=f"✅ Changelog in <#{channel_id}> gepostet.",
                color=0x57F287,
            )
            await interaction.followup.send(embed=confirm, ephemeral=True)
        except Exception as e:
            await interaction.followup.send(f"❌ Fehler: {e}", ephemeral=True)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(ChangelogPublisher(bot))
