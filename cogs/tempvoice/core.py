# cogs/tempvoice/core.py
# TempVoiceCore – Auto-Lanes, Owner-Logik, Persistenz (zentrale DB)
from __future__ import annotations

import asyncio
import logging
import re
import time
from dataclasses import dataclass
from datetime import datetime
from typing import Any

import discord
from discord.ext import commands

from service import db
from service.guild_config import get_guild_config

log = logging.getLogger("TempVoiceCore")


def _safe_log_value(value: Any) -> str:
    """Sanitize values before logging to prevent log injection attacks."""
    text = "" if value is None else str(value)
    return text.replace("\r", "\\r").replace("\n", "\\n")


_cfg = get_guild_config()

# --------- IDs / Konfiguration ---------
# Werte kommen nun zentral aus service.guild_config, damit ID-Änderungen nur dort gepflegt werden müssen.
CASUAL_STAGING_ID = _cfg.TEMPVOICE_STAGING_CASUAL
STAGING_CHANNEL_IDS: set[int] = set(_cfg.tempvoice_staging_channels)
FIXED_LANE_IDS: set[int] = {
    _cfg.TEMPVOICE_PERMANENT_CASUAL_CHANNEL,  # Permanenter Chill-Voice direkt unter dem Staging
    1411391356278018245,  # Dauerhafter Voice-Channel (nicht von TempVoice verwalten)
    1470126503252721845,  # Ausgenommen: nie von TempVoice verwalten/loeschen
}
MINRANK_CATEGORY_IDS: set[int] = set(_cfg.tempvoice_minrank_categories)
# Per-Staging-Speziallogik (nur Lanes die vom Standard abweichen)
STAGING_RULES: dict[int, dict[str, Any]] = {
    _cfg.TEMPVOICE_STAGING_STREET_BRAWL: {  # Street Brawl
        "prefix": "Street Brawl",
        "user_limit": 4,
        "max_limit": 4,
        "disable_rank_caps": True,
        "disable_min_rank": True,
    },
    CASUAL_STAGING_ID: {  # Chill Lanes: kein Rang-System, Name aus Owner-Rang
        "prefix_from_rank": True,
    },
    # COMP/Ranked Staging hat keine Sonderregel → normales Rang-System
}
CASUAL_RANK_FALLBACK = "Chill"
MINRANK_CATEGORY_ID: int = _cfg.TEMPVOICE_CATEGORY_COMP  # Comp/Ranked Kategorie
RANKED_CATEGORY_ID: int = _cfg.TEMPVOICE_CATEGORY_COMP  # Comp/Ranked Kategorie (war: Grind)
INTERFACE_TEXT_CHANNEL_ID: int = (
    _cfg.TEMPVOICE_INTERFACE_CHANNEL
)  # exportiert (wird vom Interface genutzt)
ENGLISH_ONLY_ROLE_ID: int = _cfg.ENGLISH_ONLY_ROLE

DEFAULT_CASUAL_CAP = 8
DEFAULT_RANKED_CAP = 6
NAME_EDIT_COOLDOWN_SEC = 120
STARTUP_PURGE_DELAY_SEC = 3
PURGE_INTERVAL_SECONDS = 180  # Optimiert: 60s → 180s (weniger CPU-Last)
OWNER_CLAIM_TOP_N = 3
OWNER_CLAIM_MIN_SECONDS = 20 * 60

# LiveMatch-Suffix (vom Worker) – NICHT von TempVoice anfassen
LIVE_SUFFIX_RX = re.compile(r"\s+•\s+\d+/\d+\s+(Im\s+Match|Im\s+Spiel|Lobby/Queue)", re.IGNORECASE)
# TempVoice darf nur in diesem Zeitfenster nach Erstellung Namen setzen
ONLY_SET_NAME_ON_CREATE = True
CREATE_RENAME_WINDOW_SEC = 45

RANK_ORDER = [
    "unknown",
    "initiate",
    "seeker",
    "alchemist",
    "arcanist",
    "ritualist",
    "emissary",
    "archon",
    "oracle",
    "phantom",
    "ascendant",
    "eternus",
]
RANK_SET = set(RANK_ORDER)
SUFFIX_THRESHOLD_RANK = "emissary"
MANAGED_PREFIXES = {"lane", "street brawl", CASUAL_RANK_FALLBACK.lower()}.union(RANK_SET)

# Kurzname → Vollname für Sub-Rang Rollen (z.B. "Asc 3" → "ascendant")
_RANK_SHORT_MAP: dict[str, str] = {
    "ini": "initiate",
    "see": "seeker",
    "alc": "alchemist",
    "arc": "arcanist",
    "rit": "ritualist",
    "emi": "emissary",
    "arch": "archon",
    "ora": "oracle",
    "pha": "phantom",
    "asc": "ascendant",
    "ete": "eternus",
}

# Export-Intent für andere Module (verhindert "unused global variable")
__all__ = [
    "STAGING_CHANNEL_IDS",
    "FIXED_LANE_IDS",
    "MINRANK_CATEGORY_ID",
    "MINRANK_CATEGORY_IDS",
    "RANKED_CATEGORY_ID",
    "INTERFACE_TEXT_CHANNEL_ID",
    "ENGLISH_ONLY_ROLE_ID",
    "RANK_ORDER",
]


@dataclass(slots=True)
class LaneTagFilter:
    channel_id: int
    min_age_tag: str | None = None
    required_tone_tag: str | None = None
    deny_ragebaiter: bool = False

    def is_enabled(self) -> bool:
        return bool(self.min_age_tag or self.required_tone_tag or self.deny_ragebaiter)


# --------- Hilfen ---------
def _is_fixed_lane(ch: discord.abc.GuildChannel | int | None) -> bool:
    try:
        lane_id = int(ch) if isinstance(ch, int) else int(getattr(ch, "id", 0))
    except Exception:
        return False
    return lane_id in FIXED_LANE_IDS or lane_id in STAGING_CHANNEL_IDS


def _is_managed_lane(ch: discord.VoiceChannel | None) -> bool:
    if not isinstance(ch, discord.VoiceChannel):
        return False
    if _is_fixed_lane(ch):
        return False
    name = ch.name.lower()
    for prefix in MANAGED_PREFIXES:
        if name == prefix or name.startswith(f"{prefix} "):
            return True
    return False


def _default_cap(ch: discord.abc.GuildChannel) -> int:
    cat_id = getattr(ch, "category_id", None)
    return DEFAULT_RANKED_CAP if cat_id == RANKED_CATEGORY_ID else DEFAULT_CASUAL_CAP


def _rank_index(name: str) -> int:
    n = name.lower()
    return RANK_ORDER.index(n) if n in RANK_SET else 0


_SUBRANK_RE = re.compile(
    rf"^({'|'.join(RANK_ORDER[1:])}|{'|'.join(_RANK_SHORT_MAP.keys())})\\s+([1-6])$",
    re.IGNORECASE,
)


def _rank_score(name: str) -> int:
    """Berechnet einen Score für Rang + Sub-Rang.

    Basis: Hauptrangindex * 6; Sub-Rang 1-6 wird addiert.
    Beispiel: Initiate (1) → 6, Initiate 3 → 9, Emissary 1 → 37.
    """
    n = name.lower().strip()
    base_idx = _rank_index(n)
    if base_idx > 0:
        return base_idx * 6
    m = _SUBRANK_RE.match(n)
    if m:
        rank_part = m.group(1).lower()
        sub = int(m.group(2))
        base_name = _RANK_SHORT_MAP.get(rank_part, rank_part)
        base_idx = _rank_index(base_name)
        if base_idx > 0:
            return base_idx * 6 + sub
    return 0


def _rank_roles(guild: discord.Guild) -> dict[str, discord.Role]:
    out: dict[str, discord.Role] = {}
    for r in guild.roles:
        n = r.name.lower()
        if n in RANK_SET or _SUBRANK_RE.match(n):
            out[n] = r
    return out


def _strip_suffixes(current: str) -> str:
    base = current
    for marker in (" • ab ",):
        if marker in base:
            base = base.split(marker)[0]
    return base


def _has_live_suffix(name: str) -> bool:
    return LIVE_SUFFIX_RX.search(name) is not None


def _age_seconds(ch: discord.VoiceChannel) -> float:
    try:
        return (discord.utils.utcnow() - ch.created_at).total_seconds()
    except Exception as e:
        log.debug("age_seconds failed for %s: %r", getattr(ch, "id", "?"), e)
        return 999999.0


def _rank_prefix_for(member: discord.Member) -> str | None:
    """Ermittle den höchsten Rang des Members anhand der Rollen-Namen."""
    best_idx = _member_rank_index(member)
    if best_idx > 0:
        return RANK_ORDER[best_idx].capitalize()
    return None


def _member_rank_index(member: discord.Member) -> int:
    """Ermittelt den höchsten Rangindex eines Members (unknown=0).
    Erkennt sowohl klassische Rollen ("Ascendant") als auch Sub-Rang Rollen ("Ascendant 3", "Asc 3")."""
    best_idx = 0
    for role in getattr(member, "roles", []):
        try:
            role_name = str(role.name).lower().strip()
        except Exception as e:
            log.debug("member rank parse failed for role on member %s: %r", member.id, e)
            continue
        idx = _rank_index(role_name)
        if idx > best_idx:
            best_idx = idx
            continue
        # Erkennt "RankName N" oder "ShortName N" (z.B. "Ascendant 3" oder "Asc 3")
        if " " in role_name:
            parts = role_name.rsplit(" ", 1)
            if len(parts) == 2 and parts[1].isdigit() and 1 <= int(parts[1]) <= 6:
                rank_part = parts[0]
                idx2 = _rank_index(rank_part)
                if idx2 == 0:
                    full_name = _RANK_SHORT_MAP.get(rank_part)
                    if full_name:
                        idx2 = _rank_index(full_name)
                if idx2 > best_idx:
                    best_idx = idx2
    return best_idx


# --------- Ban-Store ---------
class AsyncBanStore:
    def __init__(self):
        pass

    async def is_banned_by_owner(self, owner_id: int, user_id: int) -> bool:
        try:
            row = await db.query_one_async(
                "SELECT 1 FROM tempvoice_bans WHERE owner_id=? AND banned_id=?",
                (int(owner_id), int(user_id)),
            )
            return row is not None
        except Exception as e:
            log.warning("is_banned_by_owner failed (%s->%s): %r", owner_id, user_id, e)
            return False

    async def list_bans(self, owner_id: int) -> list[int]:
        try:
            rows = await db.query_all_async(
                "SELECT banned_id FROM tempvoice_bans WHERE owner_id=?",
                (int(owner_id),),
            )
            return [int(r["banned_id"]) for r in rows]
        except Exception as e:
            log.warning("list_bans failed for %s: %r", owner_id, e)
            return []

    async def add_ban(self, owner_id: int, user_id: int):
        try:
            await db.execute_async(
                "INSERT OR IGNORE INTO tempvoice_bans(owner_id, banned_id) VALUES(?,?)",
                (int(owner_id), int(user_id)),
            )
        except Exception as e:
            log.warning("add_ban failed (%s->%s): %r", owner_id, user_id, e)

    async def remove_ban(self, owner_id: int, user_id: int):
        try:
            await db.execute_async(
                "DELETE FROM tempvoice_bans WHERE owner_id=? AND banned_id=?",
                (int(owner_id), int(user_id)),
            )
        except Exception as e:
            log.warning("remove_ban failed (%s->%s): %r", owner_id, user_id, e)


