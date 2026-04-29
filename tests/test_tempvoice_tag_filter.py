from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import discord

from cogs.tempvoice.core import LaneTagFilter, TempVoiceCore
from service import db


def _reset_test_db(path: Path) -> None:
    db.close_connection()
    db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
    db.DB_PATH = path  # type: ignore[assignment]
    os.environ["DEADLOCK_DB_PATH"] = str(path)
    db.connect()


class _FakeTagService:
    def __init__(
        self,
        *,
        user_tags: dict[int, dict[str, str]] | None = None,
        mod_tags: dict[int, set[str]] | None = None,
    ) -> None:
        self._user_tags = user_tags or {}
        self._mod_tags = mod_tags or {}

    async def get_user_tags(self, user_id: int) -> dict[str, str]:
        return dict(self._user_tags.get(int(user_id), {}))

    async def has_active_mod_tag(self, user_id: int, tag: str) -> bool:
        return tag in self._mod_tags.get(int(user_id), set())


class TempVoiceTagFilterTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        super().setUp()
        self._tmpdir = tempfile.TemporaryDirectory()
        self._previous_db_path = os.environ.get("DEADLOCK_DB_PATH")
        _reset_test_db(Path(self._tmpdir.name) / "tempvoice-tag-filter.sqlite3")

    def tearDown(self) -> None:
        db.close_connection()
        db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
        if self._previous_db_path is None:
            os.environ.pop("DEADLOCK_DB_PATH", None)
        else:
            os.environ["DEADLOCK_DB_PATH"] = self._previous_db_path
        self._tmpdir.cleanup()
        super().tearDown()

    async def test_set_lane_tag_filter_persists_row(self) -> None:
        bot = mock.Mock()
        bot.get_cog.return_value = _FakeTagService()
        core = TempVoiceCore(bot)

        lane_filter = await core.set_lane_tag_filter(
            3210,
            min_age_tag="25+",
            required_tone_tag="ragebaiter_free",
            deny_ragebaiter=True,
        )

        self.assertEqual(
            lane_filter,
            LaneTagFilter(
                channel_id=3210,
                min_age_tag="25+",
                required_tone_tag="ragebaiter_free",
                deny_ragebaiter=True,
            ),
        )
        row = db.query_one(
            """
            SELECT min_age_tag, required_tone_tag, deny_ragebaiter
            FROM tempvoice_lane_tag_filter
            WHERE channel_id = ?
            """,
            (3210,),
        )
        self.assertIsNotNone(row)
        self.assertEqual(row["min_age_tag"], "25+")
        self.assertEqual(row["required_tone_tag"], "ragebaiter_free")
        self.assertEqual(row["deny_ragebaiter"], 1)

    async def test_apply_tag_filter_blocks_member_without_min_age(self) -> None:
        tag_service = _FakeTagService(
            user_tags={
                1001: {"age": "u25"},
                1002: {"age": "25+"},
            }
        )
        bot = mock.Mock()
        bot.get_cog.return_value = tag_service
        core = TempVoiceCore(bot)

        blocked_member = mock.Mock(id=1001, display_name="Blocked")
        blocked_member.voice = SimpleNamespace(channel=None)
        allowed_member = mock.Mock(id=1002, display_name="Allowed")
        allowed_member.voice = SimpleNamespace(channel=None)

        guild = mock.Mock()
        lane = mock.Mock()
        lane.id = 4444
        lane.guild = guild
        lane.members = [blocked_member, allowed_member]
        lane.overwrites_for.side_effect = lambda _target: discord.PermissionOverwrite()
        lane.set_permissions = mock.AsyncMock()

        await core._apply_tag_filter(
            lane,
            LaneTagFilter(
                channel_id=4444,
                min_age_tag="25+",
                required_tone_tag=None,
                deny_ragebaiter=False,
            ),
        )

        lane.set_permissions.assert_awaited_once()
        args, kwargs = lane.set_permissions.await_args
        self.assertIs(args[0], blocked_member)
        self.assertFalse(kwargs["overwrite"].connect)
        self.assertEqual(kwargs["reason"], "TempVoice: Tag-Filter deny")

    async def test_mod_tag_added_disconnects_member_from_blocked_lane(self) -> None:
        tag_service = _FakeTagService(mod_tags={2001: {"ragebaiter"}})
        bot = mock.Mock()
        bot.get_cog.return_value = tag_service
        core = TempVoiceCore(bot)

        member = mock.Mock(id=2001, display_name="Ragebait")
        member.voice = SimpleNamespace(channel=None)
        member.move_to = mock.AsyncMock()

        guild = mock.Mock()
        guild.get_member.return_value = member
        bot.guilds = [guild]

        lane = mock.Mock()
        lane.id = 5555
        lane.guild = guild
        lane.members = [member]
        lane.overwrites_for.side_effect = lambda _target: discord.PermissionOverwrite()
        lane.set_permissions = mock.AsyncMock()
        member.voice.channel = lane

        core.created_channels.add(lane.id)
        core.lane_owner[lane.id] = 9999
        core.lane_tag_filters[lane.id] = LaneTagFilter(
            channel_id=lane.id,
            min_age_tag=None,
            required_tone_tag=None,
            deny_ragebaiter=True,
        )

        await core.on_mod_tag_added(2001, "ragebaiter", 42, "cleanup")

        member.move_to.assert_awaited_once_with(None, reason="TempVoice: Ragebaiter tag applied")
        lane.set_permissions.assert_awaited_once()
        args, kwargs = lane.set_permissions.await_args
        self.assertIs(args[0], member)
        self.assertFalse(kwargs["overwrite"].connect)
        self.assertEqual(kwargs["reason"], "TempVoice: Tag-Filter deny")


if __name__ == "__main__":
    unittest.main(verbosity=2)
