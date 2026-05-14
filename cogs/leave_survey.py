"""
Leave Survey Cog

Sendet Usern nach dem Verlassen des Servers automatisch eine DM mit einer
kurzen Exit-Umfrage. Die Fragen werden nach User-Typ (Bucket A/B/C)
angepasst und koennen per Dropdown + Modal beantwortet werden.
"""

import logging
import re
import secrets
import time
from dataclasses import dataclass
from typing import Any

import discord
from discord import app_commands
from discord.ext import commands

from cogs import privacy_core as privacy
from service import db as central_db

logger = logging.getLogger(__name__)


def _safe_log_value(value: Any) -> str:
    """Sanitize values before logging to prevent log injection attacks."""
    text = "" if value is None else str(value)
    return text.replace("\r", "\\r").replace("\n", "\\n")


@dataclass
class LeaveSurveyConfig:
    logs_channel_id: int = 1374364800817303632
    survey_base_url: str = "https://deutsche-deadlock-community.de/survey"
    min_days_between_surveys: int = 30
    bucket_a_max_days: int = 2
    bucket_a_max_messages: int = 3
    bucket_b_min_days: int = 14
    bucket_b_min_weekly_sessions: float = 0.5
    bucket_b_min_messages: int = 30
    bucket_b_min_voice_seconds: int = 3600


REASON_OPTIONS: dict[str, list[tuple[str, str]]] = {
    "A": [
        ("Verifizierung/Onboarding hat nicht geklappt", "onboarding_failed"),
        ("Server war unuebersichtlich", "confusing"),
        ("Hab nicht gefunden wonach ich gesucht hab", "not_found"),
        ("War aus Versehen / falscher Server", "wrong_server"),
        ("Technisches Problem (Bot, Links, Channels)", "technical"),
        ("Anderer Grund", "other"),
    ],
    "B": [
        ("Stimmung/Community hat sich veraendert", "mood_changed"),
        ("Konflikt oder Aerger mit jemandem", "conflict"),
        ("Zu wenig los / keine Mitspieler mehr", "inactive_server"),
        ("Spiele Deadlock kaum/nicht mehr", "stopped_playing"),
        ("Moderation / Regeln", "moderation"),
        ("Persoenliche Gruende / keine Zeit", "personal"),
        ("Anderer Grund", "other"),
    ],
    "C": [
        ("War nie richtig warm geworden", "never_warmed_up"),
        ("Zu wenig Aktivitaet / Mitspieler", "low_activity"),
        ("Spiele Deadlock nicht mehr", "stopped_playing"),
        ("Keine Zeit / Discord aufgeraeumt", "no_time"),
        ("Hat mir nicht gefallen", "disliked"),
        ("Anderer Grund", "other"),
    ],
}

FOLLOW_UP_QUESTIONS: dict[str, str] = {
    "onboarding_failed": "An welchem Schritt hing es genau?",
    "confusing": "Was hast du gesucht und nicht gefunden?",
    "technical": "Welcher Bot/Link/Channel und was ist passiert?",
    "conflict": "Magst du sagen was vorgefallen ist? Bleibt vertraulich.",
    "mood_changed": "Was hat sich veraendert und seit wann fuehlte es sich anders an?",
    "moderation": "Welche Entscheidung oder Regel war das Problem?",
    "stopped_playing": "Was muesste passieren damit du wieder Deadlock spielst?",
    "inactive_server": "Was haette mehr los gemacht fuer dich?",
    "low_activity": "Was haette mehr los gemacht fuer dich?",
    "personal": "Alles gut - magst du trotzdem kurz sagen ob etwas am Server lag?",
    "no_time": "Alles gut - magst du trotzdem kurz sagen ob etwas am Server lag?",
    "never_warmed_up": "Was haette dir geholfen anzukommen?",
    "not_found": "Wonach hast du gesucht?",
    "wrong_server": "Kein Problem - alles gut.",
    "disliked": "Was genau hat dir nicht gefallen?",
    "other": "Erzaehl gern in eigenen Worten.",
}

