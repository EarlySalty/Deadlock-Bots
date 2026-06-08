# cogs/tempvoice/__init__.py
from __future__ import annotations

from discord.ext import commands


async def setup(bot: commands.Bot):
    # Beide Cogs in EINER Extension registrieren, mit sauberer Verkabelung.
    from .core import TempVoiceCore
    from .duo_lanes import DuoLanes
    from .interface import TempVoiceInterface
    from .lane_sorting import TempVoiceLaneSorting
    from .new_player_lanes import NewPlayerAdaptiveLanes
    from .router import RouterCog
    from .router_interface import RouterInterfaceCog
    from .util import TempVoiceUtil

    core = TempVoiceCore(bot)
    util = TempVoiceUtil(core)

    await bot.add_cog(core)
    await bot.add_cog(TempVoiceInterface(bot, core, util))
    await bot.add_cog(TempVoiceLaneSorting(bot, core))
    await bot.add_cog(NewPlayerAdaptiveLanes(bot))
    await bot.add_cog(DuoLanes(bot))
    await bot.add_cog(RouterCog(bot))
    await bot.add_cog(RouterInterfaceCog(bot))
