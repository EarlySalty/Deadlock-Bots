from __future__ import annotations

from datetime import UTC, datetime, timedelta
from typing import Any

import discord
from discord import app_commands
from discord.ext import commands

from service import db

from .constants import MOD_TAG_DEFAULT_DURATIONS
from .core import TagService

MOD_ROLE_ID = 1337518124647579661
LOG_CHANNEL_ID = 1374364800817303632


class ModTagCommands(commands.GroupCog, group_name="mod-tag"):
    """Moderationsbefehle für Mod-Tags."""

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot

    def _get_tag_service(self) -> TagService:
        service = self.bot.get_cog("TagService")
        if not isinstance(service, TagService):
            raise RuntimeError("TagService is not loaded")
        return service

    def _has_mod_permission(self, interaction: discord.Interaction) -> bool:
        permissions = getattr(interaction.user, "guild_permissions", None)
        if permissions and bool(getattr(permissions, "manage_messages", False)):
            return True
        role_ids = {
            int(role.id)
            for role in getattr(interaction.user, "roles", [])
            if getattr(role, "id", None)
        }
        return MOD_ROLE_ID in role_ids

    async def _ensure_mod_permission(self, interaction: discord.Interaction) -> bool:
        if self._has_mod_permission(interaction):
            return True
        await interaction.response.send_message(
            "Dafür brauchst du `Nachrichten verwalten` oder die Mod-Rolle.",
            ephemeral=True,
        )
        return False

    async def _resolve_log_channel(self) -> Any | None:
        channel = self.bot.get_channel(LOG_CHANNEL_ID)
        if channel is not None:
            return channel
        return await self.bot.fetch_channel(LOG_CHANNEL_ID)

    async def _log_action(self, message: str) -> None:
        channel = await self._resolve_log_channel()
        if channel is None or not hasattr(channel, "send"):
            return
        await channel.send(message)

    def _format_expiry(self, expires_at: datetime | None) -> str:
        if expires_at is None:
            return "unbegrenzt"
        return expires_at.astimezone(UTC).date().isoformat()

    def _resolve_expiry(self, tag: str, expires_in_days: int | None) -> datetime | None:
        if expires_in_days is not None:
            return datetime.now(UTC) + timedelta(days=expires_in_days)
        duration = MOD_TAG_DEFAULT_DURATIONS.get(tag)
        if duration is None:
            return None
        return datetime.now(UTC) + duration

    def _parse_datetime(self, raw: Any) -> datetime | None:
        if raw is None:
            return None
        text = str(raw).strip()
        if not text:
            return None
        parsed = datetime.fromisoformat(text)
        if parsed.tzinfo is None:
            return parsed.replace(tzinfo=UTC)
        return parsed.astimezone(UTC)

    async def _fetch_active_mod_tag_rows(self, user_id: int) -> list[dict[str, Any]]:
        rows = await db.query_all_async(
            """
            SELECT mod_tag, set_by, reason, expires_at
            FROM user_mod_tags
            WHERE user_id = ?
            ORDER BY mod_tag
            """,
            (int(user_id),),
        )
        now = datetime.now(UTC)
        active_rows: list[dict[str, Any]] = []
        for row in rows:
            expires_at = self._parse_datetime(row["expires_at"])
            if expires_at is not None and expires_at <= now:
                continue
            active_rows.append(
                {
                    "mod_tag": str(row["mod_tag"]),
                    "set_by": int(row["set_by"]),
                    "reason": row["reason"],
                    "expires_at": expires_at,
                }
            )
        return active_rows

    @app_commands.command(name="set", description="Setzt ein Mod-Tag für einen User.")
    @app_commands.guild_only()
    @app_commands.describe(
        user="Betroffener User",
        tag="Zu setzendes Mod-Tag",
        reason="Optionaler Grund",
        expires_in_days="Ablauf in Tagen",
    )
    @app_commands.choices(tag=[app_commands.Choice(name="Ragebaiter", value="ragebaiter")])
    async def set_tag(
        self,
        interaction: discord.Interaction,
        user: discord.Member,
        tag: str,
        reason: str | None = None,
        expires_in_days: int | None = None,
    ) -> None:
        if not await self._ensure_mod_permission(interaction):
            return
        if expires_in_days is not None and expires_in_days <= 0:
            await interaction.response.send_message(
                "`expires_in_days` muss größer als 0 sein.",
                ephemeral=True,
            )
            return

        service = self._get_tag_service()
        expires_at = self._resolve_expiry(tag, expires_in_days)
        await service.add_mod_tag(
            user.id,
            tag,
            set_by=interaction.user.id,
            reason=reason,
            expires_at=expires_at,
        )
        await self._log_action(
            f"[ModTag] {user.mention} got tag '{tag}' "
            f"(by {interaction.user.mention}, reason: {reason or '-'}, "
            f"expires: {self._format_expiry(expires_at)})"
        )
        await interaction.response.send_message(
            f"Mod-Tag `{tag}` wurde für {user.mention} gesetzt.",
            ephemeral=True,
        )

    @app_commands.command(name="remove", description="Entfernt ein Mod-Tag von einem User.")
    @app_commands.guild_only()
    @app_commands.describe(
        user="Betroffener User",
        tag="Zu entfernendes Mod-Tag",
    )
    @app_commands.choices(tag=[app_commands.Choice(name="Ragebaiter", value="ragebaiter")])
    async def remove_tag(
        self,
        interaction: discord.Interaction,
        user: discord.Member,
        tag: str,
    ) -> None:
        if not await self._ensure_mod_permission(interaction):
            return

        service = self._get_tag_service()
        if not await service.has_active_mod_tag(user.id, tag):
            await interaction.response.send_message(
                f"{user.mention} hat kein aktives Mod-Tag `{tag}`.",
                ephemeral=True,
            )
            return

        await service.remove_mod_tag(user.id, tag, interaction.user.id)
        await self._log_action(
            f"[ModTag] {user.mention} lost tag '{tag}' (by {interaction.user.mention})"
        )
        await interaction.response.send_message(
            f"Mod-Tag `{tag}` wurde von {user.mention} entfernt.",
            ephemeral=True,
        )

    @app_commands.command(name="list", description="Zeigt aktive Mod-Tags eines Users.")
    @app_commands.guild_only()
    @app_commands.describe(user="Betroffener User")
    async def list_tags(self, interaction: discord.Interaction, user: discord.Member) -> None:
        if not await self._ensure_mod_permission(interaction):
            return

        rows = await self._fetch_active_mod_tag_rows(user.id)
        embed = discord.Embed(
            title=f"Mod-Tags für {getattr(user, 'display_name', user.id)}",
            color=discord.Color.orange(),
        )
        if not rows:
            embed.description = "Keine aktiven Mod-Tags."
            await interaction.response.send_message(embed=embed, ephemeral=True)
            return

        for row in rows:
            embed.add_field(
                name=row["mod_tag"],
                value=(
                    f"Reason: {row['reason'] or '-'}\n"
                    f"Expires: {self._format_expiry(row['expires_at'])}\n"
                    f"Set by: <@{row['set_by']}>"
                ),
                inline=False,
            )
        await interaction.response.send_message(embed=embed, ephemeral=True)
