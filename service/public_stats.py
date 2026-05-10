"""Public activity statistics server (port 8768).

Shows aggregated, anonymous statistics about when ranks are active,
which lanes they prefer, and heuristics for rank estimation.

Public leaderboards expose Discord display names; personal dashboard data
is only available via Discord OAuth session cookie (delegated via central dashboard).
"""

from __future__ import annotations

import asyncio
import base64
import hashlib
import hmac
import json
import logging
import os
import sqlite3
import time
from collections.abc import Iterable
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from aiohttp import ClientSession, ClientTimeout, web

from service import db

log = logging.getLogger(__name__)

PUBLIC_STATS_PORT = int(os.getenv("PUBLIC_STATS_PORT", "8768"))
PUBLIC_STATS_HOST = os.getenv("PUBLIC_STATS_HOST", "127.0.0.1")
PUBLIC_STATS_SESSION_COOKIE = "dl_session"
PUBLIC_STATS_PRE_AUTH_COOKIE = "dl_pre_auth"
PUBLIC_STATS_SESSION_TTL = 14 * 24 * 60 * 60
PUBLIC_STATS_PRE_AUTH_TTL = 10 * 60
PUBLIC_STATS_DEFAULT_REDIRECT = "/aktivitaet/"
PUBLIC_STATS_DEFAULT_CORS_ORIGINS = (
    "http://localhost:5173",
    "http://localhost:5174",
    "http://localhost:5175",
    "http://127.0.0.1:5173",
    "http://127.0.0.1:5174",
    "http://127.0.0.1:5175",
)
# Callback-URL nach Discord-OAuth (muss auf deutsche-deadlock-community.de zeigen)
PUBLIC_STATS_CALLBACK_URL = os.getenv(
    "PUBLIC_STATS_CALLBACK_URL",
    "https://deutsche-deadlock-community.de/aktivitaet/auth/discord/complete",
)
DASHBOARD_INTERNAL_API_BASE = os.getenv("DASHBOARD_INTERNAL_API_BASE", "http://127.0.0.1:8766")
_DASHBOARD_INTERNAL_TOKEN_ENV_NAMES = (
    "TURNIER_INTERNAL_API_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
    "TWITCH_INTERNAL_API_TOKEN",
)
# ENV:
# - PUBLIC_STATS_SESSION_SECRET: für dl_session-Cookie-Signierung
#   (fällt auf SESSIONS_ENCRYPTION_KEY zurück)
# - PUBLIC_STATS_CALLBACK_URL: öffentliche URL für /aktivitaet/auth/complete
# - DASHBOARD_INTERNAL_API_BASE: interne URL des Dashboards (Standard: http://127.0.0.1:8766)
# - TURNIER_INTERNAL_API_TOKEN / MAIN_BOT_INTERNAL_TOKEN: X-Internal-Token fürs Dashboard
# - PUBLIC_STATS_INSECURE_COOKIE=1: Secure-Flag für lokale HTTP-Tests deaktivieren
# - PUBLIC_STATS_COOKIE_SECURE=0|1: expliziter Secure-Override
# - PUBLIC_STATS_CORS_ORIGINS: CSV-Allowlist für /api/public/* und /auth/*

_HTML_PATH = Path(__file__).resolve().parent / "static" / "activity_stats.html"

