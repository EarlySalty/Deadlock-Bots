from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from service import db
from service.tierlist_public import TierlistPublicServer


def _reset_test_db(path: Path) -> None:
    db.close_connection()
    db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
    db.DB_PATH = path  # type: ignore[assignment]
    os.environ["DEADLOCK_DB_PATH"] = str(path)
    db.connect()


def _seed_catalog() -> None:
    now = 1_714_000_000
    db.executemany(
        """
        INSERT INTO deadlock_heroes(hero_id, name, is_active, created_at, updated_at)
        VALUES (?, ?, 1, ?, ?)
        """,
        [
            (1, "Abrams", now, now),
            (2, "Haze", now, now),
        ],
    )
    db.executemany(
        """
        INSERT INTO deadlock_hero_builds(
            hero_id, build_id, build_name, author_name, is_active, sort_order, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, 1, ?, ?, ?)
        """,
        [
            (1, 1001, "Abrams Core", "CoachA", 10, now, now),
            (2, 1002, "Haze DPS", "CoachB", 20, now, now),
        ],
    )


class TierlistRefreshTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        super().setUp()
        self._tmpdir = tempfile.TemporaryDirectory()
        self._previous_db_path = os.environ.get("DEADLOCK_DB_PATH")
        _reset_test_db(Path(self._tmpdir.name) / "tierlist-refresh.sqlite3")
        _seed_catalog()

    def tearDown(self) -> None:
        db.close_connection()
        db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
        if self._previous_db_path is None:
            os.environ.pop("DEADLOCK_DB_PATH", None)
        else:
            os.environ["DEADLOCK_DB_PATH"] = self._previous_db_path
        self._tmpdir.cleanup()
        super().tearDown()

    async def test_refresh_inserts_snapshots_and_tier_payload_tracks_delta(self) -> None:
        bot = SimpleNamespace(get_cog=lambda name: None)
        server = TierlistPublicServer(bot)  # type: ignore[arg-type]

        responses = iter(
            [
                [{"pub_date": "2026-04-01T00:00:00Z"}],
                [
                    {"hero_id": 1, "matches": 600, "wins": 318, "losses": 282, "winrate": 53.0},
                    {"hero_id": 2, "matches": 450, "wins": 248, "losses": 202, "winrate": 55.11},
                    {"hero_id": 999, "matches": 900, "wins": 495, "losses": 405, "winrate": 55.0},
                ],
                [
                    {"hero_id": 1, "matches": 600, "wins": 318, "losses": 282, "winrate": 53.0},
                    {"hero_id": 2, "matches": 450, "wins": 248, "losses": 202, "winrate": 55.11},
                ],
                [
                    {"hero_id": 1, "matches": 600, "wins": 318, "losses": 282, "winrate": 53.0},
                    {"hero_id": 2, "matches": 450, "wins": 248, "losses": 202, "winrate": 55.11},
                ],
                [{"pub_date": "2026-04-01T00:00:00Z"}],
                [
                    {"hero_id": 1, "matches": 710, "wins": 387, "losses": 323, "winrate": 54.5},
                    {"hero_id": 2, "matches": 420, "wins": 235, "losses": 185, "winrate": 55.95},
                ],
                [
                    {"hero_id": 1, "matches": 710, "wins": 387, "losses": 323, "winrate": 54.5},
                    {"hero_id": 2, "matches": 420, "wins": 235, "losses": 185, "winrate": 55.95},
                ],
                [
                    {"hero_id": 1, "matches": 710, "wins": 387, "losses": 323, "winrate": 54.5},
                    {"hero_id": 2, "matches": 420, "wins": 235, "losses": 185, "winrate": 55.95},
                ],
            ]
        )

        async def fake_fetch_json(url: str, *, params=None):
            return next(responses)

        server._fetch_json = fake_fetch_json  # type: ignore[method-assign]

        first = await server.refresh_once()
        second = await server.refresh_once()

        self.assertTrue(first["ok"])
        self.assertTrue(second["ok"])
        self.assertEqual(len(first["snapshots"]), 3)
        self.assertEqual(len(second["snapshots"]), 3)

        snapshot_count = db.query_one("SELECT COUNT(*) AS c FROM tierlist_snapshots")
        snapshot_hero_count = db.query_one("SELECT COUNT(*) AS c FROM tierlist_snapshot_heroes")
        self.assertEqual(int(snapshot_count["c"]), 6)
        self.assertEqual(int(snapshot_hero_count["c"]), 12)

        payload = server.build_tierlist_payload("all")
        visible_heroes = [
            hero
            for tier in payload["tiers"]
            for hero in tier["heroes"]
        ]
        self.assertEqual([hero["hero_id"] for hero in visible_heroes], [1])
        self.assertEqual(visible_heroes[0]["tier"], "S+")
        self.assertEqual(visible_heroes[0]["wr"], 54.5)
        self.assertEqual(visible_heroes[0]["wr_change"], 1.5)

        history = server.build_tierlist_history_payload("all")
        self.assertEqual(len(history["snapshots"]), 2)
        self.assertEqual(history["snapshots"][0]["heroes"][0]["hero_id"], 1)


if __name__ == "__main__":
    unittest.main()
