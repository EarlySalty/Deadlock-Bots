from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from aiohttp.test_utils import AioHTTPTestCase

from service import db
from service.tierlist_public import TierlistPublicServer


def _reset_test_db(path: Path) -> None:
    db.close_connection()
    db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
    db.DB_PATH = path  # type: ignore[assignment]
    os.environ["DEADLOCK_DB_PATH"] = str(path)
    db.connect()


def _seed_endpoint_data() -> None:
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
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (1, 1001, "Abrams Core", "CoachA", 1, 10, now, now),
            (2, 1002, "Haze DPS", "CoachB", 1, 20, now, now),
        ],
    )
    db.execute(
        """
        INSERT INTO tierlist_build_votes(build_id, upvotes, downvotes, updated_at)
        VALUES (1001, 5, 1, ?)
        """,
        (now,),
    )
    db.execute(
        """
        INSERT INTO tierlist_hero_meta(hero_id, description, updated_at)
        VALUES (1, 'Frontline menace', ?)
        """,
        (now,),
    )
    db.execute(
        """
        INSERT INTO tierlist_streamers(
            hero_id, twitch_login, display_name, sort_order, is_active, created_at
        )
        VALUES (1, 'abramsmain', 'AbramsMain', 1, 1, ?)
        """,
        (now,),
    )
    for key, value in {
        "thresholds_json": '{"a_min":48.0,"b_min":46.0,"s_min":50.0,"s_plus_min":52.0}',
        "refresh_interval_seconds": "28800",
        "patch_override_unix": "",
        "description_text": "About tierlist",
        "min_matches": "500",
    }.items():
        db.execute(
            """
            INSERT INTO tierlist_settings(k, v, updated_at)
            VALUES (?, ?, ?)
            """,
            (key, value, now),
        )

    db.execute(
        """
        INSERT INTO tierlist_snapshots(bucket, patch_id, patch_unix, fetched_at)
        VALUES ('all', '2026-04-01T00:00:00Z', 1711929600, 1000)
        """
    )
    snapshot_one = db.query_one(
        """
        SELECT id FROM tierlist_snapshots
         WHERE bucket = 'all' AND fetched_at = 1000
        """
    )
    db.executemany(
        """
        INSERT INTO tierlist_snapshot_heroes(snapshot_id, hero_id, matches, wins, losses, winrate)
        VALUES (?, ?, ?, ?, ?, ?)
        """,
        [
            (int(snapshot_one["id"]), 1, 610, 311, 299, 51.0),
            (int(snapshot_one["id"]), 2, 430, 236, 194, 54.88),
        ],
    )

    db.execute(
        """
        INSERT INTO tierlist_snapshots(bucket, patch_id, patch_unix, fetched_at)
        VALUES ('all', '2026-04-01T00:00:00Z', 1711929600, 2000)
        """
    )
    snapshot_two = db.query_one(
        """
        SELECT id FROM tierlist_snapshots
         WHERE bucket = 'all' AND fetched_at = 2000
        """
    )
    db.executemany(
        """
        INSERT INTO tierlist_snapshot_heroes(snapshot_id, hero_id, matches, wins, losses, winrate)
        VALUES (?, ?, ?, ?, ?, ?)
        """,
        [
            (int(snapshot_two["id"]), 1, 650, 341, 309, 52.5),
            (int(snapshot_two["id"]), 2, 450, 247, 203, 54.89),
        ],
    )


class _FakeDashboardServer:
    def validate_discord_session(self, session_id: str):
        if session_id == "valid-session":
            return {"user_id": 1234, "username": "admin", "display_name": "Admin User"}
        return None


class _FakeBot:
    def __init__(self) -> None:
        self.dashboard = _FakeDashboardServer()

    def get_cog(self, name: str):
        if name == "DashboardCog":
            return SimpleNamespace(dashboard=self.dashboard)
        return None


