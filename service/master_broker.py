from __future__ import annotations

import asyncio
import hashlib
import inspect
import ipaddress
import json
import logging
import os
import secrets
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlsplit

import discord
from aiohttp import web

logger = logging.getLogger(__name__)


def _safe_log_value(value: Any) -> str:
    """Sanitize values before logging to prevent log injection attacks."""
    text = "" if value is None else str(value)
    return text.replace("\r", "\\r").replace("\n", "\\n")


_INTERNAL_TOKEN_HEADER = "X-Internal-Token"
_IDEMPOTENCY_HEADER = "X-Idempotency-Key"
_REQUEST_ID_HEADER = "X-Request-Id"


@dataclass(slots=True)
class _IdempotencyRecord:
    payload_hash: str
    response_status: int
    response_body: dict[str, Any]
    created_monotonic: float


@dataclass(slots=True)
class _IdempotencyInFlight:
    payload_hash: str
    future: asyncio.Future[tuple[int, dict[str, Any]]]
    created_monotonic: float


@dataclass(slots=True)
class _IdempotencyDecision:
    should_execute: bool
    cached_status: int = 0
    cached_body: dict[str, Any] | None = None
    pending_future: asyncio.Future[tuple[int, dict[str, Any]]] | None = None


@dataclass(slots=True)
class _RichMessagePayload:
    channel_id: int
    content: str | None
    embed_dict: dict[str, Any]
    embed: discord.Embed
    allowed_user_ids: list[int]
    allowed_role_ids: list[int]
    allowed_mentions: discord.AllowedMentions
    view_spec: dict[str, Any] | None


@dataclass(slots=True)
class _ResolvedBrokerView:
    view: discord.ui.View | None
    should_register: bool = False


