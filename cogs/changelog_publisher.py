from __future__ import annotations

import datetime
import logging
import os
from typing import TYPE_CHECKING

import discord
from aiohttp import web
from discord import app_commands
from discord.ext import commands

if TYPE_CHECKING:
    pass

log = logging.getLogger(__name__)

DEV_UPDATES_CHANNEL_ID = 1492910851483504821
TWITCH_BOT_CHANNEL_ID = 1318329964713611385
CHANGELOG_API_TOKEN = os.getenv("CHANGELOG_API_TOKEN", "changeme-local")
CHANGELOG_API_PORT = int(os.getenv("CHANGELOG_API_PORT", "8899"))

VALID_TARGETS = {"all", "twitch"}


class ChangelogPublisher(commands.Cog):
    """Posts changelog entries as Discord embeds and exposes an internal HTTP API."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self._runner: web.AppRunner | None = None
        self._site: web.TCPSite | None = None

    async def cog_load(self) -> None:
        app = web.Application()
        app.router.add_post("/changelog", self._http_handler)
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
            timestamp=datetime.datetime.now(datetime.timezone.utc),
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
            return web.json_response({"ok": False, "error": "title and content required"}, status=400)
        if target not in VALID_TARGETS:
            return web.json_response({"ok": False, "error": f"target must be one of {VALID_TARGETS}"}, status=400)

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
    @app_commands.choices(target=[
        app_commands.Choice(name="Alle Bots (Dev Updates)", value="all"),
        app_commands.Choice(name="Twitch Bot", value="twitch"),
    ])
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
