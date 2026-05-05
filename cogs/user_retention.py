"""
User Retention Cog - "Wir vermissen dich"-Feature

Analysiert Voice-Aktivitätsmuster und sendet freundliche DMs an User,
die normalerweise aktiv sind aber länger als 2 Wochen nicht da waren.

Funktionsweise:
1. Trackt wann User zuletzt aktiv waren (Voice-Sessions)
2. Berechnet durchschnittliche Aktivität pro Woche
3. Erkennt "regelmäßig aktive" User (mind. 1x/Woche über 4 Wochen)
4. Sendet DM wenn solche User >14 Tage inaktiv sind
5. Opt-out Möglichkeit für User die keine DMs wollen
"""

import logging
import re
import time
from dataclasses import dataclass
from datetime import datetime
from typing import Any

import discord
from discord import app_commands
from discord.ext import commands, tasks

from cogs import privacy_core as privacy
from service import db as central_db

logger = logging.getLogger(__name__)


def _safe_log_value(value: Any) -> str:
    """Sanitize values before logging to prevent log injection attacks."""
    text = "" if value is None else str(value)
    return text.replace("\r", "\\r").replace("\n", "\\n")


# ========= Konfiguration =========
@dataclass
class RetentionConfig:
    # Ab wann gilt ein User als "regelmäßig aktiv"?
    min_weeks_for_regular: int = 4  # Mind. 4 Wochen Datenhistorie
    min_weekly_sessions: float = 0.5  # Mind. 0.5 Sessions/Woche (alle 2 Wochen)
    min_total_active_days: int = 3  # Mind. 3 verschiedene Tage aktiv gewesen

    # Ab wann senden wir "Wir vermissen dich"?
    inactivity_threshold_days: int = 14  # 14 Tage ohne Aktivität

    # Spam-Schutz
    min_days_between_messages: int = 30  # Frühestens nach 30 Tagen wieder
    max_miss_you_per_user: int = 1  # Max 1 "Vermissen dich"-Nachricht je User (nicht nerven!)

    # Check-Intervall
    check_hour: int = 12  # Um 12 Uhr mittags checken

    # Direktlinks zu Channels (feste URLs)
    server_link: str | None = "https://discord.com/channels/1289721245281292288/1289721245281292291"
    voice_link: str | None = "https://discord.com/channels/1289721245281292288/1501089974093873232"

    # Ausgeschlossene Rollen (bekommen keine Nachrichten)
    excluded_role_ids: tuple = (
        1304416311383818240,  # Ausgeschlossene Rolle 1
        1309741866098491479,  # Ausgeschlossene Rolle 2
    )


# ========= Feedback View =========
class FeedbackModal(discord.ui.Modal, title="Feedback geben"):
    """Modal für User-Feedback."""

    feedback = discord.ui.TextInput(
        label="Was können wir verbessern?",
        style=discord.TextStyle.paragraph,
        placeholder="Erzähl uns, was dich stört oder was wir besser machen können...",
        required=True,
        max_length=1000,
    )

    def __init__(self, guild_id: int, guild_name: str):
        super().__init__()
        self.guild_id = guild_id
        self.guild_name = guild_name

    async def on_submit(self, interaction: discord.Interaction):
        # Feedback in DB speichern
        now = int(time.time())
        try:
            central_db.execute(
                """
                INSERT INTO user_retention_messages
                (user_id, guild_id, message_type, sent_at, delivery_status, error_message)
                VALUES (?, ?, 'feedback', ?, 'received', ?)
                """,
                (interaction.user.id, self.guild_id, now, self.feedback.value),
            )
        except Exception as e:
            logger.error(f"Fehler beim Speichern des Feedbacks: {e}")

        await interaction.response.send_message(
            f"Danke für dein Feedback! Wir werden es uns anschauen und versuchen, **{self.guild_name}** für dich zu verbessern.",
            ephemeral=True,
        )
        logger.info(
            "Feedback erhalten von %s: %s...",
            interaction.user.id,
            _safe_log_value(self.feedback.value[:100]),
        )