RANK_ORDER = [
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

RANK_SHORT = {
    "initiate": "Init",
    "seeker": "Seek",
    "alchemist": "Alc",
    "arcanist": "Arc",
    "ritualist": "Rit",
    "emissary": "Emi",
    "archon": "Arch",
    "oracle": "Ora",
    "phantom": "Pha",
    "ascendant": "Asc",
    "eternus": "Ete",
}

RANK_COLORS: dict[str, str] = {
    "initiate": "#8fa4b4",
    "seeker": "#72aa5a",
    "alchemist": "#3dbb44",
    "arcanist": "#18bba8",
    "ritualist": "#2288ee",
    "emissary": "#5055ee",
    "archon": "#8833dd",
    "oracle": "#cc33bb",
    "phantom": "#dd3344",
    "ascendant": "#ee9922",
    "eternus": "#f5cc11",
}


def _load_html() -> str:
    try:
        return _HTML_PATH.read_text(encoding="utf-8")
    except Exception as exc:
        log.error("activity_stats.html nicht lesbar: %s", exc)
        raise


def _estimate_rank_from_co_players(user_id: int, co_players: list[int]) -> str | None:
    """Estimate rank for a player with no rank based on their co-players' ranks."""
    if len(co_players) < 3:
        return None

    rank_scores = []
    for cp_id in co_players:
        row = db.query_one(
            """
            SELECT deadlock_rank_name, deadlock_subrank
            FROM steam_links
            WHERE user_id = ? AND verified = 1
            ORDER BY primary_account DESC, deadlock_rank_updated_at DESC
            LIMIT 1
            """,
            (cp_id,),
        )
        if row and row["deadlock_rank_name"]:
            score = _rank_to_score(row["deadlock_rank_name"], row["deadlock_subrank"] or 3)
            rank_scores.append(score)

    if not rank_scores:
        return None

    avg_score = sum(rank_scores) / len(rank_scores)
    return _score_to_bucket(avg_score)


def _rank_to_score(rank_name: str, subrank: int) -> float:
    try:
        base = RANK_ORDER.index(rank_name.lower()) + 1
    except ValueError:
        base = 0
    return base * 6 + max(1, min(6, subrank))


def _score_to_bucket(score: float) -> str:
    if score <= 18:
        return "low"
    elif score <= 42:
        return "mid"
    return "high"


def _detect_lane_from_name(name: str | None) -> str | None:
    if not name:
        return None
    n = name.lower()
    if "mid" in n:
        return "mid"
    if "off" in n or "offlane" in n or "off-lane" in n:
        return "off"
    if "safe" in n or "carry" in n:
        return "safe"
    if "jungle" in n or "jg" in n:
        return "jungle"
    if "new" in n or "neue" in n or "🆕" in n:
        return "new_player"
    return None


def _env_flag(name: str, default: bool = False) -> bool:
    raw = os.getenv(name)
    if raw is None:
        return default
    return raw.strip().lower() in {"1", "true", "yes", "on"}


def _cookie_secure() -> bool:
    if _env_flag("PUBLIC_STATS_INSECURE_COOKIE", False):
        return False
    return _env_flag("PUBLIC_STATS_COOKIE_SECURE", True)


def _session_secret() -> str:
    return (
        os.getenv("PUBLIC_STATS_SESSION_SECRET", "").strip()
        or os.getenv("SESSIONS_ENCRYPTION_KEY", "").strip()
    )


def _dashboard_internal_token() -> str:
    for name in _DASHBOARD_INTERNAL_TOKEN_ENV_NAMES:
        val = os.getenv(name, "").strip()
        if val:
            return val
    return ""


async def _call_dashboard_api(path: str, payload: dict[str, Any]) -> dict[str, Any] | None:
    token = _dashboard_internal_token()
    if not token:
        log.warning("Kein Dashboard-Internal-Token konfiguriert (public_stats).")
        return None
    timeout = ClientTimeout(total=20)
    url = f"{DASHBOARD_INTERNAL_API_BASE.rstrip('/')}{path}"
    headers = {"X-Internal-Token": token, "Content-Type": "application/json"}
    try:
        async with ClientSession(timeout=timeout) as client:
            async with client.post(url, json=payload, headers=headers) as response:
                if response.status != 200:
                    body = await response.text()
                    log.warning(
                        "Dashboard-API Fehler (path=%s status=%s body=%s)",
                        path,
                        response.status,
                        body[:200],
                    )
                    return None
                data = await response.json()
        return data if isinstance(data, dict) else None
    except Exception:
        log.exception("Dashboard-API Aufruf fehlgeschlagen (path=%s)", path)
        return None


def _b64_encode(value: str) -> str:
    return base64.urlsafe_b64encode(value.encode("utf-8")).decode("ascii").rstrip("=")


def _b64_decode(value: str) -> str:
    padding = "=" * (-len(value) % 4)
    return base64.urlsafe_b64decode(f"{value}{padding}".encode("ascii")).decode("utf-8")


def _sign_hmac(value: str) -> str:
    secret = _session_secret()
    if not secret:
        return ""
    return hmac.new(secret.encode("utf-8"), value.encode("utf-8"), hashlib.sha256).hexdigest()


def _sign(payload: dict[str, Any]) -> str:
    body = json.dumps(payload, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    encoded = _b64_encode(body)
    return f"{encoded}.{_sign_hmac(encoded)}"


def _verify(cookie: str) -> dict[str, Any] | None:
    if not cookie or "." not in cookie or not _session_secret():
        return None
    encoded, signature = cookie.rsplit(".", 1)
    expected = _sign_hmac(encoded)
    if not expected or not hmac.compare_digest(signature, expected):
        return None
    try:
        raw = _b64_decode(encoded)
        payload = json.loads(raw)
    except Exception:
        return None
    if not isinstance(payload, dict):
        return None
    try:
        exp = int(payload.get("exp", 0))
    except (TypeError, ValueError):
        return None
    if exp <= int(time.time()):
        return None
    return payload


def _require_session(request: web.Request) -> dict[str, Any]:
    cached = request.get("dl_session")
    if isinstance(cached, dict):
        return cached
    cookie = request.cookies.get(PUBLIC_STATS_SESSION_COOKIE)
    payload = _verify(cookie or "")
    if not payload:
        raise web.HTTPUnauthorized(
            text='{"error":"unauthenticated"}',
            content_type="application/json",
        )
    request["dl_session"] = payload
    return payload


def _sanitize_redirect_path(value: str | None) -> str:
    raw = (value or "").strip()
    if not raw:
        return PUBLIC_STATS_DEFAULT_REDIRECT
    parsed = urlsplit(raw)
    if parsed.scheme or parsed.netloc:
        return PUBLIC_STATS_DEFAULT_REDIRECT
    if not parsed.path.startswith("/") or parsed.path.startswith("//"):
        return PUBLIC_STATS_DEFAULT_REDIRECT
    if any(ch in raw for ch in ("\r", "\n", "\x00")):
        return PUBLIC_STATS_DEFAULT_REDIRECT
    return raw


def _parse_dt(value: Any) -> datetime | None:
    if value is None:
        return None
    if isinstance(value, datetime):
        return value
    if isinstance(value, (int, float)):
        try:
            return datetime.fromtimestamp(float(value))
        except Exception:
            return None
    try:
        text = str(value).strip()
        if not text:
            return None
        return datetime.fromisoformat(text.replace("Z", "+00:00"))
    except Exception:
        return None


def _to_iso(value: Any) -> str | None:
    dt = _parse_dt(value)
    if dt is None:
        if value in (None, ""):
            return None
        return str(value)
    return dt.isoformat()


def _safe_int(value: Any, default: int = 0) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return default


def _safe_query_all(
    sql: str,
    params: Iterable[Any] = (),
    *,
    optional_tables: Iterable[str] = (),
) -> list[Any]:
    try:
        return db.query_all(sql, params)
    except sqlite3.OperationalError as exc:
        message = str(exc).lower()
        if "no such table" in message and any(
            table.lower() in message for table in optional_tables
        ):
            log.warning("Optionale Tabelle fehlt für PublicStats: %s", exc)
            return []
        raise


def _safe_query_one(
    sql: str,
    params: Iterable[Any] = (),
    *,
    optional_tables: Iterable[str] = (),
) -> Any | None:
    try:
        return db.query_one(sql, params)
    except sqlite3.OperationalError as exc:
        message = str(exc).lower()
        if "no such table" in message and any(
            table.lower() in message for table in optional_tables
        ):
            log.warning("Optionale Tabelle fehlt für PublicStats: %s", exc)
            return None
        raise


def _resolve_display_names(user_ids: Iterable[int]) -> dict[int, str]:
    names: dict[int, str] = {}
    unique_ids = sorted({int(uid) for uid in user_ids if uid})
    for uid in unique_ids:
        row = _safe_query_one(
            """
            SELECT display_name
            FROM member_events
            WHERE user_id = ?
              AND display_name IS NOT NULL
              AND TRIM(display_name) != ''
            ORDER BY datetime(timestamp) DESC, id DESC
            LIMIT 1
            """,
            (uid,),
            optional_tables=("member_events",),
        )
        name = (row["display_name"] if row else None) or f"User {uid}"
        names[uid] = str(name)
    return names


def _discord_avatar_url(user_id: str | int | None, avatar_hash: str | None) -> str | None:
    if not user_id or not avatar_hash:
        return None
    ext = "gif" if str(avatar_hash).startswith("a_") else "png"
    return f"https://cdn.discordapp.com/avatars/{user_id}/{avatar_hash}.{ext}?size=256"


def _public_user_name(name_map: dict[int, str], user_id: int, fallback: str | None = None) -> str:
    name = name_map.get(user_id) or fallback
    return str(name or f"User {user_id}")


def _get_updated_at(table_name: str) -> str:
    row = _safe_query_one(
        f"SELECT MAX(last_update) AS updated_at FROM {table_name}",
        optional_tables=(table_name,),
    )
    return _to_iso(row["updated_at"] if row else None) or datetime.now().isoformat()


def _normalize_mode(mode_raw: str | None, *, default: str = "day") -> str:
    mode = (mode_raw or default).strip().lower()
    if mode not in {"hour", "day", "week", "month"}:
        raise web.HTTPBadRequest(text='{"error":"invalid_mode"}', content_type="application/json")
    return mode


def _parse_positive_int(
    raw_value: str | None,
    *,
    default: int,
    minimum: int,
    maximum: int,
    field_name: str,
) -> int:
    try:
        value = int(raw_value) if raw_value is not None else default
    except ValueError as exc:
        raise web.HTTPBadRequest(
            text=json.dumps({"error": f"invalid_{field_name}"}, separators=(",", ":")),
            content_type="application/json",
        ) from exc
    if value < minimum or value > maximum:
        raise web.HTTPBadRequest(
            text=json.dumps({"error": f"invalid_{field_name}"}, separators=(",", ":")),
            content_type="application/json",
        )
    return value


def _parse_user_id_from_session(session: dict[str, Any]) -> int:
    try:
        user_id = int(session.get("user_id"))
    except (TypeError, ValueError) as exc:
        raise web.HTTPUnauthorized(
            text='{"error":"unauthenticated"}',
            content_type="application/json",
        ) from exc
    if user_id <= 0:
        raise web.HTTPUnauthorized(
            text='{"error":"unauthenticated"}', content_type="application/json"
        )
    return user_id


def _rank_for_points(table_name: str, user_id: int) -> int | None:
    row = _safe_query_one(
        f"SELECT total_points FROM {table_name} WHERE user_id = ?",
        (user_id,),
        optional_tables=(table_name,),
    )
    if not row:
        return None
    better = _safe_query_one(
        f"SELECT COUNT(*) AS cnt FROM {table_name} WHERE total_points > ?",
        (_safe_int(row["total_points"]),),
        optional_tables=(table_name,),
    )
    return _safe_int(better["cnt"] if better else 0) + 1


def _build_voice_matrix(rows: Iterable[Any]) -> tuple[list[list[int]], int]:
    matrix = [[0 for _ in range(24)] for _ in range(7)]
    total_seconds = 0
    for row in rows:
        started = _parse_dt(row["started_at"])
        ended = _parse_dt(row["ended_at"])
        if started is None:
            continue
        if ended is None:
            duration_seconds = _safe_int(row["duration_seconds"])
            if duration_seconds <= 0:
                continue
            ended = started + timedelta(seconds=duration_seconds)
        if ended <= started:
            continue
        cursor = started
        while cursor < ended:
            next_hour = cursor.replace(minute=0, second=0, microsecond=0) + timedelta(hours=1)
            segment_end = min(next_hour, ended)
            seconds = int((segment_end - cursor).total_seconds())
            if seconds > 0:
                matrix[cursor.weekday() % 7][cursor.hour] += seconds
                total_seconds += seconds
            cursor = segment_end
    return matrix, total_seconds


def _co_participant_count(raw_value: Any, user_id: int | None = None) -> int:
    if not raw_value:
        return 0
    try:
        decoded = json.loads(raw_value)
    except Exception:
        return 0
    if not isinstance(decoded, list):
        return 0
    seen: set[int] = set()
    for entry in decoded:
        try:
            candidate = int(entry)
        except (TypeError, ValueError):
            continue
        if candidate <= 0 or (user_id is not None and candidate == user_id):
            continue
        seen.add(candidate)
    return len(seen)


# ── API Handlers ────────────────────────────────────────────────────────────────


async def handle_index(request: web.Request) -> web.Response:
    try:
        html = _load_html()
    except Exception:
        return web.Response(text="Seite nicht verfügbar.", status=503)
    return web.Response(text=html, content_type="text/html")


async def handle_activity_heatmap(request: web.Request) -> web.Response:
    """Returns 7x24 matrix of avg active users by rank and hour/day."""
    now = datetime.now()
    cutoff = now - timedelta(days=14)

    # Get sessions from last 2 weeks
    rows = db.query_all(
        """
        SELECT started_at, duration_seconds, channel_name, co_player_ids, user_id
        FROM voice_session_log
        WHERE started_at >= ?
        ORDER BY started_at
        """,
        (cutoff.isoformat(),),
    )

    # Build heatmap data: day (0=Mon..6=Sun) x hour (0..23) x rank
    heatmap: dict[str, dict[int, dict[int, list[int]]]] = {}
    for rank in RANK_ORDER:
        heatmap[rank] = {d: {h: [] for h in range(24)} for d in range(7)}

    # Users without rank get assigned via co-player heuristic
    no_rank_users: dict[int, str] = {}  # user_id -> estimated bucket

    for row in rows:
        try:
            started = datetime.fromisoformat(row["started_at"])
        except Exception:
            continue

        day = (started.weekday()) % 7
        hour = started.hour
        user_id = row["user_id"]
        co_players_raw = row["co_player_ids"] or "[]"
        _channel_name = row["channel_name"]

        # Determine rank for this user
        user_rank = _get_user_rank(user_id)
        if not user_rank:
            # Try heuristic via co-players
            if user_id in no_rank_users:
                user_rank = no_rank_users[user_id]
            else:
                try:
                    co_ids = json.loads(co_players_raw)
                    if isinstance(co_ids, list):
                        estimated = _estimate_rank_from_co_players(user_id, co_ids)
                        user_rank = estimated
                        if estimated:
                            no_rank_users[user_id] = estimated
                except Exception:
                    user_rank = None

        if not user_rank:
            continue

        # Add to heatmap buckets
        if user_rank in heatmap:
            heatmap[user_rank][day][hour].append(row["user_id"])

    # Convert to avg per hour/day
    result = {}
    for rank in RANK_ORDER:
        result[rank] = []
        for day in range(7):
            for hour in range(24):
                count = len(heatmap[rank][day][hour])
                result[rank].append({"day": day, "hour": hour, "count": count})

    return web.json_response(
        {"heatmap": result, "rank_order": RANK_ORDER, "generated_at": now.isoformat()}
    )


async def handle_rank_distribution(request: web.Request) -> web.Response:
    """Returns current rank distribution and trend over time."""
    now = datetime.now()
    _cutoff_30d = now - timedelta(days=30)
    weeks_count = _parse_positive_int(
        request.query.get("weeks"),
        default=4,
        minimum=1,
        maximum=52,
        field_name="weeks",
    )

    # Current distribution from steam_links
    rank_counts: dict[str, int] = {r: 0 for r in RANK_ORDER}
    rows = db.query_all(
        """
        SELECT deadlock_rank_name, COUNT(*) as cnt
        FROM steam_links
        WHERE verified = 1 AND deadlock_rank_name IS NOT NULL
        GROUP BY deadlock_rank_name
        """
    )
    for row in rows:
        name = (row["deadlock_rank_name"] or "").lower()
        if name in rank_counts:
            rank_counts[name] = row["cnt"]

    # Activity by rank over last 30 days (weekly buckets)
    weekly = []
    for week in range(weeks_count):
        week_start = now - timedelta(weeks=week + 1)
        week_end = now - timedelta(weeks=week)
        week_data = {r: 0 for r in RANK_ORDER}
        week_rows = db.query_all(
            """
            SELECT user_id, started_at
            FROM voice_session_log
            WHERE started_at >= ? AND started_at < ?
            """,
            (week_start.isoformat(), week_end.isoformat()),
        )
        seen_users: set[int] = set()
        for row in week_rows:
            uid = row["user_id"]
            if uid in seen_users:
                continue
            rank = _get_user_rank(uid)
            if rank and rank in week_data:
                week_data[rank] += 1
                seen_users.add(uid)
        weekly.append({"week": week + 1, "data": week_data})

    weekly.reverse()  # oldest first

    return web.json_response(
        {
            "distribution": rank_counts,
            "rank_order": RANK_ORDER,
            "weekly_trend": weekly,
            "generated_at": now.isoformat(),
        }
    )


async def handle_lane_preferences(request: web.Request) -> web.Response:
    """Returns which lanes each rank prefers."""
    now = datetime.now()
    cutoff = now - timedelta(days=30)

    lane_rank_data: dict[str, dict[str, int]] = {}
    for rank in RANK_ORDER:
        lane_rank_data[rank] = {
            "mid": 0,
            "off": 0,
            "safe": 0,
            "jungle": 0,
            "new_player": 0,
            "unknown": 0,
        }

    rows = db.query_all(
        """
        SELECT user_id, channel_name, co_player_ids
        FROM voice_session_log
        WHERE started_at >= ?
        """,
        (cutoff.isoformat(),),
    )

    seen_users_per_lane: dict[str, set[int]] = {
        "mid": set(),
        "off": set(),
        "safe": set(),
        "jungle": set(),
        "new_player": set(),
        "unknown": set(),
    }

    for row in rows:
        lane = _detect_lane_from_name(row["channel_name"]) or "unknown"
        user_id = row["user_id"]

        # Only count each user once per lane category
        if user_id in seen_users_per_lane[lane]:
            continue

        rank = _get_user_rank(user_id)
        if not rank:
            # heuristic
            try:
                co_ids = json.loads(row["co_player_ids"] or "[]")
                rank = _estimate_rank_from_co_players(
                    user_id, co_ids if isinstance(co_ids, list) else []
                )
            except Exception:
                rank = None

        if rank and rank in lane_rank_data:
            lane_rank_data[rank][lane] += 1
            seen_users_per_lane[lane].add(user_id)

    return web.json_response(
        {
            "preferences": lane_rank_data,
            "rank_order": RANK_ORDER,
            "lanes": ["mid", "off", "safe", "jungle", "new_player", "unknown"],
            "generated_at": now.isoformat(),
        }
    )


async def handle_new_player_windows(request: web.Request) -> web.Response:
    """Returns time windows when new players are most active."""
    now = datetime.now()
    cutoff = now - timedelta(days=30)

    # New player lanes detection (channel names with "new", "neue", "🆕")
    hour_new_player_count: dict[int, list[int]] = {h: [] for h in range(24)}
    day_hour_counts: dict[int, dict[int, list[int]]] = {
        d: {h: [] for h in range(24)} for d in range(7)
    }

    rows = db.query_all(
        """
        SELECT started_at, channel_name, user_id
        FROM voice_session_log
        WHERE started_at >= ?
        """,
        (cutoff.isoformat(),),
    )

    for row in rows:
        try:
            started = datetime.fromisoformat(row["started_at"])
        except Exception:
            continue

        lane = _detect_lane_from_name(row["channel_name"])
        if lane == "new_player":
            day = started.weekday() % 7
            hour = started.hour
            day_hour_counts[day][hour].append(row["user_id"])
            hour_new_player_count[hour].append(row["user_id"])

    # Find peak hours and days
    peak_hours = sorted(hour_new_player_count.items(), key=lambda x: len(x[1]), reverse=True)[:5]
    peak_hours = [{"hour": h, "count": len(users)} for h, users in peak_hours]

    peak_days = []
    for day in range(7):
        total = sum(len(day_hour_counts[day][h]) for h in range(24))
        peak_days.append({"day": day, "count": total})
    peak_days.sort(key=lambda x: x["count"], reverse=True)

    return web.json_response(
        {
            "peak_hours": peak_hours,
            "peak_days": peak_days[:3],
            "day_hour_matrix": {
                str(d): {str(h): len(day_hour_counts[d][h]) for h in range(24)} for d in range(7)
            },
            "generated_at": now.isoformat(),
        }
    )


async def handle_timeline(request: web.Request) -> web.Response:
    """Returns streaming-style timeline of when ranks are most active.

    Query params: metric - 'players' (default) or 'hours'; days - 1..365
    """
    metric = request.query.get("metric", "players")
    days = _parse_positive_int(
        request.query.get("days"),
        default=7,
        minimum=1,
        maximum=365,
        field_name="days",
    )
    now = datetime.now()
    cutoff = now - timedelta(days=days)

    rows = db.query_all(
        """
        SELECT started_at, user_id, co_player_ids, channel_name, duration_seconds
        FROM voice_session_log
        WHERE started_at >= ?
        ORDER BY started_at
        """,
        (cutoff.isoformat(),),
    )

    # Build hourly aggregates
    hourly: dict[int, dict[str, int]] = {h: {r: 0 for r in RANK_ORDER} for h in range(24)}
    hourly_hours: dict[int, dict[str, float]] = {h: {r: 0.0 for r in RANK_ORDER} for h in range(24)}
    for row in rows:
        try:
            started = datetime.fromisoformat(row["started_at"])
        except Exception:
            continue

        hour = started.hour
        rank = _get_user_rank(row["user_id"])
        if not rank:
            try:
                co_ids = json.loads(row["co_player_ids"] or "[]")
                rank = _estimate_rank_from_co_players(
                    row["user_id"], co_ids if isinstance(co_ids, list) else []
                )
            except Exception:
                rank = None
        if rank and rank in hourly[hour]:
            hourly[hour][rank] += 1
            duration = row["duration_seconds"] or 0
            hourly_hours[hour][rank] += float(duration) / 3600  # convert to hours

    timeline = []
    for hour in range(24):
        entry = {"hour": hour, "ranks": {}}
        for rank in RANK_ORDER:
            if metric == "hours":
                entry["ranks"][rank] = round(hourly_hours[hour][rank], 2)
            else:
                entry["ranks"][rank] = hourly[hour][rank]
        timeline.append(entry)

    # Find peak times per rank
    peak_times = {}
    for rank in RANK_ORDER:
        if metric == "hours":
            peaks = sorted(
                enumerate(hourly_hours[h][rank] for h in range(24)),
                key=lambda x: x[1],
                reverse=True,
            )
        else:
            peaks = sorted(
                enumerate(hourly[h][rank] for h in range(24)), key=lambda x: x[1], reverse=True
            )
        peak_times[rank] = [
            {"hour": h, "count": round(c, 2) if metric == "hours" else c}
            for h, c in peaks[:3]
            if c > 0
        ]

    return web.json_response(
        {
            "timeline": timeline,
            "peak_times": peak_times,
            "rank_order": RANK_ORDER,
            "metric": metric,
            "generated_at": now.isoformat(),
        }
    )


async def handle_voice_history(request: web.Request) -> web.Response:
    """Returns aggregated voice activity stats - no user data exposed.

    Returns:
    - daily: daily totals for the last N days
    - hourly: hourly breakdown by mode (hour/day/week/month)
    - summary: total sessions, total time, unique users, avg session length
    """
    range_raw = request.query.get("range")
    mode_raw = request.query.get("mode") or "hour"
    try:
        days = int(range_raw) if range_raw else 14
        days = min(max(days, 1), 90)
    except ValueError:
        days = 14

    mode = mode_raw.strip().lower()
    if mode not in {"hour", "day", "week", "month"}:
        mode = "hour"

    cutoff = f"-{days} day"
    now = datetime.now()

    try:
        # Daily summary (last N days)
        daily_rows = db.query_all(
            """
            SELECT date(started_at) AS day,
                   SUM(duration_seconds) AS total_seconds,
                   COUNT(*) AS sessions,
                   COUNT(DISTINCT user_id) AS unique_users
            FROM voice_session_log
            WHERE started_at >= datetime('now', ?)
            GROUP BY date(started_at)
            ORDER BY day DESC
            """,
            (cutoff,),
        )

        # Overall summary
        summary_rows = db.query_all(
            """
            SELECT COUNT(*) AS total_sessions,
                   COALESCE(SUM(duration_seconds), 0) AS total_seconds,
                   COUNT(DISTINCT user_id) AS total_users
            FROM voice_session_log
            WHERE started_at >= datetime('now', ?)
            """,
            (cutoff,),
        )
        summary = (
            summary_rows[0]
            if summary_rows
            else {"total_sessions": 0, "total_seconds": 0, "total_users": 0}
        )

        # Hourly/weekly breakdown
        hourly_rows = db.query_all(
            """
            WITH grouped AS (
                SELECT
                    CASE
                        WHEN ? = 'hour' THEN strftime('%H', started_at)
                        WHEN ? = 'day' THEN strftime('%w', started_at)
                        WHEN ? = 'week' THEN strftime('%Y-%W', started_at)
                        ELSE strftime('%Y-%m', started_at)
                    END AS bucket,
                    duration_seconds,
                    COALESCE(peak_users, 0) AS peak_users
                FROM voice_session_log
                WHERE started_at >= datetime('now', ?)
            )
            SELECT bucket,
                   SUM(duration_seconds) AS total_seconds,
                   COUNT(*) AS sessions,
                   SUM(peak_users) AS sum_peak
            FROM grouped
            GROUP BY bucket
            ORDER BY bucket
            """,
            (mode, mode, mode, cutoff),
        )

    except Exception as exc:
        log.exception("Failed to load voice history: %s", exc)
        return web.json_response({"error": "Voice history unavailable"}, status=500)

    # Format hourly data
    hourly_data = []
    for row in hourly_rows:
        total_s = int(row["total_seconds"] or 0)
        sessions = int(row["sessions"] or 0)
        hourly_data.append(
            {
                "bucket": row["bucket"],
                "total_seconds": total_s,
                "sessions": sessions,
                "avg_session_minutes": round(total_s / sessions / 60, 1) if sessions > 0 else 0,
            }
        )

    # Format daily data
    daily_data = []
    for row in daily_rows:
        total_s = int(row["total_seconds"] or 0)
        sessions = int(row["sessions"] or 0)
        daily_data.append(
            {
                "day": row["day"],
                "total_seconds": total_s,
                "sessions": sessions,
                "unique_users": int(row["unique_users"] or 0),
            }
        )

    total_s = int(summary["total_seconds"] or 0)
    total_sessions = int(summary["total_sessions"] or 0)
    total_users = int(summary["total_users"] or 0)

    return web.json_response(
        {
            "summary": {
                "total_sessions": total_sessions,
                "total_seconds": total_s,
                "total_users": total_users,
                "avg_session_minutes": round(total_s / total_sessions / 60, 1)
                if total_sessions > 0
                else 0,
                "days": days,
            },
            "daily": daily_data,
            "hourly": hourly_data,
            "mode": mode,
            "generated_at": now.isoformat(),
        }
    )


async def _handle_public_voice_leaderboard(request: web.Request) -> web.Response:
    limit = _parse_positive_int(
        request.query.get("limit"),
        default=50,
        minimum=1,
        maximum=100,
        field_name="limit",
    )
    rows = db.query_all(
        """
        SELECT user_id, total_seconds, total_points
        FROM voice_stats
        ORDER BY total_points DESC, total_seconds DESC
        LIMIT ?
        """,
        (limit,),
    )
    user_ids = [_safe_int(row["user_id"]) for row in rows]
    name_map = _resolve_display_names(user_ids)
    entries = [
        {
            "rank": index,
            "user_id": str(_safe_int(row["user_id"])),
            "name": _public_user_name(name_map, _safe_int(row["user_id"])),
            "avatar_url": None,
            "total_seconds": _safe_int(row["total_seconds"]),
            "total_points": _safe_int(row["total_points"]),
            "hours": round(_safe_int(row["total_seconds"]) / 3600, 1),
        }
        for index, row in enumerate(rows, start=1)
    ]
    return web.json_response({"entries": entries, "updated_at": _get_updated_at("voice_stats")})


async def _handle_public_text_leaderboard(request: web.Request) -> web.Response:
    limit = _parse_positive_int(
        request.query.get("limit"),
        default=50,
        minimum=1,
        maximum=100,
        field_name="limit",
    )
    rows = _safe_query_all(
        """
        SELECT user_id, total_messages, total_points
        FROM text_stats
        ORDER BY total_points DESC, total_messages DESC
        LIMIT ?
        """,
        (limit,),
        optional_tables=("text_stats",),
    )
    user_ids = [_safe_int(row["user_id"]) for row in rows]
    name_map = _resolve_display_names(user_ids)
    entries = [
        {
            "rank": index,
            "user_id": str(_safe_int(row["user_id"])),
            "name": _public_user_name(name_map, _safe_int(row["user_id"])),
            "avatar_url": None,
            "total_messages": _safe_int(row["total_messages"]),
            "total_points": _safe_int(row["total_points"]),
        }
        for index, row in enumerate(rows, start=1)
    ]
    return web.json_response({"entries": entries, "updated_at": _get_updated_at("text_stats")})


async def _handle_public_me(request: web.Request) -> web.Response:
    session = _require_session(request)
    return web.json_response(
        {
            "user_id": str(session.get("user_id") or ""),
            "name": str(session.get("name") or ""),
            "avatar_url": _discord_avatar_url(session.get("user_id"), session.get("avatar")),
        }
    )


async def _handle_public_me_stats(request: web.Request) -> web.Response:
    session = _require_session(request)
    user_id = _parse_user_id_from_session(session)

    voice_row = db.query_one(
        """
        SELECT total_seconds, total_points
        FROM voice_stats
        WHERE user_id = ?
        """,
        (user_id,),
    )
    text_row = _safe_query_one(
        """
        SELECT total_messages, total_points
        FROM text_stats
        WHERE user_id = ?
        """,
        (user_id,),
        optional_tables=("text_stats",),
    )

    payload = {
        "voice": {
            "lifetime_seconds": _safe_int(voice_row["total_seconds"] if voice_row else 0),
            "lifetime_points": _safe_int(voice_row["total_points"] if voice_row else 0),
            "rank": _rank_for_points("voice_stats", user_id),
        },
        "text": {
            "lifetime_messages": _safe_int(text_row["total_messages"] if text_row else 0),
            "lifetime_points": _safe_int(text_row["total_points"] if text_row else 0),
            "rank": _rank_for_points("text_stats", user_id),
        },
    }
    return web.json_response(payload)


async def _handle_public_me_voice_history(request: web.Request) -> web.Response:
    session = _require_session(request)
    user_id = _parse_user_id_from_session(session)
    days = _parse_positive_int(
        request.query.get("range"),
        default=30,
        minimum=1,
        maximum=365,
        field_name="range",
    )
    recent_limit = _parse_positive_int(
        request.query.get("sessions"),
        default=12,
        minimum=1,
        maximum=50,
        field_name="sessions",
    )
    mode = _normalize_mode(request.query.get("mode"), default="day")
    cutoff = f"-{days} day"

    daily_rows = db.query_all(
        """
        SELECT date(started_at) AS day,
               SUM(duration_seconds) AS total_seconds,
               COUNT(*) AS sessions,
               COUNT(DISTINCT user_id) AS users
        FROM voice_session_log
        WHERE started_at >= datetime('now', ?)
          AND user_id = ?
        GROUP BY date(started_at)
        ORDER BY day DESC
        """,
        (cutoff, user_id),
    )
    top_users_rows = db.query_all(
        """
        SELECT user_id,
               MAX(display_name) AS display_name,
               SUM(duration_seconds) AS total_seconds,
               SUM(points) AS total_points,
               COUNT(*) AS sessions
        FROM voice_session_log
        WHERE started_at >= datetime('now', ?)
          AND user_id = ?
        GROUP BY user_id
        ORDER BY total_seconds DESC, total_points DESC
        LIMIT 1
        """,
        (cutoff, user_id),
    )
    bucket_rows = db.query_all(
        """
        WITH grouped AS (
            SELECT
                CASE
                    WHEN ? = 'hour' THEN strftime('%H', started_at)
                    WHEN ? = 'day' THEN strftime('%w', started_at)
                    WHEN ? = 'week' THEN strftime('%Y-%W', started_at)
                    ELSE strftime('%Y-%m', started_at)
                END AS bucket,
                duration_seconds,
                COALESCE(peak_users, 0) AS peak_users
            FROM voice_session_log
            WHERE started_at >= datetime('now', ?)
              AND user_id = ?
        )
        SELECT bucket,
               SUM(duration_seconds) AS total_seconds,
               COUNT(*) AS sessions,
               SUM(peak_users) AS sum_peak
        FROM grouped
        GROUP BY bucket
        ORDER BY bucket
        """,
        (mode, mode, mode, cutoff, user_id),
    )
    range_stats = db.query_one(
        """
        SELECT SUM(duration_seconds) AS total_seconds,
               SUM(points) AS total_points,
               COUNT(*) AS sessions,
               SUM(COALESCE(peak_users, 0)) AS sum_peak,
               COUNT(DISTINCT date(started_at)) AS active_days,
               MAX(ended_at) AS last_session
        FROM voice_session_log
        WHERE started_at >= datetime('now', ?)
          AND user_id = ?
        """,
        (cutoff, user_id),
    )
    lifetime_stats = db.query_one(
        """
        SELECT total_seconds, total_points, last_update
        FROM voice_stats
        WHERE user_id = ?
        """,
        (user_id,),
    )
    lifetime_sessions_row = db.query_one(
        """
        SELECT COUNT(*) AS sessions, MAX(ended_at) AS last_session
        FROM voice_session_log
        WHERE user_id = ?
        """,
        (user_id,),
    )
    recent_rows = db.query_all(
        """
        SELECT id, guild_id, channel_id, channel_name, started_at, ended_at,
               duration_seconds, points, peak_users, co_player_ids
        FROM voice_session_log
        WHERE user_id = ?
        ORDER BY datetime(ended_at) DESC, id DESC
        LIMIT ?
        """,
        (user_id, recent_limit),
    )

    name_map = _resolve_display_names([user_id])
    daily = [
        {
            "day": row["day"],
            "total_seconds": _safe_int(row["total_seconds"]),
            "sessions": _safe_int(row["sessions"]),
            "users": _safe_int(row["users"]),
        }
        for row in daily_rows
    ]

    buckets = [
        {
            "label": row["bucket"],
            "total_seconds": _safe_int(row["total_seconds"]),
            "sessions": _safe_int(row["sessions"]),
            "avg_peak": (
                _safe_int(row["sum_peak"]) / _safe_int(row["sessions"])
                if _safe_int(row["sessions"]) > 0
                else 0
            ),
        }
        for row in bucket_rows
    ]
    if mode == "hour":
        existing = {bucket["label"]: bucket for bucket in buckets}
        buckets = [
            existing.get(
                str(hour).zfill(2),
                {"label": str(hour).zfill(2), "total_seconds": 0, "sessions": 0, "avg_peak": 0},
            )
            for hour in range(24)
        ]
    elif mode == "day":
        weekdays = [
            "Sonntag",
            "Montag",
            "Dienstag",
            "Mittwoch",
            "Donnerstag",
            "Freitag",
            "Samstag",
        ]
        existing = {bucket["label"]: bucket for bucket in buckets}
        buckets = [
            {
                "label": weekdays[day],
                "total_seconds": existing.get(str(day), {}).get("total_seconds", 0),
                "sessions": existing.get(str(day), {}).get("sessions", 0),
                "avg_peak": existing.get(str(day), {}).get("avg_peak", 0),
            }
            for day in range(7)
        ]

    co_ids_all: set[int] = set()
    parsed_co_ids: list[list[int]] = []
    for row in recent_rows:
        ids: list[int] = []
        try:
            decoded = json.loads(row["co_player_ids"] or "[]")
        except Exception:
            decoded = []
        if isinstance(decoded, list):
            seen_ids: set[int] = set()
            for entry in decoded:
                try:
                    co_id = int(entry)
                except (TypeError, ValueError):
                    continue
                if co_id <= 0 or co_id == user_id or co_id in seen_ids:
                    continue
                seen_ids.add(co_id)
                ids.append(co_id)
                co_ids_all.add(co_id)
        parsed_co_ids.append(ids)
    co_name_map = _resolve_display_names(co_ids_all) if co_ids_all else {}

    recent_sessions = []
    for index, row in enumerate(recent_rows):
        co_ids = parsed_co_ids[index] if index < len(parsed_co_ids) else []
        recent_sessions.append(
            {
                "id": _safe_int(row["id"]),
                "guild_id": str(_safe_int(row["guild_id"]))
                if row["guild_id"] is not None
                else None,
                "channel_id": str(_safe_int(row["channel_id"]))
                if row["channel_id"] is not None
                else None,
                "channel_name": row["channel_name"] or None,
                "started_at": _to_iso(row["started_at"]),
                "ended_at": _to_iso(row["ended_at"]),
                "duration_seconds": _safe_int(row["duration_seconds"]),
                "points": _safe_int(row["points"]),
                "peak_users": _safe_int(row["peak_users"]),
                "co_player_count": len(co_ids),
                "co_players": [
                    {
                        "user_id": str(co_id),
                        "display_name": co_name_map.get(co_id, f"User {co_id}"),
                    }
                    for co_id in co_ids
                ],
            }
        )

    range_seconds = _safe_int(range_stats["total_seconds"] if range_stats else 0)
    range_sessions = _safe_int(range_stats["sessions"] if range_stats else 0)
    last_session = range_stats["last_session"] if range_stats else None
    if not last_session and lifetime_sessions_row:
        last_session = lifetime_sessions_row["last_session"]

    payload = {
        "range_days": days,
        "mode": mode,
        "user": {
            "user_id": str(user_id),
            "display_name": _public_user_name(name_map, user_id),
        },
        "daily": daily,
        "top_users": [
            {
                "user_id": str(user_id),
                "display_name": row["display_name"] or _public_user_name(name_map, user_id),
                "total_seconds": _safe_int(row["total_seconds"]),
                "total_points": _safe_int(row["total_points"]),
                "sessions": _safe_int(row["sessions"]),
            }
            for row in top_users_rows
        ],
        "buckets": buckets,
        "user_summary": {
            "user_id": str(user_id),
            "display_name": _public_user_name(name_map, user_id),
            "range_seconds": range_seconds,
            "range_points": _safe_int(range_stats["total_points"] if range_stats else 0),
            "range_sessions": range_sessions,
            "range_days": _safe_int(range_stats["active_days"] if range_stats else 0),
            "range_avg_session_seconds": (range_seconds / range_sessions) if range_sessions else 0,
            "range_avg_peak": (
                _safe_int(range_stats["sum_peak"] if range_stats else 0) / range_sessions
                if range_sessions
                else 0
            ),
            "lifetime_seconds": _safe_int(lifetime_stats["total_seconds"] if lifetime_stats else 0),
            "lifetime_points": _safe_int(lifetime_stats["total_points"] if lifetime_stats else 0),
            "lifetime_sessions": _safe_int(
                lifetime_sessions_row["sessions"] if lifetime_sessions_row else 0
            ),
            "lifetime_last_update": _to_iso(
                lifetime_stats["last_update"] if lifetime_stats else None
            ),
            "last_session": _to_iso(last_session),
        },
        "recent_sessions_limit": recent_limit,
        "recent_sessions": recent_sessions,
    }
    return web.json_response(payload)


async def _handle_public_me_text_history(request: web.Request) -> web.Response:
    session = _require_session(request)
    user_id = _parse_user_id_from_session(session)
    days = _parse_positive_int(
        request.query.get("range"),
        default=30,
        minimum=1,
        maximum=365,
        field_name="range",
    )
    recent_limit = _parse_positive_int(
        request.query.get("sessions"),
        default=12,
        minimum=1,
        maximum=50,
        field_name="sessions",
    )
    mode = _normalize_mode(request.query.get("mode"), default="day")
    cutoff = f"-{days} day"

    daily_rows = _safe_query_all(
        """
        SELECT date(started_at) AS day,
               SUM(message_count) AS total_messages,
               SUM(points) AS total_points,
               COUNT(*) AS sessions
        FROM text_conversation_log
        WHERE started_at >= datetime('now', ?)
          AND user_id = ?
        GROUP BY date(started_at)
        ORDER BY day DESC
        """,
        (cutoff, user_id),
        optional_tables=("text_conversation_log",),
    )
    bucket_rows = _safe_query_all(
        """
        WITH grouped AS (
            SELECT
                CASE
                    WHEN ? = 'hour' THEN strftime('%H', started_at)
                    WHEN ? = 'day' THEN strftime('%w', started_at)
                    WHEN ? = 'week' THEN strftime('%Y-%W', started_at)
                    ELSE strftime('%Y-%m', started_at)
                END AS bucket,
                message_count,
                points
            FROM text_conversation_log
            WHERE started_at >= datetime('now', ?)
              AND user_id = ?
        )
        SELECT bucket,
               SUM(message_count) AS total_messages,
               SUM(points) AS total_points,
               COUNT(*) AS sessions
        FROM grouped
        GROUP BY bucket
        ORDER BY bucket
        """,
        (mode, mode, mode, cutoff, user_id),
        optional_tables=("text_conversation_log",),
    )
    range_stats = _safe_query_one(
        """
        SELECT SUM(message_count) AS total_messages,
               SUM(points) AS total_points,
               COUNT(*) AS sessions,
               MAX(ended_at) AS last_session
        FROM text_conversation_log
        WHERE started_at >= datetime('now', ?)
          AND user_id = ?
        """,
        (cutoff, user_id),
        optional_tables=("text_conversation_log",),
    )
    lifetime_stats = _safe_query_one(
        """
        SELECT total_messages, total_points, last_update
        FROM text_stats
        WHERE user_id = ?
        """,
        (user_id,),
        optional_tables=("text_stats",),
    )
    recent_rows = _safe_query_all(
        """
        SELECT id, guild_id, channel_id, started_at, ended_at,
               message_count, points, co_participant_ids, had_interaction
        FROM text_conversation_log
        WHERE user_id = ?
        ORDER BY datetime(ended_at) DESC, id DESC
        LIMIT ?
        """,
        (user_id, recent_limit),
        optional_tables=("text_conversation_log",),
    )

    buckets = [
        {
            "label": row["bucket"],
            "total_messages": _safe_int(row["total_messages"]),
            "total_points": _safe_int(row["total_points"]),
            "sessions": _safe_int(row["sessions"]),
        }
        for row in bucket_rows
    ]
    if mode == "hour":
        existing = {bucket["label"]: bucket for bucket in buckets}
        buckets = [
            existing.get(
                str(hour).zfill(2),
                {
                    "label": str(hour).zfill(2),
                    "total_messages": 0,
                    "total_points": 0,
                    "sessions": 0,
                },
            )
            for hour in range(24)
        ]
    elif mode == "day":
        weekdays = [
            "Sonntag",
            "Montag",
            "Dienstag",
            "Mittwoch",
            "Donnerstag",
            "Freitag",
            "Samstag",
        ]
        existing = {bucket["label"]: bucket for bucket in buckets}
        buckets = [
            {
                "label": weekdays[day],
                "total_messages": existing.get(str(day), {}).get("total_messages", 0),
                "total_points": existing.get(str(day), {}).get("total_points", 0),
                "sessions": existing.get(str(day), {}).get("sessions", 0),
            }
            for day in range(7)
        ]

    payload = {
        "range_days": days,
        "mode": mode,
        "user_summary": {
            "user_id": str(user_id),
            "lifetime_messages": _safe_int(
                lifetime_stats["total_messages"] if lifetime_stats else 0
            ),
            "lifetime_points": _safe_int(lifetime_stats["total_points"] if lifetime_stats else 0),
            "range_messages": _safe_int(range_stats["total_messages"] if range_stats else 0),
            "range_points": _safe_int(range_stats["total_points"] if range_stats else 0),
            "range_sessions": _safe_int(range_stats["sessions"] if range_stats else 0),
            "last_session": _to_iso(
                (range_stats["last_session"] if range_stats else None)
                or (lifetime_stats["last_update"] if lifetime_stats else None)
            ),
        },
        "daily": [
            {
                "day": row["day"],
                "total_messages": _safe_int(row["total_messages"]),
                "total_points": _safe_int(row["total_points"]),
                "sessions": _safe_int(row["sessions"]),
            }
            for row in daily_rows
        ],
        "buckets": buckets,
        "recent_sessions_limit": recent_limit,
        "recent_sessions": [
            {
                "id": _safe_int(row["id"]),
                "channel_id": str(_safe_int(row["channel_id"]))
                if row["channel_id"] is not None
                else None,
                "started_at": _to_iso(row["started_at"]),
                "ended_at": _to_iso(row["ended_at"]),
                "message_count": _safe_int(row["message_count"]),
                "points": _safe_int(row["points"]),
                "had_interaction": _safe_int(row["had_interaction"]),
                "co_participants": _co_participant_count(row["co_participant_ids"], user_id),
            }
            for row in recent_rows
        ],
    }
    return web.json_response(payload)


async def _handle_public_me_heatmap(request: web.Request) -> web.Response:
    session = _require_session(request)
    user_id = _parse_user_id_from_session(session)
    days = _parse_positive_int(
        request.query.get("days"),
        default=90,
        minimum=7,
        maximum=365,
        field_name="days",
    )
    cutoff = (datetime.now() - timedelta(days=days)).isoformat()
    rows = db.query_all(
        """
        SELECT started_at, ended_at, duration_seconds
        FROM voice_session_log
        WHERE user_id = ? AND started_at >= ?
        ORDER BY started_at
        """,
        (user_id, cutoff),
    )
    matrix, total_seconds = _build_voice_matrix(rows)
    return web.json_response({"matrix": matrix, "total_seconds": total_seconds})


async def _handle_public_me_co_players(request: web.Request) -> web.Response:
    session = _require_session(request)
    user_id = _parse_user_id_from_session(session)
    limit = _parse_positive_int(
        request.query.get("limit"),
        default=15,
        minimum=1,
        maximum=50,
        field_name="limit",
    )
    rows = _safe_query_all(
        """
        SELECT co_player_id, sessions_together, total_minutes_together,
               last_played_together, co_player_display_name
        FROM user_co_players
        WHERE user_id = ?
        ORDER BY sessions_together DESC, total_minutes_together DESC, last_played_together DESC
        LIMIT ?
        """,
        (user_id, limit),
        optional_tables=("user_co_players",),
    )
    co_ids = [_safe_int(row["co_player_id"]) for row in rows]
    fallback_names = _resolve_display_names(co_ids)
    entries = [
        {
            "user_id": str(_safe_int(row["co_player_id"])),
            "name": str(
                row["co_player_display_name"]
                or fallback_names.get(_safe_int(row["co_player_id"]))
                or f"User {_safe_int(row['co_player_id'])}"
            ),
            "sessions_together": _safe_int(row["sessions_together"]),
            "total_minutes_together": _safe_int(row["total_minutes_together"]),
            "last_played": _to_iso(row["last_played_together"]),
        }
        for row in rows
    ]
    return web.json_response({"entries": entries})


async def _handle_discord_login(request: web.Request) -> web.Response:
    redirect_path = _sanitize_redirect_path(request.query.get("redirect"))
    data = await _call_dashboard_api(
        "/internal/v1/discord/initiate",
        {
            "scope": "identify",
            "redirect_after": PUBLIC_STATS_CALLBACK_URL,
            "requesting_service": "activity",
        },
    )
    if not data:
        return web.Response(
            text="Auth-Service nicht erreichbar.", status=503, content_type="text/plain"
        )
    authorize_url = str(data.get("authorize_url") or "").strip()
    state_id = str(data.get("state_id") or "").strip()
    if not authorize_url or not state_id:
        return web.Response(
            text="Auth-Service Antwort ungültig.", status=503, content_type="text/plain"
        )

    pre_auth = _sign(
        {
            "state_id": state_id,
            "redirect": redirect_path,
            "exp": int(time.time()) + PUBLIC_STATS_PRE_AUTH_TTL,
        }
    )
    response = web.HTTPFound(authorize_url)
    response.set_cookie(
        PUBLIC_STATS_PRE_AUTH_COOKIE,
        pre_auth,
        max_age=PUBLIC_STATS_PRE_AUTH_TTL,
        httponly=True,
        secure=_cookie_secure(),
        samesite="Lax",
        path="/",
    )
    return response


async def _handle_discord_complete(request: web.Request) -> web.Response:
    state_id = (request.query.get("state_id") or "").strip()
    pre_auth_cookie = request.cookies.get(PUBLIC_STATS_PRE_AUTH_COOKIE, "")

    redirect_path = PUBLIC_STATS_DEFAULT_REDIRECT
    if pre_auth_cookie:
        pre_auth = _verify(pre_auth_cookie)
        if pre_auth:
            redirect_path = _sanitize_redirect_path(pre_auth.get("redirect"))

    log.info(
        "discord_complete: state_id=%s pre_auth_found=%s redirect=%s",
        bool(state_id),
        bool(pre_auth_cookie),
        redirect_path,
    )

    response = web.HTTPFound(redirect_path)
    response.del_cookie(PUBLIC_STATS_PRE_AUTH_COOKIE, path="/")

    if not state_id:
        log.warning("discord_complete: kein state_id → kein Login")
        return response

    data = await _call_dashboard_api(
        "/internal/v1/discord/consume-result",
        {"state_id": state_id},
    )
    log.info(
        "discord_complete: consume-result data=%s",
        {k: v for k, v in (data or {}).items() if k != "discord_avatar"} if data else None,
    )
    if not data or not data.get("discord_id"):
        log.warning("discord_complete: kein discord_id in consume-result → kein Login")
        return response

    session_payload = {
        "user_id": str(data["discord_id"]),
        "name": str(data.get("discord_name") or ""),
        "avatar": data.get("discord_avatar") or None,
        "iat": int(time.time()),
        "exp": int(time.time()) + PUBLIC_STATS_SESSION_TTL,
    }
    response.set_cookie(
        PUBLIC_STATS_SESSION_COOKIE,
        _sign(session_payload),
        max_age=PUBLIC_STATS_SESSION_TTL,
        httponly=True,
        secure=_cookie_secure(),
        samesite="Lax",
        path="/",
    )
    return response


async def _handle_discord_logout(request: web.Request) -> web.Response:
    response = web.Response(status=204)
    response.del_cookie(PUBLIC_STATS_SESSION_COOKIE, path="/")
    return response


async def _handle_cors_preflight(request: web.Request) -> web.Response:
    return web.Response(status=204)


async def handle_health(request: web.Request) -> web.Response:
    return web.json_response({"ok": True, "ts": int(datetime.now().timestamp())})


async def handle_rank_colors(request: web.Request) -> web.Response:
    """Returns rank colors for the UI."""
    return web.json_response({"colors": RANK_COLORS, "generated_at": datetime.now().isoformat()})


async def handle_best_times(request: web.Request) -> web.Response:
    """Returns the best time windows for a specific rank.

    Query param: rank (e.g. 'initiate', 'seeker', etc.)
    Returns top 3 peak hours with day distribution.
    """
    rank = request.query.get("rank", "")
    if rank not in RANK_ORDER:
        return web.json_response({"error": "Invalid rank"}, status=400)

    now = datetime.now()
    cutoff = now - timedelta(days=7)

    # Get timeline data for the rank
    rows = db.query_all(
        """
        SELECT started_at, user_id, co_player_ids
        FROM voice_session_log
        WHERE started_at >= ?
        ORDER BY started_at
        """,
        (cutoff.isoformat(),),
    )

    # Build hourly counts for this rank
    hourly_counts: dict[int, int] = {h: 0 for h in range(24)}
    day_hour_counts: dict[int, dict[int, list[int]]] = {
        d: {h: [] for h in range(24)} for d in range(7)
    }

    for row in rows:
        try:
            started = datetime.fromisoformat(row["started_at"])
        except Exception:
            continue

        hour = started.hour
        day = started.weekday() % 7
        user_rank = _get_user_rank(row["user_id"])
        if not user_rank:
            try:
                co_ids = json.loads(row["co_player_ids"] or "[]")
                user_rank = _estimate_rank_from_co_players(
                    row["user_id"], co_ids if isinstance(co_ids, list) else []
                )
            except Exception:
                user_rank = None

        if user_rank == rank:
            hourly_counts[hour] += 1
            day_hour_counts[day][hour].append(row["user_id"])

    # Find top 3 peak hours
    peaks = sorted(hourly_counts.items(), key=lambda x: x[1], reverse=True)[:3]
    peak_hours = [{"hour": h, "count": c} for h, c in peaks if c > 0]

    # Day distribution for peak hours
    day_distribution = []
    for d in range(7):
        total = sum(len(day_hour_counts[d][h]) for h in range(24))
        day_distribution.append({"day": d, "count": total})

    return web.json_response(
        {
            "rank": rank,
            "peak_hours": peak_hours,
            "day_distribution": day_distribution,
            "generated_at": now.isoformat(),
        }
    )


# ── Server class ──────────────────────────────────────────────────────────────


class PublicStatsServer:
    def __init__(self, *, host: str = PUBLIC_STATS_HOST, port: int = PUBLIC_STATS_PORT) -> None:
        self.host = host
        self.port = port
        self._runner: web.AppRunner | None = None
        self._cors_origins = self._load_cors_origins()

        self.app = web.Application(middlewares=[self._security_mw])
        self.app["cors_origins"] = self._cors_origins
        self.app.router.add_get("/", handle_index)
        self.app.router.add_get("/api/activity-heatmap", handle_activity_heatmap)
        self.app.router.add_get("/api/rank-distribution", handle_rank_distribution)
        self.app.router.add_get("/api/lane-preferences", handle_lane_preferences)
        self.app.router.add_get("/api/new-player-windows", handle_new_player_windows)
        self.app.router.add_get("/api/timeline", handle_timeline)
        self.app.router.add_get("/api/rank-colors", handle_rank_colors)
        self.app.router.add_get("/api/best-times", handle_best_times)
        self.app.router.add_get("/api/voice-history", handle_voice_history)
        self.app.router.add_get("/api/public/leaderboard/voice", _handle_public_voice_leaderboard)
        self.app.router.add_get("/api/public/leaderboard/text", _handle_public_text_leaderboard)
        self.app.router.add_get("/api/public/me", _handle_public_me)
        self.app.router.add_get("/api/public/me/stats", _handle_public_me_stats)
        self.app.router.add_get("/api/public/me/voice-history", _handle_public_me_voice_history)
        self.app.router.add_get("/api/public/me/text-history", _handle_public_me_text_history)
        self.app.router.add_get("/api/public/me/heatmap", _handle_public_me_heatmap)
        self.app.router.add_get("/api/public/me/co-players", _handle_public_me_co_players)
        self.app.router.add_get("/auth/discord/login", _handle_discord_login)
        self.app.router.add_get("/auth/discord/complete", _handle_discord_complete)
        self.app.router.add_post("/auth/discord/logout", _handle_discord_logout)
        self.app.router.add_route("OPTIONS", "/api/public/{tail:.*}", _handle_cors_preflight)
        self.app.router.add_route("OPTIONS", "/auth/{tail:.*}", _handle_cors_preflight)
        self.app.router.add_get("/health", handle_health)

        # Serve rank icons
        from pathlib import Path

        icons_path = Path(__file__).resolve().parent / "static" / "rank_icons"
        if icons_path.is_dir():
            self.app.router.add_static("/rank_icons/", icons_path)

    @staticmethod
    def _load_cors_origins() -> set[str]:
        raw = os.getenv("PUBLIC_STATS_CORS_ORIGINS", "")
        values = [value.strip() for value in raw.split(",") if value.strip()] if raw else []
        return set(values or PUBLIC_STATS_DEFAULT_CORS_ORIGINS)

    @staticmethod
    def _is_cors_path(path: str) -> bool:
        return path.startswith("/api/public/") or path.startswith("/auth/")

    @staticmethod
    def _is_private_path(path: str) -> bool:
        return path.startswith("/api/public/me") or path.startswith("/auth/")

    @staticmethod
    def _append_vary(resp: web.StreamResponse, value: str) -> None:
        current = resp.headers.get("Vary")
        if not current:
            resp.headers["Vary"] = value
            return
        values = [entry.strip() for entry in current.split(",") if entry.strip()]
        if value not in values:
            values.append(value)
            resp.headers["Vary"] = ", ".join(values)

    def _apply_cors_headers(self, request: web.Request, resp: web.StreamResponse) -> None:
        if not self._is_cors_path(request.path):
            return
        origin = request.headers.get("Origin")
        if origin and origin in self._cors_origins:
            resp.headers["Access-Control-Allow-Origin"] = origin
            resp.headers["Access-Control-Allow-Credentials"] = "true"
            resp.headers["Access-Control-Allow-Methods"] = "GET,POST,OPTIONS"
            resp.headers["Access-Control-Allow-Headers"] = (
                request.headers.get("Access-Control-Request-Headers") or "Content-Type"
            )
            resp.headers["Access-Control-Max-Age"] = "600"
            self._append_vary(resp, "Origin")

    @web.middleware
    async def _security_mw(self, request: web.Request, handler):
        try:
            resp = await handler(request)
        except web.HTTPException as ex:
            resp = ex
        except Exception:
            log.exception("Unhandled error in public stats server")
            resp = web.Response(
                text='{"error":"internal"}', content_type="application/json", status=500
            )

        if isinstance(resp, web.StreamResponse):
            if self._is_private_path(request.path):
                resp.headers["Cache-Control"] = "no-store"
            else:
                resp.headers["Cache-Control"] = "public, max-age=60"
            resp.headers["X-Content-Type-Options"] = "nosniff"
            self._apply_cors_headers(request, resp)
        return resp

    async def start(self) -> None:
        self._runner = web.AppRunner(self.app)
        await self._runner.setup()

        max_retries = 5
        retry_delay = 0.5
        for attempt in range(max_retries):
            try:
                site = web.TCPSite(self._runner, host=self.host, port=self.port)
                await site.start()
                log.info("PublicStatsServer läuft auf %s:%s", self.host, self.port)
                return
            except OSError as e:
                import errno

                is_in_use = e.errno == errno.EADDRINUSE
                if is_in_use and attempt < max_retries - 1:
                    await asyncio.sleep(retry_delay)
                    retry_delay *= 2
                    continue
                log.exception("PublicStatsServer konnte nicht starten")
                break

    async def stop(self) -> None:
        if self._runner:
            await self._runner.cleanup()
            self._runner = None
            log.info("PublicStatsServer gestoppt")


# ── Helper ─────────────────────────────────────────────────────────────────────


def _get_user_rank(user_id: int) -> str | None:
    row = db.query_one(
        """
        SELECT deadlock_rank_name
        FROM steam_links
        WHERE user_id = ? AND verified = 1
        ORDER BY primary_account DESC, deadlock_rank_updated_at DESC
        LIMIT 1
        """,
        (user_id,),
    )
    if row and row["deadlock_rank_name"]:
        name = row["deadlock_rank_name"].lower()
        if name in RANK_ORDER:
            return name
    return None
