"""
Coaching Panel - Ein großes Formular fuer Coaching-Anfragen.
"""

import asyncio
import logging
import time
from datetime import datetime

import discord
from discord import app_commands
from discord.ext import commands

from service import db
from service.config import settings

log = logging.getLogger(__name__)

PANEL_KV_NS = "coaching"


async def _get_stored_panel_msg_id() -> int | None:
    row = await db.query_one_async(
        "SELECT v FROM kv_store WHERE ns = ? AND k = ?",
        (PANEL_KV_NS, "panel_message_id"),
    )
    if not row:
        return None
    try:
        value = row[0] if not isinstance(row, dict) else row.get("v")
        return int(value)
    except (TypeError, ValueError):
        return None


async def _store_panel_msg_id(message_id: int) -> None:
    await db.execute_async(
        "INSERT OR REPLACE INTO kv_store (ns, k, v) VALUES (?, ?, ?)",
        (PANEL_KV_NS, "panel_message_id", str(message_id)),
    )


class CoachingRequestModal(discord.ui.Modal, title="Deadlock Coaching"):
    def __init__(self, cog: "CoachingPanelCog"):
        super().__init__(custom_id="coaching_request_modal")
        self.cog = cog

        self.rank_input = discord.ui.TextInput(
            label="Rang + Subrank",
            placeholder="z.B. Archon 3, Ascendant VI, Emissary II",
            max_length=60,
        )
        self.hero_input = discord.ui.TextInput(
            label="Main-Hero",
            placeholder="z.B. Haze, Seven, Vindicta",
            max_length=50,
        )
        self.availability_input = discord.ui.TextInput(
            label="Wann hast du Zeit? (Datum + Uhrzeit)",
            placeholder="z.B. Montag 18:00, Heute Abend ab 20 Uhr...",
            max_length=120,
        )
        self.games_hours_input = discord.ui.TextInput(
            label="Games / Stunden",
            placeholder="z.B. 300 Games / 150 Stunden",
            max_length=120,
        )
        self.problems_input = discord.ui.TextInput(
            label="Probleme / Ziele",
            style=discord.TextStyle.paragraph,
            placeholder="Wobei brauchst du Hilfe? (Keine DMs/FAs an Coaches! Kommunikation nur im Chat!)",
            max_length=1000,
        )

        self.add_item(self.rank_input)
        self.add_item(self.hero_input)
        self.add_item(self.availability_input)
        self.add_item(self.games_hours_input)
        self.add_item(self.problems_input)

    async def on_submit(self, interaction: discord.Interaction) -> None:
        try:
            await self.cog._submit_coaching_request(
                interaction,
                rank_input=self.rank_input.value,
                hero=self.hero_input.value,
                availability=self.availability_input.value,
                games_hours=self.games_hours_input.value,
                problems=self.problems_input.value,
            )
        except Exception:
            log.exception("Coaching modal submit failed for user %s", interaction.user.id)
            if interaction.response.is_done():
                await interaction.followup.send(
                    "❌ Beim Absenden der Anfrage ist ein Fehler aufgetreten. Bitte versuche es erneut.",
                    ephemeral=True,
                )
            else:
                await interaction.response.send_message(
                    "❌ Beim Absenden der Anfrage ist ein Fehler aufgetreten. Bitte versuche es erneut.",
                    ephemeral=True,
                )

    async def on_error(self, interaction: discord.Interaction, error: Exception) -> None:
        log.exception("Coaching modal error for user %s: %s", interaction.user.id, error)
        if interaction.response.is_done():
            await interaction.followup.send(
                "❌ Beim Coaching-Formular ist ein Fehler aufgetreten. Bitte versuche es erneut.",
                ephemeral=True,
            )
        else:
            await interaction.response.send_message(
                "❌ Beim Coaching-Formular ist ein Fehler aufgetreten. Bitte versuche es erneut.",
                ephemeral=True,
            )