# --------- Lurker-Store ---------
class LurkerStore:
    def __init__(self):
        pass

    async def add_lurker(
        self, guild_id: int, channel_id: int, user_id: int, original_nick: str | None
    ):
        try:
            await db.execute_async(
                """
                INSERT INTO tempvoice_lurkers(guild_id, channel_id, user_id, original_nick)
                VALUES(?,?,?,?)
                ON CONFLICT(channel_id, user_id) DO UPDATE SET
                    original_nick=excluded.original_nick,
                    created_at=CURRENT_TIMESTAMP
                """,
                (int(guild_id), int(channel_id), int(user_id), original_nick),
            )
        except Exception as e:
            log.warning("add_lurker failed (%s in %s): %r", user_id, channel_id, e)

    async def get_lurker(self, channel_id: int, user_id: int) -> dict | None:
        try:
            row = await db.query_one_async(
                "SELECT * FROM tempvoice_lurkers WHERE channel_id=? AND user_id=?",
                (int(channel_id), int(user_id)),
            )
            return dict(row) if row else None
        except Exception as e:
            log.warning("get_lurker failed (%s in %s): %r", user_id, channel_id, e)
            return None

    async def remove_lurker(self, channel_id: int, user_id: int):
        try:
            await db.execute_async(
                "DELETE FROM tempvoice_lurkers WHERE channel_id=? AND user_id=?",
                (int(channel_id), int(user_id)),
            )
        except Exception as e:
            log.warning("remove_lurker failed (%s in %s): %r", user_id, channel_id, e)


