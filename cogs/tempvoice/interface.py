# cogs/tempvoice/interface.py
import asyncio
import logging

import discord
from discord.ext import commands

from service.config import settings

from .core import (
    FIXED_LANE_IDS,
    MINRANK_CATEGORY_IDS,
    RANK_ORDER,
    RANKED_CATEGORY_ID,
    LaneTagFilter,
    _member_rank_index,
)

logger = logging.getLogger("cogs.tempvoice.interface")

# Speichert den gewählten Haupt-Rang bis der Sub-Rang gewählt wird (lane_id → rank)
_pending_main_rank: dict[int, str] = {}


async def setup(bot: commands.Bot):
    """
    Lokales Setup nur für den Fall, dass diese Extension
    separat geladen wird. Erkennt vorhandene Core/Util-Cogs
    und vermeidet Doppel-Registrierungen.
    """
    # 1) Core ermitteln oder anlegen
    core = bot.get_cog("TempVoiceCore")
    if core is None:
        try:
            from .core import TempVoiceCore

            core = TempVoiceCore(bot)
            await bot.add_cog(core)
        except Exception as e:
            logger.exception(
                "TempVoiceInterface.setup: Konnte TempVoiceCore nicht initialisieren: %r",
                e,
            )
            return

    # 2) Util ermitteln oder anlegen
    util = getattr(core, "util", None)
    if util is None:
        try:
            from .util import TempVoiceUtil

            util = TempVoiceUtil(core)
            # util ist ein Hilfsobjekt, kein Cog – muss i. d. R. nicht registriert werden
        except Exception as e:
            logger.exception(
                "TempVoiceInterface.setup: Konnte TempVoiceUtil nicht initialisieren: %r",
                e,
            )
            return

    # 3) Interface-Cog hinzufügen (nur einmal)
    if bot.get_cog("TempVoiceInterface") is None:
        await bot.add_cog(TempVoiceInterface(bot, core, util))


def _find_rank_emoji(guild: discord.Guild | None, rank: str):
    if not guild:
        return None
    return discord.utils.get(guild.emojis, name=rank)


def _resolve_member_rank_index(core, member: discord.Member) -> int:
    """Nutzt bevorzugt den Rank-Manager und faellt sonst auf Rollen-Namen zurueck."""
    bot = getattr(core, "bot", None)
    if bot and hasattr(bot, "get_cog"):
        rank_mgr = bot.get_cog("RolePermissionVoiceManager")
        if rank_mgr and hasattr(rank_mgr, "get_user_rank_from_roles"):
            try:
                _rank_name, rank_value, _subrank = rank_mgr.get_user_rank_from_roles(member)
            except Exception as exc:
                logger.debug("Min-rank member lookup failed for %s: %r", member.id, exc)
            else:
                try:
                    resolved = int(rank_value or 0)
                except (TypeError, ValueError):
                    resolved = 0
                if resolved > 0:
                    return min(resolved, len(RANK_ORDER) - 1)
    return _member_rank_index(member)


def _is_owner_or_mod(core, lane: discord.VoiceChannel, member: discord.Member) -> bool:
    owner_id = core.lane_owner.get(lane.id, member.id)
    perms = lane.permissions_for(member)
    return bool(owner_id == member.id or perms.manage_channels or perms.administrator)


def _format_tag_filter_summary(lane_filter: LaneTagFilter) -> str:
    parts: list[str] = []
    if lane_filter.min_age_tag == "25+":
        parts.append("25+")
    if lane_filter.required_tone_tag == "ragebaiter_free":
        parts.append("Ragebaiter-Free")
    if lane_filter.deny_ragebaiter:
        parts.append("Ragebaiter blockiert")
    if not parts:
        return "Tag-Filter aus."
    return f"Tag-Filter aktiv: {' · '.join(parts)}."


