# =========================================
# Deadlock-Bots – Zentrale SQLite-DB (KANONISCH)
# =========================================
# - Eine gemeinsame SQLite-Datei für alle Bots/Cogs.
# - Konfiguration (nur noch diese ENV-Keys):
#     DEADLOCK_DB_PATH  -> kompletter Dateipfad (höchste Priorität)
#     DEADLOCK_DB_DIR   -> Verzeichnis; Datei heißt dann deadlock.sqlite3
# - KEIN automatisches Setzen von LIVE_DB_PATH mehr (ENV bleibt sauber).
# - WAL, FOREIGN_KEYS, Busy-Timeout, Autocheckpoint, Journal-Limit aktiviert.
# - Python ≥ 3.10
# =========================================

from __future__ import annotations

import asyncio
import contextvars
import json
import logging
import os
import sqlite3
import threading
import time
from collections.abc import AsyncIterator, Iterable, Iterator
from contextlib import asynccontextmanager, contextmanager
from pathlib import Path
from typing import Any

log = logging.getLogger(__name__)

# ---- Timeouts (per-connection) ---------------------------------------------
# Default: wait up to 15s on busy locks; override with ENV if needed.
DB_BUSY_TIMEOUT_MS = int(os.environ.get("DEADLOCK_DB_BUSY_TIMEOUT_MS", "15000"))
DB_CONNECT_TIMEOUT = float(os.environ.get("DEADLOCK_DB_TIMEOUT", str(DB_BUSY_TIMEOUT_MS / 1000)))

# ---- Env-Keys (nur diese beiden werden unterstützt) ----
ENV_DB_PATH = "DEADLOCK_DB_PATH"  # kompletter Pfad zur DB-Datei (höchste Prio)
ENV_DB_DIR = "DEADLOCK_DB_DIR"  # nur Verzeichnis; Datei = deadlock.sqlite3

# ---- Fixer DB-Pfad relativ zum Repo-Root ----
# data/deadlock.sqlite3 liegt eine Ebene über diesem service/-Verzeichnis.
_REPO_ROOT = Path(__file__).parent.parent
DEFAULT_DIR = str(_REPO_ROOT / "data")
DB_NAME = "deadlock.sqlite3"
# Maximal zugelassene Zeilen in der steam_tasks-Tabelle (älteste werden gekappt)
STEAM_TASKS_MAX_ROWS = int(os.environ.get("STEAM_TASKS_MAX_ROWS", "1000"))
STEAM_TASKS_KV_NS = "steam_tasks"
STEAM_TASKS_KV_MAX_ROWS_KEY = "max_rows"
DEFAULT_OAUTH_STATE_TTL_SECONDS = int(os.environ.get("DEADLOCK_OAUTH_STATE_TTL_SECONDS", "21600"))

# ---- Modulweiter Zustand ----
_CONN: sqlite3.Connection | None = None
_LOCK = threading.RLock()
_ASYNC_LOCK = asyncio.Lock()
_DB_PATH_CACHED: str | None = None

logger = logging.getLogger(__name__)
Row = sqlite3.Row  # Typalias für Konsumenten

# ContextVar, um festzustellen ob wir uns in einem (verschachtelten) Transaction-Block befinden
_TX_DEPTH: contextvars.ContextVar[int] = contextvars.ContextVar("deadlock_db_tx_depth", default=0)


def _table_columns(cursor: sqlite3.Cursor, table_name: str) -> list[str]:
    rows = cursor.execute(f"PRAGMA table_info({table_name})").fetchall()
    return [str(row[1]) for row in rows]


def _migrate_legacy_coaching_sessions(cursor: sqlite3.Cursor) -> None:
    tables = cursor.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='coaching_sessions'"
    ).fetchone()
    if not tables:
        return

    columns = _table_columns(cursor, "coaching_sessions")
    if "id" in columns and "discord_user_id" in columns:
        return
    if "user_id" not in columns:
        return

    log.warning("Migrating legacy coaching_sessions table to the current schema")
    cursor.execute("ALTER TABLE coaching_sessions RENAME TO coaching_sessions_legacy")
    cursor.execute(
        """
        CREATE TABLE coaching_sessions (
          id TEXT PRIMARY KEY,
          request_id INTEGER,
          coach_id TEXT,
          discord_user_id INTEGER NOT NULL,
          discord_username TEXT,
          discord_channel_id INTEGER,
          discord_thread_id INTEGER,
          status TEXT DEFAULT 'active',
          role_assigned_at INTEGER,
          role_reminder_at INTEGER,
          role_expires_at INTEGER,
          voice_channel_id INTEGER,
          voice_started_at INTEGER,
          voice_last_seen_at INTEGER,
          survey_sent_at INTEGER,
          scheduled_at INTEGER,
          started_at INTEGER DEFAULT (strftime('%s','now')),
          completed_at INTEGER,
          created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        )
        """
    )
    cursor.execute(
        """
        INSERT INTO coaching_sessions (
          id,
          request_id,
          coach_id,
          discord_user_id,
          discord_username,
          discord_channel_id,
          discord_thread_id,
          status,
          voice_channel_id,
          voice_started_at,
          voice_last_seen_at,
          survey_sent_at,
          started_at,
          created_at
        )
        SELECT
          COALESCE(match_id, 'legacy-' || user_id || '-' || COALESCE(thread_id, rowid)),
          NULL,
          NULL,
          user_id,
          NULL,
          NULL,
          thread_id,
          CASE WHEN COALESCE(is_active, 1) = 1 THEN 'active' ELSE 'completed' END,
          voice_channel_id,
          voice_started_at,
          voice_last_seen_at,
          survey_sent_at,
          CAST(COALESCE(strftime('%s', created_at), strftime('%s', 'now')) AS INTEGER),
          CAST(COALESCE(strftime('%s', created_at), strftime('%s', 'now')) AS INTEGER)
        FROM coaching_sessions_legacy
        """
    )


class DBCursorProxy:
    """Small compatibility wrapper around sqlite3.Cursor."""

    __slots__ = ("_cursor", "_lock")

    def __init__(self, cursor: sqlite3.Cursor, lock: threading.RLock | None = None) -> None:
        self._cursor = cursor
        self._lock = lock

    def _run(self, fn, *args):
        if self._lock is None:
            return fn(*args)
        with self._lock:
            return fn(*args)

    def execute(self, sql: str, params: Iterable[Any] = ()) -> DBCursorProxy:
        self._run(self._cursor.execute, sql, params)
        return self

    def executemany(self, sql: str, seq_of_params: Iterable[Iterable[Any]]) -> DBCursorProxy:
        self._run(self._cursor.executemany, sql, seq_of_params)
        return self

    def fetchone(self):
        return self._run(self._cursor.fetchone)

    def fetchall(self):
        return self._run(self._cursor.fetchall)

    def fetchmany(self, size: int | None = None):
        if size is None:
            return self._run(self._cursor.fetchmany)
        return self._run(self._cursor.fetchmany, size)

    def close(self) -> None:
        self._run(self._cursor.close)

    @property
    def rowcount(self) -> int:
        return self._run(lambda: self._cursor.rowcount)

    @property
    def lastrowid(self) -> int:
        return self._run(lambda: self._cursor.lastrowid)

    @property
    def description(self):
        return self._run(lambda: self._cursor.description)

    def __iter__(self):
        return iter(self.fetchall())

    def __getattr__(self, name: str):
        return getattr(self._cursor, name)


