from __future__ import annotations

import json
import logging
import sqlite3
import time
from collections.abc import Iterable

from service import db

log = logging.getLogger(__name__)

# Namespaces with user-specific KV entries
AI_ONBOARDING_SESSIONS_NS = "ai_onboarding:sessions"
AI_ONBOARDING_VIEWS_NS = "ai_onboarding:persistent_views"
VOICE_NUDGE_NAMESPACES = ("voice_nudge_first_seen", "voice_nudge_done")

# Tables keyed by a single user column
_USER_TABLES: tuple[tuple[str, str], ...] = (
    ("voice_stats", "user_id"),
    ("voice_session_log", "user_id"),
    ("voice_feedback_requests", "user_id"),
    ("voice_feedback_responses", "user_id"),
    ("user_activity_patterns", "user_id"),
    ("message_activity", "user_id"),
    ("member_events", "user_id"),
    ("member_leave_surveys", "user_id"),
    ("steam_links", "user_id"),
    ("steam_cleanup_poll_state", "user_id"),
    ("steam_friendship_miss_tracker", "user_id"),
    ("steam_role_cleanup_pending", "user_id"),
    ("steam_beta_invites", "discord_id"),
    ("beta_invite_intent", "discord_id"),
    ("beta_invite_audit", "discord_id"),
    ("server_faq_logs", "user_id"),
    ("persistent_views", "user_id"),
    ("user_retention_tracking", "user_id"),
    ("user_retention_messages", "user_id"),
    ("voice_channel_anchors", "user_id"),
    ("coaching_sessions", "user_id"),
    ("claimed_threads", "assigned_user_id"),
    ("claimed_threads", "claimed_by_id"),
    ("steam_nudge_state", "user_id"),
    ("twitch_streamers", "discord_user_id"),
    ("twitch_link_clicks", "discord_user_id"),
    ("user_data", "user_id"),
    ("notification_log", "user_id"),
    ("notification_queue", "user_id"),
    ("dm_response_tracking", "user_id"),
    ("tempvoice_owner_prefs", "owner_id"),
    ("tempvoice_lanes", "owner_id"),
    ("tempvoice_lurkers", "user_id"),
    ("tempvoice_bans", "owner_id"),
    ("tempvoice_bans", "banned_id"),
    ("steam_quick_invites", "reserved_by"),
    ("issue_reports", "user_id"),
)

_STEAM_SIDE_TABLES: tuple[tuple[str, str], ...] = (
    ("live_player_state", "steam_id"),
    ("deadlock_voice_watch", "steam_id"),
    ("steam_rich_presence", "steam_id"),
    ("steam_presence_watchlist", "steam_id"),
    ("steam_friend_requests", "steam_id"),
    ("steam_friendship_miss_tracker", "steam_id"),
    ("steam_beta_invites", "steam_id64"),
    ("beta_invite_audit", "steam_id64"),
)