class TempVoiceInterface(commands.Cog):
    """UI/Buttons – persistente View & Interface-Message-Handling"""

    def __init__(self, bot: commands.Bot, core, util):
        self.bot = bot
        self.core = core  # TempVoiceCore
        self.util = util  # TempVoiceUtil

    def _view_for_category(self, category_id: int | None) -> discord.ui.View:
        is_ranked = category_id == RANKED_CATEGORY_ID
        return MainView(self.core, self.util, include_minrank=is_ranked, include_presets=is_ranked)

    async def cog_load(self):
        # Persistente Views registrieren (superset, damit alle Custom IDs bekannt sind)
        self.bot.add_view(
            MainView(self.core, self.util, include_minrank=True, include_presets=True)
        )
        self.bot.add_view(
            MainView(self.core, self.util, include_minrank=False, include_presets=False)
        )
        asyncio.create_task(self._startup())

    async def _startup(self):
        await self.bot.wait_until_ready()
        await self.ensure_interface_message()
        await self.refresh_all_interfaces()

    async def ensure_interface_message(
        self,
        channel_hint: discord.TextChannel | None = None,
        *,
        category_id: int | None = None,
    ):
        """
        Stellt sicher, dass die Interface-Nachricht existiert – exakt in dem Textkanal,
        in dem der Command ausgeführt wurde (channel_hint).
        Fällt nicht auf andere Channels zurück.
        """
        ch: discord.TextChannel | None = (
            channel_hint if isinstance(channel_hint, discord.TextChannel) else None
        )
        guild = ch.guild if ch else None

        if ch is None or guild is None:
            return None  # Muss mit einem Textkanal aufgerufen werden

        embed = discord.Embed(
            title="🚧 Sprachkanal verwalten",
            description=(
                "So funktioniert Temp Voice:\n"
                "• Betritt einen **(+) Sprachkanal**, deine eigene Lane wird automatisch erstellt.\n"
                "• Passe deine Lane hier an; die Buttons wirken sofort, wenn du Owner bist.\n\n"
                "Was ihr hier machen könnt:\n"
                "• **Kick:** Jemand AFK oder stört? Entferne die Person, wenn Reden nicht reicht.\n"
                "• **Ban:** Sperre jemanden dauerhaft aus deinem Kanal, solange du Owner bist.\n"
                "• **Unban:** Hebe die Sperre wieder auf.\n"
                "• **Duo/Trio Call:** Stelle 2er/3er-Runden ein; andere können (fast) nicht beitreten.\n"
                "• **Normale Lane:** Setzt die Berechtigungen wieder auf offen.\n"
                "• **Lurker-Rolle:** Für Zuhörer; schafft einen zusätzlichen Platz für Mitspieler.\n"
                "• **Limit & Sprache:** Setze Teilnehmerlimit (0–99) und Deutsch/Offen-Filter.\n"
                "• **Owner Claim & Mindest-Rang:** Übernimm die Lane und lege optional einen Mindest-Rang fest."
            ),
            color=0x2ECC71,
        )
        embed.set_footer(text="Deutsche Deadlock Community • TempVoice")

        view = self._view_for_category(category_id)

        # Immer in diesem Channel senden (kein Fallback/Fetch anderer Messages)
        try:
            msg = await ch.send(embed=embed, view=view)
            await self._record_interface_message(
                int(guild.id), int(ch.id), int(msg.id), category_id, None
            )
        except discord.Forbidden:
            logger.warning(
                "ensure_interface_message: Keine Berechtigung zum Senden in %s (%s)",
                ch,
                ch.id,
            )
            return None
        except discord.HTTPException as e:
            logger.warning(
                "ensure_interface_message: HTTP-Fehler beim Senden/Speichern der Interface-Nachricht: %r",
                e,
            )
            return None
        except Exception as e:
            logger.debug(
                "ensure_interface_message: Fehler beim Senden/Speichern der Interface-Nachricht: %r",
                e,
            )
            return None
        return ch

    @commands.guild_only()
    @commands.has_guild_permissions(manage_guild=True)
    @commands.command(name="tvpanel", aliases=["tempvoicepanel", "tvinterface"])
    async def cmd_tvpanel(self, ctx: commands.Context):
        """
        Erstellt/aktualisiert das TempVoice Interface und speichert es, damit es nach Neustart erhalten bleibt.
        """
        try:
            async with ctx.typing():
                ch = await self.ensure_interface_message(
                    ctx.channel if isinstance(ctx.channel, discord.TextChannel) else None
                )
        except discord.Forbidden:
            await ctx.reply(
                "❌ Keine Berechtigung, um das Interface zu erstellen.",
                mention_author=False,
            )
            return
        except Exception as e:
            logger.exception("tvpanel command failed: %r", e)
            await ctx.reply(
                "❌ Konnte das Interface nicht erstellen (Fehler im Log).",
                mention_author=False,
            )
            return
        try:
            if isinstance(ch, discord.TextChannel):
                await ctx.reply(
                    f"✅ TempVoice Interface erstellt/aktualisiert in {ch.mention}.",
                    mention_author=False,
                )
            else:
                await ctx.reply(
                    "⚠️ Konnte das Interface nicht erstellen (kein Textkanal oder keine Rechte).",
                    mention_author=False,
                )
        except Exception as e:
            logger.exception("tvpanel command failed: %r", e)
            await ctx.reply(
                "❌ Konnte das Interface nicht erstellen (Fehler im Log).",
                mention_author=False,
            )

    async def _record_interface_message(
        self,
        guild_id: int,
        channel_id: int,
        message_id: int,
        category_id: int | None,
        lane_id: int | None,
    ):
        try:
            if lane_id is not None:
                await self.core.db.execute_async(
                    """
                    INSERT INTO tempvoice_interface(guild_id, channel_id, message_id, category_id, lane_id, updated_at)
                    VALUES(?,?,?,?,?,CURRENT_TIMESTAMP)
                    ON CONFLICT(lane_id) DO UPDATE SET
                        channel_id=excluded.channel_id,
                        message_id=excluded.message_id,
                        category_id=excluded.category_id,
                        updated_at=CURRENT_TIMESTAMP
                    """,
                    (guild_id, channel_id, message_id, category_id, lane_id),
                )
            else:
                await self.core.db.execute_async(
                    """
                    INSERT INTO tempvoice_interface(guild_id, channel_id, message_id, category_id, lane_id, updated_at)
                    VALUES(?,?,?,?,NULL,CURRENT_TIMESTAMP)
                    ON CONFLICT(guild_id, message_id) DO UPDATE SET
                        channel_id=excluded.channel_id,
                        category_id=excluded.category_id,
                        updated_at=CURRENT_TIMESTAMP
                    """,
                    (guild_id, channel_id, message_id, category_id),
                )
        except Exception as e:
            logger.debug(
                "_record_interface_message: Persistenz fehlgeschlagen (guild=%s lane=%s): %r",
                guild_id,
                lane_id,
                e,
            )

    def _lane_embed(self, lane: discord.VoiceChannel, owner_id: int | None) -> discord.Embed:
        owner_display = "Unbekannt"
        if owner_id:
            owner = lane.guild.get_member(int(owner_id))
            if owner:
                owner_display = owner.mention
            else:
                owner_display = f"<@{owner_id}>"
        embed = discord.Embed(
            title=f"🎙️ TempVoice – {lane.name}",
            description=(
                f"**Owner:** {owner_display}\n\n"
                "**Steuerung** *(nur wenn du in dieser Lane bist)*\n"
                "🇩🇪 / 🌍 – Sprachfilter: nur DE oder alle EU\n"
                "👑 – Owner übernehmen (wenn der Owner die Lane verlassen hat)\n"
                "🔢 – Spielerlimit setzen (0 = kein Limit)\n"
                "🦵 Kick · 🚫 Ban · ✅ Unban – Mitglieder verwalten\n"
                "👥 Duo / Trio · 🔄 Reset – Lane-Größe schnell anpassen\n"
                "👻 Lurker – stumm beitreten ohne Limit-Slot zu belegen\n\n"
                "**Mindest-Rang** *(nur Ranked)*\n"
                "① Wähle den **Haupt-Rang** (z. B. Archon)\n"
                "② Wähle dann den **Sub-Rang** (1–6)\n"
                "→ Der Rang wird erst gesetzt, wenn **beide** gewählt sind."
            ),
            color=0x2ECC71,
        )
        embed.set_footer(text="Deutsche Deadlock Community • TempVoice")
        return embed

    async def ensure_lane_interface(self, lane: discord.VoiceChannel, owner_id: int | None = None):
        owner_id = owner_id or self.core.lane_owner.get(lane.id)
        embed = self._lane_embed(lane, owner_id)
        view = self._view_for_category(lane.category_id)

        row = None
        try:
            row = await self.core.db.query_one_async(
                "SELECT channel_id, message_id FROM tempvoice_interface WHERE lane_id=?",
                (int(lane.id),),
            )
        except Exception as e:
            logger.debug(
                "ensure_lane_interface: DB-Select fehlgeschlagen für Lane %s: %r",
                lane.id,
                e,
            )

        if row:
            target = self.bot.get_channel(int(row["channel_id"])) or lane
            try:
                msg = await target.fetch_message(int(row["message_id"]))
                await msg.edit(embed=embed, view=view)
                await self._record_interface_message(
                    int(lane.guild.id),
                    int(target.id),
                    int(msg.id),
                    int(lane.category_id) if lane.category_id else None,
                    int(lane.id),
                )
                return
            except (discord.NotFound, discord.Forbidden):
                await self._remove_lane_interface_record(int(lane.id))
            except discord.HTTPException as e:
                logger.debug(
                    "ensure_lane_interface: HTTP-Fehler beim Editieren (Lane %s): %r",
                    lane.id,
                    e,
                )
                return
            except Exception as e:
                logger.debug(
                    "ensure_lane_interface: Fehler beim Editieren (Lane %s): %r",
                    lane.id,
                    e,
                )
                return

        # Interface im Voice-Call-Chat deaktiviert – wird nur über den dedizierten
        # Interface-Kanal verwaltet (globale Interface-Nachrichten ohne lane_id).
        logger.debug(
            "ensure_lane_interface: Kein Interface-Eintrag für Lane %s – Senden in Voice-Chat deaktiviert.",
            lane.id,
        )

    async def rehydrate_lane_interfaces(self):
        try:
            rows = await self.core.db.query_all_async(
                "SELECT channel_id, message_id, lane_id FROM tempvoice_interface WHERE lane_id IS NOT NULL"
            )
        except Exception as e:
            logger.debug("rehydrate_lane_interfaces: DB-Select fehlgeschlagen: %r", e)
            return

        for row in rows:
            lane_id = int(row["lane_id"])
            lane = self.bot.get_channel(lane_id)
            if not isinstance(lane, discord.VoiceChannel):
                await self._remove_lane_interface_record(lane_id)
                continue

            target = self.bot.get_channel(int(row["channel_id"])) or lane
            try:
                msg = await target.fetch_message(int(row["message_id"]))
            except (discord.NotFound, discord.Forbidden):
                await self._remove_lane_interface_record(lane_id)
                await self.ensure_lane_interface(lane)
                continue
            except discord.HTTPException as e:
                logger.debug(
                    "rehydrate_lane_interfaces: HTTP-Fehler beim Laden (Lane %s): %r",
                    lane_id,
                    e,
                )
                continue
            except Exception as e:
                logger.debug(
                    "rehydrate_lane_interfaces: Fehler beim Laden (Lane %s): %r",
                    lane_id,
                    e,
                )
                continue

            owner_id = self.core.lane_owner.get(lane_id)
            try:
                is_ranked = bool(lane and lane.category_id in MINRANK_CATEGORY_IDS)
                await msg.edit(
                    embed=self._lane_embed(lane, owner_id),
                    view=MainView(
                        self.core,
                        self.util,
                        include_minrank=is_ranked,
                        include_presets=is_ranked,
                    ),
                )
                await self._record_interface_message(
                    int(lane.guild.id),
                    int(target.id),
                    int(msg.id),
                    int(lane.category_id) if lane.category_id else None,
                    lane_id,
                )
            except discord.HTTPException as e:
                logger.debug(
                    "rehydrate_lane_interfaces: HTTP-Fehler beim Editieren (Lane %s): %r",
                    lane_id,
                    e,
                )
            except Exception as e:
                logger.debug(
                    "rehydrate_lane_interfaces: Fehler beim Editieren (Lane %s): %r",
                    lane_id,
                    e,
                )

    async def _refresh_global_interface_messages(self):
        """
        Aktualisiert Interface-Nachrichten (ohne Lane-Bindung) mit der aktuellen View,
        damit neue Buttons (z. B. Normale Lane) auch dort erscheinen.
        """
        try:
            rows = await self.core.db.query_all_async(
                "SELECT guild_id, channel_id, message_id, category_id FROM tempvoice_interface WHERE lane_id IS NULL"
            )
        except Exception as e:
            logger.debug("refresh_global_interfaces: DB-Select fehlgeschlagen: %r", e)
            return

        for row in rows:
            channel = self.bot.get_channel(int(row["channel_id"]))
            if not isinstance(channel, (discord.TextChannel, discord.VoiceChannel)):
                continue
            try:
                msg = await channel.fetch_message(int(row["message_id"]))
            except (discord.NotFound, discord.Forbidden):
                continue
            except discord.HTTPException as e:
                logger.debug(
                    "refresh_global_interfaces: fetch fehlgeschlagen f�r Message %s: %r",
                    row["message_id"],
                    e,
                )
                continue
            except Exception as e:
                logger.debug(
                    "refresh_global_interfaces: unerwarteter Fehler f�r Message %s: %r",
                    row["message_id"],
                    e,
                )
                continue

            try:
                view = self._view_for_category(
                    int(row["category_id"]) if row["category_id"] else None
                )
                await msg.edit(view=view)
                await self._record_interface_message(
                    int(row["guild_id"]),
                    int(channel.id),
                    int(msg.id),
                    int(row["category_id"]) if row["category_id"] else None,
                    None,
                )
            except discord.HTTPException as e:
                logger.debug(
                    "refresh_global_interfaces: edit fehlgeschlagen f�r Message %s: %r",
                    msg.id,
                    e,
                )
            except Exception as e:
                logger.debug(
                    "refresh_global_interfaces: unerwarteter Fehler beim Editieren f�r Message %s: %r",
                    msg.id,
                    e,
                )

    async def refresh_all_interfaces(self):
        await self._refresh_global_interface_messages()
        await self.rehydrate_lane_interfaces()

    async def _remove_lane_interface_record(self, lane_id: int):
        try:
            await self.core.db.execute_async(
                "DELETE FROM tempvoice_interface WHERE lane_id=?", (int(lane_id),)
            )
        except Exception as e:
            logger.debug(
                "remove_lane_interface_record: DB-Delete fehlgeschlagen für Lane %s: %r",
                lane_id,
                e,
            )

    @commands.Cog.listener()
    async def on_tempvoice_lane_created(self, lane: discord.VoiceChannel, owner: discord.Member):
        await self.ensure_lane_interface(lane, owner_id=owner.id)

    @commands.Cog.listener()
    async def on_tempvoice_lane_owner_changed(self, lane: discord.VoiceChannel, owner_id: int):
        await self.ensure_lane_interface(lane, owner_id=owner_id)

    @commands.Cog.listener()
    async def on_tempvoice_lane_deleted(self, lane_id: int):
        await self._remove_lane_interface_record(int(lane_id))


