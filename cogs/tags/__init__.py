from __future__ import annotations

from discord.ext import commands

from .core import TagService


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(TagService(bot))