# Delete SQL mapping
_DELETE_SQL_BY_TARGET: dict[tuple[str, str], str] = {
    ("voice_stats", "user_id"): "DELETE FROM voice_stats WHERE user_id=?",
    ("voice_session_log", "user_id"): "DELETE FROM voice_session_log WHERE user_id=?",
    ("voice_feedback_requests", "user_id"): "DELETE FROM voice_feedback_requests WHERE user_id=?",
    ("voice_feedback_responses", "user_id"): "DELETE FROM voice_feedback_responses WHERE user_id=?",
    ("user_activity_patterns", "user_id"): "DELETE FROM user_activity_patterns WHERE user_id=?",
    ("message_activity", "user_id"): "DELETE FROM message_activity WHERE user_id=?",
    ("member_events", "user_id"): "DELETE FROM member_events WHERE user_id=?",
    ("member_leave_surveys", "user_id"): "DELETE FROM member_leave_surveys WHERE user_id=?",
    ("steam_links", "user_id"): "DELETE FROM steam_links WHERE user_id=?",
    ("steam_cleanup_poll_state", "user_id"): "DELETE FROM steam_cleanup_poll_state WHERE user_id=?",
    (
        "steam_friendship_miss_tracker",
        "user_id",
    ): "DELETE FROM steam_friendship_miss_tracker WHERE user_id=?",
    (
        "steam_role_cleanup_pending",
        "user_id",
    ): "DELETE FROM steam_role_cleanup_pending WHERE user_id=?",
    ("steam_beta_invites", "discord_id"): "DELETE FROM steam_beta_invites WHERE discord_id=?",
    ("beta_invite_intent", "discord_id"): "DELETE FROM beta_invite_intent WHERE discord_id=?",
    ("beta_invite_audit", "discord_id"): "DELETE FROM beta_invite_audit WHERE discord_id=?",
    ("server_faq_logs", "user_id"): "DELETE FROM server_faq_logs WHERE user_id=?",
    ("persistent_views", "user_id"): "DELETE FROM persistent_views WHERE user_id=?",
    ("user_retention_tracking", "user_id"): "DELETE FROM user_retention_tracking WHERE user_id=?",
    ("user_retention_messages", "user_id"): "DELETE FROM user_retention_messages WHERE user_id=?",
    ("voice_channel_anchors", "user_id"): "DELETE FROM voice_channel_anchors WHERE user_id=?",
    ("coaching_sessions", "user_id"): "DELETE FROM coaching_sessions WHERE user_id=?",
    ("claimed_threads", "assigned_user_id"): "DELETE FROM claimed_threads WHERE assigned_user_id=?",
    ("claimed_threads", "claimed_by_id"): "DELETE FROM claimed_threads WHERE claimed_by_id=?",
    ("steam_nudge_state", "user_id"): "DELETE FROM steam_nudge_state WHERE user_id=?",
    ("twitch_streamers", "discord_user_id"): "DELETE FROM twitch_streamers WHERE discord_user_id=?",
    (
        "twitch_link_clicks",
        "discord_user_id",
    ): "DELETE FROM twitch_link_clicks WHERE discord_user_id=?",
    ("user_data", "user_id"): "DELETE FROM user_data WHERE user_id=?",
    ("notification_log", "user_id"): "DELETE FROM notification_log WHERE user_id=?",
    ("notification_queue", "user_id"): "DELETE FROM notification_queue WHERE user_id=?",
    ("dm_response_tracking", "user_id"): "DELETE FROM dm_response_tracking WHERE user_id=?",
    ("tempvoice_owner_prefs", "owner_id"): "DELETE FROM tempvoice_owner_prefs WHERE owner_id=?",
    ("tempvoice_lanes", "owner_id"): "DELETE FROM tempvoice_lanes WHERE owner_id=?",
    ("tempvoice_lurkers", "user_id"): "DELETE FROM tempvoice_lurkers WHERE user_id=?",
    ("tempvoice_bans", "owner_id"): "DELETE FROM tempvoice_bans WHERE owner_id=?",
    ("tempvoice_bans", "banned_id"): "DELETE FROM tempvoice_bans WHERE banned_id=?",
    ("steam_quick_invites", "reserved_by"): "DELETE FROM steam_quick_invites WHERE reserved_by=?",
    ("issue_reports", "user_id"): "DELETE FROM issue_reports WHERE user_id=?",
    ("live_player_state", "steam_id"): "DELETE FROM live_player_state WHERE steam_id=?",
    ("deadlock_voice_watch", "steam_id"): "DELETE FROM deadlock_voice_watch WHERE steam_id=?",
    ("steam_rich_presence", "steam_id"): "DELETE FROM steam_rich_presence WHERE steam_id=?",
    (
        "steam_presence_watchlist",
        "steam_id",
    ): "DELETE FROM steam_presence_watchlist WHERE steam_id=?",
    ("steam_friend_requests", "steam_id"): "DELETE FROM steam_friend_requests WHERE steam_id=?",
    (
        "steam_friendship_miss_tracker",
        "steam_id",
    ): "DELETE FROM steam_friendship_miss_tracker WHERE steam_id=?",
    ("steam_beta_invites", "steam_id64"): "DELETE FROM steam_beta_invites WHERE steam_id64=?",
    ("beta_invite_audit", "steam_id64"): "DELETE FROM beta_invite_audit WHERE steam_id64=?",
}