# -------------------- UI Komponenten --------------------


class MainView(discord.ui.View):
    def __init__(self, core, util, *, include_minrank: bool, include_presets: bool):
        super().__init__(timeout=None)
        self.core = core
        self.util = util
        # Row 0: Region + OwnerClaim + Limit
        self.add_item(RegionDEButton(core))
        self.add_item(RegionEUButton(core))
        self.add_item(OwnerClaimButton(core))
        self.add_item(LimitButton(core))
        # Row 1: Kick/Ban/Unban (+ Lurker bei Ranked, da Row 3 für Presets genutzt wird)
        self.add_item(KickButton(util))
        self.add_item(BanButton(util))
        self.add_item(UnbanButton(util))
        if include_minrank:
            # Ranked: Lurker und Tag-Filter zu Row 1 (Mod-Reihe), damit Row 3
            # für Quick-Templates + Presets frei bleibt (max. 5 Items pro Row).
            self.add_item(LurkerButton(util, row=1))
            self.add_item(TagFilterButton(core, row=1))
            # Row 2: Haupt-Rang Selektor
            self.add_item(MinRankSelect(core))
            # Row 3: Quick Templates + Presets
            self.add_item(ResetLaneButton(core))
            self.add_item(DuoCallButton(core))
            self.add_item(TrioCallButton(core))
            if include_presets:
                self.add_item(SavePresetButton(core, row=3))
                self.add_item(LoadPresetButton(core, row=3))
            # Row 4: Sub-Rang Selektor
            self.add_item(SubRankSelectPermanent(core))
        else:
            # Nicht-Ranked: Standard-Layout
            self.add_item(TagFilterButton(core, row=2))
            self.add_item(ResetLaneButton(core))
            self.add_item(DuoCallButton(core))
            self.add_item(TrioCallButton(core))
            self.add_item(LurkerButton(util))

    @staticmethod
    def lane_of(itx: discord.Interaction) -> discord.VoiceChannel | None:
        m: discord.Member = itx.user  # type: ignore
        lane = (
            m.voice.channel
            if (m.voice and isinstance(m.voice.channel, discord.VoiceChannel))
            else None
        )
        if lane is None:
            return None
        if lane.id in FIXED_LANE_IDS:
            return None
        return lane


