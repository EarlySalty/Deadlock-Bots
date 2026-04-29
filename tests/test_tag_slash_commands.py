from __future__ import annotations

import os
import tempfile
import types
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import discord

from cogs.tags import setup
from cogs.tags.core import TagService
from service import db


def _reset_test_db(path: Path) -> None:
    db.close_connection()
    db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
    db.DB_PATH = path  # type: ignore[assignment]
    os.environ["DEADLOCK_DB_PATH"] = str(path)
    db.connect()


class _FakePermissions:
    def __init__(self, *, manage_messages: bool = False) -> None:
        self.manage_messages = manage_messages


class _FakeRole:
    def __init__(self, role_id: int) -> None:
        self.id = role_id


class _FakeUser:
    def __init__(
        self,
        user_id: int,
        *,
        manage_messages: bool = False,
        role_ids: list[int] | None = None,
        display_name: str | None = None,
    ) -> None:
        self.id = user_id
        self.display_name = display_name or f"user-{user_id}"
        self.mention = f"<@{user_id}>"
        self.guild_permissions = _FakePermissions(manage_messages=manage_messages)
        self.roles = [_FakeRole(role_id) for role_id in (role_ids or [])]


class _FakeResponse:
    def __init__(self) -> None:
        self.sent_messages: list[dict[str, Any]] = []
        self.edited_messages: list[dict[str, Any]] = []
        self.deferred: list[dict[str, Any]] = []
        self._done = False

    def is_done(self) -> bool:
        return self._done

    async def send_message(
        self,
        content: str | None = None,
        *,
        embed: discord.Embed | None = None,
        view: discord.ui.View | None = None,
        ephemeral: bool = False,
    ) -> None:
        self.sent_messages.append(
            {
                "content": content,
                "embed": embed,
                "view": view,
                "ephemeral": ephemeral,
            }
        )
        self._done = True

    async def edit_message(
        self,
        *,
        content: str | None = None,
        embed: discord.Embed | None = None,
        view: discord.ui.View | None = None,
    ) -> None:
        self.edited_messages.append({"content": content, "embed": embed, "view": view})
        self._done = True

    async def defer(self, *, ephemeral: bool = False, thinking: bool = False) -> None:
        self.deferred.append({"ephemeral": ephemeral, "thinking": thinking})
        self._done = True


class _FakeFollowup:
    def __init__(self) -> None:
        self.sent_messages: list[dict[str, Any]] = []

    async def send(
        self,
        content: str | None = None,
        *,
        embed: discord.Embed | None = None,
        view: discord.ui.View | None = None,
        ephemeral: bool = False,
    ) -> None:
        self.sent_messages.append(
            {
                "content": content,
                "embed": embed,
                "view": view,
                "ephemeral": ephemeral,
            }
        )


class _FakeInteraction:
    def __init__(self, user: _FakeUser) -> None:
        self.user = user
        self.guild = types.SimpleNamespace(id=1, owner_id=999)
        self.response = _FakeResponse()
        self.followup = _FakeFollowup()


class _FakeLogChannel:
    def __init__(self, channel_id: int) -> None:
        self.id = channel_id
        self.sent_messages: list[dict[str, Any]] = []

    async def send(self, content: str | None = None, **kwargs: Any) -> None:
        payload = {"content": content}
        payload.update(kwargs)
        self.sent_messages.append(payload)


class _FakeBot:
    def __init__(self) -> None:
        self._cogs: dict[str, object] = {}
        self._channels: dict[int, object] = {}
        self.added_cogs: list[object] = []
        self.added_views: list[tuple[discord.ui.View, int | None]] = []
        self.dispatched: list[tuple[str, tuple[object, ...]]] = []

    def dispatch(self, event_name: str, *args: object) -> None:
        self.dispatched.append((event_name, args))

    async def add_cog(self, cog: object) -> None:
        self.added_cogs.append(cog)
        self._cogs[cog.__class__.__name__] = cog

    def get_cog(self, name: str) -> object | None:
        return self._cogs.get(name)

    def add_channel(self, channel: object) -> None:
        self._channels[int(channel.id)] = channel  # type: ignore[attr-defined]

    def get_channel(self, channel_id: int) -> object | None:
        return self._channels.get(int(channel_id))

    async def fetch_channel(self, channel_id: int) -> object | None:
        return self.get_channel(channel_id)

    def add_view(self, view: discord.ui.View, *, message_id: int | None = None) -> None:
        self.added_views.append((view, message_id))


class TagSlashCommandTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        super().setUp()
        self._tmpdir = tempfile.TemporaryDirectory()
        self._previous_db_path = os.environ.get("DEADLOCK_DB_PATH")
        _reset_test_db(Path(self._tmpdir.name) / "tag-slash.sqlite3")
        self.bot = _FakeBot()
        self.service = TagService(self.bot)  # type: ignore[arg-type]
        self.bot._cogs["TagService"] = self.service

    async def asyncTearDown(self) -> None:
        await self.service.cog_unload()
        db.close_connection()
        db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
        if self._previous_db_path is None:
            os.environ.pop("DEADLOCK_DB_PATH", None)
        else:
            os.environ["DEADLOCK_DB_PATH"] = self._previous_db_path
        self._tmpdir.cleanup()
        await super().asyncTearDown()

    async def test_meine_tags_command_sends_ephemeral_view(self) -> None:
        from cogs.tags.interface import TagInterface

        await self.service.set_user_tag(1001, "age", "25+")
        cog = TagInterface(self.bot)  # type: ignore[arg-type]
        interaction = _FakeInteraction(_FakeUser(1001))

        await TagInterface.meine_tags.callback(cog, interaction)

        self.assertEqual(len(interaction.response.sent_messages), 1)
        payload = interaction.response.sent_messages[0]
        self.assertTrue(payload["ephemeral"])
        self.assertIsInstance(payload["view"], discord.ui.View)
        self.assertIn("Voice-Lobbies", payload["embed"].description)

    async def test_my_tags_view_save_persists_selected_tags(self) -> None:
        from cogs.tags.interface import MeineTagsView

        view = MeineTagsView(self.service, user_id=1002)
        view.pending_tags["age"] = "u25"
        view.pending_tags["tone"] = "banter_ok"
        interaction = _FakeInteraction(_FakeUser(1002))

        await view._save_tags(interaction)

        self.assertEqual(
            await self.service.get_user_tags(1002),
            {"age": "u25", "tone": "banter_ok"},
        )
        self.assertEqual(len(interaction.response.edited_messages), 1)
        payload = interaction.response.edited_messages[0]
        self.assertIn("gespeichert", payload["embed"].description.lower())

    async def test_my_tags_view_reset_clears_existing_tags(self) -> None:
        from cogs.tags.interface import MeineTagsView

        await self.service.set_user_tag(1003, "age", "25+")
        await self.service.set_user_tag(1003, "tone", "ragebaiter_free")
        view = MeineTagsView(
            self.service,
            user_id=1003,
            current_tags={"age": "25+", "tone": "ragebaiter_free"},
        )
        interaction = _FakeInteraction(_FakeUser(1003))

        await view._reset_tags(interaction)

        self.assertEqual(await self.service.get_user_tags(1003), {})
        self.assertEqual(len(interaction.response.edited_messages), 1)
        payload = interaction.response.edited_messages[0]
        self.assertIn("zurückgesetzt", payload["embed"].description.lower())

    async def test_mod_tag_set_uses_default_expiry_and_logs(self) -> None:
        from cogs.tags.mod_commands import LOG_CHANNEL_ID, MOD_ROLE_ID, ModTagCommands

        log_channel = _FakeLogChannel(LOG_CHANNEL_ID)
        self.bot.add_channel(log_channel)
        cog = ModTagCommands(self.bot)  # type: ignore[arg-type]
        interaction = _FakeInteraction(_FakeUser(2001, role_ids=[MOD_ROLE_ID]))
        target = _FakeUser(3001, display_name="target-user")

        before = datetime.now(UTC)
        await ModTagCommands.set_tag.callback(
            cog,
            interaction,
            target,
            "ragebaiter",
            "provokation",
            None,
        )
        after = datetime.now(UTC)

        self.assertTrue(await self.service.has_active_mod_tag(3001, "ragebaiter"))
        row = db.query_one(
            """
            SELECT expires_at, reason, set_by
            FROM user_mod_tags
            WHERE user_id = ? AND mod_tag = ?
            """,
            (3001, "ragebaiter"),
        )
        self.assertIsNotNone(row)
        expires_at = datetime.fromisoformat(row["expires_at"])
        self.assertEqual(row["reason"], "provokation")
        self.assertEqual(row["set_by"], 2001)
        self.assertGreaterEqual(expires_at, before + timedelta(days=14) - timedelta(minutes=1))
        self.assertLessEqual(expires_at, after + timedelta(days=14) + timedelta(minutes=1))
        self.assertEqual(len(log_channel.sent_messages), 1)
        self.assertIn("[ModTag]", log_channel.sent_messages[0]["content"])
        self.assertEqual(len(interaction.response.sent_messages), 1)
        self.assertTrue(interaction.response.sent_messages[0]["ephemeral"])

    async def test_mod_tag_set_rejects_user_without_permissions(self) -> None:
        from cogs.tags.mod_commands import ModTagCommands

        cog = ModTagCommands(self.bot)  # type: ignore[arg-type]
        interaction = _FakeInteraction(_FakeUser(2002))
        target = _FakeUser(3002)

        await ModTagCommands.set_tag.callback(
            cog,
            interaction,
            target,
            "ragebaiter",
            None,
            None,
        )

        self.assertFalse(await self.service.has_active_mod_tag(3002, "ragebaiter"))
        self.assertEqual(len(interaction.response.sent_messages), 1)
        self.assertIn("Nachrichten verwalten", interaction.response.sent_messages[0]["content"])

    async def test_mod_tag_list_shows_active_tags_with_reason_and_expiry(self) -> None:
        from cogs.tags.mod_commands import ModTagCommands

        cog = ModTagCommands(self.bot)  # type: ignore[arg-type]
        interaction = _FakeInteraction(_FakeUser(2003, manage_messages=True))
        target = _FakeUser(3003, display_name="listed-user")
        expires_at = datetime.now(UTC) + timedelta(days=3)
        await self.service.add_mod_tag(
            3003,
            "ragebaiter",
            set_by=2003,
            reason="manual review",
            expires_at=expires_at,
        )

        await ModTagCommands.list_tags.callback(cog, interaction, target)

        self.assertEqual(len(interaction.response.sent_messages), 1)
        payload = interaction.response.sent_messages[0]
        self.assertTrue(payload["ephemeral"])
        embed = payload["embed"]
        self.assertEqual(embed.title, "Mod-Tags für listed-user")
        self.assertEqual(embed.fields[0].name, "ragebaiter")
        self.assertIn("manual review", embed.fields[0].value)
        self.assertIn(expires_at.date().isoformat(), embed.fields[0].value)

    async def test_setup_registers_tag_service_and_command_cogs(self) -> None:
        setup_bot = _FakeBot()

        await setup(setup_bot)  # type: ignore[arg-type]

        self.assertEqual(
            [cog.__class__.__name__ for cog in setup_bot.added_cogs],
            ["TagService", "TagInterface", "ModTagCommands"],
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
