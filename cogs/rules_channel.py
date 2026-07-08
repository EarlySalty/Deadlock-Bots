"""
Rules Panel Cog - startet das statische Multi-Step Onboarding im privaten Thread.
- Persistente Panel-View (nur custom_id-Buttons, kein Link-Button)
- Delegiert an StaticOnboarding (cogs/onboarding.py)
"""

from __future__ import annotations

import logging

import discord
from discord import app_commands
from discord.ext import commands

from service.discord_utils import is_transient_discord_http_error, retry_discord_http

# ========== Konfiguration ==========
MAIN_GUILD_ID = 1289721245281292288
RULES_CHANNEL_ID = 1315684135175716975
PANEL_MESSAGE_ID = (
    1413481216509874207  # Bestehende Panel-Message – wird editiert statt neu gepostet
)

log = logging.getLogger("RulesPanel")


# ------------------------------ Helpers ------------------------------ #
async def _send_interaction_notice(interaction: discord.Interaction, content: str) -> None:
    try:
        if not interaction.response.is_done():
            await interaction.response.send_message(content, ephemeral=True)
        else:
            await interaction.followup.send(content, ephemeral=True)
    except Exception as exc:
        log.debug("Konnte Interaktionshinweis nicht senden: %s", exc)


async def _delete_thread_quietly(thread: discord.Thread) -> None:
    try:
        await retry_discord_http(
            lambda: thread.delete(),
            log=log,
            operation_name=f"delete failed onboarding thread {thread.id}",
        )
    except (discord.NotFound, discord.Forbidden, discord.HTTPException):
        log.debug("Konnte fehlgeschlagenen Onboarding-Thread %s nicht aufräumen.", thread.id)


async def _create_onboarding_thread(
    channel: discord.TextChannel,
    *,
    name: str,
    invite_user: discord.abc.Snowflake | None,
    log_context: str,
) -> discord.Thread:
    private_thread: discord.Thread | None = None
    try:
        private_thread = await retry_discord_http(
            lambda: channel.create_thread(
                name=name,
                type=discord.ChannelType.private_thread,
                invitable=True,
                auto_archive_duration=60,
            ),
            log=log,
            operation_name=f"create private onboarding thread ({log_context})",
        )
        if invite_user is not None:
            await retry_discord_http(
                lambda: private_thread.add_user(invite_user),
                log=log,
                operation_name=f"add user to private onboarding thread ({log_context})",
            )
        return private_thread
    except discord.Forbidden as exc:
        log.debug("Privater Thread konnte nicht erstellt/zugewiesen werden: %s", exc)
    except discord.HTTPException as exc:
        if is_transient_discord_http_error(exc):
            log.warning(
                "Temporärer Discord-Fehler beim privaten Onboarding-Thread (%s): %s. Nutze Public-Fallback.",
                log_context,
                exc,
            )
        else:
            log.debug(
                "Privater Onboarding-Thread nicht nutzbar (%s): %s. Nutze Public-Fallback.",
                log_context,
                exc,
            )

    if private_thread is not None:
        await _delete_thread_quietly(private_thread)

    return await retry_discord_http(
        lambda: channel.create_thread(
            name=name,
            type=discord.ChannelType.public_thread,
            auto_archive_duration=60,
        ),
        log=log,
        operation_name=f"create public onboarding thread ({log_context})",
    )


async def _create_user_thread(
    interaction: discord.Interaction,
) -> discord.Thread | None:
    """Erstellt einen (bevorzugt) privaten Thread im Regelkanal und fügt den Nutzer hinzu."""
    guild = interaction.guild
    if not guild:
        await interaction.response.send_message(
            "❌ Dieser Button funktioniert nur in der Guild.", ephemeral=True
        )
        return None

    channel = guild.get_channel(RULES_CHANNEL_ID)
    if not isinstance(channel, discord.TextChannel):
        await interaction.response.send_message(
            "❌ Regelkanal nicht gefunden/kein Textkanal.", ephemeral=True
        )
        return None

    name = f"onboarding-{interaction.user.name}".replace(" ", "-")[:90]

    try:
        return await _create_onboarding_thread(
            channel,
            name=name,
            invite_user=interaction.user,
            log_context=f"user={interaction.user.id}",
        )
    except discord.HTTPException as exc:
        if is_transient_discord_http_error(exc):
            log.warning(
                "Onboarding-Thread konnte für %s temporär nicht erstellt werden: %s",
                interaction.user.id,
                exc,
            )
            await _send_interaction_notice(
                interaction,
                "⚠️ Discord hat gerade Serverprobleme. Bitte versuche es in ein paar Sekunden erneut.",
            )
            return None
        log.error("Thread creation failed: %r", exc)
        await _send_interaction_notice(interaction, "❌ Konnte keinen Thread erstellen.")
        return None
    except Exception as e:
        log.error("Thread creation failed: %r", e)
        await _send_interaction_notice(interaction, "❌ Konnte keinen Thread erstellen.")
        return None