BUCKET_DESCRIPTIONS: dict[str, str] = {
    "A": "Hey, du warst nur kurz bei uns - schade! Woran lag es?",
    "B": "Hey {name}, du warst eine Weile aktiv dabei - schade dass du gegangen bist. Ehrliches Feedback hilft uns wirklich.",
    "C": "Schade dass du gegangen bist. Magst du kurz sagen warum?",
}


def _truncate_label(text: str, limit: int = 45) -> str:
    if len(text) <= limit:
        return text
    return text[: limit - 3].rstrip() + "..."


class LeaveSurveyModal(discord.ui.Modal):
    def __init__(
        self,
        cog: "LeaveSurveyCog",
        survey_id: int,
        guild_id: int,
        bucket: str,
        reason_code: str,
        follow_up_question: str,
    ):
        super().__init__(title="Kurzes Feedback")
        self.cog = cog
        self.survey_id = survey_id
        self.guild_id = guild_id
        self.bucket = bucket
        self.reason_code = reason_code
        self.follow_up_question = follow_up_question

        self.follow_up_input = discord.ui.TextInput(
            label=_truncate_label(follow_up_question),
            style=discord.TextStyle.paragraph,
            required=False,
            max_length=1000,
        )
        self.extra_input = discord.ui.TextInput(
            label="Moechtest du noch etwas loswerden?",
            style=discord.TextStyle.paragraph,
            required=False,
            max_length=1000,
        )
        self.add_item(self.follow_up_input)
        self.add_item(self.extra_input)

    async def on_submit(self, interaction: discord.Interaction):
        now = int(time.time())
        follow_up_text = self.follow_up_input.value.strip() or None
        extra_text = self.extra_input.value.strip() or None

        try:
            central_db.execute(
                """
                UPDATE member_leave_surveys
                SET follow_up_question = ?,
                    follow_up_text = ?,
                    extra_text = ?,
                    responded_at = ?
                WHERE id = ? AND user_id = ?
                """,
                (
                    self.follow_up_question,
                    follow_up_text,
                    extra_text,
                    now,
                    self.survey_id,
                    interaction.user.id,
                ),
            )

            await interaction.response.send_message(
                "Danke fuer dein ehrliches Feedback.",
                ephemeral=True,
            )

            await self.cog._log_follow_up_response(
                interaction.user,
                self.guild_id,
                self.bucket,
                self.reason_code,
                self.follow_up_question,
                follow_up_text,
                extra_text,
            )
        except Exception as exc:
            logger.error(
                "Fehler beim Speichern des Leave-Survey-Feedbacks fuer %s: %s",
                interaction.user.id,
                exc,
                exc_info=True,
            )
            if not interaction.response.is_done():
                await interaction.response.send_message(
                    "Feedback konnte gerade nicht gespeichert werden.",
                    ephemeral=True,
                )

    async def on_error(self, interaction: discord.Interaction, error: Exception) -> None:
        logger.error(
            "Modal-Fehler im Leave Survey fuer %s: %s",
            interaction.user.id if interaction and interaction.user else "unknown",
            error,
            exc_info=True,
        )
        if interaction and not interaction.response.is_done():
            await interaction.response.send_message(
                "Beim Verarbeiten ist ein Fehler aufgetreten.",
                ephemeral=True,
            )


class LeaveSurveyReasonSelect(discord.ui.Select):
    def __init__(self, bucket: str):
        self.bucket = bucket
        options = [
            discord.SelectOption(label=label, value=value) for label, value in REASON_OPTIONS[bucket]
        ]
        super().__init__(
            placeholder="Warum bist du gegangen?",
            min_values=1,
            max_values=1,
            options=options,
            custom_id=f"leave_survey:reason:{bucket}",
        )

    async def callback(self, interaction: discord.Interaction):
        cog = interaction.client.get_cog("LeaveSurveyCog")
        if not isinstance(cog, LeaveSurveyCog):
            await interaction.response.send_message(
                "Leave-Survey ist gerade nicht verfuegbar.",
                ephemeral=True,
            )
            return
        await cog.handle_reason_selection(interaction, self.bucket, self.values[0])


class LeaveSurveyViewA(discord.ui.View):
    def __init__(self):
        super().__init__(timeout=None)
        self.add_item(LeaveSurveyReasonSelect("A"))


