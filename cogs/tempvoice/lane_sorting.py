from __future__ import annotations

import asyncio
import logging
import re
from dataclasses import dataclass
from typing import TYPE_CHECKING

import discord
from discord.ext import commands

from service.guild_config import get_guild_config

from .core import FIXED_LANE_IDS, MINRANK_CATEGORY_IDS, RANK_ORDER, _member_rank_index, _rank_index

if TYPE_CHECKING:
    from .core import TempVoiceCore


log = logging.getLogger("TempVoiceLaneSorting")

_cfg = get_guild_config()
SUPPORTED_CATEGORY_IDS: set[int] = {
    _cfg.TEMPVOICE_CATEGORY_CHILL,
    _cfg.TEMPVOICE_CATEGORY_COMP,
}
CHILL_STAGING_CHANNEL_ID = _cfg.TEMPVOICE_STAGING_CASUAL
PERMANENT_CHILL_LANE_ID = _cfg.TEMPVOICE_PERMANENT_CASUAL_CHANNEL
PINNED_LAST_CHANNEL_IDS: set[int] = {1505618194017161267}
REORDER_DEBOUNCE_SECONDS = 2.0
STARTUP_REORDER_DELAY_SECONDS = 5.0
RANK_LABEL_RE = re.compile(
    rf"^\s*(?P<rank>{'|'.join(re.escape(rank) for rank in RANK_ORDER[1:])})"
    r"(?:\s+(?P<subrank>[1-6]))?\b",
    re.IGNORECASE,
)


@dataclass(frozen=True, slots=True)
class LaneSortSnapshot:
    lane_id: int
    current_position: int
    rank_index: int
    subrank: int
    stable_order: int

    @property
    def sort_key(self) -> tuple[int, int, int, int]:
        return (self.rank_index, self.subrank, self.stable_order, self.lane_id)


def parse_rank_label(label: str | None) -> tuple[int, int]:
    """Extrahiert Hauptrang und optionalen Subrank aus einem Lane-Label."""
    if not label:
        return 0, 0
    match = RANK_LABEL_RE.match(str(label).strip())
    if not match:
        return 0, 0
    rank_index = _rank_index(match.group("rank"))
    if rank_index <= 0:
        return 0, 0
    subrank_raw = match.group("subrank")
    subrank = int(subrank_raw) if subrank_raw else 0
    return rank_index, subrank


def plan_lane_reorder(entries: list[LaneSortSnapshot]) -> list[tuple[int, int]]:
    """Plant Zielpositionen für die sortierbaren Lane-Slots einer Kategorie."""
    if len(entries) <= 1:
        return []
    slot_positions = sorted(entry.current_position for entry in entries)
    ordered_entries = sorted(entries, key=lambda entry: entry.sort_key)
    moves: list[tuple[int, int]] = []
    for idx, entry in enumerate(ordered_entries):
        target_position = slot_positions[idx]
        if entry.current_position != target_position:
            moves.append((entry.lane_id, target_position))
    return moves