class RegionDEButton(discord.ui.Button):
    def __init__(self, core):
        super().__init__(
            label="🇩🇪 DE",
            style=discord.ButtonStyle.primary,
            row=0,
            custom_id="tv_region_de",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        owner_id = self.core.lane_owner.get(lane.id, m.id)
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur Owner/Mods dürfen den Sprachfilter ändern.", ephemeral=True
            )
            return
        # Interaction sofort bestätigen, damit das Token nicht abläuft
        await itx.response.defer(ephemeral=True)
        await self.core.set_owner_region(owner_id, "DE")
        await self.core.apply_owner_region_to_lane(lane, owner_id)
        await itx.followup.send("Deutsch-Only aktiv.", ephemeral=True)


class RegionEUButton(discord.ui.Button):
    def __init__(self, core):
        super().__init__(
            label="🇪🇺 EU",
            style=discord.ButtonStyle.secondary,
            row=0,
            custom_id="tv_region_e",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        owner_id = self.core.lane_owner.get(lane.id, m.id)
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur Owner/Mods dürfen den Sprachfilter ändern.", ephemeral=True
            )
            return
        # Interaction sofort bestätigen, damit das Token nicht abläuft
        await itx.response.defer(ephemeral=True)
        await self.core.set_owner_region(owner_id, "EU")
        await self.core.apply_owner_region_to_lane(lane, owner_id)
        await itx.followup.send("Sprachfilter aufgehoben (EU).", ephemeral=True)


class OwnerClaimButton(discord.ui.Button):
    def __init__(self, core):
        super().__init__(
            label="👑 Owner Claim",
            style=discord.ButtonStyle.success,
            row=0,
            custom_id="tv_owner_claim",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        # Interaction sofort bestätigen, damit das Token nicht abläuft
        await itx.response.defer(ephemeral=True)
        ok, msg = await self.core.request_owner_claim(lane, m)
        await itx.followup.send(msg, ephemeral=True)


class LimitButton(discord.ui.Button):
    def __init__(self, core):
        super().__init__(
            label="🎚️ Limit setzen",
            style=discord.ButtonStyle.secondary,
            row=0,
            custom_id="tv_limit_btn",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        owner_id = self.core.lane_owner.get(lane.id, m.id)
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur Owner/Mods dürfen das Limit setzen.", ephemeral=True
            )
            return
        await itx.response.send_modal(LimitModal(self.core, lane))


class LimitModal(discord.ui.Modal, title="Limit setzen"):
    value = discord.ui.TextInput(
        label="Limit (0-99)", placeholder="z.B. 6", required=True, max_length=2
    )

    def __init__(self, core, lane: discord.VoiceChannel):
        super().__init__(timeout=120)
        self.core = core
        self.lane = lane

    async def on_submit(self, itx: discord.Interaction):
        txt = str(self.value.value).strip()
        try:
            val = int(txt)
        except ValueError:
            await itx.response.send_message("Bitte Zahl (0-99) eingeben.", ephemeral=True)
            return
        if val < 0 or val > 99:
            await itx.response.send_message("Limit muss 0-99 sein.", ephemeral=True)
            return
        try:
            enforced_val = self.core.enforce_limit(self.lane, val)  # type: ignore[attr-defined]
        except Exception:
            enforced_val = val
        try:
            await itx.response.defer(ephemeral=True, thinking=False)
        except discord.HTTPException as e:
            logger.debug("LimitModal: defer fehlgeschlagen: %r", e)
        except Exception as e:
            logger.debug("LimitModal: unerwarteter defer-Fehler: %r", e)
        await self.core.safe_edit_channel(
            self.lane, desired_limit=enforced_val, reason="TempVoice: Limit gesetzt"
        )
        await self.core.refresh_name(self.lane)
        msg = f"Limit auf {enforced_val} gesetzt."
        if enforced_val != val:
            msg += " (Maximal 4 in Street Brawl Lanes.)"
        try:
            await itx.followup.send(msg, ephemeral=True)
        except discord.HTTPException as e:
            logger.debug("LimitModal: followup.send fehlgeschlagen: %r", e)
        except Exception as e:
            logger.debug("LimitModal: unerwarteter followup-Fehler: %r", e)


class QuickTemplateButton(discord.ui.Button):
    def __init__(self, core, *, label: str, template_name: str, limit: int, custom_id: str):
        super().__init__(label=label, style=discord.ButtonStyle.primary, row=3, custom_id=custom_id)
        self.core = core
        self.template_name = template_name
        self.limit = limit

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        owner_id = self.core.lane_owner.get(lane.id, m.id)
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur Owner/Mods d�rfen Templates benutzen.", ephemeral=True
            )
            return
        await itx.response.defer(ephemeral=True)
        await self.core.set_lane_template(lane, base_name=self.template_name, limit=self.limit)
        await itx.followup.send(
            f"Lane auf {self.template_name} gestellt (Limit {self.limit}).",
            ephemeral=True,
        )


