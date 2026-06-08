from __future__ import annotations

import logging

import discord
from discord.ext import commands

from service.guild_config import get_guild_config

log = logging.getLogger("cogs.tempvoice.router_interface")

_MODES: dict[str, tuple[str, str]] = {
    "casual":       ("🎮", "Casual"),
    "ranked":       ("🏆", "Ranked"),
    "street_brawl": ("⚡", "Street Brawl"),
}


class ModeButton(discord.ui.Button):
    def __init__(self, mode: str) -> None:
        emoji, label = _MODES[mode]
        super().__init__(
            label=f"{emoji} {label}",
            custom_id=f"router_mode_{mode}",
            style=discord.ButtonStyle.secondary,
            row=0,
        )
        self.mode = mode

    async def callback(self, itx: discord.Interaction) -> None:
        router_cog = itx.client.get_cog("RouterCog")
        if router_cog is None:
            await itx.response.send_message("Router nicht verfügbar.", ephemeral=True)
            return

        pref = await router_cog.get_user_pref(itx.user.id)
        auto_join = pref["auto_join"] if pref is not None else False
        await router_cog.set_user_pref(itx.user.id, self.mode, auto_join)

        label = _MODES[self.mode][1]
        cfg = get_guild_config()
        member = itx.user
        if (
            auto_join
            and isinstance(member, discord.Member)
            and member.voice is not None
            and member.voice.channel is not None
            and member.voice.channel.id == cfg.TEMPVOICE_ROUTER_VC
        ):
            await itx.response.defer(ephemeral=True)
            await router_cog._smart_route(member, self.mode)
            await itx.followup.send(
                f"Modus **{label}** gesetzt — du wirst geroutet.", ephemeral=True
            )
        else:
            await itx.response.send_message(
                f"Standard-Modus auf **{label}** gesetzt.", ephemeral=True
            )


class AutoJoinButton(discord.ui.Button):
    def __init__(self) -> None:
        super().__init__(
            label="Auto-Join",
            custom_id="router_autojoin_toggle",
            style=discord.ButtonStyle.secondary,
            emoji="⬜",
            row=1,
        )

    async def callback(self, itx: discord.Interaction) -> None:
        router_cog = itx.client.get_cog("RouterCog")
        if router_cog is None:
            await itx.response.send_message("Router nicht verfügbar.", ephemeral=True)
            return

        pref = await router_cog.get_user_pref(itx.user.id)
        if pref is None:
            await itx.response.send_message(
                "Wähle zuerst einen Spielmodus.", ephemeral=True
            )
            return

        new_val = not pref["auto_join"]
        await router_cog.set_user_pref(itx.user.id, pref["mode"], new_val)
        status = "✅ aktiviert" if new_val else "❌ deaktiviert"
        await itx.response.send_message(f"Auto-Join **{status}**.", ephemeral=True)

        cfg = get_guild_config()
        member = itx.user
        if (
            new_val
            and isinstance(member, discord.Member)
            and member.voice is not None
            and member.voice.channel is not None
            and member.voice.channel.id == cfg.TEMPVOICE_ROUTER_VC
        ):
            await router_cog._smart_route(member, pref["mode"])


class RouterView(discord.ui.View):
    def __init__(self) -> None:
        super().__init__(timeout=None)
        for mode in ("casual", "ranked", "street_brawl"):
            self.add_item(ModeButton(mode))
        self.add_item(AutoJoinButton())


class RouterInterfaceCog(commands.Cog, name="RouterInterfaceCog"):
    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot

    async def cog_load(self) -> None:
        self.bot.add_view(RouterView())
        self.bot.loop.create_task(self._ensure_interface_message())

    async def _ensure_interface_message(self) -> None:
        await self.bot.wait_until_ready()
        cfg = get_guild_config()
        ch = self.bot.get_channel(cfg.TEMPVOICE_ROUTER_TEXT)
        if not isinstance(ch, discord.TextChannel):
            log.warning(
                "RouterInterfaceCog: Textkanal %s nicht gefunden", cfg.TEMPVOICE_ROUTER_TEXT
            )
            return

        embed = discord.Embed(
            title="🎮 Spielmodus wählen",
            description=(
                "Wähle deinen **Standard-Spielmodus**.\n\n"
                "**Auto-Join aus** → eine eigene Lane wird nur für dich erstellt\n"
                "**Auto-Join an** → du landest in einer passenden Lane mit freien Plätzen\n\n"
                "Ranked-Lanes erfordern einen verifizierten Rang (Steam-Verknüpfung)."
            ),
            color=0x5865F2,
        )
        async for msg in ch.history(limit=10):
            if msg.author == self.bot.user and msg.embeds:
                try:
                    await msg.edit(embed=embed, view=RouterView())
                except discord.HTTPException:
                    pass
                return

        await ch.send(embed=embed, view=RouterView())


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(RouterInterfaceCog(bot))
