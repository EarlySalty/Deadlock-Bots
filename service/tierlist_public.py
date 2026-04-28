from __future__ import annotations

import asyncio
import json
import logging
import os
import time
from collections import defaultdict
from collections.abc import Iterable
from datetime import datetime, timezone
from typing import TYPE_CHECKING, Any

from aiohttp import ClientError, ClientSession, ClientTimeout, web

from service import db

if TYPE_CHECKING:
    from discord.ext.commands import Bot

log = logging.getLogger(__name__)

TIERLIST_PUBLIC_PORT = int(os.getenv("TIERLIST_PUBLIC_PORT", "8771"))
TIERLIST_PUBLIC_HOST = "127.0.0.1"

DEADLOCK_API_BASE = "https://api.deadlock-api.com/v1"
REFRESH_DEFAULT_SECONDS = 8 * 60 * 60
REFRESH_TIMEOUT_SECONDS = 30
VOTE_RATE_LIMIT_SECONDS = 5
SNAPSHOT_RETENTION_PER_BUCKET = 30
TIERLIST_SESSION_COOKIE = "master_dash_session"

BUCKETS: dict[str, tuple[int, int]] = {
    "all": (0, 116),
    "phantom_plus": (80, 116),
    "eternus": (100, 116),
}

DEFAULT_THRESHOLDS = {
    "s_plus_min": 52.0,
    "s_min": 50.0,
    "a_min": 48.0,
    "b_min": 46.0,
}

DEFAULT_SETTINGS = {
    "thresholds_json": json.dumps(DEFAULT_THRESHOLDS, separators=(",", ":"), sort_keys=True),
    "refresh_interval_seconds": str(REFRESH_DEFAULT_SECONDS),
    "patch_override_unix": "",
    "description_text": "",
    "min_matches": "500",
}

TIER_ORDER = (
    ("S+", "Overpowered"),
    ("S", "Meta-Defining"),
    ("A", "Strong Picks"),
    ("B", "Viable"),
    ("C", "Situational"),
)


class RetryableRefreshError(RuntimeError):
    """Refresh failure that should only skip the current tick."""


def _now_ts() -> int:
    return int(time.time())


def _error_payload(code: str, message: str) -> dict[str, str]:
    return {"error": code, "message": message}


def _slugify_hero(name: str) -> str:
    slug = str(name or "").strip().lower()
    slug = slug.replace("&", "and")
    slug = slug.replace(" ", "_")
    return slug


def _coerce_int(value: Any, default: int | None = None) -> int | None:
    if value in (None, "", False):
        return default
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def _coerce_float(value: Any, default: float | None = None) -> float | None:
    if value in (None, "", False):
        return default
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


def _coerce_bool(value: Any, default: bool = False) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, int):
        return value != 0
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized in {"1", "true", "yes", "on"}:
            return True
        if normalized in {"0", "false", "no", "off"}:
            return False
    return default


def _parse_unix_or_iso(value: Any) -> int | None:
    if value in (None, "", False):
        return None
    if isinstance(value, (int, float)):
        return int(value)
    text = str(value).strip()
    if not text:
        return None
    if text.isdigit():
        return int(text)
    try:
        if text.endswith("Z"):
            text = text[:-1] + "+00:00"
        parsed = datetime.fromisoformat(text)
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return int(parsed.timestamp())
    except ValueError:
        return None


def _normalize_thresholds(payload: Any) -> dict[str, float]:
    data = payload if isinstance(payload, dict) else {}
    normalized = {
        "s_plus_min": _coerce_float(
            data.get("s_plus_min", data.get("sPlusMin", data.get("S+"))),
            DEFAULT_THRESHOLDS["s_plus_min"],
        ),
        "s_min": _coerce_float(
            data.get("s_min", data.get("sMin", data.get("S"))),
            DEFAULT_THRESHOLDS["s_min"],
        ),
        "a_min": _coerce_float(
            data.get("a_min", data.get("aMin", data.get("A"))),
            DEFAULT_THRESHOLDS["a_min"],
        ),
        "b_min": _coerce_float(
            data.get("b_min", data.get("bMin", data.get("B"))),
            DEFAULT_THRESHOLDS["b_min"],
        ),
    }
    if None in normalized.values():
        raise ValueError("thresholds must contain numeric values")
    if not (
        normalized["s_plus_min"] >= normalized["s_min"] >= normalized["a_min"] >= normalized["b_min"]
    ):
        raise ValueError("thresholds must be descending (S+ >= S >= A >= B)")
    return {key: round(float(value), 2) for key, value in normalized.items()}


def _tier_for_winrate(winrate: float, thresholds: dict[str, float]) -> str:
    if winrate >= thresholds["s_plus_min"]:
        return "S+"
    if winrate >= thresholds["s_min"]:
        return "S"
    if winrate >= thresholds["a_min"]:
        return "A"
    if winrate >= thresholds["b_min"]:
        return "B"
    return "C"


def _tier_bounds(tier: str, thresholds: dict[str, float]) -> tuple[float | None, float | None]:
    if tier == "S+":
        return thresholds["s_plus_min"], None
    if tier == "S":
        return thresholds["s_min"], thresholds["s_plus_min"]
    if tier == "A":
        return thresholds["a_min"], thresholds["s_min"]
    if tier == "B":
        return thresholds["b_min"], thresholds["a_min"]
    return None, thresholds["b_min"]


