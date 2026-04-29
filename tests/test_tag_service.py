from __future__ import annotations

import asyncio
import os
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path

from cogs.tags.core import TagService
from service import db


class _FakeBot:
    def __init__(self) -> None:
        self.dispatched: list[tuple[str, tuple[object, ...]]] = []

    def dispatch(self, event_name: str, *args: object) -> None:
        self.dispatched.append((event_name, args))


def _reset_test_db(path: Path) -> None:
    db.close_connection()
    db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
    db.DB_PATH = path  # type: ignore[assignment]
    os.environ["DEADLOCK_DB_PATH"] = str(path)
    db.connect()


class TagServiceTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        super().setUp()
        self._tmpdir = tempfile.TemporaryDirectory()
        self._previous_db_path = os.environ.get("DEADLOCK_DB_PATH")
        _reset_test_db(Path(self._tmpdir.name) / "tag-service.sqlite3")
        self.bot = _FakeBot()
        self.service = TagService(self.bot)  # type: ignore[arg-type]

    def tearDown(self) -> None:
        db.close_connection()
        db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
        if self._previous_db_path is None:
            os.environ.pop("DEADLOCK_DB_PATH", None)
        else:
            os.environ["DEADLOCK_DB_PATH"] = self._previous_db_path
        self._tmpdir.cleanup()
        super().tearDown()

    async def asyncTearDown(self) -> None:
        await self.service.cog_unload()
        await super().asyncTearDown()

    async def test_set_and_get_user_tag(self) -> None:
        await self.service.set_user_tag(1001, "tone", "banter_ok")

        tags = await self.service.get_user_tags(1001)

        self.assertEqual(tags, {"tone": "banter_ok"})
        row = db.query_one(
            "SELECT tag_value FROM user_tags WHERE user_id = ? AND tag_key = ?",
            (1001, "tone"),
        )
        self.assertIsNotNone(row)
        self.assertEqual(row["tag_value"], "banter_ok")

    async def test_clear_user_tag(self) -> None:
        await self.service.set_user_tag(1002, "age", "25+")

        await self.service.clear_user_tag(1002, "age")

        self.assertEqual(await self.service.get_user_tags(1002), {})
        row = db.query_one(
            "SELECT 1 FROM user_tags WHERE user_id = ? AND tag_key = ?",
            (1002, "age"),
        )
        self.assertIsNone(row)

    async def test_set_mod_tag_with_expiry(self) -> None:
        expires_at = datetime.now(UTC) + timedelta(days=7)

        await self.service.add_mod_tag(
            1003,
            "ragebaiter",
            set_by=777,
            reason="manual test",
            expires_at=expires_at,
        )

        tags = await self.service.get_mod_tags(1003)
        row = db.query_one(
            """
            SELECT mod_tag, set_by, reason, expires_at
            FROM user_mod_tags
            WHERE user_id = ? AND mod_tag = ?
            """,
            (1003, "ragebaiter"),
        )
        self.assertEqual(tags, ["ragebaiter"])
        self.assertIsNotNone(row)
        self.assertEqual(row["set_by"], 777)
        self.assertEqual(row["reason"], "manual test")
        self.assertEqual(row["expires_at"], expires_at.isoformat())

    async def test_has_active_mod_tag_respects_expiry(self) -> None:
        expired_at = datetime.now(UTC) - timedelta(minutes=1)
        db.execute(
            """
            INSERT INTO user_mod_tags(user_id, mod_tag, set_by, reason, expires_at)
            VALUES (?, ?, ?, ?, ?)
            """,
            (1004, "ragebaiter", 999, "expired", expired_at.isoformat()),
        )

        await self.service.cog_load()

        self.assertFalse(await self.service.has_active_mod_tag(1004, "ragebaiter"))
        self.assertEqual(await self.service.get_mod_tags(1004), [])

    async def test_invalid_tag_value_raises(self) -> None:
        with self.assertRaises(ValueError):
            await self.service.set_user_tag(1005, "tone", "invalid")

    async def test_mod_tag_cleanup_removes_expired(self) -> None:
        expired_at = datetime.now(UTC) - timedelta(minutes=5)
        db.execute(
            """
            INSERT INTO user_mod_tags(user_id, mod_tag, set_by, reason, expires_at)
            VALUES (?, ?, ?, ?, ?)
            """,
            (1006, "ragebaiter", 555, "expired cleanup", expired_at.isoformat()),
        )

        removed = await self.service._cleanup_expired_mod_tags()

        self.assertEqual(removed, [(1006, "ragebaiter")])
        row = db.query_one(
            "SELECT 1 FROM user_mod_tags WHERE user_id = ? AND mod_tag = ?",
            (1006, "ragebaiter"),
        )
        self.assertIsNone(row)
        self.assertIn(("mod_tag_removed", (1006, "ragebaiter", 0)), self.bot.dispatched)

    async def test_dispatches_tag_changed_event(self) -> None:
        await self.service.set_user_tag(1007, "tone", "ragebaiter_free")

        self.assertIn(
            ("tag_changed", (1007, "tone", None, "ragebaiter_free")),
            self.bot.dispatched,
        )

    async def test_cog_load_rehydrates_existing_rows(self) -> None:
        db.execute(
            """
            INSERT INTO user_tags(user_id, tag_key, tag_value)
            VALUES (?, ?, ?)
            """,
            (1008, "age", "u25"),
        )
        active_until = datetime.now(UTC) + timedelta(days=1)
        db.execute(
            """
            INSERT INTO user_mod_tags(user_id, mod_tag, set_by, reason, expires_at)
            VALUES (?, ?, ?, ?, ?)
            """,
            (1008, "ragebaiter", 42, "rehydrate", active_until.isoformat()),
        )

        await self.service.cog_load()

        self.assertEqual(await self.service.get_user_tags(1008), {"age": "u25"})
        self.assertEqual(await self.service.get_mod_tags(1008), ["ragebaiter"])

    async def test_setup_registers_cog(self) -> None:
        class _SetupBot(_FakeBot):
            def __init__(self) -> None:
                super().__init__()
                self.added_cogs: list[object] = []

            async def add_cog(self, cog: object) -> None:
                self.added_cogs.append(cog)

        from cogs.tags import setup

        setup_bot = _SetupBot()
        await setup(setup_bot)  # type: ignore[arg-type]

        self.assertEqual(len(setup_bot.added_cogs), 1)
        self.assertIsInstance(setup_bot.added_cogs[0], TagService)

    async def test_cleanup_loop_task_can_start_and_stop(self) -> None:
        await self.service.cog_load()

        self.assertIsNotNone(self.service._cleanup_task)
        self.assertFalse(self.service._cleanup_task.done())

        await self.service.cog_unload()
        await asyncio.sleep(0)

        self.assertTrue(self.service._cleanup_task is None or self.service._cleanup_task.done())


if __name__ == "__main__":
    unittest.main(verbosity=2)
