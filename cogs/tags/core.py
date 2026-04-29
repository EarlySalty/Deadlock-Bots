from __future__ import annotations

import asyncio
import logging
from datetime import UTC, datetime
from typing import Any

from discord.ext import commands

from service import db

from .constants import AGE_TAGS, MOD_TAG_DEFAULT_DURATIONS, MOD_TAGS, TONE_TAGS, USER_TAG_KEYS

log = logging.getLogger(__name__)


class TagService(commands.Cog):
    """Single source of truth for user and moderator tags."""

    CLEANUP_INTERVAL_SECONDS = 300

    def __init__(self, bot: commands.Bot) -> None:
        self.bot = bot
        self._user_tags: dict[int, dict[str, str]] = {}
        self._mod_tags: dict[int, dict[str, dict[str, Any]]] = {}
        self._cleanup_task: asyncio.Task[None] | None = None
        self._cleanup_lock = asyncio.Lock()

    async def cog_load(self) -> None:
        await self._rehydrate_cache()
        await self._cleanup_expired_mod_tags()
        self._start_cleanup_task()

    async def cog_unload(self) -> None:
        if self._cleanup_task is None:
            return
        self._cleanup_task.cancel()
        try:
            await self._cleanup_task
        except asyncio.CancelledError:
            pass
        finally:
            self._cleanup_task = None

    async def get_user_tags(self, user_id: int) -> dict[str, str]:
        return dict(self._user_tags.get(int(user_id), {}))

    async def set_user_tag(self, user_id: int, key: str, value: str) -> None:
        normalized_key = self._validate_user_tag(key, value)
        normalized_user_id = int(user_id)
        current = self._user_tags.setdefault(normalized_user_id, {})
        old_value = current.get(normalized_key)

        await db.execute_async(
            """
            INSERT INTO user_tags(user_id, tag_key, tag_value)
            VALUES (?, ?, ?)
            ON CONFLICT(user_id, tag_key) DO UPDATE SET
              tag_value = excluded.tag_value,
              set_at = CURRENT_TIMESTAMP
            """,
            (normalized_user_id, normalized_key, value),
        )
        current[normalized_key] = value
        if old_value != value:
            self.bot.dispatch("tag_changed", normalized_user_id, normalized_key, old_value, value)

    async def clear_user_tag(self, user_id: int, key: str) -> None:
        normalized_user_id = int(user_id)
        normalized_key = self._validate_user_key(key)
        old_value = self._user_tags.get(normalized_user_id, {}).get(normalized_key)

        await db.execute_async(
            "DELETE FROM user_tags WHERE user_id = ? AND tag_key = ?",
            (normalized_user_id, normalized_key),
        )

        user_tags = self._user_tags.get(normalized_user_id)
        if user_tags is not None:
            user_tags.pop(normalized_key, None)
            if not user_tags:
                self._user_tags.pop(normalized_user_id, None)

        if old_value is not None:
            self.bot.dispatch("tag_changed", normalized_user_id, normalized_key, old_value, None)

    async def get_mod_tags(self, user_id: int) -> list[str]:
        normalized_user_id = int(user_id)
        self._prune_expired_cached_tags(normalized_user_id)
        return sorted(self._mod_tags.get(normalized_user_id, {}).keys())

    async def add_mod_tag(
        self,
        user_id: int,
        tag: str,
        *,
        set_by: int,
        reason: str | None,
        expires_at: datetime | None,
    ) -> None:
        normalized_user_id = int(user_id)
        normalized_tag = self._validate_mod_tag(tag)
        normalized_expires_at = self._normalize_expiry(normalized_tag, expires_at)
        expires_at_payload = (
            normalized_expires_at.isoformat() if normalized_expires_at is not None else None
        )

        await db.execute_async(
            """
            INSERT INTO user_mod_tags(user_id, mod_tag, set_by, reason, expires_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(user_id, mod_tag) DO UPDATE SET
              set_by = excluded.set_by,
              reason = excluded.reason,
              expires_at = excluded.expires_at,
              set_at = CURRENT_TIMESTAMP
            """,
            (normalized_user_id, normalized_tag, int(set_by), reason, expires_at_payload),
        )

        self._mod_tags.setdefault(normalized_user_id, {})[normalized_tag] = {
            "set_by": int(set_by),
            "reason": reason,
            "expires_at": normalized_expires_at,
        }
        self.bot.dispatch("mod_tag_added", normalized_user_id, normalized_tag, int(set_by), reason)

    async def remove_mod_tag(self, user_id: int, tag: str, removed_by: int) -> None:
        normalized_user_id = int(user_id)
        normalized_tag = self._validate_mod_tag(tag)
        existing = self._mod_tags.get(normalized_user_id, {}).get(normalized_tag)

        await db.execute_async(
            "DELETE FROM user_mod_tags WHERE user_id = ? AND mod_tag = ?",
            (normalized_user_id, normalized_tag),
        )

        if existing is None:
            return

        user_tags = self._mod_tags.get(normalized_user_id)
        if user_tags is not None:
            user_tags.pop(normalized_tag, None)
            if not user_tags:
                self._mod_tags.pop(normalized_user_id, None)
        self.bot.dispatch("mod_tag_removed", normalized_user_id, normalized_tag, int(removed_by))

    async def has_active_mod_tag(self, user_id: int, tag: str) -> bool:
        normalized_user_id = int(user_id)
        normalized_tag = self._validate_mod_tag(tag)
        self._prune_expired_cached_tags(normalized_user_id)
        return normalized_tag in self._mod_tags.get(normalized_user_id, {})

    async def has_user_tag(self, user_id: int, key: str, value: str) -> bool:
        normalized_key = self._validate_user_tag(key, value)
        normalized_user_id = int(user_id)
        return self._user_tags.get(normalized_user_id, {}).get(normalized_key) == value

    async def _rehydrate_cache(self) -> None:
        user_rows = await db.query_all_async(
            "SELECT user_id, tag_key, tag_value FROM user_tags ORDER BY user_id, tag_key"
        )
        mod_rows = await db.query_all_async(
            """
            SELECT user_id, mod_tag, set_by, reason, expires_at
            FROM user_mod_tags
            ORDER BY user_id, mod_tag
            """
        )

        user_cache: dict[int, dict[str, str]] = {}
        mod_cache: dict[int, dict[str, dict[str, Any]]] = {}

        for row in user_rows:
            user_cache.setdefault(int(row["user_id"]), {})[str(row["tag_key"])] = str(
                row["tag_value"]
            )

        now = self._utcnow()
        for row in mod_rows:
            expires_at = self._parse_datetime(row["expires_at"])
            if expires_at is not None and expires_at <= now:
                continue
            mod_cache.setdefault(int(row["user_id"]), {})[str(row["mod_tag"])] = {
                "set_by": int(row["set_by"]),
                "reason": row["reason"],
                "expires_at": expires_at,
            }

        self._user_tags = user_cache
        self._mod_tags = mod_cache

    def _start_cleanup_task(self) -> None:
        if self._cleanup_task is not None and not self._cleanup_task.done():
            return
        self._cleanup_task = asyncio.create_task(self._cleanup_loop())

    async def _cleanup_loop(self) -> None:
        try:
            while True:
                await asyncio.sleep(self.CLEANUP_INTERVAL_SECONDS)
                await self._cleanup_expired_mod_tags()
        except asyncio.CancelledError:
            raise
        except Exception:
            log.exception("TagService cleanup loop crashed")

    async def _cleanup_expired_mod_tags(self) -> list[tuple[int, str]]:
        async with self._cleanup_lock:
            rows = await db.query_all_async(
                """
                SELECT user_id, mod_tag, expires_at
                FROM user_mod_tags
                WHERE expires_at IS NOT NULL
                """
            )
            now = self._utcnow()
            expired: list[tuple[int, str]] = []
            for row in rows:
                expires_at = self._parse_datetime(row["expires_at"])
                if expires_at is None or expires_at > now:
                    continue
                expired.append((int(row["user_id"]), str(row["mod_tag"])))

            if not expired:
                return []

            async with db.transaction() as conn:
                for user_id, tag in expired:
                    conn.execute(
                        "DELETE FROM user_mod_tags WHERE user_id = ? AND mod_tag = ?",
                        (user_id, tag),
                    )

            for user_id, tag in expired:
                cached_tags = self._mod_tags.get(user_id)
                if cached_tags is not None:
                    cached_tags.pop(tag, None)
                    if not cached_tags:
                        self._mod_tags.pop(user_id, None)
                self.bot.dispatch("mod_tag_removed", user_id, tag, 0)
            return expired

    def _prune_expired_cached_tags(self, user_id: int) -> None:
        cached_tags = self._mod_tags.get(int(user_id))
        if not cached_tags:
            return

        now = self._utcnow()
        expired = [
            tag
            for tag, payload in cached_tags.items()
            if (payload.get("expires_at") is not None and payload["expires_at"] <= now)
        ]
        for tag in expired:
            cached_tags.pop(tag, None)
        if not cached_tags:
            self._mod_tags.pop(int(user_id), None)

    def _validate_user_key(self, key: str) -> str:
        normalized_key = str(key or "").strip().lower()
        if normalized_key not in USER_TAG_KEYS:
            raise ValueError(f"unsupported user tag key: {key}")
        return normalized_key

    def _validate_user_tag(self, key: str, value: str) -> str:
        normalized_key = self._validate_user_key(key)
        normalized_value = str(value or "").strip().lower()
        allowed_values = {
            "age": set(AGE_TAGS),
            "tone": set(TONE_TAGS),
        }[normalized_key]
        if normalized_value not in allowed_values:
            raise ValueError(f"unsupported value for {normalized_key}: {value}")
        return normalized_key

    def _validate_mod_tag(self, tag: str) -> str:
        normalized_tag = str(tag or "").strip().lower()
        if normalized_tag not in MOD_TAGS:
            raise ValueError(f"unsupported mod tag: {tag}")
        return normalized_tag

    def _normalize_expiry(self, tag: str, expires_at: datetime | None) -> datetime | None:
        if expires_at is None:
            default_duration = MOD_TAG_DEFAULT_DURATIONS.get(tag)
            if default_duration is None:
                return None
            expires_at = self._utcnow() + default_duration

        if expires_at.tzinfo is None:
            expires_at = expires_at.replace(tzinfo=UTC)
        else:
            expires_at = expires_at.astimezone(UTC)
        return expires_at

    def _parse_datetime(self, raw: Any) -> datetime | None:
        if raw is None:
            return None
        text = str(raw).strip()
        if not text:
            return None
        try:
            parsed = datetime.fromisoformat(text)
        except ValueError:
            return None
        if parsed.tzinfo is None:
            return parsed.replace(tzinfo=UTC)
        return parsed.astimezone(UTC)

    def _utcnow(self) -> datetime:
        return datetime.now(UTC)
