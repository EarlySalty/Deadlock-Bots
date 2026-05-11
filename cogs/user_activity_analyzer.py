"""
User Activity Analyzer & Smart Pinging System

Features:
- Analysiert User-Aktivität der letzten 2 Wochen
- Erkennt typische Online-Zeiten (Uhrzeiten & Wochentage)
- Trackt wer mit wem zusammen spielt
- Smart Pinging mit KI-generierten, menschlich klingenden Nachrichten
- Rate-Limiting zum Schutz vor Spam
"""

from __future__ import annotations

import asyncio
import json
import logging
import math
import time
from collections import defaultdict
from datetime import datetime, timedelta
from typing import Any

import discord
from discord.ext import commands, tasks

from cogs import privacy_core as privacy
from service import db as central_db

logger = logging.getLogger(__name__)
TEXT_SESSION_WINDOW_SECONDS = 600


def _safe_log_value(value: Any) -> str:
    """Sanitize values before logging to prevent log injection attacks."""
    text = "" if value is None else str(value)
    return text.replace("\r", "\\r").replace("\n", "\\n")


def _format_leaderboard_number(value: int) -> str:
    return f"{int(value):,}".replace(",", ".")


class UserActivityAnalyzer(commands.Cog):
    """
    Analysiert User-Aktivität und bietet Smart-Pinging mit personalisierten Nachrichten.
    """

    def __init__(self, bot):
        self.bot = bot

        # Cache für Co-Spieler-Daten (wird alle 30 Min refreshed)
        self._co_player_cache: dict[int, list[tuple[int, int]]] = {}
        self._cache_timestamp = datetime.utcnow()
        # Namens-Cache für Co-Player, damit Display-Namen auch nach Neustarts verfügbar bleiben
        self._display_name_cache: dict[int, tuple[str, float]] = {}
        self._display_name_cache_ttl = 3600.0  # Sekunden
        # Invite-Snapshots pro Guild zur Join-Quellen-Erkennung
        self._join_invite_snapshot: dict[int, dict[str, dict[str, Any]]] = {}
        self._join_vanity_snapshot: dict[int, dict[str, Any]] = {}
        self._join_source_locks: dict[int, asyncio.Lock] = {}
        self._invite_warmup_task: asyncio.Task | None = None
        self._twitch_invite_table_available: bool | None = None
        self._open_text_sessions: dict[tuple[int, int], dict[str, Any]] = {}

        logger.info("User Activity Analyzer initializing")

    async def cog_load(self):
        """Startet Background-Tasks für Analyse und Tracking."""
        # Starte Background-Tasks
        self.analyze_user_activity.start()
        self.track_co_players_realtime.start()
        self.cleanup_old_pings.start()
        self.flush_text_sessions.start()

        logger.info("User Activity Analyzer loaded - Background tasks started")

        # Initialisiere neue DB-Tabellen (falls nicht vorhanden)
        self._ensure_new_tables()
        self._invite_warmup_task = asyncio.create_task(
            self._prime_join_source_cache(),
            name="user-activity.join-source-prime",
        )
        asyncio.create_task(
            self._backfill_member_events(),
            name="user-activity.member-events-backfill",
        )

    async def cog_unload(self):
        """Stoppt Background-Tasks sauber."""
        if self._invite_warmup_task and not self._invite_warmup_task.done():
            self._invite_warmup_task.cancel()
            await asyncio.gather(self._invite_warmup_task, return_exceptions=True)

        tasks_to_cancel = [
            self.analyze_user_activity,
            self.track_co_players_realtime,
            self.cleanup_old_pings,
            self.flush_text_sessions,
        ]
        for task in tasks_to_cancel:
            if task.is_running():
                task.cancel()

        await asyncio.gather(
            *[
                task.wait_for_cancel() if hasattr(task, "wait_for_cancel") else asyncio.sleep(0)
                for task in tasks_to_cancel
                if task.is_running()
            ],
            return_exceptions=True,
        )

        await self._flush_all_text_sessions()

        logger.info("User Activity Analyzer unloaded")

    # ========== ACTIVITY ANALYSIS ==========

    @tasks.loop(hours=6)
    async def analyze_user_activity(self):
        """
        Analysiert alle User-Aktivitäten der letzten 2 Wochen.
        Läuft alle 6 Stunden.
        """
        try:
            logger.info("Starting activity analysis for last 2 weeks...")

            # Cutoff: 2 Wochen zurück
            cutoff = datetime.utcnow() - timedelta(days=14)
            cutoff_str = cutoff.strftime("%Y-%m-%d %H:%M:%S")

            # Hole alle Sessions der letzten 2 Wochen
            sessions = central_db.query_all(
                """
                SELECT user_id, started_at, ended_at, duration_seconds, channel_id
                FROM voice_session_log
                WHERE started_at >= ?
                ORDER BY user_id, started_at
                """,
                (cutoff_str,),
            )

            if not sessions:
                logger.info("No sessions found in last 2 weeks")
                return

            # Gruppiere Sessions nach User
            user_sessions = defaultdict(list)
            for row in sessions:
                user_sessions[row[0]].append(
                    {
                        "user_id": row[0],
                        "started_at": datetime.strptime(row[1], "%Y-%m-%d %H:%M:%S")
                        if row[1]
                        else None,
                        "ended_at": datetime.strptime(row[2], "%Y-%m-%d %H:%M:%S")
                        if row[2]
                        else None,
                        "duration_seconds": row[3] or 0,
                        "channel_id": row[4],
                    }
                )

            # Analysiere jeden User
            analyzed_count = 0
            for user_id, user_session_list in user_sessions.items():
                await self._analyze_single_user(user_id, user_session_list)
                analyzed_count += 1

            logger.info(f"Activity analysis completed: {analyzed_count} users analyzed")

        except Exception as e:
            logger.error(f"Error in activity analysis: {e}", exc_info=True)

    @analyze_user_activity.before_loop
    async def before_analyze(self):
        await self.bot.wait_until_ready()

    def _display_name_from_db(self, user_id: int) -> str | None:
        """
        Liefert den zuletzt bekannten Anzeigenamen aus der DB (falls vorhanden).
        Nutzt co-player- sowie voice_session_log-Daten als Fallback.
        """
        try:
            row = central_db.query_one(
                """
                SELECT name FROM (
                  SELECT user_display_name AS name, last_played_together AS ts
                  FROM user_co_players
                  WHERE user_id = ? AND user_display_name IS NOT NULL
                  UNION ALL
                  SELECT co_player_display_name AS name, last_played_together AS ts
                  FROM user_co_players
                  WHERE co_player_id = ? AND co_player_display_name IS NOT NULL
                  UNION ALL
                  SELECT display_name AS name, ended_at AS ts
                  FROM voice_session_log
                  WHERE user_id = ? AND display_name IS NOT NULL
                )
                WHERE name IS NOT NULL AND name != ''
                ORDER BY ts DESC
                LIMIT 1
                """,
                (user_id, user_id, user_id),
            )
            if row and row[0]:
                return str(row[0])
        except Exception as exc:
            logger.debug("Display name lookup failed for %s: %s", user_id, exc, exc_info=True)
        return None

    async def _display_name_for(self, user_id: int) -> str:
        """Resolve a display name and persist it in a short-lived cache."""
        now = time.time()
        cached = self._display_name_cache.get(user_id)
        if cached and now - cached[1] < self._display_name_cache_ttl:
            return cached[0]

        name = self._display_name_from_db(user_id)

        if not name:
            user_obj = self.bot.get_user(user_id)
            if user_obj:
                name = getattr(user_obj, "display_name", None) or getattr(user_obj, "name", None)

        if not name:
            try:
                user_obj = await self.bot.fetch_user(user_id)
                name = getattr(user_obj, "display_name", None) or getattr(user_obj, "name", None)
            except (discord.HTTPException, discord.NotFound, discord.Forbidden):
                name = None

        resolved = name or f"User {user_id}"
        self._display_name_cache[user_id] = (resolved, now)

        if len(self._display_name_cache) > 512:
            oldest = sorted(self._display_name_cache.items(), key=lambda item: item[1][1])[:128]
            for uid, _ in oldest:
                self._display_name_cache.pop(uid, None)

        return resolved

    async def _analyze_single_user(self, user_id: int, sessions: list[dict]):
        """Analysiert einen einzelnen User und speichert die Patterns."""
        try:
            if not sessions:
                return

            # === Zeitfenster-Analyse ===
            hour_counts = defaultdict(int)  # Stunde -> Anzahl Sessions
            day_counts = defaultdict(int)  # Wochentag (0=Mo, 6=So) -> Anzahl Sessions

            total_minutes = 0
            last_active = None

            for session in sessions:
                started_at = session.get("started_at")
                duration_seconds = session.get("duration_seconds") or 0
                duration_minutes = duration_seconds // 60

                if started_at:
                    # Stunde (0-23)
                    hour = started_at.hour
                    hour_counts[hour] += 1

                    # Wochentag (0=Mo, 6=So)
                    weekday = started_at.weekday()
                    day_counts[weekday] += 1

                    # Last Active
                    if last_active is None or started_at > last_active:
                        last_active = started_at

                # Total Minutes
                total_minutes += duration_minutes

                # Co-Spieler aus dieser Session (falls vorhanden)
                # Wird vom voice_activity_tracker als JSON-Liste gespeichert
                # TODO: Diese Daten werden erst nach dem nächsten Reload verfügbar sein

            # === Top 3 häufigste Stunden ===
            top_hours = sorted(hour_counts.items(), key=lambda x: x[1], reverse=True)[:3]
            typical_hours = [h for h, _ in top_hours]

            # === Top 3 häufigste Wochentage ===
            top_days = sorted(day_counts.items(), key=lambda x: x[1], reverse=True)[:3]
            typical_days = [d for d, _ in top_days]

            # === Activity Score (Anzahl Sessions in 2W) ===
            activity_score = len(sessions)

            # === Speichere in DB ===
            central_db.execute(
                """
                INSERT INTO user_activity_patterns(
                    user_id, typical_hours, typical_days,
                    activity_score_2w, sessions_count_2w, total_minutes_2w,
                    last_active_at, last_analyzed_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
                ON CONFLICT(user_id) DO UPDATE SET
                    typical_hours = excluded.typical_hours,
                    typical_days = excluded.typical_days,
                    activity_score_2w = excluded.activity_score_2w,
                    sessions_count_2w = excluded.sessions_count_2w,
                    total_minutes_2w = excluded.total_minutes_2w,
                    last_active_at = excluded.last_active_at,
                    last_analyzed_at = CURRENT_TIMESTAMP
                """,
                (
                    user_id,
                    json.dumps(typical_hours),
                    json.dumps(typical_days),
                    activity_score,
                    len(sessions),
                    total_minutes,
                    last_active.strftime("%Y-%m-%d %H:%M:%S") if last_active else None,
                ),
            )

            # === Co-Spieler aus Session-Logs analysieren (falls vorhanden) ===
            await self._analyze_co_players_from_sessions(user_id)

        except Exception as e:
            logger.error(f"Error analyzing user {user_id}: {e}", exc_info=True)

    async def _analyze_co_players_from_sessions(self, user_id: int):
        """
        Analysiert Co-Spieler aus voice_session_log.co_player_ids
        und aktualisiert die user_co_players Tabelle.
        """
        try:
            cutoff = datetime.utcnow() - timedelta(days=14)
            cutoff_str = cutoff.strftime("%Y-%m-%d %H:%M:%S")

            # Hole alle Sessions mit co_player_ids
            rows = central_db.query_all(
                """
                SELECT co_player_ids, duration_seconds
                FROM voice_session_log
                WHERE user_id = ? AND started_at >= ? AND co_player_ids IS NOT NULL
                """,
                (user_id, cutoff_str),
            )

            if not rows:
                return

            # Aggregiere Co-Spieler-Daten
            co_player_stats = defaultdict(lambda: {"sessions": 0, "minutes": 0})

            for row in rows:
                co_player_ids_json = row[0]
                duration_seconds = row[1] or 0
                duration_minutes = duration_seconds // 60

                if not co_player_ids_json:
                    continue

                try:
                    co_player_ids = json.loads(co_player_ids_json)
                    for co_id in co_player_ids:
                        try:
                            co_int = int(co_id)
                        except Exception:
                            continue
                        if co_int == user_id:
                            continue
                        co_player_stats[co_int]["sessions"] += 1
                        co_player_stats[co_int]["minutes"] += duration_minutes
                except json.JSONDecodeError:
                    continue

            base_display_name = await self._display_name_for(user_id)

            # Speichere/Update in DB (mit persistenten Anzeigenamen)
            for co_id, stats in co_player_stats.items():
                co_display_name = await self._display_name_for(co_id)
                central_db.execute(
                    """
                    INSERT INTO user_co_players(
                        user_id, co_player_id, sessions_together,
                        total_minutes_together, last_played_together,
                        user_display_name, co_player_display_name
                    )
                    VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP, ?, ?)
                    ON CONFLICT(user_id, co_player_id) DO UPDATE SET
                        sessions_together = sessions_together + excluded.sessions_together,
                        total_minutes_together = total_minutes_together + excluded.total_minutes_together,
                        last_played_together = CURRENT_TIMESTAMP,
                        user_display_name = COALESCE(excluded.user_display_name, user_display_name),
                        co_player_display_name = COALESCE(excluded.co_player_display_name, co_player_display_name)
                    """,
                    (
                        user_id,
                        co_id,
                        stats["sessions"],
                        stats["minutes"],
                        base_display_name,
                        co_display_name,
                    ),
                )

        except Exception as e:
            logger.error(f"Error analyzing co-players for {user_id}: {e}", exc_info=True)

    # ========== CO-PLAYER TRACKING ==========

    @tasks.loop(minutes=10)
    async def track_co_players_realtime(self):
        """
        Trackt wer aktuell mit wem in Voice-Channels ist.
        Läuft alle 10 Minuten.
        """
        try:
            logger.debug("Tracking co-players in voice channels...")

            # Durchlaufe alle Guilds
            for guild in self.bot.guilds:
                await self._track_guild_co_players(guild)

        except Exception as e:
            logger.error(f"Error tracking co-players: {e}", exc_info=True)

    @track_co_players_realtime.before_loop
    async def before_track_co_players(self):
        await self.bot.wait_until_ready()

    async def _track_guild_co_players(self, guild: discord.Guild):
        """Trackt Co-Spieler für eine einzelne Guild."""
        try:
            # Durchlaufe alle Voice-Channels
            for channel in guild.voice_channels:
                if not channel.members or len(channel.members) < 2:
                    continue

                # Filter: Nur echte User (keine Bots)
                real_members = [m for m in channel.members if not m.bot]

                if len(real_members) < 2:
                    continue

                # Alle Paarungen speichern
                for i, member1 in enumerate(real_members):
                    for member2 in real_members[i + 1 :]:
                        await self._record_co_player_session(
                            member1.id,
                            member2.id,
                            duration_minutes=10,  # 10 Min pro Loop-Interval
                            user_name=getattr(member1, "display_name", None),
                            co_player_name=getattr(member2, "display_name", None),
                        )

        except Exception as e:
            logger.error(f"Error tracking co-players in guild {guild.id}: {e}", exc_info=True)

    async def _record_co_player_session(
        self,
        user_id: int,
        co_player_id: int,
        duration_minutes: int = 10,
        user_name: str | None = None,
        co_player_name: str | None = None,
    ):
        """Speichert eine Co-Player-Session (bidirektional) und persistiert Namen."""
        if user_id == co_player_id:
            return
        try:
            base_user_name = user_name or await self._display_name_for(user_id)
            base_co_name = co_player_name or await self._display_name_for(co_player_id)

            # Beide Richtungen speichern (A->B und B->A)
            pairs = [
                (user_id, co_player_id, base_user_name, base_co_name),
                (co_player_id, user_id, base_co_name, base_user_name),
            ]
            for uid, co_uid, uid_name, co_name in pairs:
                central_db.execute(
                    """
                    INSERT INTO user_co_players(
                        user_id, co_player_id, sessions_together,
                        total_minutes_together, last_played_together,
                        user_display_name, co_player_display_name
                    )
                    VALUES (?, ?, 1, ?, CURRENT_TIMESTAMP, ?, ?)
                    ON CONFLICT(user_id, co_player_id) DO UPDATE SET
                        sessions_together = sessions_together + 1,
                        total_minutes_together = total_minutes_together + excluded.total_minutes_together,
                        last_played_together = CURRENT_TIMESTAMP,
                        user_display_name = COALESCE(excluded.user_display_name, user_display_name),
                        co_player_display_name = COALESCE(excluded.co_player_display_name, co_player_display_name)
                    """,
                    (uid, co_uid, duration_minutes, uid_name, co_name),
                )

            # Invalidiere Cache
            if user_id in self._co_player_cache:
                del self._co_player_cache[user_id]
            if co_player_id in self._co_player_cache:
                del self._co_player_cache[co_player_id]

        except Exception as e:
            logger.error(f"Error recording co-player session: {e}", exc_info=True)

    async def get_top_co_players(self, user_id: int, limit: int = 5) -> list[tuple[int, int]]:
        """
        Gibt die Top Co-Spieler eines Users zurück.
        Returns: List of (co_player_id, sessions_together)
        """
        # Cache-Check
        now = datetime.utcnow()
        if (
            user_id in self._co_player_cache
            and (now - self._cache_timestamp).total_seconds() < 1800
        ):
            return self._co_player_cache[user_id][:limit]

        try:
            rows = central_db.query_all(
                """
                SELECT co_player_id, sessions_together
                FROM user_co_players
                WHERE user_id = ?
                ORDER BY sessions_together DESC, total_minutes_together DESC
                LIMIT ?
                """,
                (user_id, limit),
            )

            result = [(row[0], row[1]) for row in rows]
            self._co_player_cache[user_id] = result
            return result

        except Exception as e:
            logger.error(f"Error getting co-players for {user_id}: {e}", exc_info=True)
            return []

    # ========== CLEANUP ==========

    @tasks.loop(hours=24)
    async def cleanup_old_pings(self):
        """
        Resettet ping_count_30d für User die länger als 30 Tage nicht gepingt wurden.
        Läuft täglich.
        """
        try:
            cutoff = datetime.utcnow() - timedelta(days=30)
            cutoff_str = cutoff.strftime("%Y-%m-%d %H:%M:%S")

            central_db.execute(
                """
                UPDATE user_activity_patterns
                SET ping_count_30d = 0
                WHERE last_pinged_at < ? OR last_pinged_at IS NULL
                """,
                (cutoff_str,),
            )

            logger.info("Cleaned up old ping counts")

        except Exception as e:
            logger.error(f"Error cleaning up pings: {e}", exc_info=True)

    @cleanup_old_pings.before_loop
    async def before_cleanup(self):
        await self.bot.wait_until_ready()

    # ========== NEW: MEMBER & MESSAGE TRACKING ==========

    def _ensure_new_tables(self):
        """Stellt sicher dass die neuen Tabellen existieren (Schema wird automatisch erstellt)."""
        # Die Tabellen werden bereits in service/db.py::init_schema erstellt
        # Dieser Call sorgt nur dafür dass init_schema nochmal läuft falls nötig
        try:
            central_db.init_schema()
            logger.info("Member & Message tracking tables initialized")
        except Exception as e:
            logger.error(f"Error initializing new tables: {e}", exc_info=True)

        try:
            central_db.execute(
                """
                CREATE TABLE IF NOT EXISTS invite_snapshot_cache (
                    guild_id INTEGER NOT NULL PRIMARY KEY,
                    snapshot_json TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                )
                """
            )
        except Exception as e:
            logger.error("Failed to create invite_snapshot_cache table: %s", e)

    def _save_invite_snapshot_to_db(
        self, guild_id: int, invites: dict[str, Any], vanity: dict[str, Any]
    ) -> None:
        try:
            payload = json.dumps({"invites": invites, "vanity": vanity}, separators=(",", ":"))
            central_db.execute(
                """
                INSERT INTO invite_snapshot_cache (guild_id, snapshot_json, updated_at)
                VALUES (?, ?, datetime('now'))
                ON CONFLICT(guild_id) DO UPDATE SET
                    snapshot_json = excluded.snapshot_json,
                    updated_at = excluded.updated_at
                """,
                (guild_id, payload),
            )
        except Exception as exc:
            logger.debug("Failed to save invite snapshot to DB (guild=%s): %s", guild_id, exc)

    def _load_invite_snapshot_from_db(
        self, guild_id: int
    ) -> tuple[dict[str, Any], dict[str, Any]] | None:
        try:
            row = central_db.query_one(
                "SELECT snapshot_json FROM invite_snapshot_cache WHERE guild_id = ?",
                (guild_id,),
            )
            if not row or not row[0]:
                return None
            data = json.loads(row[0])
            return data.get("invites", {}), data.get("vanity", {})
        except Exception as exc:
            logger.debug("Failed to load invite snapshot from DB (guild=%s): %s", guild_id, exc)
            return None

    @staticmethod
    def _text_session_ts(dt: datetime) -> str:
        return dt.strftime("%Y-%m-%d %H:%M:%S")

    def _text_leaderboard_footer(self, user_id: int) -> str:
        row = central_db.query_one(
            "SELECT total_points, total_messages FROM text_stats WHERE user_id = ?",
            (user_id,),
        )
        if not row:
            return "Noch keine Punkte"

        total_points = int(row[0] or 0)
        total_messages = int(row[1] or 0)
        rank_row = central_db.query_one(
            """
            SELECT COUNT(*) + 1
            FROM text_stats
            WHERE total_points > ?
               OR (total_points = ? AND total_messages > ?)
            """,
            (total_points, total_points, total_messages),
        )
        rank = int(rank_row[0] or 1) if rank_row else 1
        return f"Du bist auf Platz {rank} · {_format_leaderboard_number(total_points)} Punkte"

    async def _flush_expired_text_sessions(self, now: datetime | None = None) -> None:
        now = now or datetime.utcnow()
        expired_sessions = [
            (key, session)
            for key, session in list(self._open_text_sessions.items())
            if (now - session["last_message_at"]).total_seconds() >= TEXT_SESSION_WINDOW_SECONDS
        ]
        for key, session in expired_sessions:
            await self._flush_text_session(key, session)

    async def _flush_all_text_sessions(self) -> None:
        for key, session in list(self._open_text_sessions.items()):
            await self._flush_text_session(key, session)

    async def _flush_text_session(self, key: tuple[int, int], session: dict[str, Any]) -> None:
        if self._open_text_sessions.get(key) is not session:
            return

        message_count = int(session.get("message_count") or 0)
        if message_count <= 0:
            self._open_text_sessions.pop(key, None)
            return

        points_accum = float(session.get("points_accum") or 0.0)
        had_interaction = bool(session.get("had_interaction"))
        final_points = round(points_accum * 1.5) if had_interaction else round(points_accum)
        co_participants = sorted(int(user_id) for user_id in session.get("co_participants", set()))
        co_participant_ids = ",".join(str(user_id) for user_id in co_participants) or None

        try:
            central_db.execute(
                """
                INSERT INTO text_conversation_log(
                    user_id, guild_id, channel_id, started_at, ended_at,
                    message_count, points, co_participant_ids, had_interaction
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    key[0],
                    session.get("guild_id"),
                    key[1],
                    self._text_session_ts(session["started_at"]),
                    self._text_session_ts(session["last_message_at"]),
                    message_count,
                    final_points,
                    co_participant_ids,
                    1 if had_interaction else 0,
                ),
            )
            central_db.execute(
                """
                INSERT INTO text_stats(user_id, total_messages, total_points, last_update)
                VALUES (?, ?, ?, CURRENT_TIMESTAMP)
                ON CONFLICT(user_id) DO UPDATE SET
                    total_messages = text_stats.total_messages + excluded.total_messages,
                    total_points = text_stats.total_points + excluded.total_points,
                    last_update = CURRENT_TIMESTAMP
                """,
                (key[0], message_count, final_points),
            )
        except Exception as exc:
            logger.error(
                "Error flushing text session for user %s in channel %s: %s",
                key[0],
                key[1],
                exc,
                exc_info=True,
            )
            return

        self._open_text_sessions.pop(key, None)

    @staticmethod
    def _to_int(value: Any, default: int | None = None) -> int | None:
        try:
            if value is None:
                return default
            return int(value)
        except (TypeError, ValueError):
            return default

    def _invite_lock_for_guild(self, guild_id: int) -> asyncio.Lock:
        lock = self._join_source_locks.get(guild_id)
        if lock is None:
            lock = asyncio.Lock()
            self._join_source_locks[guild_id] = lock
        return lock

    async def _collect_join_invite_snapshot(self, guild: discord.Guild) -> dict[str, Any]:
        invites: dict[str, dict[str, Any]] = {}
        invites_ok = False

        try:
            invite_rows = await guild.invites()
            invites_ok = True
            for invite in invite_rows:
                code = str(getattr(invite, "code", "") or "").strip()
                if not code:
                    continue
                inviter = getattr(invite, "inviter", None)
                channel = getattr(invite, "channel", None)
                invite_url = (
                    str(getattr(invite, "url", "") or "").strip() or f"https://discord.gg/{code}"
                )
                invites[code] = {
                    "uses": self._to_int(getattr(invite, "uses", 0), 0) or 0,
                    "url": invite_url,
                    "inviter_id": self._to_int(getattr(inviter, "id", None), None),
                    "inviter_name": str(inviter) if inviter else None,
                    "inviter_bot": bool(getattr(inviter, "bot", False)) if inviter else False,
                    "channel_id": self._to_int(getattr(channel, "id", None), None),
                    "channel_name": str(getattr(channel, "name", "") or "").strip() or None,
                }
        except discord.Forbidden:
            logger.debug("Join source: guild.invites forbidden (guild=%s)", guild.id)
        except discord.HTTPException as exc:
            logger.debug("Join source: guild.invites failed (guild=%s): %s", guild.id, exc)
        except Exception as exc:
            logger.debug(
                "Join source: guild.invites unexpected (guild=%s): %s",
                guild.id,
                exc,
                exc_info=True,
            )

        vanity_code_raw = getattr(guild, "vanity_url_code", None)
        vanity_code = str(vanity_code_raw).strip() if vanity_code_raw else None
        vanity_uses: int | None = None
        vanity_ok = False
        try:
            vanity_invite = await guild.vanity_invite()
            if vanity_invite:
                vanity_ok = True
                code_from_invite = str(getattr(vanity_invite, "code", "") or "").strip()
                if code_from_invite:
                    vanity_code = code_from_invite
                vanity_uses = self._to_int(getattr(vanity_invite, "uses", None), None)
        except discord.Forbidden:
            logger.debug("Join source: guild.vanity_invite forbidden (guild=%s)", guild.id)
        except discord.HTTPException as exc:
            logger.debug("Join source: guild.vanity_invite failed (guild=%s): %s", guild.id, exc)
        except Exception as exc:
            logger.debug(
                "Join source: guild.vanity_invite unexpected (guild=%s): %s",
                guild.id,
                exc,
                exc_info=True,
            )

        return {
            "invites": invites,
            "invites_ok": invites_ok,
            "vanity": {
                "code": vanity_code,
                "uses": vanity_uses,
                "ok": vanity_ok,
            },
        }

    async def _prime_join_source_cache(self) -> None:
        try:
            await self.bot.wait_until_ready()
            guilds = list(getattr(self.bot, "guilds", []))

            # Schritt 1: Aus DB wiederherstellen (kein API-Call)
            restored = 0
            for guild in guilds:
                cached = self._load_invite_snapshot_from_db(guild.id)
                if cached is not None:
                    invites, vanity = cached
                    self._join_invite_snapshot[guild.id] = invites
                    self._join_vanity_snapshot[guild.id] = vanity
                    restored += 1
            logger.info("Invite cache restored from DB for %d/%d guild(s)", restored, len(guilds))

            # Schritt 2: API-Fetch nur für Guilds ohne DB-Eintrag
            missing = [g for g in guilds if g.id not in self._join_invite_snapshot]
            for guild in missing:
                lock = self._invite_lock_for_guild(guild.id)
                async with lock:
                    snapshot = await self._collect_join_invite_snapshot(guild)
                    if snapshot.get("invites_ok"):
                        invites = snapshot.get("invites", {})
                        vanity = snapshot.get("vanity", {})
                        self._join_invite_snapshot[guild.id] = invites
                        self._save_invite_snapshot_to_db(guild.id, invites, vanity)
                    else:
                        self._join_invite_snapshot.pop(guild.id, None)
                    self._join_vanity_snapshot[guild.id] = snapshot.get("vanity", {})
                await asyncio.sleep(0.2)
            if missing:
                logger.debug("Invite cache API-fetched for %d guild(s)", len(missing))
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            logger.debug("Join source cache prime failed: %s", exc, exc_info=True)

    async def _backfill_member_events(self) -> None:
        """Legt rückwirkend join-Events für alle Mitglieder ohne DB-Eintrag an."""
        try:
            await self.bot.wait_until_ready()
            guilds = list(getattr(self.bot, "guilds", []))
            total_inserted = 0

            for guild in guilds:
                try:
                    # Bekannte User-IDs aus member_events holen
                    existing_rows = central_db.query_all(
                        """
                        SELECT DISTINCT user_id FROM member_events
                        WHERE guild_id = ? AND event_type = 'join'
                        """,
                        (guild.id,),
                    )
                    existing_ids: set[int] = {int(r[0]) for r in existing_rows}

                    # Twitch-Invite-Lookup für Backfill-Korrelation
                    twitch_lookup = self._load_twitch_invite_lookup(guild.id)

                    inserted = 0
                    for member in guild.members:
                        if member.bot:
                            continue
                        if member.id in existing_ids:
                            continue
                        if privacy.is_opted_out(member.id):
                            continue

                        joined_ts = (
                            member.joined_at.strftime("%Y-%m-%d %H:%M:%S")
                            if member.joined_at
                            else None
                        )
                        account_created = (
                            member.created_at.strftime("%Y-%m-%d %H:%M:%S")
                            if member.created_at
                            else None
                        )

                        # Quelle: Snapshot-Cache für bekannte Invite-Codes prüfen
                        snapshot = self._join_invite_snapshot.get(guild.id, {})
                        matched_code: str | None = None
                        matched_twitch: str | None = None
                        matched_bot: bool = False

                        # Prüfe ob ein bekannter Twitch-Code aktiv war — grob korrelierbar
                        for code, info in snapshot.items():
                            twitch = twitch_lookup.get(code.lower())
                            if twitch and self._to_int(info.get("uses"), 0):
                                # Wenn nur ein aktiver Twitch-Code existiert → wahrscheinlich
                                if not matched_twitch:
                                    matched_code = code
                                    matched_twitch = twitch
                                    matched_bot = bool(info.get("inviter_bot"))

                        source_bucket = "unknown"
                        source_kind = "backfilled"
                        source_label = "Vor Tracking (rückwirkend)"
                        source_confidence = "none"

                        metadata = json.dumps(
                            {
                                "join_source_bucket": source_bucket,
                                "join_source_kind": source_kind,
                                "join_source_label": source_label,
                                "join_source_confidence": source_confidence,
                                "backfilled": True,
                                "invite_code": matched_code,
                                "twitch_streamer_login": matched_twitch,
                                "matched_twitch_hint": matched_twitch,
                                "inviter_bot": matched_bot or None,
                            }
                        )

                        central_db.execute(
                            """
                            INSERT OR IGNORE INTO member_events
                                (user_id, guild_id, event_type, timestamp,
                                 display_name, account_created_at, metadata)
                            VALUES (?, ?, 'join', ?, ?, ?, ?)
                            """,
                            (
                                member.id,
                                guild.id,
                                joined_ts,
                                member.display_name,
                                account_created,
                                metadata,
                            ),
                        )
                        inserted += 1

                    total_inserted += inserted
                    if inserted:
                        logger.info(
                            "Backfill: %d join-Events für Guild %s nachgetragen",
                            inserted,
                            guild.name,
                        )
                except Exception as exc:
                    logger.warning("Backfill fehlgeschlagen für Guild %s: %s", guild.id, exc)

            logger.info("Backfill abgeschlossen: %d Events gesamt eingetragen", total_inserted)
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            logger.error("Backfill member events fehlgeschlagen: %s", exc, exc_info=True)

    def _lookup_twitch_streamer_for_code(self, guild_id: int, invite_code: str) -> str | None:
        code = (invite_code or "").strip()
        if not code:
            return None
        if self._twitch_invite_table_available is False:
            return None
        try:
            row = central_db.query_one(
                """
                SELECT streamer_login
                FROM twitch_streamer_invites
                WHERE LOWER(invite_code) = LOWER(?)
                  AND (? IS NULL OR guild_id = ?)
                ORDER BY COALESCE(last_sent_at, created_at) DESC
                LIMIT 1
                """,
                (code, guild_id, guild_id),
            )
        except Exception as exc:
            if self._is_missing_twitch_invites_table(exc):
                self._mark_twitch_invites_unavailable()
                return None
            return None
        self._twitch_invite_table_available = True
        if not row or not row[0]:
            return None
        login = str(row[0]).strip().lower()
        return login or None

    @staticmethod
    def _is_missing_twitch_invites_table(exc: BaseException) -> bool:
        message = str(exc or "").lower()
        return "no such table" in message and "twitch_streamer_invites" in message

    def _mark_twitch_invites_unavailable(self) -> None:
        if self._twitch_invite_table_available is False:
            return
        self._twitch_invite_table_available = False
        logger.info(
            "Legacy Twitch invite table not available; skipping Twitch invite correlation in UserActivityAnalyzer."
        )

    def _load_twitch_invite_lookup(self, guild_id: int) -> dict[str, str]:
        if self._twitch_invite_table_available is False:
            return {}
        try:
            rows = central_db.query_all(
                """
                SELECT LOWER(invite_code), streamer_login
                FROM twitch_streamer_invites
                WHERE guild_id = ?
                """,
                (guild_id,),
            )
        except Exception as exc:
            if self._is_missing_twitch_invites_table(exc):
                self._mark_twitch_invites_unavailable()
                return {}
            raise
        self._twitch_invite_table_available = True
        return {r[0]: r[1] for r in rows if r[0]}

    def _classify_join_source(
        self,
        *,
        guild_id: int,
        before_invites: dict[str, dict[str, Any]] | None,
        after_invites: dict[str, dict[str, Any]],
        before_vanity: dict[str, Any] | None,
        after_vanity: dict[str, Any],
        invites_ok: bool,
    ) -> dict[str, Any]:
        detected_at = datetime.utcnow().strftime("%Y-%m-%d %H:%M:%S")
        payload: dict[str, Any] = {
            "join_source_bucket": "unknown",
            "join_source_kind": "unknown",
            "join_source_label": "Unbekannt",
            "join_source_confidence": "low",
            "join_source_detected_at": detected_at,
        }

        increased_codes: list[tuple[int, str, dict[str, Any]]] = []
        if before_invites is not None:
            for code, invite_info in after_invites.items():
                after_uses = self._to_int(invite_info.get("uses"), 0) or 0
                before_uses = self._to_int((before_invites.get(code) or {}).get("uses"), 0) or 0
                delta = after_uses - before_uses
                if delta > 0:
                    increased_codes.append((delta, code, invite_info))

        if increased_codes:
            increased_codes.sort(key=lambda item: (-item[0], item[1].lower()))
            _, code, invite_info = increased_codes[0]
            twitch_login = self._lookup_twitch_streamer_for_code(guild_id, code)

            payload.update(
                {
                    "invite_code": code,
                    "invite_url": invite_info.get("url") or f"https://discord.gg/{code}",
                    "inviter_id": invite_info.get("inviter_id"),
                    "inviter_name": invite_info.get("inviter_name"),
                    "invite_channel_id": invite_info.get("channel_id"),
                    "invite_channel_name": invite_info.get("channel_name"),
                    "join_source_confidence": "high",
                }
            )

            if twitch_login:
                payload.update(
                    {
                        "join_source_bucket": "twitch",
                        "join_source_kind": "twitch_streamer",
                        "join_source_label": f"Twitch: {twitch_login}",
                        "twitch_streamer_login": twitch_login,
                    }
                )
            elif invite_info.get("inviter_bot"):
                inviter_name = invite_info.get("inviter_name") or "Bot"
                payload.update(
                    {
                        "join_source_bucket": "bot_invite",
                        "join_source_kind": "bot_invite",
                        "join_source_label": f"Bot Invite: {inviter_name}",
                        "inviter_bot": True,
                    }
                )
            else:
                payload.update(
                    {
                        "join_source_bucket": "personal",
                        "join_source_kind": "invite_link",
                        "join_source_label": "Persoenliche Einladung",
                    }
                )
            return payload

        before_vanity_uses = self._to_int((before_vanity or {}).get("uses"), None)
        after_vanity_uses = self._to_int((after_vanity or {}).get("uses"), None)
        vanity_delta: int | None = None
        if before_vanity_uses is not None and after_vanity_uses is not None:
            vanity_delta = after_vanity_uses - before_vanity_uses

        if vanity_delta and vanity_delta > 0:
            vanity_code = str(after_vanity.get("code") or "").strip() or None
            payload.update(
                {
                    "join_source_bucket": "public",
                    "join_source_kind": "vanity",
                    "join_source_label": "Public: Vanity-Link",
                    "join_source_confidence": "high",
                    "vanity_code": vanity_code,
                    "vanity_url": f"https://discord.gg/{vanity_code}" if vanity_code else None,
                }
            )
            return payload

        has_baseline = before_invites is not None or before_vanity is not None
        if has_baseline and invites_ok:
            payload.update(
                {
                    "join_source_bucket": "public",
                    "join_source_kind": "server_discovery",
                    "join_source_label": "Public: Server entdecken",
                    "join_source_confidence": "medium",
                }
            )
            return payload

        payload["join_source_reason"] = (
            "baseline_missing" if not has_baseline else "invite_snapshot_unavailable"
        )
        return payload

    async def _detect_join_source(self, guild: discord.Guild) -> dict[str, Any]:
        guild_id = guild.id
        lock = self._invite_lock_for_guild(guild_id)
        async with lock:
            baseline_invites = self._join_invite_snapshot.get(guild_id)
            baseline_vanity = self._join_vanity_snapshot.get(guild_id)

            attempts = 2 if (baseline_invites is not None or baseline_vanity is not None) else 1
            latest_snapshot: dict[str, Any] | None = None
            detected: dict[str, Any] | None = None

            for attempt in range(attempts):
                latest_snapshot = await self._collect_join_invite_snapshot(guild)
                detected = self._classify_join_source(
                    guild_id=guild_id,
                    before_invites=baseline_invites,
                    after_invites=latest_snapshot.get("invites", {}),
                    before_vanity=baseline_vanity,
                    after_vanity=latest_snapshot.get("vanity", {}),
                    invites_ok=bool(latest_snapshot.get("invites_ok")),
                )
                kind = str(detected.get("join_source_kind") or "").lower()
                if kind not in {"server_discovery", "unknown"}:
                    break
                if attempt < attempts - 1:
                    await asyncio.sleep(1.0)

            if latest_snapshot:
                if latest_snapshot.get("invites_ok"):
                    invites = latest_snapshot.get("invites", {})
                    vanity = latest_snapshot.get("vanity", {})
                    self._join_invite_snapshot[guild_id] = invites
                    self._save_invite_snapshot_to_db(guild_id, invites, vanity)
                self._join_vanity_snapshot[guild_id] = latest_snapshot.get("vanity", {})

            return detected or {
                "join_source_bucket": "unknown",
                "join_source_kind": "unknown",
                "join_source_label": "Unbekannt",
                "join_source_confidence": "low",
                "join_source_detected_at": datetime.utcnow().strftime("%Y-%m-%d %H:%M:%S"),
            }

    @commands.Cog.listener()
    async def on_member_join(self, member: discord.Member):
        """Trackt wenn ein Member dem Server beitritt."""
        try:
            if member.bot:
                return
            if privacy.is_opted_out(member.id):
                return

            # Berechne Join-Position (wievielter Member ist das?)
            join_position = len(member.guild.members)

            # Account-Erstellungsdatum
            account_created = (
                member.created_at.strftime("%Y-%m-%d %H:%M:%S") if member.created_at else None
            )

            # Metadata (zusätzliche Infos als JSON)
            metadata = {
                "avatar_url": str(member.display_avatar.url) if member.display_avatar else None,
                "is_pending": member.pending if hasattr(member, "pending") else None,
            }
            join_source = await self._detect_join_source(member.guild)
            metadata.update(join_source)

            central_db.execute(
                """
                INSERT INTO member_events(
                    user_id, guild_id, event_type, display_name,
                    account_created_at, join_position, metadata
                )
                VALUES (?, ?, 'join', ?, ?, ?, ?)
                """,
                (
                    member.id,
                    member.guild.id,
                    member.display_name,
                    account_created,
                    join_position,
                    json.dumps(metadata),
                ),
            )

            logger.info(
                "Member join tracked: %s (%s) -> %s [source=%s/%s]",
                member.display_name,
                member.id,
                member.guild.name,
                metadata.get("join_source_bucket", "unknown"),
                metadata.get("join_source_kind", "unknown"),
            )

        except Exception as e:
            logger.error(f"Error tracking member join: {e}", exc_info=True)

    @commands.Cog.listener()
    async def on_member_remove(self, member: discord.Member):
        """Trackt wenn ein Member den Server verlässt UND erstellt automatisch eine Analyse."""
        try:
            if member.bot:
                return
            if privacy.is_opted_out(member.id):
                return

            recent = central_db.query_one(
                """
                SELECT 1 FROM member_events
                WHERE user_id = ? AND guild_id = ? AND event_type = 'leave'
                  AND timestamp >= datetime('now', '-10 seconds')
                """,
                (member.id, member.guild.id),
            )
            if recent:
                return

            # Event loggen
            central_db.execute(
                """
                INSERT INTO member_events(
                    user_id, guild_id, event_type, display_name
                )
                VALUES (?, ?, 'leave', ?)
                """,
                (member.id, member.guild.id, member.display_name),
            )

            logger.info(
                f"Member leave tracked: {member.display_name} ({member.id}) -> {member.guild.name}"
            )

            # AUTOMATISCHE ANALYSE beim Leave (Optional - kann aktiviert werden)
            # Uncomment die nächste Zeile wenn du automatische Analysen beim Leave willst:
            # await self._send_leave_analysis(member)

        except Exception as e:
            logger.error(f"Error tracking member leave: {e}", exc_info=True)

    @commands.Cog.listener()
    async def on_member_ban(self, guild: discord.Guild, user: discord.User):
        """Trackt wenn ein Member gebannt wird."""
        try:
            if user.bot:
                return
            if privacy.is_opted_out(user.id):
                return

            # Ban-Event loggen
            central_db.execute(
                """
                INSERT INTO member_events(
                    user_id, guild_id, event_type, display_name
                )
                VALUES (?, ?, 'ban', ?)
                """,
                (user.id, guild.id, user.display_name),
            )

            logger.info(
                "Member ban tracked: %s (%s) -> %s",
                _safe_log_value(user.display_name),
                user.id,
                _safe_log_value(guild.name),
            )

        except Exception as e:
            logger.error(f"Error tracking member ban: {e}", exc_info=True)

    @commands.Cog.listener()
    async def on_member_unban(self, guild: discord.Guild, user: discord.User):
        """Trackt wenn ein Member entbannt wird."""
        try:
            if user.bot:
                return
            if privacy.is_opted_out(user.id):
                return

            # Unban-Event loggen
            central_db.execute(
                """
                INSERT INTO member_events(
                    user_id, guild_id, event_type, display_name
                )
                VALUES (?, ?, 'unban', ?)
                """,
                (user.id, guild.id, user.display_name),
            )

            logger.info(
                "Member unban tracked: %s (%s) -> %s",
                _safe_log_value(user.display_name),
                user.id,
                _safe_log_value(guild.name),
            )

        except Exception as e:
            logger.error(f"Error tracking member unban: {e}", exc_info=True)

    @commands.Cog.listener()
    async def on_invite_create(self, invite: discord.Invite) -> None:
        """Neuen Invite sofort in den Cache aufnehmen ohne API-Refetch."""
        try:
            guild = getattr(invite, "guild", None)
            if guild is None:
                return
            code = str(getattr(invite, "code", "") or "").strip()
            if not code:
                return
            inviter = getattr(invite, "inviter", None)
            channel = getattr(invite, "channel", None)
            entry = {
                "uses": self._to_int(getattr(invite, "uses", 0), 0) or 0,
                "url": str(getattr(invite, "url", "") or "").strip()
                or f"https://discord.gg/{code}",
                "inviter_id": self._to_int(getattr(inviter, "id", None), None),
                "inviter_name": str(inviter) if inviter else None,
                "inviter_bot": bool(getattr(inviter, "bot", False)) if inviter else False,
                "channel_id": self._to_int(getattr(channel, "id", None), None),
                "channel_name": str(getattr(channel, "name", "") or "").strip() or None,
            }
            lock = self._invite_lock_for_guild(guild.id)
            async with lock:
                snapshot = self._join_invite_snapshot.setdefault(guild.id, {})
                snapshot[code] = entry
                vanity = self._join_vanity_snapshot.get(guild.id, {})
                self._save_invite_snapshot_to_db(guild.id, snapshot, vanity)
            logger.debug("Invite %s created for guild %s — cache updated", code, guild.id)
        except Exception as exc:
            logger.debug("on_invite_create cache update failed: %s", exc)

    @commands.Cog.listener()
    async def on_invite_delete(self, invite: discord.Invite) -> None:
        """Gelöschten Invite aus dem Cache entfernen ohne API-Refetch."""
        try:
            guild = getattr(invite, "guild", None)
            if guild is None:
                return
            code = str(getattr(invite, "code", "") or "").strip()
            if not code:
                return
            lock = self._invite_lock_for_guild(guild.id)
            async with lock:
                snapshot = self._join_invite_snapshot.get(guild.id, {})
                if code in snapshot:
                    del snapshot[code]
                    vanity = self._join_vanity_snapshot.get(guild.id, {})
                    self._save_invite_snapshot_to_db(guild.id, snapshot, vanity)
            logger.debug("Invite %s deleted for guild %s — cache updated", code, guild.id)
        except Exception as exc:
            logger.debug("on_invite_delete cache update failed: %s", exc)

    @commands.Cog.listener()
    async def on_raw_member_remove(self, payload: discord.RawMemberRemoveEvent):
        """
        Fängt Leave/Kick/Ban Events auch dann ab, wenn das Member nicht im Cache ist.
        Nutzt ein Dedupe-Fenster, um doppelte Einträge mit on_member_remove zu vermeiden.
        """
        try:
            user = payload.user
            if user and user.bot:
                return

            user_id = getattr(payload, "user_id", None) or (user.id if user else None)
            if user_id is None:
                logger.debug("Raw member remove ohne user_id erhalten; Event übersprungen.")
                return
            if privacy.is_opted_out(int(user_id)):
                return

            recent = central_db.query_one(
                """
                SELECT 1 FROM member_events
                WHERE user_id = ? AND guild_id = ? AND event_type = 'leave'
                  AND timestamp >= datetime('now', '-10 seconds')
                """,
                (user_id, payload.guild_id),
            )
            if recent:
                return

            display_name = user.name if user else None
            if display_name is None:
                try:
                    fetched = await self.bot.fetch_user(user_id)
                    display_name = fetched.name
                except Exception:
                    display_name = None

            central_db.execute(
                """
                INSERT INTO member_events(
                    user_id, guild_id, event_type, display_name
                )
                VALUES (?, ?, 'leave', ?)
                """,
                (user_id, payload.guild_id, display_name),
            )

            logger.info(
                "Member leave tracked (raw): %s (%s) -> guild %s",
                display_name or "unknown",
                user_id,
                payload.guild_id,
            )

        except Exception as e:
            logger.error(f"Error tracking member leave (raw): {e}", exc_info=True)

    @commands.Cog.listener()
    async def on_message(self, message: discord.Message):
        """Trackt Message-Aktivität von Usern."""
        try:
            # Ignore Bots & DMs
            if not message.guild or message.author.bot:
                return
            if privacy.is_opted_out(message.author.id):
                return

            # Update/Insert Message Activity
            central_db.execute(
                """
                INSERT INTO message_activity(
                    user_id, guild_id, channel_id, message_count,
                    last_message_at, first_message_at
                )
                VALUES (?, ?, ?, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
                ON CONFLICT(user_id, guild_id) DO UPDATE SET
                    message_count = message_count + 1,
                    last_message_at = CURRENT_TIMESTAMP,
                    channel_id = excluded.channel_id
                """,
                (message.author.id, message.guild.id, message.channel.id),
            )

            now = datetime.utcnow()
            await self._flush_expired_text_sessions(now)

            session_key = (message.author.id, message.channel.id)
            session = self._open_text_sessions.get(session_key)
            if session is None:
                session = {
                    "started_at": now,
                    "last_message_at": now,
                    "guild_id": message.guild.id,
                    "message_count": 0,
                    "points_accum": 0.0,
                    "co_participants": set(),
                    "had_interaction": False,
                    "last_reply_credited": False,
                }
                self._open_text_sessions[session_key] = session

            other_sessions = [
                ((user_id, channel_id), other_session)
                for (user_id, channel_id), other_session in self._open_text_sessions.items()
                if channel_id == message.channel.id and user_id != message.author.id
            ]
            if other_sessions:
                session["had_interaction"] = True
                for (other_user_id, _channel_id), other_session in other_sessions:
                    session["co_participants"].add(other_user_id)
                    other_session["had_interaction"] = True
                    other_session["co_participants"].add(message.author.id)

            msg_index = int(session["message_count"]) + 1
            raw_points = 2 * math.sqrt(msg_index)
            if message.reference and not session["last_reply_credited"]:
                raw_points += 1
                session["last_reply_credited"] = True

            session["message_count"] = msg_index
            session["points_accum"] = float(session["points_accum"]) + raw_points
            session["last_message_at"] = now
            session["guild_id"] = message.guild.id

            # Kein Log hier (zu viel Spam bei vielen Messages)

        except Exception as e:
            logger.error(f"Error tracking message activity: {e}", exc_info=True)

    @tasks.loop(seconds=60)
    async def flush_text_sessions(self):
        try:
            await self._flush_expired_text_sessions()
        except Exception as exc:
            logger.error("Error while flushing expired text sessions: %s", exc, exc_info=True)

    @flush_text_sessions.before_loop
    async def before_text_session_flush(self):
        await self.bot.wait_until_ready()

    async def _send_leave_analysis(self, member: discord.Member):
        """
        Sendet eine automatische Analyse wenn ein Member den Server verlässt.
        Zeigt was der User alles gemacht hat.
        """
        try:
            # Finde einen geeigneten Channel zum Posten (z.B. einen Log-Channel)
            # Du kannst das anpassen auf deinen spezifischen Log-Channel
            log_channel = None
            for channel in member.guild.text_channels:
                if "log" in channel.name.lower() or "audit" in channel.name.lower():
                    log_channel = channel
                    break

            if not log_channel:
                return  # Kein Log-Channel gefunden

            # Generiere Analyse-Embed
            embed = await self._generate_user_analysis_embed(member)

            await log_channel.send(f"📊 **User-Leave-Analyse:** {member.mention}", embed=embed)

        except Exception as e:
            logger.error(f"Error sending leave analysis: {e}", exc_info=True)

    async def _generate_user_analysis_embed(self, member: discord.Member) -> discord.Embed:
        """Generiert ein umfassendes Analyse-Embed für einen User."""
        embed = discord.Embed(
            title=f"📊 User-Aktivitäts-Analyse - {member.display_name}",
            color=discord.Color.orange(),
        )
        embed.set_thumbnail(url=member.display_avatar.url)

        # === 1. Join/Leave History ===
        join_leave_events = central_db.query_all(
            """
            SELECT event_type, timestamp, account_created_at, join_position
            FROM member_events
            WHERE user_id = ? AND guild_id = ?
            ORDER BY timestamp ASC
            """,
            (member.id, member.guild.id),
        )

        if join_leave_events:
            first_join = join_leave_events[0]
            joins = [e for e in join_leave_events if e[0] == "join"]
            leaves = [e for e in join_leave_events if e[0] == "leave"]
            bans = [e for e in join_leave_events if e[0] == "ban"]

            history_text = (
                f"**Erstes Join:** {first_join[1][:16] if first_join[1] else 'Unbekannt'}\n"
            )
            history_text += f"**Joins:** {len(joins)} | **Leaves:** {len(leaves)}"
            if bans:
                history_text += f" | **Bans:** {len(bans)}"

            if first_join[2]:  # account_created_at
                history_text += f"\n**Account erstellt:** {first_join[2][:10]}"
            if first_join[3]:  # join_position
                history_text += f"\n**Join-Position:** #{first_join[3]}"

            embed.add_field(name="📅 Server-History", value=history_text, inline=False)

        # === 2. Voice Activity ===
        voice_stats = central_db.query_one(
            """
            SELECT total_seconds, total_points
            FROM voice_stats
            WHERE user_id = ?
            """,
            (member.id,),
        )

        if voice_stats and voice_stats[0]:
            total_seconds = voice_stats[0]
            total_points = voice_stats[1] or 0
            hours = total_seconds // 3600
            minutes = (total_seconds % 3600) // 60

            # Letzte Voice-Session
            last_session = central_db.query_one(
                """
                SELECT channel_name, ended_at, duration_seconds
                FROM voice_session_log
                WHERE user_id = ? AND guild_id = ?
                ORDER BY ended_at DESC
                LIMIT 1
                """,
                (member.id, member.guild.id),
            )

            voice_text = f"**Gesamtzeit:** {hours}h {minutes}m\n**Punkte:** {total_points}"
            if last_session:
                voice_text += f"\n**Letzter Voice:** {last_session[0]} ({last_session[2] // 60}min)"

            embed.add_field(name="🎙️ Voice-Aktivität", value=voice_text, inline=True)
        else:
            embed.add_field(name="🎙️ Voice-Aktivität", value="Keine Voice-Aktivität", inline=True)

        # === 3. Message Activity ===
        msg_stats = central_db.query_one(
            """
            SELECT message_count, last_message_at, first_message_at
            FROM message_activity
            WHERE user_id = ? AND guild_id = ?
            """,
            (member.id, member.guild.id),
        )

        if msg_stats and msg_stats[0]:
            msg_count = msg_stats[0]
            last_msg = msg_stats[1][:16] if msg_stats[1] else "Unbekannt"
            first_msg = msg_stats[2][:16] if msg_stats[2] else "Unbekannt"

            msg_text = f"**Nachrichten:** {msg_count}\n"
            msg_text += f"**Erste Nachricht:** {first_msg}\n"
            msg_text += f"**Letzte Nachricht:** {last_msg}"

            embed.add_field(name="💬 Nachrichten", value=msg_text, inline=True)
        else:
            embed.add_field(name="💬 Nachrichten", value="Keine Nachrichten", inline=True)

        # === 4. Co-Spieler (Top 3) ===
        co_players = await self.get_top_co_players(member.id, limit=3)
        if co_players:
            co_text = ""
            for co_id, sessions_together in co_players:
                co_member = member.guild.get_member(co_id)
                name = co_member.display_name if co_member else f"User {co_id}"
                co_text += f"**{name}** ({sessions_together}x)\n"
            embed.add_field(name="👥 Top Mitspieler", value=co_text, inline=False)

        # === 5. Aktivitätsmuster ===
        patterns = central_db.query_one(
            """
            SELECT typical_hours, typical_days, activity_score_2w
            FROM user_activity_patterns
            WHERE user_id = ?
            """,
            (member.id,),
        )

        if patterns and patterns[2]:  # activity_score_2w
            typical_hours = json.loads(patterns[0]) if patterns[0] else []
            typical_days = json.loads(patterns[1]) if patterns[1] else []
            activity_score = patterns[2]

            pattern_text = f"**Activity Score:** {activity_score}\n"
            if typical_hours:
                hours_str = ", ".join([f"{h}:00" for h in typical_hours[:3]])
                pattern_text += f"**Typische Zeiten:** {hours_str}\n"
            if typical_days:
                day_names = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]
                days_str = ", ".join([day_names[d] for d in typical_days[:3]])
                pattern_text += f"**Typische Tage:** {days_str}"

            embed.add_field(name="📈 Aktivitätsmuster", value=pattern_text, inline=False)

        embed.set_footer(text=f"User ID: {member.id}")
        embed.timestamp = datetime.utcnow()

        return embed

    # ========== SMART PINGING ==========

    async def should_ping_user(self, user_id: int, max_pings_30d: int = 3) -> tuple[bool, str]:
        """
        Prüft ob ein User gepingt werden sollte.

        Returns:
            (can_ping: bool, reason: str)
        """
        try:
            # Hole Pattern-Daten
            row = central_db.query_one(
                """
                SELECT typical_hours, typical_days, activity_score_2w,
                       last_pinged_at, ping_count_30d
                FROM user_activity_patterns
                WHERE user_id = ?
                """,
                (user_id,),
            )

            if not row:
                return False, "Keine Aktivitätsdaten vorhanden"

            typical_hours_json = row[0]
            typical_days_json = row[1]
            activity_score = row[2] or 0
            last_pinged_str = row[3]
            ping_count = row[4] or 0

            # Check 1: Rate-Limit (max 3 Pings in 30 Tagen)
            if ping_count >= max_pings_30d:
                return (
                    False,
                    f"Rate-Limit erreicht ({ping_count}/{max_pings_30d} in 30d)",
                )

            # Check 2: Mindestens 1 Tag seit letztem Ping
            if last_pinged_str:
                last_pinged = datetime.strptime(last_pinged_str, "%Y-%m-%d %H:%M:%S")
                time_since_ping = (datetime.utcnow() - last_pinged).total_seconds()
                if time_since_ping < 86400:  # 24h
                    hours_remaining = (86400 - time_since_ping) / 3600
                    return (
                        False,
                        f"Zu früh (noch {hours_remaining:.1f}h bis nächster Ping)",
                    )

            # Check 3: User sollte aktiv sein (min 5 Sessions in 2W)
            if activity_score < 5:
                return False, f"User zu inaktiv (nur {activity_score} Sessions in 2W)"

            # Check 4: Typische Online-Zeit?
            now = datetime.utcnow()
            current_hour = now.hour
            current_day = now.weekday()

            typical_hours = json.loads(typical_hours_json) if typical_hours_json else []
            typical_days = json.loads(typical_days_json) if typical_days_json else []

            # Flexibles Zeitfenster: ±2 Stunden von typischen Zeiten
            hour_match = False
            for typ_hour in typical_hours:
                if (
                    abs(current_hour - typ_hour) <= 2 or abs(current_hour - typ_hour) >= 22
                ):  # wrap around
                    hour_match = True
                    break

            if not hour_match:
                return (
                    False,
                    f"Außerhalb typischer Online-Zeiten (typisch: {typical_hours}h)",
                )

            # Wochentag ist optional (falls verfügbar, prüfen wir es)
            if typical_days and current_day not in typical_days:
                day_names = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]
                return (
                    False,
                    f"Unpassender Wochentag (typisch: {[day_names[d] for d in typical_days]})",
                )

            return True, "OK - User kann gepingt werden"

        except Exception as e:
            logger.error(f"Error checking ping eligibility for {user_id}: {e}", exc_info=True)
            return False, f"Fehler: {e}"

    async def generate_ping_message(
        self,
        user: discord.Member,
        reason: str = "join",
        co_players: list[discord.Member] | None = None,
    ) -> str:
        """Generiert eine menschlich klingende Ping-Nachricht mit KI."""
        # Kontext fuer KI aufbauen
        context_parts = []
        if co_players:
            names = ", ".join([m.display_name for m in co_players[:3]])
            if len(co_players) > 3:
                names += f" und {len(co_players) - 3} weitere"
            context_parts.append(f"Deine Mitspieler {names} sind gerade online")

        reason_texts = {
            "join": "zum Spielen einladen",
            "game_ready": "dass ein Game startet",
            "friends_online": "dass Freunde online sind",
            "ranked_session": "zu einer Ranked Session einladen",
        }
        reason_text = reason_texts.get(reason, "zum Spielen einladen")

        system_prompt = 'Du bist ein freundlicher Discord-Bot der Spieler zum Gaming einlaedt.\n\nSchreibe eine kurze, lockere Nachricht auf Deutsch die:\n- Natuerlich und menschlich klingt (keine AI-Sprache!)\n- Freundlich und einladend ist\n- Nicht nach Bot klingt\n- Umgangssprache nutzt (z.B. "Bock auf ne Runde?", "Hey, Zeit fuer Deadlock?")\n- Maximal 1-2 Saetze lang ist\n- Keine Emojis enthaelt (ausser hoechstens 1-2 am Anfang)\n\nSchreibe NUR die Nachricht, nichts anderes.'

        user_prompt = f"""Schreib eine Ping-Nachricht, um {user.display_name} {reason_text}.

Kontext:
{chr(10).join(context_parts) if context_parts else "Keine besonderen Infos"}

Wichtig: Die Nachricht soll locker und wie von einem Freund klingen, nicht wie von einem Bot!"""

        ai = getattr(self.bot, "get_cog", lambda name: None)("AIConnector")
        if not ai:
            return await self._generate_fallback_message(user, reason, co_players)

        try:
            text, _meta = await ai.generate_text(
                provider="openai",
                prompt=user_prompt,
                system_prompt=system_prompt,
                model="gpt-4o-mini",
                max_output_tokens=100,
                temperature=0.9,
            )
            if not text:
                return await self._generate_fallback_message(user, reason, co_players)
            return f"{user.mention} {text.strip()}"
        except Exception as e:
            logger.error(f"Error generating AI message: {e}", exc_info=True)
            return await self._generate_fallback_message(user, reason, co_players)

    async def _generate_fallback_message(
        self,
        user: discord.Member,
        reason: str,
        co_players: list[discord.Member] | None = None,
    ) -> str:
        """Fallback-Nachrichten ohne KI."""
        templates = {
            "join": [
                f"{user.mention} Hey, Bock auf ne Runde Deadlock?",
                f"{user.mention} Wir sind am zocken, hast du Zeit?",
                f"{user.mention} Lust auf ein Match?",
            ],
            "game_ready": [
                f"{user.mention} Game ist ready, bist du dabei?",
                f"{user.mention} Wir starten gleich, kommst du?",
            ],
            "friends_online": [
                f"{user.mention} Deine Crew ist online!",
                f"{user.mention} Deine Mitspieler warten schon",
            ],
        }

        import secrets

        options = templates.get(reason, templates["join"])
        base_msg = secrets.choice(options)

        # Co-Spieler erwähnen
        if co_players:
            names = ", ".join([m.display_name for m in co_players[:2]])
            base_msg += f" ({names} sind schon da)"

        return base_msg

    async def record_ping(self, user_id: int):
        """Speichert dass ein User gepingt wurde (für Rate-Limiting)."""
        try:
            central_db.execute(
                """
                UPDATE user_activity_patterns
                SET last_pinged_at = CURRENT_TIMESTAMP,
                    ping_count_30d = ping_count_30d + 1
                WHERE user_id = ?
                """,
                (user_id,),
            )
        except Exception as e:
            logger.error(f"Error recording ping for {user_id}: {e}", exc_info=True)

    # ========== COMMANDS ==========

    @commands.command(name="myactivity")
    async def my_activity_command(self, ctx, user: discord.Member | None = None):
        """Zeigt deine Aktivitätsmuster der letzten 2 Wochen."""
        target = user or ctx.author

        try:
            row = central_db.query_one(
                """
                SELECT typical_hours, typical_days, activity_score_2w,
                       sessions_count_2w, total_minutes_2w, last_active_at,
                       ping_count_30d, last_pinged_at
                FROM user_activity_patterns
                WHERE user_id = ?
                """,
                (target.id,),
            )

            if not row:
                await ctx.send(f"❌ Keine Aktivitätsdaten für {target.display_name} vorhanden.")
                return

            typical_hours = json.loads(row[0]) if row[0] else []
            typical_days = json.loads(row[1]) if row[1] else []
            activity_score = row[2] or 0
            total_minutes = row[4] or 0
            last_active_str = row[5]
            ping_count = row[6] or 0
            last_pinged_str = row[7]

            embed = discord.Embed(
                title=f"📊 Aktivitätsmuster - {target.display_name}",
                color=discord.Color.blue(),
            )

            # Typische Online-Zeiten
            if typical_hours:
                hours_str = ", ".join([f"{h}:00" for h in typical_hours])
                embed.add_field(name="🕐 Typische Online-Zeiten", value=hours_str, inline=False)

            # Typische Wochentage
            if typical_days:
                day_names = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]
                days_str = ", ".join([day_names[d] for d in typical_days])
                embed.add_field(name="📅 Typische Wochentage", value=days_str, inline=False)

            # Activity Score
            embed.add_field(
                name="⭐ Activity Score (2W)",
                value=f"{activity_score} Sessions",
                inline=True,
            )
            embed.add_field(
                name="⏱️ Gesamtzeit (2W)",
                value=f"{total_minutes // 60}h {total_minutes % 60}m",
                inline=True,
            )

            # Last Active
            if last_active_str:
                last_active = datetime.strptime(last_active_str, "%Y-%m-%d %H:%M:%S")
                time_ago = datetime.utcnow() - last_active
                days_ago = time_ago.days
                hours_ago = time_ago.seconds // 3600
                embed.add_field(
                    name="🔴 Zuletzt aktiv",
                    value=f"vor {days_ago}d {hours_ago}h",
                    inline=True,
                )

            # Ping Stats
            embed.add_field(name="📬 Pings (30d)", value=f"{ping_count}/3", inline=True)
            if last_pinged_str:
                embed.add_field(name="📬 Zuletzt gepingt", value=last_pinged_str, inline=True)

            # Co-Spieler
            co_players = await self.get_top_co_players(target.id, limit=5)
            if co_players:
                co_player_lines = []
                for co_id, sessions_together in co_players[:3]:
                    co_member = ctx.guild.get_member(co_id)
                    name = co_member.display_name if co_member else f"User {co_id}"
                    co_player_lines.append(f"**{name}** ({sessions_together}x zusammen)")

                embed.add_field(
                    name="👥 Top Mitspieler",
                    value="\n".join(co_player_lines),
                    inline=False,
                )

            embed.set_thumbnail(url=target.display_avatar.url)
            embed.set_footer(text="Daten der letzten 2 Wochen")
            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in myactivity command: {e}", exc_info=True)
            await ctx.send(f"❌ Fehler beim Abrufen der Daten: {e}")

    @commands.command(name="smartping")
    @commands.has_permissions(manage_messages=True)
    async def smart_ping_command(self, ctx, user: discord.Member, reason: str = "join"):
        """
        Pingt einen User mit einer personalisierten Nachricht (nur mit Permission).

        Usage: !smartping @User [reason]
        Reasons: join, game_ready, friends_online, ranked_session
        """
        # Check ob Ping erlaubt
        can_ping, check_reason = await self.should_ping_user(user.id)

        if not can_ping:
            await ctx.send(f"❌ Kann {user.display_name} nicht pingen: {check_reason}")
            return

        # Hole Co-Spieler die online sind
        co_players_data = await self.get_top_co_players(user.id, limit=10)
        online_co_players = []

        for co_id, _ in co_players_data:
            member = ctx.guild.get_member(co_id)
            if member and member.voice and member.voice.channel:
                online_co_players.append(member)

        # Generiere Nachricht
        message = await self.generate_ping_message(user, reason, online_co_players)

        # Sende Nachricht
        await ctx.send(message)

        # Record Ping
        await self.record_ping(user.id)

        logger.info(
            "Smart ping sent to %s by %s",
            _safe_log_value(user.display_name),
            _safe_log_value(ctx.author.display_name),
        )

    @commands.command(name="checkping")
    async def check_ping_command(self, ctx, user: discord.Member | None = None):
        """Prüft ob ein User gepingt werden kann."""
        target = user or ctx.author

        can_ping, reason = await self.should_ping_user(target.id)

        embed = discord.Embed(
            title=f"🔔 Ping-Check - {target.display_name}",
            color=discord.Color.green() if can_ping else discord.Color.red(),
        )

        embed.add_field(
            name="Status",
            value="✅ Kann gepingt werden" if can_ping else "❌ Kann nicht gepingt werden",
            inline=False,
        )

        embed.add_field(name="Grund", value=reason, inline=False)

        await ctx.send(embed=embed)

    # ========== NEW COMMANDS: USER ANALYSIS ==========

    @commands.command(name="useranalysis", aliases=["ua", "analyze"])
    async def user_analysis_command(self, ctx, user: discord.Member | None = None):
        """
        Zeigt eine umfassende Analyse eines Users.
        Zeigt alle Aktivitäten: Joins, Leaves, Voice, Messages, Co-Spieler, Bans etc.

        Usage: !useranalysis [@User]
        """
        target = user or ctx.author

        try:
            embed = await self._generate_user_analysis_embed(target)
            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in user_analysis command: {e}", exc_info=True)
            await ctx.send(f"❌ Fehler beim Erstellen der Analyse: {e}")

    @commands.command(name="memberevents", aliases=["mevents"])
    async def member_events_command(self, ctx, user: discord.Member | None = None, limit: int = 10):
        """
        Zeigt die letzten Member-Events eines Users (Joins, Leaves, Bans).

        Usage: !memberevents [@User] [limit]
        """
        target = user or ctx.author

        try:
            events = central_db.query_all(
                """
                SELECT event_type, timestamp, display_name
                FROM member_events
                WHERE user_id = ? AND guild_id = ?
                ORDER BY timestamp DESC
                LIMIT ?
                """,
                (target.id, ctx.guild.id, min(limit, 50)),
            )

            if not events:
                await ctx.send(f"❌ Keine Events für {target.display_name} gefunden.")
                return

            embed = discord.Embed(
                title=f"📋 Member-Events - {target.display_name}",
                color=discord.Color.blue(),
            )

            event_icons = {"join": "➕", "leave": "➖", "ban": "🔨", "unban": "✅"}

            desc = ""
            for event_type, timestamp, display_name in events:
                icon = event_icons.get(event_type, "•")
                timestamp_str = timestamp[:16] if timestamp else "Unbekannt"
                desc += f"{icon} **{event_type.upper()}** - {timestamp_str}\n"

            embed.description = desc
            embed.set_thumbnail(url=target.display_avatar.url)
            embed.set_footer(text=f"Zeige {len(events)} von max {limit} Events")

            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in member_events command: {e}", exc_info=True)
            await ctx.send(f"❌ Fehler beim Abrufen der Events: {e}")

    @commands.command(name="tleaderboard", aliases=["tlb", "texttop"])
    @commands.cooldown(1, 15, commands.BucketType.guild)
    async def text_leaderboard_command(self, ctx):
        try:
            rows = central_db.query_all(
                """
                SELECT user_id, total_messages, total_points
                FROM text_stats
                ORDER BY total_points DESC, total_messages DESC
                LIMIT 10
                """
            )

            embed = discord.Embed(
                title=f"🏆 Text-Leaderboard - {ctx.guild.name}",
                color=discord.Color.gold(),
            )

            if rows:
                desc_lines = []
                for i, (user_id, total_messages, total_points) in enumerate(rows, 1):
                    member = ctx.guild.get_member(int(user_id))
                    name = member.display_name if member else "Unbekannt"
                    medal = "🥇" if i == 1 else "🥈" if i == 2 else "🥉" if i == 3 else f"{i}."
                    desc_lines.append(
                        f"{medal} **{name}** — {_format_leaderboard_number(total_messages or 0)} Msgs · "
                        f"{_format_leaderboard_number(total_points or 0)} Punkte"
                    )
                embed.description = "\n".join(desc_lines)
            else:
                embed.description = "📊 Noch keine Text-Aktivität aufgezeichnet."

            embed.set_footer(text=self._text_leaderboard_footer(ctx.author.id))
            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in tleaderboard: {e}", exc_info=True)
            await ctx.send(f"❌ Fehler beim Abrufen des Leaderboards: {e}")

    @commands.command(name="messagestats", aliases=["msgstats"])
    async def message_stats_command(self, ctx, user: discord.Member | None = None):
        """
        Zeigt Message-Statistiken eines Users.

        Usage: !messagestats [@User]
        """
        target = user or ctx.author

        try:
            stats = central_db.query_one(
                """
                SELECT message_count, last_message_at, first_message_at, channel_id
                FROM message_activity
                WHERE user_id = ? AND guild_id = ?
                """,
                (target.id, ctx.guild.id),
            )

            if not stats or not stats[0]:
                await ctx.send(f"❌ Keine Message-Aktivität für {target.display_name} gefunden.")
                return

            msg_count = stats[0]
            last_msg_at = stats[1][:16] if stats[1] else "Unbekannt"
            first_msg_at = stats[2][:16] if stats[2] else "Unbekannt"
            channel_id = stats[3]

            embed = discord.Embed(
                title=f"💬 Message-Statistiken - {target.display_name}",
                color=discord.Color.green(),
            )

            embed.add_field(name="📊 Nachrichten", value=f"{msg_count:,}", inline=True)
            embed.add_field(name="📅 Erste Nachricht", value=first_msg_at, inline=True)
            embed.add_field(name="🕐 Letzte Nachricht", value=last_msg_at, inline=True)

            if channel_id:
                channel = ctx.guild.get_channel(channel_id)
                if channel:
                    embed.add_field(name="📍 Letzter Channel", value=channel.mention, inline=True)

            # Durchschnitt pro Tag
            if stats[1] and stats[2]:
                try:
                    first = datetime.strptime(stats[2], "%Y-%m-%d %H:%M:%S")
                    last = datetime.strptime(stats[1], "%Y-%m-%d %H:%M:%S")
                    days = max(1, (last - first).days)
                    avg_per_day = msg_count / days
                    embed.add_field(
                        name="📈 Durchschnitt/Tag",
                        value=f"{avg_per_day:.1f}",
                        inline=True,
                    )
                except Exception as exc:
                    logger.debug(
                        "Konnte Durchschnitt/Tag nicht berechnen: %s",
                        exc,
                        exc_info=True,
                    )

            embed.set_thumbnail(url=target.display_avatar.url)
            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in message_stats command: {e}", exc_info=True)
            await ctx.send(f"❌ Fehler beim Abrufen der Statistiken: {e}")

    @commands.command(name="serverstats")
    @commands.has_permissions(manage_guild=True)
    async def server_stats_command(self, ctx):
        """
        Zeigt Server-weite Statistiken (nur für Admins).

        Usage: !serverstats
        """
        try:
            embed = discord.Embed(
                title=f"📊 Server-Statistiken - {ctx.guild.name}",
                color=discord.Color.gold(),
            )

            # Member Events
            event_counts = central_db.query_all(
                """
                SELECT event_type, COUNT(*) as count
                FROM member_events
                WHERE guild_id = ?
                GROUP BY event_type
                ORDER BY count DESC
                """,
                (ctx.guild.id,),
            )

            if event_counts:
                event_text = ""
                for event_type, count in event_counts:
                    event_text += f"**{event_type.upper()}:** {count}\n"
                embed.add_field(name="📋 Member Events", value=event_text, inline=True)

            # Message Activity
            total_messages = central_db.query_one(
                """
                SELECT SUM(message_count)
                FROM message_activity
                WHERE guild_id = ?
                """,
                (ctx.guild.id,),
            )

            if total_messages and total_messages[0]:
                embed.add_field(
                    name="💬 Nachrichten (gesamt)",
                    value=f"{total_messages[0]:,}",
                    inline=True,
                )

            # Voice Activity
            total_voice_time = central_db.query_one(
                """
                SELECT SUM(duration_seconds)
                FROM voice_session_log
                WHERE guild_id = ?
                """,
                (ctx.guild.id,),
            )

            if total_voice_time and total_voice_time[0]:
                hours = total_voice_time[0] // 3600
                embed.add_field(name="🎙️ Voice-Zeit (gesamt)", value=f"{hours:,}h", inline=True)

            # Top Active Users (nach Messages)
            top_users = central_db.query_all(
                """
                SELECT user_id, message_count
                FROM message_activity
                WHERE guild_id = ?
                ORDER BY message_count DESC
                LIMIT 5
                """,
                (ctx.guild.id,),
            )

            if top_users:
                top_text = ""
                for i, (user_id, msg_count) in enumerate(top_users, 1):
                    member = ctx.guild.get_member(user_id)
                    name = member.display_name if member else f"User {user_id}"
                    top_text += f"{i}. **{name}** - {msg_count:,} Messages\n"
                embed.add_field(name="🏆 Top 5 Aktivste User", value=top_text, inline=False)

            embed.set_footer(text=f"Server ID: {ctx.guild.id}")
            embed.timestamp = datetime.utcnow()

            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in server_stats command: {e}", exc_info=True)
            await ctx.send(f"❌ Fehler beim Abrufen der Server-Statistiken: {e}")

    @commands.command(name="rawmember")
    @commands.has_permissions(manage_guild=True)
    async def raw_member_command(self, ctx, member: discord.Member):
        """Zeigt den rohen API-Response für einen Member (Debug: Join-Source Felder)."""
        try:
            raw = await self.bot.http.get_member(ctx.guild.id, member.id)
            # Nur interessante Felder ausgeben
            interesting = {
                k: v
                for k, v in raw.items()
                if k not in {"user", "roles", "avatar", "banner", "avatar_decoration_data"}
            }
            import pprint

            text = pprint.pformat(interesting, width=80)
            # In chunks senden wenn zu lang
            chunks = [text[i : i + 1900] for i in range(0, len(text), 1900)]
            for i, chunk in enumerate(chunks[:3]):
                await ctx.send(
                    f"```python\n# Raw Member Data ({i + 1}/{len(chunks)})\n{chunk}\n```"
                )
        except Exception as e:
            await ctx.send(f"❌ Fehler: {e}")


async def setup(bot):
    await bot.add_cog(UserActivityAnalyzer(bot))