class TagFilterButton(discord.ui.Button):
    def __init__(self, core, row: int = 3):
        super().__init__(
            label="🛡️ Tag-Filter",
            style=discord.ButtonStyle.secondary,
            row=row,
            custom_id="tv_tag_filter",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        member: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        if not _is_owner_or_mod(self.core, lane, member):
            await itx.response.send_message(
                "Nur Owner/Mods dürfen den Tag-Filter ändern.",
                ephemeral=True,
            )
            return
        lane_filter = await self.core.get_lane_tag_filter(lane.id)
        await itx.response.send_message(
            "Tag-Filter konfigurieren:",
            view=TagFilterConfigView(self.core, lane, member, lane_filter),
            ephemeral=True,
        )


class TagFilterMinAgeSelect(discord.ui.Select):
    def __init__(self, lane_filter: LaneTagFilter):
        options = [
            discord.SelectOption(
                label="Aus",
                value="off",
                default=lane_filter.min_age_tag is None,
            ),
            discord.SelectOption(
                label="25+",
                value="25+",
                default=lane_filter.min_age_tag == "25+",
            ),
        ]
        super().__init__(
            placeholder="Mindest-Alter",
            min_values=1,
            max_values=1,
            options=options,
            row=0,
        )

    async def callback(self, itx: discord.Interaction):
        view: TagFilterConfigView = self.view  # type: ignore[assignment]
        view.min_age_tag = None if self.values[0] == "off" else self.values[0]
        await itx.response.defer()


class TagFilterToneSelect(discord.ui.Select):
    def __init__(self, lane_filter: LaneTagFilter):
        options = [
            discord.SelectOption(
                label="Aus",
                value="off",
                default=lane_filter.required_tone_tag is None,
            ),
            discord.SelectOption(
                label="Ragebaiter-Free",
                value="ragebaiter_free",
                default=lane_filter.required_tone_tag == "ragebaiter_free",
            ),
        ]
        super().__init__(
            placeholder="Tonfall",
            min_values=1,
            max_values=1,
            options=options,
            row=1,
        )

    async def callback(self, itx: discord.Interaction):
        view: TagFilterConfigView = self.view  # type: ignore[assignment]
        view.required_tone_tag = None if self.values[0] == "off" else self.values[0]
        await itx.response.defer()


class TagFilterRagebaiterSelect(discord.ui.Select):
    def __init__(self, lane_filter: LaneTagFilter):
        options = [
            discord.SelectOption(
                label="Aus",
                value="off",
                default=not lane_filter.deny_ragebaiter,
            ),
            discord.SelectOption(
                label="An",
                value="on",
                default=lane_filter.deny_ragebaiter,
            ),
        ]
        super().__init__(
            placeholder="Ragebaiter blockieren",
            min_values=1,
            max_values=1,
            options=options,
            row=2,
        )

    async def callback(self, itx: discord.Interaction):
        view: TagFilterConfigView = self.view  # type: ignore[assignment]
        view.deny_ragebaiter = self.values[0] == "on"
        await itx.response.defer()


class TagFilterSaveButton(discord.ui.Button):
    def __init__(self):
        super().__init__(
            label="Speichern",
            style=discord.ButtonStyle.success,
            row=3,
        )

    async def callback(self, itx: discord.Interaction):
        view: TagFilterConfigView = self.view  # type: ignore[assignment]
        await view.save(itx)


class TagFilterConfigView(discord.ui.View):
    def __init__(
        self,
        core,
        lane: discord.VoiceChannel,
        requester: discord.Member,
        lane_filter: LaneTagFilter,
    ):
        super().__init__(timeout=120)
        self.core = core
        self.lane = lane
        self.requester = requester
        self.min_age_tag = lane_filter.min_age_tag
        self.required_tone_tag = lane_filter.required_tone_tag
        self.deny_ragebaiter = lane_filter.deny_ragebaiter
        self.add_item(TagFilterMinAgeSelect(lane_filter))
        self.add_item(TagFilterToneSelect(lane_filter))
        self.add_item(TagFilterRagebaiterSelect(lane_filter))
        self.add_item(TagFilterSaveButton())

    async def interaction_check(self, itx: discord.Interaction) -> bool:
        member: discord.Member = itx.user  # type: ignore
        if member.id == self.requester.id:
            return True
        if _is_owner_or_mod(self.core, self.lane, member):
            return True
        await itx.response.send_message(
            "Nur Owner/Mods dürfen den Tag-Filter ändern.",
            ephemeral=True,
        )
        return False

    async def save(self, itx: discord.Interaction) -> None:
        await itx.response.defer(ephemeral=True)
        lane_filter = await self.core.set_lane_tag_filter(
            self.lane.id,
            min_age_tag=self.min_age_tag,
            required_tone_tag=self.required_tone_tag,
            deny_ragebaiter=self.deny_ragebaiter,
        )
        await self.core._apply_tag_filter(self.lane, lane_filter)  # type: ignore[attr-defined]
        summary = _format_tag_filter_summary(lane_filter)
        try:
            if isinstance(itx.channel, discord.TextChannel):
                await itx.channel.send(summary)
        except Exception as exc:
            logger.debug("TagFilterConfigView channel send failed: %r", exc)
        await itx.followup.send(summary, ephemeral=True)
        self.stop()


class DuoCallButton(QuickTemplateButton):
    def __init__(self, core):
        super().__init__(
            core,
            label="Duo Call (2)",
            template_name="Duo Call",
            limit=2,
            custom_id="tv_tpl_duo",
        )


class TrioCallButton(QuickTemplateButton):
    def __init__(self, core):
        super().__init__(
            core,
            label="Trio Call (3)",
            template_name="Trio Call",
            limit=3,
            custom_id="tv_tpl_trio",
        )


class ResetLaneButton(discord.ui.Button):
    def __init__(self, core):
        super().__init__(
            label="Normale Lane",
            style=discord.ButtonStyle.secondary,
            row=3,
            custom_id="tv_tpl_reset",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        owner_id = self.core.lane_owner.get(lane.id, m.id)
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur Owner/Mods d�rfen Templates benutzen.", ephemeral=True
            )
            return
        await itx.response.defer(ephemeral=True)
        base, limit = await self.core.reset_lane_template(lane)
        await itx.followup.send(
            f"Lane auf {base} zur�ckgesetzt (Limit {limit}).",
            ephemeral=True,
        )


class SavePresetButton(discord.ui.Button):
    def __init__(self, core, row: int = 4):
        super().__init__(
            label="💾 Preset speichern",
            style=discord.ButtonStyle.success,
            row=row,
            custom_id="tv_preset_save",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        owner_id = self.core.lane_owner.get(lane.id, m.id)
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur Owner/Mods d\u00fcrfen Presets speichern.", ephemeral=True
            )
            return
        base = self.core.lane_base.get(lane.id) or lane.name
        modal = SavePresetModal(self.core, lane, owner_id, preset_name_hint=base)
        await itx.response.send_modal(modal)


class SavePresetModal(discord.ui.Modal, title="Preset speichern"):
    preset_name = discord.ui.TextInput(
        label="Preset-Name",
        placeholder="z.B. Lane 1 / Asc Duo",
        max_length=64,
        required=True,
    )

    def __init__(self, core, lane: discord.VoiceChannel, owner_id: int, preset_name_hint: str):
        super().__init__(timeout=120)
        self.core = core
        self.lane = lane
        self.owner_id = owner_id
        # Vorbelegen (nur visuell)
        try:
            self.preset_name.default = preset_name_hint[:64]
        except Exception as exc:
            logger.debug("SavePresetModal default value konnte nicht gesetzt werden: %r", exc)

    async def on_submit(self, itx: discord.Interaction):
        name = str(self.preset_name.value).strip()
        if not name:
            await itx.response.send_message("Name darf nicht leer sein.", ephemeral=True)
            return
        try:
            await self.core.save_preset(self.lane, self.owner_id, name)
        except Exception as e:
            logger.debug("SavePresetModal failed: %r", e)
            await itx.response.send_message(
                "Preset konnte nicht gespeichert werden.", ephemeral=True
            )
            return
        await itx.response.send_message(f'Preset "{name}" gespeichert.', ephemeral=True)


class LoadPresetButton(discord.ui.Button):
    def __init__(self, core, row: int = 4):
        super().__init__(
            label="\ud83d\uddc2 Preset laden",
            style=discord.ButtonStyle.secondary,
            row=row,
            custom_id="tv_preset_load",
        )
        self.core = core

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        if lane.category_id != RANKED_CATEGORY_ID:
            await itx.response.send_message(
                "Presets gibt es nur f\u00fcr Ranked Lanes.", ephemeral=True
            )
            return
        owner_id = self.core.lane_owner.get(lane.id, m.id)
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur Owner/Mods d\u00fcrfen Presets laden.", ephemeral=True
            )
            return
        presets = await self.core.list_presets(owner_id, int(lane.category_id or 0))
        if not presets:
            await itx.response.send_message(
                "Du hast noch keine Presets gespeichert.", ephemeral=True
            )
            return
        options = []
        for row in presets[:25]:  # Discord Select max 25 Optionen
            label = f"{row['name']}"
            min_rank = row.get("min_rank") if isinstance(row, dict) else None
            min_rank = min_rank or "unknown"
            min_part = "Kein Limit" if min_rank == "unknown" else f"Min {min_rank}"
            region = row.get("region") if isinstance(row, dict) else None
            region_part = "DE" if region == "DE" else "EU"
            desc = f"{row['base_name']} • Limit {row['limit']} • {min_part} • {region_part}"
            options.append(
                discord.SelectOption(label=label[:100], value=row["name"], description=desc[:100])
            )
        view = PresetSelectView(
            self.core,
            lane,
            owner_id,
            options,
            batch=False,
            category=lane.category,
            requester=m,
        )
        await itx.response.send_message("Preset ausw\u00e4hlen:", view=view, ephemeral=True)