class MissYouView(discord.ui.View):
    """View mit Buttons für die Miss-You-Nachricht."""

    def __init__(
        self,
        guild_id: int,
        guild_name: str,
        invite_url: str | None = None,
        server_link: str | None = None,
        voice_link: str | None = None,
    ):
        super().__init__(timeout=None)  # Persistent view
        self.guild_id = guild_id
        self.guild_name = guild_name

        # Direktlink zum Server-/Text-Channel
        if server_link:
            self.add_item(
                discord.ui.Button(
                    label="Zum Server",
                    style=discord.ButtonStyle.link,
                    url=server_link,
                    emoji="🏠",
                )
            )

        # Direktlink zum Voice-Channel
        if voice_link:
            self.add_item(
                discord.ui.Button(
                    label="Zum Voice",
                    style=discord.ButtonStyle.link,
                    url=voice_link,
                    emoji="🎧",
                )
            )

        # Fallback: Invite-Link (falls kein Channel-Link konfiguriert)
        if invite_url and not server_link:
            self.add_item(
                discord.ui.Button(
                    label="Zum Server",
                    style=discord.ButtonStyle.link,
                    url=invite_url,
                    emoji="🎮",
                )
            )

    @discord.ui.button(
        label="Feedback geben",
        style=discord.ButtonStyle.primary,
        emoji="💬",
        custom_id="retention_feedback",
    )
    async def feedback_button(self, interaction: discord.Interaction, button: discord.ui.Button):
        """Öffnet das Feedback-Modal."""
        await interaction.response.send_modal(FeedbackModal(self.guild_id, self.guild_name))

    # @discord.ui.button(label="Keine Nachrichten mehr", style=discord.ButtonStyle.secondary, emoji="🔕", custom_id="retention_optout_btn")
    # async def optout_button(self, interaction: discord.Interaction, button: discord.ui.Button):
    #     """Opt-out direkt aus der Nachricht."""
    #     now = int(time.time())
    #     central_db.execute(
    #         """
    #         INSERT INTO user_retention_tracking (user_id, guild_id, opted_out, updated_at)
    #         VALUES (?, ?, 1, ?)
    #         ON CONFLICT(user_id) DO UPDATE SET opted_out = 1, updated_at = ?
    #         """,
    #         (interaction.user.id, self.guild_id, now, now)
    #     )
    #     await interaction.response.send_message(
    #         "Okay, du bekommst keine solchen Nachrichten mehr von uns.",
    #         ephemeral=True
    #     )
    #     logger.info(f"User {interaction.user.id} hat sich per Button abgemeldet")


