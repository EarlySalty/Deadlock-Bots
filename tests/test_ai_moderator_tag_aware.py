from __future__ import annotations

import os
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from cogs.ai_moderator import AIModeratorCog, AIVerdict
from cogs.tags.core import TagService
from service import db


class _FakeBot:
    def __init__(self) -> None:
        self.dispatched: list[tuple[str, tuple[object, ...]]] = []
        self._cogs: dict[str, object] = {}

    def dispatch(self, event_name: str, *args: object) -> None:
        self.dispatched.append((event_name, args))

    def get_cog(self, name: str) -> object | None:
        return self._cogs.get(name)

    def get_channel(self, channel_id: int) -> None:
        return None

    async def fetch_channel(self, channel_id: int) -> None:
        return None


class _FakeMember:
    def __init__(self, user_id: int) -> None:
        self.id = user_id
        self.bot = False
        self.display_name = f"user-{user_id}"
        self.guild_permissions = SimpleNamespace(manage_messages=False)
        self.send = AsyncMock()


def _reset_test_db(path: Path) -> None:
    db.close_connection()
    db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
    db.DB_PATH = path  # type: ignore[assignment]
    os.environ["DEADLOCK_DB_PATH"] = str(path)
    db.connect()


def _make_message(
    *,
    author: _FakeMember,
    channel_id: int,
    message_id: int,
    content: str,
) -> SimpleNamespace:
    return SimpleNamespace(
        id=message_id,
        content=content,
        attachments=[],
        author=author,
        channel=SimpleNamespace(id=channel_id),
        guild=SimpleNamespace(id=9876),
        webhook_id=None,
        is_system=lambda: False,
    )


class AIModeratorTagAwareTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        super().setUp()
        self._tmpdir = tempfile.TemporaryDirectory()
        self._previous_db_path = os.environ.get("DEADLOCK_DB_PATH")
        _reset_test_db(Path(self._tmpdir.name) / "ai-moderator-tag-aware.sqlite3")
        self.bot = _FakeBot()
        self.tag_service = TagService(self.bot)  # type: ignore[arg-type]
        self.bot._cogs["TagService"] = self.tag_service
        self.cog = AIModeratorCog(self.bot)  # type: ignore[arg-type]
        self.cog._ensure_schema_sync()
        self.cog.per_user_cooldown_seconds = 0
        self.cog.scan_channel_ids = {222, 223, 444, 445, 446}

    async def asyncTearDown(self) -> None:
        await self.tag_service.cog_unload()
        db.close_connection()
        db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
        if self._previous_db_path is None:
            os.environ.pop("DEADLOCK_DB_PATH", None)
        else:
            os.environ["DEADLOCK_DB_PATH"] = self._previous_db_path
        self._tmpdir.cleanup()
        await super().asyncTearDown()

    async def test_persistent_ragebait_sets_mod_tag_in_db(self) -> None:
        now = datetime(2026, 4, 29, 12, 0, tzinfo=UTC)
        self.cog._now_utc = lambda: now  # type: ignore[method-assign]
        self.cog.ragebait_escalate_threshold = 1
        author = _FakeMember(101)
        message = _make_message(
            author=author,
            channel_id=222,
            message_id=333,
            content="provocation one",
        )
        verdict = AIVerdict(
            verdict="ok",
            category="ragebait_ok",
            confidence=0.61,
            reason="bait",
            needs_context=False,
            raw_json="{}",
        )

        escalated = await self.cog._handle_ragebait_hit(message, verdict)

        self.assertIsNotNone(escalated)
        self.assertEqual(escalated.category, "persistent_ragebait")
        row = db.query_one(
            """
            SELECT mod_tag, set_by, reason, expires_at
            FROM user_mod_tags
            WHERE user_id = ? AND mod_tag = ?
            """,
            (101, "ragebaiter"),
        )
        self.assertIsNotNone(row)
        self.assertEqual(row["mod_tag"], "ragebaiter")
        self.assertEqual(row["set_by"], 0)
        self.assertEqual(row["reason"], "auto: persistent_ragebait")
        self.assertEqual(row["expires_at"], (now + timedelta(days=14)).isoformat())

    async def test_persistent_ragebait_resets_mod_tag_expiry_on_repeat(self) -> None:
        first_now = datetime(2026, 4, 29, 12, 0, tzinfo=UTC)
        second_now = first_now + timedelta(days=3)
        self.cog.ragebait_escalate_threshold = 1
        author = _FakeMember(202)
        message = _make_message(
            author=author,
            channel_id=223,
            message_id=334,
            content="provocation repeat",
        )
        verdict = AIVerdict(
            verdict="ok",
            category="ragebait_ok",
            confidence=0.62,
            reason="bait",
            needs_context=False,
            raw_json="{}",
        )

        self.cog._now_utc = lambda: first_now  # type: ignore[method-assign]
        await self.cog._handle_ragebait_hit(message, verdict)
        first_row = db.query_one(
            "SELECT expires_at FROM user_mod_tags WHERE user_id = ? AND mod_tag = ?",
            (202, "ragebaiter"),
        )

        self.cog._now_utc = lambda: second_now  # type: ignore[method-assign]
        await self.cog._handle_ragebait_hit(message, verdict)
        second_row = db.query_one(
            "SELECT expires_at FROM user_mod_tags WHERE user_id = ? AND mod_tag = ?",
            (202, "ragebaiter"),
        )

        self.assertIsNotNone(first_row)
        self.assertIsNotNone(second_row)
        self.assertEqual(first_row["expires_at"], (first_now + timedelta(days=14)).isoformat())
        self.assertEqual(second_row["expires_at"], (second_now + timedelta(days=14)).isoformat())
        self.assertNotEqual(first_row["expires_at"], second_row["expires_at"])

    async def test_lower_threshold_applies_only_in_ragebaiter_free_lane(self) -> None:
        db.execute(
            """
            INSERT INTO tempvoice_lane_tag_filter(channel_id, required_tone_tag)
            VALUES (?, ?)
            """,
            (444, "ragebaiter_free"),
        )
        author = _FakeMember(303)
        message = _make_message(
            author=author,
            channel_id=444,
            message_id=335,
            content="maybe harassment",
        )
        verdict = AIVerdict(
            verdict="propose",
            category="harassment",
            confidence=0.45,
            reason="borderline harassment",
            needs_context=False,
            raw_json="{}",
        )

        with patch("cogs.ai_moderator.discord.Member", _FakeMember):
            with patch.object(
                self.cog, "_classify_message", AsyncMock(return_value=(verdict, False))
            ):
                with patch.object(self.cog, "_create_proposal_case", AsyncMock()) as create_case:
                    await self.cog.on_message(message)

        create_case.assert_awaited_once()

        other_author = _FakeMember(304)
        other_message = _make_message(
            author=other_author,
            channel_id=445,
            message_id=336,
            content="maybe harassment elsewhere",
        )
        with patch("cogs.ai_moderator.discord.Member", _FakeMember):
            with patch.object(
                self.cog, "_classify_message", AsyncMock(return_value=(verdict, False))
            ):
                with patch.object(self.cog, "_create_proposal_case", AsyncMock()) as create_case:
                    await self.cog.on_message(other_message)

        create_case.assert_not_awaited()

    async def test_ragebaiter_free_lane_warns_then_proposes_on_repeat(self) -> None:
        db.execute(
            """
            INSERT INTO tempvoice_lane_tag_filter(channel_id, required_tone_tag)
            VALUES (?, ?)
            """,
            (446, "ragebaiter_free"),
        )
        author = _FakeMember(404)
        first_message = _make_message(
            author=author,
            channel_id=446,
            message_id=337,
            content="light bait one",
        )
        second_message = _make_message(
            author=author,
            channel_id=446,
            message_id=338,
            content="light bait two",
        )
        verdict = AIVerdict(
            verdict="ok",
            category="ragebait_ok",
            confidence=0.45,
            reason="light bait",
            needs_context=False,
            raw_json="{}",
        )
        first_now = datetime(2026, 4, 29, 12, 0, tzinfo=UTC)
        second_now = first_now + timedelta(minutes=10)

        with patch("cogs.ai_moderator.discord.Member", _FakeMember):
            with patch.object(self.cog, "_handle_ragebait_hit", AsyncMock(return_value=None)):
                with patch.object(
                    self.cog, "_classify_message", AsyncMock(return_value=(verdict, False))
                ):
                    with patch.object(
                        self.cog, "_create_proposal_case", AsyncMock()
                    ) as create_case:
                        self.cog._now_utc = lambda: first_now  # type: ignore[method-assign]
                        await self.cog.on_message(first_message)

                        author.send.assert_awaited_once()
                        create_case.assert_not_awaited()

                        self.cog._now_utc = lambda: second_now  # type: ignore[method-assign]
                        await self.cog.on_message(second_message)

        self.assertEqual(author.send.await_count, 1)
        create_case.assert_awaited_once()


if __name__ == "__main__":
    unittest.main(verbosity=2)
