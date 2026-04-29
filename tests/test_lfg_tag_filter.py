from __future__ import annotations

import os
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path
from unittest import mock

import discord

from cogs.lfg import SmartLFGAgent
from service import db


def _reset_test_db(path: Path) -> None:
    db.close_connection()
    db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
    db.DB_PATH = path  # type: ignore[assignment]
    os.environ["DEADLOCK_DB_PATH"] = str(path)
    db.connect()


class _FakeTagService:
    def __init__(self, tags_by_user: dict[int, dict[str, str]] | None = None) -> None:
        self._tags_by_user = tags_by_user or {}

    async def get_user_tags(self, user_id: int) -> dict[str, str]:
        return dict(self._tags_by_user.get(int(user_id), {}))


class _FakeBot:
    def __init__(self, *, tag_service: _FakeTagService | None = None) -> None:
        self._tag_service = tag_service

    def get_cog(self, name: str) -> object | None:
        if name == "TagService":
            return self._tag_service
        return None


class _FakeMember:
    def __init__(
        self,
        *,
        user_id: int,
        guild: _FakeGuild,
        display_name: str,
        bot: bool = False,
    ) -> None:
        self.id = user_id
        self.guild = guild
        self.display_name = display_name
        self.bot = bot
        self.voice = None
        self.status = discord.Status.online
        self.roles: list[object] = []
        self.mention = f"<@{user_id}>"


class _FakeGuild:
    def __init__(self, members: list[_FakeMember]) -> None:
        self._members = {member.id: member for member in members}

    def get_member(self, user_id: int) -> _FakeMember | None:
        return self._members.get(int(user_id))

    def get_channel(self, _channel_id: int) -> None:
        return None


class SmartLFGTagFilterTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        super().setUp()
        self._tmpdir = tempfile.TemporaryDirectory()
        self._previous_db_path = os.environ.get("DEADLOCK_DB_PATH")
        _reset_test_db(Path(self._tmpdir.name) / "lfg-tag-filter.sqlite3")

    def tearDown(self) -> None:
        db.close_connection()
        db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
        if self._previous_db_path is None:
            os.environ.pop("DEADLOCK_DB_PATH", None)
        else:
            os.environ["DEADLOCK_DB_PATH"] = self._previous_db_path
        self._tmpdir.cleanup()
        super().tearDown()

    async def test_find_matching_players_applies_age_and_tone_filters(self) -> None:
        author = _FakeMember(user_id=1000, guild=None, display_name="Author")  # type: ignore[arg-type]
        member_ok = _FakeMember(user_id=2001, guild=None, display_name="Passend")  # type: ignore[arg-type]
        member_missing_age = _FakeMember(
            user_id=2002,
            guild=None,
            display_name="Zu jung",
        )  # type: ignore[arg-type]
        member_missing_tone = _FakeMember(
            user_id=2003,
            guild=None,
            display_name="Kein Tone-Tag",
        )  # type: ignore[arg-type]
        guild = _FakeGuild([author, member_ok, member_missing_age, member_missing_tone])
        author.guild = guild
        member_ok.guild = guild
        member_missing_age.guild = guild
        member_missing_tone.guild = guild

        db.execute(
            "INSERT INTO user_tags(user_id, tag_key, tag_value) VALUES (?, ?, ?)",
            (2001, "age", "25+"),
        )
        db.execute(
            "INSERT INTO user_tags(user_id, tag_key, tag_value) VALUES (?, ?, ?)",
            (2001, "tone", "ragebaiter_free"),
        )
        db.execute(
            "INSERT INTO user_tags(user_id, tag_key, tag_value) VALUES (?, ?, ?)",
            (2002, "tone", "ragebaiter_free"),
        )
        db.execute(
            "INSERT INTO user_tags(user_id, tag_key, tag_value) VALUES (?, ?, ?)",
            (2003, "age", "25+"),
        )

        agent = SmartLFGAgent(_FakeBot())  # type: ignore[arg-type]
        steam_links = {
            1000: ["steam-author"],
            2001: ["steam-2001"],
            2002: ["steam-2002"],
            2003: ["steam-2003"],
        }
        online_users = {
            "steam-2001": ("lobby", None),
            "steam-2002": ("lobby", None),
            "steam-2003": ("lobby", None),
        }

        with (
            mock.patch.object(agent, "_get_steam_friend_ids", new=mock.AsyncMock(return_value=set())),
            mock.patch.object(agent, "_fetch_co_player_stats", new=mock.AsyncMock(return_value={})),
            mock.patch.object(agent, "_fetch_activity_patterns", new=mock.AsyncMock(return_value={})),
            mock.patch.object(agent, "_fetch_lane_activity_users", new=mock.AsyncMock(return_value=set())),
        ):
            candidates = await agent._find_matching_players(
                author,
                "suche mitspieler 25+ ragebaiter-free",
                0,
                None,
                steam_links=steam_links,
                online_users=online_users,
            )

        self.assertEqual([candidate["user_id"] for candidate in candidates], [2001])

    async def test_find_matching_players_excludes_active_ragebaiter_on_ragebaiter_free(self) -> None:
        author = _FakeMember(user_id=1000, guild=None, display_name="Author")  # type: ignore[arg-type]
        member_ok = _FakeMember(user_id=2101, guild=None, display_name="Safe")  # type: ignore[arg-type]
        member_ragebaiter = _FakeMember(
            user_id=2102,
            guild=None,
            display_name="Geblockt",
        )  # type: ignore[arg-type]
        guild = _FakeGuild([author, member_ok, member_ragebaiter])
        author.guild = guild
        member_ok.guild = guild
        member_ragebaiter.guild = guild

        for user_id in (2101, 2102):
            db.execute(
                "INSERT INTO user_tags(user_id, tag_key, tag_value) VALUES (?, ?, ?)",
                (user_id, "tone", "ragebaiter_free"),
            )

        db.execute(
            """
            INSERT INTO user_mod_tags(user_id, mod_tag, set_by, reason, expires_at)
            VALUES (?, ?, ?, ?, ?)
            """,
            (
                2102,
                "ragebaiter",
                99,
                "test",
                (datetime.now(UTC) + timedelta(days=3)).isoformat(),
            ),
        )

        agent = SmartLFGAgent(_FakeBot())  # type: ignore[arg-type]
        steam_links = {
            1000: ["steam-author"],
            2101: ["steam-2101"],
            2102: ["steam-2102"],
        }
        online_users = {
            "steam-2101": ("lobby", None),
            "steam-2102": ("lobby", None),
        }

        with (
            mock.patch.object(agent, "_get_steam_friend_ids", new=mock.AsyncMock(return_value=set())),
            mock.patch.object(agent, "_fetch_co_player_stats", new=mock.AsyncMock(return_value={})),
            mock.patch.object(agent, "_fetch_activity_patterns", new=mock.AsyncMock(return_value={})),
            mock.patch.object(agent, "_fetch_lane_activity_users", new=mock.AsyncMock(return_value=set())),
        ):
            candidates = await agent._find_matching_players(
                author,
                "lfg ragebaiter-free",
                0,
                None,
                steam_links=steam_links,
                online_users=online_users,
            )

        self.assertEqual([candidate["user_id"] for candidate in candidates], [2101])

    async def test_get_visible_user_tag_line_formats_supported_tags(self) -> None:
        bot = _FakeBot(
            tag_service=_FakeTagService(
                {
                    3001: {
                        "age": "25+",
                        "tone": "banter_ok",
                    }
                }
            )
        )
        agent = SmartLFGAgent(bot)  # type: ignore[arg-type]

        line = await agent._get_visible_user_tag_line(3001)

        self.assertEqual(line, "Tags: 25+ · Banter-OK")


if __name__ == "__main__":
    unittest.main(verbosity=2)