class DBConnectionProxy:
    """
    Controlled DB session for external callers.
    SQL is still executed via the single shared connection managed by this module.
    """

    __slots__ = ("_conn", "_lock_per_call")

    def __init__(self, conn: sqlite3.Connection, *, lock_per_call: bool = False) -> None:
        self._conn = conn
        self._lock_per_call = lock_per_call

    def _run(self, fn, *args):
        if not self._lock_per_call:
            return fn(*args)
        with _LOCK:
            return fn(*args)

    def execute(self, sql: str, params: Iterable[Any] = ()) -> DBCursorProxy:
        cur = self._run(self._conn.execute, sql, params)
        return DBCursorProxy(cur, lock=_LOCK if self._lock_per_call else None)

    def executemany(self, sql: str, seq_of_params: Iterable[Iterable[Any]]) -> DBCursorProxy:
        cur = self._run(self._conn.executemany, sql, seq_of_params)
        return DBCursorProxy(cur, lock=_LOCK if self._lock_per_call else None)

    def executescript(self, sql_script: str) -> DBCursorProxy:
        cur = self._run(self._conn.executescript, sql_script)
        return DBCursorProxy(cur, lock=_LOCK if self._lock_per_call else None)

    def cursor(self) -> DBCursorProxy:
        cur = self._run(self._conn.cursor)
        return DBCursorProxy(cur, lock=_LOCK if self._lock_per_call else None)

    def commit(self) -> None:
        self._run(self._conn.commit)

    def rollback(self) -> None:
        self._run(self._conn.rollback)

    def close(self) -> None:
        # Shared connection lifecycle is managed by service.db.close_connection().
        logger.debug("Ignored close() call on shared DB proxy")

    @property
    def row_factory(self):
        return self._run(lambda: self._conn.row_factory)

    @row_factory.setter
    def row_factory(self, value) -> None:
        self._run(setattr, self._conn, "row_factory", value)

    @property
    def total_changes(self) -> int:
        return self._run(lambda: self._conn.total_changes)


# ---------- Pfad-Auflösung ----------


def _resolve_db_path() -> str:
    """
    Ermittelt den endgültigen DB-Pfad (eine Quelle der Wahrheit).
    Prio:
      1) DEADLOCK_DB_PATH (vollständiger Pfad)
      2) DEADLOCK_DB_DIR + DB_NAME
      3) DEFAULT_DIR + DB_NAME
    """
    p = os.environ.get(ENV_DB_PATH)
    if p:
        return str(Path(p))

    d = os.environ.get(ENV_DB_DIR) or DEFAULT_DIR
    return str(Path(d) / DB_NAME)


def _ensure_parent(path: str) -> None:
    Path(path).parent.mkdir(parents=True, exist_ok=True)


def db_path() -> str:
    """
    Liefert den tatsächlich verwendeten DB-Pfad.
    Hinweis: Der Pfad ist nach dem ersten Aufruf gecached.
    """
    global _DB_PATH_CACHED
    if _DB_PATH_CACHED:
        return _DB_PATH_CACHED
    _DB_PATH_CACHED = _resolve_db_path()
    return _DB_PATH_CACHED


# Praktischer Alias für Altcode:
DB_PATH: Path = Path(db_path())
log.debug("DB_PATH alias initialisiert: %s", DB_PATH)


# ---------- Verbindung / PRAGMA / Schema ----------


def _is_connection_alive(conn: sqlite3.Connection) -> bool:
    """
    Prüft, ob die Verbindung noch funktionsfähig ist.
    Gibt False zurück, wenn die Connection geschlossen oder korrupt ist.
    """
    try:
        conn.execute("SELECT 1").fetchone()
        return True
    except (sqlite3.ProgrammingError, sqlite3.OperationalError):
        return False


def connect() -> sqlite3.Connection:
    """
    Stellt eine einzelne, thread-safe geteilte Verbindung her.
    - Autocommit (isolation_level=None)
    - Row-Factory = sqlite3.Row
    - PRAGMAs gesetzt
    - Schema (idempotent) initialisiert
    - Auto-Recovery: Stellt bei korrupter Connection eine neue her
    """
    global _CONN, _DB_PATH_CACHED, DB_PATH
    if _CONN is not None:
        if _is_connection_alive(_CONN):
            return _CONN
        else:
            log.warning("Database connection is closed/corrupt. Reconnecting...")
            _CONN = None  # Reset to force reconnection

    path = _resolve_db_path()
    _ensure_parent(path)
    _DB_PATH_CACHED = path
    DB_PATH = Path(path)  # Alias aktualisieren
    log.debug("DB_PATH alias aktualisiert: %s", DB_PATH)

    _CONN = sqlite3.connect(
        path,
        check_same_thread=False,
        isolation_level=None,  # Autocommit
        timeout=DB_CONNECT_TIMEOUT,
    )
    _CONN.row_factory = sqlite3.Row

    with _LOCK:
        # PRAGMAs: stabil & praxiserprobt
        _CONN.executescript(
            """
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA foreign_keys=ON;
            PRAGMA wal_autocheckpoint=1000;
            PRAGMA journal_size_limit=104857600;
            PRAGMA temp_store=MEMORY;
            """
        )
        # Performance PRAGMAs (2026-02-20)
        _CONN.execute("PRAGMA cache_size=-20000")  # 20 MB page cache
        _CONN.execute("PRAGMA mmap_size=268435456")  # 256 MB memory-mapped I/O
        _CONN.execute("PRAGMA optimize")  # selektives ANALYZE für Query Planner
        _CONN.execute("PRAGMA wal_checkpoint(PASSIVE)")  # WAL-Größe reduzieren (VS Code Öffnen)

        init_schema(_CONN)

    log.info("SQLite connection established to %s", path)
    return _CONN


def connect_proxy() -> DBConnectionProxy:
    """
    Returns a guarded connection proxy for external callers.
    Each operation is serialized through the module lock.
    """
    return DBConnectionProxy(connect(), lock_per_call=True)


def is_connected() -> bool:
    """
    Gibt zurueck, ob bereits eine gemeinsame Verbindung initialisiert wurde.
    Baut keine neue Verbindung auf.
    """
    return _CONN is not None


def close_connection() -> None:
    """
    Schliesst die geteilte Verbindung und setzt den Zustand zurueck.
    Wird genutzt, um den DB-Layer als Cog neu zu laden.
    """
    global _CONN
    with _LOCK:
        if _CONN is None:
            return
        try:
            _CONN.close()
            log.info("SQLite-Verbindung geschlossen (manual reset)")
        except sqlite3.Error as e:
            log.warning("Fehler beim Schliessen der DB-Verbindung: %s", e)
        finally:
            _CONN = None


def _ensure_steam_tasks_cap_trigger(conn: sqlite3.Connection, max_rows: int) -> None:
    """
    Creates an AFTER INSERT trigger that trims steam_tasks to the newest N rows.
    Older rows (preferring finished ones) are deleted to keep the table bounded.
    """
    capped_max_rows = int(max_rows)
    if capped_max_rows <= 0:
        log.warning("Steam task cap disabled because max_rows=%s <= 0", max_rows)
        return

    conn.executemany(
        """
        INSERT INTO kv_store(ns, k, v)
        VALUES (?, ?, ?)
        ON CONFLICT(ns, k) DO UPDATE SET v = excluded.v
        """,
        [(STEAM_TASKS_KV_NS, STEAM_TASKS_KV_MAX_ROWS_KEY, str(capped_max_rows))],
    )

    conn.executescript(
        """
        CREATE TRIGGER IF NOT EXISTS trg_cap_steam_tasks
        AFTER INSERT ON steam_tasks
        BEGIN
          DELETE FROM steam_tasks
          WHERE id IN (
            SELECT id FROM steam_tasks
            WHERE status NOT IN ('PENDING','RUNNING')
            ORDER BY created_at ASC, id ASC
            LIMIT (
              SELECT CASE WHEN total > max_rows THEN total - max_rows ELSE 0 END
              FROM (
                SELECT
                  COUNT(*) AS total,
                  COALESCE(
                    (
                      SELECT CAST(v AS INTEGER)
                      FROM kv_store
                      WHERE ns = 'steam_tasks' AND k = 'max_rows'
                    ),
                    0
                  ) AS max_rows
                FROM steam_tasks
              )
            )
          );
        END;
        """
    )