class PresetSelect(discord.ui.Select):
    def __init__(self, options):
        super().__init__(
            min_values=1, max_values=1, options=options, row=0, custom_id="tv_preset_pick"
        )

    async def callback(self, itx: discord.Interaction):
        view: PresetSelectView = self.view  # type: ignore
        await view.apply(itx, self.values[0])


class PresetSelectView(discord.ui.View):
    def __init__(
        self,
        core,
        lane: discord.VoiceChannel,
        owner_id: int,
        options,
        *,
        batch: bool = False,
        category: discord.CategoryChannel | None = None,
        requester: discord.Member | None = None,
    ):
        super().__init__(timeout=60)
        self.core = core
        self.lane = lane
        self.owner_id = owner_id
        self.batch = batch
        self.category = category
        self.requester = requester
        self.add_item(PresetSelect(options))

    async def apply(self, itx: discord.Interaction, preset_name: str):
        ok = await self.core.apply_preset(self.lane, self.owner_id, preset_name)
        if not ok:
            await itx.response.send_message(
                "Preset nicht gefunden oder Fehler beim Anwenden.", ephemeral=True
            )
            return
        await self.core.refresh_name(self.lane)
        await itx.response.send_message(f'Preset "{preset_name}" angewendet.', ephemeral=True)


class MinRankSelect(discord.ui.Select):
    def __init__(self, core):
        self.core = core
        guild = None
        ref_guild = self.core.first_guild()
        if ref_guild:
            guild = ref_guild
        options = []
        for r in RANK_ORDER[1:]:
            options.append(
                discord.SelectOption(
                    label=r.capitalize(), value=r, emoji=_find_rank_emoji(guild, r)
                )
            )
        super().__init__(
            placeholder="① Haupt-Rang wählen →",
            min_values=1,
            max_values=1,
            options=options,
            row=2,
            custom_id="tv_minrank",
        )

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return
        verified_role_id = getattr(settings, "verified_role_id", None)
        if verified_role_id:
            has_verified = any(r.id == verified_role_id for r in getattr(m, "roles", []))
            if not has_verified:
                await itx.response.send_message(
                    "Du kannst den Mindest-Rang nur setzen, wenn du verifiziert bist.",
                    ephemeral=True,
                )
                return
        if getattr(self.core, "is_min_rank_blocked", None) and self.core.is_min_rank_blocked(lane):  # type: ignore[attr-defined]
            await itx.response.send_message("Mindest-Rang ist hier deaktiviert.", ephemeral=True)
            return
        if lane.category_id not in MINRANK_CATEGORY_IDS:
            await itx.response.send_message("Mindest-Rang ist hier deaktiviert.", ephemeral=True)
            return
        choice = self.values[0]

        try:
            choice_idx = RANK_ORDER.index(choice)
        except ValueError:
            choice_idx = 0
        member_rank_idx = _resolve_member_rank_index(self.core, m)
        if choice_idx > member_rank_idx:
            user_rank_label = (
                RANK_ORDER[member_rank_idx].capitalize()
                if member_rank_idx < len(RANK_ORDER)
                else "Unknown"
            )
            await itx.response.send_message(
                f"Du kannst keinen Mindest-Rang über deinem eigenen setzen. Dein Rang: {user_rank_label}.",
                ephemeral=True,
            )
            return

        # Haupt-Rang speichern – Sub-Rang muss noch über ② gewählt werden
        _pending_main_rank[lane.id] = choice
        try:
            await itx.response.send_message(
                f"Haupt-Rang **{choice.capitalize()}** gespeichert – jetzt **② Sub-Rang (1–6)** auswählen.",
                ephemeral=True,
            )
        except discord.HTTPException as e:
            logger.debug("MinRankSelect: send fehlgeschlagen: %r", e)


