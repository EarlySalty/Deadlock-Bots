"""
Smart LFG Agent - Deadlock LFG System
Analysiert Anfragen mit KI und routet Spieler basierend auf Skill, Modus und Verfügbarkeit.
"""

from __future__ import annotations

import json
import logging
import re
import time
from collections.abc import Iterable
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta

import discord
from discord.ext import commands

from service import db

log = logging.getLogger("SmartLFG")

# --- Konfiguration ---

# LFG Eingangskanal (User schreibt hier)
LFG_CHANNEL_ID = 1376335502919335936

# Output-Kanal für Bot-Antworten
OUTPUT_CHANNEL_ID = 1376335502919335936

GUILD_ID = 1289721245281292288

# AI Config (ChatGPT/OpenAI)
OPENAI_MODEL = "gpt-5.3"
NO_LFG_TOKEN = "NO_LFG"  # noqa: S105
LFG_INTENT_MAX_TOKENS = 8
USE_AI_LFG_DETECTION = True

# Steam presence freshness (wie lange die Daten maximal alt sein dürfen)
PRESENCE_STALE_SECONDS = 120  # 2 Minuten (reduziert für genauere Erkennung)

# Rank-Matching: wie viele Ränge Unterschied sind erlaubt?
RANK_TOLERANCE = 1.5  # ±1.5 Ränge (mit Subranks)
MAX_MENTION_PINGS = 10
ACTIVITY_LOOKBACK_DAYS = 14
MIN_ACTIVITY_SCORE_SESSIONS = 3
MIN_TIME_MATCH_SCORE = 0.5
# Player-Matching bleibt im Code, ist für die User-Ausgabe aber deaktiviert.
ENABLE_PLAYER_SUGGESTIONS = True
ENABLE_PLAYER_PINGS = True

# Scoring Weights (angepasst auf neues Schema)
WEIGHT_RANK = 20
WEIGHT_TIME = 15
WEIGHT_LANE = 20
WEIGHT_COPLAY = 20
WEIGHT_PRESENCE = 30  # stärkstes Signal: echter Online-Status
WEIGHT_ACTIVITY = 15

# Lane Routing Toleranzen
LANE_RANK_TOLERANCE_RANKED = 2.0  # Ranked: ±2 Ränge
LANE_RANK_TOLERANCE_CASUAL = 3.0  # Casual: ±3 Ränge (war 4.0 - verhindert Initiate→Phantom)
COPLAYER_IN_LANE_BONUS = 40.0  # Score-Bonus wenn Co-Player in Lane
COPLAYER_IN_LANE_SESSIONS_THRESHOLD = 2
ACTIVITY_UPCOMING_WINDOW_HOURS = 2
ACTIVITY_UPCOMING_MIN_SCORE = 4
LOG_CHANNEL_ID = 1374364800817303632  # Decision Logs (separater Admin-Channel)

# Neue Spieler: Initiate bis Arcanist gelten als Anfänger
NEW_PLAYER_MAX_RANK = 4
NEW_PLAYER_FALLBACK_RANK_NAME = "Alchemist"
NEW_PLAYER_FALLBACK_RANK_VALUE = 3
NEW_PLAYER_FALLBACK_SUBRANK = 1
# Maximal angezeigte Lobbys im Finder
MAX_JOIN_LOBBIES_SHOWN = 3

# Spezielle Channel / Kategorien
NEW_PLAYER_CATEGORY_ID = 1465839366634209361
NEW_PLAYER_LANE_ID = 1465839460485697556
NEW_PLAYER_MAX_MEMBERS = 6
STREET_BRAWL_LANE_ID = 1357422958544420944
STAGING_CASUAL_ID = 1501089974093873232
STAGING_STREET_BRAWL_ID = 1357422958544420944
STAGING_RANKED_ID = 1412804671432818890
# Vorgabe des Users: Casual & Ranked Kategorien
CASUAL_CATEGORY_ID = 1289721245281292290
RANKED_CATEGORY_ID = 1412804540994162789
STREET_BRAWL_CATEGORY_ID = 1357422957017698478
JUICE_KAMMER_CHANNEL_ID = 1493690350580138114
JUICE_KAMMER_FIXED_RANK_VALUE = 11
JUICE_KAMMER_FIXED_RANK_LABEL = "Eternus"
OFFTOPIC_NAME_SUBSTRING = "off topic voice"
LOBBY_MAYBE_FULL_THRESHOLD = 6
RANK_WARNING_DIFF = 1.5

# Rollen & Ranks
UNKNOWN_ROLE_ID = 1397687886580547745
UNKNOWN_RANK_NAME = "Unbekannt"
UNKNOWN_RANK_NAME_LOWER = UNKNOWN_RANK_NAME.lower()
# Bis zu welchem Rang sollen Unbekannte gematcht werden (Default: Emissary = 6)
UNKNOWN_MAX_MATCH_RANK = 6

# Rank Definitionen
# 1-5: New Player Friendly
# 6-7: Mid Elo
# 8+: High Elo
DISCORD_RANK_ROLES = {
    1331457571118387210: ("Initiate", 1),
    1331457652877955072: ("Seeker", 2),
    1331457699992436829: ("Alchemist", 3),
    1331457724848017539: ("Arcanist", 4),
    1331457879345070110: ("Ritualist", 5),
    1331457898781474836: ("Emissary", 6),
    1331457949654319114: ("Archon", 7),
    1316966867033653338: ("Oracle", 8),
    1331458016356208680: ("Phantom", 9),
    1331458049637875785: ("Ascendant", 10),
    1331458087349129296: ("Eternus", 11),
    1397687886580547745: ("Unbekannt", 0),
}
# Unverifizierte Rang-Rollen (feste IDs, ein Eintrag pro Tier).
UNVERIFIED_RANK_ROLES = {
    1492959003700101180: ("Eternus", 11),
    1491935935414276198: ("Ascendant", 10),
    1492959474468655134: ("Phantom", 9),
    1492959889767534602: ("Oracle", 8),
    1492959936513052672: ("Archon", 7),
    1492960184920834110: ("Emissary", 6),
    1492960262184239178: ("Ritualist", 5),
    1492960274096066831: ("Arcanist", 4),
    1492960350755225730: ("Alchemist", 3),
    1492959966284218611: ("Seeker", 2),
    1492960891619250408: ("Initiate", 1),
}
UNVERIFIED_ROLE_RE = re.compile(r"^Unverifiziert\s+(.+)$", re.IGNORECASE)

# Sub-Rank Rollen (z. B. "Ascendant 3" oder "Asc 3")
RANK_SHORT_NAMES = {
    "Initiate": "Ini",
    "Seeker": "See",
    "Alchemist": "Alc",
    "Arcanist": "Arc",
    "Ritualist": "Rit",
    "Emissary": "Emi",
    "Archon": "Arch",
    "Oracle": "Ora",
    "Phantom": "Pha",
    "Ascendant": "Asc",
    "Eternus": "Ete",
}
SHORT_NAME_TO_RANK = {v.casefold(): k for k, v in RANK_SHORT_NAMES.items()}
RANK_NAME_TO_VALUE = {name.lower(): val for name, val in DISCORD_RANK_ROLES.values()}
VALUE_TO_RANK_NAME = {
    val: name for name, val in DISCORD_RANK_ROLES.values() if val > 0
}
_SUBRANK_PATTERN = "|".join(
    re.escape(n) for n in list(RANK_NAME_TO_VALUE.keys()) + list(RANK_SHORT_NAMES.values())
)
SUBRANK_ROLE_RE = re.compile(rf"^({_SUBRANK_PATTERN})\s+([1-6])$", re.IGNORECASE)
SHORT_LFG_COUNT_RE = re.compile(
    r"^\s*(?:(?:suche|suchen|lfm|lfg)\s*\+?\s*[1-6]|\+\s*[1-6])\s*$",
    re.IGNORECASE,
)
PLUS_PLAYER_RE = re.compile(r"\+\s*[1-6](?:\D|$)")
MESSAGE_RANK_ALIASES = {
    "ini": "Initiate",
    "seek": "Seeker",
    "alch": "Alchemist",
    "arc": "Arcanist",
    "rit": "Ritualist",
    "emi": "Emissary",
    "emiss": "Emissary",
    "arch": "Archon",
    "asc": "Ascendant",
    "et": "Eternus",
    "arkanist": "Arcanist",
    "ascendent": "Ascendant",
    "ethernus": "Eternus",
}
LFG_AGE_FILTER_RE = re.compile(r"(?<!\d)25\+(?!\d)")
LFG_RAGEBAITER_FREE_RE = re.compile(r"\bragebaiter(?:[\s-]?free)\b", re.IGNORECASE)
VISIBLE_AGE_TAG_LABELS = {
    "25+": "25+",
    "u25": "U25",
}
VISIBLE_TONE_TAG_LABELS = {
    "banter_ok": "Banter-OK",
    "ragebaiter_free": "Ragebaiter-Free",
}


@dataclass
class LaneInfo:
    """Gescannte Lane mit allen relevanten Daten."""

    channel: discord.VoiceChannel
    category_id: int
    label: str  # "Casual" / "Ranked" / "Street Brawl" / "New Player"
    member_count: int
    user_limit: int
    has_space: bool
    slots_free: int
    avg_rank_value: float
    avg_rank_label: str
    deadlock_active_count: int = 0
    member_ids: list[int] = field(default_factory=list)
    co_player_ids_present: list[int] = field(default_factory=list)
    is_staging: bool = False


@dataclass
class LaneRoutingResult:
    """Routing-Entscheidung mit vollständigem Decision Log."""

    best_join_lane: LaneInfo | None = None
    co_player_lanes: list[LaneInfo] = field(default_factory=list)
    suggested_category_id: int = 0
    suggested_category_label: str = ""
    mode: str = "create_new"  # "join_existing" | "create_new" | "co_player_lane"
    decision_log: list[str] = field(default_factory=list)


@dataclass
class UserActivityProfile:
    """Aktivitätsprofil eines Users."""

    typical_hours: list[int] = field(default_factory=list)
    typical_days: list[int] = field(default_factory=list)
    sessions_count_2w: int = 0
    activity_score: int = 0
    top_co_players: list[tuple[int, int, int]] = field(
        default_factory=list
    )  # (co_id, sessions, minutes)
    is_likely_active_now: bool = False
    is_likely_active_soon: bool = False


@dataclass(frozen=True)
class LFGTagFilters:
    min_age_tag: str | None = None
    require_ragebaiter_free: bool = False

    @property
    def enabled(self) -> bool:
        return self.min_age_tag is not None or self.require_ragebaiter_free