class LeaveSurveyViewB(discord.ui.View):
    def __init__(self):
        super().__init__(timeout=None)
        self.add_item(LeaveSurveyReasonSelect("B"))


class LeaveSurveyViewC(discord.ui.View):
    def __init__(self):
        super().__init__(timeout=None)
        self.add_item(LeaveSurveyReasonSelect("C"))


class LeaveSurveyCog(commands.Cog):
    def __init__(self, bot: commands.Bot):
        self.bot = bot
        self.config = LeaveSurveyConfig()
        logger.info("LeaveSurvey Cog initialisiert")

    async def cog_load(self):
        try:
            central_db.query_one("SELECT 1")
        except Exception as exc:
            logger.error(f"DB nicht verfuegbar: {exc}")
            raise

        self.bot.add_view(LeaveSurveyViewA())
        self.bot.add_view(LeaveSurveyViewB())
        self.bot.add_view(LeaveSurveyViewC())
        logger.info("LeaveSurvey Views registriert")

    async def cog_unload(self):
        logger.info("LeaveSurvey Cog entladen")

    @commands.Cog.listener()
    async def on_member_remove(self, member: discord.Member):
        try:
            if member.bot:
                return
            if privacy.is_opted_out(member.id):
                return

            recent_ban = central_db.query_one(
                """
                SELECT 1 FROM member_events
                WHERE user_id = ? AND guild_id = ? AND event_type = 'ban'
                  AND timestamp >= datetime('now', '-15 seconds')
                LIMIT 1
                """,
                (member.id, member.guild.id),
            )
            if recent_ban:
                return

            min_created_at = int(time.time()) - (self.config.min_days_between_surveys * 24 * 60 * 60)
            recent_survey = central_db.query_one(
                """
                SELECT 1 FROM member_leave_surveys
                WHERE user_id = ? AND strftime('%s', created_at) >= ?
                LIMIT 1
                """,
                (member.id, min_created_at),
            )
            if recent_survey:
                return

            bucket = await self._classify(member)
            days_on_server = self._get_days_on_server(member)
            survey_token = secrets.token_urlsafe(16)
            left_at = int(time.time())

            central_db.execute(
                """
                INSERT INTO member_leave_surveys(
                    user_id,
                    guild_id,
                    left_at,
                    display_name,
                    user_bucket,
                    days_on_server,
                    survey_token,
                    dm_status
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, NULL)
                """,
                (
                    member.id,
                    member.guild.id,
                    left_at,
                    member.display_name,
                    bucket,
                    days_on_server,
                    survey_token,
                ),
            )
            survey_row = central_db.query_one(
                "SELECT id FROM member_leave_surveys WHERE survey_token = ?",
                (survey_token,),
            )
            if not survey_row:
                raise RuntimeError("Leave-Survey konnte nach Insert nicht geladen werden.")
            survey_id = int(survey_row[0])

            dm_status = "failed"
            try:
                embed = self._build_dm_embed(member, member.guild.name, bucket, survey_token)
                view = self._build_view_for_bucket(bucket)
                await member.send(embed=embed, view=view)
                dm_status = "sent"
            except discord.Forbidden:
                dm_status = "blocked"
            except Exception as exc:
                dm_status = "failed"
                logger.error(
                    "Fehler beim Senden der Leave-Survey-DM an %s: %s",
                    member.id,
                    exc,
                    exc_info=True,
                )

            central_db.execute(
                "UPDATE member_leave_surveys SET dm_status = ? WHERE id = ?",
                (dm_status, survey_id),
            )

            await self._log_leave_trigger(member, bucket, days_on_server, dm_status)
        except Exception as exc:
            logger.error(
                "Fehler im Leave-Survey-Listener fuer %s: %s",
                getattr(member, "id", "unknown"),
                exc,
                exc_info=True,
            )

    async def _classify(self, member: discord.Member) -> str:
        days_on_server = self._get_days_on_server(member)

        voice_sessions_row = central_db.query_one(
            "SELECT COUNT(*) FROM voice_session_log WHERE user_id = ?",
            (member.id,),
        )
        message_count_row = central_db.query_one(
            """
            SELECT message_count
            FROM message_activity
            WHERE user_id = ? AND guild_id = ?
            """,
            (member.id, member.guild.id),
        )
        retention_row = central_db.query_one(
            "SELECT avg_weekly_sessions FROM user_retention_tracking WHERE user_id = ?",
            (member.id,),
        )
        voice_stats_row = central_db.query_one(
            "SELECT total_seconds FROM voice_stats WHERE user_id = ?",
            (member.id,),
        )

        voice_sessions = int(voice_sessions_row[0] or 0) if voice_sessions_row else 0
        message_count = int(message_count_row[0] or 0) if message_count_row else 0
        avg_weekly_sessions = (
            float(retention_row[0] or 0) if retention_row and retention_row[0] is not None else 0.0
        )
        voice_total_seconds = int(voice_stats_row[0] or 0) if voice_stats_row else 0

        if (
            days_on_server < self.config.bucket_a_max_days
            and voice_sessions == 0
            and message_count < self.config.bucket_a_max_messages
        ):
            return "A"

        if (
            days_on_server >= self.config.bucket_b_min_days
            and (
                avg_weekly_sessions >= self.config.bucket_b_min_weekly_sessions
                or message_count >= self.config.bucket_b_min_messages
                or voice_total_seconds > self.config.bucket_b_min_voice_seconds
            )
        ):
            return "B"

        return "C"

    def _get_days_on_server(self, member: discord.Member) -> int:
        now = int(time.time())
        join_row = central_db.query_one(
            """
            SELECT MIN(strftime('%s', timestamp))
            FROM member_events
            WHERE user_id = ? AND guild_id = ? AND event_type = 'join'
            """,
            (member.id, member.guild.id),
        )

        join_ts = None
        if join_row and join_row[0]:
            join_ts = int(join_row[0])
        elif member.joined_at:
            join_ts = int(member.joined_at.timestamp())

        if not join_ts:
            return 0
        return max(0, (now - join_ts) // 86400)

    def _build_dm_embed(
        self,
        user: discord.abc.User,
        guild_name: str,
        bucket: str,
        survey_token: str,
    ) -> discord.Embed:
        description = BUCKET_DESCRIPTIONS[bucket]
        if "{name}" in description:
            description = description.format(name=user.display_name)

        survey_url = self._build_survey_url(survey_token)
        embed = discord.Embed(
            title=f"Feedback zu {guild_name}",
            description=(
                f"{description}\n\n"
                f"Bitte waehle unten den passendsten Grund aus.\n\n"
                f"Wenn du ausfuehrlicher Feedback geben magst (auch mit Bildern): {survey_url}"
            ),
            color=discord.Color.blurple(),
        )
        return embed

    def _build_view_for_bucket(self, bucket: str) -> discord.ui.View:
        if bucket == "A":
            return LeaveSurveyViewA()
        if bucket == "B":
            return LeaveSurveyViewB()
        return LeaveSurveyViewC()

    def _build_survey_url(self, survey_token: str) -> str:
        return f"{self.config.survey_base_url}?t={survey_token}"

    async def handle_reason_selection(
        self, interaction: discord.Interaction, bucket: str, reason_code: str
    ):
        try:
            survey_row = central_db.query_one(
                """
                SELECT id, guild_id, user_bucket
                FROM member_leave_surveys
                WHERE user_id = ? AND responded_at IS NULL
                ORDER BY id DESC
                LIMIT 1
                """,
                (interaction.user.id,),
            )
            if not survey_row:
                await interaction.response.send_message(
                    "Zu dieser Auswahl wurde kein offener Survey gefunden.",
                    ephemeral=True,
                )
                return

            survey_id = int(survey_row[0])
            guild_id = int(survey_row[1])
            survey_bucket = str(survey_row[2] or bucket)
            follow_up_question = FOLLOW_UP_QUESTIONS.get(reason_code, FOLLOW_UP_QUESTIONS["other"])

            central_db.execute(
                """
                UPDATE member_leave_surveys
                SET reason_code = ?
                WHERE id = ? AND user_id = ?
                """,
                (reason_code, survey_id, interaction.user.id),
            )

            await interaction.response.send_modal(
                LeaveSurveyModal(
                    self,
                    survey_id,
                    guild_id,
                    survey_bucket,
                    reason_code,
                    follow_up_question,
                )
            )
        except Exception as exc:
            logger.error(
                "Fehler bei Leave-Survey-Auswahl fuer %s: %s",
                interaction.user.id,
                exc,
                exc_info=True,
            )
            if not interaction.response.is_done():
                await interaction.response.send_message(
                    "Die Auswahl konnte gerade nicht verarbeitet werden.",
                    ephemeral=True,
                )

    async def _log_leave_trigger(
        self,
        member: discord.Member,
        bucket: str,
        days_on_server: int,
        dm_status: str,
    ) -> None:
        channel = await self._get_logs_channel()
        if channel is None:
            return

        embed = discord.Embed(
            title="Leave-Survey ausgeloest",
            color=discord.Color.orange(),
        )
        embed.add_field(
            name="User",
            value=f"<@{member.id}> ({member.id})",
            inline=False,
        )
        embed.add_field(name="Name", value=member.display_name or "Unbekannt", inline=True)
        embed.add_field(name="Bucket", value=bucket, inline=True)
        embed.add_field(name="Tage auf dem Server", value=str(days_on_server), inline=True)
        embed.add_field(name="DM-Status", value=dm_status, inline=True)

        try:
            await channel.send(embed=embed)
        except Exception as exc:
            logger.error(
                "Fehler beim Leave-Survey-Log fuer %s: %s",
                member.id,
                exc,
                exc_info=True,
            )

    async def _log_follow_up_response(
        self,
        user: discord.abc.User,
        guild_id: int,
        bucket: str,
        reason_code: str,
        follow_up_question: str,
        follow_up_text: str | None,
        extra_text: str | None,
    ) -> None:
        channel = await self._get_logs_channel()
        if channel is None:
            return

        embed = discord.Embed(
            title="Leave-Survey Antwort",
            color=discord.Color.green(),
        )
        embed.add_field(name="User", value=f"<@{user.id}> ({user.id})", inline=False)
        embed.add_field(name="Guild", value=str(guild_id), inline=True)
        embed.add_field(name="Bucket", value=bucket, inline=True)
        embed.add_field(name="Grund", value=reason_code, inline=True)
        embed.add_field(name="Folgefrage", value=follow_up_question, inline=False)
        embed.add_field(name="Antwort", value=follow_up_text or "-", inline=False)
        embed.add_field(name="Extra", value=extra_text or "-", inline=False)

        try:
            await channel.send(embed=embed)
        except Exception as exc:
            logger.error(
                "Fehler beim Follow-up-Log fuer %s: %s",
                user.id,
                exc,
                exc_info=True,
            )

    async def _get_logs_channel(self) -> discord.abc.Messageable | None:
        channel = self.bot.get_channel(self.config.logs_channel_id)
        if channel is not None:
            return channel
        try:
            fetched = await self.bot.fetch_channel(self.config.logs_channel_id)
            if isinstance(fetched, discord.abc.Messageable):
                return fetched
        except Exception as exc:
            logger.debug("Logs-Channel nicht verfuegbar: %s", exc)
        return None

    def _normalize_bucket(self, bucket: str) -> str:
        normalized = str(bucket or "").strip().upper()
        if normalized not in {"A", "B", "C"}:
            raise ValueError("Bucket muss A, B oder C sein.")
        return normalized

    async def _resolve_user(self, user_ref: str) -> discord.User:
        match = re.search(r"\d+", str(user_ref))
        if not match:
            raise ValueError("Bitte gib eine gueltige User-ID oder @Mention an.")
        return await self.bot.fetch_user(int(match.group(0)))

    @commands.command(name="leavesurvey_status")
    @commands.has_permissions(administrator=True)
    async def leavesurvey_status(self, ctx: commands.Context):
        try:
            total = central_db.query_one("SELECT COUNT(*) FROM member_leave_surveys")[0] or 0
            sent_count = (
                central_db.query_one(
                    "SELECT COUNT(*) FROM member_leave_surveys WHERE dm_status = 'sent'"
                )[0]
                or 0
            )
            responded = (
                central_db.query_one(
                    "SELECT COUNT(*) FROM member_leave_surveys WHERE responded_at IS NOT NULL"
                )[0]
                or 0
            )
            web_submitted = (
                central_db.query_one(
                    "SELECT COUNT(*) FROM member_leave_surveys WHERE web_submitted_at IS NOT NULL"
                )[0]
                or 0
            )
            bucket_rows = central_db.query_all(
                """
                SELECT user_bucket, COUNT(*)
                FROM member_leave_surveys
                GROUP BY user_bucket
                ORDER BY user_bucket
                """
            )
            status_rows = central_db.query_all(
                """
                SELECT COALESCE(dm_status, 'pending'), COUNT(*)
                FROM member_leave_surveys
                GROUP BY COALESCE(dm_status, 'pending')
                ORDER BY COALESCE(dm_status, 'pending')
                """
            )

            bucket_text = "\n".join(f"{row[0]}: {row[1]}" for row in bucket_rows) or "Keine"
            status_text = "\n".join(f"{row[0]}: {row[1]}" for row in status_rows) or "Keine"
            dm_rate = (responded / sent_count * 100) if sent_count else 0.0
            web_rate = (web_submitted / sent_count * 100) if sent_count else 0.0

            embed = discord.Embed(title="Leave Survey Status", color=discord.Color.blue())
            embed.add_field(name="Surveys gesamt", value=str(total), inline=True)
            embed.add_field(name="DMs gesendet", value=str(sent_count), inline=True)
            embed.add_field(name="DM Response-Rate", value=f"{dm_rate:.1f}%", inline=True)
            embed.add_field(name="Web Response-Rate", value=f"{web_rate:.1f}%", inline=True)
            embed.add_field(name="Je Bucket", value=bucket_text, inline=False)
            embed.add_field(name="Je DM-Status", value=status_text, inline=False)

            await ctx.send(embed=embed)
        except Exception as exc:
            await ctx.send(f"Fehler: {exc}")

    @commands.command(name="leavesurvey_test")
    @commands.has_permissions(administrator=True)
    async def leavesurvey_test(self, ctx: commands.Context, user_ref: str, bucket: str):
        try:
            normalized_bucket = self._normalize_bucket(bucket)
            user = await self._resolve_user(user_ref)
            guild_name = ctx.guild.name if ctx.guild else "dem Server"
            embed = self._build_dm_embed(user, guild_name, normalized_bucket, "preview-token")
            view = self._build_view_for_bucket(normalized_bucket)
            await user.send(embed=embed, view=view)
            await ctx.send(
                f"Test-DM fuer Bucket {normalized_bucket} an {user.display_name} ({user.id}) gesendet."
            )
        except discord.Forbidden:
            await ctx.send("Konnte keine DM senden.")
        except ValueError as exc:
            await ctx.send(str(exc))
        except Exception as exc:
            await ctx.send(f"Fehler beim Testversand: {exc}")

    @commands.command(name="leavesurvey_recent")
    @commands.has_permissions(administrator=True)
    async def leavesurvey_recent(self, ctx: commands.Context, limit: int = 10):
        try:
            limit = max(1, min(limit, 25))
            rows = central_db.query_all(
                """
                SELECT user_id, display_name, user_bucket, reason_code, follow_up_text, responded_at
                FROM member_leave_surveys
                WHERE responded_at IS NOT NULL
                ORDER BY responded_at DESC
                LIMIT ?
                """,
                (limit,),
            )

            if not rows:
                await ctx.send("Noch keine beantworteten Leave-Surveys.")
                return

            embed = discord.Embed(title="Letzte Leave-Survey-Antworten", color=discord.Color.green())

            for row in rows:
                user_id = row[0]
                display_name = row[1] or "Unbekannt"
                bucket = row[2] or "?"
                reason_code = row[3] or "-"
                follow_up_text = (row[4] or "-").strip() or "-"
                if len(follow_up_text) > 250:
                    follow_up_text = follow_up_text[:247].rstrip() + "..."
                embed.add_field(
                    name=f"{display_name} ({user_id}) [{bucket}]",
                    value=f"Grund: `{reason_code}`\nText: {follow_up_text}",
                    inline=False,
                )

            await ctx.send(embed=embed)
        except Exception as exc:
            await ctx.send(f"Fehler: {exc}")


async def setup(bot: commands.Bot):
    await bot.add_cog(LeaveSurveyCog(bot))