class SubRankSelectPermanent(discord.ui.Select):
    """Dauerhafter Sub-Rang Selektor in der MainView (Row 4, nur Ranked).
    Kombiniert mit dem gewählten Haupt-Rang aus _pending_main_rank und wendet diesen an.
    """

    def __init__(self, core):
        self.core = core
        options = [discord.SelectOption(label=f"Sub-Rang {n}", value=str(n)) for n in range(1, 7)]
        super().__init__(
            placeholder="② Sub-Rang wählen (1–6)",
            min_values=1,
            max_values=1,
            options=options,
            row=4,
            custom_id="tv_subrank_perm",
        )

    async def callback(self, itx: discord.Interaction):
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Tritt zuerst deiner Lane bei.", ephemeral=True)
            return

        main_rank = _pending_main_rank.get(lane.id)
        if not main_rank:
            await itx.response.send_message(
                "Bitte zuerst den **① Haupt-Rang** auswählen.", ephemeral=True
            )
            return

        subrank = int(self.values[0])
        rank_label = f"{main_rank} {subrank}"
        _pending_main_rank.pop(lane.id, None)

        try:
            await itx.response.defer(ephemeral=True, thinking=False)
        except discord.HTTPException as e:
            logger.debug("SubRankSelectPermanent: defer fehlgeschlagen: %r", e)

        self.core.lane_min_rank[lane.id] = rank_label
        await self.core._apply_min_rank(lane, rank_label)  # type: ignore[attr-defined]
        await self.core.refresh_name(lane)

        try:
            await itx.followup.send(
                f"Mindest-Rang gesetzt auf: **{rank_label.title()}**.", ephemeral=True
            )
        except Exception as e:
            logger.debug("SubRankSelectPermanent: followup fehlgeschlagen: %r", e)


class SubRankSelect(discord.ui.Select):
    def __init__(self):
        options = [
            discord.SelectOption(label="Kein Sub-Rang (nur Hauptrang)", value="0"),
        ]
        for n in range(1, 7):
            options.append(discord.SelectOption(label=f"Sub-Rang {n}", value=str(n)))
        super().__init__(
            placeholder="Sub-Rang wählen (optional)",
            min_values=1,
            max_values=1,
            options=options,
            row=0,
            custom_id="tv_subrank",
        )

    async def callback(self, itx: discord.Interaction):
        view: SubRankSelectView = self.view  # type: ignore
        await view.apply(itx, int(self.values[0]))


