from __future__ import annotations

from typing import Any

import discord
from discord import app_commands
from discord.ext import commands

from .core import TagService

_UNSET_VALUE = "__unset__"
_AGE_LABELS = {
    "25+": "25+",
    "u25": "U25",
}
_TONE_LABELS = {
    "banter_ok": "Banter-OK",
    "ragebaiter_free": "Ragebaiter-Free",
}


class _TagSelect(discord.ui.Select):
    def __init__(
        self,
        *,
        tag_key: str,
        placeholder: str,
        options: list[discord.SelectOption],
    ) -> None:
        super().__init__(
            custom_id=f"tags:{tag_key}",
            placeholder=placeholder,
            min_values=1,
            max_values=1,
            options=options,
        )
        self.tag_key = tag_key

    async def callback(self, interaction: discord.Interaction) -> None:
        view = self.view
        if not isinstance(view, MeineTagsView):
            return
        selected = self.values[0]
        view.pending_tags[self.tag_key] = None if selected == _UNSET_VALUE else selected
        view._sync_button_state()
        await interaction.response.edit_message(
            embed=view.build_embed(status_text="Auswahl aktualisiert."),
            view=view,
        )


class MeineTagsView(discord.ui.View):
    def __init__(
        self,
        tag_service: TagService,
        *,
        user_id: int,
        current_tags: dict[str, str] | None = None,
    ) -> None:
        super().__init__(timeout=600)
        self.tag_service = tag_service
        self.user_id = int(user_id)
        self.current_tags = dict(current_tags or {})
        self.pending_tags: dict[str, str | None] = {
            "age": self.current_tags.get("age"),
            "tone": self.current_tags.get("tone"),
        }
        self.add_item(
            _TagSelect(
                tag_key="age",
                placeholder="Alter auswählen",
                options=self._build_age_options(),
            )
        )
        self.add_item(
            _TagSelect(
                tag_key="tone",
                placeholder="Tonfall auswählen",
                options=self._build_tone_options(),
            )
        )
        self._sync_button_state()

    async def interaction_check(self, interaction: discord.Interaction) -> bool:
        if interaction.user.id != self.user_id:
            await interaction.response.send_message(
                "Nur du kannst deine Tags in dieser Ansicht ändern.",
                ephemeral=True,
            )
            return False
        return True

    def _build_age_options(self) -> list[discord.SelectOption]:
        current_age = self.pending_tags.get("age")
        return [
            discord.SelectOption(
                label="Nicht gesetzt",
                value=_UNSET_VALUE,
                default=current_age is None,
            ),
            discord.SelectOption(
                label="25+",
                value="25+",
                default=current_age == "25+",
            ),
            discord.SelectOption(
                label="U25",
                value="u25",
                default=current_age == "u25",
            ),
        ]

    def _build_tone_options(self) -> list[discord.SelectOption]:
        current_tone = self.pending_tags.get("tone")
        return [
            discord.SelectOption(
                label="Nicht gesetzt",
                value=_UNSET_VALUE,
                default=current_tone is None,
            ),
            discord.SelectOption(
                label="Banter-OK",
                value="banter_ok",
                description="Hier darf es mal ruppiger werden.",
                default=current_tone == "banter_ok",
            ),
            discord.SelectOption(
                label="Ragebaiter-Free",
                value="ragebaiter_free",
                description="Hier bitte ohne gezielte Provokationen.",
                default=current_tone == "ragebaiter_free",
            ),
        ]

    def _sync_button_state(self) -> None:
        has_any_tag = any(value is not None for value in self.pending_tags.values())
        self.reset_button.disabled = not has_any_tag

    def _label_for(self, key: str, value: str | None) -> str:
        if value is None:
            return "Nicht gesetzt"
        if key == "age":
            return _AGE_LABELS.get(value, value)
        return _TONE_LABELS.get(value, value)

    def build_embed(self, *, status_text: str | None = None) -> discord.Embed:
        description = "Wähle deinen Lieblings-Ton, damit Voice-Lobbies besser zu dir passen."
        if status_text:
            description = f"{description}\n\n{status_text}"
        embed = discord.Embed(
            title="Meine Tags",
            description=description,
            color=discord.Color.blurple(),
        )
        embed.add_field(
            name="Aktuell",
            value=(
                f"Alter: {self._label_for('age', self.current_tags.get('age'))}\n"
                f"Tonfall: {self._label_for('tone', self.current_tags.get('tone'))}"
            ),
            inline=False,
        )
        embed.add_field(
            name="Ausgewählt",
            value=(
                f"Alter: {self._label_for('age', self.pending_tags.get('age'))}\n"
                f"Tonfall: {self._label_for('tone', self.pending_tags.get('tone'))}"
            ),
            inline=False,
        )
        return embed

    async def _save_tags(self, interaction: discord.Interaction) -> None:
        for key in ("age", "tone"):
            value = self.pending_tags.get(key)
            if value is None:
                await self.tag_service.clear_user_tag(self.user_id, key)
            else:
                await self.tag_service.set_user_tag(self.user_id, key, value)
        self.current_tags = {
            key: value for key, value in self.pending_tags.items() if value is not None
        }
        self._sync_button_state()
        await interaction.response.edit_message(
            embed=self.build_embed(status_text="Deine Tags wurden gespeichert."),
            view=self,
        )

    async def _reset_tags(self, interaction: discord.Interaction) -> None:
        for key in ("age", "tone"):
            await self.tag_service.clear_user_tag(self.user_id, key)
            self.pending_tags[key] = None
        self.current_tags = {}
        self._sync_button_state()
        await interaction.response.edit_message(
            embed=self.build_embed(status_text="Deine Tags wurden zurückgesetzt."),
            view=self,
        )

    @discord.ui.button(
        label="Speichern",
        style=discord.ButtonStyle.primary,
        custom_id="tags:save",
        row=2,
    )
    async def save_button(
        self, interaction: discord.Interaction, _button: discord.ui.Button[Any]
    ) -> None:
        await self._save_tags(interaction)

    @discord.ui.button(
        label="Reset",
        style=discord.ButtonStyle.secondary,
        custom_id="tags:reset",
        row=2,
    )
    async def reset_button(
        self, interaction: discord.Interaction, _button: discord.ui.Button[Any]
    ) -> None:
        await self._reset_tags(interaction)


class TagInterface(commands.Cog):
    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot

    def _get_tag_service(self) -> TagService:
        service = self.bot.get_cog("TagService")
        if not isinstance(service, TagService):
            raise RuntimeError("TagService is not loaded")
        return service

    @app_commands.command(
        name="meine-tags",
        description="Verwalte deine sichtbaren Voice-Tags.",
    )
    @app_commands.guild_only()
    async def meine_tags(self, interaction: discord.Interaction) -> None:
        service = self._get_tag_service()
        current_tags = await service.get_user_tags(interaction.user.id)
        view = MeineTagsView(service, user_id=interaction.user.id, current_tags=current_tags)
        await interaction.response.send_message(
            embed=view.build_embed(),
            view=view,
            ephemeral=True,
        )