# --------- Core-Cog ---------
class TempVoiceCore(commands.Cog):
    """Kern: Auto-Lanes, Owner, Persistenz, MinRank, Region-Filter, Purge"""

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        # self._tvdb entfernt - wir nutzen service.db direkt
        self.bans = AsyncBanStore()
        self.lurkers = LurkerStore()

        # Laufzeit-State
        self.created_channels: set[int] = set()
        self.lane_owner: dict[int, int] = {}
        self.lane_initial_owner: dict[int, int] = {}
        self.lane_base: dict[int, str] = {}
        self.lane_min_rank: dict[int, str] = {}
        self.lane_tag_filters: dict[int, LaneTagFilter] = {}
        self._tag_filter_blocked_members: dict[int, set[int]] = {}
        self.join_time: dict[int, dict[int, float]] = {}
        self._edit_locks: dict[int, asyncio.Lock] = {}
        self._lane_creation_locks: dict[int, asyncio.Lock] = {}
        self._last_name_desired: dict[int, str] = {}
        self._last_name_patch_ts: dict[int, float] = {}
        self._bg_tasks: set[asyncio.Task] = set()
        self._shutting_down: bool = False

        # Performance: Role-Caching (5min TTL)
        self._rank_roles_cache: dict[int, dict[str, discord.Role]] = {}
        self._cache_timestamp: dict[int, float] = {}
        self.lane_rules: dict[int, dict[str, Any]] = {}
        self.minrank_blocked_lanes: set[int] = set()
        self.category_rules: dict[int, dict[str, Any]] = {}
        self.category_to_staging: dict[int, int] = {}

    def _rules_for_staging(self, staging: discord.abc.GuildChannel) -> dict[str, Any]:
        try:
            sid = int(getattr(staging, "id", 0))
        except Exception:
            return {}
        return STAGING_RULES.get(sid, {})

    def _rules_from_base(self, base_name: str) -> tuple[dict[str, Any], int | None]:
        base_lower = base_name.lower()
        for sid, rule in STAGING_RULES.items():
            if rule.get("prefix_from_rank"):
                first_token = base_lower.split(" ", 1)[0]
                if first_token in RANK_SET or first_token == CASUAL_RANK_FALLBACK.lower():
                    return rule, sid
                continue
            prefix = str(rule.get("prefix") or "Lane").lower()
            if base_lower.startswith(prefix):
                return rule, sid
        return {}, None

    def _refresh_category_rules(self, guild: discord.Guild | None):
        if not guild:
            return
        for staging_id, rules in STAGING_RULES.items():
            ch = guild.get_channel(int(staging_id))
            if not isinstance(ch, discord.VoiceChannel):
                continue
            cat = ch.category
            if not cat:
                continue
            self.category_rules[int(cat.id)] = rules
            self.category_to_staging[int(cat.id)] = int(staging_id)

    def _rules_for_category(self, category: discord.CategoryChannel | None) -> dict[str, Any]:
        if not category:
            return {}
        return self.category_rules.get(int(category.id), {})

    def _source_staging_for_category(self, category: discord.CategoryChannel | None) -> int | None:
        if not category:
            return None
        return self.category_to_staging.get(int(category.id))

    def _average_rank_prefix_for_lane(self, lane: discord.VoiceChannel) -> str | None:
        """Berechnet den Durchschnittsrang der Lane-Mitglieder (nur bekannte Ränge)."""
        members = [m for m in getattr(lane, "members", []) if isinstance(m, discord.Member)]
        return self._average_rank_prefix_for_members(members)

    def _average_rank_prefix_for_members(self, members: list[discord.Member]) -> str | None:
        """Berechnet den Durchschnittsrang der Member-Liste (nur bekannte Ränge)."""
        if not members:
            return None
        total = 0
        count = 0
        for member in members:
            idx = _member_rank_index(member)
            # Unknown (0) ignorieren: in Chill/Casual soll ohne Rangrolle auf "Lane X" zurückgefallen werden.
            if idx <= 0:
                continue
            total += idx
            count += 1
        if count <= 0:
            return None
        avg_idx = int((total / count) + 0.5)
        avg_idx = max(1, min(avg_idx, len(RANK_ORDER) - 1))
        return RANK_ORDER[avg_idx].capitalize()

    def get_initial_owner_id(self, lane: discord.VoiceChannel | int | None) -> int | None:
        try:
            lane_id = int(lane) if isinstance(lane, int) else int(getattr(lane, "id", 0))
        except Exception:
            return None
        owner_id = self.lane_initial_owner.get(lane_id)
        return int(owner_id) if owner_id else None

    def _extract_lane_number(self, base_name: str) -> str | None:
        """Extract the trailing number from a lane name (e.g., 'Lane 1' -> '1')."""
        if not base_name:
            return None
        match = re.search(r"\s+(\d+)$", base_name)
        return match.group(1) if match else None

    def _desired_prefix_for_rules(self, lane: discord.VoiceChannel, rules: dict[str, Any]) -> str:
        if rules.get("prefix_from_rank"):
            owner_id = self.lane_owner.get(lane.id)
            member = lane.guild.get_member(int(owner_id)) if owner_id else None
            owner_prefix = _rank_prefix_for(member) if member else None
            if owner_prefix:
                return owner_prefix
            return "Lane"
        return str(rules.get("prefix") or "Lane")

    def _store_lane_rules(self, lane_id: int, rules: dict[str, Any]):
        if rules:
            self.lane_rules[lane_id] = rules
        else:
            self.lane_rules.pop(lane_id, None)

    def is_min_rank_blocked(self, lane: discord.VoiceChannel) -> bool:
        return lane.id in self.minrank_blocked_lanes

    def _enforce_limit(self, lane_id: int, desired_limit: int) -> int:
        rules = self.lane_rules.get(lane_id)
        if not rules:
            return desired_limit
        max_limit = rules.get("max_limit")
        if max_limit is None:
            return desired_limit
        try:
            max_limit_int = int(max_limit)
        except (TypeError, ValueError):
            return desired_limit
        return max(1, min(max_limit_int, int(desired_limit)))

    def _default_limit_for_lane(self, lane: discord.abc.GuildChannel) -> int:
        rules = self.lane_rules.get(getattr(lane, "id", 0))
        if rules and "user_limit" in rules:
            try:
                return int(rules["user_limit"])
            except (TypeError, ValueError):
                pass
        return _default_cap(lane)

    def enforce_limit(self, lane: discord.VoiceChannel, requested: int) -> int:
        return self._enforce_limit(lane.id, requested)

    async def _disable_rank_caps(self, lane: discord.VoiceChannel):
        mgr = self.bot.get_cog("RolePermissionVoiceManager")
        if mgr is None:
            try:
                await db.execute_async(
                    """
                    CREATE TABLE IF NOT EXISTS voice_channel_settings (
                        channel_id  INTEGER PRIMARY KEY,
                        guild_id    INTEGER NOT NULL,
                        enabled     INTEGER NOT NULL DEFAULT 1,
                        created_at  TEXT DEFAULT CURRENT_TIMESTAMP,
                        updated_at  TEXT DEFAULT CURRENT_TIMESTAMP
                    )
                    """
                )
                await db.execute_async(
                    """
                    INSERT INTO voice_channel_settings(channel_id, guild_id, enabled)
                    VALUES(?,?,0)
                    ON CONFLICT(channel_id) DO UPDATE SET
                        enabled=0,
                        updated_at=CURRENT_TIMESTAMP
                    """,
                    (int(lane.id), int(getattr(lane.guild, "id", 0))),
                )
            except Exception as e:
                log.debug(
                    "disable_rank_caps: direct DB toggle failed for %s: %r",
                    getattr(lane, "id", "?"),
                    e,
                )
            return
        try:
            await mgr.set_channel_system_enabled(lane, False)
        except Exception as e:
            log.debug(
                "disable_rank_caps: toggle failed for %s: %r",
                getattr(lane, "id", "?"),
                e,
            )
        try:
            await mgr.remove_channel_anchor(lane)
        except Exception as e:
            log.debug(
                "disable_rank_caps: remove anchor failed for %s: %r",
                getattr(lane, "id", "?"),
                e,
            )
        try:
            await mgr.clear_role_permissions(lane)
        except Exception as e:
            log.debug(
                "disable_rank_caps: clear perms failed for %s: %r",
                getattr(lane, "id", "?"),
                e,
            )

    async def _apply_lane_rules(self, lane: discord.VoiceChannel, rules: dict[str, Any]):
        if not rules:
            self._store_lane_rules(lane.id, {})
            self.minrank_blocked_lanes.discard(lane.id)
            return
        self._store_lane_rules(lane.id, rules)
        if rules.get("disable_min_rank"):
            self.minrank_blocked_lanes.discard(lane.id)
            self.lane_min_rank[lane.id] = "unknown"
            try:
                await self._apply_min_rank(lane, "unknown")
            except Exception as e:
                log.debug("apply_lane_rules: reset min rank failed for %s: %r", lane.id, e)
            self.minrank_blocked_lanes.add(lane.id)
        else:
            self.minrank_blocked_lanes.discard(lane.id)
        if rules.get("disable_rank_caps"):
            await self._disable_rank_caps(lane)

    # --------- Lifecycle ---------
    async def cog_load(self):
        # DB Verbindung ist bereits global da
        self._track(self._startup())

    async def cog_unload(self):
        self._shutting_down = True
        for t in list(self._bg_tasks):
            t.cancel()
        if self._bg_tasks:
            await asyncio.gather(*list(self._bg_tasks), return_exceptions=True)

    def _track(self, aw: Any):
        t = asyncio.create_task(aw)
        self._bg_tasks.add(t)
        t.add_done_callback(lambda _: self._bg_tasks.discard(t))

    async def _ensure_tables(self):
        """Stellt sicher, dass TempVoice-spezifische Tabellen existieren."""
        await db.execute_async("""
            CREATE TABLE IF NOT EXISTS tempvoice_bans (
                owner_id    INTEGER NOT NULL,
                banned_id   INTEGER NOT NULL,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (owner_id, banned_id)
            )
        """)
        await db.execute_async("""
            CREATE TABLE IF NOT EXISTS tempvoice_lanes (
                channel_id  INTEGER PRIMARY KEY,
                guild_id    INTEGER NOT NULL,
                owner_id    INTEGER NOT NULL,
                initial_owner_id INTEGER,
                base_name   TEXT NOT NULL,
                category_id INTEGER NOT NULL,
                source_staging_id INTEGER,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        """)
        await db.execute_async(
            """
            CREATE TABLE IF NOT EXISTS tempvoice_presets (
                user_id     INTEGER NOT NULL,
                category_id INTEGER NOT NULL,
                name        TEXT NOT NULL,
                base_name   TEXT NOT NULL,
                "limit"     INTEGER NOT NULL,
                min_rank    TEXT NOT NULL DEFAULT 'unknown',
                region      TEXT NOT NULL DEFAULT 'EU' CHECK(region IN ('DE','EU')),
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (user_id, category_id, name)
            )
            """
        )
        try:
            cols = await db.query_all_async("PRAGMA table_info(tempvoice_presets)")
            col_names = {str(c["name"]) for c in cols}
            if "min_rank" not in col_names:
                await db.execute_async(
                    "ALTER TABLE tempvoice_presets ADD COLUMN min_rank TEXT NOT NULL DEFAULT 'unknown'"
                )
            if "region" not in col_names:
                await db.execute_async(
                    "ALTER TABLE tempvoice_presets ADD COLUMN region TEXT NOT NULL DEFAULT 'EU'"
                )
        except Exception as e:
            log.debug("tempvoice_presets schema check failed: %r", e)
        try:
            cols = await db.query_all_async("PRAGMA table_info(tempvoice_lanes)")
            col_names = {str(c["name"]) for c in cols}
            if "initial_owner_id" not in col_names:
                await db.execute_async(
                    "ALTER TABLE tempvoice_lanes ADD COLUMN initial_owner_id INTEGER"
                )
                await db.execute_async(
                    "UPDATE tempvoice_lanes SET initial_owner_id=owner_id WHERE initial_owner_id IS NULL"
                )
            if "source_staging_id" not in col_names:
                await db.execute_async(
                    "ALTER TABLE tempvoice_lanes ADD COLUMN source_staging_id INTEGER"
                )
        except Exception as e:
            log.debug("tempvoice_lanes schema check failed: %r", e)
        await db.execute_async("""
            CREATE TABLE IF NOT EXISTS tempvoice_owner_prefs (
                owner_id    INTEGER PRIMARY KEY,
                region      TEXT NOT NULL CHECK(region IN ('DE','EU')),
                updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
        """)
        await db.execute_async("""
            CREATE TABLE IF NOT EXISTS tempvoice_lurkers (
                guild_id       INTEGER NOT NULL,
                channel_id     INTEGER NOT NULL,
                user_id        INTEGER NOT NULL,
                original_nick  TEXT,
                created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (channel_id, user_id)
            )
        """)
        await self._ensure_interface_table()

    async def _ensure_interface_table(self):
        rows = await db.query_all_async("PRAGMA table_info(tempvoice_interface)")

        if not rows:
            await db.execute_async("""
                CREATE TABLE IF NOT EXISTS tempvoice_interface (
                    guild_id    INTEGER NOT NULL,
                    channel_id  INTEGER NOT NULL,
                    message_id  INTEGER NOT NULL,
                    category_id INTEGER,
                    lane_id     INTEGER,
                    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (guild_id, message_id),
                    UNIQUE(lane_id)
                )
            """)
            return

        col_names = {str(row["name"]) for row in rows}
        pk_cols = [str(row["name"]) for row in rows if int(row["pk"]) > 0]
        required = {
            "guild_id",
            "channel_id",
            "message_id",
            "category_id",
            "lane_id",
            "created_at",
            "updated_at",
        }

        if required.issubset(col_names) and pk_cols == ["guild_id", "message_id"]:
            return

        # Migration logic
        try:
            await db.execute_async(
                "ALTER TABLE tempvoice_interface RENAME TO tempvoice_interface_old"
            )
        except Exception as e:
            log.debug("tempvoice_interface rename failed (migration skipped): %r", e)
            return

        await db.execute_async("""
            CREATE TABLE IF NOT EXISTS tempvoice_interface (
                guild_id    INTEGER NOT NULL,
                channel_id  INTEGER NOT NULL,
                message_id  INTEGER NOT NULL,
                category_id INTEGER,
                lane_id     INTEGER,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (guild_id, message_id),
                UNIQUE(lane_id)
            )
        """)
        try:
            await db.execute_async("""
                INSERT INTO tempvoice_interface(guild_id, channel_id, message_id, updated_at)
                SELECT guild_id, channel_id, message_id, COALESCE(updated_at, CURRENT_TIMESTAMP)
                FROM tempvoice_interface_old
            """)
            await db.execute_async("DROP TABLE IF EXISTS tempvoice_interface_old")
        except Exception as e:
            log.error("tempvoice_interface migration copy failed: %r - rolling back!", e)
            try:
                await db.execute_async("DROP TABLE IF EXISTS tempvoice_interface")
                await db.execute_async(
                    "ALTER TABLE tempvoice_interface_old RENAME TO tempvoice_interface"
                )
                log.info("tempvoice_interface migration rolled back successfully")
            except Exception as rollback_exc:
                log.critical("Failed to rollback tempvoice_interface migration: %r", rollback_exc)

    async def _startup(self):
        await self.bot.wait_until_ready()
        await asyncio.sleep(STARTUP_PURGE_DELAY_SEC)

        self._refresh_category_rules(self._first_guild())
        await self._ensure_tables()

        await self._rehydrate_lane_tag_filters()
        await self._rehydrate_from_db()
        await self._purge_empty_lanes_once()
        self._track(self._delayed_purge(30))
        log.info("TempVoiceCore bereit • verwaltete Lanes: %d", len(self.created_channels))

    async def _delayed_purge(self, delay: int):
        try:
            await asyncio.sleep(delay)
            while not self._shutting_down:
                # db.connected Check entfällt, da service.db dies managed
                try:
                    await self._purge_empty_lanes_once()
                except Exception as inner:
                    log.exception("TempVoice purge loop failed: %r", inner)
                await asyncio.sleep(PURGE_INTERVAL_SECONDS)
        except asyncio.CancelledError:
            return
        except Exception as e:
            log.exception("TempVoice purge loop crashed: %r", e)

    # --------- Rehydrierung / Purge ---------
    def _first_guild(self) -> discord.Guild | None:
        return self.bot.guilds[0] if self.bot.guilds else None

    async def _rehydrate_from_db(self):
        guild = self._first_guild()
        if not guild:
            return
        try:
            rows = await db.query_all_async(
                "SELECT channel_id, owner_id, initial_owner_id, base_name, category_id, source_staging_id FROM tempvoice_lanes WHERE guild_id=?",
                (int(guild.id),),
            )
        except Exception as e:
            log.warning("rehydrate: fetch failed: %r", e)
            return

        for r in rows:
            lane_id = int(r["channel_id"])
            if _is_fixed_lane(lane_id):
                await self._forget_lane(lane_id)
                continue
            lane: discord.VoiceChannel | None = guild.get_channel(lane_id)  # type: ignore
            if not isinstance(lane, discord.VoiceChannel):
                try:
                    await db.execute_async(
                        "DELETE FROM tempvoice_lanes WHERE channel_id=?", (lane_id,)
                    )
                except Exception as e:
                    log.debug("rehydrate: cleanup row failed for %s: %r", lane_id, e)
                continue

            self.created_channels.add(lane.id)
            self.lane_owner[lane.id] = int(r["owner_id"])
            self.lane_initial_owner[lane.id] = int(r["initial_owner_id"] or r["owner_id"])
            self.lane_base[lane.id] = str(r["base_name"])
            self.lane_min_rank.setdefault(lane.id, "unknown")
            self.join_time.setdefault(lane.id, {})
            rules: dict[str, Any] = {}
            source_id: int | None = None
            try:
                if r["source_staging_id"]:
                    # source_staging_id bekannt → darauf verlassen, KEIN _rules_from_base Fallback.
                    # Lanes aus normalen Ranked/Grind-Stagings haben {} Regeln (kein prefix_from_rank).
                    source_id = int(r["source_staging_id"])
                    rules = STAGING_RULES.get(source_id, {})
                else:
                    # Nur wenn source_staging_id fehlt (alte Lanes ohne DB-Eintrag), Namen-Fallback nutzen.
                    rules, source_id = self._rules_from_base(self.lane_base[lane.id])
                    # Verhindert falsche prefix_from_rank-Regel für Ranked/Grind Lanes
                    if rules.get("prefix_from_rank") and lane.category_id in MINRANK_CATEGORY_IDS:
                        rules = {}
                        source_id = None
            except Exception as e:
                log.debug("rehydrate: staging lookup failed for lane %s: %r", lane.id, e)
            if rules:
                try:
                    await self._apply_lane_rules(lane, rules)
                except Exception as e:
                    log.debug("rehydrate: apply lane rules failed for %s: %r", lane.id, e)
                if source_id and not r["source_staging_id"]:
                    try:
                        await db.execute_async(
                            "UPDATE tempvoice_lanes SET source_staging_id=? WHERE channel_id=?",
                            (int(source_id), int(lane.id)),
                        )
                    except Exception as e:
                        log.debug(
                            "rehydrate: persist source_staging_id failed for %s: %r",
                            lane.id,
                            e,
                        )

            await self._apply_owner_settings(lane, self.lane_owner[lane.id])
            # KEIN aggressives Rename hier – _refresh_name() prüft Schutzbedingungen
            await self._refresh_name(lane)

    async def _rehydrate_lane_tag_filters(self) -> None:
        try:
            rows = await db.query_all_async(
                """
                SELECT channel_id, min_age_tag, required_tone_tag, deny_ragebaiter
                FROM tempvoice_lane_tag_filter
                """
            )
        except Exception as e:
            log.warning("rehydrate lane tag filters failed: %r", e)
            return

        cache: dict[int, LaneTagFilter] = {}
        for row in rows:
            try:
                lane_filter = self._row_to_lane_tag_filter(row)
            except ValueError as e:
                log.debug(
                    "rehydrate lane tag filter skipped for lane %s: %r",
                    row.get("channel_id"),
                    e,
                )
                continue
            cache[lane_filter.channel_id] = lane_filter
        self.lane_tag_filters = cache

    async def _purge_empty_lanes_once(self):
        guild = self._first_guild()
        if not guild:
            return

        try:
            rows = await db.query_all_async(
                "SELECT channel_id FROM tempvoice_lanes WHERE guild_id=?",
                (int(guild.id),),
            )
        except Exception as e:
            log.warning("purge: fetch failed: %r", e)
            return

        processed_lane_ids: set[int] = set()
        for r in rows:
            lane_id = int(r["channel_id"])
            processed_lane_ids.add(lane_id)
            if _is_fixed_lane(lane_id):
                await self._forget_lane(lane_id)
                continue
            try:
                lane = guild.get_channel(lane_id)
                if not isinstance(lane, discord.VoiceChannel):
                    await self._cleanup_lane(
                        lane_id,
                        channel=None,
                        reason="TempVoice: Cleanup (missing channel)",
                    )
                    continue
                if len(lane.members) == 0:
                    await self._cleanup_lane(
                        lane_id, channel=lane, reason="TempVoice: Cleanup (leer)"
                    )
            except Exception as e:
                log.debug("purge: inspect lane %s failed: %r", lane_id, e)

        for ch in list(guild.voice_channels):
            if not _is_managed_lane(ch) or ch.id in processed_lane_ids:
                continue
            try:
                if len(ch.members) == 0:
                    await self._cleanup_lane(
                        int(ch.id), channel=ch, reason="TempVoice: Sweep (leer)"
                    )
            except Exception as e:
                log.debug("sweep: inspect lane %s failed: %r", ch.id, e)

    async def _forget_lane(self, lane_id: int) -> None:
        try:
            await db.execute_async("DELETE FROM tempvoice_lanes WHERE channel_id=?", (lane_id,))
        except Exception as e:
            log.debug("cleanup: delete row %s failed: %r", lane_id, e)
        try:
            await db.execute_async("DELETE FROM tempvoice_interface WHERE lane_id=?", (lane_id,))
        except Exception as e:
            log.debug("cleanup: delete interface row %s failed: %r", lane_id, e)
        try:
            await db.execute_async(
                "DELETE FROM tempvoice_lane_tag_filter WHERE channel_id=?",
                (lane_id,),
            )
        except Exception as e:
            log.debug("cleanup: delete tag filter row %s failed: %r", lane_id, e)

        self.created_channels.discard(lane_id)
        for mapping in (
            self.lane_owner,
            self.lane_initial_owner,
            self.lane_base,
            self.lane_min_rank,
            self.join_time,
            self._last_name_desired,
            self._last_name_patch_ts,
        ):
            mapping.pop(lane_id, None)
        self.lane_tag_filters.pop(lane_id, None)
        self._tag_filter_blocked_members.pop(lane_id, None)
        self.lane_rules.pop(lane_id, None)
        self.minrank_blocked_lanes.discard(lane_id)
        self._edit_locks.pop(lane_id, None)
        try:
            self.bot.dispatch("tempvoice_lane_deleted", lane_id)
        except Exception as e:
            log.debug("dispatch lane_deleted failed for %s: %r", lane_id, e)

    async def _cleanup_lane(
        self,
        lane_id: int,
        *,
        channel: discord.VoiceChannel | None,
        reason: str,
    ) -> None:
        if _is_fixed_lane(lane_id):
            log.debug("TempVoice: skip cleanup for fixed lane %s", lane_id)
            await self._forget_lane(lane_id)
            return

        if channel:
            try:
                await channel.delete(reason=reason)
            except discord.NotFound:
                # Channel wurde bereits gelöscht - das ist OK
                log.debug(
                    "TempVoice: lane %s (%s) bereits gelöscht",
                    lane_id,
                    getattr(channel, "name", "?"),
                )
            except discord.Forbidden as e:
                log.warning(
                    "TempVoice: missing permission to delete lane %s (%s): %s",
                    lane_id,
                    getattr(channel, "name", "?"),
                    e,
                )
            except Exception as e:
                log.warning(
                    "TempVoice: unexpected error deleting lane %s (%s): %r",
                    lane_id,
                    getattr(channel, "name", "?"),
                    e,
                )
        await self._forget_lane(lane_id)

    # --------- Öffentliche Helfer (von UI aufgerufen) ---------
    async def parse_user_identifier(
        self, guild: discord.Guild, raw: str
    ) -> tuple[int | None, str | None]:
        s = raw.strip()
        if not s:
            return None, "Eingabe ist leer."

        # 1. Direkte User Mention (<@ID> oder <@!ID>)
        mention_match = re.match(r"<@!?(\d+)>", s)
        if mention_match:
            try:
                user_id = int(mention_match.group(1))
                return user_id, None
            except ValueError:
                return None, "Ungültiges Format für User-Erwähnung."

        # 2. Reine ID
        if s.isdigit():
            return int(s), None

        # 3. Name (mit oder ohne '@')
        name_search = s
        if name_search.startswith("@"):
            name_search = name_search[1:].strip()
            if not name_search:
                return None, "Nach '@' fehlt der Name."

        low_name_search = name_search.lower()
        matches: list[discord.Member] = []
        for m in guild.members:
            # Check display_name, global_name, and name
            if m.display_name and m.display_name.lower() == low_name_search:
                matches.append(m)
            elif m.global_name and m.global_name.lower() == low_name_search:
                matches.append(m)
            elif m.name and m.name.lower() == low_name_search:
                matches.append(m)

        if len(matches) == 1:
            return matches[0].id, None
        elif len(matches) > 1:
            match_names = ", ".join([f"{m.display_name} ({m.id})" for m in matches[:5]])
            if len(matches) > 5:
                match_names += ", ..."
            return None, f"Mehrere User gefunden: {match_names}. Bitte ID nutzen."

        return None, "Nutzer nicht gefunden."

    async def resolve_member(self, guild: discord.Guild, user_id: int) -> discord.Member | None:
        """Finde ein Member-Objekt für set_permissions (inkl. Fetch-Fallback)."""
        member = guild.get_member(int(user_id))
        if member:
            return member
        try:
            return await guild.fetch_member(int(user_id))
        except discord.NotFound:
            log.debug("resolve_member: user %s not found in guild %s", user_id, guild.id)
        except discord.HTTPException as exc:
            log.debug(
                "resolve_member: fetch_member failed for %s in guild %s: %r",
                user_id,
                guild.id,
                exc,
            )
        return None

    async def set_region_pref(self, owner_id: int, region: str):
        try:
            await db.execute_async(
                """
                INSERT INTO tempvoice_owner_prefs(owner_id, region, updated_at)
                VALUES(?,?,CURRENT_TIMESTAMP)
                ON CONFLICT(owner_id) DO UPDATE SET
                    region=excluded.region,
                    updated_at=CURRENT_TIMESTAMP
                """,
                (int(owner_id), "DE" if region == "DE" else "EU"),
            )
        except Exception as e:
            log.warning("set_region_pref failed for %s: %r", owner_id, e)

    async def get_region_pref(self, owner_id: int) -> str:
        try:
            row = await db.query_one_async(
                "SELECT region FROM tempvoice_owner_prefs WHERE owner_id=?",
                (int(owner_id),),
            )
            if row and str(row["region"]) in ("DE", "EU"):
                return str(row["region"])
        except Exception as e:
            log.debug("get_region_pref failed for %s: %r", owner_id, e)
        return "EU"

    async def apply_region(self, lane: discord.VoiceChannel, region: str):
        role = lane.guild.get_role(ENGLISH_ONLY_ROLE_ID)
        if not role:
            return
        try:
            if region == "DE":
                ow = lane.overwrites_for(role)
                ow.connect = False
                await lane.set_permissions(role, overwrite=ow, reason="TempVoice: Deutsch-Only")
            else:
                await lane.set_permissions(
                    role, overwrite=None, reason="TempVoice: Sprachfilter frei"
                )
        except Exception as e:
            log.debug("apply_region failed for lane %s: %r", lane.id, e)

    async def claim_owner(self, lane: discord.VoiceChannel, member: discord.Member):
        previous_owner = self.lane_owner.get(lane.id)
        if previous_owner == member.id:
            return
        log.info(
            "TempVoice owner claim applied: lane=%s lane_id=%s previous_owner_id=%s new_owner=%s new_owner_id=%s",
            lane.name,
            lane.id,
            previous_owner,
            str(member),
            member.id,
        )
        self.lane_owner[lane.id] = member.id
        try:
            await db.execute_async(
                "UPDATE tempvoice_lanes SET owner_id=? WHERE channel_id=?",
                (int(member.id), int(lane.id)),
            )
        except Exception as e:
            log.debug("claim_owner: db update failed for lane %s: %r", lane.id, e)
        if previous_owner and previous_owner != member.id:
            try:
                await self._clear_owner_bans(lane, previous_owner)
            except Exception as e:
                log.debug("claim_owner: clear_owner_bans failed for lane %s: %r", lane.id, e)
        try:
            await self._apply_owner_settings(lane, member.id)
        except Exception as e:
            log.debug("claim_owner: apply_owner_settings failed for lane %s: %r", lane.id, e)
        try:
            self.bot.dispatch("tempvoice_lane_owner_changed", lane, int(member.id))
        except Exception as e:
            log.debug(
                "claim_owner: dispatch lane_owner_changed failed for lane %s: %r",
                lane.id,
                e,
            )

    # ===== Öffentliche Fassade für das Interface =====
    @property
    def db(self):
        # Legacy support property for interface.py compatibility if needed
        # But interface.py should ideally be refactored too.
        # For now, we can return a dummy object or just assume callers
        # access db directly if we updated interface.py
        return db

    def first_guild(self):
        return self._first_guild()

    # Public helper for interface
    def is_managed_lane(self, ch: discord.VoiceChannel | None) -> bool:
        return _is_managed_lane(ch)

    async def safe_edit_channel(
        self,
        lane: discord.VoiceChannel,
        *,
        desired_name: str | None = None,
        desired_limit: int | None = None,
        reason: str | None = None,
        force_name: bool = False,
    ):
        await self._safe_edit_channel(
            lane,
            desired_name=desired_name,
            desired_limit=desired_limit,
            reason=reason,
            force_name=force_name,
        )

    async def refresh_name(self, lane: discord.VoiceChannel):
        await self._refresh_name(lane)

    async def set_owner_region(self, owner_id: int, region: str):
        await self.set_region_pref(owner_id, region)

    async def apply_owner_region_to_lane(self, lane: discord.VoiceChannel, owner_id: int):
        region = await self.get_region_pref(owner_id)
        await self.apply_region(lane, region)

    def _sorted_lane_members_by_join(
        self, lane: discord.VoiceChannel
    ) -> list[tuple[discord.Member, float | None]]:
        tsmap = self.join_time.get(lane.id, {})
        members = [m for m in lane.members if isinstance(m, discord.Member)]
        members.sort(key=lambda m: (tsmap.get(m.id, float("inf")), m.id))
        return [(member, tsmap.get(member.id)) for member in members]

    def _describe_lane_member_order(
        self, lane: discord.VoiceChannel, *, limit: int = 5
    ) -> list[str]:
        now_ts = datetime.utcnow().timestamp()
        described: list[str] = []
        for idx, (member, joined_ts) in enumerate(self._sorted_lane_members_by_join(lane), start=1):
            if idx > limit:
                break
            if joined_ts is None:
                connected_for = "unknown"
            else:
                connected_for = str(max(0, int(now_ts - joined_ts)))
            described.append(f"{idx}:{member.display_name}:{member.id}:connected_s={connected_for}")
        return described

    def evaluate_owner_claim(
        self, lane: discord.VoiceChannel, member: discord.Member
    ) -> dict[str, Any]:
        now_ts = datetime.utcnow().timestamp()
        owner_id = self.lane_owner.get(lane.id)
        owner_member = lane.guild.get_member(owner_id) if owner_id else None
        owner_present = any(m.id == owner_id for m in lane.members) if owner_id else False
        ranked_members = self._sorted_lane_members_by_join(lane)
        ranked_ids = [m.id for m, _ in ranked_members]
        actor_position = ranked_ids.index(member.id) + 1 if member.id in ranked_ids else None
        joined_ts = self.join_time.get(lane.id, {}).get(member.id)
        connected_seconds = None if joined_ts is None else max(0, int(now_ts - joined_ts))

        reasons: list[str] = []
        if owner_id == member.id:
            reasons.append("already_owner")
        if member.id not in ranked_ids:
            reasons.append("not_in_lane")
        if owner_present and owner_id != member.id:
            reasons.append("current_owner_present")
        if actor_position is None or actor_position > OWNER_CLAIM_TOP_N:
            reasons.append("outside_top_connected")
        if connected_seconds is None or connected_seconds < OWNER_CLAIM_MIN_SECONDS:
            reasons.append("connected_too_short")

        allowed = len(reasons) == 0
        return {
            "allowed": allowed,
            "lane_id": lane.id,
            "lane_name": lane.name,
            "owner_id": owner_id,
            "owner_name": owner_member.display_name if owner_member else None,
            "owner_present": owner_present,
            "actor_id": member.id,
            "actor_name": member.display_name,
            "actor_position": actor_position,
            "connected_seconds": connected_seconds,
            "top_connected": self._describe_lane_member_order(lane, limit=OWNER_CLAIM_TOP_N),
            "reasons": reasons,
        }

    async def request_owner_claim(
        self, lane: discord.VoiceChannel, member: discord.Member
    ) -> tuple[bool, str]:
        status = self.evaluate_owner_claim(lane, member)
        log.info(
            "TempVoice owner claim attempt: lane=%s lane_id=%s current_owner_id=%s current_owner=%s owner_present=%s actor=%s actor_id=%s actor_position=%s actor_connected_s=%s reasons=%s top_connected=%s",
            lane.name,
            lane.id,
            status["owner_id"],
            status["owner_name"],
            status["owner_present"],
            status["actor_name"],
            status["actor_id"],
            status["actor_position"],
            status["connected_seconds"],
            ",".join(status["reasons"]) if status["reasons"] else "eligible",
            status["top_connected"],
        )

        if status["allowed"]:
            await self.claim_owner(lane, member)
            return True, "Du bist jetzt Owner dieser Lane."

        reasons = status["reasons"]
        if "already_owner" in reasons:
            return False, "Du bist bereits Owner dieser Lane."

        details: list[str] = []
        if "current_owner_present" in reasons:
            owner_name = status["owner_name"] or "Der aktuelle Owner"
            details.append(f"{owner_name} ist noch im Channel")
        if "outside_top_connected" in reasons:
            position = status["actor_position"]
            if position is None:
                details.append(
                    f"du bist aktuell nicht sauber in der Join-Reihenfolge erfasst; claimen dürfen nur die ersten {OWNER_CLAIM_TOP_N}"
                )
            else:
                details.append(
                    f"du bist aktuell Platz {position} nach Verbindungszeit; claimen dürfen nur die ersten {OWNER_CLAIM_TOP_N}"
                )
        if "connected_too_short" in reasons:
            connected_seconds = status["connected_seconds"]
            if connected_seconds is None:
                details.append("deine Verbindungszeit ist aktuell nicht verfügbar")
            else:
                details.append(
                    f"du bist erst seit {connected_seconds // 60}m {connected_seconds % 60}s im Channel; mindestens 20 Minuten sind nötig"
                )
        if "not_in_lane" in reasons:
            details.append("du bist nicht mehr sauber in dieser Lane erfasst")

        if not details:
            details.append("die Claim-Bedingungen sind aktuell nicht erfüllt")

        return False, "Owner-Claim derzeit nicht möglich: " + "; ".join(details) + "."

    async def transfer_owner(self, lane: discord.VoiceChannel, member_id: int):
        m = lane.guild.get_member(int(member_id))
        if m:
            await self.claim_owner(lane, m)

    # --------- Channel Updates ---------
    def _lock_for(self, channel_id: int) -> asyncio.Lock:
        lock = self._edit_locks.get(channel_id)
        if lock is None:
            lock = asyncio.Lock()
            self._edit_locks[channel_id] = lock
        return lock

    async def _safe_edit_channel(
        self,
        lane: discord.VoiceChannel,
        *,
        desired_name: str | None = None,
        desired_limit: int | None = None,
        reason: str | None = None,
        force_name: bool = False,
    ):
        if _is_fixed_lane(lane):
            log.debug("TempVoice: skip edit for fixed lane %s", getattr(lane, "id", "?"))
            return
        lock = self._lock_for(lane.id)
        async with lock:
            kwargs: dict[str, Any] = {}
            now = time.time()
            may_rename = False

            # ===== Name bearbeiten? Nur beim Erstellen & wenn KEIN Live-Match-Suffix vorhanden ist =====
            if desired_name is not None and lane.name != desired_name:
                may_rename = True
                if not force_name and ONLY_SET_NAME_ON_CREATE:
                    if _age_seconds(lane) > CREATE_RENAME_WINDOW_SEC:
                        may_rename = False
                if not force_name and _has_live_suffix(lane.name):
                    may_rename = False

                if may_rename:
                    last_desired = self._last_name_desired.get(lane.id)
                    if last_desired == desired_name:
                        last_ts = self._last_name_patch_ts.get(lane.id, 0.0)
                        if now - last_ts >= NAME_EDIT_COOLDOWN_SEC:
                            await self.bot.queue_channel_rename(
                                lane.id,
                                desired_name,
                                reason=reason or "TempVoice: Name Update",
                            )
                            self._last_name_desired[lane.id] = desired_name
                            self._last_name_patch_ts[lane.id] = now
                    else:
                        await self.bot.queue_channel_rename(
                            lane.id,
                            desired_name,
                            reason=reason or "TempVoice: Name Update",
                        )
                        self._last_name_desired[lane.id] = desired_name
                        self._last_name_patch_ts[lane.id] = now
                # sonst: Name bleibt in Ruhe

            if desired_limit is not None:
                desired_limit = self._enforce_limit(lane.id, int(desired_limit))

            if desired_limit is not None and desired_limit != lane.user_limit:
                kwargs["user_limit"] = max(0, min(99, desired_limit))

            if not kwargs:
                return

            try:
                await lane.edit(**kwargs, reason=reason or "TempVoice: Update")
            except discord.HTTPException as e:
                log.warning(
                    "TempVoice: lane.edit failed for %s (payload=%s): %s",
                    lane.id,
                    kwargs,
                    e,
                )

    async def set_lane_template(
        self, lane: discord.VoiceChannel, *, base_name: str, limit: int
    ) -> None:
        base = base_name.strip()
        if not base:
            return
        await self._persist_lane_base(lane.id, base)
        enforced_limit = self._enforce_limit(lane.id, max(0, min(99, limit)))
        await self.safe_edit_channel(
            lane,
            desired_name=base,
            desired_limit=enforced_limit,
            reason=f"TempVoice: Template {base}",
            force_name=True,
        )

    async def reset_lane_template(self, lane: discord.VoiceChannel) -> tuple[str, int]:
        """
        Stellt die Lane auf den Standard-Namen und das Standard-Limit zur�ck.
        - Name: n�chste freie "Lane X" in der Kategorie (oder vorhandene Lane-Basis, falls schon Lane).
        - Limit: Standard-Cap je nach Kategorie (Ranked/Casual).
        """
        rules = self.lane_rules.get(lane.id, {})
        base = self.lane_base.get(lane.id) or _strip_suffixes(lane.name)
        prefix = str(rules.get("prefix") or "Lane")
        if not base.startswith("Lane ") or (rules and not base.lower().startswith(prefix.lower())):
            base = await self._next_name(lane.category, prefix)
        limit = self._default_limit_for_lane(lane)
        await self.set_lane_template(lane, base_name=base, limit=limit)
        return base, limit

    # --------- Presets (nur Ranked) ---------
    async def save_preset(self, lane: discord.VoiceChannel, owner_id: int, preset_name: str):
        preset_name = preset_name.strip()
        if not preset_name:
            raise ValueError("Preset-Name darf nicht leer sein.")
        base = self.lane_base.get(lane.id) or _strip_suffixes(lane.name)
        limit = lane.user_limit or self._default_limit_for_lane(lane)
        enforced_limit = self._enforce_limit(lane.id, limit)
        min_rank = self.lane_min_rank.get(lane.id, "unknown")
        region = await self.get_region_pref(owner_id)
        try:
            await db.execute_async(
                """
                INSERT INTO tempvoice_presets(user_id, category_id, name, base_name, "limit", min_rank, region)
                VALUES(?,?,?,?,?,?,?)
                ON CONFLICT(user_id, category_id, name) DO UPDATE SET
                    base_name=excluded.base_name,
                    "limit"=excluded."limit",
                    min_rank=excluded.min_rank,
                    region=excluded.region,
                    updated_at=CURRENT_TIMESTAMP
                """,
                (
                    int(owner_id),
                    int(lane.category_id or 0),
                    preset_name[:64],
                    base,
                    int(enforced_limit),
                    str(min_rank),
                    "DE" if region == "DE" else "EU",
                ),
            )
        except Exception as e:
            log.debug("save_preset failed for user %s: %r", owner_id, e)
            raise

    async def list_presets(
        self, owner_id: int, category_id: int, limit: int = 25
    ) -> list[dict[str, Any]]:
        try:
            rows = await db.query_all_async(
                """
                SELECT name, base_name, "limit", min_rank, region
                FROM tempvoice_presets
                WHERE user_id=? AND category_id=?
                ORDER BY updated_at DESC
                LIMIT ?
                """,
                (int(owner_id), int(category_id), int(max(1, limit))),
            )
            return rows or []
        except Exception as e:
            log.debug("list_presets failed for user %s: %r", owner_id, e)
            return []

    async def apply_preset(self, lane: discord.VoiceChannel, owner_id: int, name: str) -> bool:
        try:
            row = await db.query_one_async(
                """
                SELECT base_name, "limit", min_rank, region FROM tempvoice_presets
                WHERE user_id=? AND category_id=? AND name=?
                """,
                (int(owner_id), int(lane.category_id or 0), name),
            )
        except Exception as e:
            log.debug("apply_preset fetch failed for user %s: %r", owner_id, e)
            return False
        if not row:
            return False
        base = row["base_name"]
        limit = int(row["limit"])
        min_rank = str(row.get("min_rank") or "unknown")
        region = "DE" if str(row.get("region")) == "DE" else "EU"
        await self.set_lane_template(lane, base_name=base, limit=limit)
        self.lane_min_rank[lane.id] = min_rank

        try:
            if lane.category_id in MINRANK_CATEGORY_IDS and not self.is_min_rank_blocked(lane):
                await self._apply_min_rank(lane, min_rank)
        except Exception as e:
            log.debug("apply_preset: min_rank apply failed for lane %s: %r", lane.id, e)

        try:
            await self.apply_region(lane, region)
        except Exception as e:
            log.debug("apply_preset: apply_region failed for lane %s: %r", lane.id, e)
        return True

    async def apply_preset_batch(
        self, category: discord.CategoryChannel, requester: discord.Member, name: str
    ) -> tuple[int, int]:
        """
        Wendet ein Preset auf alle gemanagten Lanes einer Kategorie an.
        - Admins/Mods (manage_channels) dürfen alle Lanes ändern.
        - Sonst nur Lanes, deren Owner der Anfragende ist.
        Returns (applied, skipped).
        """
        perms = category.permissions_for(requester)
        is_mod = perms.manage_channels or perms.administrator
        applied = 0
        skipped = 0
        for lane in category.voice_channels:
            if not self.is_managed_lane(lane):
                continue
            owner_id = self.lane_owner.get(lane.id)
            if not is_mod and owner_id != requester.id:
                skipped += 1
                continue
            ok = await self.apply_preset(lane, requester.id, name)
            if not ok:
                skipped += 1
                continue
            applied += 1
            try:
                await self.refresh_name(lane)
            except Exception as exc:
                log.debug("apply_preset_batch: refresh_name failed for lane %s: %r", lane.id, exc)
        return applied, skipped

    async def _persist_lane_base(self, lane_id: int, base_name: str) -> None:
        self.lane_base[lane_id] = base_name
        if not db.is_connected():
            return
        try:
            await db.execute_async(
                "UPDATE tempvoice_lanes SET base_name=? WHERE channel_id=?",
                (base_name, int(lane_id)),
            )
        except Exception as e:
            log.debug("TempVoice: update lane base failed (%s): %r", lane_id, e)

    def _compose_name(self, lane: discord.VoiceChannel) -> str:
        rules = self.lane_rules.get(lane.id) or self._rules_for_category(lane.category)
        if rules.get("prefix_from_rank"):
            prefix = self._desired_prefix_for_rules(lane, rules)
            current_base = self.lane_base.get(lane.id) or _strip_suffixes(lane.name)
            number = self._extract_lane_number(current_base)
            base = f"{prefix} {number}" if number else prefix
        else:
            base = self.lane_base.get(lane.id) or _strip_suffixes(lane.name)
        parts = [base]
        if lane.category_id in MINRANK_CATEGORY_IDS:
            min_rank = self.lane_min_rank.get(lane.id, "unknown")
            if (
                min_rank
                and min_rank != "unknown"
                and _rank_index(min_rank) >= _rank_index(SUFFIX_THRESHOLD_RANK)
            ):
                parts.append(f" • ab {min_rank.capitalize()}")
        return "".join(parts)

    async def _refresh_name(self, lane: discord.VoiceChannel):
        if _is_fixed_lane(lane):
            return
        # Schutz: Nie rumpfuschen, wenn LiveMatch-Suffix dran ist oder Channel nicht frisch ist
        if _has_live_suffix(lane.name):
            return
        rules = self.lane_rules.get(lane.id) or self._rules_for_category(lane.category)
        dynamic_rank_prefix = bool(rules.get("prefix_from_rank"))
        if (
            ONLY_SET_NAME_ON_CREATE
            and _age_seconds(lane) > CREATE_RENAME_WINDOW_SEC
            and not dynamic_rank_prefix
        ):
            return

        # Prüfe ob der Name überhaupt geändert werden muss - verhindert redundante API-Calls
        desired_name = self._compose_name(lane)
        if lane.name == desired_name:
            return

        await self._safe_edit_channel(
            lane, desired_name=desired_name, reason="TempVoice: Name aktualisiert"
        )

    def _current_member_and_channel(
        self, guild: discord.Guild, member_id: int
    ) -> tuple[discord.Member | None, discord.VoiceChannel | None]:
        member = guild.get_member(int(member_id))
        channel: discord.VoiceChannel | None = None
        if member and member.voice and isinstance(member.voice.channel, discord.VoiceChannel):
            channel = member.voice.channel
        return member, channel

    async def _next_name(self, category: discord.CategoryChannel | None, prefix: str) -> str:
        if not category:
            return f"{prefix} 1"
        used: set[int] = set()
        pat = re.compile(rf"^{re.escape(prefix)}\s+(\d+)\b")
        for c in category.voice_channels:
            m = pat.match(c.name)
            if m:
                try:
                    used.add(int(m.group(1)))
                except Exception as e:
                    log.debug("next_name: parse existing index failed for %s: %r", c.name, e)
        n = 1
        while n in used:
            n += 1
        return f"{prefix} {n}"

    async def _handle_category_change(
        self, before: discord.VoiceChannel, after: discord.VoiceChannel
    ):
        if _is_fixed_lane(after):
            return
        if not _is_managed_lane(after):
            return
        if after.id not in self.lane_owner:
            return

        guild = after.guild
        self._refresh_category_rules(guild)
        rules = self._rules_for_category(after.category)
        source_id = self._source_staging_for_category(after.category)

        try:
            if rules:
                await self._apply_lane_rules(after, rules)
            else:
                self._store_lane_rules(after.id, {})
                self.minrank_blocked_lanes.discard(after.id)
        except Exception as e:
            log.debug("category change: apply rules failed for %s: %r", after.id, e)

        prefix = self._desired_prefix_for_rules(after, rules)
        base = self.lane_base.get(after.id) or _strip_suffixes(after.name)
        if not base.lower().startswith(prefix.lower()):
            try:
                base = await self._next_name(after.category, prefix)
            except Exception as e:
                log.debug("category change: next_name failed for %s: %r", after.id, e)
                base = prefix

        desired_limit = self._default_limit_for_lane(after)
        await self.set_lane_template(after, base_name=base, limit=desired_limit)

        try:
            await db.execute_async(
                "UPDATE tempvoice_lanes SET category_id=?, source_staging_id=? WHERE channel_id=?",
                (
                    int(after.category_id) if after.category_id else 0,
                    int(source_id) if source_id else None,
                    int(after.id),
                ),
            )
        except Exception as e:
            log.debug("category change: persist failed for lane %s: %r", after.id, e)

        try:
            self.bot.dispatch("tempvoice_lane_category_changed", after, int(after.category_id or 0))
        except Exception as e:
            log.debug("dispatch lane_category_changed failed for %s: %r", after.id, e)

        log.info(
            "TempVoice: Lane %s moved from cat %s to %s -> prefix=%s limit=%s",
            after.id,
            getattr(before, "category_id", None),
            getattr(after, "category_id", None),
            prefix,
            desired_limit,
        )

    async def _apply_owner_bans(self, lane: discord.VoiceChannel, owner_id: int):
        banned = await self.bans.list_bans(owner_id)
        for uid in banned:
            try:
                member = await self.resolve_member(lane.guild, uid)
                if not member:
                    log.debug(
                        "apply_owner_bans: member %s not found in guild %s",
                        uid,
                        lane.guild.id,
                    )
                    continue
                ow = lane.overwrites_for(member)
                ow.connect = False
                await lane.set_permissions(member, overwrite=ow, reason="Owner-Ban (persistent)")
                await asyncio.sleep(0.02)
            except Exception as e:
                log.debug("apply_owner_bans: failed for %s in lane %s: %r", uid, lane.id, e)

    async def _clear_owner_bans(self, lane: discord.VoiceChannel, owner_id: int | None):
        if not owner_id:
            return
        try:
            banned = await self.bans.list_bans(owner_id)
        except Exception as e:
            log.debug("clear_owner_bans: list_bans failed for owner %s: %r", owner_id, e)
            return
        for uid in banned:
            try:
                member = await self.resolve_member(lane.guild, uid)
                if not member:
                    log.debug(
                        "clear_owner_bans: member %s not found in guild %s",
                        uid,
                        lane.guild.id,
                    )
                    continue
                await lane.set_permissions(
                    member, overwrite=None, reason="TempVoice: Ownerwechsel Ban-Reset"
                )
            except Exception as e:
                log.debug(
                    "clear_owner_bans: reset failed for target %s in lane %s: %r",
                    uid,
                    lane.id,
                    e,
                )
            await asyncio.sleep(0.02)

    async def _apply_owner_settings(self, lane: discord.VoiceChannel, owner_id: int):
        region = await self.get_region_pref(owner_id)
        await self.apply_region(lane, region)
        await self._apply_owner_bans(lane, owner_id)
        lane_filter = await self.get_lane_tag_filter(lane.id)
        await self._apply_tag_filter(lane, lane_filter)

    async def _apply_owner_settings_background(self, lane: discord.VoiceChannel, owner_id: int):
        try:
            await self._apply_owner_settings(lane, owner_id)
        except Exception as e:
            log.debug(
                "apply_owner_settings background failed for lane %s: %r",
                getattr(lane, "id", "?"),
                e,
            )

    def _rank_roles_cached(self, guild: discord.Guild) -> dict[str, discord.Role]:
        """Cached version of _rank_roles - invalidiert alle 5 Minuten"""
        now = time.time()
        if guild.id in self._rank_roles_cache:
            if now - self._cache_timestamp.get(guild.id, 0) < 300:  # 5min TTL
                return self._rank_roles_cache[guild.id]

        # Cache miss oder abgelaufen - neu berechnen
        out = {
            r.name.lower(): r
            for r in guild.roles
            if r.name.lower() in RANK_SET or _SUBRANK_RE.match(r.name)
        }
        self._rank_roles_cache[guild.id] = out
        self._cache_timestamp[guild.id] = now
        return out

    async def _apply_min_rank(self, lane: discord.VoiceChannel, min_rank: str):
        if lane.id in self.minrank_blocked_lanes:
            return
        if lane.category_id not in MINRANK_CATEGORY_IDS:
            return
        guild = lane.guild
        ranks = self._rank_roles_cached(guild)  # Nutze gecachte Version!

        # Sammle alle Permission-Updates zuerst, dann parallel ausführen
        async def _set_perm_safe(role, overwrite, reason):
            try:
                await lane.set_permissions(role, overwrite=overwrite, reason=reason)
            except Exception as e:
                log.debug("apply_min_rank %s failed for role %s: %r", reason, role.id, e)

        if min_rank == "unknown":
            # Reset alle Permissions parallel
            tasks = []
            for role in ranks.values():
                ow = lane.overwrites_for(role)
                if ow.connect is not None:
                    tasks.append(_set_perm_safe(role, None, "TempVoice: MinRank reset"))
            if tasks:
                await asyncio.gather(*tasks, return_exceptions=True)
            return

        # Apply MinRank parallel
        min_score = _rank_score(min_rank)
        tasks = []
        for name, role in ranks.items():
            score = _rank_score(name)
            if score == 0:
                continue
            if score < min_score:
                ow = lane.overwrites_for(role)
                ow.connect = False
                tasks.append(_set_perm_safe(role, ow, "TempVoice: MinRank deny"))
            else:
                ow = lane.overwrites_for(role)
                if ow.connect is not None:
                    tasks.append(_set_perm_safe(role, None, "TempVoice: MinRank clear"))
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)

    def _row_to_lane_tag_filter(self, row: dict[str, Any]) -> LaneTagFilter:
        channel_id = int(row["channel_id"])
        min_age_tag = row.get("min_age_tag")
        required_tone_tag = row.get("required_tone_tag")
        deny_ragebaiter = bool(int(row.get("deny_ragebaiter") or 0))
        return LaneTagFilter(
            channel_id=channel_id,
            min_age_tag=self._normalize_min_age_tag(min_age_tag),
            required_tone_tag=self._normalize_required_tone_tag(required_tone_tag),
            deny_ragebaiter=deny_ragebaiter,
        )

    def _normalize_min_age_tag(self, value: Any) -> str | None:
        if value in (None, "", "off"):
            return None
        normalized = str(value).strip().lower()
        if normalized == "25+":
            return "25+"
        raise ValueError(f"Invalid min_age_tag: {value!r}")

    def _normalize_required_tone_tag(self, value: Any) -> str | None:
        if value in (None, "", "off"):
            return None
        normalized = str(value).strip().lower()
        if normalized == "ragebaiter_free":
            return "ragebaiter_free"
        raise ValueError(f"Invalid required_tone_tag: {value!r}")

    async def get_lane_tag_filter(self, channel_id: int) -> LaneTagFilter:
        lane_id = int(channel_id)
        cached = self.lane_tag_filters.get(lane_id)
        if cached is not None:
            return cached
        try:
            row = await db.query_one_async(
                """
                SELECT channel_id, min_age_tag, required_tone_tag, deny_ragebaiter
                FROM tempvoice_lane_tag_filter
                WHERE channel_id=?
                """,
                (lane_id,),
            )
        except Exception as e:
            log.debug("get_lane_tag_filter failed for lane %s: %r", lane_id, e)
            return LaneTagFilter(channel_id=lane_id)
        if not row:
            return LaneTagFilter(channel_id=lane_id)
        lane_filter = self._row_to_lane_tag_filter(row)
        self.lane_tag_filters[lane_id] = lane_filter
        return lane_filter

    async def set_lane_tag_filter(
        self,
        channel_id: int,
        *,
        min_age_tag: str | None,
        required_tone_tag: str | None,
        deny_ragebaiter: bool,
    ) -> LaneTagFilter:
        lane_id = int(channel_id)
        lane_filter = LaneTagFilter(
            channel_id=lane_id,
            min_age_tag=self._normalize_min_age_tag(min_age_tag),
            required_tone_tag=self._normalize_required_tone_tag(required_tone_tag),
            deny_ragebaiter=bool(deny_ragebaiter),
        )
        await db.execute_async(
            """
            INSERT INTO tempvoice_lane_tag_filter(
                channel_id,
                min_age_tag,
                required_tone_tag,
                deny_ragebaiter,
                updated_at
            )
            VALUES(?,?,?,?,CURRENT_TIMESTAMP)
            ON CONFLICT(channel_id) DO UPDATE SET
                min_age_tag=excluded.min_age_tag,
                required_tone_tag=excluded.required_tone_tag,
                deny_ragebaiter=excluded.deny_ragebaiter,
                updated_at=CURRENT_TIMESTAMP
            """,
            (
                lane_id,
                lane_filter.min_age_tag,
                lane_filter.required_tone_tag,
                1 if lane_filter.deny_ragebaiter else 0,
            ),
        )
        self.lane_tag_filters[lane_id] = lane_filter
        return lane_filter

    async def _member_block_reason(
        self,
        member: discord.Member,
        lane_filter: LaneTagFilter,
        *,
        tag_service: Any,
    ) -> str | None:
        if lane_filter.min_age_tag:
            user_tags = await tag_service.get_user_tags(int(member.id))
            if user_tags.get("age") != lane_filter.min_age_tag:
                return "min_age"
        if lane_filter.deny_ragebaiter and await tag_service.has_active_mod_tag(
            int(member.id), "ragebaiter"
        ):
            return "ragebaiter"
        return None

    async def _set_tag_filter_permission(
        self,
        lane: discord.VoiceChannel,
        member: discord.Member,
        *,
        deny: bool,
    ) -> None:
        overwrite = lane.overwrites_for(member)
        if deny:
            if overwrite.connect is False:
                return
            overwrite.connect = False
            await lane.set_permissions(
                member,
                overwrite=overwrite,
                reason="TempVoice: Tag-Filter deny",
            )
            self._tag_filter_blocked_members.setdefault(lane.id, set()).add(int(member.id))
            return

        blocked_members = self._tag_filter_blocked_members.setdefault(lane.id, set())
        if int(member.id) not in blocked_members:
            return
        owner_id = self.lane_owner.get(lane.id)
        if owner_id and await self.bans.is_banned_by_owner(owner_id, int(member.id)):
            blocked_members.discard(int(member.id))
            return
        await lane.set_permissions(member, overwrite=None, reason="TempVoice: Tag-Filter clear")
        blocked_members.discard(int(member.id))

    async def _disconnect_member_for_tag_filter(
        self,
        member: discord.Member,
        *,
        reason: str,
    ) -> None:
        try:
            await member.move_to(None, reason=reason)
        except Exception as e:
            log.debug(
                "tag filter disconnect failed for member %s: %r",
                getattr(member, "id", "?"),
                e,
            )

    async def _apply_tag_filter(
        self,
        lane: discord.VoiceChannel,
        lane_filter: LaneTagFilter,
        *,
        target_members: list[discord.Member] | None = None,
        disconnect_blocked: bool = True,
    ) -> None:
        """Setzt Member-Overwrites für Tag-Filter und trennt geblockte Nutzer bei Bedarf."""
        if not lane_filter.is_enabled():
            members = target_members or list(getattr(lane, "members", []))
            for member in members:
                await self._set_tag_filter_permission(lane, member, deny=False)
            return

        tag_service = self.bot.get_cog("TagService")
        if tag_service is None:
            return

        members = target_members or list(getattr(lane, "members", []))
        for member in members:
            try:
                block_reason = await self._member_block_reason(
                    member,
                    lane_filter,
                    tag_service=tag_service,
                )
                should_block = block_reason is not None
                await self._set_tag_filter_permission(lane, member, deny=should_block)
                if should_block and disconnect_blocked:
                    await self._disconnect_member_for_tag_filter(
                        member,
                        reason="TempVoice: Tag-Filter enforced",
                    )
            except Exception as e:
                log.debug(
                    "apply_tag_filter failed for lane %s member %s: %r",
                    getattr(lane, "id", "?"),
                    getattr(member, "id", "?"),
                    e,
                )

    # --------- Lane-Erstellung ---------
    async def _create_lane(self, member: discord.Member, staging: discord.VoiceChannel):
        guild = member.guild
        member_id = int(member.id)
        lock = self._lane_creation_locks.setdefault(member_id, asyncio.Lock())
        if lock.locked():
            log.debug(
                "TempVoice create_lane: duplicate request for %s ignored (staging=%s)",
                member_id,
                staging.id,
            )
            return

        try:
            async with lock:
                fresh_member, current_channel = self._current_member_and_channel(guild, member_id)
                if not fresh_member or not current_channel or current_channel.id != staging.id:
                    log.debug(
                        "TempVoice create_lane aborted: member=%s not in staging anymore (expected %s, got %s)",
                        member_id,
                        staging.id,
                        getattr(current_channel, "id", None),
                    )
                    return
                member = fresh_member

                rules = self._rules_for_staging(staging)
                cat = staging.category

                # Bestimme initialen Namen: Wenn Ranked-Kategorie, Grind oder prefix_from_rank
                use_rank_name = rules.get("prefix_from_rank") or (
                    cat and cat.id in MINRANK_CATEGORY_IDS
                )

                mgr = self.bot.get_cog("RolePermissionVoiceManager")
                if use_rank_name:
                    # Versuche den Rang-Manager zu nutzen
                    if mgr:
                        rn, rv, rs = mgr.get_user_rank_from_roles(member)
                        if rv is None or rv <= 0:
                            rn = None
                        if rs is None:
                            rs = await mgr.get_user_subrank_from_db(member)
                        if rn and rs:
                            base = f"{rn} {rs}"
                        elif rn:
                            base = rn
                        else:
                            base = None
                    else:
                        idx = _member_rank_index(member)
                        base = RANK_ORDER[idx].capitalize() if idx > 0 else None
                else:
                    base = None

                if not base:
                    prefix = str(rules.get("prefix") or "Lane")
                    base = await self._next_name(cat, prefix)

                bitrate = getattr(guild, "bitrate_limit", None) or 256000
                try:
                    cap = int(rules.get("user_limit", _default_cap(staging)))
                except (TypeError, ValueError):
                    cap = _default_cap(staging)
                try:
                    lane = await guild.create_voice_channel(
                        name=base,
                        category=cat,
                        user_limit=cap,
                        bitrate=bitrate,
                        reason=f"Auto-Lane für {member.display_name}",
                        overwrites=cat.overwrites if cat else None,
                    )
                    log.info(
                        "TempVoice: Lane %s erstellt für %s (staging %s, base=%s, cap=%s)",
                        lane.id,
                        member_id,
                        staging.id,
                        base,
                        cap,
                    )
                except discord.Forbidden:
                    log.error(
                        "TempVoice: fehlende Rechte zum Erstellen von VoiceChannels in cat %s (staging=%s)",
                        getattr(cat, "id", None),
                        staging.id,
                    )
                    return
                except Exception as e:
                    log.error(
                        "TempVoice: create_lane error for member %s in staging %s: %r",
                        member_id,
                        staging.id,
                        e,
                    )
                    return

                self.created_channels.add(lane.id)
                self.lane_owner[lane.id] = member.id
                self.lane_initial_owner[lane.id] = member.id
                self.lane_base[lane.id] = base
                self.lane_min_rank[lane.id] = "unknown"
                self.join_time.setdefault(lane.id, {})
                if rules:
                    try:
                        await self._apply_lane_rules(lane, rules)
                    except Exception as e:
                        log.debug(
                            "create_lane: apply lane rules failed for %s: %r",
                            lane.id,
                            e,
                        )

                try:
                    await db.execute_async(
                        "INSERT OR REPLACE INTO tempvoice_lanes(channel_id, guild_id, owner_id, initial_owner_id, base_name, category_id, source_staging_id) "
                        "VALUES(?,?,?,?,?,?,?)",
                        (
                            int(lane.id),
                            int(guild.id),
                            int(member.id),
                            int(member.id),
                            base,
                            int(cat.id) if cat else 0,
                            int(staging.id) if staging else None,
                        ),
                    )
                except Exception as e:
                    log.warning(
                        "TempVoice: DB insert failed for lane %s (owner=%s, category=%s): %r",
                        lane.id,
                        member.id,
                        getattr(cat, "id", None),
                        e,
                    )

                refreshed_member, refreshed_channel = self._current_member_and_channel(
                    guild, member_id
                )
                if not refreshed_member or not refreshed_channel:
                    await self._cleanup_lane(
                        int(lane.id),
                        channel=lane,
                        reason="TempVoice: Owner nicht auffindbar",
                    )
                    return
                if refreshed_channel.id != staging.id:
                    await self._cleanup_lane(
                        int(lane.id),
                        channel=lane,
                        reason="TempVoice: Owner nicht mehr im Staging",
                    )
                    return
                member = refreshed_member

                try:
                    await member.move_to(lane, reason="TempVoice: Auto-Lane erstellt")
                except discord.Forbidden as e:
                    log.warning(
                        "TempVoice: move_to forbidden (member=%s staging=%s lane=%s): %s",
                        member.id,
                        staging.id,
                        lane.id,
                        e,
                    )
                    await self._cleanup_lane(
                        int(lane.id),
                        channel=lane,
                        reason="TempVoice: Move fehlgeschlagen (forbidden)",
                    )
                    return
                except discord.HTTPException as e:
                    log.warning(
                        "TempVoice: move_to HTTP error (member=%s staging=%s lane=%s): %s",
                        member.id,
                        staging.id,
                        lane.id,
                        e,
                    )
                    await self._cleanup_lane(
                        int(lane.id),
                        channel=lane,
                        reason="TempVoice: Move fehlgeschlagen (http)",
                    )
                    return
                except Exception as e:
                    log.warning(
                        "TempVoice: move_to failed unexpectedly (member=%s staging=%s lane=%s): %r",
                        member.id,
                        staging.id if staging else "?",
                        lane.id,
                        e,
                    )
                    await self._cleanup_lane(
                        int(lane.id),
                        channel=lane,
                        reason="TempVoice: Move fehlgeschlagen (unexpected)",
                    )
                    return

                asyncio.create_task(self._apply_owner_settings_background(lane, member.id))

                # Manueller Trigger für Rang-Permissions & Name im Manager
                # NUR für Lanes die vom RolePermissionVoiceManager überwacht werden (Ranked/Grind).
                # is_monitored_channel prüft monitored_categories – das schließt Chill/Normal Lanes aus.
                if mgr and mgr.is_monitored_channel(lane):

                    async def _delayed_rank_setup(ch, m, mgr_ref):
                        try:
                            # 1s warten damit Discord den Member im Channel sieht
                            await asyncio.sleep(1.0)
                            # Anchor manuell setzen
                            rn, rv, rs = mgr_ref.get_user_rank_from_roles(m)
                            if rs is None:
                                rs = await mgr_ref.get_user_subrank_from_db(m)
                            await mgr_ref.set_channel_anchor(ch, m, rn, rv, rs)
                            # Permissions forcieren (jetzt robuster gegen leere Member-Liste)
                            await mgr_ref.update_channel_permissions_via_roles(ch, force=True)
                        except Exception as exc:
                            log.debug("Delayed rank setup failed for lane %s: %r", ch.id, exc)

                    asyncio.create_task(_delayed_rank_setup(lane, member, mgr))

                # NUR hier initial den Namen setzen (innerhalb des Create-Fensters)
                await self._refresh_name(lane)
                try:
                    self.bot.dispatch("tempvoice_lane_created", lane, member)
                except Exception as e:
                    log.debug("dispatch lane_created failed for %s: %r", lane.id, e)
        finally:
            lock_ref = self._lane_creation_locks.get(member_id)
            if lock_ref and not lock_ref.locked():
                self._lane_creation_locks.pop(member_id, None)

    # --------- Events ---------
    @commands.Cog.listener()
    async def on_guild_channel_update(
        self,
        before: discord.abc.GuildChannel,
        after: discord.abc.GuildChannel,
    ):
        try:
            if not isinstance(before, discord.VoiceChannel) or not isinstance(
                after, discord.VoiceChannel
            ):
                return
            if before.category_id == after.category_id:
                return
            await self._handle_category_change(before, after)
        except Exception as e:
            log.debug(
                "guild_channel_update handler failed for %s: %r",
                getattr(after, "id", "?"),
                e,
            )

    @commands.Cog.listener()
    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ):
        before_channel: discord.VoiceChannel | None = (
            before.channel if before and isinstance(before.channel, discord.VoiceChannel) else None
        )
        after_channel: discord.VoiceChannel | None = (
            after.channel if after and isinstance(after.channel, discord.VoiceChannel) else None
        )
        left_previous_channel = bool(
            before_channel and (not after_channel or before_channel.id != after_channel.id)
        )
        joined_new_channel = bool(
            after_channel and (not before_channel or before_channel.id != after_channel.id)
        )

        # Auto-Lane bei Join in Staging (+ Fallback: Name beginnt mit '(+)' oder '+')
        try:
            joined_staging = False
            if joined_new_channel and after_channel:
                if after_channel.id in STAGING_CHANNEL_IDS:
                    joined_staging = True
            if joined_staging:
                log.info(
                    "TempVoice: %s joined staging %s (configured-id, set=%s) -> create_lane",
                    member.id,
                    after_channel.id,
                    ",".join(str(s) for s in STAGING_CHANNEL_IDS),
                )
                new_player_router = self.bot.get_cog("NewPlayerAdaptiveLanes")
                if new_player_router is not None:
                    try:
                        rerouted = await new_player_router.maybe_route_new_player(
                            member, after_channel
                        )
                    except Exception as e:
                        rerouted = False
                        log.debug(
                            "TempVoice: new player routing hook failed for %s in staging %s: %r",
                            member.id,
                            after_channel.id,
                            e,
                        )
                    if rerouted:
                        return
                await self._create_lane(member, after_channel)
        except Exception as e:
            log.warning("Auto-lane create failed: %s", _safe_log_value(e))

        # Check for Lurker Leave & Owner logic
        try:
            if left_previous_channel and before_channel:
                ch = before_channel

                # --- Lurker Cleanup Start ---
                lurker_data = await self.lurkers.get_lurker(ch.id, member.id)
                if lurker_data:
                    await self.lurkers.remove_lurker(ch.id, member.id)

                    # Role remove
                    role = discord.utils.get(member.guild.roles, name="Lurker")
                    if role:
                        try:
                            await member.remove_roles(role, reason="TempVoice: Lurker left")
                        except Exception as e:
                            log.debug("Lurker role remove failed: %r", e)

                    # Nick restore
                    orig_nick = lurker_data.get("original_nick")
                    # If orig_nick is None/Empty, we reset to None (remove nickname)
                    try:
                        await member.edit(nick=orig_nick, reason="TempVoice: Lurker left")
                    except Exception as e:
                        log.debug("Lurker nick restore failed: %r", e)

                    # Limit decrease
                    if ch.user_limit > 0:
                        await self.safe_edit_channel(
                            ch,
                            desired_limit=max(0, ch.user_limit - 1),
                            reason="TempVoice: Lurker left",
                        )
                # --- Lurker Cleanup End ---

                if ch.id in self.join_time:
                    self.join_time[ch.id].pop(member.id, None)
                if ch.id in self._tag_filter_blocked_members:
                    try:
                        await self._set_tag_filter_permission(ch, member, deny=False)
                    except Exception as e:
                        log.debug(
                            "tag filter leave cleanup failed for lane %s member %s: %r",
                            ch.id,
                            member.id,
                            e,
                        )
                if ch.id in self.lane_owner and self.lane_owner[ch.id] == member.id:
                    if len(ch.members) > 0:
                        tsmap = self.join_time.get(ch.id, {})
                        candidates = list(ch.members)
                        candidates.sort(key=lambda m: tsmap.get(m.id, float("inf")))
                        new_owner_member = candidates[0]
                        log.info(
                            "TempVoice owner auto-transfer: lane=%s lane_id=%s previous_owner=%s previous_owner_id=%s new_owner=%s new_owner_id=%s top_connected=%s",
                            ch.name,
                            ch.id,
                            str(member),
                            member.id,
                            str(new_owner_member),
                            new_owner_member.id,
                            self._describe_lane_member_order(ch, limit=OWNER_CLAIM_TOP_N),
                        )
                        self.lane_owner[ch.id] = new_owner_member.id
                        try:
                            await db.execute_async(
                                "UPDATE tempvoice_lanes SET owner_id=? WHERE channel_id=?",
                                (int(self.lane_owner[ch.id]), int(ch.id)),
                            )
                        except Exception as e:
                            log.debug(
                                "owner transfer db update failed for lane %s: %r",
                                ch.id,
                                e,
                            )
                        try:
                            await self._clear_owner_bans(ch, member.id)
                        except Exception as e:
                            log.debug(
                                "owner transfer clear_owner_bans failed for lane %s: %r",
                                ch.id,
                                e,
                            )
                        try:
                            await self._apply_owner_settings(ch, new_owner_member.id)
                        except Exception as e:
                            log.debug(
                                "owner transfer apply settings failed for lane %s: %r",
                                ch.id,
                                e,
                            )
                        try:
                            self.bot.dispatch(
                                "tempvoice_lane_owner_changed",
                                ch,
                                int(new_owner_member.id),
                            )
                        except Exception as e:
                            log.debug(
                                "dispatch lane_owner_changed failed for %s: %r",
                                ch.id,
                                e,
                            )
                    else:
                        lane_id = int(ch.id)
                        await self._cleanup_lane(lane_id, channel=ch, reason="TempVoice: Lane leer")

                if _is_managed_lane(ch) and len(ch.members) > 0:
                    await self._refresh_name(ch)
        except Exception as e:
            log.debug("owner/cleanup flow failed: %r", e)

        # Join-Zeit & Bannprüfung; KEIN Namens-Refresh mehr außer im Create-Fenster
        try:
            if (
                joined_new_channel
                and after_channel
                and isinstance(after_channel, discord.VoiceChannel)
            ):
                ch = after_channel
                self.join_time.setdefault(ch.id, {})
                self.join_time[ch.id][member.id] = datetime.utcnow().timestamp()

                if _is_managed_lane(ch) and ch.id not in self.lane_owner:
                    base_name = self.lane_base.get(ch.id) or _strip_suffixes(ch.name)
                    self.lane_owner[ch.id] = member.id
                    self.lane_initial_owner.setdefault(ch.id, member.id)
                    self.lane_base[ch.id] = base_name
                    self.created_channels.add(ch.id)
                    self.lane_min_rank.setdefault(ch.id, "unknown")
                    log.info(
                        "TempVoice owner backfill assigned: lane=%s lane_id=%s owner=%s owner_id=%s reason=no_owner_present_on_join",
                        ch.name,
                        ch.id,
                        str(member),
                        member.id,
                    )
                    rules: dict[str, Any] = {}
                    source_id: int | None = None
                    try:
                        rules, source_id = self._rules_from_base(base_name)
                        # Verhindert falsche prefix_from_rank-Regel für Ranked/Grind Lanes
                        if rules.get("prefix_from_rank") and ch.category_id in MINRANK_CATEGORY_IDS:
                            rules = {}
                            source_id = None
                        if rules:
                            await self._apply_lane_rules(ch, rules)
                    except Exception as e:
                        log.debug(
                            "lane owner backfill apply rules failed for %s: %r",
                            ch.id,
                            e,
                        )
                    try:
                        await db.execute_async(
                            """
                            INSERT INTO tempvoice_lanes(channel_id, guild_id, owner_id, initial_owner_id, base_name, category_id, source_staging_id)
                            VALUES(?,?,?,?,?,?,?)
                            ON CONFLICT(channel_id) DO UPDATE SET
                                owner_id=excluded.owner_id,
                                initial_owner_id=COALESCE(tempvoice_lanes.initial_owner_id, excluded.initial_owner_id),
                                base_name=excluded.base_name,
                                category_id=excluded.category_id,
                                source_staging_id=COALESCE(tempvoice_lanes.source_staging_id, excluded.source_staging_id)
                            """,
                            (
                                int(ch.id),
                                int(ch.guild.id),
                                int(member.id),
                                int(member.id),
                                base_name,
                                int(ch.category_id) if ch.category_id else 0,
                                int(source_id) if source_id else None,
                            ),
                        )
                    except Exception as e:
                        log.debug("lane owner backfill db failed for %s: %r", ch.id, e)
                    try:
                        await self._apply_owner_settings(ch, member.id)
                    except Exception as e:
                        log.debug(
                            "lane owner backfill apply settings failed for %s: %r",
                            ch.id,
                            e,
                        )
                    try:
                        self.bot.dispatch("tempvoice_lane_owner_changed", ch, int(member.id))
                    except Exception as e:
                        log.debug(
                            "dispatch lane_owner_changed (backfill) failed for %s: %r",
                            ch.id,
                            e,
                        )

                # Nur falls sehr frisch und noch ohne Live-Suffix – s. _refresh_name
                if _is_managed_lane(ch):
                    lane_filter = await self.get_lane_tag_filter(ch.id)
                    await self._apply_tag_filter(
                        ch,
                        lane_filter,
                        target_members=[member],
                        disconnect_blocked=True,
                    )
                    await self._refresh_name(ch)
        except Exception as e:
            log.debug("post-join flow failed: %r", e)

    @commands.Cog.listener()
    async def on_mod_tag_added(
        self,
        user_id: int,
        tag: str,
        set_by: int,
        reason: str | None,
    ) -> None:
        del set_by, reason
        if tag != "ragebaiter":
            return
        guilds = list(getattr(self.bot, "guilds", []))
        for guild in guilds:
            member = guild.get_member(int(user_id))
            if not member or not getattr(member, "voice", None):
                continue
            lane = getattr(member.voice, "channel", None)
            if lane is None or not getattr(lane, "id", None):
                continue
            lane_filter = self.lane_tag_filters.get(int(lane.id))
            if not lane_filter or not lane_filter.deny_ragebaiter:
                continue
            await self._apply_tag_filter(
                lane,
                lane_filter,
                target_members=[member],
                disconnect_blocked=False,
            )
            if getattr(member.voice, "channel", None) == lane:
                await self._disconnect_member_for_tag_filter(
                    member,
                    reason="TempVoice: Ragebaiter tag applied",
                )


async def setup(bot: commands.Bot):
    await bot.add_cog(TempVoiceCore(bot))
