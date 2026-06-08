from __future__ import annotations

import asyncio
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

    @commands.Cog.listener()
    async def on_ready(self) -> None:
        if getattr(self, "_interface_posted", False):
            return
        self._interface_posted = True
        log.info("RouterInterfaceCog: on_ready → poste Interface-Message")
        try:
            await self._post_interface_message()
        except Exception:
            log.exception("RouterInterfaceCog: _post_interface_message fehlgeschlagen")

    @staticmethod
    def _build_guide_embed() -> discord.Embed:
        e = discord.Embed(
            title="📖 Router — Wie funktioniert das?",
            color=0x2B2D31,
        )
        e.add_field(
            name="1️⃣  Modus wählen",
            value=(
                "Klick unten auf **Casual**, **Ranked** oder **Street Brawl**. "
                "Deine Wahl wird gespeichert und gilt für alle zukünftigen Joins."
            ),
            inline=False,
        )
        e.add_field(
            name="2️⃣  Auto-Join",
            value=(
                "**Aus (grau)** → du bekommst immer eine eigene, leere Lane.\n"
                "**An (grün)** → der Bot sucht eine passende Lane mit freien Plätzen (<6 Personen). "
                "Bekannte Mitspieler werden dabei bevorzugt. "
                "Gibt es keine freie Lane, wird eine neue für dich erstellt."
            ),
            inline=False,
        )
        e.add_field(
            name="3️⃣  Router-VC betreten",
            value=(
                "Sobald du den **Deadlock Router**-Sprachkanal betrittst, passiert alles automatisch:\n"
                "• Neuer Spieler → New-Player-Lane\n"
                "• Kein Modus gesetzt → du bleibst im Router-VC bis du unten einen wählst\n"
                "• Modus gesetzt, Auto-Join aus → sofort eigene Lane\n"
                "• Modus gesetzt, Auto-Join an → Smart Routing"
            ),
            inline=False,
        )
        e.add_field(
            name="🏆  Ranked",
            value=(
                "Ranked-Lanes erfordern einen **verifizierten Rang** (Steam-Verknüpfung). "
                "Ohne Rang bekommst du eine DM mit dem Link zur Verifizierung — "
                "du bleibst dann im Router-VC und kannst danach Casual oder Street Brawl wählen."
            ),
            inline=False,
        )
        e.add_field(
            name="🔄  Lane-Modus wechseln",
            value=(
                "Als Lane-Owner kannst du deinen aktiven Kanal nachträglich umstellen: "
                "im Lane-Control-Interface gibt es **Modus wechseln** (Casual / Ranked / Street Brawl / Off Topic) "
                "und **Umbenennen**. "
                "Der Kanal zieht dabei physisch in die passende Kategorie um."
            ),
            inline=False,
        )
        e.set_footer(text="Die alten Staging-Kanäle (Casual / Comp / Street Brawl) laufen weiterhin parallel.")
        return e

    @staticmethod
    def _build_interface_embed() -> discord.Embed:
        return discord.Embed(
            title="🎮 Spielmodus wählen",
            description=(
                "Wähle deinen **Standard-Spielmodus** und stelle den Auto-Join-Toggle ein.\n\n"
                "**Auto-Join aus** → eigene Lane wird für dich erstellt\n"
                "**Auto-Join an** → Smart Routing in eine passende Lane"
            ),
            color=0x5865F2,
        )

    async def _post_interface_message(self) -> None:
        cfg = get_guild_config()
        ch = self.bot.get_channel(cfg.TEMPVOICE_ROUTER_TEXT)
        if ch is None:
            ch = await self.bot.fetch_channel(cfg.TEMPVOICE_ROUTER_TEXT)
        if not isinstance(ch, discord.TextChannel):
            log.warning(
                "RouterInterfaceCog: Textkanal %s nicht erreichbar", cfg.TEMPVOICE_ROUTER_TEXT
            )
            return

        guide_embed = self._build_guide_embed()
        interface_embed = self._build_interface_embed()

        # Bestehende Bot-Nachrichten einlesen (max. 15, älteste zuerst)
        bot_msgs: list[discord.Message] = []
        async for msg in ch.history(limit=15, oldest_first=True):
            if msg.author == self.bot.user and msg.embeds:
                bot_msgs.append(msg)

        # Anleitung-Message: erste Bot-Nachricht ohne View (kein Button)
        guide_msg = next((m for m in bot_msgs if not m.components), None)
        # Interface-Message: erste Bot-Nachricht mit Buttons (components vorhanden)
        iface_msg = next((m for m in bot_msgs if m.components), None)

        if guide_msg:
            await guide_msg.edit(embed=guide_embed)
            log.info("RouterInterfaceCog: Anleitung aktualisiert (ID %s)", guide_msg.id)
        else:
            guide_msg = await ch.send(embed=guide_embed)
            log.info("RouterInterfaceCog: Anleitung gepostet (ID %s)", guide_msg.id)

        if iface_msg:
            await iface_msg.edit(embed=interface_embed, view=RouterView())
            log.info("RouterInterfaceCog: Interface aktualisiert (ID %s)", iface_msg.id)
        else:
            iface_msg = await ch.send(embed=interface_embed, view=RouterView())
            log.info("RouterInterfaceCog: Interface gepostet (ID %s)", iface_msg.id)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(RouterInterfaceCog(bot))