# Select SQL mapping
_SELECT_SQL_BY_TARGET: dict[tuple[str, str], str] = {
    ("voice_stats", "user_id"): "SELECT * FROM voice_stats WHERE user_id=?",
    ("voice_session_log", "user_id"): "SELECT * FROM voice_session_log WHERE user_id=?",
    ("voice_feedback_requests", "user_id"): "SELECT * FROM voice_feedback_requests WHERE user_id=?",
    (
        "voice_feedback_responses",
        "user_id",
    ): "SELECT * FROM voice_feedback_responses WHERE user_id=?",
    ("user_activity_patterns", "user_id"): "SELECT * FROM user_activity_patterns WHERE user_id=?",
    ("message_activity", "user_id"): "SELECT * FROM message_activity WHERE user_id=?",
    ("member_events", "user_id"): "SELECT * FROM member_events WHERE user_id=?",
    ("member_leave_surveys", "user_id"): "SELECT * FROM member_leave_surveys WHERE user_id=?",
    ("steam_links", "user_id"): "SELECT * FROM steam_links WHERE user_id=?",
    (
        "steam_cleanup_poll_state",
        "user_id",
    ): "SELECT * FROM steam_cleanup_poll_state WHERE user_id=?",
    (
        "steam_friendship_miss_tracker",
        "user_id",
    ): "SELECT * FROM steam_friendship_miss_tracker WHERE user_id=?",
    (
        "steam_role_cleanup_pending",
        "user_id",
    ): "SELECT * FROM steam_role_cleanup_pending WHERE user_id=?",
    ("steam_beta_invites", "discord_id"): "SELECT * FROM steam_beta_invites WHERE discord_id=?",
    ("beta_invite_intent", "discord_id"): "SELECT * FROM beta_invite_intent WHERE discord_id=?",
    ("beta_invite_audit", "discord_id"): "SELECT * FROM beta_invite_audit WHERE discord_id=?",
    ("server_faq_logs", "user_id"): "SELECT * FROM server_faq_logs WHERE user_id=?",
    ("persistent_views", "user_id"): "SELECT * FROM persistent_views WHERE user_id=?",
    ("user_retention_tracking", "user_id"): "SELECT * FROM user_retention_tracking WHERE user_id=?",
    ("user_retention_messages", "user_id"): "SELECT * FROM user_retention_messages WHERE user_id=?",
    ("voice_channel_anchors", "user_id"): "SELECT * FROM voice_channel_anchors WHERE user_id=?",
    ("coaching_sessions", "user_id"): "SELECT * FROM coaching_sessions WHERE user_id=?",
    (
        "claimed_threads",
        "assigned_user_id",
    ): "SELECT * FROM claimed_threads WHERE assigned_user_id=?",
    ("claimed_threads", "claimed_by_id"): "SELECT * FROM claimed_threads WHERE claimed_by_id=?",
    ("steam_nudge_state", "user_id"): "SELECT * FROM steam_nudge_state WHERE user_id=?",
    (
        "twitch_streamers",
        "discord_user_id",
    ): "SELECT * FROM twitch_streamers WHERE discord_user_id=?",
    (
        "twitch_link_clicks",
        "discord_user_id",
    ): "SELECT * FROM twitch_link_clicks WHERE discord_user_id=?",
    ("user_data", "user_id"): "SELECT * FROM user_data WHERE user_id=?",
    ("notification_log", "user_id"): "SELECT * FROM notification_log WHERE user_id=?",
    ("notification_queue", "user_id"): "SELECT * FROM notification_queue WHERE user_id=?",
    ("dm_response_tracking", "user_id"): "SELECT * FROM dm_response_tracking WHERE user_id=?",
    ("tempvoice_owner_prefs", "owner_id"): "SELECT * FROM tempvoice_owner_prefs WHERE owner_id=?",
    ("tempvoice_lanes", "owner_id"): "SELECT * FROM tempvoice_lanes WHERE owner_id=?",
    ("tempvoice_lurkers", "user_id"): "SELECT * FROM tempvoice_lurkers WHERE user_id=?",
    ("tempvoice_bans", "owner_id"): "SELECT * FROM tempvoice_bans WHERE owner_id=?",
    ("tempvoice_bans", "banned_id"): "SELECT * FROM tempvoice_bans WHERE banned_id=?",
    ("steam_quick_invites", "reserved_by"): "SELECT * FROM steam_quick_invites WHERE reserved_by=?",
    ("issue_reports", "user_id"): "SELECT * FROM issue_reports WHERE user_id=?",
    ("live_player_state", "steam_id"): "SELECT * FROM live_player_state WHERE steam_id=?",
    ("deadlock_voice_watch", "steam_id"): "SELECT * FROM deadlock_voice_watch WHERE steam_id=?",
    ("steam_rich_presence", "steam_id"): "SELECT * FROM steam_rich_presence WHERE steam_id=?",
    (
        "steam_presence_watchlist",
        "steam_id",
    ): "SELECT * FROM steam_presence_watchlist WHERE steam_id=?",
    ("steam_friend_requests", "steam_id"): "SELECT * FROM steam_friend_requests WHERE steam_id=?",
    (
        "steam_friendship_miss_tracker",
        "steam_id",
    ): "SELECT * FROM steam_friendship_miss_tracker WHERE steam_id=?",
    ("steam_beta_invites", "steam_id64"): "SELECT * FROM steam_beta_invites WHERE steam_id64=?",
    ("beta_invite_audit", "steam_id64"): "SELECT * FROM beta_invite_audit WHERE steam_id64=?",
}