class TierlistPublicServer:
    def __init__(
        self,
        bot: Bot,
        *,
        host: str = TIERLIST_PUBLIC_HOST,
        port: int = TIERLIST_PUBLIC_PORT,
    ) -> None:
        self.bot = bot
        self.host = host
        self.port = port
        self._runner: web.AppRunner | None = None
        self._site: web.TCPSite | None = None
        self._http_session: ClientSession | None = None
        self._refresh_task: asyncio.Task[None] | None = None
        self._refresh_lock = asyncio.Lock()
        self._vote_lock = asyncio.Lock()
        self._vote_rate_limits: dict[str, float] = {}

        self.app = web.Application(middlewares=[self._security_headers_mw])
        self.app.router.add_get("/api/heroes", self.handle_api_heroes)
        self.app.router.add_get("/api/tierlist", self.handle_api_tierlist)
        self.app.router.add_get("/api/tierlist/history", self.handle_api_tierlist_history)
        self.app.router.add_post("/api/builds/{build_id}/vote", self.handle_api_build_vote)
        self.app.router.add_get("/api/admin/me", self.handle_api_admin_me)
        self.app.router.add_get("/api/admin/hero/{hero_id}", self.handle_api_admin_hero_get)
        self.app.router.add_put("/api/admin/hero/{hero_id}", self.handle_api_admin_hero_put)
        self.app.router.add_get("/api/admin/settings", self.handle_api_admin_settings_get)
        self.app.router.add_put("/api/admin/settings", self.handle_api_admin_settings_put)
        self.app.router.add_post("/api/admin/refresh", self.handle_api_admin_refresh)

    @web.middleware
    async def _security_headers_mw(self, request: web.Request, handler):
        try:
            response = await handler(request)
        except web.HTTPException as exc:
            response = web.Response(
                status=exc.status,
                text=exc.text,
                headers=exc.headers,
            )
            if exc.content_type:
                response.content_type = exc.content_type
        except Exception:
            log.exception("Unhandled error in tierlist public server")
            response = web.json_response(
                _error_payload("internal_error", "Interner Serverfehler."),
                status=500,
            )

        if isinstance(response, web.StreamResponse):
            response.headers["Cache-Control"] = "no-store"
            response.headers["X-Content-Type-Options"] = "nosniff"
            response.headers["Referrer-Policy"] = "strict-origin-when-cross-origin"
            response.headers["X-Frame-Options"] = "DENY"
            response.headers["Permissions-Policy"] = "geolocation=(), microphone=(), camera=()"
        return response

    async def start(self) -> None:
        if self._runner is not None:
            return
        self._runner = web.AppRunner(self.app)
        await self._runner.setup()
        self._site = web.TCPSite(self._runner, self.host, self.port)
        await self._site.start()
        self._refresh_task = asyncio.create_task(self._refresh_loop())
        log.info("TierlistPublicServer started on %s:%s", self.host, self.port)

    async def stop(self) -> None:
        if self._refresh_task and not self._refresh_task.done():
            self._refresh_task.cancel()
            try:
                await self._refresh_task
            except asyncio.CancelledError:
                pass
        self._refresh_task = None

        if self._http_session and not self._http_session.closed:
            await self._http_session.close()
        self._http_session = None

        if self._runner is not None:
            await self._runner.cleanup()
        self._site = None
        self._runner = None

    async def _get_http_session(self) -> ClientSession:
        if self._http_session is None or self._http_session.closed:
            self._http_session = ClientSession(
                timeout=ClientTimeout(total=REFRESH_TIMEOUT_SECONDS)
            )
        return self._http_session

    async def _fetch_json(self, url: str, *, params: dict[str, Any] | None = None) -> Any:
        try:
            session = await self._get_http_session()
            async with session.get(url, params=params) as response:
                if response.status >= 500:
                    body = await response.text()
                    raise RetryableRefreshError(
                        f"upstream returned {response.status}: {body[:200]}"
                    )
                if response.status >= 400:
                    body = await response.text()
                    raise RuntimeError(f"upstream returned {response.status}: {body[:200]}")
                return await response.json()
        except RetryableRefreshError:
            raise
        except asyncio.TimeoutError as exc:
            raise RetryableRefreshError("upstream request timed out") from exc
        except ClientError as exc:
            raise RetryableRefreshError(f"upstream request failed: {exc}") from exc

    async def _refresh_loop(self) -> None:
        while True:
            try:
                await self.refresh_once()
            except RetryableRefreshError as exc:
                log.warning("Tierlist refresh skipped for this tick: %s", exc)
            except asyncio.CancelledError:
                raise
            except Exception:
                log.exception("Tierlist refresh failed")

            try:
                await asyncio.sleep(self._get_refresh_interval_seconds())
            except asyncio.CancelledError:
                raise

    def _ensure_default_settings(self) -> None:
        now = _now_ts()
        for key, value in DEFAULT_SETTINGS.items():
            db.execute(
                """
                INSERT INTO tierlist_settings(k, v, updated_at)
                VALUES (?, ?, ?)
                ON CONFLICT(k) DO NOTHING
                """,
                (key, value, now),
            )

    def _get_settings(self) -> dict[str, Any]:
        self._ensure_default_settings()
        rows = db.query_all("SELECT k, v FROM tierlist_settings")
        raw = {str(row["k"]): row["v"] for row in rows}

        try:
            thresholds_raw = json.loads(raw.get("thresholds_json") or DEFAULT_SETTINGS["thresholds_json"])
        except json.JSONDecodeError:
            thresholds_raw = DEFAULT_THRESHOLDS
        thresholds = _normalize_thresholds(thresholds_raw)

        refresh_interval = _coerce_int(
            raw.get("refresh_interval_seconds"),
            REFRESH_DEFAULT_SECONDS,
        )
        if refresh_interval is None or refresh_interval <= 0:
            refresh_interval = REFRESH_DEFAULT_SECONDS

        min_matches = _coerce_int(raw.get("min_matches"), 500)
        if min_matches is None or min_matches < 0:
            min_matches = 500

        patch_override_unix = _parse_unix_or_iso(raw.get("patch_override_unix"))

        return {
            "thresholds": thresholds,
            "refresh_interval_seconds": int(refresh_interval),
            "patch_override_unix": patch_override_unix,
            "description_text": str(raw.get("description_text") or ""),
            "min_matches": int(min_matches),
        }

    def _get_refresh_interval_seconds(self) -> int:
        return int(self._get_settings()["refresh_interval_seconds"])

    def _set_setting(self, key: str, value: str) -> None:
        db.execute(
            """
            INSERT INTO tierlist_settings(k, v, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(k) DO UPDATE SET
              v = excluded.v,
              updated_at = excluded.updated_at
            """,
            (key, value, _now_ts()),
        )

    def _normalize_bucket(self, request: web.Request) -> str:
        bucket = str(request.query.get("bucket") or "all").strip().lower()
        if bucket not in BUCKETS:
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_bucket", "Ungültiger Bucket."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )
        return bucket

    async def _read_json_object(self, request: web.Request) -> dict[str, Any]:
        try:
            payload = await request.json()
        except Exception as exc:
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_json", "Ungültiger JSON-Body."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            ) from exc
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_json", "JSON-Objekt erwartet."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )
        return payload

    def _dashboard_server(self) -> Any | None:
        get_cog = getattr(self.bot, "get_cog", None)
        if callable(get_cog):
            cog = get_cog("DashboardCog")
            dashboard = getattr(cog, "dashboard", None) if cog else None
            if dashboard is not None:
                return dashboard
        dashboard = getattr(self.bot, "dashboard", None)
        if dashboard is not None:
            return dashboard
        return None

    def _get_admin_session(self, request: web.Request) -> dict[str, Any] | None:
        session_id = str(request.cookies.get(TIERLIST_SESSION_COOKIE) or "").strip()
        if not session_id:
            return None
        dashboard = self._dashboard_server()
        if dashboard is None:
            return None
        validator = getattr(dashboard, "validate_discord_session", None)
        if callable(validator):
            session = validator(session_id)
            return session if isinstance(session, dict) else None

        sessions = getattr(dashboard, "_discord_sessions", None)
        if not isinstance(sessions, dict):
            return None
        session = sessions.get(session_id)
        if not isinstance(session, dict):
            return None
        expires_at = _coerce_float(session.get("expires_at"), 0.0) or 0.0
        if expires_at <= time.time():
            sessions.pop(session_id, None)
            return None
        return session

    def _require_admin_session(self, request: web.Request) -> dict[str, Any]:
        session = self._get_admin_session(request)
        if session is None:
            raise web.HTTPUnauthorized(
                text=json.dumps(
                    _error_payload("unauthorized", "Anmeldung erforderlich."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )
        return session

    async def _fetch_current_patch(self) -> dict[str, Any]:
        payload = await self._fetch_json(f"{DEADLOCK_API_BASE}/patches")
        if isinstance(payload, dict):
            candidates = payload.get("patches")
        else:
            candidates = payload
        if not isinstance(candidates, list):
            raise RuntimeError("unexpected patch payload")

        latest_item: dict[str, Any] | None = None
        latest_unix = -1
        for item in candidates:
            if not isinstance(item, dict):
                continue
            patch_unix = _parse_unix_or_iso(item.get("pub_date", item.get("pubDate")))
            if patch_unix is None:
                continue
            if patch_unix > latest_unix:
                latest_unix = patch_unix
                latest_item = item
        if latest_item is None or latest_unix < 0:
            raise RuntimeError("could not determine current patch")

        patch_id = str(
            latest_item.get("pub_date")
            or latest_item.get("pubDate")
            or latest_item.get("patch_id")
            or latest_item.get("id")
            or latest_unix
        )
        return {
            "patch_id": patch_id,
            "patch_unix": latest_unix,
        }

    def _normalize_hero_stats_payload(self, payload: Any) -> list[dict[str, Any]]:
        if isinstance(payload, list):
            raw_rows = payload
        elif isinstance(payload, dict):
            raw_rows = (
                payload.get("heroes")
                or payload.get("hero_stats")
                or payload.get("heroStats")
                or payload.get("results")
                or payload.get("data")
                or payload.get("stats")
                or []
            )
        else:
            raw_rows = []

        stats_by_hero: dict[int, dict[str, Any]] = {}
        for item in raw_rows:
            if not isinstance(item, dict):
                continue

            hero_id = _coerce_int(item.get("hero_id", item.get("heroId", item.get("id"))))
            if hero_id is None:
                continue

            matches = _coerce_int(
                item.get(
                    "matches",
                    item.get("matches_played", item.get("match_count", item.get("games_played"))),
                )
            )
            wins = _coerce_int(item.get("wins", item.get("win_count")))
            losses = _coerce_int(item.get("losses", item.get("loss_count")))
            winrate = _coerce_float(item.get("winrate", item.get("win_rate", item.get("wr"))))

            if matches is None and wins is not None and losses is not None:
                matches = wins + losses
            if losses is None and wins is not None and matches is not None:
                losses = max(matches - wins, 0)
            if wins is None and losses is not None and matches is not None:
                wins = max(matches - losses, 0)

            if winrate is None and matches and wins is not None:
                winrate = (wins / matches) * 100.0
            elif winrate is not None and winrate <= 1.0:
                winrate = winrate * 100.0

            if matches is None or wins is None or losses is None or winrate is None:
                continue
            if matches < 0 or wins < 0 or losses < 0:
                continue

            normalized = {
                "hero_id": int(hero_id),
                "matches": int(matches),
                "wins": int(wins),
                "losses": int(losses),
                "winrate": round(float(winrate), 4),
            }
            stats_by_hero[int(hero_id)] = normalized

        return list(stats_by_hero.values())

    async def _fetch_bucket_stats(self, bucket: str, *, patch_unix: int) -> list[dict[str, Any]]:
        min_badge, max_badge = BUCKETS[bucket]
        payload = await self._fetch_json(
            f"{DEADLOCK_API_BASE}/analytics/hero-stats",
            params={
                "min_average_badge": min_badge,
                "max_average_badge": max_badge,
                "min_unix_timestamp": patch_unix,
                "game_mode": "normal",
            },
        )
        return self._normalize_hero_stats_payload(payload)

    def _known_hero_ids(self) -> set[int]:
        rows = db.query_all("SELECT hero_id FROM deadlock_heroes WHERE is_active = 1")
        return {int(row["hero_id"]) for row in rows}

    def _allocate_fetched_at(self, conn: Any, bucket: str, preferred_ts: int) -> int:
        fetched_at = int(preferred_ts)
        while True:
            exists = conn.execute(
                "SELECT 1 FROM tierlist_snapshots WHERE bucket = ? AND fetched_at = ?",
                (bucket, fetched_at),
            ).fetchone()
            if not exists:
                return fetched_at
            fetched_at += 1

    def _insert_snapshot_bundle(
        self,
        conn: Any,
        *,
        bucket: str,
        patch_id: str,
        patch_unix: int,
        fetched_at: int,
        stats: Iterable[dict[str, Any]],
    ) -> dict[str, Any]:
        snapshot_ts = self._allocate_fetched_at(conn, bucket, fetched_at)
        cursor = conn.execute(
            """
            INSERT INTO tierlist_snapshots(bucket, patch_id, patch_unix, fetched_at)
            VALUES (?, ?, ?, ?)
            """,
            (bucket, patch_id, patch_unix, snapshot_ts),
        )
        snapshot_id = int(cursor.lastrowid)

        rows = [
            (
                snapshot_id,
                int(item["hero_id"]),
                int(item["matches"]),
                int(item["wins"]),
                int(item["losses"]),
                float(item["winrate"]),
            )
            for item in stats
        ]
        if rows:
            conn.executemany(
                """
                INSERT INTO tierlist_snapshot_heroes(
                    snapshot_id, hero_id, matches, wins, losses, winrate
                )
                VALUES (?, ?, ?, ?, ?, ?)
                """,
                rows,
            )

        self._prune_snapshots(conn, bucket)
        return {
            "snapshot_id": snapshot_id,
            "bucket": bucket,
            "fetched_at": snapshot_ts,
            "heroes": len(rows),
        }

    def _prune_snapshots(self, conn: Any, bucket: str) -> None:
        stale_rows = conn.execute(
            """
            SELECT id
              FROM tierlist_snapshots
             WHERE bucket = ?
             ORDER BY fetched_at DESC, id DESC
             LIMIT -1 OFFSET ?
            """,
            (bucket, SNAPSHOT_RETENTION_PER_BUCKET),
        ).fetchall()
        stale_ids = [int(row["id"]) for row in stale_rows]
        if not stale_ids:
            return
        placeholders = ", ".join("?" for _ in stale_ids)
        conn.execute(
            f"DELETE FROM tierlist_snapshots WHERE id IN ({placeholders})",
            tuple(stale_ids),
        )

    async def refresh_once(self) -> dict[str, Any]:
        async with self._refresh_lock:
            settings = self._get_settings()
            patch = await self._fetch_current_patch()
            patch_unix = int(settings["patch_override_unix"] or patch["patch_unix"])
            patch_id = str(patch["patch_id"])
            fetched_at = _now_ts()
            known_hero_ids = self._known_hero_ids()

            bucket_results: dict[str, list[dict[str, Any]]] = {}
            for bucket in BUCKETS:
                stats = await self._fetch_bucket_stats(bucket, patch_unix=patch_unix)
                bucket_results[bucket] = [
                    item for item in stats if int(item["hero_id"]) in known_hero_ids
                ]

            inserted: list[dict[str, Any]] = []
            async with db.transaction() as conn:
                for bucket, stats in bucket_results.items():
                    inserted.append(
                        self._insert_snapshot_bundle(
                            conn,
                            bucket=bucket,
                            patch_id=patch_id,
                            patch_unix=patch_unix,
                            fetched_at=fetched_at,
                            stats=stats,
                        )
                    )

            summary = {
                "ok": True,
                "patch_id": patch_id,
                "patch_unix": patch_unix,
                "fetched_at": fetched_at,
                "snapshots": inserted,
            }
            log.info(
                "Tierlist refresh completed: patch_id=%s patch_unix=%s buckets=%s",
                patch_id,
                patch_unix,
                len(inserted),
            )
            return summary

    def _hero_catalog(self) -> dict[int, dict[str, Any]]:
        rows = db.query_all(
            """
            SELECT hero_id, name
              FROM deadlock_heroes
             WHERE is_active = 1
             ORDER BY name COLLATE NOCASE ASC
            """
        )
        heroes: dict[int, dict[str, Any]] = {}
        for row in rows:
            hero_id = int(row["hero_id"])
            name = str(row["name"])
            slug = _slugify_hero(name)
            heroes[hero_id] = {
                "hero_id": hero_id,
                "name": name,
                "slug": slug,
                "image_url": f"/heroes/{slug}.png",
            }
        return heroes

    def _load_builds_by_hero(self) -> dict[int, list[dict[str, Any]]]:
        rows = db.query_all(
            """
            SELECT
              hb.hero_id,
              hb.build_id,
              hb.build_name,
              hb.author_name,
              hb.sort_order,
              COALESCE(v.upvotes, 0) AS upvotes,
              COALESCE(v.downvotes, 0) AS downvotes
            FROM deadlock_hero_builds hb
            LEFT JOIN tierlist_build_votes v ON v.build_id = hb.build_id
            WHERE hb.is_active = 1
            ORDER BY hb.hero_id ASC, hb.sort_order ASC, hb.build_id ASC
            """
        )
        builds_by_hero: dict[int, list[dict[str, Any]]] = defaultdict(list)
        for row in rows:
            hero_id = int(row["hero_id"])
            builds_by_hero[hero_id].append(
                {
                    "build_id": int(row["build_id"]),
                    "build_name": str(row["build_name"]),
                    "author_name": str(row["author_name"]),
                    "sort_order": int(row["sort_order"] or 100),
                    "upvotes": int(row["upvotes"] or 0),
                    "downvotes": int(row["downvotes"] or 0),
                }
            )

        for hero_id, items in builds_by_hero.items():
            items.sort(
                key=lambda item: (
                    int(item["sort_order"]),
                    -(int(item["upvotes"]) - int(item["downvotes"])),
                    -int(item["upvotes"]),
                    int(item["build_id"]),
                )
            )
        return dict(builds_by_hero)

    def _load_streamers_by_hero(self) -> dict[int, list[dict[str, Any]]]:
        rows = db.query_all(
            """
            SELECT id, hero_id, twitch_login, display_name, sort_order
              FROM tierlist_streamers
             WHERE is_active = 1
             ORDER BY hero_id ASC, sort_order ASC, id ASC
            """
        )
        streamers_by_hero: dict[int, list[dict[str, Any]]] = defaultdict(list)
        for row in rows:
            hero_id = int(row["hero_id"])
            streamers_by_hero[hero_id].append(
                {
                    "id": int(row["id"]),
                    "twitch_login": str(row["twitch_login"]),
                    "display_name": str(row["display_name"]),
                    "sort_order": int(row["sort_order"] or 100),
                }
            )
        return dict(streamers_by_hero)

    def _load_hero_descriptions(self) -> dict[int, str]:
        rows = db.query_all("SELECT hero_id, description FROM tierlist_hero_meta")
        return {int(row["hero_id"]): str(row["description"] or "") for row in rows}

    def _latest_snapshot(self, bucket: str) -> dict[str, Any] | None:
        row = db.query_one(
            """
            SELECT id, bucket, patch_id, patch_unix, fetched_at
              FROM tierlist_snapshots
             WHERE bucket = ?
             ORDER BY fetched_at DESC, id DESC
             LIMIT 1
            """,
            (bucket,),
        )
        return dict(row) if row else None

    def _previous_snapshot(self, bucket: str, snapshot_id: int) -> dict[int, float]:
        row = db.query_one(
            """
            SELECT id
              FROM tierlist_snapshots
             WHERE bucket = ? AND id != ?
             ORDER BY fetched_at DESC, id DESC
             LIMIT 1
            """,
            (bucket, snapshot_id),
        )
        if not row:
            return {}
        snapshot_rows = db.query_all(
            """
            SELECT hero_id, winrate
              FROM tierlist_snapshot_heroes
             WHERE snapshot_id = ?
            """,
            (int(row["id"]),),
        )
        return {
            int(item["hero_id"]): round(float(item["winrate"]), 2)
            for item in snapshot_rows
        }

    def _snapshot_rows(self, snapshot_id: int) -> list[dict[str, Any]]:
        rows = db.query_all(
            """
            SELECT hero_id, matches, wins, losses, winrate
              FROM tierlist_snapshot_heroes
             WHERE snapshot_id = ?
            """,
            (snapshot_id,),
        )
        return [dict(row) for row in rows]

    def build_tierlist_payload(self, bucket: str) -> dict[str, Any]:
        settings = self._get_settings()
        thresholds = settings["thresholds"]
        min_matches = int(settings["min_matches"])
        latest = self._latest_snapshot(bucket)
        heroes = self._hero_catalog()
        descriptions = self._load_hero_descriptions()
        builds_by_hero = self._load_builds_by_hero()
        streamers_by_hero = self._load_streamers_by_hero()

        tier_groups: dict[str, list[dict[str, Any]]] = {tier: [] for tier, _ in TIER_ORDER}
        if latest is None:
            return {
                "bucket": bucket,
                "patch_id": None,
                "patch_unix": None,
                "last_updated": None,
                "description": settings["description_text"],
                "thresholds": thresholds,
                "min_matches": min_matches,
                "tiers": [
                    {
                        "key": tier,
                        "title": title,
                        "min_wr": _tier_bounds(tier, thresholds)[0],
                        "max_wr": _tier_bounds(tier, thresholds)[1],
                        "heroes": [],
                    }
                    for tier, title in TIER_ORDER
                ],
            }

        previous_wr = self._previous_snapshot(bucket, int(latest["id"]))
        for row in self._snapshot_rows(int(latest["id"])):
            hero_id = int(row["hero_id"])
            hero = heroes.get(hero_id)
            if hero is None:
                continue
            matches = int(row["matches"])
            if matches < min_matches:
                continue
            winrate = round(float(row["winrate"]), 2)
            tier = _tier_for_winrate(winrate, thresholds)
            prior = previous_wr.get(hero_id)
            tier_groups[tier].append(
                {
                    **hero,
                    "wr": winrate,
                    "wr_change": None if prior is None else round(winrate - prior, 2),
                    "matches": matches,
                    "tier": tier,
                    "description": descriptions.get(hero_id, ""),
                    "builds": builds_by_hero.get(hero_id, []),
                    "streamers": streamers_by_hero.get(hero_id, []),
                }
            )

        for heroes_in_tier in tier_groups.values():
            heroes_in_tier.sort(key=lambda item: (-float(item["wr"]), -int(item["matches"]), item["name"]))

        return {
            "bucket": bucket,
            "patch_id": str(latest["patch_id"]),
            "patch_unix": int(latest["patch_unix"]),
            "last_updated": int(latest["fetched_at"]),
            "description": settings["description_text"],
            "thresholds": thresholds,
            "min_matches": min_matches,
            "tiers": [
                {
                    "key": tier,
                    "title": title,
                    "min_wr": _tier_bounds(tier, thresholds)[0],
                    "max_wr": _tier_bounds(tier, thresholds)[1],
                    "heroes": tier_groups[tier],
                }
                for tier, title in TIER_ORDER
            ],
        }

    def build_tierlist_history_payload(self, bucket: str) -> dict[str, Any]:
        settings = self._get_settings()
        thresholds = settings["thresholds"]
        min_matches = int(settings["min_matches"])
        hero_ids = self._hero_catalog().keys()
        known_hero_ids = set(hero_ids)

        snapshot_rows = db.query_all(
            """
            SELECT id, patch_id, fetched_at
              FROM tierlist_snapshots
             WHERE bucket = ?
             ORDER BY fetched_at DESC, id DESC
             LIMIT ?
            """,
            (bucket, SNAPSHOT_RETENTION_PER_BUCKET),
        )

        snapshots: list[dict[str, Any]] = []
        for snapshot in snapshot_rows:
            hero_rows = db.query_all(
                """
                SELECT hero_id, matches, winrate
                  FROM tierlist_snapshot_heroes
                 WHERE snapshot_id = ?
                """,
                (int(snapshot["id"]),),
            )
            heroes = []
            for row in hero_rows:
                hero_id = int(row["hero_id"])
                if hero_id not in known_hero_ids:
                    continue
                matches = int(row["matches"])
                if matches < min_matches:
                    continue
                winrate = round(float(row["winrate"]), 2)
                heroes.append(
                    {
                        "hero_id": hero_id,
                        "wr": winrate,
                        "tier": _tier_for_winrate(winrate, thresholds),
                    }
                )
            heroes.sort(key=lambda item: (-float(item["wr"]), int(item["hero_id"])))
            snapshots.append(
                {
                    "snapshot_id": int(snapshot["id"]),
                    "fetched_at": int(snapshot["fetched_at"]),
                    "patch_id": str(snapshot["patch_id"]),
                    "heroes": heroes,
                }
            )

        return {
            "bucket": bucket,
            "snapshots": snapshots,
        }

    def _load_admin_hero_payload(self, hero_id: int) -> dict[str, Any] | None:
        hero = db.query_one(
            "SELECT hero_id, name FROM deadlock_heroes WHERE hero_id = ?",
            (hero_id,),
        )
        if hero is None:
            return None
        meta = db.query_one(
            "SELECT description FROM tierlist_hero_meta WHERE hero_id = ?",
            (hero_id,),
        )
        builds = db.query_all(
            """
            SELECT build_id, build_name, author_name, is_active, sort_order
              FROM deadlock_hero_builds
             WHERE hero_id = ?
             ORDER BY sort_order ASC, build_id ASC
            """,
            (hero_id,),
        )
        streamers = db.query_all(
            """
            SELECT id, twitch_login, display_name, sort_order, is_active
              FROM tierlist_streamers
             WHERE hero_id = ?
             ORDER BY sort_order ASC, id ASC
            """,
            (hero_id,),
        )
        return {
            "hero_id": int(hero["hero_id"]),
            "name": str(hero["name"]),
            "description": str(meta["description"]) if meta else "",
            "builds_meta": [
                {
                    "build_id": int(row["build_id"]),
                    "build_name": str(row["build_name"]),
                    "author_name": str(row["author_name"]),
                    "is_active": bool(int(row["is_active"])),
                    "sort_order": int(row["sort_order"] or 100),
                }
                for row in builds
            ],
            "streamers": [
                {
                    "id": int(row["id"]),
                    "twitch_login": str(row["twitch_login"]),
                    "display_name": str(row["display_name"]),
                    "sort_order": int(row["sort_order"] or 100),
                    "is_active": bool(int(row["is_active"])),
                }
                for row in streamers
            ],
        }

    async def handle_api_heroes(self, request: web.Request) -> web.Response:
        heroes = self._hero_catalog()
        payload = {
            str(hero_id): {
                "name": item["name"],
                "slug": item["slug"],
                "image_url": item["image_url"],
            }
            for hero_id, item in heroes.items()
        }
        return web.json_response(payload)

    async def handle_api_tierlist(self, request: web.Request) -> web.Response:
        bucket = self._normalize_bucket(request)
        return web.json_response(self.build_tierlist_payload(bucket))

    async def handle_api_tierlist_history(self, request: web.Request) -> web.Response:
        bucket = self._normalize_bucket(request)
        return web.json_response(self.build_tierlist_history_payload(bucket))

    async def handle_api_build_vote(self, request: web.Request) -> web.Response:
        try:
            build_id = int(request.match_info["build_id"])
        except Exception as exc:
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_build_id", "Ungültige Build-ID."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            ) from exc

        payload = await self._read_json_object(request)
        vote = str(payload.get("vote") or "").strip().lower()
        if vote not in {"up", "down"}:
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_vote", "vote muss 'up' oder 'down' sein."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )

        build_exists = db.query_one(
            "SELECT 1 FROM deadlock_hero_builds WHERE build_id = ? LIMIT 1",
            (build_id,),
        )
        if build_exists is None:
            raise web.HTTPNotFound(
                text=json.dumps(
                    _error_payload("build_not_found", "Build nicht gefunden."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )

        forwarded_for = str(request.headers.get("X-Forwarded-For") or "").split(",", 1)[0].strip()
        peer = forwarded_for or str(request.remote or "unknown")

        async with self._vote_lock:
            now = time.monotonic()
            last_vote = self._vote_rate_limits.get(peer)
            if last_vote is not None and now - last_vote < VOTE_RATE_LIMIT_SECONDS:
                raise web.HTTPTooManyRequests(
                    text=json.dumps(
                        _error_payload(
                            "rate_limited",
                            "Bitte warte kurz, bevor du erneut abstimmst.",
                        ),
                        ensure_ascii=False,
                    ),
                    content_type="application/json",
                )
            self._vote_rate_limits = {
                ip: ts for ip, ts in self._vote_rate_limits.items() if now - ts < VOTE_RATE_LIMIT_SECONDS
            }
            self._vote_rate_limits[peer] = now

        async with db.transaction() as conn:
            conn.execute(
                """
                INSERT INTO tierlist_build_votes(build_id, upvotes, downvotes, updated_at)
                VALUES (?, 0, 0, ?)
                ON CONFLICT(build_id) DO NOTHING
                """,
                (build_id, _now_ts()),
            )
            if vote == "up":
                conn.execute(
                    """
                    UPDATE tierlist_build_votes
                       SET upvotes = upvotes + 1,
                           updated_at = ?
                     WHERE build_id = ?
                    """,
                    (_now_ts(), build_id),
                )
            else:
                conn.execute(
                    """
                    UPDATE tierlist_build_votes
                       SET downvotes = downvotes + 1,
                           updated_at = ?
                     WHERE build_id = ?
                    """,
                    (_now_ts(), build_id),
                )
            row = conn.execute(
                """
                SELECT upvotes, downvotes
                  FROM tierlist_build_votes
                 WHERE build_id = ?
                """,
                (build_id,),
            ).fetchone()

        return web.json_response(
            {
                "ok": True,
                "build_id": build_id,
                "upvotes": int(row["upvotes"] if row else 0),
                "downvotes": int(row["downvotes"] if row else 0),
            }
        )

    async def handle_api_admin_me(self, request: web.Request) -> web.Response:
        session = self._require_admin_session(request)
        return web.json_response(
            {
                "id": int(session.get("user_id") or 0),
                "username": str(session.get("display_name") or session.get("username") or ""),
            }
        )

    async def handle_api_admin_hero_get(self, request: web.Request) -> web.Response:
        self._require_admin_session(request)
        try:
            hero_id = int(request.match_info["hero_id"])
        except Exception as exc:
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_hero_id", "Ungültige Hero-ID."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            ) from exc

        payload = self._load_admin_hero_payload(hero_id)
        if payload is None:
            raise web.HTTPNotFound(
                text=json.dumps(
                    _error_payload("hero_not_found", "Hero nicht gefunden."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )
        return web.json_response(payload)

    async def handle_api_admin_hero_put(self, request: web.Request) -> web.Response:
        self._require_admin_session(request)
        try:
            hero_id = int(request.match_info["hero_id"])
        except Exception as exc:
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_hero_id", "Ungültige Hero-ID."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            ) from exc

        payload = await self._read_json_object(request)
        hero_exists = db.query_one(
            "SELECT 1 FROM deadlock_heroes WHERE hero_id = ? LIMIT 1",
            (hero_id,),
        )
        if hero_exists is None:
            raise web.HTTPNotFound(
                text=json.dumps(
                    _error_payload("hero_not_found", "Hero nicht gefunden."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )

        has_description = "description" in payload
        has_streamers = "streamers" in payload
        has_builds_meta = "builds_meta" in payload or "buildsMeta" in payload

        description = str(payload.get("description") or "")
        raw_streamers = payload.get("streamers", [])
        raw_builds_meta = payload.get("builds_meta", payload.get("buildsMeta", []))

        if has_streamers and not isinstance(raw_streamers, list):
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_streamers", "streamers muss ein Array sein."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )
        if has_builds_meta and not isinstance(raw_builds_meta, list):
            raise web.HTTPBadRequest(
                text=json.dumps(
                    _error_payload("invalid_builds_meta", "builds_meta muss ein Array sein."),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            )

        parsed_streamers: list[dict[str, Any]] = []
        if has_streamers:
            for index, item in enumerate(raw_streamers, start=1):
                if not isinstance(item, dict):
                    raise web.HTTPBadRequest(
                        text=json.dumps(
                            _error_payload(
                                "invalid_streamers",
                                f"streamers[{index}] muss ein Objekt sein.",
                            ),
                            ensure_ascii=False,
                        ),
                        content_type="application/json",
                    )
                login = str(item.get("twitch_login", item.get("twitchLogin")) or "").strip().lower()
                display_name = str(item.get("display_name", item.get("displayName")) or login).strip()
                if not login:
                    raise web.HTTPBadRequest(
                        text=json.dumps(
                            _error_payload(
                                "invalid_streamers",
                                f"streamers[{index}].twitch_login fehlt.",
                            ),
                            ensure_ascii=False,
                        ),
                        content_type="application/json",
                    )
                sort_order = _coerce_int(item.get("sort_order", item.get("sortOrder", 100)), 100) or 100
                is_active = _coerce_bool(item.get("is_active", item.get("isActive", True)), True)
                parsed_streamers.append(
                    {
                        "twitch_login": login,
                        "display_name": display_name or login,
                        "sort_order": int(sort_order),
                        "is_active": is_active,
                    }
                )

        parsed_builds_meta: list[dict[str, Any]] = []
        if has_builds_meta:
            existing_builds = {
                int(row["build_id"])
                for row in db.query_all(
                    "SELECT build_id FROM deadlock_hero_builds WHERE hero_id = ?",
                    (hero_id,),
                )
            }
            for index, item in enumerate(raw_builds_meta, start=1):
                if not isinstance(item, dict):
                    raise web.HTTPBadRequest(
                        text=json.dumps(
                            _error_payload(
                                "invalid_builds_meta",
                                f"builds_meta[{index}] muss ein Objekt sein.",
                            ),
                            ensure_ascii=False,
                        ),
                        content_type="application/json",
                    )
                build_id = _coerce_int(item.get("build_id", item.get("buildId")))
                if build_id is None or build_id not in existing_builds:
                    raise web.HTTPBadRequest(
                        text=json.dumps(
                            _error_payload(
                                "invalid_builds_meta",
                                f"builds_meta[{index}].build_id ist unbekannt.",
                            ),
                            ensure_ascii=False,
                        ),
                        content_type="application/json",
                    )
                sort_order = _coerce_int(item.get("sort_order", item.get("sortOrder", 100)), 100) or 100
                is_active = _coerce_bool(item.get("is_active", item.get("isActive", True)), True)
                parsed_builds_meta.append(
                    {
                        "build_id": int(build_id),
                        "sort_order": int(sort_order),
                        "is_active": is_active,
                    }
                )

        ts = _now_ts()
        async with db.transaction() as conn:
            if has_description:
                conn.execute(
                    """
                    INSERT INTO tierlist_hero_meta(hero_id, description, updated_at)
                    VALUES (?, ?, ?)
                    ON CONFLICT(hero_id) DO UPDATE SET
                      description = excluded.description,
                      updated_at = excluded.updated_at
                    """,
                    (hero_id, description, ts),
                )

            if has_streamers:
                conn.execute("DELETE FROM tierlist_streamers WHERE hero_id = ?", (hero_id,))
                if parsed_streamers:
                    conn.executemany(
                        """
                        INSERT INTO tierlist_streamers(
                            hero_id, twitch_login, display_name, sort_order, is_active, created_at
                        )
                        VALUES (?, ?, ?, ?, ?, ?)
                        """,
                        [
                            (
                                hero_id,
                                item["twitch_login"],
                                item["display_name"],
                                item["sort_order"],
                                1 if item["is_active"] else 0,
                                ts,
                            )
                            for item in parsed_streamers
                        ],
                    )

            if has_builds_meta:
                conn.executemany(
                    """
                    UPDATE deadlock_hero_builds
                       SET sort_order = ?, is_active = ?, updated_at = ?
                     WHERE hero_id = ? AND build_id = ?
                    """,
                    [
                        (
                            item["sort_order"],
                            1 if item["is_active"] else 0,
                            ts,
                            hero_id,
                            item["build_id"],
                        )
                        for item in parsed_builds_meta
                    ],
                )

        hero_payload = self._load_admin_hero_payload(hero_id)
        return web.json_response({"ok": True, "hero": hero_payload})

    async def handle_api_admin_settings_get(self, request: web.Request) -> web.Response:
        self._require_admin_session(request)
        return web.json_response(self._get_settings())

    async def handle_api_admin_settings_put(self, request: web.Request) -> web.Response:
        self._require_admin_session(request)
        payload = await self._read_json_object(request)
        current = self._get_settings()

        if "thresholds" in payload:
            try:
                thresholds = _normalize_thresholds(payload["thresholds"])
            except ValueError as exc:
                raise web.HTTPBadRequest(
                    text=json.dumps(
                        _error_payload("invalid_thresholds", str(exc)),
                        ensure_ascii=False,
                    ),
                    content_type="application/json",
                ) from exc
            self._set_setting(
                "thresholds_json",
                json.dumps(thresholds, separators=(",", ":"), sort_keys=True),
            )
            current["thresholds"] = thresholds

        if "refresh_interval_seconds" in payload or "refreshIntervalSeconds" in payload:
            refresh_interval = _coerce_int(
                payload.get("refresh_interval_seconds", payload.get("refreshIntervalSeconds"))
            )
            if refresh_interval is None or refresh_interval <= 0:
                raise web.HTTPBadRequest(
                    text=json.dumps(
                        _error_payload(
                            "invalid_refresh_interval",
                            "refresh_interval_seconds muss > 0 sein.",
                        ),
                        ensure_ascii=False,
                    ),
                    content_type="application/json",
                )
            self._set_setting("refresh_interval_seconds", str(refresh_interval))
            current["refresh_interval_seconds"] = refresh_interval

        if "min_matches" in payload or "minMatches" in payload:
            min_matches = _coerce_int(payload.get("min_matches", payload.get("minMatches")))
            if min_matches is None or min_matches < 0:
                raise web.HTTPBadRequest(
                    text=json.dumps(
                        _error_payload("invalid_min_matches", "min_matches muss >= 0 sein."),
                        ensure_ascii=False,
                    ),
                    content_type="application/json",
                )
            self._set_setting("min_matches", str(min_matches))
            current["min_matches"] = min_matches

        if "patch_override_unix" in payload or "patchOverrideUnix" in payload:
            raw_override = payload.get("patch_override_unix", payload.get("patchOverrideUnix"))
            override = _parse_unix_or_iso(raw_override)
            if raw_override not in (None, "", False) and override is None:
                raise web.HTTPBadRequest(
                    text=json.dumps(
                        _error_payload(
                            "invalid_patch_override",
                            "patch_override_unix muss ein Unix-Timestamp, ISO-Datum oder null sein.",
                        ),
                        ensure_ascii=False,
                    ),
                    content_type="application/json",
                )
            self._set_setting("patch_override_unix", "" if override is None else str(override))
            current["patch_override_unix"] = override

        if "description_text" in payload or "descriptionText" in payload:
            description_text = str(
                payload.get("description_text", payload.get("descriptionText")) or ""
            )
            self._set_setting("description_text", description_text)
            current["description_text"] = description_text

        return web.json_response(current)

    async def handle_api_admin_refresh(self, request: web.Request) -> web.Response:
        self._require_admin_session(request)
        try:
            result = await self.refresh_once()
        except RetryableRefreshError as exc:
            raise web.HTTPBadGateway(
                text=json.dumps(
                    _error_payload("upstream_unavailable", str(exc)),
                    ensure_ascii=False,
                ),
                content_type="application/json",
            ) from exc
        return web.json_response(result)