class CoachingStartView(discord.ui.View):
    def __init__(self, cog: "CoachingPanelCog"):
        super().__init__(timeout=None)
        self.cog = cog

    @discord.ui.button(
        label="🎮 Coaching anfragen",
        style=discord.ButtonStyle.primary,
        custom_id="coaching_panel_start",
    )
    async def start(self, interaction: discord.Interaction, button: discord.ui.Button) -> None:
        await self.cog._start_coaching_flow(interaction)


class CoachingPanelCog(commands.Cog):
    """Coaching Panel - grosses Formular statt Multi-Step-Flow."""

    def __init__(self, bot: commands.Bot):
        self.bot = bot
        self._db_ready = False
        self._panel_message_id: int | None = None
        self._panel_setup_task: asyncio.Task | None = None

    async def _db_connect(self) -> None:
        if self._db_ready:
            return
        self._db_ready = True

    async def cog_load(self) -> None:
        await self._db_connect()
        self.bot.add_view(CoachingStartView(self))
        self._panel_setup_task = asyncio.create_task(self._delayed_panel_setup())

    async def cog_unload(self) -> None:
        if self._panel_setup_task:
            self._panel_setup_task.cancel()

    async def _delayed_panel_setup(self) -> None:
        await self.bot.wait_until_ready()
        await self._ensure_panel()

    def _build_panel_embed(self) -> discord.Embed:
        embed = discord.Embed(
            title="🎮  Deadlock Coaching",
            description=(
                "Du willst besser werden? Unsere Coaches helfen dir, dein Spiel gezielt zu verbessern.\n\n"
                "**⚠️ Wichtige Regeln:**\n"
                "• Die Kommunikation findet **ausschließlich** im Coaching-Chat statt.\n"
                "• Bitte sende **keine** Freundschaftsanfragen (FAs) oder DMs an die Coaches.\n"
                "• Sei bereit zu antworten, wenn sich ein Coach meldet.\n\n"
                "Klicke auf den Button und fülle das Formular direkt aus."
            ),
            color=discord.Color.blue(),
        )
        embed.add_field(
            name="📋 Ablauf",
            value=(
                "1. Formular ausfüllen\n"
                "2. AI analysiert deine Anfrage\n"
                "3. Du bekommst die Coaching-Rolle\n"
                "4. Ein Coach meldet sich bei dir"
            ),
            inline=False,
        )
        return embed

    async def _get_panel_channel(self) -> discord.TextChannel | None:
        channel = self.bot.get_channel(settings.coaching_panel_channel_id)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(settings.coaching_panel_channel_id)
            except (discord.NotFound, discord.Forbidden):
                return None
        if isinstance(channel, discord.TextChannel):
            return channel
        return None

    async def _ensure_panel(self) -> None:
        channel = await self._get_panel_channel()
        if not channel:
            log.warning("Coaching panel channel %s not found", settings.coaching_panel_channel_id)
            return

        stored_message_id = await _get_stored_panel_msg_id()
        panel_embed = self._build_panel_embed()
        panel_view = CoachingStartView(self)

        if stored_message_id:
            try:
                message = await channel.fetch_message(stored_message_id)
                await message.edit(embed=panel_embed, view=panel_view)
                self._panel_message_id = message.id
                return
            except discord.NotFound:
                log.info(
                    "Stored coaching panel message %s not found, recreating", stored_message_id
                )
            except discord.Forbidden:
                log.warning(
                    "Missing permission to fetch/edit coaching panel message %s", stored_message_id
                )
                return

        message = await channel.send(embed=panel_embed, view=panel_view)
        await _store_panel_msg_id(message.id)
        self._panel_message_id = message.id

    def _discard_incomplete_pending_requests(self, user_id: int) -> None:
        db.execute(
            """DELETE FROM coaching_requests
               WHERE discord_user_id=?
                 AND status='pending'
                 AND (
                     current_problems IS NULL OR TRIM(current_problems) = ''
                 )""",
            (user_id,),
        )

    async def _start_coaching_flow(self, interaction: discord.Interaction) -> None:
        await self._db_connect()
        
        # Ban check
        now = int(time.time())
        ban = db.query_one("SELECT * FROM coaching_bans WHERE discord_user_id=? AND expires_at > ?", (interaction.user.id, now))
        if ban:
            expiry_dt = datetime.fromtimestamp(ban["expires_at"])
            await interaction.response.send_message(
                f"❌ Du bist aktuell vom Coaching-System gesperrt.\n"
                f"Grund: {ban['reason'] or 'Unbekannt'}\n"
                f"Sperre endet am: {expiry_dt.strftime('%d.%m.%Y %H:%M')}",
                ephemeral=True
            )
            return

        self._discard_incomplete_pending_requests(interaction.user.id)
        await interaction.response.send_modal(CoachingRequestModal(self))

    async def _submit_coaching_request(
        self,
        interaction: discord.Interaction,
        *,
        rank_input: str,
        hero: str,
        availability: str,
        games_hours: str,
        problems: str,
    ) -> None:
        await self._db_connect()
        log.info("Submitting coaching request for user %s", interaction.user.id)

        now = int(time.time())
        rank_raw = " ".join(rank_input.split()) or "Nicht angegeben"
        hero_raw = hero.strip() or "Nicht angegeben"
        availability_raw = availability.strip() or "Nicht angegeben"
        games_hours_raw = " ".join(games_hours.split()) or "Nicht angegeben"
        problems_raw = problems.strip()

        db.execute(
            """INSERT INTO coaching_requests (
                   discord_user_id, discord_username, rank, subrank, hero, availability,
                   scheduled_slot, games_played, hours_played, current_problems, status, created_at, updated_at
               )
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)""",
            (
                interaction.user.id,
                interaction.user.display_name,
                rank_raw,
                "",
                hero_raw,
                availability_raw,
                availability_raw, # Use availability also for scheduled_slot field
                games_hours_raw,
                "",
                problems_raw,
                now,
                now,
            ),
        )
        log.info("Stored coaching request for user %s", interaction.user.id)

        row = db.query_one(
            """SELECT * FROM coaching_requests
               WHERE discord_user_id=? AND status='pending'
               ORDER BY created_at DESC, id DESC LIMIT 1""",
            (interaction.user.id,),
        )

        await interaction.response.send_message(
            "✅ Deine Coaching-Anfrage wurde gespeichert. Die AI analysiert sie jetzt und postet sie automatisch im Coaching-Channel.",
            ephemeral=True,
        )

        cog = interaction.client.get_cog("CoachingRequestCog")
        if cog and row:
            asyncio.create_task(cog._trigger_analysis_for_user(interaction.user.id))
        else:
            log.error(
                "CoachingRequestCog is not loaded; request %s cannot be analyzed automatically",
                row["id"] if row else "unknown",
            )

    @app_commands.command(name="coaching-anfrage", description="Stelle eine Coaching-Anfrage")
    async def coaching_anfrage(self, interaction: discord.Interaction) -> None:
        await self._start_coaching_flow(interaction)

    @app_commands.command(name="coaching-status", description="Pruefe den Status deiner Anfrage")
    async def coaching_status(self, interaction: discord.Interaction) -> None:
        await self._db_connect()

        row = db.query_one(
            "SELECT * FROM coaching_requests WHERE discord_user_id=? ORDER BY created_at DESC LIMIT 1",
            (interaction.user.id,),
        )
        if not row:
            await interaction.response.send_message(
                "Du hast keine Coaching-Anfrage gestellt.",
                ephemeral=True,
            )
            return

        status = row["status"]
        if status == "pending":
            msg = "⏳ Deine Anfrage wird gerade analysiert. Bitte warte."
        elif status == "analyzed":
            msg = "✅ Deine Anfrage wurde analysiert und wartet auf einen Coach."
        elif status == "matched":
            msg = "🎉 Ein Coach hat sich fuer dich gemeldet. Check deine DMs."
        elif status == "active":
            msg = "🎮 Deine Coaching-Session laeuft gerade."
        elif status == "completed":
            msg = "✅ Deine letzte Session ist abgeschlossen."
        elif status == "cancelled":
            msg = "❌ Deine Anfrage wurde abgebrochen."
        else:
            msg = f"Status: {status}"

        await interaction.response.send_message(msg, ephemeral=True)


async def setup(bot: commands.Bot):
    await bot.add_cog(CoachingPanelCog(bot))