class TempVoiceLaneSorting(commands.Cog):
    """Sortiert TempVoice-Lanes innerhalb der Chill-/Comp-Kategorien nach Rang."""

    def __init__(self, bot: commands.Bot, core: TempVoiceCore):
        self.bot = bot
        self.core = core
        self._dirty_categories: set[tuple[int, int]] = set()
        self._category_tasks: dict[tuple[int, int], asyncio.Task] = {}
        self._category_locks: dict[tuple[int, int], asyncio.Lock] = {}
        self._startup_task: asyncio.Task | None = None

    async def cog_load(self) -> None:
        self._startup_task = asyncio.create_task(self._schedule_startup_reorders())

    async def cog_unload(self) -> None:
        if self._startup_task and not self._startup_task.done():
            self._startup_task.cancel()
        for task in list(self._category_tasks.values()):
            if not task.done():
                task.cancel()
        self._category_tasks.clear()
        self._dirty_categories.clear()

    async def _schedule_startup_reorders(self) -> None:
        try:
            await self.bot.wait_until_ready()
            await asyncio.sleep(STARTUP_REORDER_DELAY_SECONDS)
            for guild in self.bot.guilds:
                for category_id in SUPPORTED_CATEGORY_IDS:
                    self.schedule_category_reorder(guild.id, category_id)
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            log.debug("startup reorder scheduling failed: %r", exc)

    def schedule_category_reorder(self, guild_id: int, category_id: int | None) -> None:
        if not guild_id or not category_id or category_id not in SUPPORTED_CATEGORY_IDS:
            return
        key = (int(guild_id), int(category_id))
        self._dirty_categories.add(key)
        task = self._category_tasks.get(key)
        if task and not task.done():
            return
        self._category_tasks[key] = asyncio.create_task(self._drain_reorders(key))

    async def _drain_reorders(self, key: tuple[int, int]) -> None:
        try:
            while True:
                await asyncio.sleep(REORDER_DEBOUNCE_SECONDS)
                self._dirty_categories.discard(key)
                await self._reorder_category(*key)
                if key not in self._dirty_categories:
                    break
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            log.warning("lane sorting failed for category %s/%s: %s", key[0], key[1], exc)
        finally:
            self._dirty_categories.discard(key)
            self._category_tasks.pop(key, None)

    async def _reorder_category(self, guild_id: int, category_id: int) -> None:
        guild = self.bot.get_guild(int(guild_id))
        if not guild:
            return
        category = guild.get_channel(int(category_id))
        if not isinstance(category, discord.CategoryChannel):
            return

        key = (int(guild_id), int(category_id))
        lock = self._category_locks.setdefault(key, asyncio.Lock())
        async with lock:
            reserved_start_position: int | None = None
            if category_id == _cfg.TEMPVOICE_CATEGORY_CHILL:
                reserved_start_position = await self._ensure_reserved_chill_slot(guild, category)

            lanes = [lane for lane in category.voice_channels if self._should_sort_lane(lane)]
            pinned_channels = [
                ch for ch in category.voice_channels if ch.id in PINNED_LAST_CHANNEL_IDS
            ]
            if len(lanes) <= 1 and not pinned_channels:
                return

            snapshots: list[LaneSortSnapshot] = []
            for lane in lanes:
                rank_index, subrank = await self._resolve_lane_rank(lane)
                snapshots.append(
                    LaneSortSnapshot(
                        lane_id=int(lane.id),
                        current_position=int(lane.position),
                        rank_index=int(rank_index),
                        subrank=int(subrank),
                        stable_order=int(lane.position),
                    )
                )

            if reserved_start_position is not None:
                ordered_entries = sorted(snapshots, key=lambda entry: entry.sort_key)
                moves = []
                for idx, entry in enumerate(ordered_entries):
                    target_position = int(reserved_start_position) + idx
                    if entry.current_position != target_position:
                        moves.append((entry.lane_id, target_position))
            else:
                moves = plan_lane_reorder(snapshots)

            if pinned_channels and snapshots:
                move_map = dict(moves)
                final_positions = [
                    move_map.get(s.lane_id, s.current_position) for s in snapshots
                ]
                end_pos = max(final_positions) + 1
                for i, pinned_ch in enumerate(pinned_channels):
                    desired = end_pos + i
                    if int(pinned_ch.position) != desired:
                        moves.append((pinned_ch.id, desired))

            if not moves:
                return

            log.info(
                "TempVoice lane sorting: category=%s moves=%s",
                category_id,
                ", ".join(f"{lane_id}->{target}" for lane_id, target in moves),
            )
            for lane_id, target_position in sorted(moves, key=lambda item: item[1]):
                fresh_channel = guild.get_channel(int(lane_id))
                if not isinstance(fresh_channel, discord.VoiceChannel):
                    continue
                if fresh_channel.category_id != category_id:
                    continue
                if fresh_channel.position == target_position:
                    continue
                try:
                    await fresh_channel.edit(
                        position=target_position,
                        reason="TempVoice: Rank lane sorting",
                    )
                except discord.NotFound:
                    continue
                except discord.Forbidden as exc:
                    log.warning(
                        "TempVoice lane sorting forbidden for %s -> %s: %s",
                        fresh_channel.id,
                        target_position,
                        exc,
                    )
                    return
                except discord.HTTPException as exc:
                    log.warning(
                        "TempVoice lane sorting HTTP error for %s -> %s: %s",
                        fresh_channel.id,
                        target_position,
                        exc,
                    )

    async def _ensure_reserved_chill_slot(
        self,
        guild: discord.Guild,
        category: discord.CategoryChannel,
    ) -> int | None:
        anchor = guild.get_channel(int(CHILL_STAGING_CHANNEL_ID))
        fixed_lane = guild.get_channel(int(PERMANENT_CHILL_LANE_ID))
        if not isinstance(anchor, discord.VoiceChannel):
            return None
        if not isinstance(fixed_lane, discord.VoiceChannel):
            return None
        if anchor.category_id != category.id or fixed_lane.category_id != category.id:
            return None

        desired_fixed_position = int(anchor.position) + 1
        if int(fixed_lane.position) != desired_fixed_position:
            try:
                await fixed_lane.edit(
                    position=desired_fixed_position,
                    reason="TempVoice: permanent chill lane pinned below staging",
                )
            except discord.NotFound:
                return None
            except discord.Forbidden as exc:
                log.warning(
                    "TempVoice chill lane pin forbidden for %s -> %s: %s",
                    fixed_lane.id,
                    desired_fixed_position,
                    exc,
                )
                return None
            except discord.HTTPException as exc:
                log.warning(
                    "TempVoice chill lane pin HTTP error for %s -> %s: %s",
                    fixed_lane.id,
                    desired_fixed_position,
                    exc,
                )
                return None
        return int(anchor.position) + 2

    def _should_sort_lane(self, lane: discord.VoiceChannel | None) -> bool:
        if not isinstance(lane, discord.VoiceChannel):
            return False
        if lane.id in FIXED_LANE_IDS:
            return False
        if lane.category_id not in SUPPORTED_CATEGORY_IDS:
            return False
        # Neue Spieler Lanes und 1411391356278018245 vom Sorting ausschließen
        if lane.id == 1411391356278018245:
            return False
        return self.core.is_managed_lane(lane)

    async def _resolve_lane_rank(self, lane: discord.VoiceChannel) -> tuple[int, int]:
        if lane.category_id in MINRANK_CATEGORY_IDS:
            return await self._resolve_comp_rank(lane)
        return await self._resolve_chill_rank(lane)

    async def _resolve_comp_rank(self, lane: discord.VoiceChannel) -> tuple[int, int]:
        manager = self.bot.get_cog("RolePermissionVoiceManager")
        if manager and hasattr(manager, "get_channel_anchor"):
            try:
                anchor = manager.get_channel_anchor(lane)
            except Exception as exc:
                log.debug("comp anchor lookup failed for %s: %r", lane.id, exc)
                anchor = None
            if anchor:
                rank_value = int(anchor[2] or 0)
                subrank = int(anchor[5] or 0)
                if rank_value > 0:
                    return rank_value, max(0, subrank)
                rank_name = str(anchor[1] or "")
                parsed_index, parsed_subrank = parse_rank_label(rank_name)
                if parsed_index > 0:
                    return parsed_index, max(parsed_subrank, subrank)

        for label in (self.core.lane_base.get(lane.id), lane.name):
            rank_index, subrank = parse_rank_label(label)
            if rank_index > 0:
                return rank_index, subrank
        return 0, 0

    async def _resolve_chill_rank(self, lane: discord.VoiceChannel) -> tuple[int, int]:
        manager = self.bot.get_cog("RolePermissionVoiceManager")
        initial_owner_id = None
        if hasattr(self.core, "get_initial_owner_id"):
            try:
                initial_owner_id = self.core.get_initial_owner_id(lane)
            except Exception as exc:
                log.debug("initial owner lookup failed for %s: %r", lane.id, exc)
        owner_id = initial_owner_id or self.core.lane_owner.get(lane.id)
        owner = lane.guild.get_member(int(owner_id)) if owner_id else None

        if owner and manager and hasattr(manager, "get_user_rank_from_roles"):
            try:
                _rank_name, rank_value, subrank = manager.get_user_rank_from_roles(owner)
            except Exception as exc:
                log.debug("owner rank lookup failed for %s: %r", lane.id, exc)
            else:
                if rank_value and rank_value > 0:
                    if subrank is None and hasattr(manager, "get_user_subrank_from_db"):
                        try:
                            subrank = await manager.get_user_subrank_from_db(owner)
                        except Exception as exc:
                            log.debug("owner subrank lookup failed for %s: %r", lane.id, exc)
                            subrank = 0
                    return int(rank_value), max(0, int(subrank or 0))

        if owner:
            owner_rank = _member_rank_index(owner)
            if owner_rank > 0:
                return owner_rank, 0

        relevant_members: list[discord.Member] = []
        if manager and hasattr(manager, "get_rank_relevant_members"):
            try:
                relevant_members = await manager.get_rank_relevant_members(lane)
            except Exception as exc:
                log.debug("relevant member lookup failed for %s: %r", lane.id, exc)
        try:
            average_label = self.core._average_rank_prefix_for_members(relevant_members)
            if average_label is None:
                average_label = self.core._average_rank_prefix_for_lane(lane)
        except Exception as exc:
            log.debug("average rank lookup failed for %s: %r", lane.id, exc)
            average_label = None
        average_rank = parse_rank_label(average_label)
        if average_rank[0] > 0:
            return average_rank

        for label in (self.core.lane_base.get(lane.id), lane.name):
            rank_index, subrank = parse_rank_label(label)
            if rank_index > 0:
                return rank_index, subrank
        return 0, 0

    @commands.Cog.listener()
    async def on_tempvoice_lane_created(
        self, lane: discord.VoiceChannel, owner: discord.Member
    ) -> None:
        self.schedule_category_reorder(lane.guild.id, lane.category_id)

    @commands.Cog.listener()
    async def on_tempvoice_lane_owner_changed(
        self, lane: discord.VoiceChannel, owner_id: int
    ) -> None:
        self.schedule_category_reorder(lane.guild.id, lane.category_id)

    @commands.Cog.listener()
    async def on_tempvoice_lane_category_changed(
        self, lane: discord.VoiceChannel, category_id: int
    ) -> None:
        self.schedule_category_reorder(lane.guild.id, category_id)

    @commands.Cog.listener()
    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ) -> None:
        if member.bot:
            return
        before_channel = (
            before.channel if before and isinstance(before.channel, discord.VoiceChannel) else None
        )
        after_channel = (
            after.channel if after and isinstance(after.channel, discord.VoiceChannel) else None
        )
        before_id = getattr(before_channel, "id", None)
        after_id = getattr(after_channel, "id", None)
        if before_id == after_id:
            return
        if self._should_sort_lane(before_channel):
            self.schedule_category_reorder(before_channel.guild.id, before_channel.category_id)
        if self._should_sort_lane(after_channel):
            self.schedule_category_reorder(after_channel.guild.id, after_channel.category_id)

    @commands.Cog.listener()
    async def on_guild_channel_update(
        self,
        before: discord.abc.GuildChannel,
        after: discord.abc.GuildChannel,
    ) -> None:
        if not isinstance(before, discord.VoiceChannel) or not isinstance(
            after, discord.VoiceChannel
        ):
            return

        if before.category_id != after.category_id:
            if self._should_sort_lane(before):
                self.schedule_category_reorder(before.guild.id, before.category_id)
            if self._should_sort_lane(after):
                self.schedule_category_reorder(after.guild.id, after.category_id)
            return

        if before.name == after.name:
            return
        if self._should_sort_lane(after):
            self.schedule_category_reorder(after.guild.id, after.category_id)