def prune_steam_tasks(limit: int | None = None, *, conn: sqlite3.Connection | None = None) -> int:
    """
    Trims the steam_tasks table to the newest ``limit`` rows (defaults to STEAM_TASKS_MAX_ROWS).
    Prefers deleting finished tasks; pending/running are kept whenever possible.
    Returns the number of rows deleted.
    """
    max_rows = int(limit or STEAM_TASKS_MAX_ROWS)
    if max_rows <= 0:
        return 0

    c = conn or connect()
    with _LOCK:
        cur = c.execute(
            """
            DELETE FROM steam_tasks
            WHERE id IN (
              SELECT id FROM steam_tasks
              WHERE status NOT IN ('PENDING','RUNNING')
              ORDER BY created_at ASC, id ASC
              LIMIT (
                SELECT CASE WHEN total > ? THEN total - ? ELSE 0 END
                FROM (SELECT COUNT(*) AS total FROM steam_tasks)
              )
            )
            """,
            (max_rows, max_rows),
        )
        deleted = cur.rowcount if cur.rowcount is not None else 0

    if deleted:
        log.info("Pruned %s rows from steam_tasks (max_rows=%s)", deleted, max_rows)
    return deleted


@contextmanager
def get_conn() -> Iterator[DBConnectionProxy]:
    """Contextmanager, der die zentrale Verbindung thread-safe bereitstellt."""
    conn = connect()
    with _LOCK:
        yield DBConnectionProxy(conn, lock_per_call=False)