class SubRankSelectView(discord.ui.View):
    def __init__(self, core, lane: discord.VoiceChannel, base_rank: str, requester: discord.Member):
        super().__init__(timeout=60)
        self.core = core
        self.lane = lane
        self.base_rank = base_rank
        self.requester = requester
        self.add_item(SubRankSelect())

    async def apply(self, itx: discord.Interaction, subrank: int):
        # Safety: only requester or mods/admins can finalize
        m: discord.Member = itx.user  # type: ignore
        perms = self.lane.permissions_for(m)
        if not (m.id == self.requester.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message(
                "Nur der ursprüngliche Auslöser oder Mods dürfen bestätigen.", ephemeral=True
            )
            return

        rank_label = self.base_rank
        if subrank > 0:
            rank_label = f"{self.base_rank} {subrank}"

        try:
            await itx.response.defer(ephemeral=True, thinking=False)
        except Exception as exc:
            logger.debug("SubRankSelect defer fehlgeschlagen: %r", exc)

        self.core.lane_min_rank[self.lane.id] = rank_label
        await self.core._apply_min_rank(self.lane, rank_label)  # type: ignore[attr-defined]
        await self.core.refresh_name(self.lane)
        label = "Kein Limit" if rank_label == "unknown" else rank_label.capitalize()
        try:
            await itx.followup.send(f"Mindest-Rang gesetzt auf: {label}.", ephemeral=True)
        except Exception as e:
            logger.debug("SubRankSelect followup fehlgeschlagen: %r", e)


class KickButton(discord.ui.Button):
    def __init__(self, util):
        super().__init__(
            label="👢 Kick",
            style=discord.ButtonStyle.secondary,
            row=1,
            custom_id="tv_kick",
        )
        self.util = util

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Du musst in einer Lane sein.", ephemeral=True)
            return
        owner_id = itx.client.get_cog("TempVoiceCore").lane_owner.get(lane.id, m.id)  # type: ignore
        perms = lane.permissions_for(m)
        if not (owner_id == m.id or perms.manage_channels or perms.administrator):
            await itx.response.send_message("Nur Owner/Mods dürfen kicken.", ephemeral=True)
            return
        options = [
            discord.SelectOption(label=u.display_name, value=str(u.id))
            for u in lane.members
            if u.id != m.id
        ]
        if not options:
            await itx.response.send_message("Niemand zum Kicken vorhanden.", ephemeral=True)
            return
        view = KickSelectView(self.util, lane, options)
        await itx.response.send_message("Wen möchtest du kicken?", view=view, ephemeral=True)


class KickSelect(discord.ui.Select):
    def __init__(self, options, placeholder="Mitglied wählen …"):
        super().__init__(min_values=1, max_values=1, options=options, placeholder=placeholder)

    async def callback(self, itx: discord.Interaction):
        view: KickSelectView = self.view  # type: ignore
        await view.handle_kick(itx, int(self.values[0]))


class KickSelectView(discord.ui.View):
    def __init__(self, util, lane: discord.VoiceChannel, options):
        super().__init__(timeout=60)
        self.util = util
        self.lane = lane
        self.add_item(KickSelect(options))

    async def handle_kick(self, itx: discord.Interaction, target_id: int):
        actor = itx.user if isinstance(itx.user, (discord.Member, discord.User)) else None
        actor_member = None
        if actor is not None:
            actor_member = self.lane.guild.get_member(actor.id)
        if actor_member is None:
            logger.warning(
                "TempVoice kick denied: actor missing in guild actor=%s actor_id=%s target_id=%s lane_id=%s",
                str(actor) if actor else "unknown",
                getattr(actor, "id", None),
                target_id,
                self.lane.id,
            )
            await itx.response.send_message(
                "Konnte deine Berechtigung nicht mehr prüfen.", ephemeral=True
            )
            return
        owner_id = itx.client.get_cog("TempVoiceCore").lane_owner.get(self.lane.id, actor_member.id)  # type: ignore
        perms = self.lane.permissions_for(actor_member)
        if not (owner_id == actor_member.id or perms.manage_channels or perms.administrator):
            logger.warning(
                "TempVoice kick denied: actor lacks permission actor=%s actor_id=%s target_id=%s lane_id=%s",
                str(actor_member),
                actor_member.id,
                target_id,
                self.lane.id,
            )
            await itx.response.send_message("Nur Owner/Mods dürfen kicken.", ephemeral=True)
            return
        logger.info(
            "TempVoice kick requested: actor=%s actor_id=%s target_id=%s lane=%s lane_id=%s",
            str(actor_member),
            actor_member.id,
            target_id,
            self.lane.name,
            self.lane.id,
        )
        await itx.response.defer(ephemeral=True, thinking=False)
        ok, msg = await self.util.kick(self.lane, target_id, actor=actor_member)
        await itx.followup.send(msg, ephemeral=True)


class BanSelect(discord.ui.Select):
    def __init__(self, options):
        super().__init__(
            min_values=1, max_values=1, options=options, placeholder="Mitglied bannen …"
        )

    async def callback(self, itx: discord.Interaction):
        view: BanSelectView = self.view  # type: ignore
        await view.handle_ban(itx, int(self.values[0]))


class BanSelectView(discord.ui.View):
    def __init__(self, util, lane: discord.VoiceChannel, options):
        super().__init__(timeout=60)
        self.util = util
        self.lane = lane
        self.add_item(BanSelect(options))

    async def handle_ban(self, itx: discord.Interaction, target_id: int):
        core = itx.client.get_cog("TempVoiceCore")  # type: ignore
        owner_id = core.lane_owner.get(self.lane.id)
        await itx.response.defer(ephemeral=True, thinking=False)
        ok, msg = await self.util.ban(self.lane, owner_id, str(target_id))
        await itx.followup.send(msg, ephemeral=True)


class UnbanSelect(discord.ui.Select):
    def __init__(self, options):
        super().__init__(
            min_values=1, max_values=1, options=options, placeholder="User entbannen …"
        )

    async def callback(self, itx: discord.Interaction):
        view: UnbanSelectView = self.view  # type: ignore
        await view.handle_unban(itx, int(self.values[0]))


class UnbanSelectView(discord.ui.View):
    def __init__(self, util, lane: discord.VoiceChannel, options):
        super().__init__(timeout=60)
        self.util = util
        self.lane = lane
        self.add_item(UnbanSelect(options))

    async def handle_unban(self, itx: discord.Interaction, target_id: int):
        core = itx.client.get_cog("TempVoiceCore")  # type: ignore
        owner_id = core.lane_owner.get(self.lane.id)
        await itx.response.defer(ephemeral=True, thinking=False)
        ok, msg = await self.util.unban(self.lane, owner_id, str(target_id))
        await itx.followup.send(msg, ephemeral=True)


class BanButton(discord.ui.Button):
    def __init__(self, util):
        super().__init__(
            label="🚫 Ban", style=discord.ButtonStyle.danger, row=1, custom_id="tv_ban"
        )
        self.util = util

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Du musst in einer Lane sein.", ephemeral=True)
            return
        owner_id = itx.client.get_cog("TempVoiceCore").lane_owner.get(lane.id)  # type: ignore
        if owner_id is None:
            await itx.response.send_message("Aktuell ist kein Owner gesetzt.", ephemeral=True)
            return
        perms = lane.permissions_for(m)
        if owner_id != m.id and not perms.administrator:
            await itx.response.send_message("Nur der Owner darf bannen.", ephemeral=True)
            return
        options = [
            discord.SelectOption(label=u.display_name, value=str(u.id))
            for u in lane.members
            if u.id != m.id
        ]
        if not options:
            await itx.response.send_message("Niemand zum Bannen vorhanden.", ephemeral=True)
            return
        view = BanSelectView(self.util, lane, options)
        await itx.response.send_message("Wen möchtest du bannen?", view=view, ephemeral=True)


class UnbanButton(discord.ui.Button):
    def __init__(self, util):
        super().__init__(
            label="♻️ Unban",
            style=discord.ButtonStyle.primary,
            row=1,
            custom_id="tv_unban",
        )
        self.util = util

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Du musst in einer Lane sein.", ephemeral=True)
            return
        core = itx.client.get_cog("TempVoiceCore")  # type: ignore
        owner_id = core.lane_owner.get(lane.id)
        if owner_id is None:
            await itx.response.send_message("Aktuell ist kein Owner gesetzt.", ephemeral=True)
            return
        perms = lane.permissions_for(m)
        if owner_id != m.id and not perms.administrator:
            await itx.response.send_message("Nur der Owner darf entbannen.", ephemeral=True)
            return
        ban_list = await core.bans.list_bans(owner_id)
        if not ban_list:
            await itx.response.send_message("Keine aktiven Bans vorhanden.", ephemeral=True)
            return
        options = []
        for uid in ban_list[:25]:
            member = lane.guild.get_member(uid)
            label = member.display_name if member else str(uid)
            options.append(discord.SelectOption(label=label, value=str(uid)))
        view = UnbanSelectView(self.util, lane, options)
        await itx.response.send_message("Wen möchtest du entbannen?", view=view, ephemeral=True)


class BanModal(discord.ui.Modal, title="User (Un)Ban"):
    target = discord.ui.TextInput(
        label="User (@Mention/Name/ID)",
        placeholder="@Name oder 123456789012345678",
        required=True,
        max_length=64,
    )

    def __init__(self, util, lane: discord.VoiceChannel, action: str):
        super().__init__(timeout=120)
        self.util = util
        self.lane = lane
        self.action = action

    async def on_submit(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        owner_id = itx.client.get_cog("TempVoiceCore").lane_owner.get(self.lane.id)  # type: ignore
        if owner_id is None:
            await itx.response.send_message("Aktuell ist kein Owner gesetzt.", ephemeral=True)
            return
        perms = self.lane.permissions_for(m)
        if owner_id != m.id and not perms.administrator:
            await itx.response.send_message("Nur der Owner darf (un)bannen.", ephemeral=True)
            return
        raw = str(self.target.value).strip()
        try:
            await itx.response.defer(ephemeral=True, thinking=False)
        except discord.HTTPException as e:
            logger.debug("BanModal: defer fehlgeschlagen: %r", e)
        except Exception as e:
            logger.debug("BanModal: unerwarteter defer-Fehler: %r", e)

        if self.action == "ban":
            ok, msg = await self.util.ban(self.lane, owner_id, raw)
        else:
            ok, msg = await self.util.unban(self.lane, owner_id, raw)
        try:
            await itx.followup.send(msg, ephemeral=True)
        except discord.HTTPException as e:
            logger.debug("BanModal: followup.send fehlgeschlagen: %r", e)
        except Exception as e:
            logger.debug("BanModal: unerwarteter followup-Fehler: %r", e)


class LurkerButton(discord.ui.Button):
    def __init__(self, util, row: int = 3):
        super().__init__(
            label="👻 Lurker",
            style=discord.ButtonStyle.secondary,
            row=row,
            custom_id="tv_lurker",
        )
        self.util = util

    async def callback(self, itx: discord.Interaction):
        m: discord.Member = itx.user  # type: ignore
        lane = MainView.lane_of(itx)
        if not lane:
            await itx.response.send_message("Du musst in einer Lane sein.", ephemeral=True)
            return

        await itx.response.defer(ephemeral=True, thinking=False)
        ok, msg = await self.util.toggle_lurker(lane, m)
        await itx.followup.send(msg, ephemeral=True)