def _table_exists(conn: sqlite3.Connection, table: str) -> bool:
    try:
        cur = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (table,),
        )
        return cur.fetchone() is not None
    except Exception:
        return False


def is_opted_out(user_id: int) -> bool:
    """Synchronously check whether the user is opted out."""
    try:
        row = db.query_one(
            "SELECT opted_out FROM user_privacy WHERE user_id=?",
            (int(user_id),),
        )
        return bool(row and row[0])
    except Exception:
        log.debug("privacy check failed for user %s", user_id, exc_info=True)
        return False


async def set_opt_in(user_id: int) -> None:
    """Remove opt-out flag (user opt-in)."""
    ts = int(time.time())
    async with db.transaction() as conn:
        conn.execute(
            """
            INSERT INTO user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
            VALUES (?, 0, NULL, 'user_opt_in', ?)
            ON CONFLICT(user_id) DO UPDATE SET
              opted_out = 0,
              deleted_at = NULL,
              reason = excluded.reason,
              updated_at = excluded.updated_at
            """,
            (int(user_id), ts),
        )


def _delete_where(conn: sqlite3.Connection, table: str, column: str, value: object) -> int:
    sql = _DELETE_SQL_BY_TARGET.get((table, column))
    if not sql or not _table_exists(conn, table):
        return 0
    cur = conn.execute(sql, (value,))
    return max(cur.rowcount or 0, 0)


def _delete_any_of(
    conn: sqlite3.Connection, table: str, columns: Iterable[str], value: object
) -> int:
    cols = list(columns)
    if not cols or not _table_exists(conn, table):
        return 0
    if table != "user_co_players":
        return 0
    if set(cols) != {"user_id", "co_player_id"}:
        return 0
    cur = conn.execute(
        "DELETE FROM user_co_players WHERE user_id=? OR co_player_id=?",
        (value, value),
    )
    return max(cur.rowcount or 0, 0)


def _fetch_rows(
    conn: sqlite3.Connection, table: str, column: str, value: object
) -> list[dict[str, object]]:
    sql = _SELECT_SQL_BY_TARGET.get((table, column))
    if not sql or not _table_exists(conn, table):
        return []
    cur = conn.execute(sql, (value,))
    cols = [col[0] for col in cur.description or []]
    rows = cur.fetchall() or []
    return [{col: row[idx] for idx, col in enumerate(cols)} for row in rows]


def _fetch_rows_any(
    conn: sqlite3.Connection, table: str, columns: Iterable[str], value: object
) -> list[dict[str, object]]:
    cols = list(columns)
    if not cols or not _table_exists(conn, table):
        return []
    if table != "user_co_players":
        return []
    if set(cols) != {"user_id", "co_player_id"}:
        return []
    cur = conn.execute(
        "SELECT * FROM user_co_players WHERE user_id=? OR co_player_id=?",
        (value, value),
    )
    colnames = [col[0] for col in cur.description or []]
    rows = cur.fetchall() or []
    return [{name: row[idx] for idx, name in enumerate(colnames)} for row in rows]