# ------------------------------ Panel-View (persistent) ------------------------------ #
class RulesPanelView(discord.ui.View):
    def __init__(self, cog: RulesPanel):
        super().__init__(timeout=None)  # PERSISTENT
        self.cog = cog

    @discord.ui.button(
        label="Hier starten ➜", style=discord.ButtonStyle.primary, custom_id="rp:panel:start"
    )
    async def start(self, interaction: discord.Interaction, _button: discord.ui.Button):
        await self.cog.start_in_thread(interaction)


# ------------------------------ Cog ------------------------------ #
class RulesPanel(commands.Cog):
    """Wrapper-Cog: Startet das KI-Onboarding im privaten Thread."""

    def __init__(self, bot: commands.Bot):
        self.bot = bot

    async def cog_load(self):
        # Nur die Panel-View persistent registrieren!
        self.bot.add_view(RulesPanelView(self))
        log.info("✅ Rules Panel geladen (Panel-View aktiv)")

    @app_commands.command(name="publish_rules_panel", description="(Admin) Regelwerk-Panel posten")
    @app_commands.checks.has_permissions(administrator=True)
    async def publish_rules_panel(self, interaction: discord.Interaction):
        guild = self.bot.get_guild(MAIN_GUILD_ID)
        if not guild:
            await interaction.response.send_message(
                "❌ MAIN_GUILD_ID ungültig oder Bot nicht auf dieser Guild.",
                ephemeral=True,
            )
            return
        ch = guild.get_channel(RULES_CHANNEL_ID)
        if not isinstance(ch, (discord.TextChannel, discord.Thread)):
            await interaction.response.send_message(
                "❌ RULES_CHANNEL_ID zeigt nicht auf einen Text-/Thread-Kanal.",
                ephemeral=True,
            )
            return

        emb = discord.Embed(
            title="\U0001f4dc Regelwerk \u00b7 Deutsche Deadlock Community",
            description=(
                "### Neu hier? Klick auf **Hier starten \u27a4**\n"
                "und wir erkl\u00e4ren dir alles in 5 Minuten.\n\n\n"
                "**Verhalten**\n"
                "- Respekt gegen\u00fcber allen \u2013 keine Beleidigungen, Diskriminierung oder pers\u00f6nlichen Angriffe\n"
                "- Keine Hassrede, kein NSFW, kein Spam, keine Fremdwerbung\n"
                "- Privatsph\u00e4re respektieren \u2013 keine fremden Daten posten\n"
                "- Sch\u00e4dliche Inhalte (Viren, IP-Grabber etc.) = sofortiger permanenter Bann\n\n"
                "**Im Spielkontext erlaubt**\n"
                "Situatives Trash Talking, Sarkasmus, Wortspiele \u2013 solange es nicht pers\u00f6nlich wird. "
                "Ohne nonverbale Signale kann Ton schnell schiefgehen, also vorher abchecken ob alle damit fein sind.\n\n"
                "**Universalregel:** Sei kein Arschloch \U0001f604\n\n"
                "**Moderation**\n"
                "Probleme? @Moderator oder @Owner pingen. "
                "Konsequenzen je nach Schwere: Verwarnung \u2192 Timeout \u2192 Ban."
            ),
            color=0x00AEEF,
        )
        view = RulesPanelView(self)

        # Bestehende Message editieren statt neu posten
        try:
            msg = await retry_discord_http(
                lambda: ch.fetch_message(PANEL_MESSAGE_ID),
                log=log,
                operation_name=f"fetch rules panel message {PANEL_MESSAGE_ID}",
            )
            await retry_discord_http(
                lambda: msg.edit(embed=emb, view=view),
                log=log,
                operation_name=f"edit rules panel message {PANEL_MESSAGE_ID}",
            )
            await interaction.response.send_message("✅ Panel aktualisiert.", ephemeral=True)
        except discord.NotFound:
            await retry_discord_http(
                lambda: ch.send(embed=emb, view=view),
                log=log,
                operation_name=f"send rules panel message to channel {ch.id}",
            )
            await interaction.response.send_message(
                "✅ Panel neu gesendet (alte Message nicht gefunden).", ephemeral=True
            )
        except discord.HTTPException as exc:
            if is_transient_discord_http_error(exc):
                log.warning("Rules panel publish hit transient Discord error: %s", exc)
                await _send_interaction_notice(
                    interaction,
                    "⚠️ Discord hat gerade Serverprobleme. Bitte versuche den Befehl gleich erneut.",
                )
                return
            log.error("Panel-Edit fehlgeschlagen: %s", exc)
            await _send_interaction_notice(interaction, f"❌ Fehler: {exc}")
        except Exception as e:
            log.error("Panel-Edit fehlgeschlagen: %s", e)
            await _send_interaction_notice(interaction, f"❌ Fehler: {e}")

    # ----- Auto-Start nach Discord Member Screening -----
    @commands.Cog.listener()
    async def on_member_update(self, before: discord.Member, after: discord.Member):
        """Startet Onboarding automatisch wenn Discord-Mitglieder-Screening abgeschlossen wird."""
        if after.guild.id != MAIN_GUILD_ID:
            return
        if before.pending and not after.pending:
            await self._auto_start_onboarding(after)

    async def _auto_start_onboarding(self, member: discord.Member):
        """Erstellt Thread + startet Onboarding ohne Interaction (via member_update)."""
        channel = member.guild.get_channel(RULES_CHANNEL_ID)
        if not isinstance(channel, discord.TextChannel):
            log.warning(
                "Regelkanal %s nicht gefunden beim Auto-Onboarding für %s",
                RULES_CHANNEL_ID,
                member.id,
            )
            return

        name = f"onboarding-{member.name}".replace(" ", "-")[:90]
        try:
            thread = await _create_onboarding_thread(
                channel,
                name=name,
                invite_user=member,
                log_context=f"member={member.id}",
            )
        except discord.HTTPException as exc:
            if is_transient_discord_http_error(exc):
                log.warning(
                    "Thread-Erstellung temporär fehlgeschlagen beim Auto-Onboarding für %s: %s",
                    member.id,
                    exc,
                )
                return
            log.error(
                "Thread-Erstellung fehlgeschlagen beim Auto-Onboarding für %s: %s",
                member.id,
                exc,
            )
            return
        except Exception:
            log.error("Thread-Erstellung fehlgeschlagen beim Auto-Onboarding für %s", member.id)
            return

        onboard_cog = self.bot.get_cog("StaticOnboarding")
        if onboard_cog and hasattr(onboard_cog, "start_in_channel"):
            try:
                await onboard_cog.start_in_channel(thread, member)
            except Exception:
                log.exception(
                    "StaticOnboarding.start_in_channel fehlgeschlagen beim Auto-Start für %s",
                    member.id,
                )
        else:
            fallback_embed = discord.Embed(
                title="Willkommen!",
                description=(
                    "Das Onboarding ist gerade nicht verfügbar.\n"
                    "Schau in #ankündigungen, finde Mitspieler in #spieler-suche "
                    "und richte dir im Sprachkanal-Panel einen eigenen Sprachkanal ein.\n"
                    "Fragen? Nutze /faq oder ping das Team. 😊"
                ),
                color=0x5865F2,
            )
            try:
                await retry_discord_http(
                    lambda: thread.send(embed=fallback_embed),
                    log=log,
                    operation_name=f"send auto-onboarding fallback to thread {thread.id}",
                )
            except discord.HTTPException as exc:
                log.warning(
                    "Fallback-Onboarding-Nachricht konnte für %s nicht gesendet werden: %s",
                    member.id,
                    exc,
                )

    # ----- Start-Flow im Thread -----
    async def start_in_thread(self, interaction: discord.Interaction):
        thread = await _create_user_thread(interaction)
        if not thread:
            return

        # Nutzer informieren
        try:
            if not interaction.response.is_done():
                await interaction.response.send_message(
                    f"🚀 Onboarding in {thread.mention} gestartet.", ephemeral=True
                )
            else:
                await interaction.followup.send(
                    f"🚀 Onboarding in {thread.mention} gestartet.", ephemeral=True
                )
        except Exception as exc:
            log.debug("Konnte Start-Hinweis nicht senden: %s", exc)

        # Statisches Onboarding starten
        onboard_cog = self.bot.get_cog("StaticOnboarding")
        if onboard_cog and hasattr(onboard_cog, "start_in_channel"):
            try:
                ok = await onboard_cog.start_in_channel(thread, interaction.user)  # type: ignore
                if ok:
                    return
            except Exception as e:
                log.warning("StaticOnboarding.start_in_channel fehlgeschlagen: %r", e)

        # Minimaler Fallback, falls die KI nicht läuft
        fallback_embed = discord.Embed(
            title="Willkommen!",
            description=(
                "Das Onboarding ist gerade nicht verfügbar.\n"
                "Schau in #ankündigungen, finde Mitspieler in #spieler-suche "
                "und richte dir im Sprachkanal-Panel einen eigenen Sprachkanal ein.\n"
                "Fragen? Nutze /faq oder ping das Team. 😊"
            ),
            color=0x5865F2,
        )
        try:
            await retry_discord_http(
                lambda: thread.send(embed=fallback_embed),
                log=log,
                operation_name=f"send onboarding fallback to thread {thread.id}",
            )
        except discord.HTTPException as exc:
            log.warning(
                "Fallback-Onboarding-Nachricht konnte in Thread %s nicht gesendet werden: %s",
                thread.id,
                exc,
            )


async def setup(bot: commands.Bot):
    await bot.add_cog(RulesPanel(bot))
