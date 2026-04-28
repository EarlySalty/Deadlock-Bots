import asyncio
import json
import logging
import os
import time
from collections import defaultdict, deque
from dataclasses import dataclass
from datetime import datetime, timedelta

import discord
from discord.ext import commands, tasks

from cogs import privacy_core as privacy

# zentrale DB-API (synchron, mit internem Lock), KEINE eigenen Tabellen-Anlagen hier!
from service import db as central_db

logger = logging.getLogger(__name__)


def _format_leaderboard_number(value: int) -> str:
    return f"{int(value):,}".replace(",", ".")


# ========= Zentrale Einzel-DB Pfad (nur zur Anzeige/Diagnose) =========
def central_db_path() -> str:
    # reine Anzeige – tatsächlicher Zugriff läuft über shared.db
    try:
        return central_db._db_file()
    except Exception:
        # Fallback NUR für Anzeige, nicht für Zugriff
        return os.path.expandvars(r"%USERPROFILE%\Documents\Deadlock\service\deadlock.sqlite3")


# ========= Defaults (werden pro Guild via kv_store überschrieben) =========
@dataclass
class VoiceTrackerConfig:
    min_users_for_tracking: int = 2
    grace_period_duration: int = 180  # 3 Minuten
    session_timeout: int = 300  # 5 Minuten
    afk_timeout: int = 1800  # aktuell ungenutzt
    special_role_id: int = 1313624729466441769
    max_sessions_per_user: int = 100


VOICE_FEEDBACK_ENABLED = str(os.getenv("VOICE_FEEDBACK_ENABLED", "1")).strip().lower() not in (
    "0",
    "false",
    "no",
)
VOICE_FEEDBACK_MIN_SECONDS = int(os.getenv("VOICE_FEEDBACK_MIN_SECONDS", "300"))
VOICE_FEEDBACK_RESPONSE_WINDOW = int(os.getenv("VOICE_FEEDBACK_RESPONSE_WINDOW", str(72 * 3600)))
VOICE_FEEDBACK_MAX_NAMES = 10
VOICE_FEEDBACK_FORWARD_USER_ID = int(
    os.getenv("VOICE_FEEDBACK_FORWARD_USER_ID", "662995601738170389")
)
VOICE_FEEDBACK_SECOND_MIN_DAYS = int(os.getenv("VOICE_FEEDBACK_SECOND_MIN_DAYS", "4"))


# ========= Feedback UI =========
class VoiceFeedbackModal(discord.ui.Modal):
    def __init__(
        self,
        cog: "VoiceActivityTrackerCog",
        request_id: int,
        forward_user_id: int | None,
    ):
        super().__init__(title="Kurzes Voice-Feedback")
        self.cog = cog
        self.request_id = request_id
        self.forward_user_id = forward_user_id
        self.q1 = discord.ui.TextInput(
            label="Wie war dein Eindruck?",
            style=discord.TextStyle.paragraph,
            max_length=500,
            required=True,
            placeholder="Kurz bewerten (1-10) und warum: Stimmung, Ablauf, Technik",
        )
        self.q2 = discord.ui.TextInput(
            label="Was sollen wir verbessern?",
            style=discord.TextStyle.paragraph,
            max_length=900,
            required=False,
            placeholder="1-2 klare Punkte: Moderation, Themen, Ablauf, Technik, Verhalten",
        )
        self.q3 = discord.ui.TextInput(
            label="Wie lief es mit den anderen?",
            style=discord.TextStyle.paragraph,
            max_length=900,
            required=False,
            placeholder="Highlights oder Probleme im Miteinander (gern mit Namen, falls relevant)",
        )
        self.q4 = discord.ui.TextInput(
            label="Noch etwas, das wir wissen sollten?",
            style=discord.TextStyle.paragraph,
            max_length=900,
            required=False,
            placeholder="Wünsche, Probleme, Ideen, was dir aufgefallen ist",
        )
        self.add_item(self.q1)
        self.add_item(self.q2)
        self.add_item(self.q3)
        self.add_item(self.q4)

    async def on_submit(self, interaction: discord.Interaction):
        content_lines = [
            f"1) {self.q1.value}".strip(),
            f"2) {self.q2.value}".strip() if self.q2.value else "2) —",
            f"3) {self.q3.value}".strip() if self.q3.value else "3) —",
            f"4) {self.q4.value}".strip() if self.q4.value else "4) —",
        ]
        combined = "\n".join(content_lines)

        try:
            central_db.execute(
                """
                INSERT INTO voice_feedback_responses(request_id, user_id, message_id, content)
                VALUES(?,?,?,?)
                """,
                (self.request_id, interaction.user.id, None, combined),
            )
            central_db.execute(
                "UPDATE voice_feedback_requests SET status='responded' WHERE id=?",
                (self.request_id,),
            )
        except Exception as exc:
            logger.error(f"Failed to store voice feedback modal response: {exc}")

        try:
            await interaction.response.send_message(
                "Danke für dein Feedback! 🙌\n\n"
                "Wenn sonst irgendwas sein sollte, kannst du dich jederzeit an unser Team wenden – hier beißt keiner und jeder hilft gerne! :) Falls es doch mal ein Problem geben sollte, wende dich bitte direkt an einen Community Moderator (bei kleineren Dingen), einen Moderator oder an den Owner. ❤️",
                ephemeral=True,
            )
        except Exception as exc:
            logger.debug("Konnte Feedback-Modal-Antwort nicht senden: %s", exc, exc_info=True)

        try:
            req_row = central_db.query_one(
                """
                SELECT co_player_names, channel_name, duration_seconds, request_type
                FROM voice_feedback_requests WHERE id=?
                """,
                (self.request_id,),
            )
            if req_row:
                co_player_names = req_row[0] or ""
                channel_name = req_row[1] or "Voice"
                duration_seconds = int(req_row[2] or 0)
                req_type = req_row[3] or "first"
            else:
                co_player_names = ""
                channel_name = "Voice"
                duration_seconds = 0
                req_type = "first"
        except Exception:
            co_player_names = ""
            channel_name = "Voice"
            duration_seconds = 0
            req_type = "first"

        if self.forward_user_id:
            await self.cog._forward_feedback(
                req_id=self.request_id,
                author=interaction.user,
                content=combined,
                request_type=req_type,
                co_player_names=co_player_names,
                channel_name=channel_name,
                duration_seconds=duration_seconds,
            )


class VoiceFeedbackView(discord.ui.View):
    def __init__(
        self,
        cog: "VoiceActivityTrackerCog",
        request_id: int,
        forward_user_id: int | None,
    ):
        # persistent view (restored on cog_load)
        super().__init__(timeout=None)
        self.cog = cog
        self.request_id = request_id
        self.forward_user_id = forward_user_id

    @discord.ui.button(
        label="Feedback ausfüllen",
        style=discord.ButtonStyle.primary,
        emoji="📝",
        custom_id="voice_feedback:start",
    )
    async def start_feedback(self, interaction: discord.Interaction, button: discord.ui.Button):
        if self.request_id:
            try:
                row = central_db.query_one(
                    "SELECT status, sent_at_ts FROM voice_feedback_requests WHERE id=?",
                    (self.request_id,),
                )
                if row:
                    status = (row[0] or "").lower()
                    sent_at_ts = int(row[1] or 0)
                    if status == "responded":
                        await interaction.response.send_message(
                            "Danke, dein Voice-Feedback ist schon angekommen. 👍",
                            ephemeral=True,
                        )
                        return
                    if (
                        VOICE_FEEDBACK_RESPONSE_WINDOW
                        and sent_at_ts
                        and (time.time() - sent_at_ts) > VOICE_FEEDBACK_RESPONSE_WINDOW
                    ):
                        await interaction.response.send_message(
                            "Dieses Feedback-Fenster ist abgelaufen. Schreib uns gern direkt, falls noch etwas offen ist.",
                            ephemeral=True,
                        )
                        return
            except Exception as exc:
                logger.debug(f"Voice feedback state lookup failed: {exc}")
        try:
            await interaction.response.send_modal(
                VoiceFeedbackModal(self.cog, self.request_id, self.forward_user_id)
            )
        except Exception as exc:
            logger.error(f"Failed to open voice feedback modal: {exc}")