class SmartLFGAgent(commands.Cog):
    """
    KI-gesteuerter LFG Bot, der Nutzer basierend auf Rang und Anfrage intelligent zuweist.
    """

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self.lfg_cooldowns: dict[int, float] = {}
        self.cooldown_seconds = 60  # Kurzer Cooldown gegen Spam

    async def cog_load(self) -> None:
        log.info(
            "SmartLFGAgent geladen - LFG Channel: %s | Output Channel: %s",
            LFG_CHANNEL_ID,
            OUTPUT_CHANNEL_ID,
        )

    async def cog_unload(self) -> None:
        log.info("SmartLFGAgent entladen")

    def _parse_subrank_role_name(self, role_name: str) -> tuple[str, int, int] | None:
        """Parst Rollen wie 'Ascendant 3' oder 'Asc 3'."""
        if not role_name:
            return None
        match = SUBRANK_ROLE_RE.match(role_name.strip())
        if not match:
            return None
        rank_label_raw = match.group(1).casefold()
        subrank = int(match.group(2))
        if subrank < 1 or subrank > 6:
            return None

        rank_name = SHORT_NAME_TO_RANK.get(rank_label_raw, rank_label_raw.title())
        rank_val = RANK_NAME_TO_VALUE.get(rank_name.lower())
        if rank_val is None:
            return None
        return rank_name, int(rank_val), subrank

    def _get_user_rank_info(self, member: discord.Member) -> tuple[str, int, int | None]:
        """Gibt (Rank-Name, Rank-Wert, Subrank) zurück, inkl. Subrank-Rollen."""
        highest_name = UNKNOWN_RANK_NAME
        highest_val = 0
        highest_sub: int | None = None
        highest_score = -1  # score = tier*10 + subrank

        for role in member.roles:
            parsed = self._parse_subrank_role_name(getattr(role, "name", ""))
            if parsed:
                r_name, r_val, sub = parsed
                score = r_val * 10 + sub
                if score > highest_score:
                    highest_name, highest_val, highest_sub = r_name, r_val, sub
                    highest_score = score
                continue

            if role.id in DISCORD_RANK_ROLES:
                r_name, r_val = DISCORD_RANK_ROLES[role.id]
                score = r_val * 10 + 5
                if score > highest_score:
                    highest_name, highest_val, highest_sub = r_name, r_val, None
                    highest_score = score

            # Unverifiziert-Rollen per fester ID: gleicher Rang, immer Sub-Rank 3
            if role.id in UNVERIFIED_RANK_ROLES:
                r_name, r_val = UNVERIFIED_RANK_ROLES[role.id]
                score = r_val * 10 + 3
                if score > highest_score:
                    highest_name, highest_val, highest_sub = r_name, r_val, 3
                    highest_score = score

            # Unverifiziert-Rollen per Name (Fallback): gleicher Rang-Name, Sub-Rank 3
            unv_match = UNVERIFIED_ROLE_RE.match(getattr(role, "name", ""))
            if unv_match:
                rank_candidate = unv_match.group(1).strip()
                r_val = RANK_NAME_TO_VALUE.get(rank_candidate.lower())
                if r_val:
                    sub = 3  # immer Sub-Rank 3 für unverifizierte Rollen
                    score = r_val * 10 + sub
                    if score > highest_score:
                        highest_name = rank_candidate.title()
                        highest_val = r_val
                        highest_sub = sub
                        highest_score = score

        if highest_val == 0:
            return (UNKNOWN_RANK_NAME, 0, None)
        return highest_name, highest_val, highest_sub

    def _get_user_rank(self, member: discord.Member) -> tuple[str, int]:
        """Ermittelt den höchsten Rang eines Users (ohne Subrank-Rückgabe)."""
        name, val, _ = self._get_user_rank_info(member)
        return name, val

    # --- Lane Scanning (Phase 2) ---

    def _is_offtopic(self, name: str) -> bool:
        return OFFTOPIC_NAME_SUBSTRING in name.lower()

    def _scan_all_lanes(
        self,
        guild: discord.Guild,
        requester_co_player_ids: set[int],
        steam_online_ids: set[int] | None = None,
    ) -> list[LaneInfo]:
        """Scannt alle Voice-Channels in relevanten Kategorien. Synchron (Discord-Cache)."""
        lanes: list[LaneInfo] = []
        steam_online_ids = steam_online_ids or set()

        def _scan_channel(ch: discord.VoiceChannel, label: str, cat_id: int) -> LaneInfo:
            members = [m for m in ch.members if not m.bot]
            member_ids = [m.id for m in members]
            ranks = []
            for m in members:
                _, r_val, _ = self._get_user_rank_info(m)
                if r_val > 0:
                    ranks.append(r_val)
            avg_rank = sum(ranks) / len(ranks) if ranks else 0.0
            if ch.id == JUICE_KAMMER_CHANNEL_ID:
                avg_rank = JUICE_KAMMER_FIXED_RANK_VALUE
                avg_label = JUICE_KAMMER_FIXED_RANK_LABEL
            elif avg_rank == 0:
                avg_label = "Leer"
            else:
                tier = min(11, max(1, round(avg_rank)))
                avg_label = VALUE_TO_RANK_NAME.get(tier, "Unbekannt")

            limit = ch.user_limit or 99
            if label == "New Player":
                limit = min(limit, NEW_PLAYER_MAX_MEMBERS)
            count = len(members)
            deadlock_active_count = sum(
                1 for mid in member_ids if steam_online_ids and mid in steam_online_ids
            )
            co_present = [mid for mid in member_ids if mid in requester_co_player_ids]
            return LaneInfo(
                channel=ch,
                category_id=cat_id,
                label=label,
                member_count=count,
                user_limit=limit,
                has_space=count < limit,
                slots_free=max(0, limit - count),
                avg_rank_value=avg_rank,
                avg_rank_label=avg_label,
                deadlock_active_count=deadlock_active_count,
                member_ids=member_ids,
                co_player_ids_present=co_present,
            )

        # New Player Kategorie
        np_cat = guild.get_channel(NEW_PLAYER_CATEGORY_ID)
        if isinstance(np_cat, discord.CategoryChannel):
            for vc in np_cat.voice_channels:
                if self._is_offtopic(vc.name):
                    continue
                lanes.append(_scan_channel(vc, "New Player", NEW_PLAYER_CATEGORY_ID))
        elif isinstance(np_ch := guild.get_channel(NEW_PLAYER_LANE_ID), discord.VoiceChannel):
            if not self._is_offtopic(np_ch.name):
                lanes.append(_scan_channel(np_ch, "New Player", np_ch.category_id or 0))

        # Street Brawl
        sb_cat = guild.get_channel(STREET_BRAWL_CATEGORY_ID)
        if isinstance(sb_cat, discord.CategoryChannel):
            for vc in sb_cat.voice_channels:
                if self._is_offtopic(vc.name):
                    continue
                lanes.append(_scan_channel(vc, "Street Brawl", STREET_BRAWL_CATEGORY_ID))
        elif isinstance(sb_ch := guild.get_channel(STREET_BRAWL_LANE_ID), discord.VoiceChannel):
            if not self._is_offtopic(sb_ch.name):
                lanes.append(_scan_channel(sb_ch, "Street Brawl", STREET_BRAWL_CATEGORY_ID))

        # Casual Category
        casual_cat = guild.get_channel(CASUAL_CATEGORY_ID)
        if isinstance(casual_cat, discord.CategoryChannel):
            for vc in casual_cat.voice_channels:
                if self._is_offtopic(vc.name):
                    continue
                if vc.id not in (NEW_PLAYER_LANE_ID, STREET_BRAWL_LANE_ID):
                    lanes.append(_scan_channel(vc, "Casual", CASUAL_CATEGORY_ID))

        # Ranked Category
        ranked_cat = guild.get_channel(RANKED_CATEGORY_ID)
        if isinstance(ranked_cat, discord.CategoryChannel):
            for vc in ranked_cat.voice_channels:
                if self._is_offtopic(vc.name):
                    continue
                lanes.append(_scan_channel(vc, "Ranked", RANKED_CATEGORY_ID))

        for lane in lanes:
            ch_name = lane.channel.name.lower()
            if (
                lane.channel.id
                in (
                    STAGING_CASUAL_ID,
                    STAGING_STREET_BRAWL_ID,
                    STAGING_RANKED_ID,
                )
                or lane.channel.name.startswith("➕")
                or "öffnen" in ch_name
                or "lanes" in ch_name
            ):
                lane.is_staging = True

        return lanes

    def _rank_fits_lane(
        self, requester_rank: int, requester_sub: int | None, lane: LaneInfo
    ) -> bool:
        """Prüft ob der Rang des Requesters zur Lane passt."""
        # Leere Lanes passen immer
        if lane.member_count == 0:
            return True

        req_val = requester_rank + (requester_sub or 5) / 10.0
        lane_val = lane.avg_rank_value

        if lane.label == "Ranked":
            return abs(req_val - lane_val) <= LANE_RANK_TOLERANCE_RANKED
        if lane.label == "Casual" or lane.label == "Street Brawl":
            return abs(req_val - lane_val) <= LANE_RANK_TOLERANCE_CASUAL
        # New Player: Unbekannt oder Low Rank immer OK
        if lane.label == "New Player":
            return requester_rank <= NEW_PLAYER_MAX_RANK or requester_rank == 0
        return True

    # --- Routing Engine (Phase 3) ---

    def _detect_intent(self, content_lower: str, rank_value: int) -> tuple[bool, bool]:
        """Erkennt Intent aus Keywords. Returns (ranked_intent, street_brawl_intent)."""
        ranked_keywords = ("ranked", "grind", "comp", "competitive", "tryhard")
        sb_keywords = ("street brawl", "streetbrawl", "brawl")

        ranked_intent = any(kw in content_lower for kw in ranked_keywords)
        sb_intent = any(kw in content_lower for kw in sb_keywords)

        # Ranked nur wenn Rang bekannt (>0) und mindestens Mid-Elo (>=6)
        if ranked_intent and (rank_value == 0 or rank_value < 6):
            ranked_intent = False

        return ranked_intent, sb_intent

    def _route_to_lane(
        self,
        guild: discord.Guild,
        requester: discord.Member,
        content_lower: str,
        rank_value: int,
        rank_sub: int | None,
        rank_name: str,
        lanes: list[LaneInfo],
        co_player_ids: set[int],
    ) -> LaneRoutingResult:
        """Routing-Entscheidung mit vollständigem Decision Log."""
        result = LaneRoutingResult()
        log_lines = result.decision_log
        start_ms = time.monotonic()

        # Schritt 1: Intent
        ranked_intent, sb_intent = self._detect_intent(content_lower, rank_value)
        sub_str = f" {rank_sub}" if rank_sub else ""
        if sb_intent:
            intent_label = "Street Brawl"
        elif ranked_intent:
            intent_label = "Ranked"
        else:
            intent_label = "Casual"
        log_lines.append(f"Intent: {intent_label} (Rang: {rank_name}{sub_str}, Val: {rank_value})")

        # Schritt 2: Scan Summary
        active_lanes = [lane for lane in lanes if lane.member_count > 0]
        casual_count = sum(1 for lane in active_lanes if lane.label == "Casual")
        ranked_count = sum(1 for lane in active_lanes if lane.label == "Ranked")
        sb_count = sum(1 for lane in active_lanes if lane.label == "Street Brawl")
        np_count = sum(1 for lane in active_lanes if lane.label == "New Player")
        log_lines.append(
            f"Scan: {len(lanes)} Lanes ({casual_count} Casual aktiv, "
            f"{ranked_count} Ranked, {sb_count} SB, {np_count} NP, "
            f"{len(lanes) - len(active_lanes)} leer)"
        )

        # Schritt 3: Filter nach Modus und Rang
        if sb_intent:
            eligible = [lane for lane in lanes if lane.label == "Street Brawl" and lane.has_space]
        elif ranked_intent:
            eligible = [
                lane
                for lane in lanes
                if lane.label == "Ranked"
                and lane.has_space
                and self._rank_fits_lane(rank_value, rank_sub, lane)
            ]
        else:
            # Neue Spieler (Initiate-Arcanist): primär New Player Lanes
            if rank_value > 0 and rank_value <= NEW_PLAYER_MAX_RANK:
                eligible = [
                    lane
                    for lane in lanes
                    if lane.label == "New Player"
                    and lane.has_space
                    and self._rank_fits_lane(rank_value, rank_sub, lane)
                ]
                # Fallback auf Casual nur wenn keine New Player Lane verfügbar
                if not eligible:
                    eligible = [
                        lane
                        for lane in lanes
                        if lane.label in ("Casual", "New Player")
                        and lane.has_space
                        and self._rank_fits_lane(rank_value, rank_sub, lane)
                    ]
            else:
                # Casual + New Player (wenn Low Elo)
                eligible = [
                    lane
                    for lane in lanes
                    if lane.label in ("Casual", "New Player")
                    and lane.has_space
                    and self._rank_fits_lane(rank_value, rank_sub, lane)
                ]
        log_lines.append(f"Rank-Filter: {len(eligible)} Lanes passen")

        # Schritt 4: Co-Player Check
        co_player_lanes = [lane for lane in eligible if lane.co_player_ids_present]
        if co_player_lanes:
            for cl in co_player_lanes:
                co_names = []
                for co_id in cl.co_player_ids_present:
                    m = guild.get_member(co_id)
                    co_names.append(m.display_name if m else str(co_id))
                log_lines.append(f"Co-Player: {', '.join(co_names)} in '{cl.channel.name}'")
            result.co_player_lanes = co_player_lanes

        # Schritt 5: Entscheidung — Beste Lane auswählen
        best: LaneInfo | None = None
        if co_player_lanes:
            # Co-Player Lane hat höchste Prio
            best = max(
                co_player_lanes,
                key=lambda lane: (len(lane.co_player_ids_present), lane.member_count),
            )
            result.mode = "co_player_lane"
            log_lines.append(
                f"Entscheidung: co_player_lane → {best.channel.name} "
                f"({best.member_count}/{best.user_limit}, {best.avg_rank_label})"
            )
        elif eligible:
            # Bevorzuge nicht-leere Lanes, dann nach member_count absteigend
            occupied = [lane for lane in eligible if lane.member_count > 0]
            if occupied:
                best = max(occupied, key=lambda lane: lane.member_count)
                result.mode = "join_existing"
                log_lines.append(
                    f"Entscheidung: join_existing → {best.channel.name} "
                    f"({best.member_count}/{best.user_limit}, {best.avg_rank_label})"
                )
            else:
                best = eligible[0]
                result.mode = "create_new"
                log_lines.append(f"Entscheidung: create_new → {best.channel.name} (leer)")
        else:
            result.mode = "create_new"
            # Fallback: empfehle passende Kategorie
            if sb_intent:
                result.suggested_category_id = STREET_BRAWL_CATEGORY_ID
                result.suggested_category_label = "Street Brawl"
            elif ranked_intent:
                result.suggested_category_id = RANKED_CATEGORY_ID
                result.suggested_category_label = "Ranked"
            elif rank_value > 0 and rank_value <= NEW_PLAYER_MAX_RANK:
                result.suggested_category_id = NEW_PLAYER_CATEGORY_ID
                result.suggested_category_label = "New Player"
            else:
                result.suggested_category_id = CASUAL_CATEGORY_ID
                result.suggested_category_label = "Casual"
            log_lines.append(
                f"Entscheidung: create_new (keine passende Lane, "
                f"Empfehlung: {result.suggested_category_label})"
            )

        result.best_join_lane = best
        elapsed_ms = (time.monotonic() - start_ms) * 1000
        log_lines.append(f"Dauer: {elapsed_ms:.0f}ms")
        return result

    def _keyword_lfg_intent(self, message_content: str) -> bool:
        """
        Primäre LFG-Erkennung per Keyword-Heuristik (kein AI).
        Basiert auf Analyse von 500 echten Nachrichten aus dem LFG-Channel.
        """
        text = (message_content or "").lower()
        if not text:
            return False

        rank_tokens = (
            tuple(RANK_NAME_TO_VALUE.keys())
            + tuple(SHORT_NAME_TO_RANK.keys())
            + tuple(MESSAGE_RANK_ALIASES.keys())
        )

        # Rang-Kontext + Spielwunsch, auch bei lockerer Schreibweise wie "hääte bock auf ründchen".
        if (
            any(token in text for token in rank_tokens)
            and ("bock" in text or "lust" in text)
            and any(
                token in text
                for token in (
                    "runde",
                    "runden",
                    "ründchen",
                    "rundchen",
                    "game",
                    "games",
                    "match",
                    "matches",
                    "spielen",
                    "zocken",
                    "grinden",
                    "gamen",
                )
            )
        ):
            return True

        # --- Sehr kurze LFG-Formate ---
        # "suchen +3", "suche+2", "lfm +1" oder nur "+3"
        if SHORT_LFG_COUNT_RE.match(text):
            return True

        # --- Direkte LFG/LFM Keywords ---
        if "lfg" in text or "lfm" in text:
            return True

        # --- Gruppen-Keywords ---
        if "duo" in text or "trio" in text or "squad" in text or "stack" in text:
            return True

        # --- "bock" Patterns (18+ Treffer im Channel) ---
        # "jemand bock", "jmd bock", "wer bock", "hat wer bock", "iwer bock",
        # "irgendwer bock", "noch bock", "hätte bock"
        if "bock" in text and any(
            w in text
            for w in (
                "jemand",
                "jmd",
                "wer",
                "iwer",
                "irgendwer",
                "noch",
                "hat",
                "hätte",
                "hättest",
            )
        ):
            return True

        # --- "lust" Patterns (10+ Treffer) ---
        # "jemand lust", "jmd lust", "wer lust", "iwer lust", "irgendwer lust"
        if "lust" in text and any(
            w in text
            for w in (
                "jemand",
                "jmd",
                "wer",
                "iwer",
                "irgendwer",
                "hat",
                "noch",
            )
        ):
            return True

        # --- "suche" breit (10+ Treffer) ---
        # "suche leute", "suche spieler", "suche wen", "suche anschluss",
        # "suche noch", "suche nach", "suche jemanden", "suche mates"
        if "suche" in text or "suchen" in text or "gesucht" in text:
            # Kurzform mit Slot-Angabe wie "suchen +3"
            if PLUS_PLAYER_RE.search(text):
                return True
            if any(
                w in text
                for w in (
                    "leute",
                    "spieler",
                    "mitspieler",
                    "team",
                    "gruppe",
                    "party",
                    "wen",
                    "anschluss",
                    "jemand",
                    "noch",
                    "nach",
                    "mates",
                    "mate",
                )
            ):
                return True

        # "sucht noch jemand"
        if "sucht" in text and "jemand" in text:
            return True

        # --- Spielen/Zocken + Frage-Kontext ---
        if ("spielen" in text or "zocken" in text or "grinden" in text or "gamen" in text) and (
            "wer" in text
            or "jemand" in text
            or "bock" in text
            or "jmd" in text
            or "iwer" in text
            or "irgendwer" in text
        ):
            return True

        # --- "paar runden/games" standalone (7+ Treffer) ---
        if "paar runden" in text or "paar games" in text or "paar rounds" in text:
            return True

        # --- "jemand down" / "wer down" (English Slang) ---
        if "down" in text and any(w in text for w in ("jemand", "wer", "iwer", "irgendwer")):
            return True

        # --- "mag wer" (3 Treffer) ---
        if "mag wer" in text:
            return True

        # --- "möchte jemand" (2 Treffer) ---
        if "möchte" in text and ("jemand" in text or "wer" in text):
            return True

        # --- "jemand am start?" ---
        if "am start" in text:
            return True

        # --- "schon jemand wach", "wer wach" ---
        if "wach" in text and any(
            w in text for w in ("jemand", "jmd", "wer", "iwer", "irgendwer")
        ):
            return True

        # --- "noch jemand on?", "wer on" ---
        if any(
            p in text
            for p in ("jemand on", "jmd on", "wer on", "iwer on", "irgendwer on")
        ):
            return True

        if "neuling" in text and "platz" in text and ("jmd" in text or "jemand" in text):
            return True

        # --- "Interesse" mit Such- und Spielkontext ---
        # Beispiele: "hat noch wer interesse?", "hat ein anderer anfänger interesse?"
        if (
            "interesse" in text
            and any(
                w in text
                for w in (
                    "jemand",
                    "wer",
                    "jmd",
                    "iwer",
                    "irgendwer",
                    "anderer",
                    "andere",
                    "noch",
                    "hat",
                    "hätte",
                )
            )
            and any(
                w in text
                for w in (
                    "spielen",
                    "zocken",
                    "grinden",
                    "gamen",
                    "runde",
                    "runden",
                    "game",
                    "games",
                    "match",
                    "matches",
                    "anfänger",
                    "anfanger",
                    "neuling",
                    "neu",
                )
            )
        ):
            return True

        # --- English LFG patterns ---
        if "hmu" in text:
            return True
        if "anyone" in text and ("wanna" in text or "down" in text or "game" in text):
            return True

        # --- "auf der suche" (2 Treffer) ---
        if "auf der suche" in text:
            return True

        return False

    async def _ai_check_lfg_intent(self, message_content: str) -> bool:
        """AI-Check ob die Nachricht wirklich LFG ist (strikt)."""
        if not message_content or not message_content.strip():
            return False
        if self._keyword_lfg_intent(message_content):
            return True

        if not USE_AI_LFG_DETECTION:
            return False

        ai = getattr(self.bot, "get_cog", lambda name: None)("AIConnector")
        if not ai:
            log.warning("AIConnector nicht geladen - LFG-Detection deaktiviert")
            return False

        prompt = (
            "Antworte strikt nur mit 'ja' oder 'nein'. "
            "Sage 'ja' nur, wenn die Nachricht eindeutig Mitspieler für Deadlock JETZT/zeitnah sucht "
            "(LFG/LFM, 'suche Leute', 'wer bock', 'jemand Lust zu zocken', duo/trio/stack). "
            "Sage 'nein' bei Smalltalk, Diskussionen, Memes, News/Leaks, Meinungen oder allem, "
            "was keine klare Spielersuche ist. Im Zweifel immer 'nein'.\n\n"
            f'Nachricht: "{message_content}"'
        )
        try:
            answer_text, _meta = await ai.generate_text(
                provider="openai",
                prompt=prompt,
                system_prompt=None,
                model=OPENAI_MODEL,
                max_output_tokens=LFG_INTENT_MAX_TOKENS,
                temperature=0,
            )
        except Exception as exc:
            log.warning("AI Intent-Check fehlgeschlagen (%s) - Fallback Keywords", exc)
            return self._keyword_lfg_intent(message_content)

        if not answer_text:
            log.warning("AI gab keine Antwort zurück - Fallback auf Keywords")
            return self._keyword_lfg_intent(message_content)

        normalized = str(answer_text).strip().lower()
        if normalized.startswith("ja") or normalized.startswith("yes"):
            return True
        if normalized.startswith("nein") or normalized.startswith("no"):
            return False

        return False

    async def _get_all_steam_links(self) -> dict[int, list[str]]:
        """
        Holt alle Discord User -> Steam ID Mappings.
        Returns: {discord_user_id: [steam_id1, steam_id2, ...]}
        """
        query = """
            SELECT user_id, steam_id
            FROM steam_links
            WHERE steam_id IS NOT NULL AND steam_id != ''
            AND verified = 1
            ORDER BY primary_account DESC, updated_at DESC
        """
        rows = await db.query_all_async(query)

        mapping: dict[int, list[str]] = {}
        for row in rows:
            uid = int(row["user_id"])
            sid = str(row["steam_id"])
            mapping.setdefault(uid, []).append(sid)

        return mapping

    async def _get_steam_friend_ids(self) -> set[int]:
        """Gibt Discord-User-IDs zurück die den Bot als Steam-Freund haben."""
        rows = await db.query_all_async(
            """
            SELECT DISTINCT user_id FROM steam_links
            WHERE verified = 1 AND is_steam_friend = 1 AND user_id > 0
            """
        )
        return {int(r["user_id"]) for r in rows}

    async def _get_online_steam_users(
        self, steam_ids: set[str]
    ) -> dict[str, tuple[str, int | None]]:
        """
        Filtert Steam-IDs nach Online-Status (in Deadlock).
        Returns: {steam_id: (stage, minutes)}
        stage: 'lobby' oder 'match'
        minutes: Spielminuten bei 'match', None bei 'lobby'
        """
        if not steam_ids:
            return {}

        now = int(time.time())
        steam_ids_json = json.dumps(sorted(steam_ids))
        rows = await db.query_all_async(
            """
            SELECT steam_id, deadlock_stage, deadlock_minutes, deadlock_updated_at, last_seen_ts
            FROM live_player_state
            WHERE steam_id IN (SELECT value FROM json_each(?))
            AND (in_deadlock_now = 1 OR deadlock_stage IS NOT NULL)
            """,
            (steam_ids_json,),
        )

        online_map: dict[str, tuple[str, int | None]] = {}

        for row in rows:
            updated_at = row["deadlock_updated_at"] or row["last_seen_ts"]
            if not updated_at:
                continue

            # Zu alte Daten überspringen
            if now - int(updated_at) > PRESENCE_STALE_SECONDS:
                continue

            stage = row["deadlock_stage"]
            if stage not in {"lobby", "match"}:
                continue

            minutes = row["deadlock_minutes"]
            # Snapshot-Korrektur: deadlock_minutes war der Stand beim letzten Update.
            # Wir addieren die seitdem vergangene Zeit für eine genauere Anzeige.
            if stage == "match" and minutes is not None:
                seconds_since_update = now - int(updated_at)
                minutes = int(minutes) + (seconds_since_update // 60)
            online_map[str(row["steam_id"])] = (stage, minutes)

        return online_map

    def _get_deadlock_active_discord_ids(
        self,
        steam_links: dict[int, list[str]],
        online_users: dict[str, tuple[str, int | None]],
    ) -> set[int]:
        if not steam_links or not online_users:
            return set()
        return {
            discord_id
            for discord_id, steam_ids in steam_links.items()
            if any(sid in online_users for sid in steam_ids)
        }

    async def _load_deadlock_presence(
        self,
    ) -> tuple[dict[int, list[str]], dict[str, tuple[str, int | None]], set[int]]:
        steam_links = await self._get_all_steam_links()
        if not steam_links:
            return {}, {}, set()

        all_steam_ids = {sid for sids in steam_links.values() for sid in sids}
        online_users = await self._get_online_steam_users(all_steam_ids)
        steam_online_ids = self._get_deadlock_active_discord_ids(steam_links, online_users)
        return steam_links, online_users, steam_online_ids

    def _chunked(self, items: Iterable[int], size: int = 400) -> Iterable[list[int]]:
        chunk: list[int] = []
        for item in items:
            chunk.append(int(item))
            if len(chunk) >= size:
                yield chunk
                chunk = []
        if chunk:
            yield chunk

    def _parse_json_list(self, raw: str | None) -> list[int]:
        if not raw:
            return []
        try:
            parsed = json.loads(raw)
            if isinstance(parsed, list):
                return [int(x) for x in parsed if str(x).isdigit() or isinstance(x, int)]
        except Exception:
            return []
        return []

    def _parse_tag_filters(self, message_content: str) -> LFGTagFilters:
        # LFG ist aktuell nachrichtenbasiert; bis ein Modal existiert, lesen wir
        # die optionalen Filter direkt aus der Nachricht.
        normalized = (message_content or "").casefold()
        min_age_tag = "25+" if LFG_AGE_FILTER_RE.search(normalized) else None
        require_ragebaiter_free = bool(LFG_RAGEBAITER_FREE_RE.search(normalized))
        return LFGTagFilters(
            min_age_tag=min_age_tag,
            require_ragebaiter_free=require_ragebaiter_free,
        )

    async def _filter_candidate_ids_by_tags(
        self,
        candidate_ids: list[int],
        tag_filters: LFGTagFilters,
    ) -> list[int]:
        if not candidate_ids or not tag_filters.enabled:
            return candidate_ids

        candidate_ids_json = json.dumps([int(user_id) for user_id in candidate_ids])
        now_iso = datetime.now(UTC).isoformat()
        rows = await db.query_all_async(
            """
            SELECT candidate.user_id
            FROM (
              SELECT CAST(value AS INTEGER) AS user_id
              FROM json_each(?)
            ) AS candidate
            WHERE (
              ? IS NULL OR EXISTS (
                SELECT 1
                FROM user_tags AS ut_age
                WHERE ut_age.user_id = candidate.user_id
                  AND ut_age.tag_key = 'age'
                  AND ut_age.tag_value = ?
              )
            )
              AND (
                ? = 0 OR EXISTS (
                  SELECT 1
                  FROM user_tags AS ut_tone
                  WHERE ut_tone.user_id = candidate.user_id
                    AND ut_tone.tag_key = 'tone'
                    AND ut_tone.tag_value = 'ragebaiter_free'
                )
              )
              AND (
                ? = 0 OR NOT EXISTS (
                  SELECT 1
                  FROM user_mod_tags AS mod_tag
                  WHERE mod_tag.user_id = candidate.user_id
                    AND mod_tag.mod_tag = 'ragebaiter'
                    AND (
                      mod_tag.expires_at IS NULL
                      OR mod_tag.expires_at > ?
                    )
                )
              )
            """,
            (
                candidate_ids_json,
                tag_filters.min_age_tag,
                tag_filters.min_age_tag,
                int(tag_filters.require_ragebaiter_free),
                int(tag_filters.require_ragebaiter_free),
                now_iso,
            ),
        )
        allowed_ids = {int(row["user_id"]) for row in rows}
        return [user_id for user_id in candidate_ids if user_id in allowed_ids]

    def _get_tag_service(self) -> object | None:
        get_cog = getattr(self.bot, "get_cog", None)
        if not callable(get_cog):
            return None
        return get_cog("TagService")

    async def _get_visible_user_tag_line(self, user_id: int) -> str | None:
        tag_service = self._get_tag_service()
        if tag_service is None:
            return None

        get_user_tags = getattr(tag_service, "get_user_tags", None)
        if not callable(get_user_tags):
            return None

        try:
            user_tags = await get_user_tags(int(user_id))
        except Exception as exc:
            log.warning("TagService get_user_tags fehlgeschlagen fuer %s: %s", user_id, exc)
            return None

        visible_labels: list[str] = []
        age_value = user_tags.get("age")
        tone_value = user_tags.get("tone")
        if age_value in VISIBLE_AGE_TAG_LABELS:
            visible_labels.append(VISIBLE_AGE_TAG_LABELS[age_value])
        if tone_value in VISIBLE_TONE_TAG_LABELS:
            visible_labels.append(VISIBLE_TONE_TAG_LABELS[tone_value])

        if not visible_labels:
            return None
        return f"Tags: {' · '.join(visible_labels)}"

    def _rank_score(
        self,
        target_rank: int,
        target_sub: int | None,
        cand_rank: int,
        cand_sub: int | None,
    ) -> float:
        """Scoring mit Subranks: toleranter innerhalb derselben Stufe."""
        t_sub = target_sub if target_sub is not None else 5
        c_sub = cand_sub if cand_sub is not None else 5
        diff_points = abs((target_rank * 10 + t_sub) - (cand_rank * 10 + c_sub))
        if diff_points <= 5:
            return 1.0
        if diff_points <= 10:
            return 0.7
        if diff_points <= 20:
            return 0.4
        return 0.0

    def _time_match_score(
        self, typical_hours: list[int], typical_days: list[int], now: datetime
    ) -> float:
        if not typical_hours and not typical_days:
            return 0.0

        hour_match = False
        for typ_hour in typical_hours:
            diff = abs(now.hour - int(typ_hour))
            if diff <= 2 or diff >= 22:
                hour_match = True
                break

        day_match = not typical_days or now.weekday() in typical_days

        if hour_match and day_match:
            return 1.0
        if hour_match:
            return 0.7
        if day_match:
            return 0.4
        return 0.0

    def _get_target_lane_channel_ids(
        self,
        guild: discord.Guild | None,
        content_lower: str,
        author_rank_value: int,
        has_rank: bool,
    ) -> list[int]:
        if not guild:
            return []

        lanes: list[int] = []

        # Spezieller Modus: Street Brawl
        if "street brawl" in content_lower or "streetbrawl" in content_lower:
            sb_cat = guild.get_channel(STREET_BRAWL_CATEGORY_ID)
            if isinstance(sb_cat, discord.CategoryChannel):
                lanes.extend(vc.id for vc in sb_cat.voice_channels)
            if not lanes and STREET_BRAWL_LANE_ID:
                lanes.append(STREET_BRAWL_LANE_ID)
            return lanes

        # Primäre Kategorie: Ranked nur wenn Rank bekannt, sonst Casual
        primary_cat_id = RANKED_CATEGORY_ID if has_rank else CASUAL_CATEGORY_ID

        # Keyword „ranked/grind“ erzwingt Ranked – aber nur wenn Rank erkennbar
        ranked_keyword = "ranked" in content_lower or "grind" in content_lower
        if ranked_keyword and not has_rank:
            primary_cat_id = CASUAL_CATEGORY_ID
        elif ranked_keyword:
            primary_cat_id = RANKED_CATEGORY_ID

        primary_cat = guild.get_channel(primary_cat_id)
        if isinstance(primary_cat, discord.CategoryChannel):
            lanes.extend(vc.id for vc in primary_cat.voice_channels)

        # Low-Elo / unbekannt: New Player Kategorie zusätzlich
        if author_rank_value <= NEW_PLAYER_MAX_RANK:
            np_cat = guild.get_channel(NEW_PLAYER_CATEGORY_ID)
            if isinstance(np_cat, discord.CategoryChannel):
                lanes.extend(vc.id for vc in np_cat.voice_channels)
            elif NEW_PLAYER_LANE_ID not in lanes:
                lanes.append(NEW_PLAYER_LANE_ID)

        # Wenn Rank erkennbar: Casual als Fallback ergänzen (mehr Kandidaten)
        if has_rank and primary_cat_id != CASUAL_CATEGORY_ID:
            casual_cat = guild.get_channel(CASUAL_CATEGORY_ID)
            if isinstance(casual_cat, discord.CategoryChannel):
                lanes.extend(vc.id for vc in casual_cat.voice_channels)

        # Deduplizieren unter Beibehaltung der Reihenfolge
        seen: set[int] = set()
        unique_lanes: list[int] = []
        for cid in lanes:
            if cid not in seen:
                unique_lanes.append(cid)
                seen.add(cid)

        return unique_lanes

    async def _fetch_activity_patterns(
        self,
        user_ids: list[int],
    ) -> dict[int, tuple[list[int], list[int], int]]:
        if not user_ids:
            return {}

        patterns: dict[int, tuple[list[int], list[int], int]] = {}
        for chunk in self._chunked(user_ids):
            chunk_json = json.dumps([int(uid) for uid in chunk])
            rows = await db.query_all_async(
                """
                SELECT user_id, typical_hours, typical_days, activity_score_2w
                FROM user_activity_patterns
                WHERE user_id IN (SELECT CAST(value AS INTEGER) FROM json_each(?))
                """,
                (chunk_json,),
            )
            for row in rows:
                uid = int(row[0])
                hours = self._parse_json_list(row[1])
                days = self._parse_json_list(row[2])
                score = int(row[3] or 0)
                patterns[uid] = (hours, days, score)
        return patterns

    async def _fetch_co_player_stats(self, user_id: int) -> dict[int, tuple[int, int]]:
        rows = await db.query_all_async(
            """
            SELECT co_player_id, sessions_together, total_minutes_together
            FROM user_co_players
            WHERE user_id = ?
            """,
            (user_id,),
        )
        stats: dict[int, tuple[int, int]] = {}
        for row in rows:
            stats[int(row[0])] = (int(row[1] or 0), int(row[2] or 0))
        return stats

    # --- User Activity Profile (Phase 4) ---

    async def _build_user_activity_profile(
        self, user_id: int, now: datetime
    ) -> UserActivityProfile:
        """Baut ein Aktivitätsprofil aus DB-Daten."""
        profile = UserActivityProfile()

        # Activity Patterns
        row = await db.query_one_async(
            """
            SELECT typical_hours, typical_days, activity_score_2w, sessions_count_2w
            FROM user_activity_patterns WHERE user_id = ?
            """,
            (user_id,),
        )
        if row:
            profile.typical_hours = self._parse_json_list(row[0])
            profile.typical_days = self._parse_json_list(row[1])
            profile.activity_score = int(row[2] or 0)
            profile.sessions_count_2w = int(row[3] or 0)

        # Co-Players (top 10 by sessions)
        co_rows = await db.query_all_async(
            """
            SELECT co_player_id, sessions_together, total_minutes_together
            FROM user_co_players WHERE user_id = ?
            ORDER BY sessions_together DESC LIMIT 10
            """,
            (user_id,),
        )
        profile.top_co_players = [(int(r[0]), int(r[1] or 0), int(r[2] or 0)) for r in co_rows]

        # Active now? Check if current hour matches typical hours
        current_hour = now.hour
        if profile.typical_hours:
            profile.is_likely_active_now = any(
                abs(current_hour - h) <= 1 or abs(current_hour - h) >= 23
                for h in profile.typical_hours
            )
        # Active soon? Check upcoming window
        if profile.typical_hours and profile.activity_score >= ACTIVITY_UPCOMING_MIN_SCORE:
            upcoming = [
                (current_hour + i) % 24 for i in range(1, ACTIVITY_UPCOMING_WINDOW_HOURS + 1)
            ]
            profile.is_likely_active_soon = any(h in profile.typical_hours for h in upcoming)

        return profile

    def _find_co_players_in_lanes(
        self,
        guild: discord.Guild,
        profile: UserActivityProfile,
        lanes: list[LaneInfo],
    ) -> dict[int, list[str]]:
        """Welche Co-Player sind gerade in Voice? Returns {co_id: [lane_name, ...]}."""
        co_ids = {cp[0] for cp in profile.top_co_players}
        result: dict[int, list[str]] = {}
        for lane in lanes:
            for mid in lane.member_ids:
                if mid in co_ids:
                    result.setdefault(mid, []).append(lane.channel.name)
        return result

    async def _find_co_players_likely_coming_online(
        self,
        guild: discord.Guild,
        profile: UserActivityProfile,
        now: datetime,
    ) -> list[tuple[int, str]]:
        """Vorhersage: Welche Co-Player kommen in den nächsten 2h wahrscheinlich online?"""
        if not profile.top_co_players:
            return []

        co_ids = [
            cp[0] for cp in profile.top_co_players if cp[1] >= COPLAYER_IN_LANE_SESSIONS_THRESHOLD
        ]
        if not co_ids:
            return []

        patterns = await self._fetch_activity_patterns(co_ids)
        upcoming_hours = [(now.hour + i) % 24 for i in range(1, ACTIVITY_UPCOMING_WINDOW_HOURS + 1)]
        likely: list[tuple[int, str]] = []

        for co_id in co_ids:
            pat = patterns.get(co_id)
            if not pat:
                continue
            typ_hours, typ_days, score = pat
            if score < ACTIVITY_UPCOMING_MIN_SCORE:
                continue
            # Check if any of their typical hours are in our upcoming window
            if any(h in typ_hours for h in upcoming_hours):
                member = guild.get_member(co_id)
                if member and not member.bot and not (member.voice and member.voice.channel):
                    likely.append((co_id, member.display_name))

        return likely

    async def _fetch_lane_activity_users(
        self,
        user_ids: list[int],
        channel_ids: list[int],
        cutoff_str: str,
    ) -> set[int]:
        if not user_ids or not channel_ids:
            return set()

        result: set[int] = set()
        channel_ids_json = json.dumps([int(cid) for cid in channel_ids])
        for chunk in self._chunked(user_ids):
            chunk_json = json.dumps([int(uid) for uid in chunk])
            rows = await db.query_all_async(
                """
                SELECT DISTINCT user_id
                FROM voice_session_log
                WHERE started_at >= ?
                  AND user_id IN (SELECT CAST(value AS INTEGER) FROM json_each(?))
                  AND channel_id IN (SELECT CAST(value AS INTEGER) FROM json_each(?))
                """,
                (cutoff_str, chunk_json, channel_ids_json),
            )
            for row in rows:
                result.add(int(row[0]))
        return result

    def _infer_target_rank_from_coplayers(
        self,
        guild: discord.Guild | None,
        co_player_ids: list[int],
    ) -> int | None:
        if not guild or not co_player_ids:
            return None
        ranks: list[int] = []
        for co_id in co_player_ids:
            member = guild.get_member(co_id)
            if not member:
                continue
            _, r_val, _ = self._get_user_rank_info(member)
            if r_val > 0:
                ranks.append(r_val)
        if not ranks:
            return None
        avg = sum(ranks) / len(ranks)
        return int(round(avg))

    async def _find_matching_players(
        self,
        author: discord.Member,
        message_content: str,
        author_rank_value: int,
        author_subrank: int | None,
        routing_result: LaneRoutingResult | None = None,
        steam_links: dict[int, list[str]] | None = None,
        online_users: dict[str, tuple[str, int | None]] | None = None,
    ) -> list[dict[str, object]]:
        guild = author.guild
        if not guild:
            return []

        if steam_links is None:
            steam_links = await self._get_all_steam_links()
        if not steam_links:
            return []

        steam_friend_ids = await self._get_steam_friend_ids()
        co_player_stats = await self._fetch_co_player_stats(author.id)

        target_rank = author_rank_value
        target_sub = author_subrank
        rank_strict = True
        if target_rank == 0:
            inferred = self._infer_target_rank_from_coplayers(guild, list(co_player_stats.keys()))
            if inferred:
                target_rank = inferred
            else:
                rank_strict = False

        candidate_ids = list(steam_links.keys())
        tag_filters = self._parse_tag_filters(message_content)
        candidate_ids = await self._filter_candidate_ids_by_tags(candidate_ids, tag_filters)
        if not candidate_ids:
            return []
        patterns = await self._fetch_activity_patterns(candidate_ids)

        content_lower = (message_content or "").lower()
        lane_channel_ids = self._get_target_lane_channel_ids(
            guild, content_lower, author_rank_value, has_rank=author_rank_value > 0
        )
        cutoff_str = (datetime.utcnow() - timedelta(days=ACTIVITY_LOOKBACK_DAYS)).strftime(
            "%Y-%m-%d %H:%M:%S"
        )
        lane_active_users = await self._fetch_lane_activity_users(
            candidate_ids, lane_channel_ids, cutoff_str
        )

        if online_users is None:
            all_steam_ids = {sid for sids in steam_links.values() for sid in sids}
            online_users = await self._get_online_steam_users(all_steam_ids)

        user_presence: dict[int, tuple[str, int | None]] = {}
        for discord_id, steam_ids in steam_links.items():
            for sid in steam_ids:
                if sid in online_users:
                    user_presence[discord_id] = online_users[sid]
                    break

        now = datetime.utcnow()
        candidates: list[dict[str, object]] = []

        for discord_id in candidate_ids:
            if discord_id == author.id:
                continue

            member = guild.get_member(discord_id)
            if not member or member.bot:
                continue

            if member.voice and member.voice.channel:
                continue

            # Ist die Person gerade auf Discord aktiv? (online/idle/dnd, nicht offline)
            is_discord_active = member.status not in (
                discord.Status.offline,
                discord.Status.invisible,
            )

            rank_name, rank_value, rank_sub = self._get_user_rank_info(member)

            if rank_strict and target_rank > 0 and rank_value > 0:
                diff_points = abs(
                    (rank_value * 10 + (rank_sub or 5)) - (target_rank * 10 + (target_sub or 5))
                )
                if diff_points > int(RANK_TOLERANCE * 10):
                    continue
                rank_score = self._rank_score(target_rank, target_sub, rank_value, rank_sub)
            elif rank_strict and target_rank > 0 and rank_value == 0:
                if target_rank >= 8:  # High Elo (Oracle+) matched nicht mit Unbekannt
                    continue
                rank_score = 0.3  # Soft-Score: möglich aber nicht priorisiert
            else:
                rank_score = 0.5

            pattern = patterns.get(discord_id)
            typical_hours = pattern[0] if pattern else []
            typical_days = pattern[1] if pattern else []
            activity_sessions = pattern[2] if pattern else 0

            time_score = self._time_match_score(typical_hours, typical_days, now)
            if activity_sessions > 0:
                activity_score = min(1.0, activity_sessions / 8.0)
            else:
                activity_score = 0.3  # neutral statt 0.0 (neue User ohne History)

            lane_score = 1.0 if discord_id in lane_active_users else 0.0

            coplay_score = 0.0
            if discord_id in co_player_stats:
                sessions, minutes = co_player_stats[discord_id]
                coplay_score = min(1.0, (sessions / 5.0) + (minutes / 300.0))

            stage = None
            minutes = None
            presence_score = 0.0
            presence = user_presence.get(discord_id)
            if presence:
                stage, minutes = presence
                if stage == "lobby":
                    presence_score = 1.0
                elif stage == "match":
                    presence_score = 0.7

            if stage is None:
                if activity_sessions == 0 and time_score < MIN_TIME_MATCH_SCORE:
                    continue
                # kein virtual boost – presence_score bleibt 0.0 für echte Offline-User

            # Co-Player in Lane Bonus
            co_lane_bonus = 0.0
            if routing_result and routing_result.co_player_lanes:
                co_lane_member_ids = set()
                for cl in routing_result.co_player_lanes:
                    co_lane_member_ids.update(cl.member_ids)
                if discord_id in co_lane_member_ids:
                    co_lane_bonus = COPLAYER_IN_LANE_BONUS

            # Steam-Freund des Bots → verifiziertere Verbindung, kleiner Bonus
            steam_friend_bonus = 10.0 if discord_id in steam_friend_ids else 0.0

            score = (
                rank_score * WEIGHT_RANK
                + time_score * WEIGHT_TIME
                + lane_score * WEIGHT_LANE
                + coplay_score * WEIGHT_COPLAY
                + presence_score * WEIGHT_PRESENCE
                + activity_score * WEIGHT_ACTIVITY
                + co_lane_bonus
                + steam_friend_bonus
            )

            candidates.append(
                {
                    "user_id": discord_id,
                    "rank_name": rank_name,
                    "rank_value": rank_value,
                    "rank_sub": rank_sub,
                    "stage": stage,
                    "minutes": minutes,
                    "score": score,
                    "time_score": time_score,
                    "activity_sessions": activity_sessions,
                    "discord_active": is_discord_active,
                    "is_steam_friend": discord_id in steam_friend_ids,
                }
            )

        candidates.sort(key=lambda c: float(c.get("score", 0.0)), reverse=True)

        online = [c for c in candidates if c.get("stage") in ("lobby", "match")]
        offline = [c for c in candidates if c.get("stage") not in ("lobby", "match")]

        selected = online
        if len(selected) < MAX_MENTION_PINGS:
            selected += offline[: max(0, MAX_MENTION_PINGS - len(selected))]

        return selected

    def _build_player_lines(
        self,
        guild: discord.Guild,
        candidates: list[dict[str, object]],
    ) -> tuple[list[str], list[str], int, int]:
        """
        Gibt zwei getrennte Listen zurück:
        - ping_lines: Spieler die im Spiel sind aber NICHT auf Discord aktiv
          → werden gepingt (könnten die Nachricht sonst nicht sehen)
        - visible_lines: Spieler die auf Discord aktiv sind
          → nur Name, kein Ping (sehen sowieso alles)
        """
        # Spieler die wir anpingen: im Spiel (Lobby/Match) UND nicht auf Discord aktiv
        ping_in_lobby: list[str] = []
        ping_in_match: list[str] = []

        # Spieler die schon auf Discord sind: nur Namen, kein Ping
        discord_active_names: list[str] = []

        for cand in candidates:
            discord_id = int(cand.get("user_id", 0) or 0)
            if not discord_id:
                continue
            member = guild.get_member(discord_id)
            if not member:
                continue

            stage = cand.get("stage")
            is_discord_active = bool(cand.get("discord_active", False))
            minutes = cand.get("minutes")

            if is_discord_active:
                # Auf Discord aktiv → nur namentlich erwähnen, kein Ping
                rank_name = str(cand.get("rank_name", ""))
                friend_badge = " 🤝" if cand.get("is_steam_friend") else ""
                label = f"{member.display_name}{friend_badge}" + (
                    f" ({rank_name})" if rank_name else ""
                )
                discord_active_names.append(label)
            elif stage == "lobby":
                ping_in_lobby.append(member.mention)
            elif stage == "match":
                suffix = f" ({minutes}m)" if minutes is not None else ""
                ping_in_match.append(f"{member.mention}{suffix}")

        lobby_count = len(ping_in_lobby)
        match_count = len(ping_in_match)

        ping_lines: list[str] = []
        remaining = MAX_MENTION_PINGS

        if ping_in_lobby:
            shown = ping_in_lobby[:remaining]
            remaining -= len(shown)
            extra = len(ping_in_lobby) - len(shown)
            extra_txt = f" (+{extra})" if extra > 0 else ""
            ping_lines.append(f"🟢 In der Lobby: {' '.join(shown)}{extra_txt}")

        if ping_in_match and remaining > 0:
            shown = ping_in_match[:remaining]
            remaining -= len(shown)
            extra = len(ping_in_match) - len(shown)
            extra_txt = f" (+{extra})" if extra > 0 else ""
            ping_lines.append(f"🎮 Im Match: {' '.join(shown)}{extra_txt}")

        visible_lines: list[str] = []
        if discord_active_names:
            shown = discord_active_names[:5]
            extra = len(discord_active_names) - len(shown)
            extra_txt = f" und {extra} weitere" if extra > 0 else ""
            visible_lines.append(", ".join(shown) + extra_txt)

        return ping_lines, visible_lines, lobby_count, match_count

    def _get_voice_state_context(self, guild: discord.Guild) -> str:
        """
        Scannt relevante Voice-Kanäle und baut einen Kontext-String für die KI.
        Zeigt: Name, ID, Anzahl User, Avg Rank (textuell).
        """
        lines = []

        # Helper um Channel-Info zu bauen
        def analyze_channel(channel: discord.VoiceChannel, label: str):
            members = [m for m in channel.members if not m.bot]
            count = len(members)

            ranks = []
            for m in members:
                _, r_val = self._get_user_rank(m)
                if r_val > 0:
                    ranks.append(r_val)

            avg_rank_str = "Leer"
            if ranks:
                avg_val = sum(ranks) / len(ranks)
                # Mapping back roughly to name
                # 1-5 Low, 6-7 Mid, 8+ High
                if avg_val < 5.5:
                    avg_rank_str = f"Low (~{avg_val:.1f})"
                elif avg_val < 7.5:
                    avg_rank_str = f"Mid (~{avg_val:.1f})"
                else:
                    avg_rank_str = f"High (~{avg_val:.1f})"

            link = f"https://discord.com/channels/{guild.id}/{channel.id}"
            return f"- {label}: {channel.name} - {link} (User: {count}, Skill: {avg_rank_str}, ID: {channel.id})"

        # 1. New Player Kategorie
        np_cat = guild.get_channel(NEW_PLAYER_CATEGORY_ID)
        if np_cat and isinstance(np_cat, discord.CategoryChannel):
            empty_shown = 0
            for vc in np_cat.voice_channels:
                if len(vc.members) > 0:
                    lines.append(analyze_channel(vc, "New Player Lane"))
                elif empty_shown < 2:
                    lines.append(analyze_channel(vc, "New Player Lane (Leer)"))
                    empty_shown += 1
        else:
            np_chan = guild.get_channel(NEW_PLAYER_LANE_ID)
            if np_chan and isinstance(np_chan, discord.VoiceChannel):
                lines.append(analyze_channel(np_chan, "New Player Lane"))

        # 2. Street Brawl
        sb_chan = guild.get_channel(STREET_BRAWL_LANE_ID)
        if sb_chan and isinstance(sb_chan, discord.VoiceChannel):
            lines.append(analyze_channel(sb_chan, "Street Brawl Lane"))

        # 3. Casual Category
        cat_casual = guild.get_channel(CASUAL_CATEGORY_ID)
        if cat_casual and isinstance(cat_casual, discord.CategoryChannel):
            # Zeige nur Channels mit Usern ODER die ersten 2 leeren
            empty_shown = 0
            for vc in cat_casual.voice_channels:
                if vc.id in [NEW_PLAYER_LANE_ID, STREET_BRAWL_LANE_ID]:
                    continue  # Skip duplicates
                if len(vc.members) > 0:
                    lines.append(analyze_channel(vc, "Casual Lane"))
                elif empty_shown < 2:
                    lines.append(analyze_channel(vc, "Casual Lane (Leer)"))
                    empty_shown += 1

        # 4. Ranked Category
        cat_ranked = guild.get_channel(RANKED_CATEGORY_ID)
        if cat_ranked and isinstance(cat_ranked, discord.CategoryChannel):
            empty_shown = 0
            for vc in cat_ranked.voice_channels:
                if len(vc.members) > 0:
                    lines.append(analyze_channel(vc, "Ranked/Grind Lane"))
                elif empty_shown < 1:  # Zeige nur 1 leeren Ranked Channel
                    lines.append(analyze_channel(vc, "Ranked/Grind Lane (Leer)"))
                    empty_shown += 1

        return "\n".join(lines)

    def _compose_intro_text(
        self,
        user_mention: str,
        rank_display: str,
        is_new_player: bool,
        has_active_lobbys: bool,
        new_player_lane_occupied: bool = False,
        lobby_count: int = 0,
    ) -> str:
        rank_part = f" ({rank_display})" if rank_display and rank_display != "Unbekannt" else ""

        # Neue Spieler bekommen spezielle, einladende Texte
        if is_new_player:
            if has_active_lobbys and new_player_lane_occupied:
                return (
                    f"Hey {user_mention}!{rank_part}\n"
                    "Willkommen! In der **Neue Spieler Lane** sind schon Leute unterwegs "
                    "— spring rein und spiel mit! Dort triffst du andere, die auch gerade "
                    "anfangen oder entspannt spielen wollen:"
                )
            if has_active_lobbys:
                return (
                    f"Hey {user_mention}!{rank_part}\n"
                    "Willkommen! Ich hab Lobbys gefunden, die gut zu dir passen. "
                    "Schau am besten auch mal in die **Neue Spieler Lane** — "
                    "da sind alle super nett und helfen gerne weiter:"
                )
            # Keine aktive Lobby
            return (
                f"Hey {user_mention}!{rank_part}\n"
                "Willkommen! Gerade ist die **Neue Spieler Lane** noch leer, aber das ist "
                "kein Problem — mach sie einfach auf! Sobald du drin bist, sehen andere "
                "dass jemand da ist und es kommen erfahrungsgemäß schnell Leute dazu. "
                "Trau dich ruhig, hier sind alle freundlich! \U0001f44b"
            )

        if has_active_lobbys:
            if lobby_count == 1:
                lobby_text = "Ich hab eine passende Lobby für dich gefunden"
            else:
                lobby_text = "Ich hab passende Lobbys für dich gefunden"
            return (
                f"Hey {user_mention}!{rank_part}\n"
                f"{lobby_text} — schau rein und spiel mit:"
            )
        return (
            f"Hey {user_mention}!{rank_part}\n"
            "Gerade ist noch niemand in einer Lobby, aber das heißt nicht dass keiner Bock hat!\n"
            "Mach einfach eine Lane auf — erfahrungsgemäß kommen schnell Leute dazu."
        )

    def _resolve_mode_label(
        self,
        routing: LaneRoutingResult,
        is_new_player: bool,
        has_active_lobbys: bool,
        has_explicit_rank: bool,
        rank_value: int,
    ) -> str:
        """Leitet das Zielgebiet für die Anzeige ab."""
        if is_new_player:
            return "New Player"
        if has_active_lobbys and routing.best_join_lane:
            return routing.best_join_lane.label
        if routing.suggested_category_label:
            if (
                routing.suggested_category_label == "Casual"
                and has_explicit_rank
                and rank_value >= 6
            ):
                return "Ranked"
            return routing.suggested_category_label
        if has_explicit_rank and rank_value >= 6:
            return "Ranked"
        return "Casual"

    def _score_lobby_suggestion(
        self,
        lane: LaneInfo,
        routing: LaneRoutingResult,
        rank_value: int,
        rank_sub: int | None,
        is_new_player: bool,
        has_explicit_rank: bool,
    ) -> float:
        """Bewertet eine Lobby für die Top-3-Auswahl."""
        score = 0.0

        if routing.best_join_lane and lane.channel.id == routing.best_join_lane.channel.id:
            score += 1000.0
        if lane.co_player_ids_present:
            score += 250.0 + len(lane.co_player_ids_present) * 25.0
        if lane.member_count > 0:
            score += 200.0 + lane.member_count * 15.0
        if is_new_player and lane.label == "New Player":
            score += 1200.0
        if routing.suggested_category_id and lane.category_id == routing.suggested_category_id:
            score += 80.0
        if rank_value > 0 and lane.avg_rank_value > 0:
            rank_diff = abs(lane.avg_rank_value - (rank_value + (rank_sub or 5) / 10.0))
            # Starke Strafe für extreme Rang-Unterschiede (>3 Ränge)
            if rank_diff > 3.0:
                score -= 500.0
            elif has_explicit_rank:
                score += max(0.0, 140.0 - rank_diff * 35.0)
            else:
                score += max(0.0, 80.0 - rank_diff * 20.0)
        if has_explicit_rank and lane.label == "Ranked":
            score += 50.0

        return score

    def _select_lobby_suggestions(
        self,
        lanes: list[LaneInfo],
        routing: LaneRoutingResult,
        rank_value: int,
        rank_sub: int | None,
        is_new_player: bool,
        has_explicit_rank: bool,
    ) -> list[LaneInfo]:
        """Wählt bis zu drei klare Lobby-Vorschläge aus."""
        candidates: list[LaneInfo] = []
        seen_lane_ids: set[int] = set()
        use_rank_filtering = has_explicit_rank or (is_new_player and rank_value > 0)

        for lane in lanes:
            if not lane.has_space or lane.is_staging or lane.channel.id in seen_lane_ids:
                continue

            if not use_rank_filtering:
                if lane.member_count > 0:
                    candidates.append(lane)
                continue

            if lane.member_count == 0:
                continue

            if lane.label == "New Player":
                if is_new_player:
                    candidates.append(lane)
                continue

            if lane.label == "Street Brawl":
                wants_brawl = (
                    routing.best_join_lane and routing.best_join_lane.label == "Street Brawl"
                )
                if wants_brawl and self._rank_fits_lane(rank_value, rank_sub, lane):
                    candidates.append(lane)
                continue

            if lane.label not in ("Casual", "Ranked"):
                continue

            if rank_value > 0 and not self._rank_fits_lane(rank_value, rank_sub, lane):
                continue

            # Harter Filter: Keine Vorschläge mit extremem Rang-Unterschied (>3 Ränge)
            # Verhindert z.B. Initiate (1) → Phantom (9) Vorschläge
            if rank_value > 0 and lane.avg_rank_value > 0:
                rank_diff = abs(rank_value - lane.avg_rank_value)
                if rank_diff > 3:
                    continue

            candidates.append(lane)

        ranked_candidates = sorted(
            candidates,
            key=lambda lane: (
                self._score_lobby_suggestion(
                    lane, routing, rank_value, rank_sub, is_new_player, use_rank_filtering
                ),
                lane.member_count,
                lane.slots_free,
                -lane.channel.position,
            ),
            reverse=True,
        )

        # Gibt es eine klar passende Lobby (Rang innerhalb der Toleranz),
        # zeigen wir nur die bestbewertete - kein Auffuellen mit mittelmaessigen Treffern.
        # Toleranz: +-1.5 wenn ein Subrang bekannt ist, sonst +-2.0.
        if rank_value > 0:
            tolerance = 1.5 if rank_sub is not None else 2.0
            requester_float = rank_value + (rank_sub or 5) / 10.0
            fitting = [
                lane
                for lane in ranked_candidates
                if lane.avg_rank_value > 0
                and abs(lane.avg_rank_value - requester_float) <= tolerance
            ]
            if fitting:
                return fitting[:1]

        return ranked_candidates[:MAX_JOIN_LOBBIES_SHOWN]

    def _build_lobby_field_value(
        self,
        guild: discord.Guild,
        lane: LaneInfo,
        warning_line: str = "",
    ) -> str:
        lines: list[str] = []
        if lane.member_count == 0:
            lines.append("Noch leer — eröffne sie doch")
        else:
            lines.append(f"{lane.member_count} im Voice")
        lines.append(f"Ø-Rang: {lane.avg_rank_label}")

        if warning_line:
            lines.append(warning_line)

        if lane.co_player_ids_present:
            co_names = []
            for co_id in lane.co_player_ids_present[:3]:
                member = guild.get_member(co_id)
                if member:
                    co_names.append(member.display_name)
            if co_names:
                verb = "sind" if len(co_names) > 1 else "ist"
                lines.append(f"👥 {', '.join(co_names)} {verb} auch da")

        if lane.member_count >= LOBBY_MAYBE_FULL_THRESHOLD:
            lines.append(
                "⚠️ Könnte schon voll sein — schau kurz rein, sonst eigene Lobby aufmachen."
            )

        lines.append(f"\nHier klicken zum Beitreten 👉 <#{lane.channel.id}>")
        return "\n".join(lines)

    def _build_staging_suggestions(
        self,
        guild: discord.Guild,
        lanes: list[LaneInfo],
        preferred_label: str,
    ) -> list[tuple[str, str]]:
        """Gibt bis zu 2 Staging-Channels als (name, value) Tuple zurück."""
        staging = [lane for lane in lanes if lane.is_staging]
        preferred = [s for s in staging if s.label == preferred_label]
        others = [s for s in staging if s.label != preferred_label]
        ordered = preferred + others

        result = []
        for lane in ordered[:2]:
            result.append(
                (
                    f"➕ {lane.channel.name}",
                    f"{lane.label} · Einfach joinen und loslegen — sobald du drin bist, sehen andere dass hier was geht.\n<#{lane.channel.id}>",
                )
            )

        if not result:
            empty = [lane for lane in lanes if lane.member_count == 0 and not lane.is_staging]
            for lane in empty[:1]:
                result.append(
                    (
                        f"➕ {lane.channel.name}",
                        f"{lane.label} · Einfach joinen und loslegen — sobald du drin bist, sehen andere dass hier was geht.\n<#{lane.channel.id}>",
                    )
                )

        return result

    def _resolve_staging_channel(
        self,
        guild: discord.Guild,
        preferred_label: str,
        lanes: list[LaneInfo],
    ) -> int | None:
        channel_id: int | None = None

        if preferred_label == "Ranked":
            channel_id = STAGING_RANKED_ID
        elif preferred_label == "Street Brawl":
            channel_id = STAGING_STREET_BRAWL_ID
            sb_cat = guild.get_channel(STREET_BRAWL_CATEGORY_ID)
            if isinstance(sb_cat, discord.CategoryChannel):
                for vc in sb_cat.voice_channels:
                    if vc.id == STAGING_STREET_BRAWL_ID or self._is_offtopic(vc.name):
                        continue
                    member_count = len([m for m in vc.members if not m.bot])
                    if member_count < LOBBY_MAYBE_FULL_THRESHOLD:
                        channel_id = vc.id
                        break
        elif preferred_label == "New Player":
            channel_id = NEW_PLAYER_LANE_ID
            np_cat = guild.get_channel(NEW_PLAYER_CATEGORY_ID)
            if isinstance(np_cat, discord.CategoryChannel):
                for vc in np_cat.voice_channels:
                    if self._is_offtopic(vc.name):
                        continue
                    member_count = len([m for m in vc.members if not m.bot])
                    if member_count < LOBBY_MAYBE_FULL_THRESHOLD:
                        channel_id = vc.id
                        break
        else:
            channel_id = STAGING_CASUAL_ID

        if channel_id and guild.get_channel(channel_id):
            return channel_id

        if preferred_label == "New Player":
            for lane in lanes:
                if lane.label == "New Player" and guild.get_channel(lane.channel.id):
                    return lane.channel.id

        return None

    def _parse_subrank_token(self, raw_token: str | None) -> int | None:
        if not raw_token:
            return None
        token = raw_token.strip().rstrip("+").casefold()
        if token.isdigit():
            value = int(token)
            return value if 1 <= value <= 6 else None
        roman_map = {
            "i": 1,
            "ii": 2,
            "iii": 3,
            "iv": 4,
            "v": 5,
            "vi": 6,
        }
        return roman_map.get(token)

    def _detect_new_player_text(self, content_lower: str) -> bool:
        normalized = re.sub(r"\s+", " ", content_lower)
        phrases = (
            "neuling",
            "neuer spieler",
            "bin neu",
            "neu im spiel",
            "anfänger",
            "anfanger",
            "noch nicht so gut",
            "mit einem neuling",
            "mit nem neuling",
            "mit 'nem neuling",
        )
        return any(phrase in normalized for phrase in phrases)

    def _is_new_player_request(
        self,
        content_lower: str,
        rank_value: int,
        has_rank_role: bool,
    ) -> bool:
        if 0 < rank_value <= NEW_PLAYER_MAX_RANK:
            return True
        if rank_value > NEW_PLAYER_MAX_RANK or has_rank_role:
            return False
        return self._detect_new_player_text(content_lower)

    def _parse_rank_from_message(self, content: str) -> tuple[str, int, int | None]:
        """Versucht Rang aus Nachrichtentext zu extrahieren, z.B. 'Oracle 3' oder 'Emissary'."""
        content_lower = content.lower()
        best_rank_name = ""
        best_rank_val = 0
        best_sub = None

        rank_tokens = {
            **{name.lower(): name for name, value in DISCORD_RANK_ROLES.values() if value > 0},
            **MESSAGE_RANK_ALIASES,
        }

        for token, full_name in rank_tokens.items():
            rank_val = RANK_NAME_TO_VALUE.get(full_name.lower(), 0)
            if rank_val == 0:
                continue

            pattern = re.compile(
                rf"\b{re.escape(token)}\b(?:\s*[-~]?\s*(?:([1-6])\+?|\b(vi|iv|v|iii|ii|i)\b))?",
                re.IGNORECASE,
            )
            match = pattern.search(content_lower)
            if not match:
                continue

            parsed_sub = self._parse_subrank_token(match.group(1) or match.group(2))
            if rank_val > best_rank_val or (rank_val == best_rank_val and parsed_sub is not None):
                best_rank_name = full_name
                best_rank_val = rank_val
                best_sub = parsed_sub

        return best_rank_name, best_rank_val, best_sub

    async def _handle_lfg_request(self, message: discord.Message):
        """
        Verarbeitet LFG-Anfrage lokal: Lane Routing + Embed + Decision Log.
        Kein AI-Call — alles per Bot-Logik.
        """
        guild = message.guild
        if not guild:
            return

        output_channel = guild.get_channel(OUTPUT_CHANNEL_ID)
        if not output_channel or not isinstance(output_channel, discord.abc.Messageable):
            log.warning(
                "Output-Channel %s nicht gefunden. Fallback auf LFG-Channel.",
                OUTPUT_CHANNEL_ID,
            )
            output_channel = message.channel

        # 1. Rank ermitteln
        rank_name, rank_val, rank_sub = self._get_user_rank_info(message.author)
        has_rank_role = rank_val > 0
        has_explicit_rank = has_rank_role
        sub_str = f" {rank_sub}" if rank_sub else ""
        rank_display = f"{rank_name}{sub_str}" if rank_val > 0 else "Unbekannt"

        # Rang aus Nachricht extrahieren wenn keine Rolle
        if rank_val == 0:
            msg_rank_name, msg_rank_val, msg_rank_sub = self._parse_rank_from_message(
                message.content
            )
            if msg_rank_val > 0:
                rank_name = msg_rank_name
                rank_val = msg_rank_val
                rank_sub = msg_rank_sub
                has_explicit_rank = True
                sub_str = f" {rank_sub}" if rank_sub else ""
                rank_display = f"{rank_name}{sub_str}"

        # 2. Co-Player Stats laden
        co_player_stats = await self._fetch_co_player_stats(message.author.id)
        co_player_ids = {
            co_id
            for co_id, (sessions, _) in co_player_stats.items()
            if sessions >= COPLAYER_IN_LANE_SESSIONS_THRESHOLD
        }

        content_lower = (message.content or "").lower()
        is_new_player = self._is_new_player_request(content_lower, rank_val, has_rank_role)

        routing_rank_name = rank_name
        routing_rank_val = rank_val
        routing_rank_sub = rank_sub
        suggestion_rank_strict = has_explicit_rank
        if is_new_player and not has_rank_role and rank_val == 0:
            routing_rank_name = NEW_PLAYER_FALLBACK_RANK_NAME
            routing_rank_val = NEW_PLAYER_FALLBACK_RANK_VALUE
            routing_rank_sub = NEW_PLAYER_FALLBACK_SUBRANK
            suggestion_rank_strict = True

        steam_links, online_users, steam_online_ids = await self._load_deadlock_presence()

        # 3. Lane Routing
        lanes = self._scan_all_lanes(guild, co_player_ids, steam_online_ids=steam_online_ids)
        routing = self._route_to_lane(
            guild,
            message.author,
            content_lower,
            routing_rank_val,
            routing_rank_sub,
            routing_rank_name,
            lanes,
            co_player_ids,
        )
        routing.decision_log.insert(0, "Mode: lobby")

        suggested_lanes = self._select_lobby_suggestions(
            lanes,
            routing,
            routing_rank_val,
            routing_rank_sub,
            is_new_player,
            suggestion_rank_strict,
        )
        has_active = len(suggested_lanes) > 0
        preferred_label = self._resolve_mode_label(
            routing, is_new_player, has_active, suggestion_rank_strict, routing_rank_val
        )

        new_player_lane_occupied = any(
            lane.label == "New Player" and lane.member_count > 0 for lane in lanes
        )
        embed = discord.Embed(
            title="\U0001f3ae Lobby-Finder",
            description=self._compose_intro_text(
                message.author.mention,
                rank_display,
                is_new_player,
                has_active,
                new_player_lane_occupied=new_player_lane_occupied,
                lobby_count=len(suggested_lanes),
            ),
            color=discord.Color.orange(),
        )
        visible_tag_line = await self._get_visible_user_tag_line(message.author.id)
        if visible_tag_line:
            embed.description = f"{embed.description}\n\n{visible_tag_line}"

        requester_rank_float = rank_val + (rank_sub or 5) / 10.0
        if has_active:
            shown_lanes = suggested_lanes[:MAX_JOIN_LOBBIES_SHOWN]
            for idx, lane in enumerate(shown_lanes):
                slots = lane.slots_free
                if slots <= 2:
                    status = "\U0001f7e1"
                else:
                    status = "\U0001f7e2"

                warning_line = ""
                if lane.member_count > 0 and rank_val > 0:
                    # Vorzeichenbehaftet: Warnung nur bei echtem Over-Rank,
                    # nicht wenn die Lobby unter dem eigenen Rang liegt.
                    rank_diff = lane.avg_rank_value - requester_rank_float
                    if rank_diff > RANK_WARNING_DIFF:
                        warning_line = "⚠️ etwas über deinem Rang"
                embed.add_field(
                    name=f"{status} {lane.channel.name}",
                    value=self._build_lobby_field_value(guild, lane, warning_line),
                    inline=False,
                )
                if idx < len(shown_lanes) - 1:
                    embed.add_field(name="​", value="​", inline=False)

            staging_id = self._resolve_staging_channel(guild, preferred_label, lanes)
            if staging_id:
                embed.add_field(
                    name="Oder eigene Lobby aufmachen?",
                    value=(
                        f"Wenn nichts passt, mach in <#{staging_id}> eine "
                        f"**{preferred_label}**-Lane auf — erfahrungsgemäß kommen schnell Leute dazu."
                    ),
                    inline=False,
                )
        else:
            staging_id = self._resolve_staging_channel(guild, preferred_label, lanes)
            if staging_id:
                embed.add_field(
                    name="Oder eigene Lobby aufmachen?",
                    value=(
                        f"Wenn nichts passt, mach in <#{staging_id}> eine "
                        f"**{preferred_label}**-Lane auf — erfahrungsgemäß kommen schnell Leute dazu."
                    ),
                    inline=False,
                )
            else:
                staging_suggestions = self._build_staging_suggestions(
                    guild,
                    lanes,
                    preferred_label,
                )
                for name, value in staging_suggestions:
                    embed.add_field(name=name, value=value, inline=False)

        await output_channel.send(
            embed=embed,
            allowed_mentions=discord.AllowedMentions(
                users=True,
                roles=False,
                everyone=False,
                replied_user=False,
            ),
        )

        # 9. Decision Log Embed (separater Log-Channel, für Admins)
        log_channel = guild.get_channel(LOG_CHANNEL_ID)
        if log_channel and isinstance(log_channel, discord.abc.Messageable):
            log_embed = discord.Embed(
                title=f"LFG Decision Log — {message.author.display_name}",
                color=discord.Color.greyple(),
            )
            icon_map = {
                "Mode": "\U0001f9ed",
                "Intent": "\u2699\ufe0f",
                "Scan": "\U0001f4e1",
                "Rank-Filter": "\U0001f50e",
                "Co-Player": "\U0001f465",
                "Entscheidung": "\U0001f3af",
                "Dauer": "\u23f1\ufe0f",
            }
            log_text_lines = []
            for line in routing.decision_log:
                prefix_icon = ""
                for key, icon in icon_map.items():
                    if line.startswith(key):
                        prefix_icon = icon + " "
                        break
                log_text_lines.append(f"{prefix_icon}{line}")

            log_embed.description = "\n".join(log_text_lines)
            try:
                await log_channel.send(embed=log_embed)
            except Exception as exc:
                log.warning("Decision Log senden fehlgeschlagen: %s", exc)

    @commands.Cog.listener()
    async def on_message(self, message: discord.Message) -> None:
        if message.author.bot:
            return

        # Nur im LFG-Channel lauschen
        if message.channel.id != LFG_CHANNEL_ID:
            return

        # Lobby-Suche: User darf nicht bereits in einer Lane sitzen — das übernimmt player_finder.py
        if message.author.voice and message.author.voice.channel:
            return

        # Keyword-only Intent Check (kein AI)
        if not self._keyword_lfg_intent(message.content):
            return

        # Cooldown Check
        now = time.time()
        if now - self.lfg_cooldowns.get(message.author.id, 0) < self.cooldown_seconds:
            return

        self.lfg_cooldowns[message.author.id] = now
        await self._handle_lfg_request(message)

    # --- Debug Commands (Phase 6) ---

    @commands.command(name="lfgtest")
    @commands.has_permissions(administrator=True)
    async def lfg_debug(self, ctx):
        """Zeigt Lane-Übersicht mit Plätzen und Rank-Info."""
        guild = ctx.guild
        if not guild:
            return

        _steam_links, _online_users, steam_online_ids = await self._load_deadlock_presence()
        lanes = self._scan_all_lanes(guild, set(), steam_online_ids=steam_online_ids)
        lines = []
        for lane in lanes:
            occ = f"{lane.member_count}/{lane.user_limit}"
            status = "Platz frei" if lane.has_space else "VOLL"
            lines.append(
                f"- **{lane.label}**: {lane.channel.name} ({occ}, {lane.avg_rank_label}, {status})"
            )

        if not lines:
            await ctx.send("Keine Lanes gefunden.")
            return

        await ctx.send(f"**Lane Übersicht ({len(lanes)} Lanes):**\n" + "\n".join(lines))

    @commands.command(name="lfgroute")
    @commands.has_permissions(administrator=True)
    async def lfg_route_debug(self, ctx, member: discord.Member | None = None):
        """Simuliert LFG-Routing für einen User. Admin-only."""
        target = member or ctx.author
        guild = ctx.guild
        if not guild:
            return

        rank_name, rank_val, rank_sub = self._get_user_rank_info(target)
        sub_str = f" {rank_sub}" if rank_sub else ""
        rank_display = f"{rank_name}{sub_str}" if rank_val > 0 else "Unbekannt"

        co_stats = await self._fetch_co_player_stats(target.id)
        co_ids = {
            co_id
            for co_id, (sessions, _) in co_stats.items()
            if sessions >= COPLAYER_IN_LANE_SESSIONS_THRESHOLD
        }

        now = datetime.utcnow()
        profile = await self._build_user_activity_profile(target.id, now)
        _steam_links, _online_users, steam_online_ids = await self._load_deadlock_presence()
        lanes = self._scan_all_lanes(guild, co_ids, steam_online_ids=steam_online_ids)
        routing = self._route_to_lane(
            guild,
            target,
            "",
            rank_val,
            rank_sub,
            rank_name,
            lanes,
            co_ids,
        )

        co_coming = await self._find_co_players_likely_coming_online(guild, profile, now)

        embed = discord.Embed(
            title=f"LFG Route Debug — {target.display_name} ({rank_display})",
            color=discord.Color.orange(),
        )

        # Decision Log
        embed.add_field(
            name="Decision Log",
            value="\n".join(routing.decision_log) or "Keine Schritte",
            inline=False,
        )

        # Profil
        hours_str = (
            ", ".join(str(h) for h in profile.typical_hours[:6]) if profile.typical_hours else "-"
        )
        days_str = ", ".join(str(d) for d in profile.typical_days) if profile.typical_days else "-"
        embed.add_field(
            name="Activity Profile",
            value=(
                f"Sessions (2W): {profile.sessions_count_2w}\n"
                f"Score: {profile.activity_score}\n"
                f"Typ. Stunden: {hours_str}\n"
                f"Typ. Tage: {days_str}\n"
                f"Aktiv jetzt: {'Ja' if profile.is_likely_active_now else 'Nein'}\n"
                f"Aktiv bald: {'Ja' if profile.is_likely_active_soon else 'Nein'}"
            ),
            inline=True,
        )

        # Co-Players
        if profile.top_co_players:
            cp_lines = []
            for co_id, sessions, minutes in profile.top_co_players[:5]:
                m = guild.get_member(co_id)
                name = m.display_name if m else str(co_id)
                cp_lines.append(f"{name}: {sessions}s, {minutes}min")
            embed.add_field(
                name="Top Co-Players",
                value="\n".join(cp_lines),
                inline=True,
            )

        # Bald online
        if co_coming:
            embed.add_field(
                name="Bald online",
                value=", ".join(n for _, n in co_coming[:5]),
                inline=False,
            )

        await ctx.send(embed=embed)


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(SmartLFGAgent(bot))
