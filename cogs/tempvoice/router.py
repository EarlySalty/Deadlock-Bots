from __future__ import annotations

import asyncio
import logging

import discord
from discord.ext import commands

from service import db
from service.guild_config import get_guild_config

log = logging.getLogger("cogs.tempvoice.router")

VERIFIED_RANK_ROLE_IDS: set[int] = {
    1331457571118387210,
    1331457652877955072,
    1331457699992436829,
    1331457724848017539,
    1331457879345070110,
    1331457898781474836,
    1331457949654319114,
    1316966867033653338,
    1331458016356208680,
    1331458049637875785,
    1331458087349129296,
}

_CREATE_TABLE = """
CREATE TABLE IF NOT EXISTS router_user_prefs (
    user_id    INTEGER PRIMARY KEY,
    mode       TEXT NOT NULL CHECK(mode IN ('ranked', 'casual', 'street_brawl')),
    auto_join  INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
"""


def _mode_to_category(mode: str) -> int:
    cfg = get_guild_config()
    return {
        "ranked": cfg.TEMPVOICE_CATEGORY_COMP,
        "casual": cfg.TEMPVOICE_CATEGORY_CHILL,
        "street_brawl": cfg.TEMPVOICE_CATEGORY_STREET_BRAWL,
    }.get(mode, cfg.TEMPVOICE_CATEGORY_CHILL)


class RouterCog(commands.Cog, name="RouterCog"):
    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self._locks: dict[int, asyncio.Lock] = {}

    async def cog_load(self) -> None:
        await db.execute_async(_CREATE_TABLE)

    # ── DB helpers ───────────────────────────────────────────────────────────

    async def get_user_pref(self, user_id: int) -> dict | None:
        row = await db.query_one_async(
            "SELECT mode, auto_join FROM router_user_prefs WHERE user_id = ?",
            (user_id,),
        )
        if row is None:
            return None
        return {"mode": row[0], "auto_join": bool(row[1])}

    async def set_user_pref(self, user_id: int, mode: str, auto_join: bool) -> None:
        await db.execute_async(
            """INSERT INTO router_user_prefs (user_id, mode, auto_join, updated_at)
               VALUES (?, ?, ?, CURRENT_TIMESTAMP)
               ON CONFLICT(user_id) DO UPDATE SET
                   mode       = excluded.mode,
                   auto_join  = excluded.auto_join,
                   updated_at = CURRENT_TIMESTAMP""",
            (user_id, mode, int(auto_join)),
        )

    def has_verified_rank(self, member: discord.Member) -> bool:
        return any(r.id in VERIFIED_RANK_ROLE_IDS for r in member.roles)

    # ── Voice routing ────────────────────────────────────────────────────────

    @commands.Cog.listener()
    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ) -> None:
        cfg = get_guild_config()
        if after.channel is None or after.channel.id != cfg.TEMPVOICE_ROUTER_VC:
            return
        lock = self._locks.setdefault(member.id, asyncio.Lock())
        if lock.locked():
            return
        async with lock:
            await self._handle_router_join(member)

    async def _handle_router_join(self, member: discord.Member) -> None:
        guild = member.guild
        cfg = get_guild_config()
        router_vc = guild.get_channel(cfg.TEMPVOICE_ROUTER_VC)

        # Neue-Spieler-Check zuerst
        new_player_cog = self.bot.get_cog("NewPlayerAdaptiveLanes")
        if new_player_cog is not None and router_vc is not None:
            try:
                rerouted = await new_player_cog.maybe_route_new_player(member, router_vc)
            except Exception as e:
                log.debug("new_player routing hook failed for %s: %r", member.id, e)
                rerouted = False
            if rerouted:
                return

        pref = await self.get_user_pref(member.id)

        # Keine Präferenz → User bleibt im Router-VC
        if pref is None:
            log.debug("RouterCog: %s hat keine Präferenz, bleibt im Router-VC", member.id)
            return

        if not pref["auto_join"]:
            # Auto-Join deaktiviert → einfache private Lane
            core = self.bot.get_cog("TempVoiceCore")
            if core is not None:
                await core._create_lane_for_router(member, pref["mode"])
            return

        await self._smart_route(member, pref["mode"])

    async def _smart_route(self, member: discord.Member, mode: str) -> None:
        guild = member.guild
        cfg = get_guild_config()

        if mode == "ranked" and not self.has_verified_rank(member):
            info_ch = guild.get_channel(cfg.TEMPVOICE_RANKED_INFO_CHANNEL)
            try:
                mention = info_ch.mention if info_ch else "den Info-Kanal im Server"
                await member.send(
                    f"Für Ranked-Lanes musst du deinen Rang verifizieren. "
                    f"Mehr Infos: {mention}"
                )
            except discord.Forbidden:
                pass
            log.debug("RouterCog: %s hat keinen verifizierten Rang für Ranked", member.id)
            return

        category_id = _mode_to_category(mode)
        lane = self._find_suitable_lane(guild, category_id)

        if lane is not None:
            try:
                await member.move_to(lane, reason="Router: passende Lane gefunden")
                log.info("RouterCog: %s → Lane %s (%s)", member.id, lane.id, mode)
            except (discord.Forbidden, discord.HTTPException) as e:
                log.warning("RouterCog: Move von %s fehlgeschlagen: %r", member.id, e)
            return

        core = self.bot.get_cog("TempVoiceCore")
        if core is not None:
            await core._create_lane_for_router(member, mode)

    def _find_suitable_lane(
        self,
        guild: discord.Guild,
        category_id: int,
        max_members: int = 6,
    ) -> discord.VoiceChannel | None:
        cat = guild.get_channel(category_id)
        if not isinstance(cat, discord.CategoryChannel):
            return None
        cfg = get_guild_config()
        skip_ids = {cfg.TEMPVOICE_STAGING_CASUAL, cfg.TEMPVOICE_STAGING_COMP,
                    cfg.TEMPVOICE_STAGING_STREET_BRAWL}
        for ch in cat.voice_channels:
            if ch.id in skip_ids:
                continue
            if 0 < len(ch.members) < max_members:
                return ch
        return None


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(RouterCog(bot))