class TierlistEndpointTests(AioHTTPTestCase):
    async def get_application(self):
        self._tmpdir = tempfile.TemporaryDirectory()
        self._previous_db_path = os.environ.get("DEADLOCK_DB_PATH")
        _reset_test_db(Path(self._tmpdir.name) / "tierlist-endpoints.sqlite3")
        _seed_endpoint_data()

        self.server = TierlistPublicServer(_FakeBot())  # type: ignore[arg-type]
        return self.server.app

    async def asyncTearDown(self) -> None:
        db.close_connection()
        db._DB_PATH_CACHED = None  # type: ignore[attr-defined]
        if getattr(self, "_previous_db_path", None) is None:
            os.environ.pop("DEADLOCK_DB_PATH", None)
        else:
            os.environ["DEADLOCK_DB_PATH"] = self._previous_db_path
        if hasattr(self, "_tmpdir"):
            self._tmpdir.cleanup()
        await super().asyncTearDown()

    async def test_public_endpoints_return_schema(self) -> None:
        heroes_response = await self.client.request("GET", "/api/heroes")
        self.assertEqual(heroes_response.status, 200)
        heroes = await heroes_response.json()
        self.assertEqual(heroes["1"]["slug"], "abrams")
        self.assertEqual(heroes["1"]["image_url"], "/heroes/abrams.png")

        tierlist_response = await self.client.request("GET", "/api/tierlist?bucket=all")
        self.assertEqual(tierlist_response.status, 200)
        tierlist = await tierlist_response.json()
        self.assertEqual(tierlist["bucket"], "all")
        self.assertEqual(tierlist["description"], "About tierlist")
        visible_heroes = [hero for tier in tierlist["tiers"] for hero in tier["heroes"]]
        self.assertEqual([hero["hero_id"] for hero in visible_heroes], [1])
        self.assertEqual(visible_heroes[0]["wr_change"], 1.5)
        self.assertEqual(visible_heroes[0]["builds"][0]["upvotes"], 5)
        self.assertEqual(visible_heroes[0]["streamers"][0]["twitch_login"], "abramsmain")

        history_response = await self.client.request("GET", "/api/tierlist/history?bucket=all")
        self.assertEqual(history_response.status, 200)
        history = await history_response.json()
        self.assertEqual(len(history["snapshots"]), 2)
        self.assertEqual(history["snapshots"][0]["heroes"][0]["tier"], "S+")

        vote_response = await self.client.request(
            "POST",
            "/api/builds/1001/vote",
            json={"vote": "up"},
        )
        self.assertEqual(vote_response.status, 200)
        vote = await vote_response.json()
        self.assertEqual(vote["upvotes"], 6)
        self.assertEqual(vote["downvotes"], 1)

    async def test_admin_endpoints_require_cookie(self) -> None:
        me_response = await self.client.request("GET", "/api/admin/me")
        self.assertEqual(me_response.status, 401)

        settings_response = await self.client.request("GET", "/api/admin/settings")
        self.assertEqual(settings_response.status, 401)

    async def test_admin_endpoints_accept_dashboard_cookie(self) -> None:
        cookies = {"master_dash_session": "valid-session"}

        me_response = await self.client.request("GET", "/api/admin/me", cookies=cookies)
        self.assertEqual(me_response.status, 200)
        me_payload = await me_response.json()
        self.assertEqual(me_payload["id"], 1234)
        self.assertEqual(me_payload["username"], "Admin User")

        settings_response = await self.client.request(
            "GET",
            "/api/admin/settings",
            cookies=cookies,
        )
        self.assertEqual(settings_response.status, 200)
        settings_payload = await settings_response.json()
        self.assertEqual(settings_payload["min_matches"], 500)
        self.assertEqual(settings_payload["thresholds"]["s_plus_min"], 52.0)

        async def fake_refresh_once():
            return {"ok": True, "snapshots": []}

        self.server.refresh_once = fake_refresh_once  # type: ignore[method-assign]
        refresh_response = await self.client.request(
            "POST",
            "/api/admin/refresh",
            cookies=cookies,
        )
        self.assertEqual(refresh_response.status, 200)
        refresh_payload = await refresh_response.json()
        self.assertTrue(refresh_payload["ok"])


if __name__ == "__main__":
    unittest.main()