def _redact_co_players(rows: list[dict[str, object]], uid: int) -> list[dict[str, object]]:
    sanitized: list[dict[str, object]] = []
    for row in rows:
        user_id = row.get("user_id")
        co_player_id = row.get("co_player_id")

        # only keep rows where the user is involved
        if int(user_id or uid) != uid and int(co_player_id or uid) != uid:
            continue

        row_copy = dict(row)
        if "co_player_id" in row_copy:
            row_copy["co_player_id"] = "redacted"
        if int(user_id or uid) != uid:
            row_copy["user_id"] = uid
        if "user_display_name" in row_copy and int(user_id or uid) != uid:
            row_copy["user_display_name"] = "redacted"
        if "co_player_display_name" in row_copy and int(co_player_id or uid) != uid:
            row_copy["co_player_display_name"] = "redacted"
        sanitized.append(row_copy)
    return sanitized


def _redact_other_ids(
    rows: list[dict[str, object]], uid: int, keep: str, redact_fields: Iterable[str]
) -> list[dict[str, object]]:
    cleaned: list[dict[str, object]] = []
    for row in rows:
        r = dict(row)
        for key in redact_fields:
            if key == keep:
                continue
            if key in r and int(r.get(key) or 0) != uid:
                r[key] = "redacted"
        cleaned.append(r)
    return cleaned


def _purge_kv_entries(conn: sqlite3.Connection, user_id: int, summary: dict[str, int]) -> None:
    if not _table_exists(conn, "kv_store"):
        return

    removed = conn.execute(
        "DELETE FROM kv_store WHERE ns=? AND k=?",
        (AI_ONBOARDING_SESSIONS_NS, str(int(user_id))),
    ).rowcount
    summary["kv_ai_onboarding_sessions"] = max(removed or 0, 0)

    removed_views = 0
    rows = conn.execute(
        "SELECT k, v FROM kv_store WHERE ns=?",
        (AI_ONBOARDING_VIEWS_NS,),
    ).fetchall()
    for row in rows:
        try:
            payload = json.loads(row["v"] if isinstance(row, sqlite3.Row) else row[1])
        except Exception:  # noqa: S112
            continue
        if int(payload.get("user_id", 0) or 0) == int(user_id):
            conn.execute(
                "DELETE FROM kv_store WHERE ns=? AND k=?",
                (
                    AI_ONBOARDING_VIEWS_NS,
                    str(row["k"] if isinstance(row, sqlite3.Row) else row[0]),
                ),
            )
            removed_views += 1
    summary["kv_ai_onboarding_views"] = removed_views

    nudge_removed = 0
    for ns in VOICE_NUDGE_NAMESPACES:
        cur = conn.execute(
            "DELETE FROM kv_store WHERE ns=? AND k=?",
            (ns, str(int(user_id))),
        )
        nudge_removed += max(cur.rowcount or 0, 0)
    summary["kv_voice_nudge"] = nudge_removed


def _fetch_steam_ids(conn: sqlite3.Connection, user_id: int) -> list[str]:
    if not _table_exists(conn, "steam_links"):
        return []
    rows = conn.execute(
        "SELECT steam_id FROM steam_links WHERE user_id=?",
        (int(user_id),),
    ).fetchall()
    steam_ids: list[str] = []
    for row in rows:
        try:
            steam_ids.append(str(row["steam_id"] if isinstance(row, sqlite3.Row) else row[0]))
        except Exception:  # noqa: S112
            continue
    return steam_ids


def _purge_steam_side(
    conn: sqlite3.Connection, steam_ids: Iterable[str], summary: dict[str, int]
) -> None:
    for sid in steam_ids:
        for table, column in _STEAM_SIDE_TABLES:
            summary[f"{table}:{sid}"] = _delete_where(conn, table, column, sid)


