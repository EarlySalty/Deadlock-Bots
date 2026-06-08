from __future__ import annotations

import io
import json
import logging

import discord
from discord import app_commands
from discord.ext import commands

from cogs import privacy_core as privacy

log = logging.getLogger(__name__)


class PrivacyConfirmView(discord.ui.View):
    def __init__(self, cog: PrivacyControls, user_id: int):
        super().__init__(timeout=600)
        self.cog = cog
        self.user_id = int(user_id)
        # final_confirm is attached via decorator below
        try:
            self.final_confirm.disabled = True
        except Exception:
            log.debug("Konnte Initialzustand der Buttons nicht setzen", exc_info=True)

    async def interaction_check(self, interaction: discord.Interaction) -> bool:
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Nur der ursprüngliche Anfragende kann diese Aktion bestätigen.",
                ephemeral=True,
            )
            return False
        return True

    @discord.ui.button(
        label="Daten herunterladen",
        style=discord.ButtonStyle.primary,
        custom_id="privacy:export",
    )
    async def export_data(
        self, interaction: discord.Interaction, _button: discord.ui.Button
    ) -> None:
        try:
            await interaction.response.defer(thinking=True, ephemeral=True)
        except Exception as exc:
            log.debug("Konnte Export-Response nicht deferen", exc_info=exc)

        try:
            data = privacy.export_user_data(self.user_id)
            payload = json.dumps(data, ensure_ascii=False, default=str, indent=2)
            buf = io.BytesIO(payload.encode("utf-8"))
            fname = f"user_{self.user_id}_datenauszug.json"
            await interaction.followup.send(
                "Hier ist deine komplette Datenauskunft (JSON).",
                file=discord.File(buf, filename=fname),
                ephemeral=True,
            )
        except Exception as exc:
            log.exception("Datenexport fehlgeschlagen")
            try:
                await interaction.followup.send(
                    f"⚠️ Konnte die Daten nicht exportieren: {exc}",
                    ephemeral=True,
                )
            except Exception as inner_exc:
                log.debug(
                    "Followup für fehlgeschlagenen Export scheiterte",
                    exc_info=inner_exc,
                )

    @discord.ui.button(
        label="Schritt 1/2: Bestätigen",
        style=discord.ButtonStyle.secondary,
        custom_id="privacy:step1",
    )
    async def step_one(self, interaction: discord.Interaction, button: discord.ui.Button) -> None:
        self.final_confirm.disabled = False
        button.disabled = True
        await interaction.response.edit_message(
            content=(
                "✅ Schritt 1/2 bestätigt.\n"
                "Klicke jetzt auf **Endgültig löschen**, um alle gespeicherten Daten zu entfernen "
                "und zukünftige Speicherung zu blockieren."
            ),
            view=self,
        )

    @discord.ui.button(
        label="Schritt 2/2: Endgültig löschen",
        style=discord.ButtonStyle.danger,
        custom_id="privacy:confirm",
    )
    async def final_confirm(
        self, interaction: discord.Interaction, button: discord.ui.Button
    ) -> None:
        self.disable_all_items()
        try:
            await interaction.response.defer(thinking=True, ephemeral=True)
        except Exception as exc:
            log.debug("Konnte Bestätigungs-Response nicht deferen", exc_info=exc)

        try:
            summary = await privacy.delete_user_data(self.user_id, reason="slash_datenschutz")
            self.cog._clear_runtime_state(self.user_id)
            msg = self.cog._format_summary(self.user_id, summary)
        except Exception as exc:
            log.exception("Datenschutz-Löschung fehlgeschlagen")
            msg = f"⚠️ Konnte die Datenlöschung nicht abschließen: {exc}"

        try:
            await interaction.edit_original_response(content=msg, view=self)
        except Exception:
            try:
                await interaction.followup.send(msg, ephemeral=True, view=self)
            except Exception:
                log.debug("Konnte Ergebnisnachricht nicht senden", exc_info=True)