# ========= Rate Limiter =========
class RateLimiter:
    def __init__(self, max_requests: int = 10, time_window: int = 60):
        self.max_requests = max_requests
        self.time_window = time_window
        self.requests = defaultdict(deque)

    def is_allowed(self, user_id: int) -> bool:
        now = time.time()
        user_requests = self.requests[user_id]
        while user_requests and user_requests[0] < now - self.time_window:
            user_requests.popleft()
        if len(user_requests) < self.max_requests:
            user_requests.append(now)
            return True
        return False

    def get_remaining_time(self, user_id: int) -> int:
        now = time.time()
        user_requests = self.requests[user_id]
        if not user_requests:
            return 0
        oldest_request = user_requests[0]
        return max(0, int(self.time_window - (now - oldest_request)))


# ========= Config Manager (per Guild in kv_store) =========
class ConfigManager:
    """
    Persistenz ausschließlich über zentrale DB (kv_store).
    ns='voice_cfg', k=str(guild_id), v=json mit Config.
    """

    NS = "voice_cfg"

    def __init__(self, defaults: VoiceTrackerConfig):
        self.defaults = defaults
        self._cache: dict[int, VoiceTrackerConfig] = {}

    def _load_from_db(self, guild_id: int) -> VoiceTrackerConfig | None:
        raw = central_db.get_kv(self.NS, str(guild_id))
        if not raw:
            return None
        try:
            d = json.loads(raw)
            return VoiceTrackerConfig(
                min_users_for_tracking=int(
                    d.get("min_users_for_tracking", self.defaults.min_users_for_tracking)
                ),
                grace_period_duration=int(
                    d.get("grace_period_duration", self.defaults.grace_period_duration)
                ),
                session_timeout=int(d.get("session_timeout", self.defaults.session_timeout)),
                afk_timeout=int(d.get("afk_timeout", self.defaults.afk_timeout)),
                special_role_id=int(d.get("special_role_id", self.defaults.special_role_id)),
                max_sessions_per_user=int(
                    d.get("max_sessions_per_user", self.defaults.max_sessions_per_user)
                ),
            )
        except Exception:
            return None

    def _save_to_db(self, guild_id: int, cfg: VoiceTrackerConfig) -> None:
        payload = {
            "min_users_for_tracking": cfg.min_users_for_tracking,
            "grace_period_duration": cfg.grace_period_duration,
            "session_timeout": cfg.session_timeout,
            "afk_timeout": cfg.afk_timeout,
            "special_role_id": cfg.special_role_id,
            "max_sessions_per_user": cfg.max_sessions_per_user,
        }
        central_db.set_kv(self.NS, str(guild_id), json.dumps(payload, separators=(",", ":")))

    async def get(self, guild_id: int) -> VoiceTrackerConfig:
        if guild_id in self._cache:
            return self._cache[guild_id]
        cfg = self._load_from_db(guild_id) or self.defaults
        # falls nicht vorhanden, sofort persistieren (ohne neue Tabellen zu erstellen)
        if self._load_from_db(guild_id) is None:
            self._save_to_db(guild_id, cfg)
        self._cache[guild_id] = cfg
        return cfg

    async def set(self, guild_id: int, field: str, value):
        cfg = await self.get(guild_id)
        if not hasattr(cfg, field):
            raise ValueError(f"Unknown field: {field}")
        setattr(cfg, field, value)
        self._save_to_db(guild_id, cfg)
        self._cache[guild_id] = cfg