class UserRetentionCog(commands.Cog):
    """
    Erkennt inaktive User und sendet freundliche "Wir vermissen dich"-Nachrichten.
    """

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        self.config = RetentionConfig()
        self._last_check_date: str | None = None
        logger.info("UserRetention Cog initialisiert")

    async def cog_load(self):
        """Startet Background-Tasks beim Laden."""
        # DB-Check
        try:
            central_db.query_one("SELECT 1")
        except Exception as e:
            logger.error(f"DB nicht verfügbar: {e}")
            raise

        # Registriere persistent View für Buttons (funktioniert nach Bot-Restart)
        self.bot.add_view(MissYouView(0, "", None, self.config.server_link, self.config.voice_link))

        # Automatischer Check wieder aktiv
        self.daily_retention_check.start()
        self.sync_activity_data.start()
        logger.info("UserRetention Background-Tasks gestartet")

    async def cog_unload(self):
        """Stoppt Background-Tasks beim Entladen."""
        self.daily_retention_check.cancel()
        self.sync_activity_data.cancel()
        logger.info("UserRetention Cog entladen")

    # ========= Activity Tracking =========

    @commands.Cog.listener()
    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ):
        """
        Trackt Voice-Aktivität für Retention-Analyse.
        Wird parallel zum voice_activity_tracker ausgeführt.
        """
        if member.bot:
            return
        if privacy.is_opted_out(member.id):
            return

        # User ist einem Voice-Channel beigetreten (nicht nur gewechselt)
        if after.channel and (not before.channel or before.channel != after.channel):
            await self._update_user_activity(member)

    async def _update_user_activity(self, member: discord.Member):
        """Aktualisiert die Aktivitätsdaten eines Users."""
        now = int(time.time())
        today = datetime.utcnow().strftime("%Y-%m-%d")

        try:
            if privacy.is_opted_out(member.id):
                return
            # Prüfe ob User heute schon getrackt wurde
            row = central_db.query_one(
                "SELECT last_active_at, total_active_days FROM user_retention_tracking WHERE user_id = ?",
                (member.id,),
            )

            if row:
                last_active_ts = row[0]
                last_active_date = datetime.utcfromtimestamp(last_active_ts).strftime("%Y-%m-%d")
                total_days = row[1]

                # Nur wenn es ein neuer Tag ist, erhöhe total_active_days
                if last_active_date != today:
                    total_days += 1

                central_db.execute(
                    """
                    UPDATE user_retention_tracking
                    SET last_active_at = ?, total_active_days = ?, updated_at = ?
                    WHERE user_id = ?
                    """,
                    (now, total_days, now, member.id),
                )
            else:
                # Neuer User
                central_db.execute(
                    """
                    INSERT INTO user_retention_tracking
                    (user_id, guild_id, first_seen_at, last_active_at, total_active_days, updated_at)
                    VALUES (?, ?, ?, ?, 1, ?)
                    """,
                    (member.id, member.guild.id, now, now, now),
                )
        except Exception as e:
            logger.error(f"Fehler beim Update der Aktivität für {member.id}: {e}")

    # ========= Background Tasks =========

    @tasks.loop(minutes=30)
    async def sync_activity_data(self):
        """
        Synchronisiert Daten aus voice_session_log in user_retention_tracking.
        Berechnet avg_weekly_sessions für alle User.
        """
        try:
            # Hole alle User mit Voice-Aktivität der letzten 60 Tage
            sixty_days_ago = int(time.time()) - (60 * 24 * 60 * 60)

            rows = central_db.query_all(
                """
                SELECT
                    user_id,
                    guild_id,
                    COUNT(DISTINCT date(started_at)) as active_days,
                    COUNT(*) as total_sessions,
                    MIN(strftime('%s', started_at)) as first_session,
                    MAX(strftime('%s', started_at)) as last_session
                FROM voice_session_log
                WHERE strftime('%s', started_at) > ?
                GROUP BY user_id
                """,
                (sixty_days_ago,),
            )

            now = int(time.time())

            for row in rows:
                user_id = row[0]
                guild_id = row[1]
                active_days = row[2]
                total_sessions = row[3]
                first_session = int(row[4]) if row[4] else now
                last_session = int(row[5]) if row[5] else now

                # Berechne Wochen seit erstem Auftauchen
                weeks_active = max(1, (now - first_session) / (7 * 24 * 60 * 60))
                avg_weekly = total_sessions / weeks_active

                # Upsert in retention_tracking
                central_db.execute(
                    """
                    INSERT INTO user_retention_tracking
                    (user_id, guild_id, first_seen_at, last_active_at, total_active_days, avg_weekly_sessions, updated_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(user_id) DO UPDATE SET
                        last_active_at = MAX(user_retention_tracking.last_active_at, excluded.last_active_at),
                        total_active_days = MAX(user_retention_tracking.total_active_days, excluded.total_active_days),
                        avg_weekly_sessions = excluded.avg_weekly_sessions,
                        updated_at = excluded.updated_at
                    """,
                    (
                        user_id,
                        guild_id,
                        first_session,
                        last_session,
                        active_days,
                        avg_weekly,
                        now,
                    ),
                )

            logger.debug(f"Synced retention data for {len(rows)} users")

        except Exception as e:
            logger.error(f"Fehler beim Sync der Aktivitätsdaten: {e}")

    @sync_activity_data.before_loop
    async def before_sync(self):
        await self.bot.wait_until_ready()

    @tasks.loop(hours=1)
    async def daily_retention_check(self):
        """
        Prüft einmal täglich (zur konfigurierten Stunde) auf inaktive User.
        """
        current_hour = datetime.utcnow().hour
        today = datetime.utcnow().strftime("%Y-%m-%d")

        # Nur zur konfigurierten Stunde und nur einmal pro Tag
        if current_hour != self.config.check_hour:
            return
        if self._last_check_date == today:
            return

        self._last_check_date = today
        logger.info("Starte täglichen Retention-Check...")

        try:
            inactive_users = await self._find_inactive_regular_users()
            logger.info(f"Gefunden: {len(inactive_users)} inaktive reguläre User")

            sent_count = 0
            for user_id, guild_id, days_inactive, display_name in inactive_users:
                success = await self._send_miss_you_message(user_id, guild_id, days_inactive)
                if success:
                    sent_count += 1

            logger.info(f"Retention-Check abgeschlossen: {sent_count} Nachrichten gesendet")

        except Exception as e:
            logger.error(f"Fehler beim Retention-Check: {e}")

    @daily_retention_check.before_loop
    async def before_daily_check(self):
        await self.bot.wait_until_ready()

    # ========= Dashboard API =========

    async def get_retention_stats(self) -> dict:
        """Liefert Statistiken für das Dashboard."""
        now = int(time.time())
        inactivity_threshold = now - (self.config.inactivity_threshold_days * 24 * 60 * 60)

        total_tracked = central_db.query_one("SELECT COUNT(*) FROM user_retention_tracking")[0] or 0
        opted_out = (
            central_db.query_one(
                "SELECT COUNT(*) FROM user_retention_tracking WHERE opted_out = 1"
            )[0]
            or 0
        )

        regular_users = (
            central_db.query_one(
                "SELECT COUNT(*) FROM user_retention_tracking WHERE avg_weekly_sessions >= ? AND total_active_days >= ?",
                (self.config.min_weekly_sessions, self.config.min_total_active_days),
            )[0]
            or 0
        )

        inactive_regular = (
            central_db.query_one(
                """SELECT COUNT(*) FROM user_retention_tracking
               WHERE avg_weekly_sessions >= ? AND total_active_days >= ?
               AND last_active_at < ? AND opted_out = 0 AND miss_you_count < ?""",
                (
                    self.config.min_weekly_sessions,
                    self.config.min_total_active_days,
                    inactivity_threshold,
                    self.config.max_miss_you_per_user,
                ),
            )[0]
            or 0
        )

        messages_sent = (
            central_db.query_one(
                "SELECT COUNT(*) FROM user_retention_messages WHERE message_type = 'miss_you'"
            )[0]
            or 0
        )

        feedback_count = (
            central_db.query_one(
                "SELECT COUNT(*) FROM user_retention_messages WHERE message_type = 'feedback'"
            )[0]
            or 0
        )

        return {
            "total_tracked": total_tracked,
            "opted_out": opted_out,
            "regular_users": regular_users,
            "inactive_eligible": inactive_regular,
            "messages_sent": messages_sent,
            "feedback_count": feedback_count,
            "config": {
                "inactivity_days": self.config.inactivity_threshold_days,
                "min_days_between": self.config.min_days_between_messages,
                "max_messages": self.config.max_miss_you_per_user,
            },
        }

    async def get_inactive_users_list(self, limit: int = 50) -> list[dict]:
        """Liefert Liste der inaktiven User für das Dashboard."""
        users = await self._find_inactive_regular_users()
        result = []
        for user_id, guild_id, days_inactive, display_name in users[:limit]:
            result.append(
                {
                    "user_id": user_id,
                    "guild_id": guild_id,
                    "days_inactive": days_inactive,
                    "display_name": display_name,
                }
            )
        return result

    async def send_message_to_user(self, user_id: int, guild_id: int) -> dict:
        """Sendet manuell eine Nachricht an einen User (Dashboard-Trigger)."""
        # Hole Tage inaktiv
        row = central_db.query_one(
            "SELECT (strftime('%s','now') - last_active_at) / 86400 FROM user_retention_tracking WHERE user_id = ?",
            (user_id,),
        )
        days_inactive = int(row[0]) if row else 14

        success = await self._send_miss_you_message(user_id, guild_id, days_inactive)
        return {"success": success, "user_id": user_id, "days_inactive": days_inactive}

    async def run_retention_check_now(self) -> dict:
        """Führt den Retention-Check manuell aus (Dashboard-Trigger)."""
        logger.info("Manueller Retention-Check gestartet (Dashboard)")

        inactive_users = await self._find_inactive_regular_users()
        sent_count = 0
        failed_count = 0

        for user_id, guild_id, days_inactive, display_name in inactive_users:
            success = await self._send_miss_you_message(user_id, guild_id, days_inactive)
            if success:
                sent_count += 1
            else:
                failed_count += 1

        logger.info(
            f"Manueller Retention-Check abgeschlossen: {sent_count} gesendet, {failed_count} fehlgeschlagen"
        )
        return {
            "total_checked": len(inactive_users),
            "sent": sent_count,
            "failed": failed_count,
        }

    async def get_feedback_list(self, limit: int = 20) -> list[dict]:
        """Liefert die letzten Feedbacks für das Dashboard."""
        rows = central_db.query_all(
            """SELECT user_id, guild_id, sent_at, error_message
               FROM user_retention_messages
               WHERE message_type = 'feedback'
               ORDER BY sent_at DESC LIMIT ?""",
            (limit,),
        )
        result = []
        for row in rows:
            result.append(
                {
                    "user_id": row[0],
                    "guild_id": row[1],
                    "timestamp": row[2],
                    "feedback": row[3] or "",
                }
            )
        return result

    # ========= Core Logic =========

    async def _find_inactive_regular_users(self) -> list[tuple[int, int, int, str]]:
        """
        Findet User die:
        1. Regelmäßig aktiv waren (avg_weekly_sessions >= min_weekly_sessions)
        2. Mind. min_total_active_days aktiv waren
        3. Jetzt > inactivity_threshold_days inaktiv sind
        4. Nicht opted-out sind
        5. Nicht kürzlich eine Nachricht bekommen haben

        Returns: Liste von (user_id, guild_id, days_inactive, display_name)
        """
        now = int(time.time())
        inactivity_threshold = now - (self.config.inactivity_threshold_days * 24 * 60 * 60)
        min_time_between = self.config.min_days_between_messages * 24 * 60 * 60

        rows = central_db.query_all(
            """
            SELECT
                user_id,
                guild_id,
                last_active_at,
                (? - last_active_at) / 86400 as days_inactive
            FROM user_retention_tracking
            WHERE
                avg_weekly_sessions >= ?
                AND total_active_days >= ?
                AND last_active_at < ?
                AND opted_out = 0
                AND miss_you_count < ?
                AND (last_miss_you_sent_at IS NULL OR last_miss_you_sent_at < ?)
            ORDER BY days_inactive DESC
            LIMIT 50
            """,
            (
                now,
                self.config.min_weekly_sessions,
                self.config.min_total_active_days,
                inactivity_threshold,
                self.config.max_miss_you_per_user,
                now - min_time_between,
            ),
        )

        result = []
        for row in rows:
            user_id = row[0]
            guild_id = row[1]
            days_inactive = row[3]

            # Versuche Display-Name zu holen und Rollen zu prüfen
            display_name = None
            skip_user = False

            try:
                guild = self.bot.get_guild(guild_id)
                if guild:
                    member = guild.get_member(user_id)
                    if member:
                        display_name = member.display_name

                        # Prüfe ob User eine ausgeschlossene Rolle hat
                        member_role_ids = {role.id for role in member.roles}
                        if member_role_ids & set(self.config.excluded_role_ids):
                            skip_user = True
                            logger.debug(f"User {user_id} übersprungen (ausgeschlossene Rolle)")

                # Wenn kein Display-Name gefunden wurde, versuche User zu fetchen
                if not display_name:
                    try:
                        fetched_user = await self.bot.fetch_user(user_id)
                        # global_name ist der Display-Name, name ist der Username
                        display_name = fetched_user.global_name or fetched_user.name
                    except discord.NotFound:
                        logger.debug(f"User {user_id} nicht gefunden")
                    except Exception as e:
                        logger.debug(f"Konnte User {user_id} nicht fetchen: {e}")
            except Exception as e:
                logger.debug(f"Fehler beim Abrufen von User {user_id}: {e}")

            # Fallback falls nichts funktioniert hat
            if not display_name:
                display_name = "Unbekannt"

            if not skip_user:
                result.append((user_id, guild_id, days_inactive, display_name))

        return result

    async def _send_miss_you_message(self, user_id: int, guild_id: int, days_inactive: int) -> bool:
        """
        Sendet eine freundliche "Wir vermissen dich"-Nachricht per DM.
        Returns: True wenn erfolgreich gesendet.
        """
        now = int(time.time())

        try:
            if privacy.is_opted_out(user_id):
                return False
            user = await self.bot.fetch_user(user_id)
            guild = self.bot.get_guild(guild_id)
            guild_name = guild.name if guild else "unserem Server"

            # Versuche Invite-Link zu bekommen
            invite_url = None
            if guild:
                try:
                    # Suche nach einem permanenten Invite
                    invites = await guild.invites()
                    for inv in invites:
                        if inv.max_age == 0:  # Permanenter Invite
                            invite_url = str(inv)
                            break
                except discord.Forbidden:
                    pass  # Keine Berechtigung für Invites

            display_name = user.display_name
            if guild:
                member = guild.get_member(user_id)
                if member:
                    display_name = member.display_name

            embed = discord.Embed(
                title=f"Hey {display_name}, wir vermissen dich! :(",
                description=(
                    f"Dir ist bestimmt aufgefallen, dass du schon **{days_inactive} Tage** "
                    f"nicht mehr aktiv in der **{guild_name}** warst.\n\n"
                    f"Wir würden uns freuen, dich mal wieder im Voice oder Chat zu sehen.\n\n"
                    f"Falls dich etwas stört oder du Feedback hast, lass es uns bitte wissen "
                    f"- wir wollen den Server für dich besser machen."
                ),
                color=discord.Color.blue(),
            )

            if guild and guild.icon:
                embed.set_thumbnail(url=guild.icon.url)

            # View mit Buttons erstellen
            view = MissYouView(
                guild_id,
                guild_name,
                invite_url,
                self.config.server_link,
                self.config.voice_link,
            )

            await user.send(embed=embed, view=view)

            # Update Tracking
            central_db.execute(
                """
                UPDATE user_retention_tracking
                SET last_miss_you_sent_at = ?, miss_you_count = miss_you_count + 1, updated_at = ?
                WHERE user_id = ?
                """,
                (now, now, user_id),
            )

            # Log Message
            central_db.execute(
                """
                INSERT INTO user_retention_messages (user_id, guild_id, message_type, sent_at, delivery_status)
                VALUES (?, ?, 'miss_you', ?, 'sent')
                """,
                (user_id, guild_id, now),
            )

            logger.info(f"Miss-you Nachricht gesendet an {user_id} ({days_inactive} Tage inaktiv)")
            return True

        except discord.Forbidden:
            # User hat DMs deaktiviert
            central_db.execute(
                """
                INSERT INTO user_retention_messages (user_id, guild_id, message_type, sent_at, delivery_status, error_message)
                VALUES (?, ?, 'miss_you', ?, 'blocked', 'DMs disabled')
                """,
                (user_id, guild_id, now),
            )
            logger.debug(f"Konnte keine DM an {user_id} senden (DMs deaktiviert)")
            return False

        except Exception as e:
            central_db.execute(
                """
                INSERT INTO user_retention_messages (user_id, guild_id, message_type, sent_at, delivery_status, error_message)
                VALUES (?, ?, 'miss_you', ?, 'failed', ?)
                """,
                (user_id, guild_id, now, str(e)),
            )
            logger.error(f"Fehler beim Senden an {user_id}: {e}")
            return False

    # ========= Slash Commands =========

    @app_commands.command(
        name="retention-optout",
        description="Deaktiviere 'Wir vermissen dich'-Nachrichten",
    )
    async def retention_optout(self, interaction: discord.Interaction):
        """Erlaubt Usern, sich von Retention-Nachrichten abzumelden."""
        now = int(time.time())

        central_db.execute(
            """
            INSERT INTO user_retention_tracking (user_id, guild_id, opted_out, updated_at)
            VALUES (?, ?, 1, ?)
            ON CONFLICT(user_id) DO UPDATE SET opted_out = 1, updated_at = ?
            """,
            (interaction.user.id, interaction.guild_id or 0, now, now),
        )

        await interaction.response.send_message(
            "✅ Du erhältst ab jetzt keine 'Wir vermissen dich'-Nachrichten mehr.",
            ephemeral=True,
        )

    @app_commands.command(
        name="retention-optin",
        description="Aktiviere 'Wir vermissen dich'-Nachrichten wieder",
    )
    async def retention_optin(self, interaction: discord.Interaction):
        """Erlaubt Usern, sich wieder für Retention-Nachrichten anzumelden."""
        now = int(time.time())

        central_db.execute(
            """
            UPDATE user_retention_tracking
            SET opted_out = 0, updated_at = ?
            WHERE user_id = ?
            """,
            (now, interaction.user.id),
        )

        await interaction.response.send_message(
            "✅ Du erhältst wieder 'Wir vermissen dich'-Nachrichten wenn du länger inaktiv bist.",
            ephemeral=True,
        )

    # ========= Admin Commands =========

    @commands.command(name="retention_status")
    @commands.has_permissions(administrator=True)
    async def retention_status(self, ctx: commands.Context):
        """Zeigt den Status des Retention-Systems."""
        try:
            # Statistiken sammeln
            total_tracked = central_db.query_one("SELECT COUNT(*) FROM user_retention_tracking")[0]

            opted_out = central_db.query_one(
                "SELECT COUNT(*) FROM user_retention_tracking WHERE opted_out = 1"
            )[0]

            regular_users = central_db.query_one(
                """
                SELECT COUNT(*) FROM user_retention_tracking
                WHERE avg_weekly_sessions >= ? AND total_active_days >= ?
                """,
                (self.config.min_weekly_sessions, self.config.min_total_active_days),
            )[0]

            now = int(time.time())
            inactivity_threshold = now - (self.config.inactivity_threshold_days * 24 * 60 * 60)

            inactive_regular = central_db.query_one(
                """
                SELECT COUNT(*) FROM user_retention_tracking
                WHERE avg_weekly_sessions >= ?
                AND total_active_days >= ?
                AND last_active_at < ?
                AND opted_out = 0
                """,
                (
                    self.config.min_weekly_sessions,
                    self.config.min_total_active_days,
                    inactivity_threshold,
                ),
            )[0]

            messages_sent = central_db.query_one(
                "SELECT COUNT(*) FROM user_retention_messages WHERE message_type = 'miss_you'"
            )[0]

            messages_last_30d = central_db.query_one(
                """
                SELECT COUNT(*) FROM user_retention_messages
                WHERE message_type = 'miss_you' AND sent_at > ?
                """,
                (now - 30 * 24 * 60 * 60,),
            )[0]

            embed = discord.Embed(title="📊 User Retention Status", color=discord.Color.blue())
            embed.add_field(name="👥 Getrackte User", value=str(total_tracked), inline=True)
            embed.add_field(name="🔄 Reguläre User", value=str(regular_users), inline=True)
            embed.add_field(name="😴 Davon inaktiv", value=str(inactive_regular), inline=True)
            embed.add_field(name="🚫 Opted-out", value=str(opted_out), inline=True)
            embed.add_field(name="📨 Nachrichten (gesamt)", value=str(messages_sent), inline=True)
            embed.add_field(
                name="📨 Nachrichten (30 Tage)",
                value=str(messages_last_30d),
                inline=True,
            )

            embed.add_field(
                name="⚙️ Konfiguration",
                value=(
                    f"• Inaktivitätsschwelle: {self.config.inactivity_threshold_days} Tage\n"
                    f"• Min. wöchentliche Sessions: {self.config.min_weekly_sessions}\n"
                    f"• Min. aktive Tage: {self.config.min_total_active_days}\n"
                    f"• Check-Zeit: {self.config.check_hour}:00 UTC"
                ),
                inline=False,
            )

            await ctx.send(embed=embed)

        except Exception as e:
            await ctx.send(f"❌ Fehler: {e}")

    @commands.command(name="retention_preview")
    @commands.has_permissions(administrator=True)
    async def retention_preview(self, ctx: commands.Context, limit: int = 10):
        """Zeigt User die als nächstes eine Nachricht bekommen würden."""
        try:
            users = await self._find_inactive_regular_users()
            users = users[:limit]

            if not users:
                await ctx.send("✅ Keine inaktiven regulären User gefunden.")
                return

            embed = discord.Embed(
                title="📋 Nächste Retention-Kandidaten",
                description=f"Diese {len(users)} User würden als nächstes kontaktiert:",
                color=discord.Color.orange(),
            )

            lines = []
            for user_id, guild_id, days_inactive, display_name in users:
                lines.append(f"• **{display_name}** - {days_inactive} Tage inaktiv")

            embed.add_field(name="User", value="\n".join(lines) or "Keine", inline=False)
            await ctx.send(embed=embed)

        except Exception as e:
            await ctx.send(f"❌ Fehler: {e}")

    @commands.command(name="retention_test")
    @commands.has_permissions(administrator=True)
    async def retention_test(self, ctx: commands.Context, user: discord.Member):
        """Testet die Retention-Nachricht für einen spezifischen User."""
        # Preview der Nachricht
        display_name = user.display_name
        embed = discord.Embed(
            title=f"Hey {display_name}, wir vermissen dich! :(",
            description=(
                f"Dir ist bestimmt aufgefallen, dass du schon **14 Tage** "
                f"nicht mehr aktiv in der **{ctx.guild.name}** warst.\n\n"
                f"Wir würden uns freuen, dich mal wieder im Voice oder Chat zu sehen.\n\n"
                f"Falls dich etwas stört oder du Feedback hast, lass es uns bitte wissen "
                f"- wir wollen den Server für dich besser machen."
            ),
            color=discord.Color.blue(),
        )

        if ctx.guild.icon:
            embed.set_thumbnail(url=ctx.guild.icon.url)

        # View mit Buttons (ohne echten Invite für Preview)
        view = MissYouView(
            ctx.guild.id,
            ctx.guild.name,
            None,
            self.config.server_link,
            self.config.voice_link,
        )

        await ctx.send(f"**Preview für {display_name}:**", embed=embed, view=view)

    @commands.command(name="retention_test_dm")
    @commands.has_permissions(administrator=True)
    async def retention_test_dm(
        self, ctx: commands.Context, user_ref: str, days_inactive: int = 14
    ):
        """Sendet die Retention-DM testweise an eine User-ID oder @Mention (ohne DB-Update)."""
        days_inactive = max(1, days_inactive)
        target_guild = ctx.guild
        guild_id = target_guild.id if target_guild else 0
        guild_name = target_guild.name if target_guild else "unserem Server"

        try:
            match = re.search(r"\d+", str(user_ref))
            if not match:
                await ctx.send("⚠️ Bitte gib eine gültige User-ID oder @Mention an.")
                return
            user_id = int(match.group(0))

            user = await self.bot.fetch_user(user_id)

            invite_url = None
            if target_guild:
                try:
                    invites = await target_guild.invites()
                    for inv in invites:
                        if inv.max_age == 0:  # Permanenter Invite
                            invite_url = str(inv)
                            break
                except discord.Forbidden:
                    pass  # Keine Berechtigung für Invites

            display_name = user.display_name
            if target_guild:
                member = target_guild.get_member(user_id)
                if member:
                    display_name = member.display_name

            embed = discord.Embed(
                title=f"Hey {display_name}, wir vermissen dich! :(",
                description=(
                    f"Dir ist bestimmt aufgefallen, dass du schon **{days_inactive} Tage** "
                    f"nicht mehr aktiv in der **{guild_name}** warst.\n\n"
                    f"Wir würden uns freuen, dich mal wieder im Voice oder Chat zu sehen.\n\n"
                    f"Falls dich etwas stört oder du Feedback hast, lass es uns bitte wissen "
                    f"- wir wollen den Server für dich besser machen."
                ),
                color=discord.Color.blue(),
            )

            if target_guild and target_guild.icon:
                embed.set_thumbnail(url=target_guild.icon.url)

            view = MissYouView(
                guild_id,
                guild_name,
                invite_url,
                self.config.server_link,
                self.config.voice_link,
            )
            await user.send(embed=embed, view=view)

            await ctx.send(
                f"✅ Test-DM an User {display_name} (ID {user_id}) gesendet ({days_inactive} Tage)."
            )

        except discord.Forbidden:
            await ctx.send(f"⚠️ Konnte keine DM an {user_id} senden (DMs deaktiviert?).")
        except Exception as e:
            await ctx.send(f"❌ Fehler beim Senden an {user_id}: {e}")

    @commands.command(name="retention_feedback")
    @commands.has_permissions(administrator=True)
    async def retention_feedback_list(self, ctx: commands.Context, limit: int = 10):
        """Zeigt die letzten Feedbacks von Usern."""
        try:
            rows = central_db.query_all(
                """
                SELECT user_id, sent_at, error_message
                FROM user_retention_messages
                WHERE message_type = 'feedback'
                ORDER BY sent_at DESC
                LIMIT ?
                """,
                (limit,),
            )

            if not rows:
                await ctx.send("Noch kein Feedback erhalten.")
                return

            embed = discord.Embed(title="💬 User Feedback", color=discord.Color.green())

            for row in rows:
                user_id = row[0]
                sent_at = datetime.utcfromtimestamp(row[1]).strftime("%d.%m.%Y %H:%M")
                feedback_text = row[2] or "Kein Text"

                # Kürze Feedback wenn zu lang
                if len(feedback_text) > 200:
                    feedback_text = feedback_text[:200] + "..."

                embed.add_field(
                    name=f"User {user_id} - {sent_at}",
                    value=feedback_text,
                    inline=False,
                )

            await ctx.send(embed=embed)

        except Exception as e:
            await ctx.send(f"Fehler: {e}")


async def setup(bot: commands.Bot):
    await bot.add_cog(UserRetentionCog(bot))
