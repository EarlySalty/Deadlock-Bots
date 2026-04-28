"""TierlistPublicCog - starts/stops the public tierlist server (port 8770)."""

from __future__ import annotations

import asyncio
import logging
from typing import TYPE_CHECKING

from discord.ext import commands

if TYPE_CHECKING:
    from main_bot import MasterBot

log = logging.getLogger(__name__)


class TierlistPublicCog(commands.Cog):
    """Wraps TierlistPublicServer as a reloadable cog."""

    def __init__(self, bot: MasterBot) -> None:
        self.bot = bot
        self.server: object | None = None
        self._start_task: asyncio.Task | None = None

        try:
            from service.tierlist_public import TierlistPublicServer

            self.server = TierlistPublicServer(self.bot)
            self._start_task = self.bot.loop.create_task(self._start_server())
            log.info("TierlistPublicServer initialisiert.")
        except Exception as exc:
            log.error("TierlistPublicServer konnte nicht initialisiert werden: %s", exc)
            self.server = None

    async def cog_unload(self) -> None:
        if self._start_task and not self._start_task.done():
            self._start_task.cancel()
            try:
                await self._start_task
            except asyncio.CancelledError:
                log.debug("TierlistPublicServer start task cancelled during cog_unload")

        if self.server:
            try:
                await self.server.stop()
            except Exception as exc:
                log.error("Fehler beim Stoppen des TierlistPublicServers: %s", exc)

    async def _start_server(self) -> None:
        await self.bot.wait_until_ready()
        if self.server:
            try:
                await self.server.start()
            except Exception as exc:
                log.error("TierlistPublicServer konnte nicht gestartet werden: %s", exc)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(TierlistPublicCog(bot))  # type: ignore[arg-type]