class MasterBroker:
    """Localhost-only broker for Discord actions handled by the master runtime."""

    def __init__(
        self,
        bot: Any,
        *,
        token: str,
        host: str = "127.0.0.1",
        port: int = 8770,
    ) -> None:
        self.bot = bot
        self.token = str(token or "").strip()
        if not self.token:
            raise ValueError("master broker token must not be empty")

        self.host = (host or "127.0.0.1").strip() or "127.0.0.1"
        self.port = int(port)
        self._bound_port = int(port)

        self._runner: web.AppRunner | None = None
        self._site: web.TCPSite | None = None
        self._started = False
        self._lock = asyncio.Lock()
        self._idempotency_lock = asyncio.Lock()
        self._idempotency_records: dict[str, _IdempotencyRecord] = {}
        self._idempotency_inflight: dict[str, _IdempotencyInFlight] = {}

        self._idempotency_ttl_seconds = self._parse_positive_float(
            os.getenv("MASTER_BROKER_IDEMPOTENCY_TTL_SECONDS"),
            default=600.0,
        )
        self._idempotency_max_entries = self._parse_positive_int(
            os.getenv("MASTER_BROKER_IDEMPOTENCY_MAX_ENTRIES"),
            default=1024,
        )
        self._idempotency_inflight_ttl_seconds = self._parse_positive_float(
            os.getenv("MASTER_BROKER_IDEMPOTENCY_INFLIGHT_TTL_SECONDS"),
            default=max(300.0, self._idempotency_ttl_seconds),
        )
        self._idempotency_waiter_timeout_seconds = self._parse_positive_float(
            os.getenv("MASTER_BROKER_IDEMPOTENCY_WAITER_TIMEOUT_SECONDS"),
            default=15.0,
        )
        self._channel_allowlist_enabled, self._allowed_channel_ids = self._read_allowlist(
            "MASTER_BROKER_ALLOWED_CHANNEL_IDS",
            "MASTER_BROKER_ALLOW_CHANNEL_IDS",
            "MASTER_BROKER_CHANNEL_ALLOWLIST_IDS",
        )
        self._guild_allowlist_enabled, self._allowed_guild_ids = self._read_allowlist(
            "MASTER_BROKER_ALLOWED_GUILD_IDS",
            "MASTER_BROKER_ALLOW_GUILD_IDS",
            "MASTER_BROKER_GUILD_ALLOWLIST_IDS",
        )
        self._role_allowlist_enabled, self._allowed_role_ids = self._read_allowlist(
            "MASTER_BROKER_ALLOWED_ROLE_IDS",
            "MASTER_BROKER_ALLOW_ROLE_IDS",
            "MASTER_BROKER_ROLE_ALLOWLIST_IDS",
        )

    @staticmethod
    def _parse_positive_float(raw: str | None, *, default: float) -> float:
        if raw is None:
            return default
        try:
            parsed = float(raw)
        except ValueError:
            return default
        if parsed <= 0:
            return default
        return parsed

    @staticmethod
    def _parse_positive_int(raw: str | None, *, default: int) -> int:
        if raw is None:
            return default
        try:
            parsed = int(raw)
        except ValueError:
            return default
        if parsed <= 0:
            return default
        return parsed

    @staticmethod
    def _parse_id_tokens(raw: str) -> set[int]:
        parsed: set[int] = set()
        for token in raw.replace(",", " ").split():
            value = token.strip()
            if not value.isdigit():
                continue
            number = int(value)
            if number > 0:
                parsed.add(number)
        return parsed

    @classmethod
    def _read_allowlist(cls, *env_names: str) -> tuple[bool, set[int]]:
        for env_name in env_names:
            raw = os.getenv(env_name)
            if raw is None:
                continue
            value = raw.strip()
            if not value:
                continue
            parsed = cls._parse_id_tokens(value)
            if not parsed:
                logger.warning(
                    "Master broker allowlist %s is set but has no valid positive IDs; denying all for this scope.",
                    env_name,
                )
            return True, parsed
        return False, set()

    @property
    def base_url(self) -> str:
        host = self.host
        if host in {"0.0.0.0", "::"}:
            host = "127.0.0.1"
        return f"http://{host}:{self._bound_port}"

    async def start(self) -> None:
        async with self._lock:
            if self._started:
                return

            app = web.Application()
            app.add_routes(
                [
                    web.get("/internal/master/v1/health", self._handle_health),
                    web.post("/internal/master/v1/discord/send-message", self._handle_send_message),
                    web.post(
                        "/internal/master/v1/discord/create-channel",
                        self._handle_create_channel,
                    ),
                    web.post(
                        "/internal/master/v1/discord/delete-channel",
                        self._handle_delete_channel,
                    ),
                    web.post(
                        "/internal/master/v1/discord/send-rich-message",
                        self._handle_send_rich_message,
                    ),
                    web.post(
                        "/internal/master/v1/discord/edit-rich-message",
                        self._handle_edit_rich_message,
                    ),
                    web.post(
                        "/internal/master/v1/discord/member/add-role",
                        self._handle_add_role,
                    ),
                    web.post(
                        "/internal/master/v1/discord/member/move-voice",
                        self._handle_move_voice,
                    ),
                    web.post(
                        "/internal/master/v1/discord/voice-channel/members",
                        self._handle_get_voice_members,
                    ),
                ]
            )

            runner = web.AppRunner(app)
            await runner.setup()

            site = web.TCPSite(runner, self.host, self.port)
            await site.start()

            self._runner = runner
            self._site = site
            self._started = True
            self._bound_port = self._resolve_bound_port(default=self.port)
            logger.info("Master broker listening on %s", self.base_url)

    async def stop(self) -> None:
        async with self._lock:
            if not self._started:
                return
            if self._site is not None:
                await self._site.stop()
            if self._runner is not None:
                await self._runner.cleanup()

            self._site = None
            self._runner = None
            self._started = False
            logger.info("Master broker stopped")

    def _resolve_bound_port(self, *, default: int) -> int:
        site = self._site
        if site is None:
            return default
        server = getattr(site, "_server", None)
        sockets = getattr(server, "sockets", None)
        if not sockets:
            return default
        try:
            sockname = sockets[0].getsockname()
            if isinstance(sockname, tuple) and len(sockname) > 1:
                return int(sockname[1])
        except Exception:
            return default
        return default

    @staticmethod
    def _request_id(request: web.Request) -> str:
        candidate = (request.headers.get(_REQUEST_ID_HEADER) or "").strip()
        if candidate and "\r" not in candidate and "\n" not in candidate:
            return candidate[:128]
        return secrets.token_hex(8)

    @staticmethod
    def _host_without_port(raw: str | None) -> str:
        if not raw:
            return ""
        value = raw.split(",")[0].strip()
        if not value:
            return ""
        if value.startswith("["):
            end = value.find("]")
            if end != -1:
                value = value[1:end]
            return value.lower()

        if value.count(":") == 1:
            host_part, port_part = value.rsplit(":", 1)
            if port_part.isdigit():
                value = host_part
        elif ":" in value:
            # Accept unbracketed IPv6 literals with a numeric trailing port ("::1:8770").
            host_part, port_part = value.rsplit(":", 1)
            if port_part.isdigit():
                try:
                    ipaddress.IPv6Address(host_part)
                    value = host_part
                except ValueError:
                    pass  # Invalid IPv6 parse means the host part is not an IPv6 literal.

        # Plain IP literals should be accepted as-is (for example "::1").
        try:
            ipaddress.ip_address(value)
            return value.lower()
        except ValueError:
            return value.lower()

    @staticmethod
    def _is_loopback_host(raw: str | None) -> bool:
        host = MasterBroker._host_without_port(raw)
        if not host:
            return False
        if host in {"localhost", "localhost."}:
            return True
        try:
            ip_obj = ipaddress.ip_address(host)
            if ip_obj.is_loopback:
                return True
            if isinstance(ip_obj, ipaddress.IPv6Address) and ip_obj.ipv4_mapped is not None:
                return ip_obj.ipv4_mapped.is_loopback
            return False
        except ValueError:
            return False

    @staticmethod
    def _peer_host(request: web.Request) -> str:
        remote = (request.remote or "").strip()
        if remote:
            return remote
        transport = getattr(request, "transport", None)
        if transport is None:
            return ""
        peer = transport.get_extra_info("peername")
        if isinstance(peer, tuple) and peer:
            return str(peer[0]).strip()
        if isinstance(peer, str):
            return peer.strip()
        return ""

    def _error_response(
        self,
        *,
        request: web.Request,
        status: int,
        code: str,
        message: str,
        idempotency_key: str | None = None,
    ) -> web.Response:
        payload = {
            "ok": False,
            "request_id": self._request_id(request),
            "idempotency_key": idempotency_key,
            "cached": False,
            "result": None,
            "error": {"code": code, "message": message},
        }
        return web.json_response(payload, status=status)

    def _success_response(
        self,
        *,
        request: web.Request,
        result: dict[str, Any],
        idempotency_key: str | None = None,
        cached: bool = False,
        status: int = 200,
    ) -> web.Response:
        payload = {
            "ok": True,
            "request_id": self._request_id(request),
            "idempotency_key": idempotency_key,
            "cached": cached,
            "result": result,
            "error": None,
        }
        return web.json_response(payload, status=status)

    def _authorize(self, request: web.Request) -> web.Response | None:
        peer = self._peer_host(request)
        if not self._is_loopback_host(peer):
            logger.warning(
                "Master broker rejected non-loopback request from %s",
                _safe_log_value(peer) or "<unknown>",
            )
            return self._error_response(
                request=request,
                status=403,
                code="forbidden",
                message="loopback requests only",
            )

        token = (request.headers.get(_INTERNAL_TOKEN_HEADER) or "").strip()
        if not token or not secrets.compare_digest(token, self.token):
            logger.warning(
                "Master broker rejected unauthorized request from %s",
                _safe_log_value(peer) or "<unknown>",
            )
            return self._error_response(
                request=request,
                status=401,
                code="unauthorized",
                message=f"missing or invalid {_INTERNAL_TOKEN_HEADER}",
            )

        return None

    async def _read_json_object(self, request: web.Request) -> dict[str, Any]:
        payload = await request.json()
        if not isinstance(payload, dict):
            raise ValueError("payload must be a JSON object")
        return payload

    @staticmethod
    def _payload_hash(payload: dict[str, Any]) -> str:
        encoded = json.dumps(payload, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
        return hashlib.sha256(encoded.encode("utf-8")).hexdigest()

    @staticmethod
    def _extract_idempotency_key(request: web.Request, payload: dict[str, Any]) -> str:
        header_key = (request.headers.get(_IDEMPOTENCY_HEADER) or "").strip()
        body_key = str(payload.get("idempotency_key") or "").strip()
        if header_key and body_key and header_key != body_key:
            raise ValueError("header/body idempotency key mismatch")
        key = header_key or body_key
        if not key:
            raise ValueError("idempotency_key is required")
        if len(key) > 128:
            raise ValueError("idempotency_key is too long")
        return key

    @staticmethod
    def _idempotency_cache_key(action: str, idempotency_key: str) -> str:
        return f"{action}:{idempotency_key}"

    @staticmethod
    def _idempotency_key_from_cache_key(cache_key: str) -> str:
        _action, _sep, key = cache_key.partition(":")
        return key

    @staticmethod
    def _inflight_failure_payload(
        *,
        idempotency_key: str,
        code: str,
        message: str,
    ) -> dict[str, Any]:
        return {
            "ok": False,
            "request_id": secrets.token_hex(8),
            "idempotency_key": idempotency_key,
            "cached": False,
            "result": None,
            "error": {"code": code, "message": message},
        }

    @staticmethod
    def _resolve_inflight_future(
        *,
        state: _IdempotencyInFlight,
        response_status: int,
        response_body: dict[str, Any],
    ) -> None:
        if state.future.done():
            return
        state.future.set_result((response_status, dict(response_body)))

    async def _begin_idempotent_request(
        self,
        *,
        action: str,
        idempotency_key: str,
        payload_hash: str,
    ) -> _IdempotencyDecision:
        now = time.monotonic()
        cache_key = self._idempotency_cache_key(action, idempotency_key)
        async with self._idempotency_lock:
            self._prune_idempotency_locked(now)

            record = self._idempotency_records.get(cache_key)
            if record is not None:
                if record.payload_hash != payload_hash:
                    raise ValueError("idempotency key already used with different payload")
                return _IdempotencyDecision(
                    should_execute=False,
                    cached_status=record.response_status,
                    cached_body=dict(record.response_body),
                )

            in_flight = self._idempotency_inflight.get(cache_key)
            if in_flight is not None:
                if in_flight.payload_hash != payload_hash:
                    raise ValueError("idempotency key already used with different payload")
                return _IdempotencyDecision(
                    should_execute=False,
                    pending_future=in_flight.future,
                )

            future: asyncio.Future[tuple[int, dict[str, Any]]] = (
                asyncio.get_running_loop().create_future()
            )
            self._idempotency_inflight[cache_key] = _IdempotencyInFlight(
                payload_hash=payload_hash,
                future=future,
                created_monotonic=now,
            )
            return _IdempotencyDecision(should_execute=True)

    async def _complete_idempotent_request(
        self,
        *,
        action: str,
        idempotency_key: str,
        payload_hash: str,
        response_status: int,
        response_body: dict[str, Any],
        cache_response: bool,
    ) -> None:
        now = time.monotonic()
        cache_key = self._idempotency_cache_key(action, idempotency_key)
        payload_copy = dict(response_body)
        async with self._idempotency_lock:
            self._prune_idempotency_locked(now)
            in_flight = self._idempotency_inflight.pop(cache_key, None)
            if cache_response:
                self._idempotency_records[cache_key] = _IdempotencyRecord(
                    payload_hash=payload_hash,
                    response_status=response_status,
                    response_body=dict(payload_copy),
                    created_monotonic=now,
                )
                if len(self._idempotency_records) > self._idempotency_max_entries:
                    oldest_key = min(
                        self._idempotency_records,
                        key=lambda item: self._idempotency_records[item].created_monotonic,
                    )
                    self._idempotency_records.pop(oldest_key, None)

            if in_flight is not None:
                self._resolve_inflight_future(
                    state=in_flight,
                    response_status=response_status,
                    response_body=payload_copy,
                )

    async def _fail_idempotent_request(
        self,
        *,
        action: str,
        idempotency_key: str,
        response_status: int,
        code: str,
        message: str,
    ) -> None:
        cache_key = self._idempotency_cache_key(action, idempotency_key)
        payload = self._inflight_failure_payload(
            idempotency_key=idempotency_key,
            code=code,
            message=message,
        )
        async with self._idempotency_lock:
            state = self._idempotency_inflight.pop(cache_key, None)
            if state is None:
                return
            self._resolve_inflight_future(
                state=state,
                response_status=response_status,
                response_body=payload,
            )

    async def _settle_idempotent_request(
        self,
        *,
        action: str,
        idempotency_key: str,
        payload_hash: str,
        response_status: int,
        response_body: dict[str, Any],
        cache_response: bool,
    ) -> None:
        try:
            await asyncio.shield(
                self._complete_idempotent_request(
                    action=action,
                    idempotency_key=idempotency_key,
                    payload_hash=payload_hash,
                    response_status=response_status,
                    response_body=response_body,
                    cache_response=cache_response,
                )
            )
        except BaseException:
            logger.exception(
                "Master broker failed to settle idempotent result (action=%s key=%s)",
                _safe_log_value(action),
                _safe_log_value(idempotency_key),
            )
            await self._fail_idempotent_request(
                action=action,
                idempotency_key=idempotency_key,
                response_status=500,
                code="internal_error",
                message="idempotent operation could not be finalized",
            )

    def _prune_idempotency_locked(self, now: float) -> None:
        stale_records = [
            key
            for key, record in self._idempotency_records.items()
            if now - record.created_monotonic > self._idempotency_ttl_seconds
        ]
        for key in stale_records:
            self._idempotency_records.pop(key, None)

        stale_in_flight = [
            key for key, state in self._idempotency_inflight.items() if state.future.done()
        ]
        for key in stale_in_flight:
            self._idempotency_inflight.pop(key, None)

        stale_pending_in_flight = [
            key
            for key, state in self._idempotency_inflight.items()
            if now - state.created_monotonic > self._idempotency_inflight_ttl_seconds
        ]
        for key in stale_pending_in_flight:
            state = self._idempotency_inflight.pop(key, None)
            if state is None:
                continue
            payload = self._inflight_failure_payload(
                idempotency_key=self._idempotency_key_from_cache_key(key),
                code="idempotency_expired",
                message="idempotent operation expired before completion",
            )
            self._resolve_inflight_future(
                state=state,
                response_status=500,
                response_body=payload,
            )

    @staticmethod
    def _response_payload(response: web.Response) -> dict[str, Any]:
        raw = response.text if response.text is not None else ""
        if not raw and response.body is not None:
            try:
                raw = response.body.decode("utf-8")
            except Exception:
                raw = ""
        if not raw:
            return {}
        try:
            parsed = json.loads(raw)
        except Exception:
            return {}
        if isinstance(parsed, dict):
            return parsed
        return {}

    @staticmethod
    def _cached_payload(payload: dict[str, Any]) -> dict[str, Any]:
        cached_payload = dict(payload)
        cached_payload["cached"] = True
        return cached_payload

    def _allowlist_check(
        self,
        *,
        request: web.Request,
        idempotency_key: str | None = None,
        scope: str,
        value: int,
        enabled: bool,
        allowed_ids: set[int],
    ) -> web.Response | None:
        if not enabled:
            return None
        if value in allowed_ids:
            return None
        return self._error_response(
            request=request,
            status=403,
            code="forbidden",
            message=f"{scope}_id {value} is not permitted",
            idempotency_key=idempotency_key,
        )

    async def _run_idempotent_action(
        self,
        *,
        request: web.Request,
        action: str,
        idempotency_key: str,
        payload_hash: str,
        operation: Callable[[], Awaitable[web.Response]],
    ) -> web.Response:
        try:
            decision = await self._begin_idempotent_request(
                action=action,
                idempotency_key=idempotency_key,
                payload_hash=payload_hash,
            )
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=409,
                code="idempotency_conflict",
                message=str(exc),
                idempotency_key=idempotency_key,
            )

        if decision.cached_body is not None:
            return web.json_response(
                self._cached_payload(decision.cached_body),
                status=decision.cached_status,
            )

        if decision.pending_future is not None:
            try:
                pending_status, pending_body = await asyncio.wait_for(
                    asyncio.shield(decision.pending_future),
                    timeout=self._idempotency_waiter_timeout_seconds,
                )
            except TimeoutError:
                return self._error_response(
                    request=request,
                    status=504,
                    code="idempotency_wait_timeout",
                    message="idempotent operation is still pending; retry later",
                    idempotency_key=idempotency_key,
                )
            except Exception:
                logger.exception(
                    "Master broker idempotent waiter failed (action=%s key=%s)",
                    _safe_log_value(action),
                    _safe_log_value(idempotency_key),
                )
                return self._error_response(
                    request=request,
                    status=500,
                    code="internal_error",
                    message="failed to await idempotent result",
                    idempotency_key=idempotency_key,
                )
            return web.json_response(
                self._cached_payload(pending_body),
                status=pending_status,
            )

        if not decision.should_execute:
            return self._error_response(
                request=request,
                status=500,
                code="internal_error",
                message="invalid idempotency state",
                idempotency_key=idempotency_key,
            )

        try:
            response = await operation()
        except asyncio.CancelledError:
            logger.warning(
                "Master broker idempotent operation cancelled (action=%s key=%s)",
                _safe_log_value(action),
                _safe_log_value(idempotency_key),
            )
            cancelled_response = self._error_response(
                request=request,
                status=503,
                code="request_cancelled",
                message="request was cancelled",
                idempotency_key=idempotency_key,
            )
            await self._settle_idempotent_request(
                action=action,
                idempotency_key=idempotency_key,
                payload_hash=payload_hash,
                response_status=cancelled_response.status,
                response_body=self._response_payload(cancelled_response),
                cache_response=False,
            )
            raise
        except BaseException as exc:
            logger.exception(
                "Master broker idempotent operation crashed (action=%s key=%s)",
                _safe_log_value(action),
                _safe_log_value(idempotency_key),
            )
            response = self._error_response(
                request=request,
                status=500,
                code="internal_error",
                message="internal broker error",
                idempotency_key=idempotency_key,
            )
            await self._settle_idempotent_request(
                action=action,
                idempotency_key=idempotency_key,
                payload_hash=payload_hash,
                response_status=response.status,
                response_body=self._response_payload(response),
                cache_response=False,
            )
            if isinstance(exc, Exception):
                return response
            raise

        response_payload = self._response_payload(response)
        cache_response = bool(response_payload.get("ok")) and 200 <= response.status < 300
        await self._settle_idempotent_request(
            action=action,
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            response_status=response.status,
            response_body=response_payload,
            cache_response=cache_response,
        )
        return response

    @staticmethod
    def _parse_positive_payload_int(payload: dict[str, Any], key: str) -> int:
        raw = payload.get(key)
        if isinstance(raw, bool):
            raise ValueError(f"{key} must be a positive integer")
        if isinstance(raw, int):
            value = raw
        elif isinstance(raw, str) and raw.strip().isdigit():
            value = int(raw.strip())
        else:
            raise ValueError(f"{key} must be a positive integer")
        if value <= 0:
            raise ValueError(f"{key} must be a positive integer")
        return value

    @staticmethod
    def _parse_optional_content(payload: dict[str, Any], key: str = "content") -> str | None:
        raw = payload.get(key)
        if raw is None:
            return None
        if not isinstance(raw, str):
            raise ValueError(f"{key} must be a string")
        content = raw.strip()
        if not content:
            return None
        if len(content) > 2000:
            raise ValueError(f"{key} exceeds Discord limit (2000)")
        return content

    @staticmethod
    def _parse_embed_dict(payload: dict[str, Any]) -> tuple[dict[str, Any], discord.Embed]:
        raw_embed = payload.get("embed")
        if not isinstance(raw_embed, dict):
            raise ValueError("embed must be a JSON object")
        embed_dict = dict(raw_embed)
        try:
            embed = discord.Embed.from_dict(embed_dict)
        except Exception as exc:
            raise ValueError("embed payload is invalid") from exc
        return embed_dict, embed

    @classmethod
    def _parse_int_id_list(cls, payload: dict[str, Any], key: str) -> list[int]:
        raw_value = payload.get(key)
        if raw_value is None:
            return []
        if not isinstance(raw_value, list):
            raise ValueError(f"{key} must be a list of positive integers")

        parsed: list[int] = []
        seen: set[int] = set()
        for item in raw_value:
            if isinstance(item, bool):
                raise ValueError(f"{key} must be a list of positive integers")
            if isinstance(item, int):
                value = item
            elif isinstance(item, str) and item.strip().isdigit():
                value = int(item.strip())
            else:
                raise ValueError(f"{key} must be a list of positive integers")
            if value <= 0:
                raise ValueError(f"{key} must be a list of positive integers")
            if value in seen:
                continue
            seen.add(value)
            parsed.append(value)
        return parsed

    @classmethod
    def _parse_role_id_list(cls, payload: dict[str, Any], key: str) -> list[int]:
        return cls._parse_int_id_list(payload, key)

    @staticmethod
    def _build_allowed_mentions(
        allowed_user_ids: list[int],
        allowed_role_ids: list[int],
    ) -> discord.AllowedMentions:
        return discord.AllowedMentions(
            everyone=False,
            users=[discord.Object(id=user_id) for user_id in allowed_user_ids]
            if allowed_user_ids
            else False,
            roles=[discord.Object(id=role_id) for role_id in allowed_role_ids]
            if allowed_role_ids
            else False,
            replied_user=False,
        )

    @staticmethod
    def _validate_url(raw_url: str) -> str:
        url = str(raw_url or "").strip()
        if not url:
            raise ValueError("view_spec.url is required")
        parsed = urlsplit(url)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            raise ValueError("view_spec.url must use http or https")
        return url

    @classmethod
    def _parse_view_spec(cls, payload: dict[str, Any]) -> dict[str, Any] | None:
        raw_spec = payload.get("view_spec")
        if raw_spec is None:
            return None
        if not isinstance(raw_spec, dict):
            raise ValueError("view_spec must be a JSON object")

        view_type = str(raw_spec.get("type") or "").strip()
        if view_type not in {"twitch_live_tracking", "link_button"}:
            raise ValueError("view_spec.type is invalid")

        parsed_spec = dict(raw_spec)
        parsed_spec["type"] = view_type
        if view_type == "link_button":
            label = str(parsed_spec.get("label") or "").strip()
            if not label:
                raise ValueError("view_spec.label is required")
            if len(label) > 80:
                raise ValueError("view_spec.label exceeds Discord limit (80)")
            parsed_spec["label"] = label
            parsed_spec["url"] = cls._validate_url(str(parsed_spec.get("url") or ""))

        return parsed_spec

    @classmethod
    def _parse_rich_message_payload(cls, payload: dict[str, Any]) -> _RichMessagePayload:
        channel_id = cls._parse_positive_payload_int(payload, "channel_id")
        content = cls._parse_optional_content(payload)
        embed_dict, embed = cls._parse_embed_dict(payload)
        allowed_user_ids = cls._parse_int_id_list(payload, "allowed_user_ids")
        allowed_role_ids = cls._parse_role_id_list(payload, "allowed_role_ids")
        view_spec = cls._parse_view_spec(payload)
        if content is None and not embed_dict:
            raise ValueError("content or embed is required")
        return _RichMessagePayload(
            channel_id=channel_id,
            content=content,
            embed_dict=embed_dict,
            embed=embed,
            allowed_user_ids=allowed_user_ids,
            allowed_role_ids=allowed_role_ids,
            allowed_mentions=cls._build_allowed_mentions(allowed_user_ids, allowed_role_ids),
            view_spec=view_spec,
        )

    async def _resolve_channel(self, channel_id: int) -> Any | None:
        channel = None
        try:
            channel = self.bot.get_channel(channel_id)
        except Exception:
            channel = None

        if channel is not None:
            return channel

        fetch_channel = getattr(self.bot, "fetch_channel", None)
        if callable(fetch_channel):
            try:
                return await fetch_channel(channel_id)
            except Exception:
                return None
        return None

    async def _resolve_user(self, user_id: int) -> Any | None:
        user = None
        try:
            user = self.bot.get_user(user_id)
        except Exception:
            user = None

        if user is not None:
            return user

        fetch_user = getattr(self.bot, "fetch_user", None)
        if callable(fetch_user):
            try:
                return await fetch_user(user_id)
            except Exception:
                return None
        return None

    async def _resolve_message(self, *, channel: Any, message_id: int) -> Any | None:
        fetch_message = getattr(channel, "fetch_message", None)
        if not callable(fetch_message):
            return None
        try:
            return await fetch_message(message_id)
        except Exception:
            return None

    async def _resolve_broker_view(
        self,
        *,
        request: web.Request,
        idempotency_key: str,
        view_spec: dict[str, Any] | None,
    ) -> _ResolvedBrokerView | web.Response:
        if view_spec is None:
            return _ResolvedBrokerView(view=None, should_register=False)

        view_type = str(view_spec.get("type") or "").strip()
        if view_type == "link_button":
            view = discord.ui.View(timeout=None)
            view.add_item(
                discord.ui.Button(
                    label=str(view_spec["label"]),
                    style=discord.ButtonStyle.link,
                    url=str(view_spec["url"]),
                )
            )
            return _ResolvedBrokerView(view=view, should_register=False)

        resolver = getattr(self.bot, "resolve_master_broker_view_spec", None)
        if not callable(resolver):
            return self._error_response(
                request=request,
                status=503,
                code="view_resolver_unavailable",
                message="twitch live tracking view resolver is unavailable",
                idempotency_key=idempotency_key,
            )

        try:
            resolved_view = resolver(dict(view_spec))
            if inspect.isawaitable(resolved_view):
                resolved_view = await resolved_view
        except Exception as exc:
            logger.error("Master broker view resolver failed (%s): %s", view_type, exc)
            return self._error_response(
                request=request,
                status=502,
                code="view_resolver_failed",
                message="failed to resolve message view",
                idempotency_key=idempotency_key,
            )

        if not isinstance(resolved_view, discord.ui.View):
            return self._error_response(
                request=request,
                status=500,
                code="view_resolver_invalid",
                message="view resolver returned an invalid view",
                idempotency_key=idempotency_key,
            )

        return _ResolvedBrokerView(view=resolved_view, should_register=True)

    async def _bind_and_register_view(
        self,
        *,
        view: discord.ui.View | None,
        channel_id: int,
        message_id: int,
        should_register: bool,
    ) -> None:
        if view is None or not should_register:
            return

        bind_method = getattr(view, "bind_to_message", None)
        if callable(bind_method):
            try:
                bind_result = bind_method(channel_id=channel_id, message_id=message_id)
                if inspect.isawaitable(bind_result):
                    await bind_result
            except Exception:
                logger.exception(
                    "Master broker could not bind persistent view (channel=%s message=%s)",
                    channel_id,
                    message_id,
                )
        try:
            self.bot.add_view(view, message_id=message_id)
        except Exception:
            logger.exception(
                "Master broker could not register persistent view (channel=%s message=%s)",
                channel_id,
                message_id,
            )

    async def _handle_health(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        ready_callable = getattr(self.bot, "is_ready", None)
        is_ready = bool(ready_callable()) if callable(ready_callable) else False
        return self._success_response(
            request=request,
            result={
                "status": "ok",
                "bot_ready": is_ready,
                "runtime_role": "master",
            },
        )

    async def _handle_send_message(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            idempotency_key = self._extract_idempotency_key(request, payload)
            content = str(payload.get("content") or "").strip()
            if not content:
                raise ValueError("content is required")
            if len(content) > 2000:
                raise ValueError("content exceeds Discord limit (2000)")
            channel_id_raw = payload.get("channel_id")
            user_id_raw = payload.get("user_id")
            if channel_id_raw is None and user_id_raw is None:
                raise ValueError("channel_id or user_id is required")
            if channel_id_raw is not None and user_id_raw is not None:
                raise ValueError("channel_id and user_id are mutually exclusive")
            channel_id = (
                self._parse_positive_payload_int(payload, "channel_id")
                if channel_id_raw is not None
                else None
            )
            user_id = (
                self._parse_positive_payload_int(payload, "user_id")
                if user_id_raw is not None
                else None
            )
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        if channel_id is not None:
            allowlist_rejected = self._allowlist_check(
                request=request,
                idempotency_key=idempotency_key,
                scope="channel",
                value=channel_id,
                enabled=self._channel_allowlist_enabled,
                allowed_ids=self._allowed_channel_ids,
            )
            if allowlist_rejected is not None:
                return allowlist_rejected

        operation_payload = {"content": content}
        if channel_id is not None:
            operation_payload["channel_id"] = channel_id
        if user_id is not None:
            operation_payload["user_id"] = user_id
        payload_hash = self._payload_hash(operation_payload)

        async def _operation() -> web.Response:
            channel_id_value: int | None = channel_id
            try:
                if channel_id is not None:
                    channel = await self._resolve_channel(channel_id)
                    if channel is None or not hasattr(channel, "send"):
                        return self._error_response(
                            request=request,
                            status=404,
                            code="not_found",
                            message=f"channel {channel_id} not found",
                            idempotency_key=idempotency_key,
                        )
                    message = await channel.send(content=content)
                else:
                    user = await self._resolve_user(user_id)
                    if user is None:
                        return self._error_response(
                            request=request,
                            status=404,
                            code="not_found",
                            message=f"user {user_id} not found",
                            idempotency_key=idempotency_key,
                        )
                    dm_channel = getattr(user, "dm_channel", None)
                    if dm_channel is None and hasattr(user, "create_dm"):
                        dm_channel = await user.create_dm()
                    if dm_channel is None or not hasattr(dm_channel, "send"):
                        return self._error_response(
                            request=request,
                            status=502,
                            code="discord_error",
                            message="failed to open DM channel",
                            idempotency_key=idempotency_key,
                        )
                    message = await dm_channel.send(content=content)
                    channel_id_value = int(getattr(dm_channel, "id", 0) or 0) or None
            except Exception as exc:
                logger.error(
                    "Master broker send_message failed (channel=%s user=%s): %s",
                    channel_id,
                    user_id,
                    exc,
                )
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="failed to send message",
                    idempotency_key=idempotency_key,
                )

            result = {
                "channel_id": channel_id if channel_id is not None else channel_id_value,
                "user_id": user_id,
                "message_id": int(getattr(message, "id", 0) or 0),
            }
            return self._success_response(
                request=request,
                idempotency_key=idempotency_key,
                result=result,
            )

        return await self._run_idempotent_action(
            request=request,
            action="discord.send_message",
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            operation=_operation,
        )

    async def _handle_create_channel(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            idempotency_key = self._extract_idempotency_key(request, payload)
            name = str(payload.get("name") or "").strip()
            if not name:
                raise ValueError("name is required")
            if len(name) > 100:
                raise ValueError("name exceeds Discord limit (100)")
            category_id = self._parse_positive_payload_int(payload, "category_id")
            topic = str(payload.get("topic") or "").strip() or None
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        payload_hash = self._payload_hash(
            {"name": name, "category_id": category_id, "topic": topic}
        )

        async def _operation() -> web.Response:
            category = await self._resolve_channel(category_id)
            if category is None or getattr(category, "guild", None) is None:
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"category {category_id} not found",
                    idempotency_key=idempotency_key,
                )

            guild = getattr(category, "guild", None)
            if guild is None or not hasattr(guild, "create_text_channel"):
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="guild unavailable for channel creation",
                    idempotency_key=idempotency_key,
                )

            try:
                channel = await guild.create_text_channel(name=name, category=category, topic=topic)
            except Exception as exc:
                logger.error(
                    "Master broker create_channel failed (category=%s name=%s): %s",
                    category_id,
                    name,
                    exc,
                )
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="failed to create channel",
                    idempotency_key=idempotency_key,
                )

            result = {
                "channel_id": int(getattr(channel, "id", 0) or 0),
                "category_id": category_id,
                "name": name,
            }
            return self._success_response(
                request=request,
                idempotency_key=idempotency_key,
                result=result,
            )

        return await self._run_idempotent_action(
            request=request,
            action="discord.create_channel",
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            operation=_operation,
        )

    async def _handle_delete_channel(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            idempotency_key = self._extract_idempotency_key(request, payload)
            channel_id = self._parse_positive_payload_int(payload, "channel_id")
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        payload_hash = self._payload_hash({"channel_id": channel_id})

        async def _operation() -> web.Response:
            channel = await self._resolve_channel(channel_id)
            if channel is None or not hasattr(channel, "delete"):
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"channel {channel_id} not found",
                    idempotency_key=idempotency_key,
                )

            try:
                await channel.delete()
            except Exception as exc:
                logger.error(
                    "Master broker delete_channel failed (channel=%s): %s", channel_id, exc
                )
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="failed to delete channel",
                    idempotency_key=idempotency_key,
                )

            return self._success_response(
                request=request,
                idempotency_key=idempotency_key,
                result={"channel_id": channel_id},
            )

        return await self._run_idempotent_action(
            request=request,
            action="discord.delete_channel",
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            operation=_operation,
        )

    async def _handle_send_rich_message(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            idempotency_key = self._extract_idempotency_key(request, payload)
            rich_payload = self._parse_rich_message_payload(payload)
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        allowlist_rejected = self._allowlist_check(
            request=request,
            idempotency_key=idempotency_key,
            scope="channel",
            value=rich_payload.channel_id,
            enabled=self._channel_allowlist_enabled,
            allowed_ids=self._allowed_channel_ids,
        )
        if allowlist_rejected is not None:
            return allowlist_rejected

        for role_id in rich_payload.allowed_role_ids:
            role_allowlist_rejected = self._allowlist_check(
                request=request,
                idempotency_key=idempotency_key,
                scope="role",
                value=role_id,
                enabled=self._role_allowlist_enabled,
                allowed_ids=self._allowed_role_ids,
            )
            if role_allowlist_rejected is not None:
                return role_allowlist_rejected

        operation_payload = {
            "channel_id": rich_payload.channel_id,
            "content": rich_payload.content,
            "embed": rich_payload.embed_dict,
            "allowed_user_ids": rich_payload.allowed_user_ids,
            "allowed_role_ids": rich_payload.allowed_role_ids,
            "view_spec": rich_payload.view_spec,
        }
        payload_hash = self._payload_hash(operation_payload)

        async def _operation() -> web.Response:
            channel = await self._resolve_channel(rich_payload.channel_id)
            if channel is None or not hasattr(channel, "send"):
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"channel {rich_payload.channel_id} not found",
                    idempotency_key=idempotency_key,
                )

            resolved_view = await self._resolve_broker_view(
                request=request,
                idempotency_key=idempotency_key,
                view_spec=rich_payload.view_spec,
            )
            if isinstance(resolved_view, web.Response):
                return resolved_view

            try:
                message = await channel.send(
                    content=rich_payload.content,
                    embed=rich_payload.embed,
                    view=resolved_view.view,
                    allowed_mentions=rich_payload.allowed_mentions,
                )
            except Exception as exc:
                logger.error(
                    "Master broker send_rich_message failed (channel=%s): %s",
                    rich_payload.channel_id,
                    exc,
                )
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="failed to send message",
                    idempotency_key=idempotency_key,
                )

            message_id = int(getattr(message, "id", 0) or 0)
            await self._bind_and_register_view(
                view=resolved_view.view,
                channel_id=rich_payload.channel_id,
                message_id=message_id,
                should_register=resolved_view.should_register,
            )
            return self._success_response(
                request=request,
                idempotency_key=idempotency_key,
                result={
                    "channel_id": rich_payload.channel_id,
                    "message_id": message_id,
                },
            )

        return await self._run_idempotent_action(
            request=request,
            action="discord.send_rich_message",
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            operation=_operation,
        )

    async def _handle_edit_rich_message(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            idempotency_key = self._extract_idempotency_key(request, payload)
            rich_payload = self._parse_rich_message_payload(payload)
            message_id = self._parse_positive_payload_int(payload, "message_id")
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        allowlist_rejected = self._allowlist_check(
            request=request,
            idempotency_key=idempotency_key,
            scope="channel",
            value=rich_payload.channel_id,
            enabled=self._channel_allowlist_enabled,
            allowed_ids=self._allowed_channel_ids,
        )
        if allowlist_rejected is not None:
            return allowlist_rejected

        for role_id in rich_payload.allowed_role_ids:
            role_allowlist_rejected = self._allowlist_check(
                request=request,
                idempotency_key=idempotency_key,
                scope="role",
                value=role_id,
                enabled=self._role_allowlist_enabled,
                allowed_ids=self._allowed_role_ids,
            )
            if role_allowlist_rejected is not None:
                return role_allowlist_rejected

        operation_payload = {
            "channel_id": rich_payload.channel_id,
            "message_id": message_id,
            "content": rich_payload.content,
            "embed": rich_payload.embed_dict,
            "allowed_user_ids": rich_payload.allowed_user_ids,
            "allowed_role_ids": rich_payload.allowed_role_ids,
            "view_spec": rich_payload.view_spec,
        }
        payload_hash = self._payload_hash(operation_payload)

        async def _operation() -> web.Response:
            channel = await self._resolve_channel(rich_payload.channel_id)
            if channel is None:
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"channel {rich_payload.channel_id} not found",
                    idempotency_key=idempotency_key,
                )

            message = await self._resolve_message(channel=channel, message_id=message_id)
            if message is None or not hasattr(message, "edit"):
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"message {message_id} not found",
                    idempotency_key=idempotency_key,
                )

            resolved_view = await self._resolve_broker_view(
                request=request,
                idempotency_key=idempotency_key,
                view_spec=rich_payload.view_spec,
            )
            if isinstance(resolved_view, web.Response):
                return resolved_view

            try:
                await message.edit(
                    content=rich_payload.content,
                    embed=rich_payload.embed,
                    view=resolved_view.view,
                    allowed_mentions=rich_payload.allowed_mentions,
                )
            except Exception as exc:
                logger.error(
                    "Master broker edit_rich_message failed (channel=%s message=%s): %s",
                    rich_payload.channel_id,
                    message_id,
                    exc,
                )
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="failed to edit message",
                    idempotency_key=idempotency_key,
                )

            await self._bind_and_register_view(
                view=resolved_view.view,
                channel_id=rich_payload.channel_id,
                message_id=message_id,
                should_register=resolved_view.should_register,
            )
            return self._success_response(
                request=request,
                idempotency_key=idempotency_key,
                result={
                    "channel_id": rich_payload.channel_id,
                    "message_id": message_id,
                },
            )

        return await self._run_idempotent_action(
            request=request,
            action="discord.edit_rich_message",
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            operation=_operation,
        )

    async def _handle_add_role(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            idempotency_key = self._extract_idempotency_key(request, payload)
            guild_id = self._parse_positive_payload_int(payload, "guild_id")
            user_id = self._parse_positive_payload_int(payload, "user_id")
            role_id = self._parse_positive_payload_int(payload, "role_id")
            reason = str(payload.get("reason") or "").strip() or "master-broker:add-role"
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        operation_payload = {
            "guild_id": guild_id,
            "user_id": user_id,
            "role_id": role_id,
            "reason": reason,
        }
        guild_allowlist_rejected = self._allowlist_check(
            request=request,
            idempotency_key=idempotency_key,
            scope="guild",
            value=guild_id,
            enabled=self._guild_allowlist_enabled,
            allowed_ids=self._allowed_guild_ids,
        )
        if guild_allowlist_rejected is not None:
            return guild_allowlist_rejected

        role_allowlist_rejected = self._allowlist_check(
            request=request,
            idempotency_key=idempotency_key,
            scope="role",
            value=role_id,
            enabled=self._role_allowlist_enabled,
            allowed_ids=self._allowed_role_ids,
        )
        if role_allowlist_rejected is not None:
            return role_allowlist_rejected

        payload_hash = self._payload_hash(operation_payload)

        async def _operation() -> web.Response:
            guild = None
            try:
                guild = self.bot.get_guild(guild_id)
            except Exception:
                guild = None

            if guild is None:
                fetch_guild = getattr(self.bot, "fetch_guild", None)
                if callable(fetch_guild):
                    try:
                        guild = await fetch_guild(guild_id)
                    except Exception:
                        guild = None

            if guild is None:
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"guild {guild_id} not found",
                    idempotency_key=idempotency_key,
                )

            try:
                role = guild.get_role(role_id)
            except Exception:
                role = None
            if role is None:
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"role {role_id} not found",
                    idempotency_key=idempotency_key,
                )

            try:
                member = guild.get_member(user_id)
            except Exception:
                member = None
            if member is None and hasattr(guild, "fetch_member"):
                try:
                    member = await guild.fetch_member(user_id)
                except Exception:
                    member = None
            if member is None:
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"member {user_id} not found",
                    idempotency_key=idempotency_key,
                )

            try:
                await member.add_roles(role, reason=reason)
            except Exception as exc:
                logger.error(
                    "Master broker add_role failed (guild=%s user=%s role=%s): %s",
                    guild_id,
                    user_id,
                    role_id,
                    exc,
                )
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="failed to add role",
                    idempotency_key=idempotency_key,
                )

            result = {
                "guild_id": guild_id,
                "user_id": user_id,
                "role_id": role_id,
            }
            return self._success_response(
                request=request,
                idempotency_key=idempotency_key,
                result=result,
            )

        return await self._run_idempotent_action(
            request=request,
            action="discord.add_role",
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            operation=_operation,
        )

    async def _handle_move_voice(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            idempotency_key = self._extract_idempotency_key(request, payload)
            guild_id = self._parse_positive_payload_int(payload, "guild_id")
            user_id = self._parse_positive_payload_int(payload, "user_id")
            channel_id_raw = payload.get("channel_id")
            channel_id = (
                self._parse_positive_payload_int(payload, "channel_id")
                if channel_id_raw is not None
                else None
            )
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        operation_payload = {
            "guild_id": guild_id,
            "user_id": user_id,
            "channel_id": channel_id,
        }
        guild_allowlist_rejected = self._allowlist_check(
            request=request,
            idempotency_key=idempotency_key,
            scope="guild",
            value=guild_id,
            enabled=self._guild_allowlist_enabled,
            allowed_ids=self._allowed_guild_ids,
        )
        if guild_allowlist_rejected is not None:
            return guild_allowlist_rejected

        if channel_id is not None:
            channel_allowlist_rejected = self._allowlist_check(
                request=request,
                idempotency_key=idempotency_key,
                scope="channel",
                value=channel_id,
                enabled=self._channel_allowlist_enabled,
                allowed_ids=self._allowed_channel_ids,
            )
            if channel_allowlist_rejected is not None:
                return channel_allowlist_rejected

        payload_hash = self._payload_hash(operation_payload)

        async def _operation() -> web.Response:
            guild = None
            try:
                guild = self.bot.get_guild(guild_id)
            except Exception:
                guild = None

            if guild is None:
                fetch_guild = getattr(self.bot, "fetch_guild", None)
                if callable(fetch_guild):
                    try:
                        guild = await fetch_guild(guild_id)
                    except Exception:
                        guild = None

            if guild is None:
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"guild {guild_id} not found",
                    idempotency_key=idempotency_key,
                )

            try:
                member = guild.get_member(user_id)
            except Exception:
                member = None
            if member is None and hasattr(guild, "fetch_member"):
                try:
                    member = await guild.fetch_member(user_id)
                except Exception:
                    member = None
            if member is None:
                return self._error_response(
                    request=request,
                    status=404,
                    code="not_found",
                    message=f"member {user_id} not found",
                    idempotency_key=idempotency_key,
                )

            channel = None
            if channel_id is not None:
                try:
                    channel = guild.get_channel(channel_id)
                except Exception:
                    channel = None
                if channel is None:
                    channel = await self._resolve_channel(channel_id)
                    channel_guild = getattr(channel, "guild", None)
                    if channel is None or getattr(channel_guild, "id", None) != guild_id:
                        channel = None
                if channel is None:
                    return self._error_response(
                        request=request,
                        status=404,
                        code="not_found",
                        message=f"channel {channel_id} not found",
                        idempotency_key=idempotency_key,
                    )

            try:
                await member.move_to(channel)
            except Exception as exc:
                logger.error(
                    "Master broker move_voice failed (guild=%s user=%s channel=%s): %s",
                    guild_id,
                    user_id,
                    channel_id,
                    exc,
                )
                return self._error_response(
                    request=request,
                    status=502,
                    code="discord_error",
                    message="failed to move member",
                    idempotency_key=idempotency_key,
                )

            return self._success_response(
                request=request,
                idempotency_key=idempotency_key,
                result={
                    "guild_id": guild_id,
                    "user_id": user_id,
                    "channel_id": channel_id,
                },
            )

        return await self._run_idempotent_action(
            request=request,
            action="discord.move_voice",
            idempotency_key=idempotency_key,
            payload_hash=payload_hash,
            operation=_operation,
        )

    async def _handle_get_voice_members(self, request: web.Request) -> web.Response:
        rejected = self._authorize(request)
        if rejected is not None:
            return rejected

        try:
            payload = await self._read_json_object(request)
            channel_id = self._parse_positive_payload_int(payload, "channel_id")
        except ValueError as exc:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=str(exc),
            )
        except Exception:
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message="invalid JSON payload",
            )

        channel_allowlist_rejected = self._allowlist_check(
            request=request,
            scope="channel",
            value=channel_id,
            enabled=self._channel_allowlist_enabled,
            allowed_ids=self._allowed_channel_ids,
        )
        if channel_allowlist_rejected is not None:
            return channel_allowlist_rejected

        channel = await self._resolve_channel(channel_id)
        if channel is None:
            return self._error_response(
                request=request,
                status=404,
                code="not_found",
                message=f"channel {channel_id} not found",
            )

        members = getattr(channel, "members", None)
        if not isinstance(members, list):
            return self._error_response(
                request=request,
                status=400,
                code="bad_request",
                message=f"channel {channel_id} does not expose voice members",
            )

        return self._success_response(
            request=request,
            result={
                "channel_id": channel_id,
                "members": [
                    {
                        "user_id": member.id,
                        "display_name": member.display_name,
                    }
                    for member in members
                ],
            },
        )


__all__ = ["MasterBroker"]