class PrivacyControls(commands.Cog):
    def __init__(self, bot: commands.Bot):
        self.bot = bot

    @staticmethod
    def _summary_value(summary: dict[str, int], key: str) -> int:
        val = summary.get(key, 0)
        try:
            return int(val)
        except Exception:
            return 0

    def _clear_runtime_state(self, user_id: int) -> None:
        """Stop ongoing runtime tracking for the user (in-memory only)."""
        try:
            vat = self.bot.get_cog("VoiceActivityTrackerCog")
            if vat and hasattr(vat, "_drop_runtime_state"):
                vat._drop_runtime_state(user_id)
        except Exception:
            log.debug("Konnte VoiceActivityTracker state nicht bereinigen", exc_info=True)

        try:
            nudge = self.bot.get_cog("SteamLinkVoiceNudge")
            tasks = getattr(nudge, "_tasks", {}) if nudge else None
            if tasks is not None:
                task = tasks.pop(int(user_id), None)
                if task and not task.done():
                    task.cancel()
        except Exception:
            log.debug("Konnte Nudge-Tasks nicht bereinigen", exc_info=True)

    def _format_summary(self, user_id: int, summary: dict[str, int]) -> str:
        voice_sessions = self._summary_value(summary, "voice_session_log.user_id")
        voice_stats = self._summary_value(summary, "voice_stats.user_id")
        steam_links = self._summary_value(summary, "steam_links.user_id")
        steam_ids = summary.get("steam_ids") or []
        retention_rows = (
            self._summary_value(summary, "user_retention_tracking.user_id")
            + self._summary_value(summary, "user_retention_messages.user_id")
            + self._summary_value(summary, "member_events.user_id")
            + self._summary_value(summary, "message_activity.user_id")
            + self._summary_value(summary, "user_activity_patterns.user_id")
            + self._summary_value(summary, "user_co_players")
        )
        tempvoice_rows = (
            self._summary_value(summary, "tempvoice_lanes.owner_id")
            + self._summary_value(summary, "tempvoice_owner_prefs.owner_id")
            + self._summary_value(summary, "tempvoice_lurkers.user_id")
            + self._summary_value(summary, "tempvoice_bans.owner_id")
            + self._summary_value(summary, "tempvoice_bans.banned_id")
        )
        claims_rows = (
            self._summary_value(summary, "coaching_sessions.user_id")
            + self._summary_value(summary, "voice_channel_anchors.user_id")
        )
        partner_rows = self._summary_value(
            summary, "twitch_streamers.discord_user_id"
        ) + self._summary_value(summary, "twitch_link_clicks.discord_user_id")
        rankbot_rows = (
            self._summary_value(summary, "user_data.user_id")
            + self._summary_value(summary, "notification_log.user_id")
            + self._summary_value(summary, "notification_queue.user_id")
            + self._summary_value(summary, "dm_response_tracking.user_id")
        )
        kv_rows = (
            self._summary_value(summary, "kv_ai_onboarding_sessions")
            + self._summary_value(summary, "kv_ai_onboarding_views")
            + self._summary_value(summary, "kv_voice_nudge")
        )

        lines = [
            "✅ Deine gespeicherten Daten wurden gelöscht und ein Opt-out ist aktiv.",
            f"- Voice: {voice_sessions} Sessions, {voice_stats} Stat-Records entfernt",
            f"- Steam: {steam_links} Verknüpfungen gelöscht"
            + (f" (IDs: {', '.join(steam_ids)})" if steam_ids else ""),
            f"- Aktivitäts-/Retention-Datenbank: {retention_rows} Einträge entfernt",
            f"- TempVoice/Claims/Coaching: {tempvoice_rows + claims_rows} Einträge entfernt",
            f"- Legacy-Partner/Rank-Bot: {partner_rows + rankbot_rows} Einträge entfernt",
            f"- KI-Onboarding/Voice-Nudge KV-Einträge: {kv_rows}",
            "Zukünftige Speicherung ist blockiert, bis du mit /datenschutz-optin wieder zustimmst.",
        ]
        return "\n".join(lines)

    @app_commands.command(
        name="datenschutz",
        description="Löscht deine gespeicherten Daten und deaktiviert zukünftige Speicherung.",
    )
    async def datenschutz(self, interaction: discord.Interaction) -> None:
        view = PrivacyConfirmView(self, interaction.user.id)
        content = (
            "Dieser Vorgang entfernt gespeicherte Daten (z. B. Voice-Statistiken und Logs, "
            "Steam-Verknüpfungen, TempVoice-, Partner- und KI-Onboarding-Daten) und setzt "
            "ein Opt-out. Standardmäßig wird danach nichts Neues gespeichert. "
            "Nutze vorher gern **Daten herunterladen**, um deine Datenauskunft zu bekommen."
        )
        await interaction.response.send_message(
            content + "\n\nBitte klicke beide Schritte, um fortzufahren.",
            view=view,
            ephemeral=True,
        )

    @app_commands.command(
        name="datenschutz-optin",
        description="Reaktiviere Speicherung nach einem Opt-out.",
    )
    async def datenschutz_optin(self, interaction: discord.Interaction) -> None:
        await privacy.set_opt_in(interaction.user.id)
        await interaction.response.send_message(
            "Du hast wieder eingewilligt. Ab jetzt dürfen Features wieder Daten speichern.",
            ephemeral=True,
        )


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(PrivacyControls(bot))