def export_user_data(user_id: int) -> dict[str, object]:
    """
    Build a JSON-friendly snapshot of all known user data (read-only).
    """
    uid = int(user_id)
    snapshot: dict[str, object] = {
        "user_id": uid,
        "generated_at": int(time.time()),
        "tables": {},
        "kv": {},
        "steam_ids": [],
    }
    with db.get_conn() as conn:
        tables: dict[str, list[dict[str, object]]] = {}
        for table, column in _USER_TABLES:
            key = f"{table}.{column}"
            tables[key] = _fetch_rows(conn, table, column, uid)

        co_player_rows = _fetch_rows_any(conn, "user_co_players", ("user_id", "co_player_id"), uid)
        tables["user_co_players"] = _redact_co_players(co_player_rows, uid)

        steam_ids = _fetch_steam_ids(conn, uid)
        snapshot["steam_ids"] = steam_ids
        for sid in steam_ids:
            for table, column in _STEAM_SIDE_TABLES:
                tables[f"{table}:{sid}"] = _fetch_rows(conn, table, column, sid)

        snapshot["tables"] = tables

        kv_data: dict[str, object] = {}
        if _table_exists(conn, "kv_store"):
            kv_data["ai_onboarding_sessions"] = (
                db.get_kv(AI_ONBOARDING_SESSIONS_NS, str(uid)) or None
            )
            kv_data["ai_onboarding_views"] = []
            rows = conn.execute(
                "SELECT k, v FROM kv_store WHERE ns=?",
                (AI_ONBOARDING_VIEWS_NS,),
            ).fetchall()
            for row in rows:
                try:
                    payload = json.loads(row["v"] if isinstance(row, sqlite3.Row) else row[1])
                except Exception:  # noqa: S112
                    continue
                target_uid = int(payload.get("user_id", 0) or 0)
                if target_uid == uid:
                    kv_data["ai_onboarding_views"].append(payload)
            kv_data["voice_nudge"] = {
                "first_seen": db.get_kv(VOICE_NUDGE_NAMESPACES[0], str(uid)),
                "done": db.get_kv(VOICE_NUDGE_NAMESPACES[1], str(uid)),
            }
        snapshot["kv"] = kv_data

        if _table_exists(conn, "user_privacy"):
            snapshot["user_privacy"] = _fetch_rows(conn, "user_privacy", "user_id", uid)

        # Redaction: co_player_ids in voice logs (contains other users)
        voice_log_key = "voice_session_log.user_id"
        if voice_log_key in tables:
            for row in tables[voice_log_key]:
                if "co_player_ids" in row:
                    row["co_player_ids"] = None

        # Redaction: avoid leaking foreign IDs in mixed tables
        if "tempvoice_bans.owner_id" in tables:
            tables["tempvoice_bans.owner_id"] = _redact_other_ids(
                tables["tempvoice_bans.owner_id"],
                uid,
                keep="owner_id",
                redact_fields=("banned_id",),
            )
        if "tempvoice_bans.banned_id" in tables:
            tables["tempvoice_bans.banned_id"] = _redact_other_ids(
                tables["tempvoice_bans.banned_id"],
                uid,
                keep="banned_id",
                redact_fields=("owner_id",),
            )
        if "claimed_threads.assigned_user_id" in tables:
            tables["claimed_threads.assigned_user_id"] = _redact_other_ids(
                tables["claimed_threads.assigned_user_id"],
                uid,
                keep="assigned_user_id",
                redact_fields=("claimed_by_id",),
            )
        if "claimed_threads.claimed_by_id" in tables:
            tables["claimed_threads.claimed_by_id"] = _redact_other_ids(
                tables["claimed_threads.claimed_by_id"],
                uid,
                keep="claimed_by_id",
                redact_fields=("assigned_user_id",),
            )

    return snapshot


async def delete_user_data(user_id: int, *, reason: str = "user_request") -> dict[str, int]:
    """
    Delete user-related data from all known tables and mark an opt-out.
    Returns a summary of deleted rows per table key.
    """
    uid = int(user_id)
    ts = int(time.time())
    summary: dict[str, int] = {}

    async with db.transaction() as conn:
        steam_ids = _fetch_steam_ids(conn, uid)

        for table, column in _USER_TABLES:
            summary[f"{table}.{column}"] = _delete_where(conn, table, column, uid)

        summary["user_co_players"] = _delete_any_of(
            conn, "user_co_players", ("user_id", "co_player_id"), uid
        )

        _purge_steam_side(conn, steam_ids, summary)

        _purge_kv_entries(conn, uid, summary)

        conn.execute(
            """
            INSERT INTO user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
            VALUES (?, 1, ?, ?, ?)
            ON CONFLICT(user_id) DO UPDATE SET
              opted_out = 1,
              deleted_at = excluded.deleted_at,
              reason = excluded.reason,
              updated_at = excluded.updated_at
            """,
            (uid, ts, reason, ts),
        )
        summary["user_privacy_updated"] = 1
        summary["steam_ids_removed"] = len(steam_ids)
        summary["steam_ids"] = steam_ids

    return summary
