from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import os
import re
import time
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Any

import discord
from discord.ext import commands

from service import db
from service.config import settings
from service.deadlock_voice_cohort import (
    evaluate_deadlock_presence_row,
    select_best_deadlock_presence,
    select_deadlock_channel_cohort,
)
from service.guild_config import get_guild_config

log = logging.getLogger("DeadlockVoiceStatus")
trace_log = logging.getLogger("DeadlockVoiceStatus.trace")
_cfg = get_guild_config()

TARGET_CATEGORY_IDS: set[int] = {
    _cfg.VOICE_STATUS_CATEGORY_CHILL,
    _cfg.VOICE_STATUS_CATEGORY_COMP,
    _cfg.VOICE_STATUS_CATEGORY_STREET_BRAWL,
}
EXCLUDED_CHANNEL_IDS: set[int] = {
    _cfg.TEMPVOICE_PERMANENT_CASUAL_CHANNEL,
}

POLL_INTERVAL_SECONDS = 60
PRESENCE_STALE_SECONDS = 180
PARTY_MEMBER_STALE_SECONDS = 600
# Cooldown between voice rename attempts (seconds). Adjust here instead of via env.
RENAME_COOLDOWN_SECONDS = 360
RENAME_REASON = "Deadlock Voice Status Update"
MIN_ACTIVE_PLAYERS = 1
MATCH_MINUTE_DISPLAY_OFFSET = max(0, settings.match_minute_offset)

_SUFFIX_REGEX = re.compile(
    r"\s*-\s*(?:in der Lobby(?:\s*\(\d+/\d+\))?|im Match Min (?:\d+|\d+\+)\s*\(\d+/\d+\))$",
    re.IGNORECASE,
)


