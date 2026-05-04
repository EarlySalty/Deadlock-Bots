"""
Coaching Survey - Post-Coaching Feedback via DM
"""

import asyncio
import logging
import time

import discord
from discord import app_commands
from discord.ext import commands

from service import db
from service.config import settings

log = logging.getLogger(__name__)


class CoachingSurveyCog(commands.Cog):
    """Coaching Survey - Post-Coaching DM Feedback"""

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        self._survey_dispatching: set[str] = set()
        self._survey_check_task: asyncio.Task | None = None

    async def cog_load(self):
        if self._survey_check_task is None or self._survey_check_task.done():
            self._survey_check_task = asyncio.create_task(self._run_survey_checks())

    async def cog_unload(self):
        if self._survey_check_task:
            self._survey_check_task.cancel()
            self._survey_check_task = None

    def _get_primary_guild(self) -> discord.Guild | None:
        return self.bot.guilds[0] if self.bot.guilds else None

    def _get_coaching_voice_channel(
        self, member: discord.Member | None
    ) -> discord.VoiceChannel | None:
        if not member or not member.voice or not member.voice.channel:
            return None
        channel = member.voice.channel
        if not isinstance(channel, discord.VoiceChannel):
            return None
        if channel.category_id != settings.coaching_voice_category_id:
            return None
        return channel

    def _get_shared_coaching_voice_channel(
        self,
        user_member: discord.Member | None,
        coach_member: discord.Member | None,
    ) -> discord.VoiceChannel | None:
        user_channel = self._get_coaching_voice_channel(user_member)
        coach_channel = self._get_coaching_voice_channel(coach_member)
        if user_channel and coach_channel and user_channel.id == coach_channel.id:
            return user_channel
        return None

    async def _run_survey_checks(self):
        await self.bot.wait_until_ready()
        while True:
            try:
                await self._scan_active_sessions()
            except Exception:
                log.exception("Coaching survey check loop failed")
            await asyncio.sleep(60)

    async def _scan_active_sessions(self):
        guild = self._get_primary_guild()
        if not guild:
            return

        sessions = db.query_all(
            """SELECT * FROM coaching_sessions
               WHERE status='active' AND survey_sent_at IS NULL"""
        )
        for session in sessions:
            await self._process_session_voice_state(guild, session)

    async def _process_session_voice_state(self, guild: discord.Guild, session) -> None:
        session_id = session["id"]
        if session_id in self._survey_dispatching:
            return

        try:
            coach_id = int(session["coach_id"])
        except (TypeError, ValueError):
            return

        user_member = guild.get_member(session["discord_user_id"])
        coach_member = guild.get_member(coach_id)
        shared_channel = self._get_shared_coaching_voice_channel(user_member, coach_member)
        now = int(time.time())

        if shared_channel:
            db.execute(
                """UPDATE coaching_sessions
                   SET voice_channel_id=?, voice_started_at=COALESCE(voice_started_at, ?),
                       voice_last_seen_at=?
                   WHERE id=?""",
                (shared_channel.id, now, now, session["id"]),
            )
            return

        if not session["voice_started_at"]:
            return

        # Voice session ended (was active before, now members are no longer in same channel)
        self._survey_dispatching.add(session_id)
        try:
            coach_name = coach_member.display_name if coach_member else f"Coach {coach_id}"
            
            # 1. Remove active role immediately
            await self._remove_active_role(guild, session["discord_user_id"])
            
            # 2. Assign reward role for 5 days
            reward_expiry = now + (5 * 24 * 60 * 60)
            await self._assign_reward_role(guild, session["discord_user_id"])

            # 3. Send feedback prompt DM
            success = await self.send_survey_dm(session["discord_user_id"], session_id, coach_name)
            if not success:
                # We still mark it as completed even if DM fails, roles were handled
                log.warning("Could not send feedback DM to user %s", session["discord_user_id"])

            db.execute(
                """UPDATE coaching_sessions
                   SET status='completed', completed_at=?, survey_sent_at=?, 
                       reward_role_expires_at=?, voice_last_seen_at=?
                   WHERE id=?""",
                (now, now, reward_expiry, now, session_id),
            )
            
            # Also update the request status and mark role as removed
            db.execute(
                "UPDATE coaching_requests SET status='completed', role_removed_at=?, updated_at=? WHERE id=?",
                (now, now, session["request_id"]),
            )
        finally:
            self._survey_dispatching.discard(session_id)

    async def _assign_reward_role(self, guild: discord.Guild, user_id: int) -> None:
        member = guild.get_member(user_id)
        if not member:
            try:
                member = await guild.fetch_member(user_id)
            except Exception:
                return
        
        role = guild.get_role(settings.coaching_reward_role_id)
        if role and role not in member.roles:
            try:
                await member.add_roles(role, reason="Coaching abgeschlossen - Feedback-Berechtigung")
                log.info("Assigned reward role to user %s", user_id)
            except Exception as e:
                log.error("Could not assign reward role to %s: %s", user_id, e)

    async def _remove_active_role(self, guild: discord.Guild, user_id: int) -> None:
        member = guild.get_member(user_id)
        if not member:
            try:
                member = await guild.fetch_member(user_id)
            except Exception:
                return
        
        coaching_role = guild.get_role(settings.coaching_active_role_id)
        if coaching_role and coaching_role in member.roles:
            try:
                await member.remove_roles(coaching_role, reason="Coaching Session beendet")
                log.info("Removed active coaching role from user %s", user_id)
            except Exception as e:
                log.error("Could not remove active role from %s: %s", user_id, e)

    async def send_survey_dm(self, user_id: int, session_id: str, coach_name: str) -> bool:
        """Send feedback prompt to user after coaching session"""
        try:
            user = self.bot.get_user(user_id)
            if not user:
                user = await self.bot.fetch_user(user_id)
        except Exception as e:
            log.error(f"Could not fetch user {user_id}: {e}")
            return False

        if not user:
            return False

        try:
            feedback_channel_url = f"https://discord.com/channels/{settings.guild_id}/{settings.coaching_feedback_channel_id}"
            
            embed = discord.Embed(
                title="🎮 Coaching abgeschlossen!",
                description=f"Deine Coaching-Session mit **{coach_name}** ist beendet. Wir hoffen, es hat dir geholfen!",
                color=discord.Color.green(),
            )
            embed.add_field(
                name="⭐ Gib uns Feedback",
                value=(
                    f"Du hast nun für **5 Tage** Zugriff auf unseren Feedback-Kanal. "
                    f"Bitte teile deine Erfahrungen dort mit uns:\n\n"
                    f"👉 [**HIER FEEDBACK ABGEBEN**]({feedback_channel_url})\n\n"
                    "Dein Feedback hilft uns die Qualität der Coaches sicherzustellen!"
                ),
                inline=False,
            )

            await user.send(embed=embed)
            return True
        except Exception as e:
            log.error(f"Failed to send feedback DM to {user_id}: {e}")
            return False

    @commands.Cog.listener()
    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ):
        guild = member.guild
        sessions = db.query_all(
            """SELECT * FROM coaching_sessions
               WHERE status='active' AND survey_sent_at IS NULL
               AND (discord_user_id=? OR coach_id=?)""",
            (member.id, member.id),
        )
        for session in sessions:
            await self._process_session_voice_state(guild, session)

    @app_commands.command(name="coaching-survey-senden", description="Survey DM senden (Admin)")
    @app_commands.describe(
        user_id="Discord User ID", session_id="Session ID", coach_name="Coach Name"
    )
    async def send_survey(
        self, interaction: discord.Interaction, user_id: str, session_id: str, coach_name: str
    ):
        """Admin command to send survey DM"""
        if not interaction.guild:
            await interaction.response.send_message("❌ Nur im Server.", ephemeral=True)
            return

        if not interaction.user.id == interaction.guild.owner_id:
            await interaction.response.send_message("❌ Nur Server-Owner.", ephemeral=True)
            return

        success = await self.send_survey_dm(int(user_id), session_id, coach_name)
        if success:
            await interaction.response.send_message("✅ Survey DM gesendet!", ephemeral=True)
        else:
            await interaction.response.send_message("❌ Konnte DM nicht senden.", ephemeral=True)

    @app_commands.command(
        name="coaching-session-beenden", description="Session beenden und Reward-Rolle vergeben (Admin)"
    )
    @app_commands.describe(session_id="Session ID")
    async def end_session(self, interaction: discord.Interaction, session_id: str):
        """Admin command to end session and trigger survey/rewards"""
        if not interaction.guild:
            await interaction.response.send_message("❌ Nur im Server.", ephemeral=True)
            return

        if not interaction.user.id == interaction.guild.owner_id:
            await interaction.response.send_message("❌ Nur Server-Owner.", ephemeral=True)
            return

        session = db.query_one("SELECT * FROM coaching_sessions WHERE id=?", (session_id,))
        if not session:
            await interaction.response.send_message("❌ Session nicht gefunden.", ephemeral=True)
            return

        if session["status"] == "completed":
            await interaction.response.send_message("❌ Session bereits beendet.", ephemeral=True)
            return

        now = int(time.time())
        reward_expiry = now + (5 * 24 * 60 * 60)
        
        # Get coach info
        coach_id = session["coach_id"]
        coach_member = interaction.guild.get_member(int(coach_id)) if coach_id else None
        coach_name = coach_member.display_name if coach_member else interaction.user.display_name

        # 1. Remove active role
        await self._remove_active_role(interaction.guild, session["discord_user_id"])
        
        # 2. Assign reward role
        await self._assign_reward_role(interaction.guild, session["discord_user_id"])

        # 3. Send feedback prompt DM
        success = await self.send_survey_dm(session["discord_user_id"], session_id, coach_name)

        # Update session
        db.execute(
            """UPDATE coaching_sessions 
               SET status='completed', completed_at=?, survey_sent_at=?, reward_role_expires_at=? 
               WHERE id=?""",
            (now, now, reward_expiry, session_id),
        )
        
        # Update request
        db.execute(
            "UPDATE coaching_requests SET status='completed', role_removed_at=?, updated_at=? WHERE id=?",
            (now, now, session["request_id"]),
        )

        if success:
            await interaction.response.send_message(
                "✅ Session beendet, Reward-Rolle vergeben und DM an User gesendet!", ephemeral=True
            )
        else:
            await interaction.response.send_message(
                "⚠️ Session beendet, aber DM konnte nicht gesendet werden. (Rollen wurden trotzdem aktualisiert)", ephemeral=True
            )


async def setup(bot: commands.Bot):
    await bot.add_cog(CoachingSurveyCog(bot))
