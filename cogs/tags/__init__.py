from __future__ import annotations

from discord.ext import commands

from .core import TagService
from .interface import TagInterface
from .mod_commands import ModTagCommands


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(TagService(bot))
    await bot.add_cog(TagInterface(bot))
    await bot.add_cog(ModTagCommands(bot))