class DeadlockVoiceStatus(commands.Cog):
    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self.channel_states: dict[int, dict[str, object]] = {}
        self._task: asyncio.Task[None] | None = None
        self.last_observation: dict[int, dict[str, Any]] = {}
        # Cache: channel_id -> (slots, timestamp). Stores party size from lobby phase
        # so it can be reused during the match when localized strings no longer show (X/Y).
        self._localized_slots_cache: dict[int, tuple[int, float]] = {}

        trace_env = (os.getenv("DEADLOCK_VS_TRACE") or "1").strip().lower()
        self.trace_enabled = trace_env not in {"0", "false", "no", "off"}
        self.trace_channel_filter: set[int] = set()
        self.trace_file = Path(
            os.getenv("DEADLOCK_VS_TRACE_FILE", "logs/deadlock_voice_status.log")
        )
        channel_filter_raw = os.getenv("DEADLOCK_VS_TRACE_CHANNELS", "")
        for part in re.split(r"[\\s,;]+", channel_filter_raw):
            part = part.strip()
            if part.isdigit():
                self.trace_channel_filter.add(int(part))
        self._trace_handler: logging.Handler | None = None
        self._trace_logger = trace_log
        if self.trace_enabled:
            self._enable_trace_logger()

    async def cog_load(self) -> None:
        self._task = asyncio.create_task(self._run_loop())
        log.info("DeadlockVoiceStatus background task started")

    async def cog_unload(self) -> None:
        if self._task:
            self._task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._task
            self._task = None
        log.info("DeadlockVoiceStatus shut down")

    def _enable_trace_logger(self) -> None:
        if self._trace_handler:
            return
        try:
            self.trace_file.parent.mkdir(parents=True, exist_ok=True)
        except Exception as exc:
            log.debug("Could not create trace log directory: %s", exc, exc_info=True)
        handler = logging.handlers.RotatingFileHandler(
            self.trace_file, maxBytes=2 * 1024 * 1024, backupCount=2, encoding="utf-8"
        )
        handler.setLevel(logging.DEBUG)
        handler.setFormatter(logging.Formatter("%(asctime)s [TRACE] %(message)s"))
        self._trace_logger.addHandler(handler)
        self._trace_logger.setLevel(logging.DEBUG)
        self._trace_logger.propagate = False  # nur Datei, kein Root/Console
        self._trace_handler = handler
        log.info("DeadlockVoiceStatus trace logging enabled at %s", self.trace_file)

    def _disable_trace_logger(self) -> None:
        if not self._trace_handler:
            self.trace_enabled = False
            return
        try:
            self._trace_logger.removeHandler(self._trace_handler)
            self._trace_handler.close()
        except Exception as exc:
            log.debug("Failed to close trace handler: %s", exc, exc_info=True)
        finally:
            self._trace_handler = None
        self.trace_enabled = False
        log.info("DeadlockVoiceStatus trace logging disabled")

    @staticmethod
    def _json_fallback(obj: Any) -> Any:
        if isinstance(obj, set):
            return list(obj)
        if isinstance(obj, Path):
            return str(obj)
        return str(obj)

    def _store_trace(self, channel_id: int, payload: dict[str, Any]) -> None:
        if not payload:
            return
        try:
            self.last_observation[channel_id] = payload
        except Exception as exc:
            log.debug(
                "Could not store last observation for %s: %s",
                channel_id,
                exc,
                exc_info=True,
            )
        if not self.trace_enabled:
            return
        if self.trace_channel_filter and channel_id not in self.trace_channel_filter:
            return
        try:
            self._trace_logger.debug(
                json.dumps(payload, ensure_ascii=True, default=self._json_fallback)
            )
        except Exception:
            self._trace_logger.debug("trace %r", payload)

    async def _run_loop(self) -> None:
        await self.bot.wait_until_ready()
        while not self.bot.is_closed():
            try:
                await self._update_all_channels()
            except Exception as exc:  # noqa: BLE001
                log.exception("DeadlockVoiceStatus update failed: %s", exc)
            await asyncio.sleep(POLL_INTERVAL_SECONDS)

    async def _update_all_channels(self) -> None:
        await self._clear_excluded_channel_statuses()
        channels = self._collect_monitored_channels()
        if not channels:
            return

        members_per_channel: dict[int, list[discord.Member]] = {}
        user_ids: set[int] = set()
        for channel in channels:
            members = [m for m in channel.members if not m.bot]
            members_per_channel[channel.id] = members
            user_ids.update(member.id for member in members)

        steam_map = await self._fetch_user_steam_ids(user_ids)
        steam_ids = {sid for sids in steam_map.values() for sid in sids}
        presence_map = await self._fetch_presence_rows(steam_ids)
        now = int(time.time())

        voice_watch_entries: dict[str, tuple[str, int, int]] = {}

        for channel in channels:
            members = members_per_channel.get(channel.id, [])
            if members:
                for member in members:
                    steam_ids = steam_map.get(member.id, [])
                    for steam_id in steam_ids:
                        voice_watch_entries[steam_id] = (
                            str(steam_id),
                            channel.guild.id,
                            channel.id,
                        )
            await self._process_channel(channel, members, steam_map, presence_map, now)

        await self._persist_voice_watch_entries(list(voice_watch_entries.values()))

    def _collect_monitored_channels(self) -> list[discord.VoiceChannel]:
        result: list[discord.VoiceChannel] = []
        for guild in self.bot.guilds:
            for channel in guild.voice_channels:
                if channel.id in EXCLUDED_CHANNEL_IDS:
                    continue
                if channel.category_id in TARGET_CATEGORY_IDS:
                    result.append(channel)
        return result

    async def _clear_excluded_channel_statuses(self) -> None:
        for guild in self.bot.guilds:
            for channel_id in EXCLUDED_CHANNEL_IDS:
                channel = guild.get_channel(int(channel_id))
                if not isinstance(channel, discord.VoiceChannel):
                    continue
                base_name, current_suffix = self._split_suffix(channel.name)
                base_name = self._resolve_base_name(channel, base_name)
                await self._apply_channel_name(
                    channel,
                    base_name,
                    None,
                    None,
                    None,
                    current_suffix,
                    None,
                    None,
                    None,
                    debug_payload={
                        "channel_id": channel.id,
                        "channel_name": channel.name,
                        "base_name": base_name,
                        "current_suffix": current_suffix,
                        "decision": {"reason": "excluded_channel"},
                    },
                )

    def _resolve_base_name(self, channel: discord.VoiceChannel, fallback_base: str) -> str:
        """Ermittelt den Basisnamen; für dynamische Chill-Lanes aus TempVoice-Regeln statt aus altem Channelnamen."""
        base = (fallback_base or "").strip()
        core = self.bot.get_cog("TempVoiceCore")
        if core is None:
            return base
        try:
            created_channels = getattr(core, "created_channels", set())
            if channel.id not in created_channels:
                return base

            lane_rules = getattr(core, "lane_rules", {}).get(channel.id)
            if not lane_rules and hasattr(core, "_rules_for_category"):
                lane_rules = core._rules_for_category(channel.category)
            if not isinstance(lane_rules, dict) or not lane_rules.get("prefix_from_rank"):
                return base

            compose_name = getattr(core, "_compose_name", None)
            if not callable(compose_name):
                return base
            desired_name = str(compose_name(channel)).strip()
            if not desired_name:
                return base
            resolved_base, _ignored_suffix = self._split_suffix(desired_name)
            return resolved_base or base
        except Exception as exc:
            log.debug(
                "Failed to resolve dynamic base name for channel %s: %s",
                channel.id,
                exc,
                exc_info=True,
            )
            return base

    async def _fetch_user_steam_ids(self, user_ids: Iterable[int]) -> dict[int, list[str]]:
        ids = {int(uid) for uid in user_ids if uid}
        if not ids:
            return {}

        ids_json = json.dumps(sorted(ids))
        query = """
            SELECT user_id, steam_id, primary_account, verified, updated_at
            FROM steam_links
            WHERE user_id IN (SELECT value FROM json_each(?))
              AND steam_id IS NOT NULL AND steam_id != ''
            ORDER BY primary_account DESC, verified DESC, updated_at DESC
        """
        rows = await db.query_all_async(query, (ids_json,))

        mapping: dict[int, list[str]] = {}
        for row in rows:
            uid = int(row["user_id"])
            sid = str(row["steam_id"])
            bucket = mapping.setdefault(uid, [])
            if sid not in bucket:
                bucket.append(sid)
        return mapping

    async def _fetch_presence_rows(self, steam_ids: Iterable[str]) -> dict[str, Any]:
        ids = {sid for sid in steam_ids if sid}
        if not ids:
            return {}

        ids_json = json.dumps(sorted(ids))
        query = """
            SELECT steam_id, deadlock_stage, deadlock_minutes, deadlock_localized,
                   deadlock_updated_at, last_seen_ts, in_deadlock_now, in_match_now_strict,
                   last_server_id, deadlock_party_hint
            FROM live_player_state
            WHERE steam_id IN (SELECT value FROM json_each(?))
        """
        rows = await db.query_all_async(query, (ids_json,))

        return {str(row["steam_id"]): row for row in rows}

    async def _fetch_party_rows_for_steam_ids(
        self,
        steam_ids: Iterable[str],
        now: int,
    ) -> list[Any]:
        ids = sorted({str(sid) for sid in steam_ids if sid})
        if not ids:
            return []

        ids_json = json.dumps(ids)
        cutoff = now - PARTY_MEMBER_STALE_SECONDS
        query = """
            SELECT party_id, steam_id, party_size, seen_at
            FROM deadlock_party_members
            WHERE steam_id IN (SELECT value FROM json_each(?))
              AND seen_at >= ?
        """
        return await db.query_all_async(query, (ids_json, cutoff))

    async def _fetch_party_rows_for_party_ids(
        self,
        party_ids: Iterable[str],
        now: int,
    ) -> list[Any]:
        ids = sorted({str(party_id) for party_id in party_ids if party_id})
        if not ids:
            return []

        ids_json = json.dumps(ids)
        cutoff = now - PARTY_MEMBER_STALE_SECONDS
        query = """
            SELECT party_id, steam_id, party_size, seen_at
            FROM deadlock_party_members
            WHERE party_id IN (SELECT value FROM json_each(?))
              AND seen_at >= ?
        """
        return await db.query_all_async(query, (ids_json, cutoff))

    @staticmethod
    def _normalize_party_size(value: Any) -> int | None:
        if value is None:
            return None
        try:
            parsed = int(value)
        except (TypeError, ValueError):
            return None
        if parsed < 1:
            return None
        return min(6, parsed)

    def _select_best_party_candidate(
        self,
        cohort_steam_ids: set[str],
        party_rows: Sequence[Any],
    ) -> dict[str, Any] | None:
        grouped: dict[str, dict[str, Any]] = {}
        for row in party_rows:
            party_id_raw = self._safe_row_value(row, "party_id")
            steam_id_raw = self._safe_row_value(row, "steam_id")
            if not party_id_raw or not steam_id_raw:
                continue
            party_id = str(party_id_raw)
            steam_id = str(steam_id_raw)
            bucket = grouped.setdefault(
                party_id,
                {
                    "party_id": party_id,
                    "cohort_steam_ids": set(),
                    "reported_sizes": [],
                    "latest_seen_at": 0,
                },
            )
            if steam_id in cohort_steam_ids:
                bucket["cohort_steam_ids"].add(steam_id)
            normalized_size = self._normalize_party_size(self._safe_row_value(row, "party_size"))
            if normalized_size is not None:
                bucket["reported_sizes"].append(normalized_size)
            seen_at_raw = self._safe_row_value(row, "seen_at")
            try:
                bucket["latest_seen_at"] = max(int(seen_at_raw or 0), int(bucket["latest_seen_at"]))
            except (TypeError, ValueError):
                pass  # Skip invalid seen_at values when computing the latest timestamp.

        best: dict[str, Any] | None = None
        best_score: tuple[int, int, int, int] | None = None
        for candidate in grouped.values():
            overlap_count = len(candidate["cohort_steam_ids"])
            if overlap_count <= 0:
                continue
            reported_size = max(candidate["reported_sizes"]) if candidate["reported_sizes"] else 0
            latest_seen_at = int(candidate["latest_seen_at"])
            score = (overlap_count, reported_size, latest_seen_at, len(candidate["party_id"]))
            if best_score is None or score > best_score:
                best = candidate
                best_score = score
        return best

    async def _resolve_effective_player_count(
        self,
        *,
        members: Sequence[discord.Member],
        steam_map: dict[int, list[str]],
        candidate_member_ids: Sequence[int],
        chosen_steam_ids: Sequence[str],
        raw_player_count: int,
        now: int,
    ) -> tuple[int, dict[str, Any]]:
        trace_details: dict[str, Any] = {
            "raw_player_count": raw_player_count,
            "candidate_member_ids": [int(member_id) for member_id in candidate_member_ids],
            "chosen_steam_ids": [str(sid) for sid in chosen_steam_ids if sid],
        }
        if raw_player_count <= 0 or not chosen_steam_ids:
            trace_details["mode"] = "raw_only"
            trace_details["reason"] = "no_candidate_steam_ids"
            return raw_player_count, trace_details

        initial_rows = await self._fetch_party_rows_for_steam_ids(chosen_steam_ids, now)
        if not initial_rows:
            trace_details["mode"] = "raw_only"
            trace_details["reason"] = "no_recent_party_rows"
            return raw_player_count, trace_details

        party_candidate = self._select_best_party_candidate(set(chosen_steam_ids), initial_rows)
        if not party_candidate:
            trace_details["mode"] = "raw_only"
            trace_details["reason"] = "no_party_candidate"
            return raw_player_count, trace_details

        full_party_rows = await self._fetch_party_rows_for_party_ids(
            [str(party_candidate["party_id"])],
            now,
        )
        visible_party_steam_ids = {
            str(self._safe_row_value(row, "steam_id"))
            for row in full_party_rows
            if self._safe_row_value(row, "steam_id")
        }
        visible_party_count = len(visible_party_steam_ids)
        reported_party_sizes = [
            size
            for size in (
                self._normalize_party_size(self._safe_row_value(row, "party_size"))
                for row in full_party_rows
            )
            if size is not None
        ]
        target_party_size = max(reported_party_sizes, default=visible_party_count)
        target_party_size = max(target_party_size, raw_player_count)
        target_party_size = min(6, target_party_size)

        unlinked_member_ids = [int(member.id) for member in members if not steam_map.get(member.id)]
        inferred_missing = max(0, target_party_size - raw_player_count)
        inferred_unlinked = min(
            inferred_missing,
            len(unlinked_member_ids),
            max(0, len(members) - raw_player_count),
        )
        effective_player_count = min(6, raw_player_count + inferred_unlinked)

        trace_details.update(
            {
                "mode": (
                    "party_verified"
                    if visible_party_count >= target_party_size
                    else "party_inferred"
                    if inferred_unlinked > 0
                    else "party_partial"
                ),
                "party_id": str(party_candidate["party_id"]),
                "party_overlap_count": len(party_candidate["cohort_steam_ids"]),
                "visible_party_count": visible_party_count,
                "reported_party_size": max(reported_party_sizes) if reported_party_sizes else None,
                "target_party_size": target_party_size,
                "unlinked_member_ids": unlinked_member_ids,
                "inferred_unlinked": inferred_unlinked,
                "effective_player_count": effective_player_count,
            }
        )
        return effective_player_count, trace_details

    async def _persist_voice_watch_entries(self, entries: list[tuple[str, int, int]]) -> None:
        now_ts = int(time.time())
        if not entries:
            await db.execute_async("DELETE FROM deadlock_voice_watch")
            return

        try:
            rows = [
                (steam_id, guild_id, channel_id, now_ts)
                for (steam_id, guild_id, channel_id) in entries
            ]
            await db.executemany_async(
                """
                INSERT INTO deadlock_voice_watch(steam_id, guild_id, channel_id, updated_at)
                VALUES(?, ?, ?, ?)
                ON CONFLICT(steam_id) DO UPDATE SET
                  guild_id=excluded.guild_id,
                  channel_id=excluded.channel_id,
                  updated_at=excluded.updated_at
                """,
                rows,
            )
            # Flat list for DELETE IN clause
            delete_ids = [steam_id for (steam_id, _, _) in entries]
            delete_json = json.dumps(delete_ids)
            await db.execute_async(
                """
                DELETE FROM deadlock_voice_watch
                WHERE steam_id NOT IN (SELECT value FROM json_each(?))
                """,
                (delete_json,),
            )
        except Exception as exc:
            log.warning("Failed to persist voice watch entries: %s", exc)

    async def _process_channel(
        self,
        channel: discord.VoiceChannel,
        members: Sequence[discord.Member],
        steam_map: dict[int, list[str]],
        presence_map: dict[str, Any],
        now: int,
    ) -> None:
        base_name, current_suffix = self._split_suffix(channel.name)
        base_name = self._resolve_base_name(channel, base_name)
        total_members = len(members)
        trace_payload: dict[str, Any] = {
            "channel_id": channel.id,
            "channel_name": channel.name,
            "base_name": base_name,
            "current_suffix": current_suffix,
            "total_members": total_members,
            "presence": [],
            "decision": {},
        }
        if total_members == 0:
            self._localized_slots_cache.pop(channel.id, None)
            trace_payload["decision"] = {"reason": "empty_channel"}
            await self._apply_channel_name(
                channel,
                base_name,
                None,
                None,
                None,
                current_suffix,
                None,
                None,
                None,
                debug_payload=trace_payload,
            )
            return
        presence_entries: list[tuple[str, int, str | None]] = []
        chosen_steam_by_member_id: dict[int, str] = {}
        for member in members:
            steam_ids = steam_map.get(member.id, [])
            best_presence = self._select_best_presence(steam_ids, presence_map, now)
            if best_presence:
                stage, minutes, server_id, chosen_sid = best_presence
            else:
                stage = minutes = server_id = chosen_sid = None
            row = presence_map.get(str(chosen_sid)) if chosen_sid else None
            trace_entry: dict[str, Any] = {
                "member_id": member.id,
                "member": member.display_name,
                "steam_ids": steam_ids,
                "chosen_steam_id": chosen_sid,
                "stage": stage,
                "minutes": minutes,
                "server_id": server_id,
                "raw_stage": self._safe_row_value(row, "deadlock_stage"),
                "raw_minutes": self._safe_row_value(row, "deadlock_minutes"),
                "raw_updated_at": self._safe_row_value(row, "deadlock_updated_at")
                or self._safe_row_value(row, "last_seen_ts"),
                "raw_localized": self._safe_row_value(row, "deadlock_localized"),
                "raw_party_hint": self._safe_row_value(row, "deadlock_party_hint"),
                "raw_last_server_id": self._safe_row_value(row, "last_server_id"),
            }
            trace_payload["presence"].append(trace_entry)
            if not best_presence:
                continue
            if stage not in {"lobby", "match"}:
                continue
            chosen_steam_by_member_id[int(member.id)] = str(chosen_sid)
            presence_entries.append(
                {
                    "member_id": member.id,
                    "stage": stage,
                    "minutes": minutes or 0,
                    "server_id": server_id,
                }
            )

        if not presence_entries:
            trace_payload["decision"] = {"reason": "no_presence_entries"}
            await self._apply_channel_name(
                channel,
                base_name,
                None,
                None,
                None,
                current_suffix,
                None,
                None,
                None,
                debug_payload=trace_payload,
            )
            return
        candidate = select_deadlock_channel_cohort(
            presence_entries,
            min_active_players=MIN_ACTIVE_PLAYERS,
        )
        candidate_stage = str(candidate["stage"]) if candidate else None
        candidate_minutes = list(candidate["minute_values"]) if candidate else []
        candidate_count = int(candidate["member_count"]) if candidate else 0
        chosen_server_id = (
            str(candidate["server_id"]) if candidate and candidate["server_id"] else None
        )

        if not candidate_stage or candidate_count < MIN_ACTIVE_PLAYERS:
            trace_payload["decision"] = {
                "reason": "no_candidate",
                "candidate_stage": candidate_stage,
                "candidate_count": candidate_count,
            }
            await self._apply_channel_name(
                channel,
                base_name,
                None,
                None,
                None,
                current_suffix,
                None,
                None,
                None,
                debug_payload=trace_payload,
            )
            return

        player_count_raw = len(candidate_minutes)
        candidate_member_ids = [int(member_id) for member_id in candidate["member_ids"]]
        candidate_steam_ids = [
            chosen_steam_by_member_id[member_id]
            for member_id in candidate_member_ids
            if member_id in chosen_steam_by_member_id
        ]
        player_count, party_trace = await self._resolve_effective_player_count(
            members=members,
            steam_map=steam_map,
            candidate_member_ids=candidate_member_ids,
            chosen_steam_ids=candidate_steam_ids,
            raw_player_count=min(player_count_raw, 6),
            now=now,
        )
        localized_slots = self._parse_voice_slots_from_localized(candidate_steam_ids, presence_map)
        _SLOTS_CACHE_TTL = 3600  # 1h
        if localized_slots:
            self._localized_slots_cache[channel.id] = (localized_slots, time.time())
        else:
            cached = self._localized_slots_cache.get(channel.id)
            if cached and (time.time() - cached[1]) < _SLOTS_CACHE_TTL:
                localized_slots = cached[0]
        voice_slots = max(player_count, localized_slots) if localized_slots else total_members
        trace_payload["decision"] = {
            "reason": "candidate_selected",
            "candidate_stage": candidate_stage,
            "candidate_count": candidate_count,
            "candidate_minutes": candidate_minutes,
            "chosen_server_id": chosen_server_id,
            "player_count_raw": player_count_raw,
            "voice_slots": voice_slots,
            "localized_slots": localized_slots,
            "member_count": total_members,
            "cohort_member_ids": list(candidate["member_ids"]) if candidate else [],
            "party_resolution": party_trace,
        }

        if candidate_stage == "lobby":
            if localized_slots and voice_slots > player_count:
                suffix = f"in der Lobby ({player_count}/{voice_slots})"
            else:
                suffix = "in der Lobby"
            trace_payload["decision"].update(
                {
                    "suffix": suffix,
                    "player_count": player_count,
                    "voice_slots_effective": voice_slots,
                }
            )
            await self._apply_channel_name(
                channel,
                base_name,
                suffix,
                candidate_stage,
                None,
                current_suffix,
                player_count,
                voice_slots,
                None,
                chosen_server_id,
                trace_payload,
            )
            return

        if candidate_stage == "match":
            max_minutes = max(candidate_minutes) if candidate_minutes else 0
            display_minutes = max(0, max_minutes + MATCH_MINUTE_DISPLAY_OFFSET)
            suffix = f"im Match Min {display_minutes} ({player_count}/{voice_slots})"
            trace_payload["decision"].update(
                {
                    "suffix": suffix,
                    "player_count": player_count,
                    "voice_slots_effective": voice_slots,
                    "bucket": str(display_minutes),
                    "max_minutes": max_minutes,
                    "display_minutes": display_minutes,
                }
            )
            await self._apply_channel_name(
                channel,
                base_name,
                suffix,
                candidate_stage,
                str(display_minutes),
                current_suffix,
                player_count,
                voice_slots,
                max_minutes,
                chosen_server_id,
                trace_payload,
            )
            return

        trace_payload["decision"]["reason"] = "fallback"
        await self._apply_channel_name(
            channel,
            base_name,
            None,
            None,
            None,
            current_suffix,
            None,
            None,
            None,
            debug_payload=trace_payload,
        )

    @staticmethod
    def _safe_row_value(row: Any | None, key: str) -> Any | None:
        if row is None:
            return None
        try:
            return row[key]
        except Exception:
            return None

    def _parse_voice_slots_from_localized(
        self,
        steam_ids: Sequence[str],
        presence_map: dict[str, Any],
    ) -> int | None:
        """Parse (X/Y) from deadlock_localized strings; return max Y found."""
        max_total: int | None = None
        for steam_id in steam_ids:
            row = presence_map.get(str(steam_id))
            localized = self._safe_row_value(row, "deadlock_localized")
            if not localized:
                continue
            m = re.search(r"\((\d+)/(\d+)\)", str(localized))
            if m:
                total = int(m.group(2))
                if max_total is None or total > max_total:
                    max_total = total
        return max_total

    def _select_best_presence(
        self,
        steam_ids: Sequence[str],
        presence_map: dict[str, Any],
        now: int,
    ) -> tuple[str, int | None, str | None, str] | None:
        return select_best_deadlock_presence(
            steam_ids,
            presence_map,
            now,
            stale_seconds=PRESENCE_STALE_SECONDS,
        )

    def _evaluate_presence(
        self,
        steam_id: str | None,
        presence_map: dict[str, Any],
        now: int,
    ) -> tuple[str, int | None, str | None] | None:
        if not steam_id:
            return None
        return evaluate_deadlock_presence_row(
            presence_map.get(str(steam_id)),
            now,
            stale_seconds=PRESENCE_STALE_SECONDS,
        )

    async def _apply_channel_name(
        self,
        channel: discord.VoiceChannel,
        base_name: str,
        desired_suffix: str | None,
        stage_label: str | None,
        bucket_label: str | None,
        current_suffix: str | None,
        player_count: int | None,
        voice_slots: int | None,
        minutes_value: int | None = None,
        server_identifier: str | None = None,
        debug_payload: dict[str, Any] | None = None,
    ) -> None:
        base_clean = base_name.rstrip()
        target_name = base_clean if not desired_suffix else f"{base_clean} - {desired_suffix}"
        trace_data = debug_payload or {}
        rename_info: dict[str, Any] = {
            "base": base_clean,
            "target_name": target_name,
            "current_name": channel.name,
            "desired_suffix": desired_suffix,
            "current_suffix": current_suffix,
            "stage": stage_label,
            "bucket": bucket_label,
            "player_count": player_count,
            "voice_slots": voice_slots,
            "minutes_value": minutes_value,
            "server_id": server_identifier,
            "attempted": False,
        }

        if channel.name == target_name:
            state = self.channel_states.setdefault(channel.id, {})
            state.update(
                {
                    "base": base_clean,
                    "stage": stage_label,
                    "bucket": bucket_label,
                    "suffix": desired_suffix,
                    "players": player_count,
                    "voice_slots": voice_slots,
                    "server_id": server_identifier,
                }
            )
            rename_info["result"] = "noop_target_matches"
            trace_data["rename"] = rename_info
            self._store_trace(channel.id, trace_data)
            return

        state = self.channel_states.setdefault(channel.id, {})
        previous_stage = state.get("stage")
        match_exit_override = previous_stage == "match" and stage_label == "lobby"
        clear_status_override = previous_stage in {"match", "lobby"} and stage_label is None
        last_rename = state.get("last_rename", 0.0)
        elapsed = time.time() - float(last_rename)

        effective_cooldown = RENAME_COOLDOWN_SECONDS
        if stage_label == "match" and minutes_value is not None:
            if minutes_value >= 25:
                effective_cooldown = max(RENAME_COOLDOWN_SECONDS, 600)

        # Retrieve previous state to implement "no rename on user join"
        previous_state = self.channel_states.get(channel.id, {})
        previous_stage = previous_state.get("stage")
        previous_member_count = previous_state.get("previous_member_count", 0)

        # Logic for "Rename soll nicht triggern wenn eine person dazu komtm in den Channel"
        # Only rename if:
        # 1. The game stage (lobby/match) has changed.
        # 2. The suffix (reflecting game time/player counts based on game status) has changed.
        # 3. If stage_label is None (no game active) AND member count changed: do NOT rename.
        #    This prevents renaming just because people join/leave an empty/non-game channel.
        #    (unless new_name is significantly different from base_name, i.e., new suffix is determined)

        # Determine if a meaningful game state change occurred
        meaningful_game_state_change = (
            stage_label != previous_stage
            or (
                desired_suffix != current_suffix
            )  # Suffix includes player counts based on game status
        )

        # Decide if we should rename
        should_rename_based_on_conditions = False
        if meaningful_game_state_change:
            should_rename_based_on_conditions = True
        elif stage_label is None and player_count != previous_member_count:
            # If no game is detected (stage_label is None), and only member count changed, DO NOT rename.
            # This is the core of "rename soll nicht triggern wenn eine person dazu komtm in den Channel".
            # It prevents renaming a "Waiting" channel to "Waiting (4)" just because someone joined.
            logging.debug(
                f"DeadlockVoiceStatus: Channel {channel.id} member count changed, but no game stage detected. Suppressing rename."
            )
            should_rename_based_on_conditions = False
        else:
            # All other cases, if the suffix or base name demands it (e.g. template changes from TempVoice)
            should_rename_based_on_conditions = (
                desired_suffix != current_suffix or base_clean != channel.name.rstrip()
            )

        should_rename = should_rename_based_on_conditions  # Final decision to rename
        rename_info.update(
            {
                "should_rename": should_rename,
                "elapsed_since_last": elapsed,
                "effective_cooldown": effective_cooldown,
                "cooldown_remaining": max(0.0, effective_cooldown - elapsed),
                "match_exit_override": match_exit_override,
                "clear_status_override": clear_status_override,
                "meaningful_game_state_change": meaningful_game_state_change,
                "previous_stage": previous_stage,
                "previous_member_count": previous_member_count,
            }
        )

        if not should_rename:
            state.update(
                {
                    "base": base_clean,
                    "stage": stage_label,
                    "bucket": bucket_label,
                    "suffix": desired_suffix,
                    "players": player_count,
                    "voice_slots": voice_slots,
                    "server_id": server_identifier,
                    "previous_member_count": player_count,  # Update member count even if not renaming
                }
            )
            rename_info["result"] = "noop_no_meaningful_change"
            trace_data["rename"] = rename_info
            self._store_trace(channel.id, trace_data)
            return

        if elapsed < effective_cooldown and not match_exit_override and not clear_status_override:
            rename_info["result"] = "cooldown"
            trace_data["rename"] = rename_info
            self._store_trace(channel.id, trace_data)
            return
        if (match_exit_override or clear_status_override) and elapsed < effective_cooldown:
            rename_info["cooldown_bypassed"] = True

        try:
            await self.bot.queue_channel_rename(channel.id, target_name, reason=RENAME_REASON)
        except Exception as exc:
            log.warning("Failed to queue rename for voice channel %s: %s", channel.id, exc)
            state["last_rename"] = (
                time.time()
            )  # Ensure cooldown still applies for direct attempts or next queue
            rename_info.update({"result": "error", "error": str(exc)})
            trace_data["rename"] = rename_info
            self._store_trace(channel.id, trace_data)
            return

        rename_info["result"] = "queued"
        rename_info["attempted"] = True
        trace_data["rename"] = rename_info
        # Update state immediately for current logic, rename queue will eventually apply it
        self.channel_states[channel.id] = {
            "base": base_clean,
            "stage": stage_label,
            "bucket": bucket_label,
            "suffix": desired_suffix,
            "players": player_count,
            "voice_slots": voice_slots,
            "server_id": server_identifier,
            "last_rename": time.time(),  # Cooldown starts from when it's queued
            "previous_member_count": player_count,  # Update member count after queueing
        }
        self._store_trace(channel.id, trace_data)

    @commands.group(name="dlvs", invoke_without_command=True)
    @commands.has_permissions(manage_guild=True)
    async def dlvs_group(self, ctx: commands.Context) -> None:
        status = "aktiv" if self.trace_enabled else "aus"
        filter_info = (
            "alle Kanaele"
            if not self.trace_channel_filter
            else ", ".join(str(cid) for cid in sorted(self.trace_channel_filter))
        )
        await ctx.send(
            f"DeadlockVoiceStatus Trace: {status} | Filter: {filter_info} | Logfile: {self.trace_file}"
        )

    @dlvs_group.command(name="trace")
    @commands.has_permissions(manage_guild=True)
    async def dlvs_trace(
        self,
        ctx: commands.Context,
        mode: str | None = None,
        channel: discord.VoiceChannel | None = None,
    ) -> None:
        if not mode:
            await self.dlvs_group(ctx)
            return

        mode_l = mode.lower()
        if mode_l in {"on", "an", "start", "enable"}:
            self.trace_enabled = True
            target_channel = channel or (ctx.author.voice.channel if ctx.author.voice else None)
            self.trace_channel_filter = {target_channel.id} if target_channel else set()
            self._enable_trace_logger()
            target_label = (
                f"Channel {target_channel.name} ({target_channel.id})" if target_channel else "alle"
            )
            await ctx.send(f"Trace an ({target_label}), schreibt nach {self.trace_file}")
            return

        if mode_l in {"off", "aus", "stop", "disable"}:
            self.trace_channel_filter.clear()
            self._disable_trace_logger()
            await ctx.send("Trace aus.")
            return

        await ctx.send(
            "Nutze `on/an` oder `off/aus`. Optional kannst du einen Voice-Channel angeben."
        )

    @dlvs_group.command(name="snapshot")
    @commands.has_permissions(manage_guild=True)
    async def dlvs_snapshot(
        self,
        ctx: commands.Context,
        channel: discord.VoiceChannel | None = None,
    ) -> None:
        target_channel = channel or (ctx.author.voice.channel if ctx.author.voice else None)
        if not target_channel:
            await ctx.send("Bitte gib einen Voice-Channel an oder sei in einem VC.")
            return

        snapshot = self.last_observation.get(target_channel.id)
        if not snapshot:
            await ctx.send("Keine Beobachtung fuer diesen Kanal vorhanden.")
            return

        decision = snapshot.get("decision", {}) or {}
        rename = snapshot.get("rename", {}) or {}
        party_resolution = decision.get("party_resolution", {}) or {}
        lines = [
            f"{target_channel.name} ({target_channel.id})",
            f"Entscheidung: {decision.get('candidate_stage')} | Server: {decision.get('chosen_server_id')} | Suffix: {decision.get('suffix')}",
            f"Bucket/Min: {decision.get('bucket') or decision.get('max_minutes')} | Spieler: {decision.get('player_count')} / {decision.get('voice_slots_effective')}",
            f"Party: {party_resolution.get('mode')} | party_id={party_resolution.get('party_id')} | raw={party_resolution.get('raw_player_count')} | effective={party_resolution.get('effective_player_count')} | inferred={party_resolution.get('inferred_unlinked')}",
            f"Rename: {rename.get('result')} | should={rename.get('should_rename')} | cooldown={rename.get('cooldown_remaining')}s",
        ]

        presence_lines = []
        for entry in (snapshot.get("presence") or [])[:10]:
            presence_lines.append(
                f"- {entry.get('member')} ({entry.get('chosen_steam_id') or '-'}) -> {entry.get('stage')} "
                f"{entry.get('minutes')}m srv={entry.get('server_id')} raw_stage={entry.get('raw_stage')}"
            )
        if not presence_lines:
            presence_lines.append("- Keine Presence-Daten.")
        if len(snapshot.get("presence") or []) > len(presence_lines):
            presence_lines.append("... gekuerzt ...")

        await ctx.send("\n".join(lines + presence_lines))

    @staticmethod
    def _split_suffix(name: str) -> tuple[str, str | None]:
        match = _SUFFIX_REGEX.search(name)
        if not match:
            return name.strip(), None
        base = name[: match.start()].rstrip()
        suffix = name[match.start() :].strip()
        return base if base else name.strip(), suffix if suffix else None

    @staticmethod
    def _bucket_minutes(minutes: int) -> str:
        if minutes >= 50:
            return "50+"
        bucket = (minutes // 5) * 5
        return str(bucket)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(DeadlockVoiceStatus(bot))
    log.info("DeadlockVoiceStatus cog added")