def init_schema(conn: sqlite3.Connection | None = None) -> None:
    """
    Legt das Schema an (idempotent). Enthält u. a.:
      - schema_version, kv_store
      - voice_stats (inkl. Migrations-Update)
      - steam_links, live_player_state
      - steam_friend_requests, steam_quick_invites, steam_tasks
    """
    c = conn or connect()
    with _LOCK:
        c.executescript(
            """
            CREATE TABLE IF NOT EXISTS schema_version(
              version INTEGER NOT NULL
            );
            INSERT INTO schema_version(version)
              SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM schema_version);

            -- generische KV-Ablage (namespaced)
            CREATE TABLE IF NOT EXISTS kv_store(
              ns TEXT NOT NULL,
              k  TEXT NOT NULL,
              v  TEXT NOT NULL,
              PRIMARY KEY(ns, k)
            );

            -- Deadlock Heroes (Admin gepflegte Stammdaten für Builds)
            CREATE TABLE IF NOT EXISTS deadlock_heroes(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              hero_id INTEGER NOT NULL,
              name TEXT NOT NULL,
              origin_build_id INTEGER,
              target_build_name_override TEXT,
              is_active INTEGER NOT NULL DEFAULT 1,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              UNIQUE(hero_id),
              UNIQUE(name)
            );

            -- Deadlock Hero Builds (n Builds pro Hero, durch Admin gepflegt)
            CREATE TABLE IF NOT EXISTS deadlock_hero_builds(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              hero_id INTEGER NOT NULL,
              build_id INTEGER NOT NULL,
              build_name TEXT NOT NULL,
              author_name TEXT NOT NULL,
              is_active INTEGER NOT NULL DEFAULT 1,
              sort_order INTEGER NOT NULL DEFAULT 100,
              sync_status TEXT,
              sync_message TEXT,
              last_checked_at INTEGER,
              last_synced_at INTEGER,
              last_alerted_at INTEGER,
              source_version INTEGER,
              source_last_updated_ts INTEGER,
              clone_build_id INTEGER,
              clone_version INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              UNIQUE(hero_id, build_id)
            );

            -- Deadlock Tierlist Settings (KV)
            CREATE TABLE IF NOT EXISTS tierlist_settings(
              k TEXT PRIMARY KEY,
              v TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            );

            -- Deadlock Tierlist Hero Meta
            CREATE TABLE IF NOT EXISTS tierlist_hero_meta(
              hero_id INTEGER PRIMARY KEY,
              description TEXT NOT NULL DEFAULT '',
              updated_at INTEGER NOT NULL
            );

            -- Deadlock Tierlist Streamers
            CREATE TABLE IF NOT EXISTS tierlist_streamers(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              hero_id INTEGER NOT NULL,
              twitch_login TEXT NOT NULL,
              display_name TEXT NOT NULL,
              sort_order INTEGER NOT NULL DEFAULT 100,
              is_active INTEGER NOT NULL DEFAULT 1,
              created_at INTEGER NOT NULL,
              UNIQUE(hero_id, twitch_login)
            );

            -- Deadlock Tierlist Snapshots
            CREATE TABLE IF NOT EXISTS tierlist_snapshots(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              bucket TEXT NOT NULL,
              patch_id TEXT NOT NULL,
              patch_unix INTEGER NOT NULL,
              fetched_at INTEGER NOT NULL,
              UNIQUE(bucket, fetched_at)
            );

            CREATE TABLE IF NOT EXISTS tierlist_snapshot_heroes(
              snapshot_id INTEGER NOT NULL,
              hero_id INTEGER NOT NULL,
              matches INTEGER NOT NULL,
              wins INTEGER NOT NULL,
              losses INTEGER NOT NULL,
              winrate REAL NOT NULL,
              PRIMARY KEY(snapshot_id, hero_id),
              FOREIGN KEY(snapshot_id) REFERENCES tierlist_snapshots(id) ON DELETE CASCADE
            );

            -- Deadlock Tierlist Build Votes
            CREATE TABLE IF NOT EXISTS tierlist_build_votes(
              build_id INTEGER PRIMARY KEY,
              upvotes INTEGER NOT NULL DEFAULT 0,
              downvotes INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_tierlist_snapshots_bucket_fetched
              ON tierlist_snapshots(bucket, fetched_at DESC);
            CREATE INDEX IF NOT EXISTS idx_tierlist_snapshot_heroes_snapshot
              ON tierlist_snapshot_heroes(snapshot_id);

            -- Voice Stats (aggregiert)
            CREATE TABLE IF NOT EXISTS voice_stats(
              user_id       INTEGER PRIMARY KEY,
              total_seconds INTEGER NOT NULL DEFAULT 0,
              total_points  INTEGER NOT NULL DEFAULT 0,
              last_update   DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            -- Voice Session Log (historisch, f\u00fcr Zeitverl\u00e4ufe)
            CREATE TABLE IF NOT EXISTS voice_session_log(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              display_name TEXT,
              guild_id INTEGER,
              channel_id INTEGER,
              channel_name TEXT,
              started_at DATETIME NOT NULL,
              ended_at DATETIME NOT NULL,
              duration_seconds INTEGER NOT NULL DEFAULT 0,
              points INTEGER NOT NULL DEFAULT 0,
              peak_users INTEGER,
              user_counts_json TEXT,
              co_player_ids TEXT
            );

            -- Voice Feedback (erste Voice-Erfahrung)
            CREATE TABLE IF NOT EXISTS voice_feedback_requests(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER,
              channel_id INTEGER,
              channel_name TEXT,
              co_player_names TEXT,
              duration_seconds INTEGER,
              request_type TEXT DEFAULT 'first',
              status TEXT,
              error_message TEXT,
              prompt_message_id INTEGER,
              sent_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            CREATE TABLE IF NOT EXISTS voice_feedback_responses(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              request_id INTEGER,
              user_id INTEGER NOT NULL,
              message_id INTEGER,
              content TEXT,
              received_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              FOREIGN KEY(request_id) REFERENCES voice_feedback_requests(id)
            );

            -- Steam-Links (mehrere Konten pro User möglich)
            CREATE TABLE IF NOT EXISTS steam_links(
              user_id    INTEGER NOT NULL,
              steam_id   TEXT    NOT NULL,
              name       TEXT,
              verified   INTEGER DEFAULT 0,
              primary_account INTEGER DEFAULT 0,
              deadlock_rank INTEGER,
              deadlock_rank_name TEXT,
              deadlock_subrank INTEGER,
              deadlock_badge_level INTEGER,
              deadlock_rank_updated_at INTEGER,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              PRIMARY KEY(user_id, steam_id)
            );

            -- Archiv-Tabelle für entfernte Steam-Links (z. B. nach Member-Leave)
            CREATE TABLE IF NOT EXISTS steam_links_archive(
              user_id    INTEGER NOT NULL,
              steam_id   TEXT    NOT NULL,
              name       TEXT,
              verified   INTEGER DEFAULT 0,
              primary_account INTEGER DEFAULT 0,
              deadlock_rank INTEGER,
              deadlock_rank_name TEXT,
              deadlock_subrank INTEGER,
              deadlock_badge_level INTEGER,
              deadlock_rank_updated_at INTEGER,
              created_at DATETIME,
              updated_at DATETIME,
              left_at    INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              guild_id   INTEGER,
              leave_reason TEXT,
              display_name TEXT,
              PRIMARY KEY(user_id, steam_id)
            );

            -- Live-Player-State (Cache der letzten Steam-API-Auswertung)
            CREATE TABLE IF NOT EXISTS live_player_state(
              steam_id TEXT PRIMARY KEY,
              last_gameid TEXT,
              last_server_id TEXT,
              last_seen_ts INTEGER,
              in_deadlock_now INTEGER DEFAULT 0,
              in_match_now_strict INTEGER DEFAULT 0,
              deadlock_stage TEXT,
              deadlock_minutes INTEGER,
              deadlock_localized TEXT,
              deadlock_hero TEXT,
              deadlock_party_hint TEXT,
              deadlock_updated_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS deadlock_voice_watch(
              steam_id TEXT PRIMARY KEY,
              guild_id INTEGER,
              channel_id INTEGER,
              updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS deadlock_party_members(
              party_id TEXT NOT NULL,
              steam_id TEXT NOT NULL,
              party_size INTEGER,
              seen_at INTEGER NOT NULL,
              PRIMARY KEY (party_id, steam_id)
            );

            -- Steam Rich Presence Cache (gefüllt vom node-steam-user Service)
            CREATE TABLE IF NOT EXISTS steam_rich_presence(
              steam_id TEXT PRIMARY KEY,
              app_id INTEGER,
              status TEXT,
              status_text TEXT,
              display TEXT,
              player_group TEXT,
              player_group_size INTEGER,
              connect TEXT,
              mode TEXT,
              map TEXT,
              party_size INTEGER,
              raw_json TEXT,
              last_update INTEGER,
              updated_at INTEGER
            );

            -- Optionale zusätzliche Watchlist für den Presence-Service
            CREATE TABLE IF NOT EXISTS steam_presence_watchlist(
              steam_id TEXT PRIMARY KEY,
              note TEXT,
              added_at INTEGER DEFAULT (strftime('%s','now'))
            );

            -- Ausgehende Freundschaftsanfragen des Steam-Bots
            CREATE TABLE IF NOT EXISTS steam_friend_requests(
              steam_id TEXT PRIMARY KEY,
              status TEXT DEFAULT 'pending',
              requested_at INTEGER DEFAULT (strftime('%s','now')),
              last_attempt INTEGER,
              attempts INTEGER DEFAULT 0,
              error TEXT
            );

            -- Polling-Status für Leave/Ban-Reconcile (restart-sicher, pro User gedrosselt)
            CREATE TABLE IF NOT EXISTS steam_cleanup_poll_state(
              user_id INTEGER PRIMARY KEY,
              last_polled_at INTEGER NOT NULL,
              last_result TEXT NOT NULL,
              last_error TEXT,
              updated_at INTEGER NOT NULL
            );

            -- Miss-Tracking für Unfollow-Reconcile (2x-Bestätigung, restart-sicher)
            CREATE TABLE IF NOT EXISTS steam_friendship_miss_tracker(
              steam_id TEXT PRIMARY KEY,
              user_id INTEGER NOT NULL,
              miss_count INTEGER NOT NULL DEFAULT 0,
              last_polled_at INTEGER NOT NULL,
              last_seen_friend_at INTEGER,
              last_miss_at INTEGER,
              last_action_at INTEGER,
              updated_at INTEGER NOT NULL
            );

            -- Persistente Queue für nachgelagerten Rollen-Entzug bei bestätigtem Unfollow
            CREATE TABLE IF NOT EXISTS steam_role_cleanup_pending(
              user_id INTEGER PRIMARY KEY,
              reason TEXT NOT NULL,
              attempts INTEGER NOT NULL DEFAULT 0,
              last_error TEXT,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            -- Vorgehaltene Steam-Quick-Invite-Links (werden vom Node-Service erzeugt)
            CREATE TABLE IF NOT EXISTS steam_quick_invites(
              token TEXT PRIMARY KEY,
              invite_link TEXT NOT NULL,
              invite_limit INTEGER DEFAULT 1,
              invite_duration INTEGER,
              created_at INTEGER NOT NULL,
              expires_at INTEGER,
              status TEXT DEFAULT 'available',
              reserved_by INTEGER,
              reserved_at INTEGER,
              last_seen INTEGER
            );

            -- Deadlock-Beta-Einladungs-Workflow (Discord ↔ Steam)
            CREATE TABLE IF NOT EXISTS steam_beta_invites(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              discord_id INTEGER NOT NULL,
              steam_id64 TEXT NOT NULL,
              account_id INTEGER,
              status TEXT NOT NULL,
              last_error TEXT,
              friend_requested_at INTEGER,
              friend_confirmed_at INTEGER,
              invite_sent_at INTEGER,
              last_notified_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              UNIQUE(discord_id),
              UNIQUE(steam_id64)
            );


            -- Intent-Gate fuer den Beta-Invite-Flow (einmalige Entscheidung)
            CREATE TABLE IF NOT EXISTS beta_invite_intent(
              discord_id INTEGER PRIMARY KEY,
              intent TEXT NOT NULL,
              decided_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              locked INTEGER NOT NULL DEFAULT 1
            );

            -- Funnel: Panel-Klicks tracken
            CREATE TABLE IF NOT EXISTS beta_invite_panel_clicks(
              discord_id INTEGER PRIMARY KEY,
              click_count INTEGER NOT NULL DEFAULT 1,
              first_clicked_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              last_clicked_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            -- Pending payments for Ko-fi tracking
            CREATE TABLE IF NOT EXISTS beta_invite_pending_payments(
              discord_id INTEGER PRIMARY KEY,
              discord_name TEXT NOT NULL,
              token TEXT UNIQUE,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            -- Private Beta-Invite-Tickets pro User (Ticket-Channel-Reuse)
            CREATE TABLE IF NOT EXISTS beta_invite_tickets(
              discord_id INTEGER PRIMARY KEY,
              guild_id INTEGER NOT NULL,
              channel_id INTEGER NOT NULL,
              status TEXT NOT NULL,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              closed_at INTEGER
            );

            -- Steuer-Tabelle für den Steam-Task-Consumer
            CREATE TABLE IF NOT EXISTS steam_tasks(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              type TEXT NOT NULL,
              payload TEXT,
              status TEXT NOT NULL DEFAULT 'PENDING',
              result TEXT,
              error TEXT,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              started_at INTEGER,
              finished_at INTEGER
            );

            -- Protokollierte Fragen & Antworten des Server-FAQ-Bots
            CREATE TABLE IF NOT EXISTS server_faq_logs(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              guild_id INTEGER,
              channel_id INTEGER,
              user_id INTEGER,
              question TEXT NOT NULL,
              answer TEXT,
              model TEXT,
              metadata TEXT
            );
            -- Standalone Bot Steuerung & Monitoring
            CREATE TABLE IF NOT EXISTS standalone_commands(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              bot TEXT NOT NULL,
              command TEXT NOT NULL,
              payload TEXT,
              status TEXT NOT NULL DEFAULT 'pending',
              result TEXT,
              error TEXT,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              started_at DATETIME,
              finished_at DATETIME
            );

            CREATE TABLE IF NOT EXISTS standalone_bot_state(
              bot TEXT PRIMARY KEY,
              heartbeat INTEGER NOT NULL,
              payload TEXT,
              updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            -- Persistent Discord UI Views (for bot restarts)
            CREATE TABLE IF NOT EXISTS persistent_views(
              message_id TEXT PRIMARY KEY,
              channel_id TEXT NOT NULL,
              guild_id TEXT NOT NULL,
              view_type TEXT NOT NULL,
              user_id TEXT,
              created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            -- User Activity Patterns (für Smart Pinging & Personalisierung)
            CREATE TABLE IF NOT EXISTS user_activity_patterns(
              user_id INTEGER PRIMARY KEY,
              typical_hours TEXT,
              typical_days TEXT,
              activity_score_2w INTEGER DEFAULT 0,
              sessions_count_2w INTEGER DEFAULT 0,
              total_minutes_2w INTEGER DEFAULT 0,
              last_active_at DATETIME,
              last_analyzed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              last_pinged_at DATETIME,
              ping_count_30d INTEGER DEFAULT 0
            );

            -- Co-Player Tracking (wer zockt mit wem)
            CREATE TABLE IF NOT EXISTS user_co_players(
              user_id INTEGER NOT NULL,
              co_player_id INTEGER NOT NULL,
              sessions_together INTEGER DEFAULT 1,
              total_minutes_together INTEGER DEFAULT 0,
              last_played_together DATETIME DEFAULT CURRENT_TIMESTAMP,
              user_display_name TEXT,
              co_player_display_name TEXT,
              PRIMARY KEY(user_id, co_player_id)
            );

            -- Member Events (Join/Leave/Ban Tracking)
            CREATE TABLE IF NOT EXISTS member_events(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              event_type TEXT NOT NULL,
              timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
              display_name TEXT,
              account_created_at DATETIME,
              join_position INTEGER,
              metadata TEXT
            );

            CREATE TABLE IF NOT EXISTS member_leave_surveys(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              left_at INTEGER NOT NULL,
              display_name TEXT,
              user_bucket TEXT NOT NULL,
              days_on_server INTEGER,
              survey_token TEXT UNIQUE NOT NULL,
              dm_status TEXT,
              reason_code TEXT,
              follow_up_question TEXT,
              follow_up_text TEXT,
              extra_text TEXT,
              responded_at INTEGER,
              web_submitted_at INTEGER,
              web_payload TEXT,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            -- Message Activity (Message Tracking pro User)
            CREATE TABLE IF NOT EXISTS message_activity(
              user_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              channel_id INTEGER,
              message_count INTEGER DEFAULT 1,
              last_message_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              first_message_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              PRIMARY KEY(user_id, guild_id)
            );

            CREATE TABLE IF NOT EXISTS text_stats(
              user_id INTEGER PRIMARY KEY,
              total_messages INTEGER NOT NULL DEFAULT 0,
              total_points INTEGER NOT NULL DEFAULT 0,
              last_update DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS text_conversation_log(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER,
              channel_id INTEGER,
              started_at DATETIME NOT NULL,
              ended_at DATETIME NOT NULL,
              message_count INTEGER NOT NULL DEFAULT 0,
              points INTEGER NOT NULL DEFAULT 0,
              co_participant_ids TEXT,
              had_interaction INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS user_tags(
              user_id INTEGER NOT NULL,
              tag_key TEXT NOT NULL,
              tag_value TEXT NOT NULL,
              set_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
              PRIMARY KEY(user_id, tag_key)
            );

            CREATE TABLE IF NOT EXISTS user_mod_tags(
              user_id INTEGER NOT NULL,
              mod_tag TEXT NOT NULL,
              set_by INTEGER NOT NULL,
              reason TEXT,
              set_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
              expires_at TIMESTAMP,
              PRIMARY KEY(user_id, mod_tag)
            );

            CREATE TABLE IF NOT EXISTS tempvoice_lane_tag_filter(
              channel_id INTEGER PRIMARY KEY,
              min_age_tag TEXT,
              required_tone_tag TEXT,
              deny_ragebaiter INTEGER DEFAULT 0,
              updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            -- Datenschutz-/Opt-out-Status je User
            CREATE TABLE IF NOT EXISTS user_privacy(
              user_id    INTEGER PRIMARY KEY,
              opted_out  INTEGER NOT NULL DEFAULT 0,
              deleted_at INTEGER,
              reason     TEXT,
              updated_at INTEGER DEFAULT (strftime('%s','now'))
            );

            -- Bug/Problem Reports mit automatischer Codex-Bearbeitung
            CREATE TABLE IF NOT EXISTS issue_reports(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER,
              guild_id INTEGER,
              channel_id INTEGER,
              message_id INTEGER,
              category TEXT,
              title TEXT,
              description TEXT NOT NULL,
              status TEXT NOT NULL DEFAULT 'pending',
              ai_response TEXT,
              ai_model TEXT,
              ai_error TEXT,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              answered_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS oauth_states (
              state TEXT PRIMARY KEY,
              provider TEXT NOT NULL,
              flow_type TEXT NOT NULL,
              requesting_service TEXT,
              redirect_after TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              expires_at INTEGER NOT NULL,
              used INTEGER DEFAULT 0,
              metadata TEXT
            );

            -- ========== COACHING SYSTEM ==========

            -- Coaches (Bot-seitig mit discord_user_id für Rollen)
            CREATE TABLE IF NOT EXISTS coaches (
              id TEXT PRIMARY KEY,
              discord_user_id INTEGER UNIQUE NOT NULL,
              discord_username TEXT,
              display_name TEXT,
              avatar_url TEXT,
              bio TEXT,
              specialties_json TEXT DEFAULT '[]',
              availability_json TEXT DEFAULT '{}',
              status TEXT DEFAULT 'pending',
              website_coach_id TEXT,
              avg_rating REAL,
              total_reviews INTEGER DEFAULT 0,
              total_sessions INTEGER DEFAULT 0,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            -- Coaching Requests (Pre-Coaching gesammelte Daten)
            CREATE TABLE IF NOT EXISTS coaching_requests (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              discord_user_id INTEGER NOT NULL,
              discord_username TEXT,
              rank TEXT NOT NULL,
              subrank TEXT NOT NULL,
              hero TEXT,
              games_played TEXT,
              hours_played TEXT,
              availability TEXT,
              scheduled_slot TEXT,
              current_problems TEXT,
              ai_summary TEXT,
              ai_insights_json TEXT,
              status TEXT DEFAULT 'pending',
              message_id INTEGER,
              channel_id INTEGER,
              role_assigned_at INTEGER,
              role_expires_at INTEGER,
              role_removed_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            -- Coaching Bans (7 Tage Sperre)
            CREATE TABLE IF NOT EXISTS coaching_bans (
              discord_user_id INTEGER PRIMARY KEY,
              banned_at INTEGER NOT NULL,
              expires_at INTEGER NOT NULL,
              reason TEXT
            );

            -- Coaching Sessions (Aktive Sessions)
            CREATE TABLE IF NOT EXISTS coaching_sessions (
              id TEXT PRIMARY KEY,
              request_id INTEGER,
              coach_id TEXT,
              discord_user_id INTEGER NOT NULL,
              discord_username TEXT,
              discord_channel_id INTEGER,
              discord_thread_id INTEGER,
              status TEXT DEFAULT 'active',
              role_assigned_at INTEGER,
              role_reminder_at INTEGER,
              role_expires_at INTEGER,
              reward_role_expires_at INTEGER,
              reward_role_removed_at INTEGER,
              voice_channel_id INTEGER,
              voice_started_at INTEGER,
              voice_last_seen_at INTEGER,
              survey_sent_at INTEGER,
              scheduled_at INTEGER,
              started_at INTEGER DEFAULT (strftime('%s','now')),
              completed_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            -- Coaching Surveys (Post-Coaching Feedback)
            CREATE TABLE IF NOT EXISTS coaching_surveys (
              id TEXT PRIMARY KEY,
              session_id TEXT UNIQUE,
              rating INTEGER CHECK(rating >= 0 AND rating <= 10),
              feedback_text TEXT,
              improved_areas TEXT,
              unresolved_items TEXT,
              would_recommend INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            -- Coach Applications
            CREATE TABLE IF NOT EXISTS coach_applications (
              id TEXT PRIMARY KEY,
              discord_user_id INTEGER UNIQUE NOT NULL,
              discord_username TEXT,
              display_name TEXT,
              application_text TEXT,
              experience_text TEXT,
              rank TEXT,
              specialties_json TEXT DEFAULT '[]',
              availability_json TEXT DEFAULT '{}',
              status TEXT DEFAULT 'pending',
              reviewed_by INTEGER,
              reviewed_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

            """
        )
        # Steam-Task-Retention: Trigger + initial Cleanup
        _ensure_steam_tasks_cap_trigger(c, STEAM_TASKS_MAX_ROWS)
        prune_steam_tasks(conn=c, limit=STEAM_TASKS_MAX_ROWS)
        _migrate_legacy_coaching_sessions(c)
        # Nachträglich hinzugefügte Spalten idempotent sicherstellen
        try:
            c.execute("ALTER TABLE voice_stats ADD COLUMN total_points INTEGER NOT NULL DEFAULT 0")
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise
        try:
            c.execute(
                "ALTER TABLE voice_feedback_requests ADD COLUMN request_type TEXT DEFAULT 'first'"
            )
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise
        try:
            c.execute("ALTER TABLE voice_session_log ADD COLUMN display_name TEXT")
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise
        try:
            c.execute("ALTER TABLE voice_session_log ADD COLUMN co_player_ids TEXT")
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise
        for alter_sql in (
            "ALTER TABLE user_co_players ADD COLUMN user_display_name TEXT",
            "ALTER TABLE user_co_players ADD COLUMN co_player_display_name TEXT",
        ):
            try:
                c.execute(alter_sql)
            except sqlite3.OperationalError as exc:
                if "duplicate column name" not in str(exc).lower():
                    raise
        # Live player state extensions
        for alter_sql in (
            "ALTER TABLE live_player_state ADD COLUMN deadlock_stage TEXT",
            "ALTER TABLE live_player_state ADD COLUMN deadlock_minutes INTEGER",
            "ALTER TABLE live_player_state ADD COLUMN deadlock_localized TEXT",
            "ALTER TABLE live_player_state ADD COLUMN deadlock_hero TEXT",
            "ALTER TABLE live_player_state ADD COLUMN deadlock_party_hint TEXT",
            "ALTER TABLE live_player_state ADD COLUMN deadlock_updated_at INTEGER",
        ):
            try:
                c.execute(alter_sql)
            except sqlite3.OperationalError as exc:
                if "duplicate column name" not in str(exc).lower():
                    raise
        # Steam friend tracking: is_steam_friend=1 bedeutet der Bot ist aktuell Steam-Freund
        try:
            c.execute("ALTER TABLE steam_links ADD COLUMN is_steam_friend INTEGER DEFAULT 0")
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise
        # Steam link rank extensions (Deadlock rank snapshot from GC profile cards)
        for alter_sql in (
            "ALTER TABLE steam_links ADD COLUMN deadlock_rank INTEGER",
            "ALTER TABLE steam_links ADD COLUMN deadlock_rank_name TEXT",
            "ALTER TABLE steam_links ADD COLUMN deadlock_subrank INTEGER",
            "ALTER TABLE steam_links ADD COLUMN deadlock_badge_level INTEGER",
            "ALTER TABLE steam_links ADD COLUMN deadlock_rank_updated_at INTEGER",
        ):
            try:
                c.execute(alter_sql)
            except sqlite3.OperationalError as exc:
                if "duplicate column name" not in str(exc).lower():
                    raise
        for alter_sql in (
            "ALTER TABLE deadlock_hero_builds ADD COLUMN sync_status TEXT",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN sync_message TEXT",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN last_checked_at INTEGER",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN last_synced_at INTEGER",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN last_alerted_at INTEGER",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN source_version INTEGER",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN source_last_updated_ts INTEGER",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN clone_build_id INTEGER",
            "ALTER TABLE deadlock_hero_builds ADD COLUMN clone_version INTEGER",
        ):
            try:
                c.execute(alter_sql)
            except sqlite3.OperationalError as exc:
                if "duplicate column name" not in str(exc).lower():
                    raise
        try:
            c.execute("ALTER TABLE deadlock_heroes ADD COLUMN target_build_name_override TEXT")
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise
        for alter_sql in (
            "ALTER TABLE coaching_requests ADD COLUMN role_assigned_at INTEGER",
            "ALTER TABLE coaching_requests ADD COLUMN role_expires_at INTEGER",
            "ALTER TABLE coaching_requests ADD COLUMN role_removed_at INTEGER",
            "ALTER TABLE coaching_requests ADD COLUMN scheduled_slot TEXT",
            "ALTER TABLE coaching_sessions ADD COLUMN voice_channel_id INTEGER",
            "ALTER TABLE coaching_sessions ADD COLUMN voice_started_at INTEGER",
            "ALTER TABLE coaching_sessions ADD COLUMN voice_last_seen_at INTEGER",
            "ALTER TABLE coaching_sessions ADD COLUMN survey_sent_at INTEGER",
            "ALTER TABLE coaching_sessions ADD COLUMN reward_role_expires_at INTEGER",
            "ALTER TABLE coaching_sessions ADD COLUMN reward_role_removed_at INTEGER",
        ):
            try:
                c.execute(alter_sql)
            except sqlite3.OperationalError as exc:
                if "duplicate column name" not in str(exc).lower():
                    raise
        for alter_sql in (
            "ALTER TABLE coaches ADD COLUMN avg_rating REAL",
            "ALTER TABLE coaches ADD COLUMN total_reviews INTEGER DEFAULT 0",
            "ALTER TABLE coaches ADD COLUMN total_sessions INTEGER DEFAULT 0",
        ):
            try:
                c.execute(alter_sql)
            except sqlite3.OperationalError as exc:
                if "duplicate column name" not in str(exc).lower():
                    raise
        # Ko-fi pending payments: token Spalte hinzufügen
        try:
            c.execute("ALTER TABLE beta_invite_pending_payments ADD COLUMN token TEXT")
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise
        try:
            c.execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_payments_token ON beta_invite_pending_payments(token)"
            )
        except sqlite3.OperationalError:
            log.debug("Skipping pending-payments index creation due to SQLite operational error")
        # Indizes ergänzen (idempotent)
        try:
            c.execute("CREATE INDEX IF NOT EXISTS idx_steam_links_user  ON steam_links(user_id)")
            c.execute("CREATE INDEX IF NOT EXISTS idx_steam_links_steam ON steam_links(steam_id)")
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_quick_invites_status ON steam_quick_invites(status, expires_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_quick_invites_reserved ON steam_quick_invites(reserved_by)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_beta_invites_status ON steam_beta_invites(status)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_beta_invites_account ON steam_beta_invites(account_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_beta_invite_tickets_channel ON beta_invite_tickets(channel_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_tasks_status ON steam_tasks(status, id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_tasks_updated ON steam_tasks(updated_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_standalone_commands_status ON standalone_commands(bot, status, id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_standalone_commands_created ON standalone_commands(created_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_standalone_state_updated ON standalone_bot_state(updated_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_log_started ON voice_session_log(started_at)"
            )
            c.execute("CREATE INDEX IF NOT EXISTS idx_voice_log_user ON voice_session_log(user_id)")
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_log_guild ON voice_session_log(guild_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_log_display_name ON voice_session_log(display_name)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_fb_req_user ON voice_feedback_requests(user_id, sent_at_ts)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_fb_req_status ON voice_feedback_requests(status, sent_at_ts)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_fb_req_type ON voice_feedback_requests(request_type, sent_at_ts)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_fb_resp_req ON voice_feedback_responses(request_id)"
            )

            # Performance-Indizes für häufige Queries
            # Leaderboard-Query: ORDER BY total_points DESC, total_seconds DESC
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_stats_leaderboard ON voice_stats(total_points DESC, total_seconds DESC)"
            )
            # User Stats Lookup mit allen Feldern (covering index)
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_stats_user_lookup ON voice_stats(user_id, total_seconds, total_points)"
            )
            # TempVoice Rehydration: WHERE guild_id=? (composite index)
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_tempvoice_lanes_guild ON tempvoice_lanes(guild_id, channel_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_deadlock_party_members_party ON deadlock_party_members(party_id, seen_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_deadlock_party_members_steam ON deadlock_party_members(steam_id, seen_at)"
            )
            # Activity Patterns: Schnelle Lookups für Smart Pinging
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_activity_patterns_score ON user_activity_patterns(activity_score_2w DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_activity_patterns_last_active ON user_activity_patterns(last_active_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_activity_patterns_last_pinged ON user_activity_patterns(last_pinged_at)"
            )
            # Co-Players: Bi-direktionale Lookups
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_co_players_user ON user_co_players(user_id, sessions_together DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_co_players_co_player ON user_co_players(co_player_id)"
            )
            # Member Events: Schnelle User-Lookups & Event-Type Filtering
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_member_events_user ON member_events(user_id, timestamp DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_member_events_guild ON member_events(guild_id, event_type, timestamp DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_member_events_type ON member_events(event_type, timestamp DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_leave_surveys_token ON member_leave_surveys(survey_token)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_leave_surveys_bucket ON member_leave_surveys(guild_id, user_bucket)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_leave_surveys_user ON member_leave_surveys(user_id, created_at DESC)"
            )
            # Message Activity: User & Guild Lookups
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_message_activity_user ON message_activity(user_id, message_count DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_message_activity_guild ON message_activity(guild_id, message_count DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_text_stats_leaderboard ON text_stats(total_points DESC, total_messages DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_text_log_user ON text_conversation_log(user_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_text_log_started ON text_conversation_log(started_at, user_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_user_mod_tags_active ON user_mod_tags(user_id, expires_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_user_privacy_opted ON user_privacy(opted_out)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_issue_reports_status ON issue_reports(status, created_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_issue_reports_user ON issue_reports(user_id, created_at DESC)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_oauth_states_provider_flow ON oauth_states(provider, flow_type, used)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_oauth_states_expiry ON oauth_states(expires_at, used)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_deadlock_heroes_hero ON deadlock_heroes(hero_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_deadlock_heroes_active ON deadlock_heroes(is_active, hero_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_deadlock_hero_builds_hero_active_sort ON deadlock_hero_builds(hero_id, is_active, sort_order)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_deadlock_hero_builds_build_id ON deadlock_hero_builds(build_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_deadlock_hero_builds_sync_status ON deadlock_hero_builds(sync_status, last_alerted_at)"
            )
            # Coaching Indizes
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_requests_status ON coaching_requests(status, created_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_requests_user ON coaching_requests(discord_user_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_requests_expires ON coaching_requests(role_expires_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_sessions_coach ON coaching_sessions(coach_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_sessions_user ON coaching_sessions(discord_user_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_sessions_status ON coaching_sessions(status)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_sessions_expires ON coaching_sessions(role_expires_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_sessions_voice ON coaching_sessions(voice_channel_id, survey_sent_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coaching_surveys_session ON coaching_surveys(session_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_coach_applications_status ON coach_applications(status)"
            )
            c.execute("CREATE INDEX IF NOT EXISTS idx_coaches_discord ON coaches(discord_user_id)")
            # Performance Indexes (2026-02-20)
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_links_verified ON steam_links(verified, user_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_links_friend ON steam_links(is_steam_friend, user_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_links_archive_user ON steam_links_archive(user_id, left_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_links_archive_steam ON steam_links_archive(steam_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_cleanup_poll_state_polled ON steam_cleanup_poll_state(last_polled_at, updated_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_friendship_miss_tracker_user ON steam_friendship_miss_tracker(user_id, last_polled_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_friendship_miss_tracker_polled ON steam_friendship_miss_tracker(last_polled_at, miss_count)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_steam_role_cleanup_pending_updated ON steam_role_cleanup_pending(updated_at)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_log_started_user ON voice_session_log(started_at, user_id)"
            )
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_voice_feedback_req_pending ON voice_feedback_requests(status, sent_at_ts) WHERE status = 'pending'"
            )
        except sqlite3.Error as e:
            logger.debug("Optionale Index-Erstellung übersprungen: %s", e, exc_info=True)
        # hero_build_clones index (table created by build_mirror cog, may not exist yet)
        try:
            c.execute(
                "CREATE INDEX IF NOT EXISTS idx_hero_build_clones_status_hero ON hero_build_clones(status, hero_id, created_at)"
            )
            c.commit()
        except sqlite3.Error as e:
            logger.debug(
                "Index-Erstellung für hero_build_clones übersprungen: %s", e, exc_info=True
            )
        # Update query planner statistics after index creation
        try:
            c.execute("ANALYZE")
            c.commit()
        except sqlite3.Error as e:
            logger.debug("ANALYZE übersprungen: %s", e, exc_info=True)


# ---------- Low-Level Helpers (sicher, mit Bind-Parametern) ----------


def _in_transaction_context() -> bool:
    """Prüft, ob der aktuelle Task in einem db.transaction()-Block läuft."""
    try:
        return _TX_DEPTH.get() > 0
    except LookupError:
        return False


def execute(sql: str, params: Iterable[Any] = ()) -> None:
    with _LOCK:
        connect().execute(sql, params)


def executemany(sql: str, seq_of_params: Iterable[Iterable[Any]]) -> None:
    with _LOCK:
        connect().executemany(sql, seq_of_params)


def query_one(sql: str, params: Iterable[Any] = ()):  # -> sqlite3.Row | None
    with _LOCK:
        cur = connect().execute(sql, params)
        try:
            return cur.fetchone()
        finally:
            cur.close()


def query_all(sql: str, params: Iterable[Any] = ()):  # -> list[sqlite3.Row]
    with _LOCK:
        cur = connect().execute(sql, params)
        try:
            return cur.fetchall()
        finally:
            cur.close()


async def execute_async(sql: str, params: Iterable[Any] = ()) -> None:
    """
    Async Wrapper f�r execute(); nutzt Thread-Offloading au�erhalb von Transaktionen.
    """
    if _in_transaction_context():
        execute(sql, params)
    else:
        await asyncio.to_thread(execute, sql, params)
    return None


async def executemany_async(sql: str, seq_of_params: Iterable[Iterable[Any]]) -> None:
    """Async Wrapper f�r executemany(); thread-offloaded au�erhalb von Transaktionen."""
    if _in_transaction_context():
        executemany(sql, seq_of_params)
    else:
        await asyncio.to_thread(executemany, sql, seq_of_params)
    return None


async def query_one_async(sql: str, params: Iterable[Any] = ()):
    """Async Wrapper f�r query_one(); thread-offloaded au�erhalb von Transaktionen."""
    if _in_transaction_context():
        return query_one(sql, params)
    return await asyncio.to_thread(query_one, sql, params)


async def query_all_async(sql: str, params: Iterable[Any] = ()):
    """Async Wrapper f�r query_all(); thread-offloaded au�erhalb von Transaktionen."""
    if _in_transaction_context():
        return query_all(sql, params)
    return await asyncio.to_thread(query_all, sql, params)


# ---------- KV (namespaced) ----------


def set_kv(ns: str, k: str, v: str) -> None:
    execute(
        """
        INSERT INTO kv_store(ns,k,v) VALUES(?,?,?)
        ON CONFLICT(ns,k) DO UPDATE SET v=excluded.v
        """,
        (ns, k, v),
    )


def get_kv(ns: str, k: str) -> str | None:
    row = query_one("SELECT v FROM kv_store WHERE ns=? AND k=?", (ns, k))
    return row[0] if row else None


def list_kv(ns: str) -> list[tuple[str, str]]:
    rows = query_all("SELECT k, v FROM kv_store WHERE ns=? ORDER BY k ASC", (ns,))
    return [(str(row[0]), str(row[1])) for row in rows]


def _encode_oauth_state_metadata(metadata: Any) -> str | None:
    if metadata is None:
        return None
    if isinstance(metadata, str):
        text = metadata.strip()
        return text or None
    return json.dumps(metadata, ensure_ascii=False, separators=(",", ":"))


def _decode_oauth_state_metadata(raw: Any) -> Any:
    if raw is None:
        return None
    if isinstance(raw, (bytes, bytearray)):
        try:
            raw = raw.decode("utf-8", errors="ignore")
        except Exception:
            return None
    if not isinstance(raw, str):
        return raw
    text = raw.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return text


def create_state(
    state: str,
    provider: str,
    flow_type: str,
    redirect_after: str,
    metadata: Any = None,
    *,
    requesting_service: str | None = None,
    ttl_seconds: int | None = None,
) -> dict[str, Any] | None:
    safe_state = str(state or "").strip()
    safe_provider = str(provider or "").strip()
    safe_flow_type = str(flow_type or "").strip()
    safe_redirect_after = str(redirect_after or "").strip()
    if not safe_state:
        raise ValueError("state must not be empty")
    if not safe_provider:
        raise ValueError("provider must not be empty")
    if not safe_flow_type:
        raise ValueError("flow_type must not be empty")
    if not safe_redirect_after:
        raise ValueError("redirect_after must not be empty")

    metadata_payload = _encode_oauth_state_metadata(metadata)
    derived_requesting_service = str(requesting_service or "").strip() or None
    if derived_requesting_service is None and isinstance(metadata, dict):
        candidate = str(metadata.get("requesting_service") or "").strip()
        derived_requesting_service = candidate or None

    now = int(time.time())
    ttl = max(1, int(ttl_seconds or DEFAULT_OAUTH_STATE_TTL_SECONDS))
    expires_at = now + ttl

    execute(
        """
        INSERT INTO oauth_states(
          state,
          provider,
          flow_type,
          requesting_service,
          redirect_after,
          created_at,
          expires_at,
          used,
          metadata
        ) VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?)
        ON CONFLICT(state) DO UPDATE SET
          provider = excluded.provider,
          flow_type = excluded.flow_type,
          requesting_service = excluded.requesting_service,
          redirect_after = excluded.redirect_after,
          created_at = excluded.created_at,
          expires_at = excluded.expires_at,
          used = 0,
          metadata = excluded.metadata
        """,
        (
            safe_state,
            safe_provider,
            safe_flow_type,
            derived_requesting_service,
            safe_redirect_after,
            now,
            expires_at,
            metadata_payload,
        ),
    )
    return validate_state(safe_state, now=now)


def validate_state(state: str, *, now: int | None = None) -> dict[str, Any] | None:
    safe_state = str(state or "").strip()
    if not safe_state:
        return None

    current = int(time.time() if now is None else now)
    row = query_one(
        """
        SELECT
          state,
          provider,
          flow_type,
          requesting_service,
          redirect_after,
          created_at,
          expires_at,
          used,
          metadata
        FROM oauth_states
        WHERE state = ?
        """,
        (safe_state,),
    )
    if not row:
        return None
    if int(row["used"] or 0) != 0:
        return None
    if int(row["expires_at"] or 0) <= current:
        return None

    return {
        "state": str(row["state"] or ""),
        "provider": str(row["provider"] or ""),
        "flow_type": str(row["flow_type"] or ""),
        "requesting_service": row["requesting_service"],
        "redirect_after": str(row["redirect_after"] or ""),
        "created_at": int(row["created_at"] or 0),
        "expires_at": int(row["expires_at"] or 0),
        "used": bool(row["used"]),
        "metadata": _decode_oauth_state_metadata(row["metadata"]),
    }


def consume_state(state: str, *, now: int | None = None) -> bool:
    safe_state = str(state or "").strip()
    if not safe_state:
        return False

    current = int(time.time() if now is None else now)
    with _LOCK:
        cur = connect().execute(
            """
            UPDATE oauth_states
            SET used = 1
            WHERE state = ?
              AND COALESCE(used, 0) = 0
              AND expires_at > ?
            """,
            (safe_state, current),
        )
        try:
            return bool(cur.rowcount and cur.rowcount > 0)
        finally:
            cur.close()


# ---------- Transactions (async) ----------


@asynccontextmanager
async def transaction() -> AsyncIterator[DBConnectionProxy]:
    """
    Einfache async Transaction (BEGIN/COMMIT/ROLLBACK) auf der gemeinsamen Verbindung.
    - serialisiert den Zugriff �ber _LOCK
    - verschachtelte Transaktionen werden auf die �u�erste zusammengefasst
    """
    depth = _TX_DEPTH.get()
    token = _TX_DEPTH.set(depth + 1)
    outermost = depth == 0

    if outermost:
        await _ASYNC_LOCK.acquire()
        _LOCK.acquire()
        try:
            conn = connect()
            conn.execute("BEGIN;")
        except Exception:
            _LOCK.release()
            _ASYNC_LOCK.release()
            raise

    try:
        yield DBConnectionProxy(connect(), lock_per_call=False)
        if outermost:
            connect().execute("COMMIT;")
    except Exception:
        if outermost:
            try:
                connect().execute("ROLLBACK;")
            except Exception as exc:  # pragma: no cover - nur Logging
                logger.error("DB rollback failed: %s", exc, exc_info=True)
            finally:
                _LOCK.release()
                _ASYNC_LOCK.release()
        raise
    else:
        if outermost:
            _LOCK.release()
            _ASYNC_LOCK.release()
    finally:
        _TX_DEPTH.reset(token)


# ---------- Pflege ----------


# Hinweis: Kein automatisches VACUUM mehr bei Prozess-Beendigung.
# Mehrere laufende Bots teilen sich die DB; ein exklusives VACUUM im atexit
# führte zu „database is locked“ Fehlern in parallel laufenden Prozessen.
# Stattdessen sollte Wartung (VACUUM/REINDEX) manuell oder in dedizierten
# Maintenance-Fenstern erfolgen, wenn keine anderen Writer aktiv sind.