# ========= Voice Cog (nur zentrale DB-Tabellen: voice_stats, kv_store) =========
class VoiceActivityTrackerCog(commands.Cog):
    """
    Voice-Tracking auf EINER zentralen DB:
      - aggregiert pro User in voice_stats (user_id, total_seconds, total_points, last_update)
      - Config pro Guild in kv_store(ns='voice_cfg')
    Keine Migration, keine Table-Erzeugung in diesem Cog.
    """

    def __init__(self, bot):
        self.bot = bot
        self.defaults = VoiceTrackerConfig()
        self.config_manager = ConfigManager(self.defaults)
        self._feedback_forward_user_id = VOICE_FEEDBACK_FORWARD_USER_ID or None

        # runtime state
        self.voice_sessions: dict[str, dict] = {}  # key=f"{user_id}:{guild_id}" → session dict
        self.grace_period_users: dict[str, dict] = {}
        self.rate_limiter = RateLimiter(max_requests=5, time_window=30)

        # Performance: Display Name Cache (TTL 5min, max 512 entries)
        self._display_name_cache: dict[int, tuple[str, float]] = {}  # user_id -> (name, timestamp)
        self._display_name_cache_ttl = 300  # 5 minutes

        self.session_stats = {
            "total_sessions_created": 0,
            "total_grace_periods": 0,
            "uptime_start": datetime.utcnow(),
        }

        logger.info("Enhanced Voice Activity Tracker initializing (DB-centralized)")

    def _drop_runtime_state(self, user_id: int) -> None:
        """Remove in-memory tracking state for an opted-out user."""
        key_prefix = f"{int(user_id)}:"
        for key in list(self.voice_sessions.keys()):
            if key.startswith(key_prefix):
                self.voice_sessions.pop(key, None)
        for key in list(self.grace_period_users.keys()):
            if key.startswith(key_prefix):
                self.grace_period_users.pop(key, None)
        self._display_name_cache.pop(int(user_id), None)

    async def cog_load(self):
        # shared.db initialisiert Schema beim connect() selbst - hier nur Smoke-Test:
        try:
            _ = central_db.query_one("SELECT 1")
        except Exception as e:
            logger.error(f"Central DB not available: {e}")
            raise

        # Background tasks (keine Backups/Migrationen hier)
        self.cleanup_sessions.start()
        self.update_sessions.start()
        self.grace_period_monitor.start()
        self.health_check.start()

        restored, resent = await self._restore_voice_feedback_views()
        if restored or resent:
            logger.info(
                f"Voice feedback buttons re-registered: restored={restored}, resent={resent}"
            )

        logger.info("Voice Activity Tracker initialized (DB-centralized)")

    async def cog_unload(self):
        """Cleanup: Cancel background tasks und warte auf sauberen Shutdown."""
        tasks_to_cancel = [
            self.cleanup_sessions,
            self.update_sessions,
            self.grace_period_monitor,
            self.health_check,
        ]
        for task in tasks_to_cancel:
            if task.is_running():
                task.cancel()

        # Warte auf sauberen Task-Shutdown (Race-Safe!)
        await asyncio.gather(
            *[
                task.wait_for_cancel() if hasattr(task, "wait_for_cancel") else asyncio.sleep(0)
                for task in tasks_to_cancel
                if task.is_running()
            ],
            return_exceptions=True,
        )

        # Invalidate Caches (verhindert stale data bei reload)
        self.config_manager._cache.clear()
        self._display_name_cache.clear()

        logger.info("Voice Activity Tracker unloaded (clean shutdown)")

    # ===== Helpers =====
    async def cfg(self, guild_id: int) -> VoiceTrackerConfig:
        return await self.config_manager.get(guild_id)

    async def has_grace_period_role(self, member: discord.Member) -> bool:
        cfg = await self.cfg(member.guild.id)
        return any(role.id == cfg.special_role_id for role in member.roles)

    async def _restore_voice_feedback_views(self) -> tuple[int, int]:
        """
        Re-attach persisted DM buttons to avoid 'Unknown integration' after restarts.
        Returns (restored_views, resent_prompts).
        """
        if not VOICE_FEEDBACK_ENABLED:
            return 0, 0
        cutoff = (
            int(time.time()) - VOICE_FEEDBACK_RESPONSE_WINDOW
            if VOICE_FEEDBACK_RESPONSE_WINDOW
            else 0
        )
        try:
            rows = central_db.query_all(
                """
                SELECT id, prompt_message_id, sent_at_ts, status,
                       user_id, guild_id, channel_name, co_player_names, duration_seconds, request_type
                FROM voice_feedback_requests
                WHERE status != 'responded'
                  AND (? <= 0 OR sent_at_ts >= ?)
                """,
                (VOICE_FEEDBACK_RESPONSE_WINDOW, cutoff),
            )
        except Exception as exc:
            logger.error(f"Voice feedback view restoration failed: {exc}")
            return 0, 0

        restored = 0
        resent = 0
        for row in rows:
            req_id = int(row[0])
            message_id = row[1]
            user_id = int(row[4] or 0)
            try:
                if message_id:
                    view = VoiceFeedbackView(self, req_id, self._feedback_forward_user_id)
                    self.bot.add_view(view, message_id=int(message_id))
                    restored += 1
                    continue
            except Exception as exc:
                logger.debug(f"Could not restore voice feedback view for req {req_id}: {exc}")
                await self._delete_old_prompt(user_id, message_id)
            # Fallback: resend a fresh prompt to the user
            try:
                did_resend = await self._resend_feedback_prompt(row)
                if did_resend:
                    resent += 1
            except Exception as exc:
                logger.debug(f"Resend failed for voice feedback req {req_id}: {exc}")
        return restored, resent

    async def _resend_feedback_prompt(self, row) -> bool:
        """Send a fresh feedback prompt if the old button could not be restored."""
        try:
            req_id = int(row[0])
            user_id = int(row[4] or 0)
        except Exception:
            return False

        user = self.bot.get_user(user_id)
        if not user:
            try:
                user = await self.bot.fetch_user(user_id)
            except Exception:
                return False
        if not user or getattr(user, "bot", False):
            return False

        # try to clean up stale prompt before re-sending
        try:
            old_message_id = row[1] if len(row) > 1 else None
            await self._delete_old_prompt(user_id, old_message_id)
        except Exception as exc:
            logger.debug(f"Old prompt delete failed for req {req_id}: {exc}")

        request_type = (row[9] or "first") if len(row) > 9 else "first"
        co_player_names = (row[7] or "-") if len(row) > 7 else "-"
        if not co_player_names or co_player_names.strip() in ("", "-", "none", "None"):
            return False
        channel_name = (row[6] or "Voice") if len(row) > 6 else "Voice"
        duration_seconds = int(row[8] or 0) if len(row) > 8 else 0
        display_name = getattr(user, "display_name", None) or getattr(
            user, "name", f"User {user_id}"
        )

        duration_min = max(1, duration_seconds // 60) if duration_seconds else "?"
        if request_type == "second":
            lines = [
                f"Hey {display_name}, danke dass du wieder im Voice warst.",
                "Kurzes Update: Was laeuft gut, was nervt, was sollen wir direkt anpassen?",
                "Klick auf den Button und schreib es in 1-2 Saetzen.",
            ]
        else:
            lines = [
                f"Hey {display_name}, schoen dich im Voice zu sehen!",
                "Was hat dir gefallen, was sollen wir besser machen?",
                "Button druecken und in 1-2 Saetzen dein Eindruck da lassen.",
            ]
        if co_player_names and co_player_names.strip() != "-":
            lines.append(f"Mit im Call: {co_player_names}")
        lines.append(f"Kanal: {channel_name} - Dauer: {duration_min} Min")
        lines.append("Button druecken, kurz tippen, fertig. Danke dir!")
        message_text = "\n\n".join(lines)

        error_message = None
        prompt_message_id = None
        try:
            dm = user.dm_channel or await user.create_dm()
            view = VoiceFeedbackView(self, req_id, self._feedback_forward_user_id)
            msg = await dm.send(message_text, view=view)
            prompt_message_id = msg.id
            status = "sent"
        except discord.Forbidden:
            status = "forbidden"
            error_message = "DMs deaktiviert"
        except Exception as exc:
            status = "error"
            error_message = str(exc)
            logger.error(f"Resend failed for voice feedback req {req_id}: {exc}")

        try:
            if status == "sent":
                central_db.execute(
                    """
                    UPDATE voice_feedback_requests
                    SET status=?, error_message=?, prompt_message_id=?, sent_at_ts=strftime('%s','now')
                    WHERE id=?
                    """,
                    (status, error_message, prompt_message_id, req_id),
                )
            else:
                # sent_at_ts bewusst nicht anfassen: Request fällt nach 72h
                # natürlich aus dem Wiederherstellungsfenster heraus.
                central_db.execute(
                    """
                    UPDATE voice_feedback_requests
                    SET status=?, error_message=?, prompt_message_id=?
                    WHERE id=?
                    """,
                    (status, error_message, prompt_message_id, req_id),
                )
        except Exception as exc:
            logger.debug(f"Failed to update voice feedback request {req_id} after resend: {exc}")

        return status == "sent"

    async def _delete_old_prompt(self, user_id: int, message_id: int | None) -> None:
        """Best-effort removal of an outdated DM prompt to avoid stale buttons."""
        if not message_id:
            return
        user = self.bot.get_user(user_id)
        if not user:
            try:
                user = await self.bot.fetch_user(user_id)
            except Exception:
                return
        if not user:
            return
        try:
            dm = user.dm_channel or await user.create_dm()
            msg = await dm.fetch_message(int(message_id))
            await msg.delete()
        except Exception:
            # swallow errors: message may already be gone or inaccessible
            return

    def calculate_points(self, seconds: int, peak_users: int) -> int:
        """Berechnet Punkte für eine abgeschlossene Session."""
        if seconds <= 0:
            return 0
        base_points = seconds // 60  # 1 Punkt pro Minute
        # kleiner Bonus bei hoher Aktivität im Channel
        if base_points > 0:
            if peak_users >= 5:
                base_points += max(1, base_points // 10)
            elif peak_users >= 3:
                base_points += max(1, base_points // 20)
        return max(0, base_points)

    def _finalize_session(self, session: dict, end_time: datetime):
        seconds = max(0, int((end_time - session["start_time"]).total_seconds()))
        if seconds <= 0:
            return 0, 0, False
        points = self.calculate_points(seconds, session.get("peak_users") or 1)
        try:
            was_first_session = not bool(
                central_db.query_one(
                    """
                    SELECT 1 FROM voice_stats WHERE user_id=?
                    UNION ALL
                    SELECT 1 FROM voice_session_log WHERE user_id=?
                    LIMIT 1
                    """,
                    (session["user_id"], session["user_id"]),
                )
            )
        except Exception as e:
            logger.debug(
                f"DB read failed on voice_stats presence for {session.get('user_id')}: {e}",
                exc_info=True,
            )
            was_first_session = False
        try:
            central_db.execute(
                """
                INSERT INTO voice_stats(user_id, total_seconds, total_points, last_update)
                VALUES(?, ?, ?, CURRENT_TIMESTAMP)
                ON CONFLICT(user_id) DO UPDATE SET
                  total_seconds = total_seconds + excluded.total_seconds,
                  total_points  = total_points  + excluded.total_points,
                  last_update   = CURRENT_TIMESTAMP
                """,
                (session["user_id"], seconds, points),
            )
        except Exception as e:
            logger.error(f"DB write failed on session finalize: {e}")

        # Historische Session protokollieren (f\u00fcr Verlauf im Dashboard)
        try:
            started_at = session.get("start_time")
            if isinstance(started_at, datetime):
                started_iso = started_at.strftime("%Y-%m-%d %H:%M:%S")
            else:
                started_iso = None
            ended_iso = end_time.strftime("%Y-%m-%d %H:%M:%S")
            user_counts_json = json.dumps(session.get("user_counts") or [])
            # Co-Spieler IDs: Set zu Liste für JSON
            co_player_ids_set = session.get("co_player_ids") or set()
            co_player_ids_json = json.dumps(list(co_player_ids_set))
            display_name = session.get("display_name")
            if not display_name:
                user_obj = self.bot.get_user(session.get("user_id"))
                display_name = getattr(user_obj, "display_name", None) if user_obj else None
            display_name = display_name or f"User {session.get('user_id')}"
            central_db.execute(
                """
                INSERT INTO voice_session_log(
                  user_id, display_name, guild_id, channel_id, channel_name,
                  started_at, ended_at, duration_seconds, points, peak_users, user_counts_json,
                  co_player_ids
                )
                VALUES(?,?,?,?,?,?,?,?,?,?,?,?)
                """,
                (
                    session.get("user_id"),
                    display_name,
                    session.get("guild_id"),
                    session.get("channel_id"),
                    session.get("channel_name"),
                    started_iso,
                    ended_iso,
                    seconds,
                    points,
                    session.get("peak_users"),
                    user_counts_json,
                    co_player_ids_json,
                ),
            )
        except Exception as e:
            logger.error(f"Failed to log voice session history: {e}")
        return seconds, points, was_first_session

    async def _resolve_co_player_names(
        self, guild: discord.Guild | None, co_player_ids
    ) -> list[str]:
        if not co_player_ids:
            return []

        names: list[str] = []
        seen = set()
        for uid in co_player_ids:
            try:
                uid_int = int(uid)
            except Exception:
                continue
            if uid_int in seen:
                continue
            seen.add(uid_int)
            name: str | None = None
            user_obj: discord.User | discord.Member | None = None
            if guild:
                try:
                    name = await self._resolve_display_name(guild, uid_int)
                except Exception as exc:
                    logger.debug(f"Failed to resolve co-player display name for {uid_int}: {exc}")
            if not name:
                user_obj = self.bot.get_user(uid_int)
                if not user_obj:
                    try:
                        user_obj = await self.bot.fetch_user(uid_int)
                    except (discord.HTTPException, discord.NotFound, discord.Forbidden):
                        user_obj = None
            if user_obj:
                name = user_obj.display_name
            names.append(name or f"User {uid_int}")
        return names

    def _build_feedback_test_session(self, ctx: commands.Context, target: discord.Member) -> dict:
        """
        Build a minimal session dict for manual feedback triggers.
        Uses the target's voice channel if available, otherwise falls back to the invoker.
        Always forces sending (even solo) so Admin-Tests funktionieren.
        """
        channel = None
        if isinstance(target, discord.Member) and target.voice and target.voice.channel:
            channel = target.voice.channel
        elif isinstance(getattr(ctx, "author", None), discord.Member):
            author = ctx.author  # type: ignore[assignment]
            if author.voice and author.voice.channel:
                channel = author.voice.channel

        channel_name = channel.name if channel else "Admin Test"
        channel_id = channel.id if channel else None

        co_player_ids = set()
        if channel:
            for m in channel.members:
                if m.bot or m.id == target.id:
                    continue
                co_player_ids.add(m.id)
        # Wenn niemand sonst da ist, fuer Tests wenigstens den Aufrufer eintragen
        if not co_player_ids and isinstance(getattr(ctx, "author", None), discord.Member):
            if ctx.author.id != target.id:
                co_player_ids.add(ctx.author.id)

        return {
            "user_id": target.id,
            "guild_id": ctx.guild.id if ctx.guild else None,
            "channel_id": channel_id,
            "channel_name": channel_name,
            "co_player_ids": co_player_ids,
            "peak_users": 1,
            "force_feedback_even_if_alone": True,
        }

    async def _purge_all_feedback_requests(self, user_id: int) -> None:
        """Remove all previous feedback prompts (any status) for this user before sending a new one."""
        try:
            rows = central_db.query_all(
                """
                SELECT id, prompt_message_id
                FROM voice_feedback_requests
                WHERE user_id=?
                """,
                (user_id,),
            )
        except Exception as exc:
            logger.debug(f"Could not load feedback requests for purge user {user_id}: {exc}")
            return

        for rid, msg_id in rows or []:
            try:
                await self._delete_old_prompt(user_id, msg_id)
            except Exception as exc:
                logger.debug(
                    "Konnte alten Feedback-Prompt %s für %s nicht löschen: %s",
                    msg_id,
                    user_id,
                    exc,
                )
            try:
                central_db.execute(
                    "DELETE FROM voice_feedback_responses WHERE request_id=?", (rid,)
                )
                central_db.execute("DELETE FROM voice_feedback_requests WHERE id=?", (rid,))
            except Exception as exc:
                logger.debug(f"Could not purge feedback request {rid} for user {user_id}: {exc}")

    async def _send_voice_feedback(self, session: dict, seconds: int, request_type: str = "first"):
        user_id = session.get("user_id")
        guild_id = session.get("guild_id")
        if not user_id:
            return

        guild = self.bot.get_guild(guild_id) if guild_id else None
        member = guild.get_member(user_id) if guild else None  # type: ignore[arg-type]
        user = member or self.bot.get_user(user_id)
        if not user:
            try:
                user = await self.bot.fetch_user(user_id)
            except (discord.HTTPException, discord.NotFound, discord.Forbidden):
                user = None
        if not user or getattr(user, "bot", False):
            return

        co_player_ids = session.get("co_player_ids") or set()
        force_even_if_alone = bool(session.get("force_feedback_even_if_alone"))
        if not co_player_ids and not force_even_if_alone:
            return
        names = await self._resolve_co_player_names(guild, co_player_ids)
        extra_count = 0
        if len(names) > VOICE_FEEDBACK_MAX_NAMES:
            extra_count = len(names) - VOICE_FEEDBACK_MAX_NAMES
            names = names[:VOICE_FEEDBACK_MAX_NAMES]
        co_player_text = ", ".join(names) if names else "-"
        if extra_count:
            co_player_text = f"{co_player_text} (+{extra_count} weitere)"

        display_name = getattr(user, "display_name", None) or getattr(
            user, "name", f"User {user_id}"
        )
        channel_name = session.get("channel_name") or "Voice"
        if request_type == "second":
            lines = [
                f"Hey {display_name}, danke fuer deine Voice-Runden.",
                "Kurzes Update: Was laeuft gut, was nervt, was sollen wir fixen?",
                "Button druecken und in 1-2 Saetzen Feedback dalassen.",
            ]
        else:
            lines = [
                f"Hey {display_name}!",
                "Wie waren deine ersten Runden bei uns? Wir würden mega gern wissen, wie's dir gefallen hat :)",
                "Hau einfach kurz auf den Button und lass uns wissen, was gut lief oder auch nicht und was vielleicht noch besser gehen könnte. Dauert nur ne Minute und wir freuen uns echt über deine Meinung ❤️",
            ]
        if names:
            lines.append(f"Mit im Call waren u.a.: {co_player_text}")
        message_text = "\n\n".join(lines)

        req_id: int | None = None
        try:
            await self._purge_all_feedback_requests(user_id)
            central_db.execute(
                """
                INSERT INTO voice_feedback_requests(
                  user_id, guild_id, channel_id, channel_name, co_player_names,
                  duration_seconds, request_type, status, error_message, prompt_message_id
                )
                VALUES(?,?,?,?,?,?,?,?,?,?)
                """,
                (
                    user_id,
                    guild_id,
                    session.get("channel_id"),
                    channel_name,
                    co_player_text,
                    seconds,
                    request_type,
                    "pending",
                    None,
                    None,
                ),
            )
            row = central_db.query_one(
                "SELECT id FROM voice_feedback_requests WHERE user_id=? ORDER BY id DESC LIMIT 1",
                (user_id,),
            )
            if row:
                req_id = int(row[0])
        except Exception as exc:
            logger.error(f"Failed to persist voice feedback request for {user_id}: {exc}")
            req_id = None

        status = "error"
        error_message = None
        prompt_message_id = None
        try:
            dm = user.dm_channel or await user.create_dm()
            view = (
                VoiceFeedbackView(self, req_id, self._feedback_forward_user_id) if req_id else None
            )
            msg = await dm.send(message_text, view=view)
            prompt_message_id = msg.id
            status = "sent"
        except discord.Forbidden:
            status = "forbidden"
            error_message = "DMs deaktiviert"
        except Exception as exc:
            status = "error"
            error_message = str(exc)
            logger.error(f"Failed to send voice feedback DM to {user_id}: {exc}")

        if req_id:
            try:
                central_db.execute(
                    """
                    UPDATE voice_feedback_requests
                    SET status=?, error_message=?, prompt_message_id=?
                    WHERE id=?
                    """,
                    (status, error_message, prompt_message_id, req_id),
                )
            except Exception as exc:
                logger.error(f"Failed to update voice feedback request {req_id}: {exc}")

    async def _maybe_send_second_feedback(self, session: dict, seconds: int):
        if not VOICE_FEEDBACK_ENABLED:
            return
        user_id = session.get("user_id")
        if not user_id:
            return

        # Nur fortfahren, wenn bereits eine erste Feedback-Anfrage existiert (nur neue User anfragen)
        try:
            had_first = central_db.query_one(
                "SELECT 1 FROM voice_feedback_requests WHERE user_id=? AND request_type='first' LIMIT 1",
                (user_id,),
            )
            if not had_first:
                return
        except Exception as exc:
            logger.debug(f"Second feedback check skipped (no first request): {exc}")
            return

        # schon gesendet?
        try:
            already = central_db.query_one(
                "SELECT 1 FROM voice_feedback_requests WHERE user_id=? AND request_type='second' LIMIT 1",
                (user_id,),
            )
            if already:
                return
        except Exception as exc:
            logger.debug(f"Second feedback check failed (existing request): {exc}")
            return

        # mind. 4 verschiedene Tage mit Voice?
        try:
            row = central_db.query_one(
                "SELECT COUNT(DISTINCT date(started_at)) FROM voice_session_log WHERE user_id=?",
                (user_id,),
            )
            distinct_days = int(row[0] or 0) if row else 0
            if distinct_days < VOICE_FEEDBACK_SECOND_MIN_DAYS:
                return
        except Exception as exc:
            logger.debug(f"Second feedback check failed (distinct days): {exc}")
            return

        await self._send_voice_feedback(session, seconds, "second")

    async def _forward_feedback(
        self,
        req_id: int,
        author: discord.User,
        content: str,
        request_type: str,
        co_player_names: str,
        channel_name: str,
        duration_seconds: int,
    ):
        if not self._feedback_forward_user_id:
            return
        # ensure we have co-player info for the forward (fallback to DB lookup)
        if not co_player_names or co_player_names.strip() in (
            "",
            "-",
            "—",
            "none",
            "None",
        ):
            try:
                row = central_db.query_one(
                    "SELECT co_player_names FROM voice_feedback_requests WHERE id=?",
                    (req_id,),
                )
                if row and row[0]:
                    co_player_names = row[0]
            except Exception as exc:
                logger.debug(
                    "Konnte Co-Player für Feedback-Forward nicht laden: %s",
                    exc,
                    exc_info=True,
                )
        try:
            target = self.bot.get_user(self._feedback_forward_user_id) or await self.bot.fetch_user(
                self._feedback_forward_user_id
            )
        except Exception as exc:
            logger.debug(f"Feedback forward target fetch failed: {exc}")
            return
        if not target:
            return

        try:
            duration_min = max(1, duration_seconds // 60) if duration_seconds else "?"
            lines = [
                f"📩 Neues Voice-Feedback (Req #{req_id}, Typ: {request_type})",
                f"Von: {author} ({author.id})",
                f"Kanal: {channel_name}",
                f"Dauer: {duration_min} Min",
                f"Mit im Call: {co_player_names or '—'}",
                "",
                "Antwort:",
                content,
            ]
            text = "\n".join(lines)
            await target.send(text)
        except Exception as exc:
            logger.debug(f"Feedback forward DM failed: {exc}")

    def is_user_active_basic(self, voice_state: discord.VoiceState) -> bool:
        if not voice_state or not voice_state.channel:
            return False
        if getattr(voice_state, "afk", False):
            return False
        is_muted_or_deaf = (
            voice_state.mute or voice_state.deaf or voice_state.self_mute or voice_state.self_deaf
        )
        return not is_muted_or_deaf

    async def is_user_active(self, member: discord.Member) -> bool:
        vs = member.voice
        if not vs or not vs.channel:
            return False
        if self.is_user_active_basic(vs):
            return True
        if await self.has_grace_period_role(member):
            grace_key = f"{member.id}:{member.guild.id}"
            if grace_key in self.grace_period_users:
                cfg = await self.cfg(member.guild.id)
                time_in_grace = (
                    datetime.utcnow() - self.grace_period_users[grace_key]["start_time"]
                ).total_seconds()
                if time_in_grace <= cfg.grace_period_duration:
                    return True
        return False

    async def start_grace_period(self, member: discord.Member):
        if not await self.has_grace_period_role(member):
            return
        grace_key = f"{member.id}:{member.guild.id}"
        if grace_key in self.grace_period_users:
            return
        self.grace_period_users[grace_key] = {
            "user_id": member.id,
            "guild_id": member.guild.id,
            "channel_id": member.voice.channel.id if member.voice else None,
            "start_time": datetime.utcnow(),
        }
        self.session_stats["total_grace_periods"] += 1
        # logger.info(f"Grace period started for {member.display_name} ({member.id})")

    async def end_grace_period(self, member_id: int, guild_id: int, reason: str = "timeout"):
        grace_key = f"{member_id}:{guild_id}"
        if grace_key in self.grace_period_users:
            del self.grace_period_users[grace_key]
            # logger.info(f"Grace period ended for {member_id} ({reason})")

    async def _resolve_display_name(self, guild: discord.Guild, user_id: int) -> str:
        """Resolve a stable display name for leaderboard rows (mit Cache)."""
        now = time.time()

        # Cache-Check
        if user_id in self._display_name_cache:
            cached_name, cached_ts = self._display_name_cache[user_id]
            if now - cached_ts < self._display_name_cache_ttl:
                return cached_name

        # Cache miss - resolve name
        member = guild.get_member(user_id)
        if member:
            name = member.display_name
            self._display_name_cache[user_id] = (name, now)
            return name

        try:
            member = await guild.fetch_member(user_id)
        except discord.NotFound:
            member = None
        except discord.HTTPException as e:
            logger.debug(f"Failed to fetch guild member {user_id}: {e}")
            member = None

        if member:
            name = member.display_name
            self._display_name_cache[user_id] = (name, now)
            return name

        user = self.bot.get_user(user_id)
        if user:
            name = user.display_name
            self._display_name_cache[user_id] = (name, now)
            return name

        try:
            user = await self.bot.fetch_user(user_id)
        except discord.NotFound:
            user = None
        except discord.HTTPException as e:
            logger.debug(f"Failed to fetch user {user_id}: {e}")
            user = None

        name = user.display_name if user else f"User {user_id}"
        self._display_name_cache[user_id] = (name, now)

        # Cache-Größenlimit: Behalte nur neueste 512 Einträge
        if len(self._display_name_cache) > 512:
            # Entferne älteste 25%
            sorted_entries = sorted(self._display_name_cache.items(), key=lambda x: x[1][1])
            to_remove = sorted_entries[:128]
            for uid, _ in to_remove:
                del self._display_name_cache[uid]

        return name

    def _voice_leaderboard_footer(self, user_id: int) -> str:
        row = central_db.query_one(
            "SELECT total_points, total_seconds FROM voice_stats WHERE user_id = ?",
            (user_id,),
        )
        if not row:
            return "Noch keine Punkte"

        total_points = int(row[0] or 0)
        total_seconds = int(row[1] or 0)
        rank_row = central_db.query_one(
            """
            SELECT COUNT(*) + 1
            FROM voice_stats
            WHERE total_points > ?
               OR (total_points = ? AND total_seconds > ?)
            """,
            (total_points, total_points, total_seconds),
        )
        rank = int(rank_row[0] or 1) if rank_row else 1
        return f"Du bist auf Platz {rank} · {_format_leaderboard_number(total_points)} Punkte"

    async def start_voice_session(self, member: discord.Member, channel: discord.VoiceChannel):
        key = f"{member.id}:{channel.guild.id}"
        if key not in self.voice_sessions:
            self.voice_sessions[key] = {
                "user_id": member.id,
                "display_name": member.display_name,
                "guild_id": channel.guild.id,
                "channel_id": channel.id,
                "channel_name": channel.name,
                "start_time": datetime.utcnow(),
                "last_update": datetime.utcnow(),
                "total_time": 0,  # Sekunden seit Start
                "peak_users": 1,
                "user_counts": [],
                "co_player_ids": set(),  # Set von User-IDs die zusammen gespielt haben
            }
            self.session_stats["total_sessions_created"] += 1
            # logger.info(f"Started voice session: {member.display_name} in {channel.name}")

    async def end_voice_session(self, member: discord.Member, guild_id: int):
        key = f"{member.id}:{guild_id}"
        session = self.voice_sessions.pop(key, None)
        if not session:
            await self.end_grace_period(member.id, guild_id, "voice_leave")
            return

        # finalisieren & persistieren
        end_time = datetime.utcnow()
        seconds, points, was_first_session = self._finalize_session(session, end_time)
        await self.end_grace_period(member.id, guild_id, "voice_leave")
        if VOICE_FEEDBACK_ENABLED and was_first_session and seconds >= VOICE_FEEDBACK_MIN_SECONDS:
            asyncio.create_task(self._send_voice_feedback(dict(session), seconds, "first"))
        asyncio.create_task(self._maybe_send_second_feedback(dict(session), seconds))

    # ===== Discord Events =====
    @commands.Cog.listener()
    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ):
        try:
            if member.bot:
                return
            if privacy.is_opted_out(member.id):
                self._drop_runtime_state(member.id)
                return

            # Logik für Grace-Start/-Ende bei (Un)Mute
            if before.channel and after.channel and before.channel == after.channel:
                was_muted = before.mute or before.self_mute or before.deaf or before.self_deaf
                is_muted = after.mute or after.self_mute or after.deaf or after.self_deaf
                if not was_muted and is_muted and await self.has_grace_period_role(member):
                    await self.start_grace_period(member)
                elif was_muted and not is_muted:
                    await self.end_grace_period(member.id, member.guild.id, "unmuted")

            # Channel-Wechsel
            if before.channel != after.channel:
                if before.channel:
                    await self.handle_voice_leave(member, before.channel)
                if after.channel:
                    await self.handle_voice_join(member, after.channel)
            elif after.channel:
                await self.update_channel_sessions(after.channel)

        except Exception as e:
            logger.error(f"Error in voice state update: {e}")

    async def handle_voice_join(self, member: discord.Member, channel: discord.VoiceChannel):
        await self.update_channel_sessions(channel)

    async def handle_voice_leave(self, member: discord.Member, channel: discord.VoiceChannel):
        await self.end_voice_session(member, channel.guild.id)
        await self.update_channel_sessions(channel)

    async def update_channel_sessions(self, channel: discord.VoiceChannel):
        if not channel.members:
            return

        # aktive User (ungemutet oder in Grace)
        active_users = []
        for m in channel.members:
            if m.bot:
                continue
            if await self.is_user_active(m):
                active_users.append(m)

        user_count = len(active_users)
        cfg = await self.cfg(channel.guild.id)

        # peak/avg-Statistik für laufende Sessions
        for member in active_users:
            k = f"{member.id}:{channel.guild.id}"
            if k in self.voice_sessions:
                s = self.voice_sessions[k]
                s["user_counts"].append(user_count)
                s["peak_users"] = max(s["peak_users"], user_count)

                # Track Co-Spieler (alle anderen aktiven User im Channel)
                co_player_ids = {m.id for m in active_users if m.id != member.id}
                s["co_player_ids"].update(co_player_ids)

        # Sessions starten/aktualisieren/enden
        if user_count >= cfg.min_users_for_tracking and active_users:
            for member in active_users:
                k = f"{member.id}:{channel.guild.id}"
                if k not in self.voice_sessions:
                    await self.start_voice_session(member, channel)
                if k in self.voice_sessions:
                    self.voice_sessions[k]["last_update"] = datetime.utcnow()

        # alle, die NICHT aktiv sind → Session ggf. beenden
        for member in channel.members:
            if member.bot:
                continue
            k = f"{member.id}:{channel.guild.id}"
            if k in self.voice_sessions and member not in active_users:
                await self.end_voice_session(member, channel.guild.id)

    @commands.Cog.listener()
    async def on_message(self, message: discord.Message):
        if message.author.bot or not VOICE_FEEDBACK_ENABLED:
            return
        if not isinstance(message.channel, discord.DMChannel):
            return
        if privacy.is_opted_out(message.author.id):
            return
        content = (message.content or "").strip()
        if not content:
            return
        try:
            row = central_db.query_one(
                """
                SELECT id, sent_at_ts, status, request_type, co_player_names, channel_name, duration_seconds
                FROM voice_feedback_requests
                WHERE user_id = ?
                ORDER BY sent_at_ts DESC
                LIMIT 1
                """,
                (message.author.id,),
            )
            if not row:
                return
            req_id = row[0]
            sent_at_ts = int(row[1] or 0)
            status = row[2] or ""
            req_type = row[3] or "first"
            co_player_names = row[4] or ""
            channel_name = row[5] or "Voice"
            duration_seconds = int(row[6] or 0)
            if sent_at_ts and (time.time() - sent_at_ts) > VOICE_FEEDBACK_RESPONSE_WINDOW:
                return
            should_ack = status != "responded"
            central_db.execute(
                """
                INSERT INTO voice_feedback_responses(request_id, user_id, message_id, content)
                VALUES(?,?,?,?)
                """,
                (req_id, message.author.id, message.id, content),
            )
            central_db.execute(
                "UPDATE voice_feedback_requests SET status='responded' WHERE id=?",
                (req_id,),
            )
            if should_ack:
                try:
                    await message.channel.send(
                        "Danke für dein Feedback! 🙌\n\n"
                        "Wenn sonst irgendwas sein sollte, kannst du dich jederzeit an unser Team wenden – hier beißt keiner und jeder hilft gerne! :) Falls es doch mal ein Problem geben sollte, wende dich bitte direkt an einen Community Moderator (bei kleineren Dingen), einen Moderator oder an den Owner. ❤️"
                    )
                except Exception as exc:
                    logger.debug(
                        "Konnte Feedback-Acknowledgement nicht senden: %s",
                        exc,
                        exc_info=True,
                    )
            if self._feedback_forward_user_id:
                asyncio.create_task(
                    self._forward_feedback(
                        req_id=req_id,
                        author=message.author,
                        content=content,
                        request_type=req_type,
                        co_player_names=co_player_names,
                        channel_name=channel_name,
                        duration_seconds=duration_seconds,
                    )
                )
        except Exception as e:
            logger.error(f"Failed to store voice feedback response: {e}")

    # ===== BACKGROUND TASKS =====
    @tasks.loop(minutes=2)
    async def update_sessions(self):
        if not self.voice_sessions:
            return
        now = datetime.utcnow()
        # lediglich Keep-Alive/Peak-Update – keine Punkteberechnung in DB
        for k, s in list(self.voice_sessions.items()):
            s["last_update"] = now

    @update_sessions.before_loop
    async def before_update_sessions(self):
        await self.bot.wait_until_ready()

    @tasks.loop(seconds=60)
    async def grace_period_monitor(self):
        if not self.grace_period_users:
            return
        now = datetime.utcnow()
        expired = []
        for k, g in self.grace_period_users.items():
            guild_id = g["guild_id"]
            cfg = await self.cfg(guild_id)
            if (now - g["start_time"]).total_seconds() >= cfg.grace_period_duration:
                expired.append((g["user_id"], guild_id))
        for uid, gid in expired:
            await self.end_grace_period(uid, gid, "timeout_3min")

    @grace_period_monitor.before_loop
    async def before_grace_period_monitor(self):
        await self.bot.wait_until_ready()

    @tasks.loop(minutes=5)
    async def cleanup_sessions(self):
        if not self.voice_sessions:
            return
        now = datetime.utcnow()

        # Gruppiere Sessions nach Guild (nur 1x Config-Lookup pro Guild statt pro Session!)
        guild_sessions = defaultdict(list)
        for k, s in self.voice_sessions.items():
            guild_sessions[s["guild_id"]].append((k, s))

        to_close = []
        for guild_id, sessions in guild_sessions.items():
            cfg = await self.cfg(guild_id)  # Nur 1x pro Guild!
            cutoff = now - timedelta(seconds=cfg.session_timeout)
            for k, s in sessions:
                if s["last_update"] < cutoff:
                    to_close.append(k)

        for k in to_close:
            s = self.voice_sessions.get(k)
            if not s:
                continue
            end_time = s["last_update"]
            seconds, points, was_first_session = self._finalize_session(s, end_time)
            session_copy = dict(s)
            user = self.bot.get_user(s["user_id"])
            if seconds > 0:
                display_name = user.display_name if user else s["user_id"]
                logger.info(
                    f"Cleaned up inactive session: {display_name} ({seconds}s, {points}pts)"
                )
            self.voice_sessions.pop(k, None)
            if (
                VOICE_FEEDBACK_ENABLED
                and was_first_session
                and seconds >= VOICE_FEEDBACK_MIN_SECONDS
            ):
                asyncio.create_task(self._send_voice_feedback(session_copy, seconds, "first"))
            asyncio.create_task(self._maybe_send_second_feedback(session_copy, seconds))

    @cleanup_sessions.before_loop
    async def before_cleanup_sessions(self):
        await self.bot.wait_until_ready()

    @tasks.loop(hours=2)
    async def health_check(self):
        try:
            _ = central_db.query_one("SELECT 1")
            active_sessions = len(self.voice_sessions)
            grace_periods = len(self.grace_period_users)
            uptime = datetime.utcnow() - self.session_stats["uptime_start"]
            logger.info(
                f"Health check: {active_sessions} sessions, {grace_periods} grace periods, uptime: {uptime}"
            )
        except Exception as e:
            logger.error(f"Health check failed: {e}")

    @health_check.before_loop
    async def before_health_check(self):
        await self.bot.wait_until_ready()

    # ===== COMMANDS =====
    @commands.command(name="vstats")
    @commands.cooldown(1, 10, commands.BucketType.user)
    async def voice_stats_command(self, ctx, user: discord.Member | None = None):
        if not self.rate_limiter.is_allowed(ctx.author.id):
            remaining = self.rate_limiter.get_remaining_time(ctx.author.id)
            await ctx.send(f"⏰ Rate limit reached. Try again in {remaining} seconds.")
            return

        target_user = user or ctx.author
        try:
            # Gesamtzeit (global über voice_stats)
            row = central_db.query_one(
                "SELECT total_seconds, total_points FROM voice_stats WHERE user_id=?",
                (target_user.id,),
            )
            total_seconds = int(row[0]) if row and row[0] else 0
            total_points = int(row[1]) if row and len(row) > 1 and row[1] is not None else 0

            # Live-Session addieren (nur Anzeige)
            session_key = f"{target_user.id}:{ctx.guild.id}"
            live_add = 0
            live_info = ""
            live_points = 0
            if session_key in self.voice_sessions:
                s = self.voice_sessions[session_key]
                live_add = int((datetime.utcnow() - s["start_time"]).total_seconds())
                live_points = self.calculate_points(live_add, s.get("peak_users") or 1)
                live_info = f"🔴 Live: +{live_add // 60}m / +{live_points}pts"

            total = total_seconds + live_add
            total_hours = total // 3600
            total_minutes = (total % 3600) // 60
            total_points_display = total_points + live_points

            embed = discord.Embed(
                title=f"📊 Voice-Statistiken - {target_user.display_name}",
                color=discord.Color.blue(),
            )
            embed.add_field(
                name="⏱️ Gesamtzeit",
                value=f"{total_hours}h {total_minutes}m",
                inline=True,
            )
            embed.add_field(name="⭐ Punkte", value=str(total_points_display), inline=True)
            if live_info:
                embed.add_field(name="Status", value=live_info, inline=True)

            # Grace-Info
            has_role = await self.has_grace_period_role(target_user)
            if has_role:
                embed.add_field(
                    name="🎖️ Spezielle Rolle",
                    value="Grace Period berechtigt (3 Min Schutz)",
                    inline=False,
                )

            embed.set_thumbnail(url=target_user.display_avatar.url)
            embed.set_footer(text=f"Angefragt von {ctx.author.display_name}")
            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in vstats: {e}")
            await ctx.send(f"❌ Fehler beim Abrufen der Statistiken: {e}")

    @commands.command(name="vleaderboard", aliases=["vlb", "voicetop"])
    @commands.cooldown(1, 15, commands.BucketType.guild)
    async def voice_leaderboard_command(self, ctx):
        if not self.rate_limiter.is_allowed(ctx.author.id):
            remaining = self.rate_limiter.get_remaining_time(ctx.author.id)
            await ctx.send(f"⏰ Rate limit reached. Try again in {remaining} seconds.")
            return

        limit = 10

        try:
            rows = central_db.query_all(
                """
                SELECT user_id, total_seconds, total_points
                FROM voice_stats
                ORDER BY total_points DESC, total_seconds DESC
                LIMIT ?
                """,
                (limit,),
            )
            embed = discord.Embed(
                title=f"🏆 Voice-Leaderboard - {ctx.guild.name}",
                color=discord.Color.gold(),
            )

            if rows:
                desc = ""
                for i, (uid, secs, pts) in enumerate(rows, 1):
                    name = await self._resolve_display_name(ctx.guild, uid)
                    hours = (secs or 0) // 3600
                    minutes = ((secs or 0) % 3600) // 60
                    medal = "🥇" if i == 1 else "🥈" if i == 2 else "🥉" if i == 3 else f"{i}."
                    points_display = _format_leaderboard_number(pts or 0)
                    desc += f"{medal} **{name}** — {hours}h {minutes}m · {points_display} Punkte\n"
                embed.description = desc
            else:
                embed.description = "📊 Noch keine Voice-Aktivität aufgezeichnet."

            embed.set_footer(text=self._voice_leaderboard_footer(ctx.author.id))
            await ctx.send(embed=embed)

        except Exception as e:
            logger.error(f"Error in vleaderboard: {e}")
            await ctx.send(f"❌ Fehler beim Abrufen des Leaderboards: {e}")

    @commands.command(name="vtest")
    async def voice_test_command(self, ctx):
        embed = discord.Embed(title="🔧 Voice System Test (Central DB)", color=0x00FF99)
        try:
            _ = central_db.query_one("SELECT 1")
            db_ok = True
        except Exception:
            db_ok = False
        embed.add_field(
            name="🗄️ Database",
            value="✅ Verbunden" if db_ok else "❌ Fehler",
            inline=True,
        )
        embed.add_field(name="🔴 Live Sessions", value=len(self.voice_sessions), inline=True)

        cfg = await self.cfg(ctx.guild.id)
        embed.add_field(name="⏱️ Grace Duration", value=f"{cfg.grace_period_duration}s", inline=True)
        embed.add_field(name="🎖️ Special Role", value=f"<@&{cfg.special_role_id}>", inline=True)
        embed.add_field(name="👥 Min Users", value=cfg.min_users_for_tracking, inline=True)

        session_key = f"{ctx.author.id}:{ctx.guild.id}"
        if session_key in self.voice_sessions:
            s = self.voice_sessions[session_key]
            duration = int((datetime.utcnow() - s["start_time"]).total_seconds())
            session_info = f"🔴 **Aktive Session**\n⏱️ {duration // 60}m"
        else:
            session_info = "⭕ Keine aktive Session"
        embed.add_field(name="📊 Deine Session", value=session_info, inline=False)

        if ctx.author.voice:
            active_users = []
            for m in ctx.author.voice.channel.members:
                if not m.bot and await self.is_user_active(m):
                    active_users.append(m)
            voice_info = (
                f"🎵 **{ctx.author.voice.channel.name}**\n"
                f"👥 {len(ctx.author.voice.channel.members)} User "
                f"({len(active_users)} aktiv)\n"
                f"✅ Du bist aktiv: {await self.is_user_active(ctx.author)}"
            )
        else:
            voice_info = "❌ Nicht in Voice"
        embed.add_field(name="🎧 Voice Status", value=voice_info, inline=False)

        uptime = datetime.utcnow() - self.session_stats["uptime_start"]
        stats_info = (
            f"🕐 Uptime: {uptime.days}d {uptime.seconds // 3600}h\n"
            f"📈 Sessions erstellt: {self.session_stats['total_sessions_created']}\n"
            f"🛡️ Grace Periods: {self.session_stats['total_grace_periods']}"
        )
        embed.add_field(name="📊 System Stats", value=stats_info, inline=True)

        embed.add_field(name="📁 DB Path", value=f"...{central_db_path()[-40:]}", inline=False)
        embed.set_footer(text="Enhanced Voice Activity Tracker (Central DB)")
        await ctx.send(embed=embed)

    # ===== ADMIN COMMANDS (Konfig nur über kv_store) =====
    @commands.command(name="vf1")
    @commands.has_permissions(administrator=True)
    async def voice_feedback_day1_test(self, ctx, target: discord.Member | None = None):
        """Admin test: send the first-day voice feedback prompt to yourself or a mentioned user."""
        if not VOICE_FEEDBACK_ENABLED:
            await ctx.send("Voice feedback ist aktuell deaktiviert (VOICE_FEEDBACK_ENABLED=0).")
            return
        if not ctx.guild:
            await ctx.send("Dieser Testbefehl funktioniert nur im Server.")
            return

        target_user = target or ctx.author
        if getattr(target_user, "bot", False):
            await ctx.send("Kann kein Feedback an Bots schicken.")
            return

        try:
            session = self._build_feedback_test_session(ctx, target_user)
            await self._send_voice_feedback(session, max(VOICE_FEEDBACK_MIN_SECONDS, 300), "first")
            await ctx.send(
                f"Feedback-Test (Tag 1) an {target_user.mention} geschickt. Bitte DMs pruefen."
            )
        except Exception as exc:
            logger.error(f"Failed to send vf1 test prompt: {exc}")
            await ctx.send(f"Fehler beim Senden des Feedback-Tests: {exc}")

    @commands.command(name="vf4")
    @commands.has_permissions(administrator=True)
    async def voice_feedback_day4_test(self, ctx, target: discord.Member | None = None):
        """Admin test: send the fourth-day voice feedback prompt to yourself or a mentioned user."""
        if not VOICE_FEEDBACK_ENABLED:
            await ctx.send("Voice feedback ist aktuell deaktiviert (VOICE_FEEDBACK_ENABLED=0).")
            return
        if not ctx.guild:
            await ctx.send("Dieser Testbefehl funktioniert nur im Server.")
            return

        target_user = target or ctx.author
        if getattr(target_user, "bot", False):
            await ctx.send("Kann kein Feedback an Bots schicken.")
            return

        try:
            session = self._build_feedback_test_session(ctx, target_user)
            await self._send_voice_feedback(session, max(VOICE_FEEDBACK_MIN_SECONDS, 300), "second")
            await ctx.send(
                f"Feedback-Test (Tag 4) an {target_user.mention} geschickt. Bitte DMs pruefen."
            )
        except Exception as exc:
            logger.error(f"Failed to send vf4 test prompt: {exc}")
            await ctx.send(f"Fehler beim Senden des Feedback-Tests: {exc}")

    @commands.command(name="voice_status")
    @commands.has_permissions(administrator=True)
    async def voice_status_command(self, ctx):
        try:
            cfg = await self.cfg(ctx.guild.id)
            embed = discord.Embed(title="🔧 Voice System Admin Status (Central DB)", color=0x00FF99)
            embed.add_field(name="🔴 Live Sessions", value=len(self.voice_sessions), inline=True)
            embed.add_field(name="🛡️ Grace Periods", value=len(self.grace_period_users), inline=True)
            try:
                _ = central_db.query_one("SELECT 1")
                db_state = "Connected"
            except Exception:
                db_state = "Disconnected"
            embed.add_field(name="🗄️ Database", value=db_state, inline=True)

            embed.add_field(name="👥 Min Users", value=cfg.min_users_for_tracking, inline=True)
            embed.add_field(
                name="⏱️ Grace Duration",
                value=f"{cfg.grace_period_duration}s",
                inline=True,
            )
            embed.add_field(name="🎖️ Role ID", value=cfg.special_role_id, inline=True)

            uptime = datetime.utcnow() - self.session_stats["uptime_start"]
            embed.add_field(
                name="🕐 Uptime",
                value=f"{uptime.days}d {uptime.seconds // 3600}h",
                inline=True,
            )
            embed.add_field(name="📁 DB Path", value=f"...{central_db_path()[-40:]}", inline=True)
            await ctx.send(embed=embed)
        except Exception as e:
            await ctx.send(f"❌ Fehler beim Abrufen des Status: {e}")

    @commands.command(name="voice_config")
    @commands.has_permissions(administrator=True)
    async def voice_config_command(self, ctx, setting=None, value=None):
        cfg = await self.cfg(ctx.guild.id)
        if not setting:
            embed = discord.Embed(title="⚙️ Voice Tracker Config (Central DB)", color=0x0099FF)
            embed.add_field(name="👥 Min Users", value=cfg.min_users_for_tracking, inline=True)
            embed.add_field(
                name="⏱️ Grace Duration",
                value=f"{cfg.grace_period_duration}s",
                inline=True,
            )
            embed.add_field(name="🎖️ Special Role", value=cfg.special_role_id, inline=True)
            embed.add_field(name="🔄 Session Timeout", value=f"{cfg.session_timeout}s", inline=True)
            embed.add_field(name="📊 Max Sessions", value=cfg.max_sessions_per_user, inline=True)
            embed.add_field(
                name="Available Settings",
                value="```\n!voice_config grace_duration <seconds>\n!voice_config grace_role <role_id>\n!voice_config min_users <2-10>\n!voice_config session_timeout <seconds>\n!voice_config max_sessions <number>\n```",
                inline=False,
            )
            await ctx.send(embed=embed)
            return

        try:
            s = setting.lower().strip()
            if s == "grace_duration":
                duration = int(value)
                if 60 <= duration <= 600:
                    await self.config_manager.set(ctx.guild.id, "grace_period_duration", duration)
                    await ctx.send(
                        f"✅ Grace period duration set to {duration} seconds (zentral gespeichert)"
                    )
                else:
                    await ctx.send("❌ Grace duration must be between 60 and 600 seconds")
            elif s == "grace_role":
                role_id = int(value)
                await self.config_manager.set(ctx.guild.id, "special_role_id", role_id)
                await ctx.send(f"✅ Grace period role set to <@&{role_id}> (zentral gespeichert)")
            elif s == "min_users":
                min_users = int(value)
                if 2 <= min_users <= 10:
                    await self.config_manager.set(ctx.guild.id, "min_users_for_tracking", min_users)
                    await ctx.send(f"✅ Minimum users set to {min_users} (zentral gespeichert)")
                else:
                    await ctx.send("❌ Minimum users must be between 2 and 10")
            elif s == "session_timeout":
                to = int(value)
                if 60 <= to <= 3600:
                    await self.config_manager.set(ctx.guild.id, "session_timeout", to)
                    await ctx.send(f"✅ Session timeout set to {to}s (zentral gespeichert)")
                else:
                    await ctx.send("❌ Session timeout must be between 60 and 3600 seconds")
            elif s == "max_sessions":
                mx = int(value)
                if 10 <= mx <= 10000:
                    await self.config_manager.set(ctx.guild.id, "max_sessions_per_user", mx)
                    await ctx.send(f"✅ Max sessions set to {mx} (zentral gespeichert)")
                else:
                    await ctx.send("❌ Max sessions must be between 10 and 10000")
            else:
                await ctx.send(f"❌ Unknown setting: {setting}")
        except ValueError:
            await ctx.send("❌ Invalid value provided")
        except Exception as e:
            await ctx.send(f"❌ Error updating config: {e}")


async def setup(bot):
    await bot.add_cog(VoiceActivityTrackerCog(bot))
