from __future__ import annotations

import asyncio
import datetime as _dt
import errno
import html
import ipaddress
import json
import logging
import math
import os
import secrets
import sqlite3
import subprocess
import time
from collections.abc import Iterable
from pathlib import Path
from typing import TYPE_CHECKING, Any
from urllib.parse import parse_qsl, urlencode, urlparse, urlunparse

from aiohttp import ClientSession, ClientTimeout, web

from service import db

logger = logging.getLogger(__name__)

DEFAULT_RETENTION_EXCLUDED_ROLE_IDS = {
    1304416311383818240,
    1309741866098491479,
}

LOG_TAIL_DEFAULT_LINES = 400
LOG_TAIL_MAX_LINES = 5000
LOG_TAIL_MAX_BYTES = 4 * 1024 * 1024
DEFAULT_DASHBOARD_MODERATOR_ROLE_ID = 1337518124647579661
TURNIER_MOD_ROLE_ID = 1401891955931222110  # Community-Moderator: nur Turnier-Zugriff
DEFAULT_DASHBOARD_OWNER_USER_ID = 662995601738170389
DEADLOCK_MISSING_BUILD_ALERT_CHANNEL_ID = 1374364800817303632
DEADLOCK_MISSING_BUILD_ALERT_INTERVAL_SECONDS = 300
KEYRING_SERVICE_NAME = "DeadlockBot"
MASTER_DASHBOARD_PUBLIC_URL = "https://admin.deutsche-deadlock-community.de"
MASTER_DASHBOARD_DISCORD_REDIRECT_URI = "https://deutsche-deadlock-community.de/callback/discord"

# Feste Steam-URLs (keine Ableitung/ENV mehr)
MASTER_DASHBOARD_STEAM_PUBLIC_URL = "https://deutsche-deadlock-community.de"
MASTER_DASHBOARD_STEAM_RETURN_URL = "https://deutsche-deadlock-community.de/link/steam/return"
MASTER_DASHBOARD_STEAM_RETURN_PATH = "/steam/return"

MASTER_DASHBOARD_DEFAULT_SCHEME = "http"
DISCORD_API_BASE_URL = "https://discord.com/api/v10"
MASTER_DISCORD_ADMIN_LOGIN_URL = "/auth/discord/login?next=%2Fadmin"
TURNIER_INTERNAL_API_TOKEN_ENV_NAMES = (
    "TURNIER_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
    "TWITCH_INTERNAL_API_TOKEN",
)
TWITCH_INTERNAL_API_TOKEN_ENV_NAMES = ("TWITCH_INTERNAL_API_TOKEN",)
DEFAULT_NSSM_SERVICE_NAME = KEYRING_SERVICE_NAME
DEFAULT_NSSM_RESTART_DELAY_SECONDS = 1.0
DEFAULT_BOT_RESTART_MIN_INTERVAL_SECONDS = 15.0
DEFAULT_DASHBOARD_SESSION_TTL_SECONDS = 6 * 3600
DEFAULT_DISCORD_OAUTH_STATE_TTL_SECONDS = 6 * 3600
NSSM_PATH_CANDIDATES = (
    r"C:\ProgramData\chocolatey\lib\NSSM\tools\nssm.exe",
    r"C:\ProgramData\chocolatey\bin\nssm.exe",
    r"C:\nssm\win64\nssm.exe",
    r"C:\nssm\nssm.exe",
)
POWERSHELL_PATH_CANDIDATES = (r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",)
SC_PATH_CANDIDATES = (r"C:\Windows\System32\sc.exe",)

try:
    from service.standalone_manager import (
        StandaloneAlreadyRunning,
        StandaloneConfigNotFound,
        StandaloneManagerError,
        StandaloneNotRunning,
    )
except Exception:
    StandaloneAlreadyRunning = StandaloneConfigNotFound = StandaloneManagerError = (
        StandaloneNotRunning
    ) = None  # type: ignore


_DASHBOARD_HTML_PATH = Path(__file__).resolve().parent / "static" / "dashboard.html"
_TURNIER_HTML_PATH = Path(__file__).resolve().parent / "static" / "turnier.html"


def _load_index_html() -> str:
    """
    Lädt das Dashboard-HTML aus service/static/dashboard.html.
    Kein Fallback, kein Caching: Fehlender/defekter File => 500.
    """
    try:
        return _DASHBOARD_HTML_PATH.read_text(encoding="utf-8")
    except Exception as exc:
        logging.getLogger(__name__).error(
            "Dashboard HTML konnte nicht geladen werden (%s): %s",
            _DASHBOARD_HTML_PATH,
            exc,
            exc_info=True,
        )
        raise


class DashboardServer:
    """Simple aiohttp based dashboard for managing the master bot."""

    def __init__(
        self,
        bot: MasterBot,
        *,
        host: str = "127.0.0.1",
        port: int = 8766,
    ) -> None:
        self.bot = bot
        self.host = host
        self.port = port
        self._runner: web.AppRunner | None = None
        self._site: web.TCPSite | None = None
        self._lock = asyncio.Lock()
        self._started = False
        self._restart_lock = asyncio.Lock()
        self._restart_task: asyncio.Task | None = None
        self._last_restart: dict[str, Any] = {"at": None, "ok": None, "error": None}
        self._lifecycle = getattr(bot, "lifecycle", None)
        self._nssm_service_name = (
            os.getenv("MASTER_NSSM_SERVICE_NAME") or DEFAULT_NSSM_SERVICE_NAME
        ).strip()
        self._nssm_executable = (os.getenv("MASTER_NSSM_EXE") or "").strip()
        self._powershell_executable = (os.getenv("MASTER_POWERSHELL_EXE") or "").strip()
        nssm_enabled_raw = (os.getenv("MASTER_NSSM_RESTART_ENABLED") or "1").strip().lower()
        self._nssm_service_restart_enabled = nssm_enabled_raw not in {
            "0",
            "false",
            "no",
            "off",
        }
        self._nssm_restart_delay_seconds = self._parse_positive_float(
            os.getenv(
                "MASTER_NSSM_RESTART_DELAY_SECONDS",
                str(DEFAULT_NSSM_RESTART_DELAY_SECONDS),
            ),
            default=DEFAULT_NSSM_RESTART_DELAY_SECONDS,
            env_name="MASTER_NSSM_RESTART_DELAY_SECONDS",
        )
        self._systemd_service_name = os.getenv(
            "MASTER_SYSTEMD_SERVICE_NAME", "deadlock-bot"
        ).strip()
        self._bot_restart_min_interval_seconds = self._parse_positive_float(
            os.getenv(
                "MASTER_BOT_RESTART_MIN_INTERVAL_SECONDS",
                str(DEFAULT_BOT_RESTART_MIN_INTERVAL_SECONDS),
            ),
            default=DEFAULT_BOT_RESTART_MIN_INTERVAL_SECONDS,
            env_name="MASTER_BOT_RESTART_MIN_INTERVAL_SECONDS",
        )
        self._bot_restart_lock = asyncio.Lock()
        self._last_bot_restart_request_monotonic = 0.0
        keyring_client_id = self._read_keyring_secret("DISCORD_OAUTH_CLIENT_ID")
        app_client_id = str(getattr(bot, "application_id", "") or "").strip()
        self._discord_client_id = (keyring_client_id or app_client_id).strip()
        self._discord_client_secret = self._read_keyring_secret(
            "DISCORD_OAUTH_CLIENT_SECRET"
        ).strip()
        self._discord_redirect_uri = MASTER_DASHBOARD_DISCORD_REDIRECT_URI
        self._turnier_internal_api_token = self._resolve_turnier_internal_api_token()
        self._twitch_internal_api_token = self._resolve_twitch_internal_api_token()
        self._discord_auth_enabled = True
        self._discord_owner_user_id = DEFAULT_DASHBOARD_OWNER_USER_ID
        self._discord_moderator_role_id = DEFAULT_DASHBOARD_MODERATOR_ROLE_ID
        self._discord_auth_guild_ids: tuple[int, ...] = ()
        self._discord_session_cookie = "master_dash_session"
        self._discord_sessions: dict[str, dict[str, Any]] = {}
        self._discord_oauth_states: dict[str, dict[str, Any]] = {}
        self._auth_rate_limits: dict[str, list[float]] = {}
        self._discord_oauth_state_ttl = int(
            self._parse_positive_float(
                os.getenv(
                    "MASTER_DASHBOARD_OAUTH_STATE_TTL_SEC",
                    str(DEFAULT_DISCORD_OAUTH_STATE_TTL_SECONDS),
                ),
                default=float(DEFAULT_DISCORD_OAUTH_STATE_TTL_SECONDS),
                env_name="MASTER_DASHBOARD_OAUTH_STATE_TTL_SEC",
            )
        )
        self._discord_session_ttl = max(
            DEFAULT_DASHBOARD_SESSION_TTL_SECONDS,
            int(
                self._parse_positive_float(
                    os.getenv(
                        "MASTER_DASHBOARD_SESSION_TTL_SEC",
                        str(DEFAULT_DASHBOARD_SESSION_TTL_SECONDS),
                    ),
                    default=float(DEFAULT_DASHBOARD_SESSION_TTL_SECONDS),
                    env_name="MASTER_DASHBOARD_SESSION_TTL_SEC",
                )
            ),
        )
        self._discord_auth_required = (
            self._discord_auth_enabled and self._is_discord_oauth_configured()
        )
        self._auth_misconfigured = False
        self._scheme = MASTER_DASHBOARD_DEFAULT_SCHEME
        self._listen_base_url = self._format_base_url(self.host, self.port, self._scheme)
        try:
            self._public_base_url = self._normalize_public_url(
                MASTER_DASHBOARD_PUBLIC_URL,
                default_scheme="https",
            )
        except Exception as exc:
            logging.warning(
                "Master dashboard public URL '%s' invalid (%s) - falling back to listen URL",
                MASTER_DASHBOARD_PUBLIC_URL,
                exc,
            )
            self._public_base_url = self._listen_base_url
        self._allowed_request_origins = self._build_allowed_request_origins()

        self._steam_return_url = self._derive_steam_return_url()
        self._health_cache: list[dict[str, Any]] = []
        self._health_cache_expiry = 0.0
        self._health_cache_lock = asyncio.Lock()
        self._health_cache_ttl = self._parse_positive_float(
            "30.0",
            default=30.0,
            env_name="DASHBOARD_HEALTHCHECK_CACHE_SECONDS",
        )
        self._health_timeout = self._parse_positive_float(
            "6.0",
            default=6.0,
            env_name="DASHBOARD_HEALTHCHECK_TIMEOUT_SECONDS",
        )
        self._health_targets = self._build_health_targets()
        self._log_dir = Path(__file__).resolve().parent.parent / "logs"
        self._public_stats_cache: dict | None = None
        self._public_stats_cache_time: float = 0.0
        self._deadlock_alert_task: asyncio.Task | None = None
        if self._discord_auth_enabled and not self._is_discord_oauth_configured():
            logging.error(
                "Dashboard Auth-Konfiguration ungültig: Discord OAuth aktiviert, aber Client ID/Secret "
                "nicht im Windows-Tresor (%s) vorhanden. Dashboard bleibt gesperrt.",
                KEYRING_SERVICE_NAME,
            )
            self._discord_auth_required = False
            self._auth_misconfigured = True
        if not self._discord_auth_required and not self._auth_misconfigured:
            logging.warning("Dashboard läuft ohne Auth (Discord OAuth deaktiviert).")

    @staticmethod
    def _sanitize(value: Any) -> Any:
        """Recursively normalise values so the JSON payload never emits NaN/Infinity."""
        if isinstance(value, dict):
            return {key: DashboardServer._sanitize(val) for key, val in value.items()}
        if isinstance(value, list):
            return [DashboardServer._sanitize(item) for item in value]
        if isinstance(value, tuple):
            return [DashboardServer._sanitize(item) for item in value]
        if isinstance(value, float) and not math.isfinite(value):
            return None
        return value

    @staticmethod
    def _safe_log_value(value: Any) -> str:
        """
        Sanitize values before logging to avoid log injection via crafted newlines.
        """
        text = "" if value is None else str(value)
        return text.replace("\r", "\\r").replace("\n", "\\n")

    @staticmethod
    def _validate_sql_identifier(name: str) -> str:
        """
        Validate that a string is a safe SQL identifier (column/table name).
        Only allows alphanumeric characters and underscores.
        Raises ValueError if the identifier is invalid.
        """
        if not name:
            raise ValueError("Empty SQL identifier")
        if not all(c.isalnum() or c == "_" for c in name):
            raise ValueError(f"Invalid SQL identifier: {name}")
        return name

    def _json(self, payload: Any, **kwargs: Any) -> web.Response:
        return web.json_response(self._sanitize(payload), **kwargs)

    @staticmethod
    def _normalize_deadlock_target_name(raw_value: Any, *, field_name: str) -> str:
        if raw_value is None:
            return ""
        if isinstance(raw_value, bool) or not isinstance(raw_value, str):
            raise web.HTTPBadRequest(text=f"{field_name} must be string")

        value = raw_value.strip()
        if not value:
            return ""
        if len(value) > 120:
            raise web.HTTPBadRequest(text=f"{field_name} must be at most 120 characters")
        if any(ord(ch) < 32 or ord(ch) == 127 for ch in value):
            raise web.HTTPBadRequest(text=f"{field_name} contains unsupported control characters")
        return value

    def _check_auth_rate_limit(
        self,
        request: web.Request,
        *,
        max_requests: int = 10,
        window_seconds: float = 60.0,
    ) -> bool:
        peer = self._peer_host(request) or "unknown"
        now = time.time()
        hits = self._auth_rate_limits.get(peer, [])
        hits = [t for t in hits if now - t < window_seconds]
        if len(hits) >= max_requests:
            self._auth_rate_limits[peer] = hits
            return False
        hits.append(now)
        self._auth_rate_limits[peer] = hits
        if len(self._auth_rate_limits) > 2000:
            self._auth_rate_limits.clear()
        return True

    async def _cleanup(self) -> None:
        if self._deadlock_alert_task:
            self._deadlock_alert_task.cancel()
            self._deadlock_alert_task = None
        if self._site:
            await self._site.stop()
        if self._runner:
            await self._runner.cleanup()
        self._site = None
        self._runner = None

    async def start(self) -> None:
        async with self._lock:
            if self._started:
                return

            @web.middleware
            async def _security_headers(request: web.Request, handler: Any) -> web.StreamResponse:
                response = await handler(request)
                response.headers.setdefault("X-Frame-Options", "DENY")
                response.headers.setdefault("X-Content-Type-Options", "nosniff")
                response.headers.setdefault("X-XSS-Protection", "1; mode=block")
                response.headers.setdefault("Referrer-Policy", "strict-origin-when-cross-origin")
                response.headers.setdefault(
                    "Permissions-Policy",
                    "geolocation=(), microphone=(), camera=(), payment=()",
                )
                return response

            app = web.Application(middlewares=[_security_headers])
            app["dashboard"] = self
            app.add_routes(
                [
                    web.get("/", self._handle_index),
                    web.get("/admin", self._handle_index),
                    web.get("/auth/discord/login", self._handle_discord_login),
                    web.get("/auth/discord/callback", self._handle_discord_callback),
                    web.get("/callback/discord", self._handle_callback_discord),
                    web.get("/auth/logout", self._handle_logout),
                    web.post("/auth/logout", self._handle_logout),
                    web.post("/internal/v1/discord/initiate", self._handle_discord_initiate),
                    web.post(
                        "/internal/v1/discord/consume-result",
                        self._handle_discord_consume_result,
                    ),
                    web.post(
                        "/internal/turnier/v1/discord/authorize-url",
                        self._handle_turnier_discord_authorize_url,
                    ),
                    web.post(
                        "/internal/turnier/v1/discord/session",
                        self._handle_turnier_discord_session,
                    ),
                    web.post(
                        "/internal/discord/v1/steam-link-session",
                        self._handle_steam_link_discord_session,
                    ),
                    web.post(
                        "/internal/twitch/v1/discord/authorize-url",
                        self._handle_twitch_discord_authorize_url,
                    ),
                    web.post(
                        "/internal/twitch/v1/discord/session",
                        self._handle_twitch_discord_session,
                    ),
                    web.post(
                        "/internal/twitch/v1/discord/validate-session",
                        self._handle_twitch_discord_validate_session,
                    ),
                    web.post(
                        "/internal/twitch/v1/discord/import-session",
                        self._handle_twitch_discord_import_session,
                    ),
                    web.get("/api/auth/me", self._handle_auth_me),
                    web.get("/api/status", self._handle_status),
                    web.post("/api/bot/restart", self._handle_bot_restart),
                    web.post("/api/dashboard/restart", self._handle_dashboard_restart),
                    web.post("/api/cogs/reload", self._handle_reload),
                    web.post("/api/cogs/load", self._handle_load),
                    web.post("/api/cogs/unload", self._handle_unload),
                    web.post("/api/cogs/reload-all", self._handle_reload_all),
                    web.post("/api/cogs/reload-namespace", self._handle_reload_namespace),
                    web.post("/api/cogs/block", self._handle_block),
                    web.post("/api/cogs/unblock", self._handle_unblock),
                    web.get("/api/voice-stats", self._handle_voice_stats),
                    web.get("/api/voice-history", self._handle_voice_history),
                    web.get("/api/user-retention", self._handle_user_retention),
                    web.get("/api/member-events", self._handle_member_events),
                    web.get("/api/message-activity", self._handle_message_activity),
                    web.get("/api/co-player-network", self._handle_co_player_network),
                    web.get("/api/co-player-network/", self._handle_co_player_network),
                    web.get("/api/server-stats", self._handle_server_stats),
                    web.get("/api/deadlock/config", self._handle_deadlock_config),
                    web.post("/api/deadlock/config", self._handle_deadlock_config_update),
                    web.get("/api/deadlock/heroes", self._handle_deadlock_heroes),
                    web.post("/api/deadlock/heroes", self._handle_deadlock_upsert_hero),
                    web.delete("/api/deadlock/heroes/{hero_id}", self._handle_deadlock_delete_hero),
                    web.post(
                        "/api/deadlock/heroes/{hero_id}/sync", self._handle_deadlock_sync_hero
                    ),
                    web.get("/api/tournament/overview", self._handle_tournament_overview),
                    web.post("/api/tournament/team", self._handle_tournament_team_create),
                    web.post("/api/tournament/assign", self._handle_tournament_assign),
                    web.post("/api/tournament/remove", self._handle_tournament_remove),
                    # Turnier-Seite (Community-Mods + Admins)
                    web.get("/turnier", self._handle_turnier_page),
                    web.get("/api/turnier/overview", self._handle_turnier_overview),
                    web.get("/api/turnier/bracket", self._handle_turnier_bracket),
                    web.post("/api/turnier/period", self._handle_turnier_period_create),
                    web.post("/api/turnier/period/close", self._handle_turnier_period_close),
                    web.post("/api/turnier/team", self._handle_turnier_team_create),
                    web.post("/api/turnier/team/delete", self._handle_turnier_team_delete),
                    web.post("/api/turnier/assign", self._handle_turnier_assign),
                    web.post("/api/turnier/remove", self._handle_turnier_remove),
                    web.post("/api/turnier/clear", self._handle_turnier_clear),
                    web.post("/api/cogs/discover", self._handle_discover),
                    web.get("/api/logs", self._handle_log_index),
                    web.get("/api/logs/{name}", self._handle_log_read),
                    web.get("/api/standalone", self._handle_standalone_list),
                    web.get("/api/standalone/{key}/logs", self._handle_standalone_logs),
                    web.post("/api/standalone/{key}/start", self._handle_standalone_start),
                    web.post("/api/standalone/{key}/stop", self._handle_standalone_stop),
                    web.post("/api/standalone/{key}/restart", self._handle_standalone_restart),
                    web.post(
                        "/api/standalone/{key}/autostart",
                        self._handle_standalone_autostart,
                    ),
                    web.post("/api/standalone/{key}/command", self._handle_standalone_command),
                    # Public endpoints (no auth required)
                    web.get("/api/public/guild-stats", self._handle_public_guild_stats),
                    web.route("OPTIONS", "/api/public/guild-stats", self._handle_public_cors),
                    web.get("/api/public/patch-notes", self._handle_public_patch_notes),
                    web.route("OPTIONS", "/api/public/patch-notes", self._handle_public_cors),
                ]
            )

            addr_in_use = {errno.EADDRINUSE, getattr(errno, "WSAEADDRINUSE", 10048)}
            win_access = {getattr(errno, "WSAEACCES", 10013), errno.EACCES}

            async def _start_with(reuse_address: bool | None) -> str:
                runner = web.AppRunner(app)
                await runner.setup()

                site_kwargs: dict[str, Any] = {}
                if reuse_address:
                    site_kwargs["reuse_address"] = True

                try:
                    site = web.TCPSite(runner, self.host, self.port, **site_kwargs)
                    await site.start()
                except OSError as e:
                    await runner.cleanup()
                    if reuse_address and os.name == "nt" and e.errno in win_access:
                        logging.warning(
                            "reuse_address konnte auf Windows nicht aktiviert werden (%s). "
                            "Starte Dashboard ohne reuse_address.",
                            e,
                        )
                        return "retry_without_reuse"
                    if e.errno in addr_in_use:
                        return "addr_in_use"
                    raise

                self._runner = runner
                self._site = site
                return "started"

            async def _start_without_reuse_with_retries() -> None:
                retries = 3
                delay = 0.5
                for attempt in range(retries):
                    attempt_result = await _start_with(reuse_address=False)
                    if attempt_result == "started":
                        return
                    if attempt_result == "addr_in_use" and attempt < retries - 1:
                        await asyncio.sleep(delay)
                        delay *= 2
                        continue
                    if attempt_result == "addr_in_use":
                        raise RuntimeError(
                            f"Dashboard-Port {self.host}:{self.port} ist bereits belegt"
                        )
                    raise RuntimeError("Dashboard konnte nicht gestartet werden")
                raise RuntimeError("Dashboard konnte nicht gestartet werden")

            if os.name != "nt":
                result = await _start_with(reuse_address=True)
                if result == "addr_in_use":
                    raise RuntimeError(f"Dashboard-Port {self.host}:{self.port} ist bereits belegt")
                if result != "started":
                    raise RuntimeError("Dashboard konnte nicht gestartet werden")
            else:
                result = await _start_with(reuse_address=True)
                if result == "started":
                    pass
                elif result == "retry_without_reuse":
                    await _start_without_reuse_with_retries()
                elif result == "addr_in_use":
                    # reuse_address hat trotzdem einen Konflikt ausgelöst – wir warten
                    # kurz und versuchen den Start ohne reuse_address erneut.
                    await asyncio.sleep(0.5)
                    await _start_without_reuse_with_retries()
                else:
                    raise RuntimeError("Dashboard konnte nicht gestartet werden")

            self._started = True
            if self._deadlock_alert_task is None:
                self._deadlock_alert_task = asyncio.create_task(
                    self._deadlock_missing_build_alert_loop()
                )
            base_no_slash = self._public_base_url.rstrip("/")
            if base_no_slash.lower().endswith("/admin"):
                admin_path = base_no_slash
            else:
                admin_path = base_no_slash + "/admin"
            logging.info("Master dashboard listening on %s", self._listen_base_url)
            if self._public_base_url != self._listen_base_url:
                logging.info("Master dashboard public URL set to %s", self._public_base_url)
            logging.info("Master dashboard admin UI: %s", admin_path)

    async def stop(self) -> None:
        async with self._lock:
            if not self._started:
                return
            try:
                await self._cleanup()
            finally:
                self._started = False
                logging.info("Master dashboard stopped")

    async def _restart_dashboard(self) -> dict[str, Any]:
        # Allow the response to be flushed before we tear the server down.
        await asyncio.sleep(0.25)
        stop_error: str | None = None
        try:
            await self.stop()
        except Exception as exc:  # pragma: no cover - defensive restart path
            stop_error = str(exc)
            logging.exception("Stopping dashboard before restart failed: %s", exc)

        await asyncio.sleep(0.1)

        try:
            await self.start()
            result: dict[str, Any] = {
                "ok": stop_error is None,
                "listen_url": self._listen_base_url,
                "public_url": self._public_base_url,
            }
            if stop_error:
                result["error"] = stop_error
        except Exception as exc:  # pragma: no cover - defensive restart path
            logging.exception("Dashboard start failed during restart: %s", exc)
            result = {"ok": False, "error": str(exc)}

        self._last_restart = {
            "ok": result.get("ok"),
            "error": result.get("error"),
            "at": _dt.datetime.utcnow().isoformat() + "Z",
        }
        return result

    def _on_restart_finished(self, task: asyncio.Task) -> None:
        try:
            result = task.result()
            if isinstance(result, dict) and not result.get("ok", True):
                logging.warning("Dashboard restart finished with errors: %s", result.get("error"))
            else:
                logging.info("Dashboard restart completed")
        except Exception:  # pragma: no cover - defensive restart path
            logging.exception("Dashboard restart task crashed")
        finally:
            self._restart_task = None

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------
    @staticmethod
    def _read_keyring_secret(key: str) -> str:
        secret_key = (key or "").strip()
        if not secret_key:
            return ""
        env_value = (os.getenv(secret_key) or "").strip()
        if env_value:
            return env_value
        try:
            import keyring
        except Exception:
            return ""
        try:
            value = keyring.get_password(KEYRING_SERVICE_NAME, secret_key)
            if not value:
                value = keyring.get_password(f"{secret_key}@{KEYRING_SERVICE_NAME}", secret_key)
        except Exception:
            return ""
        return str(value or "").strip()

    @staticmethod
    def _resolve_turnier_internal_api_token() -> str:
        for env_name in TURNIER_INTERNAL_API_TOKEN_ENV_NAMES:
            token = (os.getenv(env_name) or "").strip()
            if token:
                return token
        return ""

    @staticmethod
    def _resolve_twitch_internal_api_token() -> str:
        for env_name in TWITCH_INTERNAL_API_TOKEN_ENV_NAMES:
            token = (os.getenv(env_name) or "").strip()
            if token:
                return token
        return ""

    def _is_discord_oauth_configured(self) -> bool:
        return bool(
            self._discord_client_id and self._discord_client_secret and self._discord_redirect_uri
        )

    def _require_turnier_internal_api_access(self, request: web.Request) -> None:
        peer = self._peer_host(request)
        if not self._is_loopback_host(peer):
            raise web.HTTPForbidden(text="loopback access required")

        expected = str(self._turnier_internal_api_token or "").strip()
        if not expected:
            raise web.HTTPServiceUnavailable(text="turnier internal api token not configured")

        presented = (request.headers.get("X-Internal-Token") or "").strip()
        if not presented or not secrets.compare_digest(presented, expected):
            raise web.HTTPUnauthorized(text="missing or invalid X-Internal-Token")

    def _require_twitch_internal_api_access(self, request: web.Request) -> None:
        peer = self._peer_host(request)
        if not self._is_loopback_host(peer):
            raise web.HTTPForbidden(text="loopback access required")

        expected = str(self._twitch_internal_api_token or "").strip()
        if not expected:
            raise web.HTTPServiceUnavailable(text="twitch internal api token not configured")

        presented = (request.headers.get("X-Internal-Token") or "").strip()
        if not presented or not secrets.compare_digest(presented, expected):
            raise web.HTTPUnauthorized(text="missing or invalid X-Internal-Token")

    def _require_any_internal_api_access(self, request: web.Request) -> None:
        peer = self._peer_host(request)
        if not self._is_loopback_host(peer):
            raise web.HTTPForbidden(text="loopback access required")

        presented = (request.headers.get("X-Internal-Token") or "").strip()
        if not presented:
            raise web.HTTPUnauthorized(text="missing or invalid X-Internal-Token")

        expected_tokens = [
            str(self._turnier_internal_api_token or "").strip(),
            str(self._twitch_internal_api_token or "").strip(),
        ]
        configured_tokens = [token for token in expected_tokens if token]
        if not configured_tokens:
            raise web.HTTPServiceUnavailable(text="internal api token not configured")
        if not any(secrets.compare_digest(presented, expected) for expected in configured_tokens):
            raise web.HTTPUnauthorized(text="missing or invalid X-Internal-Token")

    def _is_auth_enforced(self) -> bool:
        return bool(self._discord_auth_required or self._auth_misconfigured)

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
        elif ":" in value:
            host_part, port_part = value.rsplit(":", 1)
            if port_part.isdigit():
                value = host_part
        return value.lower()

    @staticmethod
    def _is_loopback_host(raw: str | None) -> bool:
        host = DashboardServer._host_without_port(raw)
        if not host:
            return False
        if host == "localhost":
            return True
        try:
            return ipaddress.ip_address(host).is_loopback
        except ValueError:
            return False

    @staticmethod
    def _peer_host(request: web.Request) -> str:
        remote = (request.remote or "").strip() if hasattr(request, "remote") else ""
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

    def _is_secure_request(self, request: web.Request) -> bool:
        peer = self._peer_host(request)
        if self._is_loopback_host(peer):
            forwarded_proto = (
                (request.headers.get("X-Forwarded-Proto") or "").split(",")[0].strip().lower()
            )
            if forwarded_proto:
                return forwarded_proto == "https"
        return bool(request.secure)

    @staticmethod
    def _normalize_auth_next_path(raw: str | None) -> str:
        fallback = "/admin"
        candidate = (raw or "").strip()
        if not candidate:
            return fallback
        if "\r" in candidate or "\n" in candidate:
            return fallback

        # Normalize backslashes (some browsers treat them as forward slashes)
        candidate = candidate.replace("\\", "/")

        try:
            parsed = urlparse(candidate)
        except Exception:
            return fallback

        # CodeQL: URL redirection from remote source.
        # Strict validation of path-only redirects.
        if parsed.scheme or parsed.netloc:
            return fallback
        if not candidate.startswith("/"):
            return fallback
        if candidate.startswith("//"):
            return fallback

        # Normalize path to collapse .. and .
        path_parts = []
        for p in parsed.path.split("/"):
            if p == "..":
                if path_parts:
                    path_parts.pop()
            elif p == "." or not p:
                continue
            else:
                path_parts.append(p)

        norm_path = "/" + "/".join(path_parts)
        if parsed.path.endswith("/") and norm_path != "/":
            norm_path += "/"

        # Final check for double slash after normalization
        if norm_path.startswith("//"):
            return fallback

        # Allowed destination prefixes
        allowed = ("/admin", "/api/", "/turnier", "/auth/")
        if any(norm_path.startswith(p) for p in allowed):
            # Reconstruct with original query/fragment
            res = norm_path
            if parsed.query:
                res += "?" + parsed.query
            if parsed.fragment:
                res += "#" + parsed.fragment
            return res

        return fallback

    @staticmethod
    def _safe_internal_redirect(location: str | None, *, fallback: str = "/admin") -> str:
        candidate = (location or "").strip()
        if not candidate:
            return fallback
        if "\r" in candidate or "\n" in candidate:
            return fallback

        # Normalize backslashes
        candidate = candidate.replace("\\", "/")

        try:
            parsed = urlparse(candidate)
        except Exception:
            return fallback

        # CodeQL: URL redirection from remote source.
        if parsed.scheme or parsed.netloc:
            return fallback
        if not candidate.startswith("/"):
            return fallback
        if candidate.startswith("//"):
            return fallback

        # Path traversal check
        path_parts = parsed.path.split("/")
        if ".." in path_parts or "." in path_parts:
            # We already have norm_path logic in _normalize_auth_next_path,
            # for general internal redirects we just reject '..' for simplicity.
            return fallback

        return candidate

    @staticmethod
    def _safe_template_href(location: str | None, *, fallback: str = "/admin") -> str:
        candidate = (location or "").strip()
        if not candidate:
            return fallback
        # Normalize backslashes
        candidate = candidate.replace("\\", "/")

        if any(ch in candidate for ch in {"\r", "\n", "<", ">", '"', "'"}):
            return fallback
        try:
            parsed = urlparse(candidate)
        except Exception:
            return fallback
        scheme = (parsed.scheme or "").strip().lower()
        if scheme:
            if scheme not in {"http", "https"}:
                return fallback
            if not parsed.netloc:
                return fallback
            return candidate
        if parsed.netloc or not candidate.startswith("/"):
            return fallback
        if candidate.startswith("//"):
            return fallback
        # Path traversal check
        if ".." in parsed.path.split("/"):
            return fallback
        return candidate

    def _public_dashboard_redirect_url(
        self, location: str | None, *, fallback: str = "/admin"
    ) -> str:
        safe_path = self._safe_internal_redirect(location, fallback=fallback)
        base_url = str(self._public_base_url or "").strip().rstrip("/")
        if not base_url:
            return safe_path
        return f"{base_url}{safe_path}"

    def _build_discord_login_url(
        self, request: web.Request, *, next_path: str | None = None
    ) -> str:
        if not self._discord_auth_required:
            return "/admin"

        # Use provided next_path OR extract from current request
        if next_path:
            raw_target = next_path
        else:
            raw_target = request.rel_url.path_qs if (request and request.rel_url) else "/admin"

        # Strict validation of the next parameter
        normalized = self._normalize_auth_next_path(raw_target)

        # CodeQL: URL redirection from remote source.
        # Ensure we only return a path relative to our own /auth/discord/login
        query = urlencode({"next": normalized})
        return f"/auth/discord/login?{query}"

    @staticmethod
    def _normalize_origin(raw: str | None) -> str | None:
        value = (raw or "").strip()
        if not value:
            return None
        try:
            parsed = urlparse(value)
        except Exception:
            return None
        scheme = (parsed.scheme or "").strip().lower()
        netloc = (parsed.netloc or "").strip().lower()
        if scheme not in {"http", "https"} or not netloc:
            return None
        return f"{scheme}://{netloc}"

    def _build_allowed_request_origins(self) -> set[str]:
        origins: set[str] = set()
        for base_url in (self._public_base_url, self._listen_base_url):
            normalized = self._normalize_origin(base_url)
            if normalized:
                origins.add(normalized)

        extra_raw = (os.getenv("MASTER_DASHBOARD_ALLOWED_ORIGINS") or "").strip()
        if extra_raw:
            for part in extra_raw.split(","):
                normalized = self._normalize_origin(part)
                if normalized:
                    origins.add(normalized)

        return origins

    def _request_origin(self, request: web.Request) -> str | None:
        origin = self._normalize_origin(request.headers.get("Origin"))
        if origin:
            return origin

        referer = (request.headers.get("Referer") or "").strip()
        if not referer:
            return None
        return self._normalize_origin(referer)

    def _is_allowed_request_origin(self, request: web.Request) -> bool:
        if not self._allowed_request_origins:
            return True
        request_origin = self._request_origin(request)
        if not request_origin:
            return False
        return request_origin in self._allowed_request_origins

    @staticmethod
    def _requires_csrf_check(request: web.Request) -> bool:
        return request.method.upper() in {"POST", "PUT", "PATCH", "DELETE"}

    def _ensure_session_csrf_token(self, session: dict[str, Any]) -> str:
        token = str(session.get("csrf_token") or "").strip()
        if token:
            return token
        token = secrets.token_urlsafe(32)
        session["csrf_token"] = token
        return token

    def _check_csrf(self, request: web.Request, session: dict[str, Any]) -> bool:
        expected = self._ensure_session_csrf_token(session)
        provided = (request.headers.get("X-CSRF-Token") or "").strip()
        if not provided:
            return False
        try:
            return secrets.compare_digest(provided, expected)
        except Exception:
            return False

    def _cleanup_discord_auth_state(self) -> None:
        now = time.time()
        expired_states = [
            key
            for key, row in self._discord_oauth_states.items()
            if now - float(row.get("created_at", 0.0)) > self._discord_oauth_state_ttl
        ]
        for key in expired_states:
            self._discord_oauth_states.pop(key, None)

        expired_sessions = [
            sid
            for sid, row in self._discord_sessions.items()
            if float(row.get("expires_at", 0.0)) <= now
        ]
        for sid in expired_sessions:
            self._discord_sessions.pop(sid, None)

        max_states = 1000
        if len(self._discord_oauth_states) > max_states:
            oldest = sorted(
                self._discord_oauth_states.items(),
                key=lambda item: float(item[1].get("created_at", 0.0)),
            )
            for state_key, _ in oldest[: len(self._discord_oauth_states) - max_states]:
                self._discord_oauth_states.pop(state_key, None)

        max_sessions = 5000
        if len(self._discord_sessions) > max_sessions:
            oldest_sessions = sorted(
                self._discord_sessions.items(),
                key=lambda item: float(item[1].get("created_at", 0.0)),
            )
            for session_key, _ in oldest_sessions[: len(self._discord_sessions) - max_sessions]:
                self._discord_sessions.pop(session_key, None)

    def _set_discord_session_cookie(
        self, response: web.StreamResponse, request: web.Request, session_id: str
    ) -> None:
        cookie_domain = self._discord_session_cookie_domain()
        response.set_cookie(
            self._discord_session_cookie,
            session_id,
            max_age=self._discord_session_ttl,
            httponly=True,
            secure=self._is_secure_request(request),
            samesite="Lax",
            path="/",
            domain=cookie_domain,
        )

    def _clear_discord_session_cookie(
        self, response: web.StreamResponse, request: web.Request
    ) -> None:
        cookie_domain = self._discord_session_cookie_domain()
        response.del_cookie(
            self._discord_session_cookie,
            path="/",
            httponly=True,
            samesite="Lax",
            secure=self._is_secure_request(request),
            domain=cookie_domain,
        )

    def _discord_session_cookie_domain(self) -> str | None:
        redirect_uri = self._normalized_discord_redirect_uri()
        if not redirect_uri:
            return None
        try:
            parsed = urlparse(redirect_uri)
        except Exception:
            return None
        hostname = str(parsed.hostname or "").strip().lower()
        if not hostname or hostname in {"localhost", "127.0.0.1", "::1"}:
            return None
        return hostname

    def _get_discord_auth_session(self, request: web.Request) -> dict[str, Any] | None:
        if not self._discord_auth_required:
            return None
        self._cleanup_discord_auth_state()
        session_id = (request.cookies.get(self._discord_session_cookie) or "").strip()
        if not session_id:
            return None
        session = self._discord_sessions.get(session_id)
        if not session:
            return None
        now = time.time()
        if float(session.get("expires_at", 0.0)) <= now:
            self._discord_sessions.pop(session_id, None)
            return None
        session["expires_at"] = now + self._discord_session_ttl
        session["last_seen_at"] = now
        self._ensure_session_csrf_token(session)
        return session

    def validate_discord_session(self, session_id: str) -> dict[str, Any] | None:
        if not self._discord_auth_required:
            return None
        token = str(session_id or "").strip()
        if not token:
            return None
        self._cleanup_discord_auth_state()
        session = self._discord_sessions.get(token)
        if not session:
            return None
        now = time.time()
        if float(session.get("expires_at", 0.0)) <= now:
            self._discord_sessions.pop(token, None)
            return None
        session["expires_at"] = now + self._discord_session_ttl
        session["last_seen_at"] = now
        self._ensure_session_csrf_token(session)
        return session

    def _auth_session_for_request(self, request: web.Request) -> dict[str, Any] | None:
        return self._get_discord_auth_session(request)

    async def _check_discord_member_access(self, discord_user_id: int) -> tuple[bool, str]:
        if discord_user_id == self._discord_owner_user_id:
            return True, "owner_override"

        guilds: list[Any] = []
        seen: set[int] = set()
        for guild_id in self._discord_auth_guild_ids:
            guild = self.bot.get_guild(guild_id)
            if guild and guild.id not in seen:
                guilds.append(guild)
                seen.add(guild.id)
        if not guilds:
            guilds = list(getattr(self.bot, "guilds", []) or [])

        for guild in guilds:
            member = guild.get_member(discord_user_id)
            if member is None:
                try:
                    member = await guild.fetch_member(discord_user_id)
                except Exception:
                    member = None
            if member is None:
                continue
            try:
                permissions = getattr(member, "guild_permissions", None)
                if permissions and bool(getattr(permissions, "administrator", False)):
                    return True, f"guild_admin:{guild.id}"
            except Exception:
                logger.debug(
                    "Failed to evaluate guild admin permissions for discord user %s in guild %s",
                    discord_user_id,
                    getattr(guild, "id", "?"),
                    exc_info=True,
                )
            try:
                role_ids = {
                    int(r.id) for r in getattr(member, "roles", []) if getattr(r, "id", None)
                }
            except Exception:
                role_ids = set()
            if self._discord_moderator_role_id in role_ids:
                return True, f"moderator_role:{guild.id}"
        return False, "missing_admin_or_moderator_role"

    async def _check_discord_turnier_only_access(self, discord_user_id: int) -> tuple[bool, str]:
        """Check if user has the Community-Moderator (turnier-only) role."""
        guilds = []
        seen: set[int] = set()
        for guild_id in self._discord_auth_guild_ids:
            guild = self.bot.get_guild(guild_id)
            if guild and guild.id not in seen:
                guilds.append(guild)
                seen.add(guild.id)
        if not guilds:
            guilds = list(getattr(self.bot, "guilds", []) or [])

        for guild in guilds:
            member = guild.get_member(discord_user_id)
            if member is None:
                try:
                    member = await guild.fetch_member(discord_user_id)
                except Exception:
                    member = None
            if member is None:
                continue
            try:
                role_ids = {
                    int(r.id) for r in getattr(member, "roles", []) if getattr(r, "id", None)
                }
            except Exception:
                role_ids = set()
            if TURNIER_MOD_ROLE_ID in role_ids:
                return True, f"turnier_mod:{guild.id}"
        return False, "missing_turnier_mod_role"

    def _check_turnier_auth(self, request: web.Request) -> None:
        """Auth check for turnier routes: accepts full mods AND community turnier mods."""
        if not self._is_auth_enforced():
            return
        if self._auth_misconfigured:
            raise web.HTTPServiceUnavailable(text="Dashboard Auth nicht korrekt konfiguriert.")
        session = self._get_discord_auth_session(request) if self._discord_auth_required else None
        if session:
            if self._requires_csrf_check(request):
                if not self._is_allowed_request_origin(request):
                    raise web.HTTPForbidden(text="Origin validation failed")
                if not self._check_csrf(request, session):
                    raise web.HTTPForbidden(text="CSRF validation failed")
            return
        login_url = self._build_discord_login_url(request, next_path="/turnier")
        headers: dict[str, str] = {"X-Auth-Login": login_url}
        raise web.HTTPUnauthorized(text="Authentication required", headers=headers)

    async def _exchange_discord_code(self, code: str, redirect_uri: str) -> dict[str, Any] | None:
        payload = {
            "client_id": self._discord_client_id,
            "client_secret": self._discord_client_secret,
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri,
        }
        headers = {"Content-Type": "application/x-www-form-urlencoded"}
        timeout = ClientTimeout(total=20)
        async with ClientSession(timeout=timeout) as session:
            async with session.post(
                f"{DISCORD_API_BASE_URL}/oauth2/token",
                data=payload,
                headers=headers,
            ) as response:
                if response.status != 200:
                    body = await response.text()
                    logger.warning(
                        "Discord OAuth code exchange failed (status=%s, body=%s)",
                        response.status,
                        self._safe_log_value(body[:200]),
                    )
                    return None
                data = await response.json()
        return data if isinstance(data, dict) else None

    async def _fetch_discord_user(self, access_token: str) -> dict[str, Any] | None:
        if not access_token:
            return None
        timeout = ClientTimeout(total=20)
        headers = {"Authorization": f"Bearer {access_token}"}
        async with ClientSession(timeout=timeout) as session:
            async with session.get(
                f"{DISCORD_API_BASE_URL}/users/@me", headers=headers
            ) as response:
                if response.status != 200:
                    body = await response.text()
                    logger.warning(
                        "Discord user lookup failed (status=%s, body=%s)",
                        response.status,
                        self._safe_log_value(body[:200]),
                    )
                    return None
                data = await response.json()
        return data if isinstance(data, dict) else None

    async def _fetch_discord_connections(self, access_token: str) -> list[dict[str, Any]] | None:
        if not access_token:
            return None
        timeout = ClientTimeout(total=20)
        headers = {"Authorization": f"Bearer {access_token}"}
        async with ClientSession(timeout=timeout) as session:
            async with session.get(
                f"{DISCORD_API_BASE_URL}/users/@me/connections",
                headers=headers,
            ) as response:
                if response.status != 200:
                    body = await response.text()
                    logger.warning(
                        "Discord connections lookup failed (status=%s, body=%s)",
                        response.status,
                        self._safe_log_value(body[:200]),
                    )
                    return None
                data = await response.json()
        if not isinstance(data, list):
            return None
        return [item for item in data if isinstance(item, dict)]

    @staticmethod
    def _discord_avatar_url(user: dict[str, Any]) -> str:
        discord_id = str(user.get("id") or "").strip()
        avatar = str(user.get("avatar") or "").strip()
        if not discord_id or not avatar:
            return ""
        ext = "gif" if avatar.startswith("a_") else "png"
        return f"https://cdn.discordapp.com/avatars/{discord_id}/{avatar}.{ext}"

    async def _fetch_discord_member_role_ids(
        self,
        discord_user_id: int,
        *,
        guild_id: int | None = None,
    ) -> list[str]:
        guilds: list[Any] = []
        seen: set[int] = set()

        if guild_id:
            guild = self.bot.get_guild(int(guild_id))
            if guild is not None:
                guilds.append(guild)
                seen.add(int(guild.id))

        for configured_guild_id in self._discord_auth_guild_ids:
            if configured_guild_id in seen:
                continue
            guild = self.bot.get_guild(configured_guild_id)
            if guild is not None:
                guilds.append(guild)
                seen.add(int(guild.id))

        if not guilds:
            guilds = list(getattr(self.bot, "guilds", []) or [])

        for guild in guilds:
            member = guild.get_member(discord_user_id)
            if member is None:
                try:
                    member = await guild.fetch_member(discord_user_id)
                except Exception:
                    member = None
            if member is None:
                continue

            role_ids: list[str] = []
            for role in getattr(member, "roles", []) or []:
                role_id = getattr(role, "id", None)
                if role_id is None:
                    continue
                try:
                    normalized_role_id = int(role_id)
                except (TypeError, ValueError):
                    continue
                if normalized_role_id == int(getattr(guild, "id", 0) or 0):
                    continue
                role_ids.append(str(normalized_role_id))
            return role_ids
        return []

    def _has_valid_auth(self, request: web.Request) -> bool:
        return bool(self._discord_auth_required and self._get_discord_auth_session(request))

    def _check_auth(
        self,
        request: web.Request,
        *,
        required: bool = False,
        require_full_access: bool = False,
    ) -> None:
        enforce = required or self._is_auth_enforced()
        if not enforce:
            return
        if self._auth_misconfigured:
            raise web.HTTPServiceUnavailable(
                text=(
                    "Dashboard Auth ist nicht korrekt konfiguriert. "
                    "Discord OAuth Client-ID/Secret fehlen im Windows-Tresor (DeadlockBot)."
                )
            )
        session = self._get_discord_auth_session(request) if self._discord_auth_required else None
        if session:
            if self._requires_csrf_check(request):
                if not self._is_allowed_request_origin(request):
                    raise web.HTTPForbidden(text="Origin validation failed")
                if not self._check_csrf(request, session):
                    raise web.HTTPForbidden(text="CSRF validation failed")
            if require_full_access and str(session.get("access_level", "full")) != "full":
                raise web.HTTPForbidden(text="Full dashboard access required")
            return

        next_path = "/admin" if request.path.startswith("/api/") else None
        login_url = self._build_discord_login_url(request, next_path=next_path)
        headers: dict[str, str] = {"X-Auth-Login": login_url}
        raise web.HTTPUnauthorized(text="Authentication required", headers=headers)

    def _log_deadlock_audit(
        self,
        request: web.Request,
        action: str,
        *,
        hero_id: int | None = None,
        changes: dict[str, Any] | None = None,
    ) -> None:
        session = self._get_discord_auth_session(request) if self._discord_auth_required else None
        user_id = session.get("user_id") if session else None
        access_level = str(session.get("access_level", "")) if session else ""
        display_name = (
            str(session.get("display_name") or session.get("username") or "") if session else ""
        )
        safe_changes = self._safe_log_value(
            json.dumps(self._sanitize(changes or {}), ensure_ascii=True, separators=(",", ":"))
        )
        logger.info(
            "AUDIT deadlock %s: discord_user=%s display_name=%s access=%s peer=%s hero_id=%s changes=%s",
            self._safe_log_value(action),
            self._safe_log_value(user_id),
            self._safe_log_value(display_name),
            self._safe_log_value(access_level),
            self._safe_log_value(self._peer_host(request)),
            hero_id if hero_id is not None else "-",
            safe_changes,
        )

    def _list_log_files(self) -> list[dict[str, Any]]:
        log_dir = self._log_dir
        if not log_dir.exists() or not log_dir.is_dir():
            return []
        entries: list[dict[str, Any]] = []
        for path in log_dir.iterdir():
            if not path.is_file():
                continue
            name = path.name
            if name.startswith("."):
                continue
            try:
                stat = path.stat()
            except OSError:
                continue
            modified = _dt.datetime.fromtimestamp(
                stat.st_mtime,
                tz=_dt.UTC,
            ).isoformat()
            entries.append(
                {
                    "name": name,
                    "size": stat.st_size,
                    "modified": modified,
                    "modified_ts": stat.st_mtime,
                }
            )
        entries.sort(key=lambda item: item["modified_ts"], reverse=True)
        for item in entries:
            item.pop("modified_ts", None)
        return entries

    def _resolve_log_file(self, name: str) -> Path:
        raw = (name or "").strip()
        if not raw:
            raise web.HTTPBadRequest(text="Log file missing")
        if raw in {".", ".."} or Path(raw).name != raw or ".." in raw:
            raise web.HTTPBadRequest(text="Invalid log filename")
        log_dir = self._log_dir
        path = log_dir / raw
        try:
            resolved = path.resolve()
            log_dir_resolved = log_dir.resolve()
        except (OSError, RuntimeError):
            raise web.HTTPBadRequest(text="Invalid log filename")
        if log_dir_resolved not in resolved.parents and resolved != log_dir_resolved:
            raise web.HTTPBadRequest(text="Invalid log filename")
        if not path.exists() or not path.is_file():
            raise web.HTTPNotFound(text="Log file not found")
        return path

    @staticmethod
    def _tail_log_lines(path: Path, limit: int) -> list[str]:
        if limit <= 0:
            return []
        block_size = 8192
        data = b""
        lines: list[bytes] = []
        with path.open("rb") as handle:
            handle.seek(0, os.SEEK_END)
            position = handle.tell()
            while position > 0 and len(lines) <= limit and len(data) < LOG_TAIL_MAX_BYTES:
                read_size = min(block_size, position)
                position -= read_size
                handle.seek(position)
                data = handle.read(read_size) + data
                lines = data.splitlines()
        if len(lines) > limit:
            lines = lines[-limit:]
        return [line.decode("utf-8", errors="replace") for line in lines]

    def _normalize_names(self, items: Iterable[str]) -> list[str]:
        normalized: list[str] = []
        for raw in items:
            resolved, matches = self.bot.resolve_cog_identifier(raw)
            if resolved:
                normalized.append(resolved)
                continue
            if matches:
                raise web.HTTPBadRequest(
                    text=f"Identifier '{raw}' is ambiguous: {', '.join(matches)}"
                )
            raise web.HTTPBadRequest(text=f"Cog '{raw}' not found")
        return normalized

    def _normalize_manage_targets(self, items: Iterable[str]) -> list[str]:
        normalized: list[str] = []
        for raw in items:
            resolved, matches = self.bot.resolve_cog_identifier(raw)
            if resolved:
                normalized.append(resolved)
                continue
            if matches:
                raise web.HTTPBadRequest(
                    text=f"Identifier '{raw}' is ambiguous: {', '.join(matches)}"
                )
            raise web.HTTPBadRequest(text=f"Cog '{raw}' not found")
        return normalized

    @staticmethod
    def _format_netloc(host: str, port: int | None, scheme: str) -> str:
        safe_host = host.strip() or "127.0.0.1"
        if ":" in safe_host and not (safe_host.startswith("[") and safe_host.endswith("]")):
            safe_host = f"[{safe_host}]"
        default_ports = {"http": 80, "https": 443}
        default_port = default_ports.get(scheme, None)
        if port is None or (default_port is not None and port == default_port):
            return safe_host
        return f"{safe_host}:{port}"

    @staticmethod
    def _format_base_url(host: str, port: int | None, scheme: str) -> str:
        netloc = DashboardServer._format_netloc(host, port, scheme)
        return urlunparse((scheme, netloc, "", "", "", ""))

    @staticmethod
    def _normalize_public_url(value: str, *, default_scheme: str) -> str:
        raw = value.strip()
        if not raw:
            raise ValueError("Dashboard public URL must not be empty")
        parsed = urlparse(raw)
        if parsed.scheme and parsed.netloc:
            try:
                parsed_port: int | None = parsed.port
            except ValueError:
                parsed_port = None
            netloc = DashboardServer._format_netloc(
                parsed.hostname or parsed.netloc,
                parsed_port,
                parsed.scheme,
            )
            path = parsed.path.rstrip("/")
            return urlunparse(
                (
                    parsed.scheme,
                    netloc,
                    path,
                    parsed.params,
                    parsed.query,
                    parsed.fragment,
                )
            )

        if parsed.netloc and not parsed.scheme:
            scheme = default_scheme
            try:
                parsed_port = parsed.port
            except ValueError:
                parsed_port = None
            netloc = DashboardServer._format_netloc(
                parsed.hostname or parsed.netloc, parsed_port, scheme
            )
            path = parsed.path.rstrip("/")
            return urlunparse((scheme, netloc, path, parsed.params, parsed.query, parsed.fragment))

        fallback = urlparse(f"{default_scheme}://{raw}")
        try:
            fallback_port = fallback.port
        except ValueError:
            fallback_port = None
        netloc = DashboardServer._format_netloc(
            fallback.hostname or fallback.netloc or fallback.path,
            fallback_port,
            fallback.scheme,
        )
        path = fallback.path.rstrip("/")
        return urlunparse(
            (
                fallback.scheme,
                netloc,
                path,
                fallback.params,
                fallback.query,
                fallback.fragment,
            )
        )

    def _resolve_nssm_executable_path(self) -> str | None:
        configured = (self._nssm_executable or "").strip()
        if configured:
            configured_path = Path(configured).expanduser()
            if configured_path.is_absolute() and configured_path.is_file():
                return str(configured_path)
            return None

        for candidate in NSSM_PATH_CANDIDATES:
            if Path(candidate).is_file():
                return candidate
        return None

    def _resolve_powershell_executable_path(self) -> str | None:
        configured = (self._powershell_executable or "").strip()
        if configured:
            configured_path = Path(configured).expanduser()
            if configured_path.is_absolute() and configured_path.is_file():
                return str(configured_path)
            return None

        for candidate in POWERSHELL_PATH_CANDIDATES:
            if Path(candidate).is_file():
                return candidate
        return None

    @staticmethod
    def _resolve_sc_executable_path() -> str | None:
        for candidate in SC_PATH_CANDIDATES:
            if Path(candidate).is_file():
                return candidate
        return None

    def _query_windows_service_pid(self, service_name: str) -> int | None:
        if os.name != "nt":
            return None
        normalized_name = (service_name or "").strip()
        if not normalized_name:
            return None

        sc_executable = self._resolve_sc_executable_path()
        if not sc_executable:
            return None

        try:
            process = subprocess.run(  # noqa: S603
                [sc_executable, "queryex", normalized_name],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=3.0,
                check=False,
            )
        except Exception as exc:
            logger.debug(
                "SC queryex failed for service '%s': %s",
                self._safe_log_value(normalized_name),
                exc,
            )
            return None

        if process.returncode != 0:
            detail_lines = (process.stderr or process.stdout or "").strip().splitlines()
            detail = detail_lines[0] if detail_lines else f"exit code {process.returncode}"
            logger.debug(
                "SC queryex returned non-zero for service '%s': %s",
                self._safe_log_value(normalized_name),
                self._safe_log_value(detail),
            )
            return None

        for line in (process.stdout or "").splitlines():
            stripped = line.strip()
            if not stripped.upper().startswith("PID"):
                continue
            if ":" not in stripped:
                continue
            _, raw_value = stripped.split(":", 1)
            value = raw_value.strip()
            if not value:
                continue
            try:
                return int(value)
            except ValueError:
                continue
        return None

    @staticmethod
    def _powershell_literal(value: str) -> str:
        return "'" + str(value).replace("'", "''") + "'"

    def _build_nssm_restart_script(self, nssm_executable: str) -> str:
        delay_ms = max(250, int(self._nssm_restart_delay_seconds * 1000))
        return (
            "$ErrorActionPreference = 'Stop'; "
            f"Start-Sleep -Milliseconds {delay_ms}; "
            f"& {self._powershell_literal(nssm_executable)} restart {self._powershell_literal(self._nssm_service_name)}"
        )

    def _schedule_nssm_service_restart(self) -> tuple[bool, str]:
        if not self._nssm_service_restart_enabled:
            return False, "NSSM restart disabled (MASTER_NSSM_RESTART_ENABLED=0)"
        if os.name != "nt":
            return False, "NSSM restart is only available on Windows hosts"
        if not self._nssm_service_name:
            return False, "NSSM service name missing (MASTER_NSSM_SERVICE_NAME)"

        nssm_executable = self._resolve_nssm_executable_path()
        if not nssm_executable:
            if self._nssm_executable:
                return False, (
                    f"NSSM executable not found or not absolute file path: {self._nssm_executable}"
                )
            return (
                False,
                "nssm.exe not found in hardened path list (set MASTER_NSSM_EXE)",
            )

        powershell_exe = self._resolve_powershell_executable_path()
        if not powershell_exe:
            if self._powershell_executable:
                return False, (
                    "PowerShell executable not found or not absolute file path: "
                    f"{self._powershell_executable}"
                )
            return False, "powershell.exe not found in hardened path list"

        creationflags = 0
        for flag_name in (
            "CREATE_NEW_PROCESS_GROUP",
            "CREATE_NO_WINDOW",
            "CREATE_BREAKAWAY_FROM_JOB",
        ):
            creationflags |= int(getattr(subprocess, flag_name, 0) or 0)

        command = [
            powershell_exe,
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            self._build_nssm_restart_script(nssm_executable),
        ]
        probe_timeout = max(2.0, self._nssm_restart_delay_seconds + 1.5)
        try:
            process = subprocess.Popen(  # noqa: S603
                command,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                universal_newlines=True,
                creationflags=creationflags,
                cwd=str(Path(__file__).resolve().parent.parent),
            )
        except Exception as exc:
            logger.exception(
                "Failed to launch NSSM restart for service '%s': %s",
                self._nssm_service_name,
                exc,
            )
            return False, f"Failed to launch NSSM restart: {exc}"

        try:
            stdout, stderr = process.communicate(timeout=probe_timeout)
        except subprocess.TimeoutExpired:
            if process.stdout:
                process.stdout.close()
            if process.stderr:
                process.stderr.close()
            logger.info(
                "NSSM restart command still running after %.1fs for service '%s'",
                probe_timeout,
                self._safe_log_value(self._nssm_service_name),
            )
        else:
            if process.returncode != 0:
                detail_lines = (stderr or stdout or "").strip().splitlines()
                detail = detail_lines[0] if detail_lines else f"exit code {process.returncode}"
                safe_detail = self._safe_log_value(detail)
                logger.warning(
                    "NSSM restart command failed for service '%s' (exit=%s, detail=%s)",
                    self._safe_log_value(self._nssm_service_name),
                    process.returncode,
                    safe_detail,
                )
                return False, (
                    f"NSSM restart command failed (exit={process.returncode}): {safe_detail}"
                )
            logger.info(
                "NSSM restart command exited quickly with code 0 for service '%s'",
                self._safe_log_value(self._nssm_service_name),
            )

        logger.info(
            "NSSM restart scheduled for service '%s' (nssm=%s, delay=%.2fs)",
            self._safe_log_value(self._nssm_service_name),
            self._safe_log_value(nssm_executable),
            self._nssm_restart_delay_seconds,
        )
        return True, f"Service restart requested ({self._nssm_service_name})"

    def _schedule_systemd_service_restart(self) -> tuple[bool, str]:
        """Schedule a systemd service restart (Linux)."""
        if os.name == "nt":
            return False, "systemd restart is only available on Linux hosts"
        if not self._systemd_service_name:
            return False, "systemd service name missing (MASTER_SYSTEMD_SERVICE_NAME)"
        try:
            import shutil

            systemctl_path = shutil.which("systemctl")
            if not systemctl_path:
                return False, "systemctl not found in PATH"
            result = subprocess.run(
                [systemctl_path, "--user", "restart", self._systemd_service_name],
                capture_output=True,
                text=True,
                timeout=30,
            )
            if result.returncode != 0:
                return (
                    False,
                    f"systemctl failed (exit={result.returncode}): {result.stderr.strip()}",
                )
            logger.info("systemd restart scheduled for service '%s'", self._systemd_service_name)
            return True, f"systemd restart requested ({self._systemd_service_name})"
        except subprocess.TimeoutExpired:
            return False, "systemctl timed out after 30s"
        except Exception as exc:
            logger.exception(
                "Failed to schedule systemd restart for service '%s': %s",
                self._systemd_service_name,
                exc,
            )
            return False, f"Failed to schedule systemd restart: {exc}"

    @staticmethod
    def _parse_positive_float(raw: str | None, *, default: float, env_name: str) -> float:
        if raw is None:
            return default
        value = raw.strip()
        if not value:
            return default
        try:
            parsed = float(value)
        except ValueError:
            # Only log env name, not the raw value, to avoid leaking sensitive env vars
            logging.warning("%s value is invalid – using default %.1fs", env_name, default)
            return default
        if parsed <= 0:
            # Only log env name, not the raw value, to avoid leaking sensitive env vars
            logging.warning("%s value must be > 0 – using default %.1fs", env_name, default)
            return default
        return parsed

    @staticmethod
    def _coerce_bool(value: Any) -> bool | None:
        if isinstance(value, bool):
            return value
        if isinstance(value, str):
            normalized = value.strip().lower()
            if normalized in {"1", "true", "yes", "on"}:
                return True
            if normalized in {"0", "false", "no", "off"}:
                return False
        return None

    @staticmethod
    def _coerce_int(value: Any, default: int | None = None) -> int | None:
        try:
            if value is None:
                return default
            return int(value)
        except (TypeError, ValueError):
            return default

    @staticmethod
    def _stringify_ids(data: Any) -> Any:
        """Recursively convert large integer IDs to strings for JSON safety."""
        if isinstance(data, list):
            return [DashboardServer._stringify_ids(x) for x in data]
        if isinstance(data, dict):
            new_dict = {}
            for k, v in data.items():
                # Discord IDs and other large numbers need to be strings in JSON
                if (
                    k
                    in (
                        "id",
                        "guild_id",
                        "user_id",
                        "created_by",
                        "team_id",
                        "period_id",
                        "message_id",
                        "channel_id",
                    )
                    and isinstance(v, int)
                    and v > 1000000
                ):
                    new_dict[k] = str(v)
                else:
                    new_dict[k] = DashboardServer._stringify_ids(v)
            return new_dict
        return data

    @staticmethod
    def _parse_metadata_json(raw: Any) -> dict[str, Any]:
        if isinstance(raw, dict):
            return raw
        if raw is None:
            return {}
        if isinstance(raw, (bytes, bytearray)):
            try:
                raw = raw.decode("utf-8", errors="ignore")
            except Exception:
                return {}
        if not isinstance(raw, str):
            return {}
        text = raw.strip()
        if not text:
            return {}
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            return {}
        return parsed if isinstance(parsed, dict) else {}

    def _derive_steam_return_url(self) -> str | None:
        return MASTER_DASHBOARD_STEAM_RETURN_URL or None

    def _build_health_targets(self) -> list[dict[str, Any]]:
        targets: list[dict[str, Any]] = []
        seen_keys: set[str] = set()

        def _append_query_param(url: str, key: str, value: str) -> str:
            try:
                parsed = urlparse(url)
            except Exception:
                return url
            query = dict(parse_qsl(parsed.query, keep_blank_values=True))
            if query.get(key) == value:
                return url
            query[key] = value
            return urlunparse(parsed._replace(query=urlencode(query)))

        def _add_target(
            label: str,
            url: str,
            *,
            key: str | None = None,
            method: str = "GET",
        ) -> None:
            safe_url = (url or "").strip()
            if not safe_url:
                return
            if safe_url.startswith("http://") or safe_url.startswith("https://"):
                try:
                    safe_url = self._normalize_public_url(safe_url, default_scheme=self._scheme)
                except Exception as exc:
                    logging.warning("Healthcheck URL '%s' invalid (%s) – skipping entry", url, exc)
                    return
            safe_label = (label or safe_url).strip() or safe_url
            safe_method = (method or "GET").strip().upper() or "GET"
            key_base = (key or self._slugify_health_key(safe_label)).strip() or "health"
            unique_key = key_base
            suffix = 2
            while unique_key in seen_keys:
                unique_key = f"{key_base}-{suffix}"
                suffix += 1
            seen_keys.add(unique_key)

            entry: dict[str, Any] = {
                "key": unique_key,
                "label": safe_label,
                "url": safe_url,
                "method": safe_method,
            }
            targets.append(entry)

        if self._steam_return_url:
            steam_health_url = _append_query_param(self._steam_return_url, "healthcheck", "1")
            _add_target("Steam OAuth Callback", steam_health_url, key="steam-oauth-callback")

        # Explicit Health Checks for Core Domains
        _add_target("Main Site", "https://deutsche-deadlock-community.de/health", key="main-site")
        _add_target(
            "Steam Link Service",
            "https://deutsche-deadlock-community.de/link/health",
            key="steam-link-service",
        )

        return targets

    @staticmethod
    def _slugify_health_key(value: str) -> str:
        slug = "".join(char.lower() if char.isalnum() else "-" for char in value)
        pieces = [part for part in slug.split("-") if part]
        return "-".join(pieces) or "health"

    @staticmethod
    def _normalize_discord_callback_uri(raw: str | None, *, expected_path: str) -> str | None:
        safe_path = str(expected_path or "").strip() or "/callback/discord"
        raw_value = (raw or "").strip()
        if not raw_value:
            return None
        candidate = raw_value if "://" in raw_value else f"https://{raw_value}"
        try:
            parsed = urlparse(candidate)
        except Exception:
            return None
        scheme = (parsed.scheme or "").strip().lower()
        host = (parsed.hostname or "").strip().lower()
        if scheme not in {"http", "https"}:
            return None
        if scheme == "http" and host not in {"127.0.0.1", "localhost", "::1"}:
            return None
        if parsed.username or parsed.password or not parsed.netloc:
            return None
        if (parsed.path or "").rstrip("/") != safe_path:
            return None
        return urlunparse((scheme, parsed.netloc, safe_path, "", "", ""))

    def _normalized_discord_redirect_uri(self) -> str | None:
        return self._normalize_discord_callback_uri(
            self._discord_redirect_uri,
            expected_path="/callback/discord",
        )

    def _normalized_shared_discord_callback_uri(
        self,
        state_data: dict[str, Any] | None = None,
    ) -> str | None:
        metadata = state_data.get("metadata") if isinstance(state_data, dict) else None
        candidates: list[str] = []
        if isinstance(metadata, dict):
            metadata_redirect_uri = str(metadata.get("redirect_uri") or "").strip()
            if metadata_redirect_uri:
                candidates.append(metadata_redirect_uri)

        public_base = str(self._public_base_url or "").strip().rstrip("/")
        if public_base:
            candidates.append(f"{public_base}/callback/discord")

        for candidate in candidates:
            normalized = self._normalize_discord_callback_uri(
                candidate,
                expected_path="/callback/discord",
            )
            if normalized:
                return normalized
        return None

    def _oauth_redirect_after_url(self, value: Any, *, fallback: str = "/admin") -> str:
        return self._safe_template_href(str(value or "").strip(), fallback=fallback)

    @staticmethod
    def _is_allowed_redirect_after(value: str) -> bool:
        candidate = str(value or "").strip()
        if not candidate:
            return False
        try:
            parsed = urlparse(candidate)
        except Exception:
            return False
        if (parsed.scheme or "").lower() != "https":
            return False
        if parsed.username or parsed.password or not parsed.netloc:
            return False
        host = (parsed.hostname or "").strip().lower()
        if host not in {
            "deutsche-deadlock-community.de",
            "admin.deutsche-deadlock-community.de",
        }:
            return False
        return True

    async def _store_oauth_state_result(
        self,
        state: str,
        metadata: Any,
        result: dict[str, Any],
    ) -> None:
        if isinstance(metadata, dict):
            payload = dict(metadata)
        elif metadata is None:
            payload = {}
        else:
            payload = {"initial_metadata": metadata}

        payload["oauth_result"] = result
        try:
            await db.execute_async(
                "UPDATE oauth_states SET metadata = ? WHERE state = ?",
                (
                    json.dumps(
                        self._sanitize(payload),
                        ensure_ascii=False,
                        separators=(",", ":"),
                    ),
                    state,
                ),
            )
        except Exception:
            logger.exception(
                "Failed to persist OAuth callback result for state=%s",
                self._safe_log_value(state),
            )

    @staticmethod
    def _extract_steam_connection_ids(connections: list[dict[str, Any]]) -> list[str]:
        steam_connection_ids: list[str] = []
        seen_ids: set[str] = set()
        for connection in connections:
            if str(connection.get("type") or "").strip().lower() != "steam":
                continue
            candidates = [
                str(connection.get("id") or "").strip(),
                str(connection.get("name") or "").strip(),
            ]
            metadata = connection.get("metadata")
            if isinstance(metadata, dict):
                candidates.append(str(metadata.get("steam_id") or "").strip())
            for candidate in candidates:
                if not candidate or candidate in seen_ids:
                    continue
                seen_ids.add(candidate)
                steam_connection_ids.append(candidate)
        return steam_connection_ids

    async def _handle_discord_login(self, request: web.Request) -> web.StreamResponse:
        if not self._check_auth_rate_limit(request, max_requests=10, window_seconds=60.0):
            raise web.HTTPTooManyRequests(
                text="Zu viele Login-Versuche. Bitte in einer Minute erneut versuchen.",
                headers={"Retry-After": "60"},
            )
        if self._auth_misconfigured:
            raise web.HTTPServiceUnavailable(
                text=(
                    "Dashboard Auth ist nicht korrekt konfiguriert. "
                    "Discord OAuth Client-ID/Secret fehlen im Windows-Tresor (DeadlockBot)."
                )
            )
        if not self._discord_auth_required:
            raise web.HTTPFound(self._safe_internal_redirect("/admin"))
        existing = self._get_discord_auth_session(request)
        next_path = self._normalize_auth_next_path(request.query.get("next"))
        if existing:
            raise web.HTTPFound(self._safe_internal_redirect("/admin"))

        redirect_uri = self._normalized_discord_redirect_uri()
        if not redirect_uri:
            expected_redirect = (
                str(self._discord_redirect_uri or "").strip()
                or "https://deutsche-deadlock-community.de/callback/discord"
            )
            raise web.HTTPServiceUnavailable(
                text=(
                    "Discord OAuth Redirect URI ist ungültig. "
                    f"Erwartet wird exakt: {expected_redirect}."
                )
            )

        self._cleanup_discord_auth_state()
        state = secrets.token_urlsafe(32)
        self._discord_oauth_states[state] = {
            "created_at": time.time(),
            "next_path": next_path,
            "redirect_uri": redirect_uri,
        }
        query = urlencode(
            {
                "client_id": self._discord_client_id,
                "redirect_uri": redirect_uri,
                "response_type": "code",
                "scope": "identify",
                "state": state,
            }
        )
        raise web.HTTPFound(f"{DISCORD_API_BASE_URL}/oauth2/authorize?{query}")

    async def _handle_discord_initiate(self, request: web.Request) -> web.Response:
        self._require_any_internal_api_access(request)
        if not self._is_discord_oauth_configured():
            return web.json_response({"error": "discord_oauth_not_configured"}, status=503)

        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        if not isinstance(payload, dict):
            return web.json_response({"error": "invalid_payload"}, status=400)

        scope = str(payload.get("scope") or "identify guilds.members.read").strip()
        if not scope:
            scope = "identify guilds.members.read"
        redirect_after = str(payload.get("redirect_after") or "").strip()
        requesting_service = str(payload.get("requesting_service") or "").strip()
        raw_metadata = payload.get("metadata")
        service_metadata = dict(raw_metadata) if isinstance(raw_metadata, dict) else {}

        if not redirect_after or not self._is_allowed_redirect_after(redirect_after):
            return web.json_response({"error": "invalid_redirect_after"}, status=400)
        if not requesting_service:
            return web.json_response({"error": "missing_requesting_service"}, status=400)

        state = secrets.token_urlsafe(32)
        redirect_uri = MASTER_DASHBOARD_DISCORD_REDIRECT_URI

        await asyncio.to_thread(
            db.create_state,
            state,
            "discord",
            f"delegated:{requesting_service}",
            redirect_after,
            {"redirect_uri": redirect_uri, "scope": scope, **service_metadata},
            requesting_service=requesting_service,
        )
        query = urlencode(
            {
                "client_id": self._discord_client_id,
                "redirect_uri": redirect_uri,
                "response_type": "code",
                "scope": scope,
                "state": state,
            }
        )
        return web.json_response(
            {
                "authorize_url": f"{DISCORD_API_BASE_URL}/oauth2/authorize?{query}",
                "state_id": state,
            }
        )

    async def _handle_callback_discord(self, request: web.Request) -> web.StreamResponse:
        if not self._check_auth_rate_limit(request, max_requests=20, window_seconds=60.0):
            raise web.HTTPTooManyRequests(
                text="Zu viele OAuth-Callback-Anfragen. Bitte in einer Minute erneut versuchen.",
                headers={"Retry-After": "60"},
            )
        if not self._is_discord_oauth_configured():
            raise web.HTTPServiceUnavailable(
                text="Discord OAuth ist nicht konfiguriert. Client ID/Secret fehlen."
            )

        state = (request.query.get("state") or "").strip()
        if not state:
            return web.Response(text="Fehlender OAuth state.", status=400)

        state_data = await asyncio.to_thread(db.validate_state, state)
        if not state_data:
            # Fallback: Admin-Login-State liegt in-memory (erzeugt von _handle_discord_login)
            if state in self._discord_oauth_states and self._discord_auth_required:
                return await self._handle_discord_callback(request)
            return web.Response(text="OAuth state ungültig oder abgelaufen.", status=400)
        if str(state_data.get("provider") or "").strip().lower() != "discord":
            return web.Response(text="OAuth state provider mismatch.", status=400)

        flow_type = str(state_data.get("flow_type") or "")
        is_delegated = flow_type.startswith("delegated:")
        if not is_delegated and not await asyncio.to_thread(db.consume_state, state):
            return web.Response(text="OAuth state ungültig oder bereits verwendet.", status=400)

        redirect_after = self._oauth_redirect_after_url(
            state_data.get("redirect_after"),
            fallback="/admin",
        )
        metadata = state_data.get("metadata")
        completed_at = int(time.time())
        error = (request.query.get("error") or "").strip()
        if error:
            await self._store_oauth_state_result(
                state,
                metadata,
                {
                    "provider": "discord",
                    "status": "error",
                    "completed_at": completed_at,
                    "error": error,
                },
            )
            if is_delegated:
                sep = "&" if "?" in redirect_after else "?"
                raise web.HTTPFound(f"{redirect_after}{sep}state_id={state}")
            raise web.HTTPFound(redirect_after)

        code = (request.query.get("code") or "").strip()
        if not code:
            await self._store_oauth_state_result(
                state,
                metadata,
                {
                    "provider": "discord",
                    "status": "error",
                    "completed_at": completed_at,
                    "error": "missing_code",
                },
            )
            if is_delegated:
                sep = "&" if "?" in redirect_after else "?"
                raise web.HTTPFound(f"{redirect_after}{sep}state_id={state}")
            raise web.HTTPFound(redirect_after)

        redirect_uri = self._normalized_shared_discord_callback_uri(state_data)
        if not redirect_uri:
            await self._store_oauth_state_result(
                state,
                metadata,
                {
                    "provider": "discord",
                    "status": "error",
                    "completed_at": completed_at,
                    "error": "invalid_redirect_uri",
                },
            )
            if is_delegated:
                sep = "&" if "?" in redirect_after else "?"
                raise web.HTTPFound(f"{redirect_after}{sep}state_id={state}")
            raise web.HTTPFound(redirect_after)

        token_data = await self._exchange_discord_code(code, redirect_uri)
        access_token = str((token_data or {}).get("access_token") or "").strip()
        if not access_token:
            await self._store_oauth_state_result(
                state,
                metadata,
                {
                    "provider": "discord",
                    "status": "error",
                    "completed_at": completed_at,
                    "error": "token_exchange_failed",
                },
            )
            if is_delegated:
                sep = "&" if "?" in redirect_after else "?"
                raise web.HTTPFound(f"{redirect_after}{sep}state_id={state}")
            raise web.HTTPFound(redirect_after)

        discord_user = await self._fetch_discord_user(access_token)
        result_payload: dict[str, Any] = {
            "provider": "discord",
            "status": "success",
            "completed_at": completed_at,
            "token": token_data,
        }
        if discord_user:
            result_payload["user"] = discord_user

        await self._store_oauth_state_result(state, metadata, result_payload)
        if is_delegated:
            sep = "&" if "?" in redirect_after else "?"
            raise web.HTTPFound(f"{redirect_after}{sep}state_id={state}")
        raise web.HTTPFound(redirect_after)

    async def _handle_discord_consume_result(self, request: web.Request) -> web.Response:
        self._require_any_internal_api_access(request)
        if not self._is_discord_oauth_configured():
            return web.json_response({"error": "discord_oauth_not_configured"}, status=503)

        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        if not isinstance(payload, dict):
            return web.json_response({"error": "invalid_payload"}, status=400)

        state_id = str(payload.get("state_id") or "").strip()
        if not state_id:
            return web.json_response({"error": "missing_state_id"}, status=400)

        state_data = await asyncio.to_thread(db.validate_state, state_id)
        if not state_data:
            return web.json_response({"error": "state_not_found_or_expired"}, status=404)

        metadata = state_data.get("metadata") or {}
        if not isinstance(metadata, dict):
            metadata = {}
        oauth_result = metadata.get("oauth_result")
        if not isinstance(oauth_result, dict):
            return web.json_response({"error": "oauth_not_completed"}, status=202)
        if oauth_result.get("status") == "error":
            await asyncio.to_thread(db.consume_state, state_id)
            return web.json_response(
                {"error": oauth_result.get("error", "oauth_failed")},
                status=400,
            )

        if not await asyncio.to_thread(db.consume_state, state_id):
            return web.json_response({"error": "state_already_consumed"}, status=409)

        token_data = oauth_result.get("token") or {}
        access_token = str(token_data.get("access_token") or "").strip()
        user_data = oauth_result.get("user") or {}
        discord_user_id = self._coerce_int(user_data.get("id"), None)
        if not discord_user_id:
            return web.json_response({"error": "invalid_discord_user"}, status=502)

        username = str(user_data.get("username") or "").strip()
        global_name = str(user_data.get("global_name") or "").strip()
        discord_name = global_name or username or f"User {discord_user_id}"
        discord_avatar = self._discord_avatar_url(user_data)
        guild_id = self._coerce_int(metadata.get("guild_id"), None)
        discord_roles = await self._fetch_discord_member_role_ids(
            int(discord_user_id),
            guild_id=guild_id,
        )

        result: dict[str, Any] = {
            "discord_id": str(discord_user_id),
            "discord_name": discord_name,
            "discord_avatar": discord_avatar,
            "discord_roles": discord_roles,
            "service_metadata": {
                key: value
                for key, value in metadata.items()
                if key not in {"redirect_uri", "scope", "oauth_result"}
            },
        }

        scope = str(metadata.get("scope") or "")
        if "connections" in scope and access_token:
            connections = await self._fetch_discord_connections(access_token)
            result["steam_connection_ids"] = self._extract_steam_connection_ids(connections or [])

        return web.json_response(result)

    async def _handle_discord_callback(self, request: web.Request) -> web.StreamResponse:
        if not self._check_auth_rate_limit(request, max_requests=20, window_seconds=60.0):
            raise web.HTTPTooManyRequests(
                text="Zu viele OAuth-Callback-Anfragen. Bitte in einer Minute erneut versuchen.",
                headers={"Retry-After": "60"},
            )
        if not self._discord_auth_required:
            raise web.HTTPFound(self._safe_internal_redirect("/admin"))

        error = (request.query.get("error") or "").strip()
        if error:
            return web.Response(text=f"Discord OAuth Fehler: {error}", status=401)

        state = (request.query.get("state") or "").strip()
        code = (request.query.get("code") or "").strip()
        if not state or not code:
            return web.Response(text="Fehlender OAuth state/code.", status=400)

        self._cleanup_discord_auth_state()
        state_data = self._discord_oauth_states.pop(state, None)
        if not state_data:
            return web.Response(text="OAuth state ungültig oder abgelaufen.", status=400)

        token_data = await self._exchange_discord_code(
            code, str(state_data.get("redirect_uri") or "")
        )
        access_token = str((token_data or {}).get("access_token") or "").strip()
        if not access_token:
            return web.Response(text="OAuth Austausch fehlgeschlagen.", status=401)

        user = await self._fetch_discord_user(access_token)
        if not user:
            return web.Response(text="Discord-User konnte nicht geladen werden.", status=401)

        user_id = self._coerce_int(user.get("id"), None)
        if not user_id:
            return web.Response(text="Ungültige Discord-User-ID.", status=401)

        allowed, reason = await self._check_discord_member_access(int(user_id))
        access_level = "full"
        if not allowed:
            # Try turnier-only access (Community-Moderator role)
            turnier_ok, turnier_reason = await self._check_discord_turnier_only_access(int(user_id))
            if turnier_ok:
                allowed = True
                reason = turnier_reason
                access_level = "turnier_only"
        if not allowed:
            logger.warning(
                "AUDIT master-dashboard login denied: discord_user=%s reason=%s peer=%s",
                user_id,
                self._safe_log_value(reason),
                self._safe_log_value(self._peer_host(request)),
            )
            return web.Response(
                text=(
                    "Kein Zugriff auf das Admin-Dashboard. "
                    "Benötigt: Administrator-Recht, Moderator-Rolle oder Community-Moderator-Rolle."
                ),
                status=403,
            )

        username = str(user.get("username") or "").strip()
        global_name = str(user.get("global_name") or "").strip()
        discriminator = str(user.get("discriminator") or "0").strip()
        if global_name:
            display_name = global_name
        elif discriminator and discriminator != "0":
            display_name = f"{username}#{discriminator}"
        else:
            display_name = username or f"User {user_id}"

        self._cleanup_discord_auth_state()
        now = time.time()
        session_id = secrets.token_urlsafe(32)
        self._discord_sessions[session_id] = {
            "user_id": int(user_id),
            "username": username,
            "display_name": display_name,
            "reason": reason,
            "access_level": access_level,  # "full" or "turnier_only"
            "csrf_token": secrets.token_urlsafe(32),
            "created_at": now,
            "last_seen_at": now,
            "expires_at": now + self._discord_session_ttl,
        }

        logger.info(
            "AUDIT master-dashboard login success: discord_user=%s reason=%s peer=%s",
            user_id,
            self._safe_log_value(reason),
            self._safe_log_value(self._peer_host(request)),
        )

        destination = self._normalize_auth_next_path(state_data.get("next_path"))
        safe_destination = self._public_dashboard_redirect_url(destination, fallback="/admin")
        response = web.HTTPFound(safe_destination)
        self._set_discord_session_cookie(response, request, session_id)
        raise response

    async def _handle_turnier_discord_authorize_url(self, request: web.Request) -> web.Response:
        self._require_turnier_internal_api_access(request)
        if not self._is_discord_oauth_configured():
            return web.json_response({"error": "discord_oauth_not_configured"}, status=503)

        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        if not isinstance(payload, dict):
            return web.json_response({"error": "invalid_payload"}, status=400)

        redirect_uri = str(payload.get("redirect_uri") or "").strip()
        if not redirect_uri:
            return web.json_response({"error": "missing_redirect_uri"}, status=400)

        scope = str(payload.get("scope") or "identify guilds.members.read").strip()
        if not scope:
            scope = "identify guilds.members.read"

        query = urlencode(
            {
                "client_id": self._discord_client_id,
                "redirect_uri": redirect_uri,
                "response_type": "code",
                "scope": scope,
            }
        )
        return web.json_response(
            {"authorize_url": f"{DISCORD_API_BASE_URL}/oauth2/authorize?{query}"}
        )

    async def _handle_turnier_discord_session(self, request: web.Request) -> web.Response:
        self._require_turnier_internal_api_access(request)
        if not self._is_discord_oauth_configured():
            return web.json_response({"error": "discord_oauth_not_configured"}, status=503)

        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        if not isinstance(payload, dict):
            return web.json_response({"error": "invalid_payload"}, status=400)

        code = str(payload.get("code") or "").strip()
        redirect_uri = str(payload.get("redirect_uri") or "").strip()
        guild_id = self._coerce_int(payload.get("guild_id"), None)
        if not code or not redirect_uri:
            return web.json_response({"error": "missing_code_or_redirect_uri"}, status=400)

        token_data = await self._exchange_discord_code(code, redirect_uri)
        access_token = str((token_data or {}).get("access_token") or "").strip()
        if not access_token:
            return web.json_response({"error": "discord_code_exchange_failed"}, status=502)

        user = await self._fetch_discord_user(access_token)
        if not user:
            return web.json_response({"error": "discord_user_lookup_failed"}, status=502)

        discord_user_id = self._coerce_int(user.get("id"), None)
        if not discord_user_id:
            return web.json_response({"error": "invalid_discord_user_id"}, status=502)

        username = str(user.get("username") or "").strip()
        global_name = str(user.get("global_name") or "").strip()
        discord_name = global_name or username or f"User {discord_user_id}"
        discord_avatar = self._discord_avatar_url(user)
        discord_roles = await self._fetch_discord_member_role_ids(
            int(discord_user_id),
            guild_id=guild_id,
        )

        return web.json_response(
            {
                "discord_id": str(discord_user_id),
                "discord_name": discord_name,
                "discord_avatar": discord_avatar,
                "discord_roles": discord_roles,
            }
        )

    async def _handle_steam_link_discord_session(self, request: web.Request) -> web.Response:
        self._require_turnier_internal_api_access(request)
        if not self._is_discord_oauth_configured():
            return web.json_response({"error": "discord_oauth_not_configured"}, status=503)

        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        if not isinstance(payload, dict):
            return web.json_response({"error": "invalid_payload"}, status=400)

        code = str(payload.get("code") or "").strip()
        redirect_uri = str(payload.get("redirect_uri") or "").strip()
        if not code or not redirect_uri:
            return web.json_response({"error": "missing_code_or_redirect_uri"}, status=400)

        token_data = await self._exchange_discord_code(code, redirect_uri)
        access_token = str((token_data or {}).get("access_token") or "").strip()
        if not access_token:
            return web.json_response({"error": "discord_code_exchange_failed"}, status=502)

        user = await self._fetch_discord_user(access_token)
        if not user:
            return web.json_response({"error": "discord_user_lookup_failed"}, status=502)

        connections = await self._fetch_discord_connections(access_token)
        if connections is None:
            return web.json_response({"error": "discord_connections_lookup_failed"}, status=502)

        discord_user_id = self._coerce_int(user.get("id"), None)
        if not discord_user_id:
            return web.json_response({"error": "invalid_discord_user_id"}, status=502)

        username = str(user.get("username") or "").strip()
        global_name = str(user.get("global_name") or "").strip()
        discord_name = global_name or username or f"User {discord_user_id}"

        return web.json_response(
            {
                "discord_id": str(discord_user_id),
                "discord_name": discord_name,
                "steam_connection_ids": self._extract_steam_connection_ids(connections),
            }
        )

    async def _handle_twitch_discord_authorize_url(self, request: web.Request) -> web.Response:
        self._require_twitch_internal_api_access(request)
        if not self._is_discord_oauth_configured():
            return web.json_response({"error": "discord_oauth_not_configured"}, status=503)

        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        if not isinstance(payload, dict):
            return web.json_response({"error": "invalid_payload"}, status=400)

        redirect_uri = str(payload.get("redirect_uri") or "").strip()
        if not redirect_uri:
            return web.json_response({"error": "missing_redirect_uri"}, status=400)

        scope = str(payload.get("scope") or "identify guilds.members.read").strip()
        if not scope:
            scope = "identify guilds.members.read"

        query = urlencode(
            {
                "client_id": self._discord_client_id,
                "redirect_uri": redirect_uri,
                "response_type": "code",
                "scope": scope,
            }
        )
        return web.json_response(
            {"authorize_url": f"{DISCORD_API_BASE_URL}/oauth2/authorize?{query}"}
        )

    async def _handle_twitch_discord_session(self, request: web.Request) -> web.Response:
        self._require_twitch_internal_api_access(request)
        if not self._is_discord_oauth_configured():
            return web.json_response({"error": "discord_oauth_not_configured"}, status=503)

        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        if not isinstance(payload, dict):
            return web.json_response({"error": "invalid_payload"}, status=400)

        code = str(payload.get("code") or "").strip()
        redirect_uri = str(payload.get("redirect_uri") or "").strip()
        guild_id = self._coerce_int(payload.get("guild_id"), None)
        if not code or not redirect_uri:
            return web.json_response({"error": "missing_code_or_redirect_uri"}, status=400)

        token_data = await self._exchange_discord_code(code, redirect_uri)
        access_token = str((token_data or {}).get("access_token") or "").strip()
        if not access_token:
            return web.json_response({"error": "discord_code_exchange_failed"}, status=502)

        user = await self._fetch_discord_user(access_token)
        if not user:
            return web.json_response({"error": "discord_user_lookup_failed"}, status=502)

        discord_user_id = self._coerce_int(user.get("id"), None)
        if not discord_user_id:
            return web.json_response({"error": "invalid_discord_user_id"}, status=502)

        username = str(user.get("username") or "").strip()
        global_name = str(user.get("global_name") or "").strip()
        discord_name = global_name or username or f"User {discord_user_id}"
        discord_avatar = self._discord_avatar_url(user)
        discord_roles = await self._fetch_discord_member_role_ids(
            int(discord_user_id),
            guild_id=guild_id,
        )

        return web.json_response(
            {
                "discord_id": str(discord_user_id),
                "discord_name": discord_name,
                "discord_avatar": discord_avatar,
                "discord_roles": discord_roles,
            }
        )

    async def _handle_twitch_discord_validate_session(self, request: web.Request) -> web.Response:
        self._require_twitch_internal_api_access(request)
        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        session_id = str(payload.get("session_id") or "").strip()
        if not session_id:
            return web.json_response({"error": "missing_session_id"}, status=400)
        session = self._discord_sessions.get(session_id)
        if not session:
            return web.json_response({"valid": False})
        now = time.time()
        if float(session.get("expires_at", 0.0)) <= now:
            self._discord_sessions.pop(session_id, None)
            return web.json_response({"valid": False})
        return web.json_response(
            {
                "valid": True,
                "user_id": str(session.get("user_id") or ""),
                "username": str(session.get("username") or ""),
                "display_name": str(session.get("display_name") or ""),
                "expires_at": float(session.get("expires_at", now + 3600)),
            }
        )

    async def _handle_twitch_discord_import_session(self, request: web.Request) -> web.Response:
        self._require_twitch_internal_api_access(request)
        try:
            payload = await request.json()
        except Exception:
            return web.json_response({"error": "invalid_json"}, status=400)
        session_id = str(payload.get("session_id") or "").strip()
        user_id = self._coerce_int(payload.get("user_id"), None)
        username = str(payload.get("username") or "").strip()
        display_name = str(payload.get("display_name") or username).strip()
        if not session_id or not user_id:
            return web.json_response({"error": "missing_session_id_or_user_id"}, status=400)
        now = time.time()
        expires_at = float(payload.get("expires_at") or (now + self._discord_session_ttl))
        self._discord_sessions[session_id] = {
            "user_id": user_id,
            "username": username,
            "display_name": display_name,
            "reason": "twitch_dashboard_import",
            "access_level": "full",
            "csrf_token": secrets.token_urlsafe(32),
            "created_at": now,
            "last_seen_at": now,
            "expires_at": expires_at,
        }
        return web.json_response({"ok": True})

    async def _handle_logout(self, request: web.Request) -> web.StreamResponse:
        session_id = (request.cookies.get(self._discord_session_cookie) or "").strip()
        if session_id:
            self._discord_sessions.pop(session_id, None)
        login_url = MASTER_DISCORD_ADMIN_LOGIN_URL if self._discord_auth_required else "/admin"
        safe_login_url = self._safe_internal_redirect(login_url)
        response = web.HTTPFound(safe_login_url)
        self._clear_discord_session_cookie(response, request)
        raise response

    async def _handle_auth_me(self, request: web.Request) -> web.Response:
        if not self._discord_auth_required:
            return self._json(
                {
                    "enabled": False,
                    "authenticated": False,
                    "mode": "none",
                }
            )

        session = self._get_discord_auth_session(request)
        if not session:
            self._check_auth(request)
            raise web.HTTPUnauthorized(text="Authentication required")

        csrf_token = self._ensure_session_csrf_token(session)
        return self._json(
            {
                "enabled": True,
                "authenticated": True,
                "mode": "discord",
                "user": {
                    "id": session.get("user_id"),
                    "display_name": session.get("display_name"),
                    "username": session.get("username"),
                },
                "csrf_token": csrf_token,
            }
        )

    async def _handle_index(self, request: web.Request) -> web.Response:
        if self._auth_misconfigured:
            raise web.HTTPServiceUnavailable(
                text=(
                    "Dashboard Auth ist nicht korrekt konfiguriert. "
                    "Discord OAuth Client-ID/Secret fehlen im Windows-Tresor (DeadlockBot)."
                )
            )
        if self._is_auth_enforced() and not self._has_valid_auth(request):
            if self._discord_auth_required:
                login_url = MASTER_DISCORD_ADMIN_LOGIN_URL
                raise web.HTTPFound(self._safe_internal_redirect(login_url))
            self._check_auth(request)

        session = self._get_discord_auth_session(request)
        # Community-Moderatoren (turnier_only) haben keinen Zugriff auf das Haupt-Dashboard
        if session and str(session.get("access_level", "full")) == "turnier_only":
            raise web.HTTPFound(self._safe_internal_redirect("/turnier"))
        display_name = str((session or {}).get("display_name") or "Nicht angemeldet")
        safe_discord_login_url = html.escape(
            self._safe_template_href(
                self._build_discord_login_url(request, next_path="/admin"),
                fallback="/auth/discord/login?next=%2Fadmin",
            ),
            quote=True,
        )
        safe_auth_logout_url = html.escape(
            self._safe_template_href("/auth/logout", fallback="/auth/logout"),
            quote=True,
        )
        html_text = (
            _load_index_html()
            .replace("{{AUTH_USER_LABEL}}", html.escape(display_name, quote=True))
            .replace("{{DISCORD_LOGIN_URL}}", safe_discord_login_url)
            .replace("{{AUTH_LOGOUT_URL}}", safe_auth_logout_url)
        )
        return web.Response(text=html_text, content_type="text/html")

    def _voice_cog(self) -> Any:
        """
        Try to retrieve the VoiceActivityTrackerCog instance without importing it directly.
        Falls back to name matching to stay resilient if the cog isn't loaded.
        """
        try:
            cog = self.bot.get_cog("VoiceActivityTrackerCog")
            if cog:
                return cog
        except Exception:
            logging.getLogger(__name__).debug(
                "VoiceActivityTrackerCog lookup failed via direct get_cog",
                exc_info=True,
            )
        for cog in self.bot.cogs.values():
            if cog.__class__.__name__ == "VoiceActivityTrackerCog":
                return cog
        return None

    def _resolve_display_names(self, user_ids: Iterable[int]) -> dict[int, str]:
        names: dict[int, str] = {}
        for uid in {u for u in user_ids if u}:
            display_name: str | None = None
            for guild in self.bot.guilds:
                try:
                    member = guild.get_member(uid)
                except Exception:
                    member = None
                if member:
                    display_name = getattr(member, "display_name", None) or getattr(
                        member, "name", None
                    )
                    break
            if not display_name:
                user = self.bot.get_user(uid)
                if user:
                    display_name = getattr(user, "display_name", None) or getattr(
                        user, "name", None
                    )
            names[uid] = display_name or f"User {uid}"
        return names

    def _retention_excluded_roles(self) -> set[int]:
        try:
            cog = self.bot.get_cog("UserRetentionCog")
            cfg = getattr(cog, "config", None) if cog else None
            roles = getattr(cfg, "excluded_role_ids", None)
            resolved = {int(r) for r in roles if r} if roles else set()
            if resolved:
                return resolved
        except Exception:  # pragma: no cover - defensive
            logger.debug("Could not resolve retention exclusion roles from cog", exc_info=True)
        return set(DEFAULT_RETENTION_EXCLUDED_ROLE_IDS)

    def _has_retention_excluded_role(
        self,
        user_id: int | None,
        guild_id: int | None,
        excluded: set[int],
    ) -> bool:
        if not user_id or not excluded:
            return False

        guilds: list[Any] = []
        if guild_id:
            try:
                guild = self.bot.get_guild(int(guild_id))
            except Exception:
                guild = None
            if guild:
                guilds.append(guild)
        if not guilds:
            guilds = list(self.bot.guilds)

        for guild in guilds:
            try:
                member = guild.get_member(int(user_id))
            except Exception:
                member = None
            if not member:
                continue
            try:
                member_roles = {r.id for r in getattr(member, "roles", []) or []}
            except Exception:
                member_roles = set()
            if member_roles and member_roles & excluded:
                return True
        return False

    async def _collect_live_voice_sessions(self) -> list[dict[str, Any]]:
        cog = self._voice_cog()
        if not cog:
            return []
        try:
            voice_sessions = dict(getattr(cog, "voice_sessions", {}) or {})
        except Exception:
            voice_sessions = {}
        now = _dt.datetime.utcnow()
        sessions: list[dict[str, Any]] = []
        for session in voice_sessions.values():
            user_id = session.get("user_id")
            start_time = session.get("start_time")
            guild_id = session.get("guild_id")
            channel_id = session.get("channel_id")
            channel_name = session.get("channel_name")
            if not channel_name and guild_id and channel_id:
                guild = self.bot.get_guild(guild_id)
                if guild:
                    channel = guild.get_channel(channel_id)
                    if channel:
                        channel_name = getattr(channel, "name", None) or channel_name
            started_at: str | None
            if isinstance(start_time, _dt.datetime):
                try:
                    started_at = start_time.replace(tzinfo=_dt.UTC).isoformat()
                except Exception:
                    started_at = start_time.isoformat()
                duration_seconds = max(0, int((now - start_time).total_seconds()))
            else:
                started_at = None
                duration_seconds = 0
            sessions.append(
                {
                    "user_id": user_id,
                    "guild_id": guild_id,
                    "channel_id": channel_id,
                    "channel_name": channel_name,
                    "duration_seconds": duration_seconds,
                    "peak_users": session.get("peak_users") or 1,
                    "started_at": started_at,
                }
            )
        sessions.sort(key=lambda s: s.get("duration_seconds", 0), reverse=True)
        return sessions

    async def _handle_voice_stats(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        raw_limit = request.query.get("limit")
        try:
            limit = int(raw_limit) if raw_limit else 10
            if limit <= 0:
                raise ValueError
            limit = min(limit, 50)
        except ValueError:
            raise web.HTTPBadRequest(text="limit must be a positive integer (max 50)")

        try:
            summary_row = db.query_one(
                """
                SELECT COUNT(*) AS user_count,
                       SUM(total_seconds) AS total_seconds,
                       SUM(total_points) AS total_points,
                       MAX(last_update) AS last_update
                FROM voice_stats
                """
            )
            top_time_rows = db.query_all(
                """
                SELECT user_id, total_seconds, total_points, last_update
                FROM voice_stats
                ORDER BY total_seconds DESC, total_points DESC
                LIMIT ?
                """,
                (limit,),
            )
            top_point_rows = db.query_all(
                """
                SELECT user_id, total_seconds, total_points, last_update
                FROM voice_stats
                ORDER BY total_points DESC, total_seconds DESC
                LIMIT ?
                """,
                (limit,),
            )
        except Exception as exc:  # noqa: BLE001
            logging.exception("Failed to load voice stats: %s", exc)
            raise web.HTTPInternalServerError(text="Voice stats unavailable")

        live_sessions = await self._collect_live_voice_sessions()
        user_ids = set()
        for row in top_time_rows + top_point_rows:
            try:
                uid = row["user_id"]
            except Exception:
                uid = None
            if uid:
                user_ids.add(uid)
        for sess in live_sessions:
            uid = sess.get("user_id")
            if uid:
                user_ids.add(uid)
        name_map = self._resolve_display_names(user_ids)

        def _map_row(row: Any) -> dict[str, Any]:
            uid = row["user_id"]
            return {
                "user_id": uid,
                "display_name": name_map.get(uid, f"User {uid}"),
                "total_seconds": int(row["total_seconds"] or 0),
                "total_points": int(row["total_points"] or 0),
                "last_update": row["last_update"],
            }

        summary = {
            "tracked_users": int(summary_row["user_count"] or 0) if summary_row else 0,
            "total_seconds": int(summary_row["total_seconds"] or 0) if summary_row else 0,
            "total_points": int(summary_row["total_points"] or 0) if summary_row else 0,
            "last_update": summary_row["last_update"] if summary_row else None,
        }
        if summary["tracked_users"] > 0:
            summary["avg_seconds_per_user"] = summary["total_seconds"] / summary["tracked_users"]
        else:
            summary["avg_seconds_per_user"] = 0

        live_summary = {
            "active_sessions": len(live_sessions),
            "total_seconds": sum(sess.get("duration_seconds", 0) for sess in live_sessions),
        }
        for sess in live_sessions:
            uid = sess.get("user_id")
            if uid:
                sess["display_name"] = name_map.get(uid, f"User {uid}")

        payload = {
            "summary": summary,
            "top_by_time": [_map_row(r) for r in top_time_rows],
            "top_by_points": [_map_row(r) for r in top_point_rows],
            "live": {
                "summary": live_summary,
                "sessions": live_sessions,
            },
        }
        return self._json(payload)

    async def _handle_voice_history(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        range_raw = request.query.get("range")
        top_raw = request.query.get("top")
        sessions_raw = request.query.get("sessions")
        mode_raw = request.query.get("mode") or "hour"
        user_raw = request.query.get("user_id")
        try:
            days = int(range_raw) if range_raw else 14
            if days <= 0:
                raise ValueError
            days = min(days, 90)
        except ValueError:
            raise web.HTTPBadRequest(text="range must be a positive integer (days, max 90)")
        try:
            top_limit = int(top_raw) if top_raw else 10
            if top_limit <= 0:
                raise ValueError
            top_limit = min(top_limit, 50)
        except ValueError:
            raise web.HTTPBadRequest(text="top must be a positive integer (max 50)")
        try:
            recent_limit = int(sessions_raw) if sessions_raw else 12
            if recent_limit <= 0:
                raise ValueError
            recent_limit = min(recent_limit, 50)
        except ValueError:
            raise web.HTTPBadRequest(text="sessions must be a positive integer (max 50)")
        mode = mode_raw.strip().lower()
        if mode not in {"hour", "day", "week", "month"}:
            raise web.HTTPBadRequest(text="mode must be one of hour, day, week, month")
        user_id: int | None = None
        if user_raw:
            try:
                user_id = int(user_raw)
            except ValueError:
                raise web.HTTPBadRequest(text="user_id must be an integer")

        cutoff = f"-{days} day"
        user_filter = user_id

        try:
            daily_rows = db.query_all(
                """
                SELECT date(started_at) AS day,
                       SUM(duration_seconds) AS total_seconds,
                       COUNT(*) AS sessions,
                       COUNT(DISTINCT user_id) AS users
                FROM voice_session_log
                WHERE started_at >= datetime('now', ?)
                GROUP BY date(started_at)
                ORDER BY day DESC
                """,
                (cutoff,),
            )
            top_users_rows = db.query_all(
                """
                SELECT user_id,
                       MAX(display_name) AS display_name,
                       SUM(duration_seconds) AS total_seconds,
                       SUM(points) AS total_points,
                       COUNT(*) AS sessions
                FROM voice_session_log
                WHERE started_at >= datetime('now', ?)
                  AND (? IS NULL OR user_id = ?)
                GROUP BY user_id
                ORDER BY total_seconds DESC, total_points DESC
                LIMIT ?
                """,
                (cutoff, user_filter, user_filter, top_limit),
            )
            hourly_rows = db.query_all(
                """
                WITH grouped AS (
                    SELECT
                        CASE
                            WHEN ? = 'hour' THEN strftime('%H', started_at)
                            WHEN ? = 'day' THEN strftime('%w', started_at)
                            WHEN ? = 'week' THEN strftime('%Y-%W', started_at)
                            ELSE strftime('%Y-%m', started_at)
                        END AS bucket,
                        duration_seconds,
                        COALESCE(peak_users, 0) AS peak_users
                    FROM voice_session_log
                    WHERE started_at >= datetime('now', ?)
                      AND (? IS NULL OR user_id = ?)
                )
                SELECT bucket,
                       SUM(duration_seconds) AS total_seconds,
                       COUNT(*) AS sessions,
                       SUM(peak_users) AS sum_peak
                FROM grouped
                GROUP BY bucket
                ORDER BY bucket
                """,
                (mode, mode, mode, cutoff, user_filter, user_filter),
            )
        except Exception as exc:  # noqa: BLE001
            logging.exception("Failed to load voice history: %s", exc)
            raise web.HTTPInternalServerError(text="Voice history unavailable")

        user_ids: set[int] = set()
        for row in top_users_rows:
            try:
                uid = row["user_id"]
            except Exception:
                uid = None
            if uid:
                user_ids.add(uid)
        if user_id:
            user_ids.add(user_id)
        name_map = self._resolve_display_names(user_ids)

        def _map_top_user(row: Any) -> dict[str, Any]:
            uid = row["user_id"]
            return {
                "user_id": uid,
                "display_name": row["display_name"] or name_map.get(uid, f"User {uid}"),
                "total_seconds": int(row["total_seconds"] or 0),
                "total_points": int(row["total_points"] or 0),
                "sessions": int(row["sessions"] or 0),
            }

        daily = [
            {
                "day": row["day"],
                "total_seconds": int(row["total_seconds"] or 0),
                "sessions": int(row["sessions"] or 0),
                "users": int(row["users"] or 0),
            }
            for row in daily_rows
        ]

        buckets = []
        for row in hourly_rows:
            sessions_count = int(row["sessions"] or 0)
            buckets.append(
                {
                    "label": row["bucket"],
                    "total_seconds": int(row["total_seconds"] or 0),
                    "sessions": sessions_count,
                    "avg_peak": (
                        (int(row["sum_peak"] or 0) / sessions_count) if sessions_count > 0 else 0
                    ),
                }
            )

        if mode == "hour":
            existing = {b["label"]: b for b in buckets}
            buckets = []
            for h in range(24):
                key = str(h).zfill(2)
                buckets.append(
                    existing.get(
                        key,
                        {
                            "label": key,
                            "total_seconds": 0,
                            "sessions": 0,
                            "avg_peak": 0,
                        },
                    )
                )

        if mode == "day":
            existing = {b["label"]: b for b in buckets}
            buckets = []
            weekdays = [
                "Sonntag",
                "Montag",
                "Dienstag",
                "Mittwoch",
                "Donnerstag",
                "Freitag",
                "Samstag",
            ]
            for day in range(7):
                key = str(day)
                data = existing.get(key, {})
                buckets.append(
                    {
                        "label": weekdays[day],
                        "total_seconds": data.get("total_seconds", 0),
                        "sessions": data.get("sessions", 0),
                        "avg_peak": data.get("avg_peak", 0),
                    }
                )

        user_summary: dict[str, Any] | None = None
        recent_sessions: list[dict[str, Any]] = []
        if user_id is not None:
            try:
                range_stats = db.query_one(
                    """
                    SELECT SUM(duration_seconds) AS total_seconds,
                           SUM(points) AS total_points,
                           COUNT(*) AS sessions,
                           SUM(COALESCE(peak_users, 0)) AS sum_peak,
                           COUNT(DISTINCT date(started_at)) AS active_days,
                           MAX(ended_at) AS last_session
                    FROM voice_session_log
                    WHERE started_at >= datetime('now', ?)
                      AND (? IS NULL OR user_id = ?)
                    """,
                    (cutoff, user_filter, user_filter),
                )
                lifetime_stats = db.query_one(
                    """
                    SELECT total_seconds, total_points, last_update
                    FROM voice_stats
                    WHERE user_id = ?
                    """,
                    (user_id,),
                )
                lifetime_sessions_row = db.query_one(
                    """
                    SELECT COUNT(*) AS sessions, MAX(ended_at) AS last_session
                    FROM voice_session_log
                    WHERE user_id = ?
                    """,
                    (user_id,),
                )
                recent_rows = db.query_all(
                    """
                    SELECT id, guild_id, channel_id, channel_name, started_at, ended_at,
                           duration_seconds, points, peak_users, co_player_ids
                    FROM voice_session_log
                    WHERE user_id = ?
                    ORDER BY datetime(ended_at) DESC, id DESC
                    LIMIT ?
                    """,
                    (user_id, recent_limit),
                )
            except Exception as exc:  # noqa: BLE001
                logging.exception("Failed to build user voice summary: %s", exc)
                raise web.HTTPInternalServerError(text="Voice history unavailable")

            range_seconds = int(range_stats["total_seconds"] or 0) if range_stats else 0
            range_points = int(range_stats["total_points"] or 0) if range_stats else 0
            range_sessions = int(range_stats["sessions"] or 0) if range_stats else 0
            range_avg_session = (range_seconds / range_sessions) if range_sessions else 0
            range_avg_peak = (
                (int(range_stats["sum_peak"] or 0) / range_sessions) if range_sessions else 0
            )
            range_days = int(range_stats["active_days"] or 0) if range_stats else 0

            lifetime_seconds = int(lifetime_stats["total_seconds"] or 0) if lifetime_stats else 0
            lifetime_points = int(lifetime_stats["total_points"] or 0) if lifetime_stats else 0
            lifetime_last_update = lifetime_stats["last_update"] if lifetime_stats else None
            lifetime_sessions = (
                int(lifetime_sessions_row["sessions"] or 0) if lifetime_sessions_row else 0
            )
            last_session = None
            if range_stats:
                last_session = range_stats["last_session"]
            if not last_session and lifetime_sessions_row:
                last_session = lifetime_sessions_row["last_session"]

            co_ids_per_session: list[list[int]] = []
            co_ids_all: set[int] = set()
            for row in recent_rows:
                parsed_ids: list[int] = []
                raw_ids = row["co_player_ids"]
                decoded: Any = []
                if raw_ids:
                    try:
                        decoded = json.loads(raw_ids)
                    except Exception:
                        decoded = []
                if isinstance(decoded, list):
                    seen_ids: set[int] = set()
                    for value in decoded:
                        try:
                            co_id = int(value)
                        except (TypeError, ValueError):
                            continue
                        if co_id <= 0 or co_id == user_id or co_id in seen_ids:
                            continue
                        seen_ids.add(co_id)
                        parsed_ids.append(co_id)
                        co_ids_all.add(co_id)
                co_ids_per_session.append(parsed_ids)

            co_name_map = self._resolve_display_names(co_ids_all) if co_ids_all else {}
            for idx, row in enumerate(recent_rows):
                co_player_ids = co_ids_per_session[idx] if idx < len(co_ids_per_session) else []
                recent_sessions.append(
                    {
                        "id": int(row["id"]),
                        "guild_id": int(row["guild_id"]) if row["guild_id"] else None,
                        "channel_id": int(row["channel_id"]) if row["channel_id"] else None,
                        "channel_name": row["channel_name"] or None,
                        "started_at": row["started_at"],
                        "ended_at": row["ended_at"],
                        "duration_seconds": int(row["duration_seconds"] or 0),
                        "points": int(row["points"] or 0),
                        "peak_users": int(row["peak_users"] or 0),
                        "co_player_count": len(co_player_ids),
                        "co_players": [
                            {
                                "user_id": co_id,
                                "display_name": co_name_map.get(co_id, f"User {co_id}"),
                            }
                            for co_id in co_player_ids
                        ],
                    }
                )

            user_summary = {
                "user_id": user_id,
                "display_name": name_map.get(user_id, f"User {user_id}"),
                "range_seconds": range_seconds,
                "range_points": range_points,
                "range_sessions": range_sessions,
                "range_days": range_days,
                "range_avg_session_seconds": range_avg_session,
                "range_avg_peak": range_avg_peak,
                "lifetime_seconds": lifetime_seconds,
                "lifetime_points": lifetime_points,
                "lifetime_sessions": lifetime_sessions,
                "lifetime_last_update": lifetime_last_update,
                "last_session": last_session,
            }

        payload = {
            "range_days": days,
            "mode": mode,
            "user": (
                {
                    "user_id": user_id,
                    "display_name": name_map.get(user_id, f"User {user_id}"),
                }
                if user_id
                else None
            ),
            "daily": daily,
            "top_users": [_map_top_user(r) for r in top_users_rows],
            "buckets": buckets,
            "user_summary": user_summary,
            "recent_sessions_limit": recent_limit,
            "recent_sessions": recent_sessions,
        }
        return self._json(payload)

    async def _handle_user_retention(self, request: web.Request) -> web.Response:
        """
        Liefert Kennzahlen für den User-Retention-Cog.
        Nutzt die gleichen Default-Schwellen wie im Cog (siehe RetentionConfig in cogs/user_retention.py).
        """
        self._check_auth(request)

        # Defaults aus RetentionConfig
        min_weekly_sessions = 0.5
        min_total_active_days = 3
        inactivity_threshold_days = 14
        min_days_between_messages = 30
        max_miss_you_per_user = 1
        excluded_roles = self._retention_excluded_roles()

        try:
            # Ermittele vorhandene Spalten, um kompatibel mit evtl. aelterem Schema zu sein
            retention_columns = set()
            try:
                rows = db.query_all("PRAGMA table_info(user_retention_tracking)")
                for r in rows:
                    # sqlite3.Row oder tuple
                    name = r["name"] if hasattr(r, "__getitem__") else r[1]
                    retention_columns.add(str(name))
            except Exception:  # pragma: no cover - defensive
                retention_columns = set()

            total_tracked_row = db.query_one("SELECT COUNT(*) FROM user_retention_tracking")
            total_tracked = total_tracked_row[0] if total_tracked_row else 0

            opted_out_row = db.query_one(
                "SELECT COUNT(*) FROM user_retention_tracking WHERE opted_out = 1"
            )
            opted_out = opted_out_row[0] if opted_out_row else 0

            regular_active_row = db.query_one(
                """
                SELECT COUNT(*)
                FROM user_retention_tracking
                WHERE avg_weekly_sessions >= ? AND total_active_days >= ?
                """,
                (min_weekly_sessions, min_total_active_days),
            )
            regular_active = regular_active_row[0] if regular_active_row else 0

            candidate_where = [
                "avg_weekly_sessions >= ?",
                "total_active_days >= ?",
                "(strftime('%s','now') - last_active_at) / 86400 >= ?",
                "opted_out = 0",
            ]
            candidate_params: list[Any] = [
                min_weekly_sessions,
                min_total_active_days,
                inactivity_threshold_days,
            ]

            has_last_sent = (
                "last_miss_you_sent_at" in retention_columns
                or "last_miss_you_at" in retention_columns
            )
            has_miss_count = (
                "miss_you_count" in retention_columns or "miss_you_sent" in retention_columns
            )

            if has_last_sent:
                candidate_where.append(
                    "(last_miss_you_sent_at IS NULL OR (strftime('%s','now') - last_miss_you_sent_at) / 86400 >= ?)"
                    if "last_miss_you_sent_at" in retention_columns
                    else "(last_miss_you_at IS NULL OR (strftime('%s','now') - last_miss_you_at) / 86400 >= ?)"
                )
                candidate_params.append(min_days_between_messages)
            if has_miss_count:
                candidate_where.append(
                    "(miss_you_count IS NULL OR miss_you_count < ?)"
                    if "miss_you_count" in retention_columns
                    else "(miss_you_sent IS NULL OR miss_you_sent < ?)"
                )
                candidate_params.append(max_miss_you_per_user)

            candidate_where_sql = " AND ".join(candidate_where)

            miss_you_row = db.query_one(
                "SELECT COUNT(*) FROM user_retention_messages WHERE message_type = 'miss_you'"
            )
            miss_you_sent = miss_you_row[0] if miss_you_row else 0

            feedback_row = db.query_one(
                "SELECT COUNT(*) FROM user_retention_messages WHERE message_type = 'feedback'"
            )
            feedback_received = feedback_row[0] if feedback_row else 0

            select_fields = [
                "urt.user_id",
                "urt.guild_id",
                "urt.last_active_at",
                "urt.total_active_days",
                "urt.avg_weekly_sessions",
                "(strftime('%s','now') - urt.last_active_at) / 86400 AS days_inactive",
                """
                (
                    SELECT m.delivery_status
                    FROM user_retention_messages m
                    WHERE m.user_id = urt.user_id AND m.message_type = 'miss_you'
                    ORDER BY m.sent_at DESC
                    LIMIT 1
                ) AS last_message_status
                """,
                """
                (
                    SELECT m.sent_at
                    FROM user_retention_messages m
                    WHERE m.user_id = urt.user_id AND m.message_type = 'miss_you'
                    ORDER BY m.sent_at DESC
                    LIMIT 1
                ) AS last_message_at
                """,
            ]

            if "last_miss_you_sent_at" in retention_columns:
                select_fields.append("urt.last_miss_you_sent_at")
            elif "last_miss_you_at" in retention_columns:
                select_fields.append("urt.last_miss_you_at AS last_miss_you_sent_at")
            else:
                select_fields.append("NULL AS last_miss_you_sent_at")

            if "miss_you_count" in retention_columns:
                select_fields.append("urt.miss_you_count")
            elif "miss_you_sent" in retention_columns:
                select_fields.append("urt.miss_you_sent AS miss_you_count")
            else:
                select_fields.append("NULL AS miss_you_count")

            candidate_select_sql = ", ".join(select_fields)
            candidate_sql = (
                "SELECT " + candidate_select_sql + "\n"  # nosec B608
                "FROM user_retention_tracking urt\n"
                "WHERE " + candidate_where_sql + "\n"
                "ORDER BY days_inactive DESC"
            )
            candidate_rows_raw = db.query_all(candidate_sql, tuple(candidate_params))

            filtered_rows = [
                row
                for row in candidate_rows_raw
                if not self._has_retention_excluded_role(
                    row["user_id"],
                    row["guild_id"],
                    excluded_roles,
                )
            ]
            inactive_candidates = len(filtered_rows)
            candidate_rows = filtered_rows[:50]

            user_ids = [row["user_id"] for row in candidate_rows if row and row["user_id"]]
            name_map = self._resolve_display_names(user_ids)

            payload = {
                "summary": {
                    "total_tracked": total_tracked,
                    "opted_out": opted_out,
                    "regular_active": regular_active,
                    "inactive_candidates": inactive_candidates,
                    "miss_you_sent": miss_you_sent,
                    "feedback_received": feedback_received,
                },
                "candidates": [
                    {
                        "display_name": (
                            name_map.get(row["user_id"])
                            if name_map.get(row["user_id"])
                            else f"User {row['user_id']}"
                        ),
                        "user_id": row["user_id"],
                        "guild_id": row["guild_id"],
                        "last_active_at": row["last_active_at"],
                        "days_inactive": max(0, int(row["days_inactive"] or 0)),
                        "total_active_days": row["total_active_days"],
                        "avg_weekly_sessions": row["avg_weekly_sessions"],
                        "last_miss_you_sent_at": row["last_miss_you_sent_at"],
                        "miss_you_count": row["miss_you_count"],
                        "last_message_status": row["last_message_status"],
                        "last_message_at": row["last_message_at"],
                    }
                    for row in candidate_rows
                ],
                # legacy key, im UI jetzt als Kandidatenliste genutzt
                "recent": [
                    {
                        "display_name": (
                            name_map.get(row["user_id"])
                            if name_map.get(row["user_id"])
                            else f"User {row['user_id']}"
                        ),
                        "user_id": row["user_id"],
                        "guild_id": row["guild_id"],
                        "last_active_at": row["last_active_at"],
                        "days_inactive": max(0, int(row["days_inactive"] or 0)),
                        "total_active_days": row["total_active_days"],
                        "avg_weekly_sessions": row["avg_weekly_sessions"],
                        "last_miss_you_sent_at": row["last_miss_you_sent_at"],
                        "miss_you_count": row["miss_you_count"],
                        "last_message_status": row["last_message_status"],
                        "last_message_at": row["last_message_at"],
                    }
                    for row in candidate_rows
                ],
            }
            return self._json(payload)

        except Exception as e:
            logger.error("Error building user retention payload: %s", e, exc_info=True)
            raise web.HTTPInternalServerError(text="Failed to load user retention data")

    async def _handle_member_events(self, request: web.Request) -> web.Response:
        """Handler für Member-Events (Joins, Leaves, Bans)."""
        self._check_auth(request)

        raw_limit = request.query.get("limit")
        event_type = request.query.get("type")  # optional filter
        guild_id_raw = request.query.get("guild_id")

        try:
            limit = int(raw_limit) if raw_limit else 50
            if limit <= 0:
                raise ValueError
            limit = min(limit, 200)
        except ValueError:
            raise web.HTTPBadRequest(text="limit must be a positive integer (max 200)")

        guild_id: int | None = None
        if guild_id_raw:
            try:
                guild_id = int(guild_id_raw)
            except ValueError:
                raise web.HTTPBadRequest(text="guild_id must be an integer")

        try:
            guild_filter = guild_id if guild_id else None
            event_filter = (event_type or "").strip() or None

            # Hole Events
            events = db.query_all(
                """
                SELECT id, user_id, guild_id, event_type, timestamp,
                       display_name, account_created_at, join_position, metadata
                FROM member_events
                WHERE (? IS NULL OR guild_id = ?)
                  AND (? IS NULL OR event_type = ?)
                ORDER BY timestamp DESC
                LIMIT ?
                """,
                (guild_filter, guild_filter, event_filter, event_filter, limit),
            )

            # Event-Type Counts
            event_counts = db.query_all(
                """
                SELECT event_type, COUNT(*) as count
                FROM member_events
                WHERE (? IS NULL OR guild_id = ?)
                  AND (? IS NULL OR event_type = ?)
                GROUP BY event_type
                ORDER BY count DESC
                """,
                (guild_filter, guild_filter, event_filter, event_filter),
            )

            # Recent Joins (letzten 7 Tage)
            recent_joins = db.query_one(
                """
                SELECT COUNT(*) as count
                FROM member_events
                WHERE event_type = 'join'
                  AND timestamp >= datetime('now', '-7 days')
                  AND (? IS NULL OR guild_id = ?)
                """,
                (guild_filter, guild_filter),
            )

            # Recent Leaves (letzten 7 Tage)
            recent_leaves = db.query_one(
                """
                SELECT COUNT(*) as count
                FROM member_events
                WHERE event_type = 'leave'
                  AND timestamp >= datetime('now', '-7 days')
                  AND (? IS NULL OR guild_id = ?)
                """,
                (guild_filter, guild_filter),
            )

            events_list = []
            for row in events:
                events_list.append(
                    {
                        "id": row[0],
                        "user_id": row[1],
                        "guild_id": row[2],
                        "event_type": row[3],
                        "timestamp": row[4],
                        "display_name": row[5],
                        "account_created_at": row[6],
                        "join_position": row[7],
                        "metadata": row[8],
                    }
                )

            counts = {row[0]: row[1] for row in event_counts}

            payload = {
                "events": events_list,
                "summary": {
                    "total_events": len(events_list),
                    "event_counts": counts,
                    "recent_joins_7d": recent_joins[0] if recent_joins else 0,
                    "recent_leaves_7d": recent_leaves[0] if recent_leaves else 0,
                },
            }
            return self._json(payload)

        except Exception as exc:
            logging.exception("Failed to load member events: %s", exc)
            raise web.HTTPInternalServerError(text="Member events unavailable")

    async def _handle_message_activity(self, request: web.Request) -> web.Response:
        """Handler für Message-Activity."""
        self._check_auth(request)

        raw_limit = request.query.get("limit")
        guild_id_raw = request.query.get("guild_id")

        try:
            limit = int(raw_limit) if raw_limit else 20
            if limit <= 0:
                raise ValueError
            limit = min(limit, 100)
        except ValueError:
            raise web.HTTPBadRequest(text="limit must be a positive integer (max 100)")

        guild_id: int | None = None
        if guild_id_raw:
            try:
                guild_id = int(guild_id_raw)
            except ValueError:
                raise web.HTTPBadRequest(text="guild_id must be an integer")

        try:
            guild_filter = guild_id if guild_id else None

            # Top Users by Message Count
            top_users = db.query_all(
                """
                SELECT user_id, guild_id, channel_id, message_count,
                       last_message_at, first_message_at
                FROM message_activity
                WHERE (? IS NULL OR guild_id = ?)
                ORDER BY message_count DESC
                LIMIT ?
                """,
                (guild_filter, guild_filter, limit),
            )

            # Summary
            summary = db.query_one(
                """
                SELECT
                    COUNT(*) as total_users,
                    SUM(message_count) as total_messages,
                    AVG(message_count) as avg_per_user
                FROM message_activity
                WHERE (? IS NULL OR guild_id = ?)
                """,
                (guild_filter, guild_filter),
            )

            # Resolve display names
            user_ids = {row[0] for row in top_users}
            name_map = self._resolve_display_names(user_ids)

            users_list = []
            for row in top_users:
                user_id = row[0]
                users_list.append(
                    {
                        "user_id": user_id,
                        "display_name": name_map.get(user_id, f"User {user_id}"),
                        "guild_id": row[1],
                        "channel_id": row[2],
                        "message_count": row[3],
                        "last_message_at": row[4],
                        "first_message_at": row[5],
                    }
                )

            payload = {
                "top_users": users_list,
                "summary": {
                    "total_users": summary[0] if summary else 0,
                    "total_messages": summary[1] if summary else 0,
                    "avg_per_user": round(summary[2], 1) if summary and summary[2] else 0,
                },
            }
            return self._json(payload)

        except Exception as exc:
            logging.exception("Failed to load message activity: %s", exc)
            raise web.HTTPInternalServerError(text="Message activity unavailable")

    async def _handle_co_player_network(self, request: web.Request) -> web.Response:
        """Aggregiertes Co-Player-Netzwerk mit persistierten Anzeigenamen."""
        self._check_auth(request)

        raw_limit = request.query.get("limit")
        raw_min_sessions = request.query.get("min_sessions")

        try:
            limit = int(raw_limit) if raw_limit else 120
            if limit <= 0:
                raise ValueError
            limit = min(limit, 400)
        except ValueError:
            raise web.HTTPBadRequest(text="limit must be a positive integer (max 400)")

        try:
            min_sessions = int(raw_min_sessions) if raw_min_sessions else 1
            if min_sessions <= 0:
                raise ValueError
            min_sessions = min(min_sessions, 100000)
        except ValueError:
            raise web.HTTPBadRequest(text="min_sessions must be a positive integer")

        try:
            rows = db.query_all(
                """
                SELECT user_id, co_player_id, sessions_together, total_minutes_together,
                       last_played_together, user_display_name, co_player_display_name
                FROM user_co_players
                WHERE sessions_together >= ?
                ORDER BY sessions_together DESC, total_minutes_together DESC, last_played_together DESC
                LIMIT ?
                """,
                (min_sessions, limit * 2),
            )
        except Exception as exc:  # noqa: BLE001
            logger.exception("Failed to load co-player network: %s", exc)
            raise web.HTTPInternalServerError(text="Co-player network unavailable")

        def _ts(value: Any) -> float:
            if value is None:
                return 0.0
            if isinstance(value, (int, float)):
                return float(value)
            try:
                normalized = str(value).replace("T", " ").replace("Z", "")
                return _dt.datetime.fromisoformat(normalized).timestamp()
            except Exception:
                return 0.0

        edges: dict[tuple[int, int], dict[str, Any]] = {}
        name_map: dict[int, str] = {}
        missing_names: set[int] = set()

        for row in rows:
            try:
                uid = int(row["user_id"])
                coid = int(row["co_player_id"])
            except Exception:
                continue
            if uid == coid:
                continue
            sessions = int(row["sessions_together"] or 0)
            minutes = int(row["total_minutes_together"] or 0)
            last_played = row["last_played_together"]
            key = (uid, coid) if uid < coid else (coid, uid)
            edge = edges.get(key)
            ts_value = _ts(last_played)
            if edge is None:
                edges[key] = {
                    "source": key[0],
                    "target": key[1],
                    "sessions": sessions,
                    "minutes": minutes,
                    "last_played": last_played,
                    "last_played_ts": ts_value,
                }
            else:
                edge["sessions"] = max(edge["sessions"], sessions)
                edge["minutes"] = max(edge["minutes"], minutes)
                if ts_value > edge.get("last_played_ts", 0):
                    edge["last_played_ts"] = ts_value
                    edge["last_played"] = last_played

            user_name = row["user_display_name"]
            co_name = row["co_player_display_name"]
            if user_name:
                name_map[uid] = user_name
            else:
                missing_names.add(uid)
            if co_name:
                name_map[coid] = co_name
            else:
                missing_names.add(coid)

        if missing_names:
            resolved = self._resolve_display_names(missing_names)
            for uid, name in resolved.items():
                if name:
                    name_map[uid] = name

        updates: list[tuple[str | None, str | None, int, int]] = []
        for row in rows:
            try:
                uid = int(row["user_id"])
                coid = int(row["co_player_id"])
            except Exception:
                continue
            new_user_name = name_map.get(uid)
            new_co_name = name_map.get(coid)
            if (not row["user_display_name"] and new_user_name) or (
                not row["co_player_display_name"] and new_co_name
            ):
                updates.append((new_user_name, new_co_name, uid, coid))

        if updates:
            try:
                db.executemany(
                    """
                    UPDATE user_co_players
                    SET user_display_name = COALESCE(?, user_display_name),
                        co_player_display_name = COALESCE(?, co_player_display_name)
                    WHERE user_id = ? AND co_player_id = ?
                    """,
                    updates,
                )
            except Exception:
                logger.debug("Could not persist co-player display names", exc_info=True)

        edge_values = sorted(
            edges.values(), key=lambda e: (e["sessions"], e["minutes"]), reverse=True
        )
        trimmed_edges = edge_values[:limit]

        nodes: dict[int, dict[str, Any]] = {}
        for edge in trimmed_edges:
            source = edge["source"]
            target = edge["target"]
            for node_id in (source, target):
                if node_id not in nodes:
                    nodes[node_id] = {
                        "id": node_id,
                        "name": name_map.get(node_id, f"User {node_id}"),
                        "sessions": 0,
                        "minutes": 0,
                        "degree": 0,
                    }
                nodes[node_id]["degree"] += 1

            nodes[source]["sessions"] += edge["sessions"]
            nodes[target]["sessions"] += edge["sessions"]
            nodes[source]["minutes"] += edge["minutes"]
            nodes[target]["minutes"] += edge["minutes"]

        for node in nodes.values():
            node["weight"] = max(node["sessions"], node["minutes"] // 10)

        links = [
            {
                "source": edge["source"],
                "target": edge["target"],
                "sessions": edge["sessions"],
                "minutes": edge["minutes"],
                "last_played": edge.get("last_played"),
            }
            for edge in trimmed_edges
        ]

        payload = {
            "nodes": sorted(nodes.values(), key=lambda n: n["sessions"], reverse=True),
            "links": links,
            "meta": {
                "total_edges": len(edges),
                "returned_edges": len(links),
                "total_nodes": len(nodes),
                "generated_at": _dt.datetime.utcnow().isoformat() + "Z",
                "min_sessions": min_sessions,
            },
        }
        return self._json(payload)

    def _collect_public_vanity_links(self, guild_filter: int | None) -> list[dict[str, Any]]:
        links: list[dict[str, Any]] = []
        for guild in self.bot.guilds:
            if guild_filter is not None and int(guild.id) != int(guild_filter):
                continue
            code = str(getattr(guild, "vanity_url_code", "") or "").strip()
            if not code:
                continue
            links.append(
                {
                    "guild_id": int(guild.id),
                    "guild_name": getattr(guild, "name", None),
                    "type": "vanity",
                    "label": "Vanity-Link",
                    "code": code,
                    "url": f"https://discord.gg/{code}",
                }
            )
        links.sort(key=lambda item: str(item.get("guild_name") or "").lower())
        return links

    def _build_member_source_analytics(
        self,
        guild_filter: int | None,
        *,
        days: int = 0,
    ) -> dict[str, Any]:
        all_time = int(days) == 0
        window_days: int | None = None if all_time else max(1, min(int(days), 365))

        if all_time:
            join_rows = db.query_all(
                """
                SELECT id, user_id, guild_id, timestamp, display_name, metadata
                FROM member_events
                WHERE event_type = 'join'
                  AND (? IS NULL OR guild_id = ?)
                ORDER BY timestamp DESC
                """,
                (guild_filter, guild_filter),
            )
        else:
            cutoff_expr = f"-{window_days} days"
            join_rows = db.query_all(
                """
                SELECT id, user_id, guild_id, timestamp, display_name, metadata
                FROM member_events
                WHERE event_type = 'join'
                  AND timestamp >= datetime('now', ?)
                  AND (? IS NULL OR guild_id = ?)
                ORDER BY timestamp DESC
                """,
                (cutoff_expr, guild_filter, guild_filter),
            )

        twitch_invite_lookup: dict[str, str] = {}
        twitch_assigned_links: list[dict[str, Any]] = []
        try:
            twitch_rows = db.query_all(
                """
                SELECT streamer_login, invite_code, invite_url, created_at, last_sent_at
                FROM twitch_streamer_invites
                WHERE (? IS NULL OR guild_id = ?)
                ORDER BY streamer_login ASC
                """,
                (guild_filter, guild_filter),
            )
            for row in twitch_rows:
                streamer_login = str(row[0] or "").strip().lower()
                invite_code = str(row[1] or "").strip()
                invite_url = str(row[2] or "").strip()
                created_at = row[3]
                last_sent_at = row[4]

                if (
                    invite_code
                    and streamer_login
                    and invite_code.lower() not in twitch_invite_lookup
                ):
                    twitch_invite_lookup[invite_code.lower()] = streamer_login

                if streamer_login or invite_code or invite_url:
                    twitch_assigned_links.append(
                        {
                            "streamer_login": streamer_login or None,
                            "invite_code": invite_code or None,
                            "invite_url": invite_url
                            or (f"https://discord.gg/{invite_code}" if invite_code else None),
                            "created_at": created_at,
                            "last_sent_at": last_sent_at,
                        }
                    )
        except sqlite3.OperationalError:
            # Tabelle existiert nur wenn Twitch-Schema initialisiert wurde.
            pass
        except Exception:
            logger.debug("Failed to load twitch invite assignments", exc_info=True)

        bucket_counts: dict[str, int] = {
            "public": 0,
            "twitch": 0,
            "personal": 0,
            "bot_invite": 0,
            "unknown": 0,
        }
        public_groups: dict[str, dict[str, Any]] = {
            "server_discovery": {
                "kind": "server_discovery",
                "label": "Server entdecken",
                "count": 0,
            },
            "other": {"kind": "other", "label": "Public (Sonstige)", "count": 0},
        }
        # vanity entries added dynamically, keyed by invite_code
        twitch_groups: dict[str, dict[str, Any]] = {}
        personal_groups: dict[str, dict[str, Any]] = {}
        bot_invite_groups: dict[str, dict[str, Any]] = {}
        recent: list[dict[str, Any]] = []
        backfill_updates: list[tuple[str, int]] = []

        for row in join_rows:
            event_id = self._coerce_int(row[0], None)
            user_id = self._coerce_int(row[1], 0) or 0
            timestamp = row[3]
            display_name = row[4] or f"User {user_id}"
            metadata = self._parse_metadata_json(row[5])
            metadata_changed = False

            bucket_raw = str(metadata.get("join_source_bucket") or "").strip().lower()
            kind_raw = (
                str(metadata.get("join_source_kind") or metadata.get("join_source_type") or "")
                .strip()
                .lower()
            )
            label_raw = str(metadata.get("join_source_label") or "").strip()
            invite_code = str(metadata.get("invite_code") or "").strip()
            invite_url = str(metadata.get("invite_url") or "").strip()
            if not invite_url and invite_code:
                invite_url = f"https://discord.gg/{invite_code}"
                metadata["invite_url"] = invite_url
                metadata_changed = True
            twitch_login = str(metadata.get("twitch_streamer_login") or "").strip().lower()
            if not twitch_login and invite_code:
                twitch_login = twitch_invite_lookup.get(invite_code.lower(), "")
                if twitch_login:
                    metadata["twitch_streamer_login"] = twitch_login
                    metadata_changed = True

            bucket = bucket_raw
            if bucket not in bucket_counts:
                if twitch_login:
                    bucket = "twitch"
                elif kind_raw in {
                    "server_discovery",
                    "discovery",
                    "public_discovery",
                    "vanity",
                    "vanity_url",
                }:
                    bucket = "public"
                elif invite_code or kind_raw in {
                    "invite_link",
                    "personal",
                    "personal_invite",
                }:
                    bucket = "personal"
                else:
                    bucket = "unknown"
                metadata["join_source_bucket"] = bucket
                metadata_changed = True

            bucket_counts[bucket] += 1

            public_kind: str | None = None
            personal_label: str | None = None
            if bucket == "public":
                if kind_raw in {"server_discovery", "discovery", "public_discovery"}:
                    public_kind = "server_discovery"
                    public_groups[public_kind]["count"] += 1
                elif kind_raw in {"vanity", "vanity_url", "public_vanity"}:
                    public_kind = "vanity"
                    vanity_key = (
                        f"vanity:{invite_code.lower()}" if invite_code else "vanity:unknown"
                    )
                    if vanity_key not in public_groups:
                        code_label = f"discord.gg/{invite_code}" if invite_code else "Vanity-Link"
                        public_groups[vanity_key] = {
                            "kind": "vanity",
                            "label": code_label,
                            "invite_code": invite_code or None,
                            "invite_url": invite_url
                            or (f"https://discord.gg/{invite_code}" if invite_code else None),
                            "count": 0,
                        }
                    public_groups[vanity_key]["count"] += 1
                else:
                    public_kind = "other"
                    public_groups[public_kind]["count"] += 1

            if not kind_raw:
                if bucket == "twitch":
                    kind_raw = "twitch_streamer"
                elif bucket == "personal":
                    kind_raw = "invite_link"
                elif bucket == "public":
                    if public_kind == "server_discovery":
                        kind_raw = "server_discovery"
                    elif public_kind == "vanity":
                        kind_raw = "vanity"
                    else:
                        kind_raw = "public_other"
                else:
                    kind_raw = "unknown"
                metadata["join_source_kind"] = kind_raw
                metadata_changed = True

            if bucket == "twitch":
                key = twitch_login or invite_code.lower() or "unknown"
                label = twitch_login or (f"Invite {invite_code}" if invite_code else "Unbekannt")
                entry = twitch_groups.get(key)
                if entry is None:
                    entry = {
                        "streamer_login": twitch_login or None,
                        "label": label,
                        "invite_code": invite_code or None,
                        "invite_url": invite_url or None,
                        "count": 0,
                    }
                    twitch_groups[key] = entry
                entry["count"] += 1
                if invite_code and not entry.get("invite_code"):
                    entry["invite_code"] = invite_code
                if invite_url and not entry.get("invite_url"):
                    entry["invite_url"] = invite_url

            if bucket == "personal":
                # Bekannte Invite-Codes mit festen Labels
                _KNOWN_CODES: dict[str, str] = {
                    "xmnqmbuz7z": "In-Game (Build Publisher)",
                }
                inviter_id = self._coerce_int(metadata.get("inviter_id"), None)
                inviter_name = str(metadata.get("inviter_name") or "").strip()
                known_label = _KNOWN_CODES.get(invite_code.lower()) if invite_code else None
                if known_label:
                    key = f"known:{invite_code.lower()}"
                    personal_label = known_label
                elif inviter_id is not None:
                    key = f"id:{inviter_id}"
                    personal_label = inviter_name or f"User {inviter_id}"
                elif inviter_name:
                    key = f"name:{inviter_name.lower()}"
                    personal_label = inviter_name
                elif invite_code:
                    key = f"code:{invite_code.lower()}"
                    personal_label = f"Invite {invite_code}"
                else:
                    key = "other"
                    personal_label = "Sonstige Invite-Links"

                entry = personal_groups.get(key)
                if entry is None:
                    entry = {
                        "label": personal_label,
                        "inviter_id": inviter_id,
                        "invite_code": invite_code or None,
                        "invite_url": invite_url or None,
                        "count": 0,
                    }
                    personal_groups[key] = entry
                entry["count"] += 1
                if invite_code and not entry.get("invite_code"):
                    entry["invite_code"] = invite_code
                if invite_url and not entry.get("invite_url"):
                    entry["invite_url"] = invite_url

            if bucket == "bot_invite":
                inviter_id_bi = self._coerce_int(metadata.get("inviter_id"), None)
                inviter_name_bi = str(metadata.get("inviter_name") or "").strip()
                key_bi = (
                    f"id:{inviter_id_bi}"
                    if inviter_id_bi is not None
                    else f"name:{inviter_name_bi.lower()}"
                    if inviter_name_bi
                    else "bot_other"
                )
                label_bi = inviter_name_bi or (
                    f"Bot {inviter_id_bi}" if inviter_id_bi else "Bot Invite"
                )
                entry_bi = bot_invite_groups.get(key_bi)
                if entry_bi is None:
                    entry_bi = {
                        "label": label_bi,
                        "inviter_id": inviter_id_bi,
                        "invite_code": invite_code or None,
                        "invite_url": invite_url or None,
                        "count": 0,
                    }
                    bot_invite_groups[key_bi] = entry_bi
                entry_bi["count"] += 1

            source_label = label_raw
            if not source_label:
                if bucket == "public":
                    if public_kind == "server_discovery":
                        source_label = "Public: Server entdecken"
                    elif public_kind == "vanity":
                        source_label = "Public: Vanity-Link"
                    else:
                        source_label = "Public"
                elif bucket == "twitch":
                    source_label = f"Twitch: {twitch_login}" if twitch_login else "Twitch"
                elif bucket == "personal":
                    source_label = f"Persoenlich: {personal_label or 'Invite-Link'}"
                elif bucket == "bot_invite":
                    inviter_name_bi = str(metadata.get("inviter_name") or "").strip()
                    source_label = (
                        f"Bot Invite: {inviter_name_bi}" if inviter_name_bi else "Bot Invite"
                    )
                else:
                    source_label = "Unbekannt"

            if not label_raw:
                metadata["join_source_label"] = source_label
                metadata_changed = True

            if not str(metadata.get("join_source_confidence") or "").strip():
                confidence = "high"
                if bucket == "unknown":
                    confidence = "low"
                elif bucket == "public" and kind_raw in {
                    "server_discovery",
                    "discovery",
                    "public_discovery",
                }:
                    confidence = "medium"
                metadata["join_source_confidence"] = confidence
                metadata_changed = True

            if not str(metadata.get("join_source_detected_at") or "").strip():
                metadata["join_source_detected_at"] = timestamp or _dt.datetime.utcnow().strftime(
                    "%Y-%m-%d %H:%M:%S"
                )
                metadata_changed = True

            if metadata_changed and event_id is not None:
                backfill_updates.append(
                    (json.dumps(metadata, separators=(",", ":")), int(event_id))
                )

            recent.append(
                {
                    "user_id": user_id,
                    "display_name": display_name,
                    "timestamp": timestamp,
                    "bucket": bucket,
                    "label": source_label,
                    "invite_code": invite_code or None,
                    "invite_url": invite_url or None,
                    "twitch_streamer_login": twitch_login or None,
                }
            )

        if backfill_updates:
            try:
                db.executemany(
                    "UPDATE member_events SET metadata = ? WHERE id = ?",
                    backfill_updates,
                )
                logger.info(
                    "Member-source metadata backfilled for %d join event(s)",
                    len(backfill_updates),
                )
            except Exception:
                logger.debug("Failed to persist member-source metadata backfill", exc_info=True)

        public_breakdown = [
            entry for entry in public_groups.values() if int(entry.get("count", 0)) > 0
        ]
        twitch_breakdown = sorted(
            twitch_groups.values(),
            key=lambda item: (
                -int(item.get("count", 0)),
                str(item.get("label") or "").lower(),
            ),
        )
        personal_breakdown = sorted(
            personal_groups.values(),
            key=lambda item: (
                -int(item.get("count", 0)),
                str(item.get("label") or "").lower(),
            ),
        )
        twitch_assigned_links = sorted(
            twitch_assigned_links,
            key=lambda item: (
                str(item.get("streamer_login") or "").lower(),
                str(item.get("invite_code") or "").lower(),
            ),
        )

        known_joins = (
            bucket_counts["public"]
            + bucket_counts["twitch"]
            + bucket_counts["personal"]
            + bucket_counts["bot_invite"]
        )
        unknown_joins = bucket_counts["unknown"]

        bot_invite_breakdown = sorted(
            bot_invite_groups.values(),
            key=lambda item: (-int(item.get("count", 0)), str(item.get("label") or "").lower()),
        )

        return {
            "window_days": window_days,
            "tracked_joins": len(join_rows),
            "known_joins": known_joins,
            "unknown_joins": unknown_joins,
            "bucket_counts": bucket_counts,
            "bucket_labels": {
                "public": "Public",
                "twitch": "Twitch",
                "personal": "Persönlich",
                "bot_invite": "Bot Invites",
                "unknown": "Unbekannt",
            },
            "public_breakdown": public_breakdown,
            "public_links": self._collect_public_vanity_links(guild_filter),
            "twitch_breakdown": twitch_breakdown,
            "twitch_assigned_links": twitch_assigned_links,
            "personal_breakdown": personal_breakdown,
            "bot_invite_breakdown": bot_invite_breakdown,
            "recent": recent[:20],
        }

    async def _handle_server_stats(self, request: web.Request) -> web.Response:
        """Handler für aggregierte Server-Statistiken."""
        self._check_auth(request)

        guild_id_raw = request.query.get("guild_id")

        guild_id: int | None = None
        if guild_id_raw:
            try:
                guild_id = int(guild_id_raw)
            except ValueError:
                raise web.HTTPBadRequest(text="guild_id must be an integer")

        try:
            guild_filter = guild_id if guild_id else None

            # Member Events Summary
            member_events_summary = db.query_all(
                """
                SELECT event_type, COUNT(*) as count
                FROM member_events
                WHERE (? IS NULL OR guild_id = ?)
                GROUP BY event_type
                """,
                (guild_filter, guild_filter),
            )

            # Message Activity Summary
            message_summary = db.query_one(
                """
                SELECT SUM(message_count) as total
                FROM message_activity
                WHERE (? IS NULL OR guild_id = ?)
                """,
                (guild_filter, guild_filter),
            )

            # Voice Activity Summary
            voice_summary = db.query_one(
                """
                SELECT SUM(duration_seconds) as total_seconds
                FROM voice_session_log
                WHERE (? IS NULL OR guild_id = ?)
                """,
                (guild_filter, guild_filter),
            )

            # Active Users (last 7 days)
            active_users_7d = db.query_one(
                """
                SELECT COUNT(DISTINCT user_id) as count
                FROM message_activity
                WHERE (? IS NULL OR guild_id = ?)
                  AND last_message_at >= datetime('now', '-7 days')
                """,
                (guild_filter, guild_filter),
            )

            # Growth (Joins vs Leaves last 30 days)
            growth = db.query_one(
                """
                SELECT
                    SUM(CASE WHEN event_type = 'join' THEN 1 ELSE 0 END) as joins,
                    SUM(CASE WHEN event_type = 'leave' THEN 1 ELSE 0 END) as leaves
                FROM member_events
                WHERE (? IS NULL OR guild_id = ?)
                  AND timestamp >= datetime('now', '-30 days')
                """,
                (guild_filter, guild_filter),
            )

            member_sources_30d = self._build_member_source_analytics(guild_filter, days=0)

            payload = {
                "member_events": {row[0]: row[1] for row in member_events_summary},
                "total_messages": message_summary[0]
                if message_summary and message_summary[0]
                else 0,
                "total_voice_hours": (voice_summary[0] // 3600)
                if voice_summary and voice_summary[0]
                else 0,
                "active_users_7d": active_users_7d[0] if active_users_7d else 0,
                "growth_30d": {
                    "joins": growth[0] if growth else 0,
                    "leaves": growth[1] if growth else 0,
                    "net": (growth[0] or 0) - (growth[1] or 0) if growth else 0,
                },
                "member_sources_30d": member_sources_30d,
            }
            return self._json(payload)

        except Exception as exc:
            logging.exception("Failed to load server stats: %s", exc)
            raise web.HTTPInternalServerError(text="Server stats unavailable")

    @staticmethod
    def _map_deadlock_hero_row(row: Any) -> dict[str, Any]:
        """Normalize hero row to JSON-friendly dict."""

        def _get(key: str, idx: int | None = None) -> Any:
            try:
                return row[key]  # type: ignore[index]
            except Exception:
                if idx is None:
                    return None
                try:
                    return row[idx]  # type: ignore[index]
                except Exception:
                    return None

        origin_raw = _get("origin_build_id", 3)
        origin_build_id: int | None
        if origin_raw in (None, ""):
            origin_build_id = None
        else:
            try:
                origin_build_id = int(origin_raw)
            except Exception:
                origin_build_id = None

        return {
            "id": int(_get("id", 0) or 0),
            "hero_id": int(_get("hero_id", 1) or 0),
            "name": str(_get("name", 2) or ""),
            "origin_build_id": origin_build_id,
            "target_build_name_override": str(_get("target_build_name_override") or ""),
            "is_active": bool(int(_get("is_active", 5) or 0)),
            "created_at": _get("created_at", 6),
            "updated_at": _get("updated_at", 7),
        }

    @staticmethod
    def _map_deadlock_hero_build_row(row: Any) -> dict[str, Any]:
        """Normalize hero-build row to JSON-friendly dict."""

        def _get(key: str, idx: int | None = None) -> Any:
            try:
                return row[key]  # type: ignore[index]
            except Exception:
                if idx is None:
                    return None
                try:
                    return row[idx]  # type: ignore[index]
                except Exception:
                    return None

        return {
            "id": int(_get("id", 0) or 0),
            "hero_id": int(_get("hero_id", 1) or 0),
            "build_id": int(_get("build_id", 2) or 0),
            "build_name": str(_get("build_name", 3) or ""),
            "author_name": str(_get("author_name", 4) or ""),
            "is_active": bool(int(_get("is_active", 5) or 0)),
            "sort_order": int(_get("sort_order", 6) or 100),
            "sync_status": _get("sync_status"),
            "sync_message": _get("sync_message"),
            "last_checked_at": _get("last_checked_at"),
            "last_synced_at": _get("last_synced_at"),
            "last_alerted_at": _get("last_alerted_at"),
            "source_version": _get("source_version"),
            "source_last_updated_ts": _get("source_last_updated_ts"),
            "clone_build_id": _get("clone_build_id"),
            "clone_version": _get("clone_version"),
            "created_at": _get("created_at", 7),
            "updated_at": _get("updated_at", 8),
        }

    @staticmethod
    def _build_deadlock_legacy_suggestion(hero: dict[str, Any]) -> dict[str, Any] | None:
        origin_build_id = hero.get("origin_build_id")
        if origin_build_id in (None, "", False):
            return None
        try:
            legacy_build_id = int(origin_build_id)
        except Exception:
            return None
        return {
            "build_id": legacy_build_id,
            "build_name": "",
            "author_name": "Legacy",
            "is_active": bool(hero.get("is_active", True)),
            "sort_order": 100,
            "suggested": True,
        }

    def _coerce_request_bool(self, raw_value: Any, *, default: bool, field_name: str) -> bool:
        if raw_value is None:
            return default
        if isinstance(raw_value, bool):
            return raw_value
        if isinstance(raw_value, int):
            if raw_value in (0, 1):
                return bool(raw_value)
            raise web.HTTPBadRequest(text=f"{field_name} must be boolean")
        parsed = self._coerce_bool(raw_value)
        if parsed is None:
            raise web.HTTPBadRequest(text=f"{field_name} must be boolean")
        return parsed

    def _parse_deadlock_build_payload_row(self, raw_build: Any, *, index: int) -> dict[str, Any]:
        if not isinstance(raw_build, dict):
            raise web.HTTPBadRequest(text=f"builds[{index}] must be an object")

        build_id = self._coerce_int(raw_build.get("build_id", raw_build.get("buildId")), None)
        if build_id is None:
            raise web.HTTPBadRequest(text=f"builds[{index}].build_id must be integer")

        build_name = self._normalize_deadlock_target_name(
            raw_build.get("build_name", raw_build.get("buildName")),
            field_name=f"builds[{index}].build_name",
        )

        author_name = str(raw_build.get("author_name", raw_build.get("authorName")) or "").strip()
        if not author_name:
            raise web.HTTPBadRequest(text=f"builds[{index}].author_name is required")

        is_active = self._coerce_request_bool(
            raw_build.get("is_active", raw_build.get("isActive", True)),
            default=True,
            field_name=f"builds[{index}].is_active",
        )

        raw_sort_order = raw_build.get("sort_order", raw_build.get("sortOrder", 100))
        if raw_sort_order in (None, ""):
            sort_order = 100
        else:
            sort_order = self._coerce_int(raw_sort_order, None)
            if sort_order is None:
                raise web.HTTPBadRequest(text=f"builds[{index}].sort_order must be integer")

        return {
            "build_id": int(build_id),
            "build_name": build_name,
            "author_name": author_name,
            "is_active": bool(is_active),
            "sort_order": int(sort_order),
        }

    def _extract_deadlock_builds_payload(
        self, payload: dict[str, Any]
    ) -> list[dict[str, Any]] | None:
        has_builds_payload = (
            "builds" in payload or "hero_builds" in payload or "heroBuilds" in payload
        )
        if not has_builds_payload:
            return None

        raw_builds = payload.get("builds", payload.get("hero_builds", payload.get("heroBuilds")))
        if raw_builds is None:
            return []
        if not isinstance(raw_builds, list):
            raise web.HTTPBadRequest(text="builds must be an array")

        seen_build_ids: set[int] = set()
        builds: list[dict[str, Any]] = []
        for index, raw_build in enumerate(raw_builds, start=1):
            parsed = self._parse_deadlock_build_payload_row(raw_build, index=index)
            build_id = int(parsed["build_id"])
            if build_id in seen_build_ids:
                raise web.HTTPBadRequest(text=f"Duplicate build_id in builds: {build_id}")
            seen_build_ids.add(build_id)
            builds.append(parsed)
        return builds

    def _load_deadlock_hero_payload(
        self,
        hero_id: int,
        *,
        conn: Any | None = None,
    ) -> dict[str, Any] | None:
        select_hero_sql = """
            SELECT
                id, hero_id, name, origin_build_id, target_build_name_override,
                is_active, created_at, updated_at
              FROM deadlock_heroes
             WHERE hero_id = ?
        """
        select_builds_sql = """
            SELECT
                id, hero_id, build_id, build_name, author_name, is_active, sort_order,
                sync_status, sync_message, last_checked_at, last_synced_at, last_alerted_at,
                source_version, source_last_updated_ts, clone_build_id, clone_version,
                created_at, updated_at
              FROM deadlock_hero_builds
             WHERE hero_id = ?
             ORDER BY sort_order ASC, build_id ASC
        """

        if conn is None:
            hero_row = db.query_one(select_hero_sql, (hero_id,))
            build_rows = db.query_all(select_builds_sql, (hero_id,))
        else:
            hero_row = conn.execute(select_hero_sql, (hero_id,)).fetchone()
            build_rows = conn.execute(select_builds_sql, (hero_id,)).fetchall()

        if hero_row is None:
            return None

        hero_payload = self._map_deadlock_hero_row(hero_row)
        hero_payload["builds"] = [self._map_deadlock_hero_build_row(row) for row in build_rows]
        if not hero_payload["builds"]:
            suggestion = self._build_deadlock_legacy_suggestion(hero_payload)
            if suggestion:
                hero_payload["legacy_build_suggestion"] = suggestion
        return hero_payload

    def _apply_deadlock_build_snapshot(
        self,
        conn: Any,
        *,
        hero_id: int,
        builds: list[dict[str, Any]],
        ts: int,
    ) -> None:
        for build in builds:
            conn.execute(
                """
                INSERT INTO deadlock_hero_builds (
                    hero_id, build_id, build_name, author_name, is_active, sort_order, created_at, updated_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(hero_id, build_id) DO UPDATE SET
                    build_name = excluded.build_name,
                    author_name = excluded.author_name,
                    is_active = excluded.is_active,
                    sort_order = excluded.sort_order,
                    updated_at = excluded.updated_at
                """,
                (
                    hero_id,
                    int(build["build_id"]),
                    str(build["build_name"]),
                    str(build["author_name"]),
                    1 if bool(build["is_active"]) else 0,
                    int(build["sort_order"]),
                    ts,
                    ts,
                ),
            )

        if builds:
            submitted_build_ids = [int(build["build_id"]) for build in builds]
            placeholders = ", ".join("?" for _ in submitted_build_ids)
            delete_sql = (
                "DELETE FROM deadlock_hero_builds WHERE hero_id = ? AND build_id NOT IN ("
                + placeholders
                + ")"
            )
            conn.execute(
                delete_sql,
                (hero_id, *submitted_build_ids),
            )
        else:
            conn.execute("DELETE FROM deadlock_hero_builds WHERE hero_id = ?", (hero_id,))

    async def _run_steam_task(
        self,
        task_type: str,
        payload: dict[str, Any] | None = None,
        *,
        timeout: float = 45.0,
        poll_interval: float = 0.5,
    ) -> dict[str, Any]:
        payload_json = json.dumps(payload) if payload is not None else None
        async with db.transaction() as conn:
            cur = conn.execute(
                "INSERT INTO steam_tasks(type, payload, status) VALUES(?, ?, 'PENDING')",
                (task_type, payload_json),
            )
            task_id = int(cur.lastrowid or 0)

        deadline = time.monotonic() + max(float(timeout), float(poll_interval))
        while True:
            row = await db.query_one_async(
                "SELECT status, result, error FROM steam_tasks WHERE id = ?",
                (task_id,),
            )
            if row is None:
                return {
                    "task_id": task_id,
                    "status": "MISSING",
                    "ok": False,
                    "timed_out": False,
                    "result": None,
                    "error": "Steam task disappeared",
                }

            status = str(row["status"] or "UNKNOWN").upper()
            raw_result = row["result"]
            result: Any = raw_result
            if isinstance(raw_result, str):
                try:
                    result = json.loads(raw_result)
                except json.JSONDecodeError:
                    result = raw_result
            error = str(row["error"]) if row["error"] is not None else None

            if status in {"DONE", "FAILED"}:
                return {
                    "task_id": task_id,
                    "status": status,
                    "ok": status == "DONE",
                    "timed_out": False,
                    "result": result,
                    "error": error,
                }

            if time.monotonic() >= deadline:
                return {
                    "task_id": task_id,
                    "status": status,
                    "ok": False,
                    "timed_out": True,
                    "result": result,
                    "error": error or f"Timed out waiting for {task_type}",
                }

            await asyncio.sleep(max(0.1, float(poll_interval)))

    async def _deadlock_missing_build_alert_loop(self) -> None:
        await self.bot.wait_until_ready()
        while True:
            try:
                await self._dispatch_deadlock_missing_build_alerts()
            except asyncio.CancelledError:
                raise
            except Exception as exc:  # noqa: BLE001
                logger.exception("Deadlock missing-build alert loop failed: %s", exc)
            await asyncio.sleep(DEADLOCK_MISSING_BUILD_ALERT_INTERVAL_SECONDS)

    async def _dispatch_deadlock_missing_build_alerts(self) -> int:
        claim_ts = int(time.time())
        claim_marker = -claim_ts
        stale_cutoff = -(claim_ts - 900)
        db.execute(
            """
            UPDATE deadlock_hero_builds
               SET last_alerted_at = NULL
             WHERE last_alerted_at < 0
               AND last_alerted_at > ?
            """,
            (stale_cutoff,),
        )
        async with db.transaction() as conn:
            rows = conn.execute(
                """
                UPDATE deadlock_hero_builds
                   SET last_alerted_at = ?
                 WHERE (hero_id, build_id) IN (
                    SELECT hb.hero_id, hb.build_id
                    FROM deadlock_hero_builds hb
                    LEFT JOIN deadlock_heroes h ON h.hero_id = hb.hero_id
                    WHERE hb.is_active = 1
                      AND COALESCE(h.is_active, 1) = 1
                      AND hb.sync_status = 'missing'
                      AND hb.last_alerted_at IS NULL
                 )
                 RETURNING hero_id, build_id, build_name, author_name, sync_message
                """,
                (claim_marker,),
            ).fetchall()
        if not rows:
            return 0

        sent = 0
        channel = self.bot.get_channel(DEADLOCK_MISSING_BUILD_ALERT_CHANNEL_ID)
        if channel is None:
            try:
                channel = await self.bot.fetch_channel(DEADLOCK_MISSING_BUILD_ALERT_CHANNEL_ID)
            except Exception as exc:  # noqa: BLE001
                logger.warning("Failed to fetch Deadlock alert channel: %s", exc)
                db.execute(
                    """
                    UPDATE deadlock_hero_builds
                       SET last_alerted_at = NULL
                     WHERE last_alerted_at = ?
                    """,
                    (claim_marker,),
                )
                return 0

        for row in rows:
            hero_row = db.query_one(
                "SELECT name FROM deadlock_heroes WHERE hero_id = ? LIMIT 1",
                (int(row["hero_id"]),),
            )
            message = (
                "Deadlock Build fehlt im GC\n"
                f"Hero: {(hero_row['name'] if hero_row else row['hero_id'])} ({row['hero_id']})\n"
                f"Build: {row['build_name']} ({row['build_id']})\n"
                f"Autor: {row['author_name']}\n"
                f"Status: {row['sync_message'] or 'Quell-Build nicht gefunden.'}"
            )
            try:
                await channel.send(message)
                db.execute(
                    """
                    UPDATE deadlock_hero_builds
                       SET last_alerted_at = ?
                     WHERE hero_id = ?
                       AND build_id = ?
                       AND last_alerted_at = ?
                    """,
                    (claim_ts, int(row["hero_id"]), int(row["build_id"]), claim_marker),
                )
                sent += 1
            except Exception as exc:  # noqa: BLE001
                db.execute(
                    """
                    UPDATE deadlock_hero_builds
                       SET last_alerted_at = NULL
                     WHERE hero_id = ?
                       AND build_id = ?
                       AND last_alerted_at = ?
                    """,
                    (int(row["hero_id"]), int(row["build_id"]), claim_marker),
                )
                logger.warning(
                    "Failed to send Deadlock missing-build alert (hero_id=%s, build_id=%s): %s",
                    row["hero_id"],
                    row["build_id"],
                    exc,
                )

        return sent

    async def _refresh_deadlock_build_sources(self, *, hero_id: int) -> dict[str, Any]:
        build_rows = await db.query_all_async(
            """
            SELECT build_id, build_name, author_name
              FROM deadlock_hero_builds
             WHERE hero_id = ?
               AND is_active = 1
             ORDER BY sort_order ASC, build_id ASC
            """,
            (hero_id,),
        )

        build_ids = [
            int(row["build_id"]) for row in build_rows if self._coerce_int(row["build_id"], None)
        ]
        summary: dict[str, Any] = {
            "requested": len(build_ids),
            "refreshed": 0,
            "failed": 0,
            "timed_out": 0,
            "bridge_ready": True,
            "details": [],
        }
        if not build_ids:
            return summary

        state_row = await db.query_one_async(
            """
            SELECT payload
              FROM standalone_bot_state
             WHERE bot = 'steam'
             LIMIT 1
            """
        )
        state_payload = self._parse_metadata_json(state_row["payload"]) if state_row else {}
        runtime = state_payload.get("runtime", {}) if isinstance(state_payload, dict) else {}
        if not runtime.get("logged_on") or not runtime.get("deadlock_gc_ready"):
            summary["bridge_ready"] = False
            summary["failed"] = len(build_ids)
            summary["details"].append(
                {
                    "status": "bridge_not_ready",
                    "logged_on": bool(runtime.get("logged_on")),
                    "deadlock_gc_ready": bool(runtime.get("deadlock_gc_ready")),
                }
            )
            return summary

        task_result = await self._run_steam_task(
            "GC_SEARCH_BUILDS",
            {"hero_id": int(hero_id)},
            timeout=45.0,
            poll_interval=0.5,
        )
        task_payload = task_result.get("result")
        task_data = task_payload.get("data") if isinstance(task_payload, dict) else None
        total_results = None
        if isinstance(task_data, dict):
            total_results = self._coerce_int(task_data.get("totalResults"), None)

        if task_result.get("timed_out"):
            summary["timed_out"] += 1

        available_ids = {
            int(row["hero_build_id"])
            for row in await db.query_all_async(
                f"""
                SELECT hero_build_id
                  FROM hero_build_sources
                 WHERE hero_build_id IN ({", ".join("?" for _ in build_ids)})
                """,
                tuple(build_ids),
            )
        }
        summary["refreshed"] = len(available_ids)
        summary["failed"] = max(0, summary["requested"] - summary["refreshed"])

        if summary["failed"] > 0:
            for row in build_rows:
                build_id = int(row["build_id"])
                if build_id in available_ids:
                    continue
                summary["details"].append(
                    {
                        "status": "source_refresh_failed",
                        "build_id": build_id,
                        "build_name": str(row["build_name"]),
                        "author_name": str(row["author_name"]),
                        "task_id": task_result.get("task_id"),
                        "task_status": task_result.get("status"),
                        "timed_out": bool(task_result.get("timed_out")),
                        "error": task_result.get("error"),
                        "total_results": total_results,
                    }
                )

        if summary["refreshed"] > 0 or summary["failed"] > 0:
            logger.info(
                "Deadlock source refresh finished (hero_id=%s): requested=%s refreshed=%s failed=%s timed_out=%s",
                hero_id,
                summary["requested"],
                summary["refreshed"],
                summary["failed"],
                summary["timed_out"],
            )
        return summary

    def _sync_deadlock_hero_builds_conn(self, conn: Any, *, hero_id: int) -> dict[str, Any]:
        summary: dict[str, Any] = {
            "hero_id": int(hero_id),
            "checked": 0,
            "queued": 0,
            "skipped_missing_source": 0,
            "errors": 0,
            "unavailable_target_schema": False,
            "details": [],
        }

        missing_tables: list[str] = []
        for table_name in ("hero_build_sources", "hero_build_clones"):
            exists = conn.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?",
                (table_name,),
            ).fetchone()
            if not exists:
                missing_tables.append(table_name)
        if missing_tables:
            summary["unavailable_target_schema"] = True
            summary["details"].append(
                {
                    "status": "unavailable_target_schema",
                    "tables": missing_tables,
                }
            )
            logger.warning(
                "Deadlock sync skipped (hero_id=%s): missing target tables %s",
                hero_id,
                ", ".join(missing_tables),
            )
            return summary

        clone_cols = {
            row["name"] for row in conn.execute("PRAGMA table_info(hero_build_clones)").fetchall()
        }
        source_cols = {
            row["name"] for row in conn.execute("PRAGMA table_info(hero_build_sources)").fetchall()
        }

        required_clone_cols = {"origin_hero_build_id", "hero_id", "target_language"}
        missing_clone_cols = sorted(required_clone_cols - clone_cols)
        if missing_clone_cols:
            summary["unavailable_target_schema"] = True
            summary["details"].append(
                {
                    "status": "unavailable_target_schema",
                    "table": "hero_build_clones",
                    "missing_columns": missing_clone_cols,
                }
            )
            logger.warning(
                "Deadlock sync skipped (hero_id=%s): hero_build_clones missing columns %s",
                hero_id,
                ", ".join(missing_clone_cols),
            )
            return summary

        source_ordering: list[str] = []
        # Whitelist of safe ORDER BY expressions - prevents SQL injection even if source_cols is compromised
        _SAFE_SOURCE_ORDER_COLS = {
            "last_seen_at": "COALESCE(last_seen_at, 0) DESC",
            "fetched_at": "COALESCE(fetched_at, 0) DESC",
            "publish_ts": "COALESCE(publish_ts, 0) DESC",
        }
        for col_name, order_expr in _SAFE_SOURCE_ORDER_COLS.items():
            if col_name in source_cols:
                source_ordering.append(order_expr)
        if not source_ordering:
            source_ordering.append("hero_build_id DESC")
        # Validate all entries are from whitelist before joining
        source_order_clause = ", ".join(source_ordering)

        active_build_rows = conn.execute(
            """
            SELECT id, hero_id, build_id, build_name, author_name, is_active, sort_order, created_at, updated_at
              FROM deadlock_hero_builds
             WHERE hero_id = ?
               AND is_active = 1
             ORDER BY sort_order ASC, build_id ASC
            """,
            (hero_id,),
        ).fetchall()
        summary["checked"] = len(active_build_rows)
        now_ts = int(time.time())

        for row in active_build_rows:
            build = self._map_deadlock_hero_build_row(row)
            build_id = int(build["build_id"])
            build_name = str(build["build_name"])
            author_name = str(build["author_name"])

            source_row = conn.execute(
                f"""
                SELECT *
                  FROM hero_build_sources
                 WHERE hero_build_id = ?
                 ORDER BY {source_order_clause}
                 LIMIT 1
                """,
                (build_id,),
            ).fetchone()
            if source_row is None:
                summary["skipped_missing_source"] += 1
                summary["details"].append(
                    {
                        "build_id": build_id,
                        "build_name": build_name,
                        "status": "skipped",
                        "reason": "missing_source",
                    }
                )
                logger.warning(
                    "Deadlock sync skipped build (hero_id=%s, build_id=%s): source missing",
                    hero_id,
                    build_id,
                )
                continue

            def _source_value(name: str, default: Any = None) -> Any:
                if name not in source_cols:
                    return default
                try:
                    return source_row[name]
                except Exception:
                    return default

            target_language = self._coerce_int(_source_value("language"), 0) or 0
            target_description = _source_value("description")
            if not target_description:
                target_description = f"Author: {author_name}"

            clone_payload: dict[str, Any] = {
                "origin_hero_build_id": build_id,
                "hero_id": int(hero_id),
                "target_language": int(target_language),
                "target_name": build_name,
                "target_description": str(target_description),
                "status": "pending",
                "status_info": "Queued by dashboard sync",
                "updated_at": now_ts,
                "attempts": 0,
                "last_attempt_at": None,
                "uploaded_build_id": None,
                "uploaded_version": None,
                "created_at": now_ts,
            }

            if "origin_build_id" in clone_cols:
                clone_payload["origin_build_id"] = _source_value("origin_build_id")
            if "author_account_id" in clone_cols:
                clone_payload["author_account_id"] = _source_value("author_account_id")
            if "source_language" in clone_cols:
                clone_payload["source_language"] = _source_value("language")
            if "source_version" in clone_cols:
                clone_payload["source_version"] = _source_value("version")
            if "source_last_updated_ts" in clone_cols:
                clone_payload["source_last_updated_ts"] = _source_value("last_updated_ts")

            clone_payload = {
                key: value for key, value in clone_payload.items() if key in clone_cols
            }

            try:
                updatable_payload = {
                    key: value
                    for key, value in clone_payload.items()
                    if key not in {"origin_hero_build_id", "target_language", "created_at"}
                }
                updated_rows = 0
                if updatable_payload:
                    # Validate all column names are safe SQL identifiers
                    safe_keys = [self._validate_sql_identifier(k) for k in updatable_payload]
                    set_sql = ", ".join(f"{key} = ?" for key in safe_keys)
                    update_params = [
                        *updatable_payload.values(),
                        build_id,
                        int(target_language),
                    ]
                    updated_rows = int(
                        conn.execute(
                            "UPDATE hero_build_clones SET "
                            + set_sql
                            + " WHERE origin_hero_build_id = ? AND target_language = ?",
                            update_params,
                        ).rowcount
                        or 0
                    )

                if updated_rows == 0:
                    insert_payload = dict(clone_payload)
                    insert_payload["origin_hero_build_id"] = build_id
                    insert_payload["target_language"] = int(target_language)
                    if "created_at" in clone_cols and "created_at" not in insert_payload:
                        insert_payload["created_at"] = now_ts
                    # Validate all column names are safe SQL identifiers
                    safe_columns = [self._validate_sql_identifier(k) for k in insert_payload.keys()]
                    columns = ", ".join(safe_columns)
                    placeholders = ", ".join("?" for _ in insert_payload)
                    insert_sql = (
                        "INSERT INTO hero_build_clones ("
                        + columns
                        + ") VALUES ("
                        + placeholders
                        + ")"
                    )
                    conn.execute(
                        insert_sql,
                        tuple(insert_payload.values()),
                    )

                summary["queued"] += 1
                summary["details"].append(
                    {
                        "build_id": build_id,
                        "build_name": build_name,
                        "status": "queued",
                    }
                )
            except Exception as exc:  # noqa: BLE001
                summary["errors"] += 1
                summary["details"].append(
                    {
                        "build_id": build_id,
                        "build_name": build_name,
                        "status": "error",
                        "reason": "queue_write_failed",
                        "error": str(exc),
                    }
                )
                logger.error(
                    "Deadlock sync queue write failed (hero_id=%s, build_id=%s): %s",
                    hero_id,
                    build_id,
                    exc,
                    exc_info=True,
                )

        logger.info(
            "Deadlock sync finished (hero_id=%s): queued=%s missing_source=%s errors=%s checked=%s",
            hero_id,
            summary["queued"],
            summary["skipped_missing_source"],
            summary["errors"],
            summary["checked"],
        )
        return summary

    async def _sync_deadlock_hero_builds(self, *, hero_id: int) -> dict[str, Any]:
        task_result = await self._run_steam_task(
            "MAINTAIN_BUILD_CATALOG",
            {"hero_id": int(hero_id)},
            timeout=180.0,
            poll_interval=1.0,
        )
        result_payload = task_result.get("result")
        result_data = result_payload.get("data") if isinstance(result_payload, dict) else {}
        if not isinstance(result_data, dict):
            result_data = {}

        summary: dict[str, Any] = {
            "hero_id": int(hero_id),
            "task_id": task_result.get("task_id"),
            "task_status": task_result.get("status"),
            "ok": bool(task_result.get("ok")),
            "timed_out": bool(task_result.get("timed_out")),
            "error": task_result.get("error"),
            "checked": int(result_data.get("checked") or 0),
            "queued": int(result_data.get("tasks_created") or 0),
            "builds_to_clone": int(result_data.get("builds_to_clone") or 0),
            "builds_to_update": int(result_data.get("builds_to_update") or 0),
            "delete_tasks_created": int(result_data.get("delete_tasks_created") or 0),
            "removed_clones": int(result_data.get("removed_clones") or 0),
            "missing_sources": int(result_data.get("missing_sources") or 0),
            "errors": int(result_data.get("errors") or 0),
            "unchanged": int(result_data.get("unchanged") or 0),
            "unavailable_target_schema": False,
            "details": [],
        }

        if summary["timed_out"]:
            summary["details"].append({"status": "timeout", "reason": summary["error"]})
        elif not summary["ok"] and summary["error"]:
            summary["details"].append({"status": "error", "reason": summary["error"]})

        if summary["ok"]:
            await self._dispatch_deadlock_missing_build_alerts()

        return summary

    async def _handle_deadlock_heroes(self, request: web.Request) -> web.Response:
        """List all Deadlock heroes including their build snapshots."""
        self._check_auth(request, required=True, require_full_access=True)
        try:
            global_target_build_name = db.get_kv("deadlock", "global_target_build_name") or ""
            hero_rows = db.query_all(
                """
                SELECT
                    id, hero_id, name, origin_build_id, target_build_name_override,
                    is_active, created_at, updated_at
                  FROM deadlock_heroes
                 ORDER BY hero_id
                """
            )
            build_rows = db.query_all(
                """
                SELECT
                    id, hero_id, build_id, build_name, author_name, is_active, sort_order,
                    sync_status, sync_message, last_checked_at, last_synced_at, last_alerted_at,
                    source_version, source_last_updated_ts, clone_build_id, clone_version,
                    created_at, updated_at
                  FROM deadlock_hero_builds
                 ORDER BY hero_id ASC, sort_order ASC, build_id ASC
                """
            )

            builds_by_hero: dict[int, list[dict[str, Any]]] = {}
            for row in build_rows:
                mapped = self._map_deadlock_hero_build_row(row)
                builds_by_hero.setdefault(int(mapped["hero_id"]), []).append(mapped)

            heroes: list[dict[str, Any]] = []
            for row in hero_rows:
                hero = self._map_deadlock_hero_row(row)
                hero_builds = builds_by_hero.get(int(hero["hero_id"]), [])
                hero["builds"] = hero_builds
                if not hero_builds:
                    suggestion = self._build_deadlock_legacy_suggestion(hero)
                    if suggestion:
                        hero["legacy_build_suggestion"] = suggestion
                heroes.append(hero)

            return self._json(
                {
                    "heroes": heroes,
                    "config": {
                        "global_target_build_name": global_target_build_name,
                    },
                }
            )
        except Exception as exc:  # noqa: BLE001
            logger.exception("Failed to load deadlock heroes: %s", exc)
            raise web.HTTPInternalServerError(text="Deadlock heroes unavailable")

    async def _handle_deadlock_config(self, request: web.Request) -> web.Response:
        self._check_auth(request, required=True, require_full_access=True)
        return self._json(
            {"global_target_build_name": (db.get_kv("deadlock", "global_target_build_name") or "")}
        )

    async def _handle_deadlock_config_update(self, request: web.Request) -> web.Response:
        self._check_auth(request, required=True, require_full_access=True)
        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON body")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Invalid JSON body")

        global_target_build_name = self._normalize_deadlock_target_name(
            payload.get(
                "global_target_build_name",
                payload.get("globalTargetBuildName"),
            ),
            field_name="global_target_build_name",
        )
        previous_global_target_build_name = str(
            db.get_kv("deadlock", "global_target_build_name") or ""
        )
        db.set_kv("deadlock", "global_target_build_name", global_target_build_name)
        self._log_deadlock_audit(
            request,
            "config.global_target_build_name",
            changes={
                "from": previous_global_target_build_name,
                "to": global_target_build_name,
            },
        )
        return self._json({"global_target_build_name": global_target_build_name})

    async def _handle_deadlock_upsert_hero(self, request: web.Request) -> web.Response:
        """Create or update a Deadlock hero entry, optionally replacing its build snapshot and syncing."""
        self._check_auth(request, required=True, require_full_access=True)
        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON body")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Invalid JSON body")

        try:
            hero_id_raw = payload.get("hero_id", payload.get("heroId"))
            hero_id = int(hero_id_raw)
        except Exception:
            raise web.HTTPBadRequest(text="hero_id must be an integer")

        name = str(payload.get("name") or "").strip()
        if not name:
            raise web.HTTPBadRequest(text="name is required")

        has_origin_build = "origin_build_id" in payload or "originBuildId" in payload
        parsed_origin_build_id: int | None = None
        if has_origin_build:
            origin_raw = payload.get("origin_build_id", payload.get("originBuildId"))
            if origin_raw in (None, "", False):
                parsed_origin_build_id = None
            else:
                parsed_origin_build_id = self._coerce_int(origin_raw, None)
                if parsed_origin_build_id is None:
                    raise web.HTTPBadRequest(text="origin_build_id must be integer or null")

        has_is_active = "is_active" in payload or "isActive" in payload
        parsed_is_active: bool | None = None
        if has_is_active:
            parsed_is_active = self._coerce_request_bool(
                payload.get("is_active", payload.get("isActive")),
                default=True,
                field_name="is_active",
            )
        has_target_build_name_override = (
            "target_build_name_override" in payload or "targetBuildNameOverride" in payload
        )
        target_build_name_override = self._normalize_deadlock_target_name(
            payload.get(
                "target_build_name_override",
                payload.get("targetBuildNameOverride"),
            ),
            field_name="target_build_name_override",
        )

        builds_snapshot = self._extract_deadlock_builds_payload(payload)
        sync_on_save = self._coerce_request_bool(
            payload.get("sync_on_save", payload.get("syncOnSave", True)),
            default=True,
            field_name="sync_on_save",
        )

        ts = int(time.time())
        saved_hero: dict[str, Any] | None = None

        try:
            async with db.transaction() as conn:
                existing = conn.execute(
                    """
                    SELECT name, origin_build_id, is_active, target_build_name_override
                      FROM deadlock_heroes
                     WHERE hero_id = ?
                    """,
                    (hero_id,),
                ).fetchone()

                origin_build_id = (
                    parsed_origin_build_id
                    if has_origin_build
                    else (existing["origin_build_id"] if existing else None)
                )
                resolved_target_build_name_override = (
                    target_build_name_override
                    if has_target_build_name_override
                    else str(existing["target_build_name_override"] or "")
                    if existing
                    else ""
                )
                is_active = (
                    bool(parsed_is_active)
                    if has_is_active and parsed_is_active is not None
                    else bool(int(existing["is_active"]))
                    if existing
                    else True
                )

                conn.execute(
                    """
                    INSERT INTO deadlock_heroes (
                        hero_id, name, origin_build_id, target_build_name_override,
                        is_active, created_at, updated_at
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(hero_id) DO UPDATE SET
                        name = excluded.name,
                        origin_build_id = excluded.origin_build_id,
                        target_build_name_override = excluded.target_build_name_override,
                        is_active = excluded.is_active,
                        updated_at = excluded.updated_at
                    """,
                    (
                        hero_id,
                        name,
                        origin_build_id,
                        resolved_target_build_name_override,
                        1 if is_active else 0,
                        ts,
                        ts,
                    ),
                )

                if builds_snapshot is not None:
                    self._apply_deadlock_build_snapshot(
                        conn,
                        hero_id=hero_id,
                        builds=builds_snapshot,
                        ts=ts,
                    )

                saved_hero = self._load_deadlock_hero_payload(hero_id, conn=conn)
        except sqlite3.IntegrityError as exc:  # noqa: BLE001
            logger.warning("Deadlock hero upsert conflict: %s", exc)
            raise web.HTTPConflict(text="Hero with same ID or name already exists")
        except Exception as exc:  # noqa: BLE001
            logger.exception("Failed to upsert deadlock hero: %s", exc)
            raise web.HTTPInternalServerError(text="Saving hero failed")

        sync_summary: dict[str, Any] | None = None
        if sync_on_save:
            sync_summary = await self._sync_deadlock_hero_builds(hero_id=hero_id)
            logger.info(
                "Deadlock hero saved+synced (hero_id=%s): queued=%s missing_sources=%s "
                "errors=%s clones=%s updates=%s deletes=%s timed_out=%s",
                hero_id,
                sync_summary.get("queued"),
                sync_summary.get("missing_sources"),
                sync_summary.get("errors"),
                sync_summary.get("builds_to_clone"),
                sync_summary.get("builds_to_update"),
                sync_summary.get("delete_tasks_created"),
                sync_summary.get("timed_out"),
            )
        else:
            logger.info("Deadlock hero saved without sync (hero_id=%s)", hero_id)

        previous_name = str(existing["name"] or "") if existing else ""
        previous_override = str(existing["target_build_name_override"] or "") if existing else ""
        self._log_deadlock_audit(
            request,
            "hero.upsert",
            hero_id=hero_id,
            changes={
                "name": {"from": previous_name, "to": name},
                "target_build_name_override": {
                    "from": previous_override,
                    "to": resolved_target_build_name_override,
                },
                "sync_on_save": sync_on_save,
                "build_count": len(builds_snapshot) if builds_snapshot is not None else None,
            },
        )

        return self._json({"hero": saved_hero, "sync_summary": sync_summary})

    async def _handle_deadlock_delete_hero(self, request: web.Request) -> web.Response:
        """Delete a hero and its associated build snapshot by hero_id."""
        self._check_auth(request, required=True, require_full_access=True)
        try:
            hero_id = int(request.match_info.get("hero_id", "0"))
        except ValueError:
            raise web.HTTPBadRequest(text="hero_id must be integer")

        try:
            async with db.transaction() as conn:
                conn.execute("DELETE FROM deadlock_hero_builds WHERE hero_id = ?", (hero_id,))
                cur = conn.execute("DELETE FROM deadlock_heroes WHERE hero_id = ?", (hero_id,))
                deleted = int(cur.rowcount or 0)
            logger.info("Deadlock hero deleted (hero_id=%s, deleted=%s)", hero_id, bool(deleted))
            sync_summary = await self._sync_deadlock_hero_builds(hero_id=hero_id)
            return self._json({"deleted": bool(deleted), "sync_summary": sync_summary})
        except Exception as exc:  # noqa: BLE001
            logger.exception("Failed to delete deadlock hero: %s", exc)
            raise web.HTTPInternalServerError(text="Delete hero failed")

    async def _handle_deadlock_sync_hero(self, request: web.Request) -> web.Response:
        """Run manual sync/copy for one hero without changing admin fields."""
        self._check_auth(request, required=True, require_full_access=True)
        try:
            hero_id = int(request.match_info.get("hero_id", "0"))
        except ValueError:
            raise web.HTTPBadRequest(text="hero_id must be integer")

        hero_exists = db.query_one("SELECT 1 FROM deadlock_heroes WHERE hero_id = ?", (hero_id,))
        if hero_exists is None:
            raise web.HTTPNotFound(text="Hero not found")

        sync_summary = await self._sync_deadlock_hero_builds(hero_id=hero_id)
        return self._json({"hero_id": hero_id, "sync_summary": sync_summary})

    def _resolve_tournament_guild_id(self, raw_value: Any) -> int:
        parsed = self._coerce_int(raw_value, None)
        if parsed is not None:
            return int(parsed)
        if self.bot.guilds:
            return int(self.bot.guilds[0].id)
        raise web.HTTPBadRequest(text="guild_id is required")

    async def _tournament_guilds_payload(self) -> list[dict[str, Any]]:
        from cogs.customgames import tournament_store as tstore

        counts = await tstore.guild_signup_counts_async()
        payload: list[dict[str, Any]] = []
        known_ids: set[int] = set()
        for guild in sorted(self.bot.guilds, key=lambda g: (g.name or "").lower()):
            guild_id = int(guild.id)
            known_ids.add(guild_id)
            payload.append(
                {
                    "id": str(guild_id),
                    "name": guild.name,
                    "signups": int(counts.get(guild_id, 0)),
                }
            )
        for guild_id, signups in sorted(counts.items(), key=lambda item: item[0]):
            gid = int(guild_id)
            if gid in known_ids:
                continue
            payload.append(
                {
                    "id": str(gid),
                    "name": f"Guild {gid}",
                    "signups": int(signups),
                }
            )
        return payload

    def _decorate_tournament_signups(
        self,
        guild_id: int,
        signups: list[dict[str, Any]],
    ) -> list[dict[str, Any]]:
        guild = self.bot.get_guild(int(guild_id))
        decorated: list[dict[str, Any]] = []
        for row in signups:
            item = dict(row)
            user_id = int(item.get("user_id") or 0)
            item["user_id"] = str(user_id)
            if "guild_id" in item:
                item["guild_id"] = str(item["guild_id"])
            member = guild.get_member(user_id) if guild else None
            item["display_name"] = member.display_name if member else f"User {user_id}"
            item["mention"] = member.mention if member else None
            rank_key = str(item.get("rank") or "initiate")
            item["rank_label"] = rank_key.capitalize()
            decorated.append(item)
        return decorated

    async def _handle_tournament_overview(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        from cogs.customgames import tournament_store as tstore

        guild_id = self._resolve_tournament_guild_id(request.query.get("guild_id"))
        teams = await tstore.list_teams_async(guild_id)
        signups = await tstore.list_signups_async(guild_id)
        summary = await tstore.summary_async(guild_id)

        payload = {
            "guild_id": guild_id,
            "guilds": await self._tournament_guilds_payload(),
            "summary": summary,
            "teams": teams,
            "signups": self._decorate_tournament_signups(guild_id, signups),
        }
        return self._json(payload)

    async def _handle_tournament_team_create(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:  # noqa: BLE001
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be a JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        name = str(payload.get("name") or "").strip()
        if not name:
            raise web.HTTPBadRequest(text="'name' is required")
        created_by = self._coerce_int(payload.get("created_by"), None)

        try:
            team = await tstore.get_or_create_team_async(guild_id, name, created_by=created_by)
        except ValueError as exc:
            logger.warning(
                "Tournament team creation rejected: %s",
                self._safe_log_value(exc),
            )
            raise web.HTTPBadRequest(text="Invalid team payload")

        return self._json(
            {
                "ok": True,
                "team": team,
                "created": bool(team.get("created")),
            },
            status=201 if bool(team.get("created")) else 200,
        )

    async def _handle_tournament_assign(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:  # noqa: BLE001
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be a JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        user_id = self._coerce_int(payload.get("user_id"), None)
        if user_id is None:
            raise web.HTTPBadRequest(text="'user_id' must be an integer")

        team_raw = payload.get("team_id")
        if team_raw in (None, ""):
            team_id: int | None = None
        else:
            team_id = self._coerce_int(team_raw, None)
            if team_id is None:
                raise web.HTTPBadRequest(text="'team_id' must be an integer or null")

        try:
            updated = await tstore.assign_signup_team_async(
                guild_id,
                int(user_id),
                team_id=team_id,
            )
        except ValueError as exc:
            logger.warning(
                "Tournament signup assignment rejected: %s",
                self._safe_log_value(exc),
            )
            raise web.HTTPBadRequest(text="Invalid team assignment payload")

        if not updated:
            raise web.HTTPNotFound(text="Signup not found")

        signup = await tstore.get_signup_async(guild_id, int(user_id))
        decorated = self._decorate_tournament_signups(guild_id, [signup]) if signup else []
        return self._json({"ok": True, "signup": decorated[0] if decorated else signup})

    async def _handle_tournament_remove(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:  # noqa: BLE001
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be a JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        user_id = self._coerce_int(payload.get("user_id"), None)
        if user_id is None:
            raise web.HTTPBadRequest(text="'user_id' must be an integer")

        removed = await tstore.remove_signup_async(guild_id, int(user_id))
        return self._json({"ok": removed, "removed": removed})

    # ─── Turnier-Seite (Community-Mods + Admins) ─────────────────────────────

    def _load_turnier_html(self) -> str:
        try:
            return _TURNIER_HTML_PATH.read_text(encoding="utf-8")
        except Exception as exc:
            logger.error("Turnier HTML nicht ladbar: %s", exc, exc_info=True)
            raise

    async def _handle_turnier_page(self, request: web.Request) -> web.Response:
        if self._auth_misconfigured:
            raise web.HTTPServiceUnavailable(text="Auth nicht konfiguriert.")
        if self._is_auth_enforced() and not self._has_valid_auth(request):
            if self._discord_auth_required:
                raise web.HTTPFound(self._build_discord_login_url(request, next_path="/turnier"))
            self._check_turnier_auth(request)
        session = self._get_discord_auth_session(request)
        display_name = str((session or {}).get("display_name") or "Nicht angemeldet")
        safe_logout = html.escape("/auth/logout", quote=True)
        safe_login = html.escape(
            self._build_discord_login_url(request, next_path="/turnier"),
            quote=True,
        )
        html_text = (
            self._load_turnier_html()
            .replace("{{AUTH_USER_LABEL}}", html.escape(display_name, quote=True))
            .replace("{{AUTH_LOGOUT_URL}}", safe_logout)
            .replace("{{DISCORD_LOGIN_URL}}", safe_login)
        )
        return web.Response(text=html_text, content_type="text/html")

    @staticmethod
    def _generate_bracket(
        signups: list[dict[str, Any]], teams: list[dict[str, Any]]
    ) -> dict[str, Any]:
        import math

        team_map = {int(t["id"]): t for t in teams}
        team_scores: dict[int, list[int]] = {}
        solo_entries: list[dict[str, Any]] = []

        for s in signups:
            rv = int(s.get("rank_value") or 1)
            rsub = int(s.get("rank_subvalue") or 3)
            score = rv * 6 + max(1, min(6, rsub))
            tid = s.get("team_id")
            if tid is not None:
                tid = int(tid)
                team_scores.setdefault(tid, []).append(score)
            else:
                solo_entries.append(
                    {
                        "type": "solo",
                        "id": f"solo_{s.get('user_id')}",
                        "name": str(s.get("display_name") or s.get("user_id")),
                        "score": score,
                    }
                )

        entries: list[dict[str, Any]] = []
        for tid, scores in team_scores.items():
            team = team_map.get(tid, {})
            avg = sum(scores) / len(scores) if scores else 0
            entries.append(
                {
                    "type": "team",
                    "id": f"team_{tid}",
                    "name": str(team.get("name") or f"Team {tid}"),
                    "score": round(avg, 2),
                    "member_count": len(scores),
                }
            )
        entries.extend(solo_entries)

        if len(entries) < 2:
            return {
                "error": "Mindestens 2 Einträge für einen Bracket benötigt.",
                "entries": entries,
                "rounds": [],
            }

        entries.sort(key=lambda e: e["score"], reverse=True)
        n = len(entries)
        slots = 1 << math.ceil(math.log2(n)) if n > 1 else 2
        num_rounds = int(math.log2(slots))

        # Seeded pairing: seed 1 vs seed N, seed 2 vs seed N-1, …
        seeded_matches = []
        for i in range(slots // 2):
            a_idx = i
            b_idx = slots - 1 - i
            a = entries[a_idx] if a_idx < n else None
            b = entries[b_idx] if b_idx < n else None
            seeded_matches.append((a, b))

        def round_label(r: int) -> str:
            rem = num_rounds - r
            if rem == 0:
                return "🏆 Finale"
            if rem == 1:
                return "🥊 Halbfinale"
            if rem == 2:
                return "⚔️ Viertelfinale"
            return f"Runde {r}"

        rounds: list[dict[str, Any]] = []
        current: list[dict[str, Any]] = []
        for i, (a, b) in enumerate(seeded_matches):
            auto_winner = None
            if a is None and b is not None:
                auto_winner = b
            elif b is None and a is not None:
                auto_winner = a
            current.append(
                {
                    "match_id": f"R1M{i + 1}",
                    "entry_a": a,
                    "entry_b": b,
                    "winner": auto_winner,
                    "is_bye": (a is None or b is None),
                }
            )
        rounds.append({"round": 1, "label": round_label(1), "matches": current})

        for r in range(2, num_rounds + 1):
            nxt = []
            for i in range(len(current) // 2):
                nxt.append(
                    {
                        "match_id": f"R{r}M{i + 1}",
                        "entry_a": None,
                        "entry_b": None,
                        "winner": None,
                        "is_bye": False,
                    }
                )
            rounds.append({"round": r, "label": round_label(r), "matches": nxt})
            current = nxt

        return {
            "entries": entries,
            "rounds": rounds,
            "num_entries": n,
            "num_rounds": num_rounds,
            "slots": slots,
        }

    async def _handle_turnier_overview(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        guild_id = self._resolve_tournament_guild_id(request.query.get("guild_id"))
        teams = await tstore.list_teams_async(guild_id)
        signups = await tstore.list_signups_async(guild_id)
        summary = await tstore.summary_async(guild_id)
        period = await tstore.get_active_period_async(guild_id)
        periods = await tstore.list_periods_async(guild_id)

        payload = {
            "guild_id": str(guild_id),
            "guilds": await self._tournament_guilds_payload(),
            "summary": summary,
            "teams": teams,
            "signups": self._decorate_tournament_signups(guild_id, signups),
            "active_period": period,
            "periods": periods,
        }
        return self._json(self._stringify_ids(payload))

    async def _handle_turnier_bracket(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        guild_id = self._resolve_tournament_guild_id(request.query.get("guild_id"))
        teams = await tstore.list_teams_async(guild_id)
        signups = self._decorate_tournament_signups(
            guild_id, await tstore.list_signups_async(guild_id)
        )
        bracket = self._generate_bracket(signups, teams)
        payload = {"guild_id": str(guild_id), "bracket": bracket}
        return self._json(self._stringify_ids(payload))

    async def _handle_turnier_period_create(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        name = str(payload.get("name") or "").strip()
        if not name:
            raise web.HTTPBadRequest(text="'name' is required")
        reg_start = str(payload.get("registration_start") or "").strip()
        reg_end = str(payload.get("registration_end") or "").strip()
        if not reg_start or not reg_end:
            raise web.HTTPBadRequest(text="'registration_start' and 'registration_end' required")
        team_size_raw = self._coerce_int(payload.get("team_size"), 6)
        team_size = max(2, min(20, int(team_size_raw or 6)))
        created_by = self._coerce_int(payload.get("created_by"), None)

        period = await tstore.create_period_async(
            guild_id,
            name=name,
            registration_start=reg_start,
            registration_end=reg_end,
            team_size=team_size,
            created_by=created_by,
        )
        return self._json(self._stringify_ids({"ok": True, "period": period}), status=201)

    async def _handle_turnier_period_close(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        period_id = self._coerce_int(payload.get("period_id"), None)
        if period_id is None:
            # Close active period
            period = await tstore.get_active_period_async(guild_id)
            if not period:
                raise web.HTTPNotFound(text="No active period found")
            period_id = int(period["id"])

        closed = await tstore.close_period_async(guild_id, int(period_id))
        return self._json(self._stringify_ids({"ok": closed, "closed": closed}))

    async def _handle_turnier_team_create(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        name = str(payload.get("name") or "").strip()
        if not name:
            raise web.HTTPBadRequest(text="'name' is required")
        created_by = self._coerce_int(payload.get("created_by"), None)

        try:
            team = await tstore.get_or_create_team_async(guild_id, name, created_by=created_by)
        except ValueError:
            # CodeQL: Information exposure through an exception. Using generic message.
            raise web.HTTPBadRequest(text="Ungültiger Team-Name oder Daten.")

        return self._json(
            self._stringify_ids({"ok": True, "team": team, "created": bool(team.get("created"))}),
            status=201 if team.get("created") else 200,
        )

    async def _handle_turnier_team_delete(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        team_id = self._coerce_int(payload.get("team_id"), None)
        if team_id is None:
            raise web.HTTPBadRequest(text="'team_id' is required")

        deleted = await tstore.delete_team_async(guild_id, int(team_id))
        if not deleted:
            raise web.HTTPNotFound(text="Team not found")
        return self._json(self._stringify_ids({"ok": True, "deleted": True}))

    async def _handle_turnier_assign(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        user_id = self._coerce_int(payload.get("user_id"), None)
        if user_id is None:
            raise web.HTTPBadRequest(text="'user_id' is required")
        team_raw = payload.get("team_id")
        team_id: int | None = None if team_raw in (None, "") else self._coerce_int(team_raw, None)

        try:
            updated = await tstore.assign_signup_team_async(guild_id, int(user_id), team_id=team_id)
        except ValueError:
            # CodeQL: Information exposure through an exception. Using generic message.
            raise web.HTTPBadRequest(text="Ungültige Zuweisung oder Team existiert nicht.")

        if not updated:
            raise web.HTTPNotFound(text="Signup not found")
        signup = await tstore.get_signup_async(guild_id, int(user_id))
        decorated = self._decorate_tournament_signups(guild_id, [signup]) if signup else []
        return self._json(
            self._stringify_ids({"ok": True, "signup": decorated[0] if decorated else signup})
        )

    async def _handle_turnier_remove(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        user_id = self._coerce_int(payload.get("user_id"), None)
        if user_id is None:
            raise web.HTTPBadRequest(text="'user_id' is required")

        removed = await tstore.remove_signup_async(guild_id, int(user_id))
        return self._json(self._stringify_ids({"ok": removed, "removed": removed}))

    async def _handle_turnier_clear(self, request: web.Request) -> web.Response:
        self._check_turnier_auth(request)
        from cogs.customgames import tournament_store as tstore

        try:
            payload = await request.json()
        except Exception:
            raise web.HTTPBadRequest(text="Invalid JSON payload")
        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be JSON object")

        guild_id = self._resolve_tournament_guild_id(payload.get("guild_id"))
        count = await tstore.clear_all_signups_async(guild_id)
        return self._json(self._stringify_ids({"ok": True, "cleared": count}))

    async def _handle_status(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        auth_session = self._auth_session_for_request(request)

        bot = self.bot
        tz = bot.startup_time.tzinfo
        now = _dt.datetime.now(tz=tz) if tz else _dt.datetime.now()
        uptime_delta = now - bot.startup_time
        uptime = str(uptime_delta).split(".")[0]

        discovered = bot.cogs_list
        status_map = bot.cog_status.copy()
        active = set(bot.active_cogs())

        items: list[dict[str, Any]] = []
        for cog in discovered:
            status = status_map.get(cog, "loaded" if cog in active else "unloaded")
            items.append(
                {
                    "name": cog,
                    "status": status,
                    "loaded": cog in active,
                    "namespace": self._namespace_for(cog),
                }
            )

        namespaces = self._namespace_summary(discovered)

        latency = getattr(bot, "latency", None)
        if latency is not None and math.isfinite(latency):
            latency_ms = round(latency * 1000, 2)
        else:
            latency_ms = None

        lifecycle_state: dict[str, Any] | None = None
        lifecycle = self._lifecycle or getattr(bot, "lifecycle", None)
        if lifecycle:
            try:
                lifecycle_state = lifecycle.snapshot()
            except Exception as exc:
                logging.getLogger(__name__).warning("Lifecycle snapshot fehlgeschlagen: %s", exc)
                lifecycle_state = {"enabled": True, "error": str(exc)}

        restart_in_progress = bool(self._restart_task and not self._restart_task.done())
        last_restart = self._last_restart if any(self._last_restart.values()) else None
        csrf_token = self._ensure_session_csrf_token(auth_session) if auth_session else None

        payload = {
            "bot": {
                "user": str(bot.user) if bot.user else None,
                "id": getattr(bot.user, "id", None),
                "uptime": uptime,
                "guilds": len(bot.guilds),
                "latency_ms": latency_ms,
            },
            "cogs": {
                "items": items,
                "active": sorted(active),
                "namespaces": namespaces,
                "discovered": discovered,
                "tree": self._build_tree(),
                "blocked": sorted(self.bot.blocked_namespaces),
            },
            "dashboard": {
                "listen_url": self._listen_base_url,
                "public_url": self._public_base_url,
                "running": self._started,
                "restart_in_progress": restart_in_progress,
                "last_restart": last_restart,
                "bot_restart_available": bool(
                    lifecycle
                    or (
                        self._nssm_service_restart_enabled
                        and os.name == "nt"
                        and bool(self._nssm_service_name)
                    )
                ),
            },
            "lifecycle": lifecycle_state or {"enabled": False},
            "settings": {
                "per_cog_unload_timeout": bot.per_cog_unload_timeout,
            },
            "auth": {
                "enforced": self._is_auth_enforced(),
                "mode": ("discord" if self._discord_auth_required else "none"),
                "discord_enabled": self._discord_auth_required,
                "login_url": self._build_discord_login_url(request, next_path="/admin"),
                "logout_url": "/auth/logout",
                "csrf_token": csrf_token,
                "user": (
                    {
                        "id": auth_session.get("user_id"),
                        "display_name": auth_session.get("display_name"),
                        "username": auth_session.get("username"),
                    }
                    if auth_session
                    else None
                ),
            },
            "health": await self._collect_health_checks(),
            "standalone": await self._collect_standalone_snapshot(),
        }
        return self._json(payload)

    async def _handle_bot_restart(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        safe_remote = self._safe_log_value(request.remote)
        logger.warning("AUDIT master-dashboard bot_restart requested from %s", safe_remote)
        async with self._bot_restart_lock:
            now = time.monotonic()
            since_last = now - self._last_bot_restart_request_monotonic
            if (
                self._last_bot_restart_request_monotonic > 0
                and since_last < self._bot_restart_min_interval_seconds
            ):
                retry_after = max(0.0, self._bot_restart_min_interval_seconds - since_last)
                return self._json(
                    {
                        "ok": False,
                        "message": f"Restart cooldown active ({retry_after:.1f}s remaining)",
                        "retry_after_seconds": round(retry_after, 1),
                    }
                )

            lifecycle = self._lifecycle or getattr(self.bot, "lifecycle", None)
            service_pid = self._query_windows_service_pid(self._nssm_service_name)
            current_pid = os.getpid()
            parent_pid = os.getppid()
            if service_pid is not None and service_pid not in {current_pid, parent_pid}:
                logger.warning(
                    "Configured NSSM service '%s' PID (%s) does not match current process tree "
                    "(current=%s, parent=%s). Continuing with hard service restart.",
                    self._safe_log_value(self._nssm_service_name),
                    service_pid,
                    current_pid,
                    parent_pid,
                )

            # Hard restart first - try systemd on Linux, NSSM on Windows
            if os.name != "nt":
                service_restart_ok, service_message = self._schedule_systemd_service_restart()
                if service_restart_ok:
                    self._last_bot_restart_request_monotonic = now
                    return self._json(
                        {
                            "ok": True,
                            "message": f"Hard restart requested via systemd: {service_message}",
                            "restart_mode": "systemd_service",
                        }
                    )
                logger.warning(
                    "systemd service restart unavailable: %s",
                    self._safe_log_value(service_message),
                )
            else:
                service_restart_ok, service_message = self._schedule_nssm_service_restart()
                if service_restart_ok:
                    self._last_bot_restart_request_monotonic = now
                    return self._json(
                        {
                            "ok": True,
                            "message": f"Hard restart requested via NSSM: {service_message}",
                            "restart_mode": "nssm_service",
                        }
                    )
                logger.warning(
                    "NSSM service restart unavailable: %s",
                    self._safe_log_value(service_message),
                )

            if not lifecycle:
                return self._json(
                    {
                        "ok": False,
                        "message": (
                            "Restart unavailable (NSSM failed and no lifecycle fallback attached): "
                            f"{service_message}"
                        ),
                    }
                )

            scheduled = await lifecycle.request_restart(
                reason="dashboard_lifecycle_after_nssm_failure"
            )
            if scheduled:
                self._last_bot_restart_request_monotonic = now
                return self._json(
                    {
                        "ok": True,
                        "message": (
                            "Hard restart unavailable; lifecycle fallback scheduled: "
                            f"{service_message}"
                        ),
                        "restart_mode": "lifecycle_fallback",
                    }
                )
            return self._json(
                {
                    "ok": False,
                    "message": "Restart already pending (lifecycle fallback path)",
                    "restart_mode": "lifecycle_fallback",
                }
            )

    async def _handle_dashboard_restart(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        if self._restart_task and not self._restart_task.done():
            return self._json({"ok": True, "message": "Dashboard restart already running"})

        self._restart_task = asyncio.create_task(self._restart_dashboard())
        self._restart_task.add_done_callback(self._on_restart_finished)
        return self._json({"ok": True, "message": "Dashboard restart scheduled"})

    async def _collect_health_checks(self) -> list[dict[str, Any]]:
        if not self._health_targets:
            return []
        now = asyncio.get_running_loop().time()
        if self._health_cache and now < self._health_cache_expiry:
            return self._health_cache
        async with self._health_cache_lock:
            if self._health_cache and now < self._health_cache_expiry:
                return self._health_cache
            data = await self._refresh_health_checks()
            self._health_cache = data
            self._health_cache_expiry = now + self._health_cache_ttl
            return data

    async def _refresh_health_checks(self) -> list[dict[str, Any]]:
        timeout = ClientTimeout(total=self._health_timeout)
        async with ClientSession(timeout=timeout) as session:
            tasks = [self._probe_health_target(session, target) for target in self._health_targets]
            return await asyncio.gather(*tasks)

    async def _probe_health_target(
        self,
        session: ClientSession,
        target: dict[str, Any],
    ) -> dict[str, Any]:
        url = target.get("url") or ""
        method = (target.get("method") or "GET").strip().upper() or "GET"
        allow_redirects_value = target.get("allow_redirects")
        allow_redirects = True
        coerced_redirects = self._coerce_bool(allow_redirects_value)
        if coerced_redirects is not None:
            allow_redirects = coerced_redirects

        verify_ssl_value = target.get("verify_ssl")
        ssl_param: Any = None
        coerced_ssl = self._coerce_bool(verify_ssl_value)
        if coerced_ssl is False:
            ssl_param = False

        timeout_value = target.get("timeout")
        request_timeout = None
        if timeout_value is not None:
            try:
                parsed_timeout = float(timeout_value)
                if parsed_timeout > 0:
                    request_timeout = ClientTimeout(total=parsed_timeout)
            except (TypeError, ValueError):
                logging.warning(
                    "Healthcheck target '%s' timeout '%s' invalid – falling back to default",
                    target.get("label") or target.get("key") or url,
                    timeout_value,
                )

        expected_status = target.get("expect_status")

        def _status_ok(status_code: int) -> bool:
            if expected_status is None:
                return 200 <= status_code < 400
            if isinstance(expected_status, int):
                return status_code == expected_status
            if isinstance(expected_status, (list, tuple, set)):
                try:
                    allowed = {int(item) for item in expected_status}
                except (TypeError, ValueError):
                    allowed = set(expected_status)
                return status_code in allowed
            if isinstance(expected_status, str):
                stripped = expected_status.strip()
                if stripped.isdigit():
                    return status_code == int(stripped)
            return 200 <= status_code < 400

        start = time.perf_counter()
        status: int | None = None
        reason: str | None = None
        ok = False
        error: str | None = None
        resolved_url = url
        body_excerpt: str | None = None

        request_kwargs: dict[str, Any] = {"allow_redirects": allow_redirects}
        if ssl_param is not None:
            request_kwargs["ssl"] = ssl_param
        if request_timeout:
            request_kwargs["timeout"] = request_timeout

        try:
            async with session.request(method, url, **request_kwargs) as resp:
                status = resp.status
                reason = resp.reason
                resolved_url = str(resp.url)
                ok = _status_ok(status)
                if not ok:
                    try:
                        text = await resp.text()
                    except Exception:
                        text = ""
                    if text:
                        body_excerpt = text[:280]
        except Exception as exc:
            error = f"{exc.__class__.__name__}: {exc}"

        duration_ms = round((time.perf_counter() - start) * 1000, 2)
        result: dict[str, Any] = {
            "key": target.get("key"),
            "label": target.get("label") or target.get("key") or url,
            "url": url,
            "method": method,
            "ok": ok,
            "status": status,
            "reason": reason,
            "latency_ms": duration_ms,
            "checked_at": _dt.datetime.utcnow().isoformat() + "Z",
        }
        if resolved_url and resolved_url != url:
            result["resolved_url"] = resolved_url
        if error:
            result["error"] = error
        if body_excerpt and not ok:
            result["body_excerpt"] = body_excerpt
        return result

    async def _collect_standalone_snapshot(self) -> list[dict[str, Any]]:
        manager = getattr(self.bot, "standalone_manager", None)
        if not manager:
            return []
        try:
            return await manager.snapshot()
        except Exception as exc:
            logging.getLogger(__name__).error("Standalone snapshot failed: %s", exc)
            return []

    def _require_standalone_manager(self):
        manager = getattr(self.bot, "standalone_manager", None)
        if not manager:
            raise web.HTTPNotFound(text="Standalone manager unavailable")
        return manager

    def _namespace_for(self, module: str) -> str:
        parts = module.split(".")
        if len(parts) >= 3:
            return ".".join(parts[:3])
        if len(parts) >= 2:
            return ".".join(parts[:2])
        return module

    def _namespace_summary(self, modules: Iterable[str]) -> list[dict[str, Any]]:
        counter: dict[str, int] = {}
        for mod in modules:
            ns = self._namespace_for(mod)
            counter[ns] = counter.get(ns, 0) + 1
        return [{"namespace": ns, "count": counter[ns]} for ns in sorted(counter.keys())]

    def _build_tree(self) -> dict[str, Any]:
        bot = self.bot
        root_dir = bot.cogs_dir
        active = set(bot.active_cogs())
        discovered = set(bot.cogs_list)
        status_map = bot.cog_status.copy()

        def is_manageable(path: str) -> bool:
            if path == "cogs":
                return False
            return path in active or path in discovered or path in status_map

        def node_status(path: str, *, blocked: bool) -> str | None:
            status = status_map.get(path)
            if status:
                return status
            if blocked:
                return "blocked"
            if path in active:
                return "loaded"
            if path in discovered:
                return "unloaded"
            return None

        def walk(directory: Path, parts: list[str]) -> dict[str, Any]:
            module_path = "cogs"
            if parts:
                module_path = "cogs." + ".".join(parts)

            blocked_dir = bot.is_namespace_blocked(module_path, assume_normalized=True)
            status = node_status(module_path, blocked=blocked_dir)
            manageable_dir = is_manageable(module_path)
            loaded_dir = module_path in active
            discovered_dir = module_path in discovered
            is_package = (
                module_path in discovered or module_path in status_map or module_path in active
            ) and module_path != "cogs"

            module_count = 1 if is_package else 0
            loaded_count = 1 if is_package and loaded_dir else 0
            discovered_count = 1 if is_package and discovered_dir else 0

            children: list[dict[str, Any]] = []
            try:
                entries = sorted(directory.iterdir(), key=lambda p: (p.is_file(), p.name))
            except FileNotFoundError:
                entries = []

            for entry in entries:
                if entry.name.startswith("__pycache__"):
                    continue
                if entry.is_dir():
                    child = walk(entry, parts + [entry.name])
                    children.append(child)
                    module_count += child.get("module_count", 0)
                    loaded_count += child.get("loaded_count", 0)
                    discovered_count += child.get("discovered_count", 0)
                    continue
                if entry.suffix != ".py" or entry.name == "__init__.py":
                    continue
                if parts:
                    mod_path = "cogs." + ".".join(parts + [entry.stem])
                else:
                    mod_path = f"cogs.{entry.stem}"
                blocked_child = bot.is_namespace_blocked(mod_path, assume_normalized=True)
                loaded_child = mod_path in active
                discovered_child = mod_path in discovered
                manageable_child = is_manageable(mod_path)
                status_child = node_status(mod_path, blocked=blocked_child) or "not_discovered"
                child = {
                    "type": "module",
                    "name": entry.stem,
                    "path": mod_path,
                    "blocked": blocked_child,
                    "loaded": loaded_child,
                    "discovered": discovered_child,
                    "manageable": manageable_child,
                    "status": status_child,
                }
                children.append(child)
                module_count += 1
                if loaded_child:
                    loaded_count += 1
                if discovered_child:
                    discovered_count += 1

            return {
                "type": "directory",
                "name": directory.name if parts else "cogs",
                "path": module_path,
                "blocked": blocked_dir,
                "status": status,
                "is_package": is_package,
                "manageable": manageable_dir,
                "loaded": loaded_dir,
                "discovered": discovered_dir,
                "module_count": module_count,
                "loaded_count": loaded_count,
                "discovered_count": discovered_count,
                "children": children,
            }

        if not root_dir.exists():
            return {
                "type": "directory",
                "name": "cogs",
                "path": "cogs",
                "blocked": bot.is_namespace_blocked("cogs"),
                "status": None,
                "is_package": False,
                "manageable": False,
                "loaded": False,
                "discovered": False,
                "module_count": 0,
                "loaded_count": 0,
                "discovered_count": 0,
                "children": [],
            }

        return walk(root_dir, [])

    async def _handle_reload(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        payload = await request.json()
        names = payload.get("names") or []
        if not isinstance(names, list) or not names:
            raise web.HTTPBadRequest(text="'names' must be a non-empty list")
        targets = self._normalize_manage_targets(names)
        safe_names = self._safe_log_value(targets)
        safe_remote = self._safe_log_value(request.remote)
        logger.info(
            "AUDIT master-dashboard cog_reload: names=%s from %s",
            safe_names,
            safe_remote,
        )

        results: dict[str, dict[str, Any]] = {}
        async with self._lock:
            for name in targets:
                if self.bot.is_namespace_blocked(name, assume_normalized=True):
                    results[name] = {
                        "ok": False,
                        "message": f"🚫 {name} ist blockiert",
                    }
                    continue
                if name not in self.bot.extensions:
                    results[name] = {
                        "ok": False,
                        "message": f"{name} is not loaded",
                    }
                    continue
                ok, message = await self.bot.reload_cog(name)
                results[name] = {"ok": ok, "message": message}
        return self._json({"results": results})

    async def _handle_load(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        payload = await request.json()
        names = payload.get("names") or []
        if not isinstance(names, list) or not names:
            raise web.HTTPBadRequest(text="'names' must be a non-empty list")
        self.bot.auto_discover_cogs()
        targets = self._normalize_manage_targets(names)

        results: dict[str, dict[str, Any]] = {}
        async with self._lock:
            for name in targets:
                if self.bot.is_namespace_blocked(name, assume_normalized=True):
                    results[name] = {
                        "ok": False,
                        "message": f"🚫 {name} ist blockiert",
                    }
                    continue
                ok, message = await self.bot.reload_cog(name)
                results[name] = {"ok": ok, "message": message}
        return self._json({"results": results})

    async def _handle_unload(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        payload = await request.json()
        names = payload.get("names") or []
        if not isinstance(names, list) or not names:
            raise web.HTTPBadRequest(text="'names' must be a non-empty list")
        targets = self._normalize_manage_targets(names)
        safe_names = self._safe_log_value(targets)
        safe_remote = self._safe_log_value(request.remote)
        logger.warning(
            "AUDIT master-dashboard cog_unload: names=%s from %s",
            safe_names,
            safe_remote,
        )

        results: dict[str, dict[str, Any]] = {}
        async with self._lock:
            unload_result = await self.bot.unload_many(targets)
            for name in targets:
                status = unload_result.get(name, "unknown")
                if status == "unloaded":
                    results[name] = {"ok": True, "message": f"✅ Unloaded {name}"}
                elif status == "timeout":
                    results[name] = {
                        "ok": False,
                        "message": f"⏱️ Timeout unloading {name}",
                    }
                elif status.startswith("error"):
                    results[name] = {"ok": False, "message": status}
                else:
                    results[name] = {"ok": False, "message": status}
        return self._json({"results": results})

    async def _handle_reload_all(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        async with self._lock:
            ok, summary = await self.bot.reload_all_cogs_with_discovery()
        if ok:
            return self._json({"ok": True, "summary": summary})
        raise web.HTTPInternalServerError(text=str(summary))

    async def _handle_reload_namespace(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        payload = await request.json()
        namespace = payload.get("namespace")
        if not namespace:
            raise web.HTTPBadRequest(text="'namespace' is required")

        try:
            normalized = self.bot.normalize_namespace(namespace)
        except ValueError:
            raise web.HTTPBadRequest(text="Invalid namespace")

        if self.bot.is_namespace_blocked(normalized, assume_normalized=True):
            return self._json(
                {
                    "ok": False,
                    "results": {},
                    "message": f"{normalized} ist blockiert",
                }
            )

        async with self._lock:
            results = await self.bot.reload_namespace(normalized)
        ok = all(v in ("loaded", "reloaded") for v in results.values())
        if not results:
            message = f"Keine Cogs unter {normalized} gefunden"
        else:
            message = f"Reloaded {len(results)} cogs under {normalized}"
        return self._json({"ok": ok, "results": results, "message": message})

    async def _handle_discover(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        before = set(self.bot.cogs_list)
        self.bot.auto_discover_cogs()
        after = set(self.bot.cogs_list)
        new = sorted(after - before)
        return self._json({"ok": True, "new": new, "count": len(after)})

    async def _handle_block(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        payload = await request.json()
        path = payload.get("path")
        if not path:
            raise web.HTTPBadRequest(text="'path' is required")
        safe_path = self._safe_log_value(path)
        safe_remote = self._safe_log_value(request.remote)
        logger.warning("AUDIT master-dashboard cog_block: path=%s from %s", safe_path, safe_remote)
        async with self._lock:
            try:
                result = await self.bot.block_namespace(path)
            except ValueError:
                raise web.HTTPBadRequest(text="Invalid namespace")
        namespace = result.get("namespace", path)
        changed = result.get("changed", False)
        unloaded = result.get("unloaded", {})
        message = f"🚫 {namespace} blockiert" if changed else f"{namespace} war bereits blockiert"
        return self._json(
            {
                "ok": True,
                "namespace": namespace,
                "changed": changed,
                "unloaded": unloaded,
                "message": message,
            }
        )

    async def _handle_unblock(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        payload = await request.json()
        path = payload.get("path")
        if not path:
            raise web.HTTPBadRequest(text="'path' is required")
        safe_path = self._safe_log_value(path)
        safe_remote = self._safe_log_value(request.remote)
        logger.info(
            "AUDIT master-dashboard cog_unblock: path=%s from %s",
            safe_path,
            safe_remote,
        )
        async with self._lock:
            try:
                result = await self.bot.unblock_namespace(path)
            except ValueError:
                raise web.HTTPBadRequest(text="Invalid namespace")
        namespace = result.get("namespace", path)
        changed = result.get("changed", False)
        message = f"✅ {namespace} freigegeben" if changed else f"{namespace} war nicht blockiert"
        return self._json(
            {
                "ok": True,
                "namespace": namespace,
                "changed": changed,
                "message": message,
            }
        )

    async def _handle_log_index(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        entries = self._list_log_files()
        return self._json({"logs": entries})

    async def _handle_log_read(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        name = request.match_info.get("name", "")
        lines_raw = request.query.get("lines")
        try:
            lines = int(lines_raw) if lines_raw else LOG_TAIL_DEFAULT_LINES
            if lines <= 0:
                raise ValueError
            lines = min(lines, LOG_TAIL_MAX_LINES)
        except ValueError:
            raise web.HTTPBadRequest(
                text=f"lines must be a positive integer <= {LOG_TAIL_MAX_LINES}"
            )
        path = self._resolve_log_file(name)
        try:
            stat = path.stat()
            entries = self._tail_log_lines(path, lines)
        except OSError as exc:
            logging.getLogger(__name__).exception(
                "Failed reading log file %s: %s", self._safe_log_value(name), exc
            )
            raise web.HTTPInternalServerError(text="Failed to read log file")
        modified = _dt.datetime.fromtimestamp(
            stat.st_mtime,
            tz=_dt.UTC,
        ).isoformat()
        return self._json(
            {
                "name": path.name,
                "size": stat.st_size,
                "modified": modified,
                "lines": entries,
            }
        )

    async def _handle_standalone_list(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        data = await self._collect_standalone_snapshot()
        return self._json({"bots": data})

    async def _handle_standalone_logs(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        manager = self._require_standalone_manager()
        key = request.match_info.get("key", "").strip()
        limit_raw = request.query.get("limit")
        try:
            limit = int(limit_raw) if limit_raw else 200
            if limit <= 0:
                raise ValueError
            limit = min(limit, 1000)
        except ValueError:
            raise web.HTTPBadRequest(text="limit must be a positive integer <= 1000")
        try:
            logs = await manager.logs(key, limit=limit)
        except Exception as exc:
            if StandaloneConfigNotFound and isinstance(exc, StandaloneConfigNotFound):
                raise web.HTTPNotFound(text="Standalone bot not found")
            raise
        return self._json({"logs": logs})

    async def _handle_standalone_start(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        manager = self._require_standalone_manager()
        key = request.match_info.get("key", "").strip()
        safe_key = self._safe_log_value(key)
        try:
            status = await manager.start(key)
        except Exception as exc:
            if StandaloneAlreadyRunning and isinstance(exc, StandaloneAlreadyRunning):
                status = await manager.status(key)
            elif StandaloneConfigNotFound and isinstance(exc, StandaloneConfigNotFound):
                raise web.HTTPNotFound(text="Standalone bot not found")
            elif StandaloneManagerError and isinstance(exc, StandaloneManagerError):
                logging.getLogger(__name__).exception(
                    "Error when starting standalone bot (key=%s)", safe_key
                )
                raise web.HTTPInternalServerError(text="An internal error has occurred.")
            else:
                logging.getLogger(__name__).exception(
                    "Unexpected error when starting standalone bot (key=%s)", safe_key
                )
                raise web.HTTPInternalServerError(text="An internal error has occurred.")
        return self._json({"standalone": status})

    async def _handle_standalone_stop(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        manager = self._require_standalone_manager()
        key = request.match_info.get("key", "").strip()
        safe_key = self._safe_log_value(key)
        try:
            status = await manager.stop(key)
        except Exception as exc:
            if StandaloneNotRunning and isinstance(exc, StandaloneNotRunning):
                status = await manager.status(key)
            elif StandaloneConfigNotFound and isinstance(exc, StandaloneConfigNotFound):
                raise web.HTTPNotFound(text="Standalone bot not found")
            elif StandaloneManagerError and isinstance(exc, StandaloneManagerError):
                logging.getLogger(__name__).exception(
                    "Error when stopping standalone bot (key=%s)", safe_key
                )
                raise web.HTTPInternalServerError(text="An internal error has occurred.")
            else:
                logging.getLogger(__name__).exception(
                    "Unexpected error when stopping standalone bot (key=%s)", safe_key
                )
                raise web.HTTPInternalServerError(text="An internal error has occurred.")
        return self._json({"standalone": status})

    async def _handle_standalone_restart(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        manager = self._require_standalone_manager()
        key = request.match_info.get("key", "").strip()
        safe_key = self._safe_log_value(key)
        try:
            status = await manager.restart(key)
        except Exception as exc:
            if StandaloneConfigNotFound and isinstance(exc, StandaloneConfigNotFound):
                raise web.HTTPNotFound(text="Standalone bot not found")
            if StandaloneManagerError and isinstance(exc, StandaloneManagerError):
                logging.getLogger(__name__).exception(
                    "Error when restarting standalone bot (key=%s)", safe_key
                )
                raise web.HTTPInternalServerError(text="An internal error has occurred.")
            logging.getLogger(__name__).exception(
                "Unexpected error when restarting standalone bot (key=%s)", safe_key
            )
            raise web.HTTPInternalServerError(text="An internal error has occurred.")
        return self._json({"standalone": status})

    async def _handle_standalone_autostart(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        manager = self._require_standalone_manager()
        key = request.match_info.get("key", "").strip()
        try:
            manager.config(key)
        except Exception as exc:
            if StandaloneConfigNotFound and isinstance(exc, StandaloneConfigNotFound):
                raise web.HTTPNotFound(text="Standalone bot not found")
            raise

        try:
            payload = await request.json()
        except Exception as exc:  # noqa: BLE001
            if isinstance(exc, web.HTTPException):
                raise
            raise web.HTTPBadRequest(text="Invalid JSON payload")

        if not isinstance(payload, dict):
            raise web.HTTPBadRequest(text="Payload must be a JSON object")

        enabled_raw = payload.get("enabled")
        enabled: bool | None
        if isinstance(enabled_raw, bool):
            enabled = enabled_raw
        elif isinstance(enabled_raw, (int, float)):
            enabled = bool(enabled_raw)
        elif isinstance(enabled_raw, str):
            lowered = enabled_raw.strip().lower()
            if lowered in {"1", "true", "yes", "on"}:
                enabled = True
            elif lowered in {"0", "false", "no", "off"}:
                enabled = False
            else:
                enabled = None
        else:
            enabled = None

        if enabled is None:
            raise web.HTTPBadRequest(text="'enabled' must be a boolean")

        try:
            status = await manager.set_autostart(key, enabled)
        except Exception as exc:
            if StandaloneConfigNotFound and isinstance(exc, StandaloneConfigNotFound):
                raise web.HTTPNotFound(text="Standalone bot not found")
            raise

        return self._json({"standalone": status})

    async def _handle_standalone_command(self, request: web.Request) -> web.Response:
        self._check_auth(request)
        manager = self._require_standalone_manager()
        key = request.match_info.get("key", "").strip()
        safe_key = self._safe_log_value(key)
        try:
            manager.config(key)
        except Exception as exc:
            if StandaloneConfigNotFound and isinstance(exc, StandaloneConfigNotFound):
                raise web.HTTPNotFound(text="Standalone bot not found")
            raise

        payload = await request.json()
        command = str(payload.get("command") or "").strip()
        if not command:
            raise web.HTTPBadRequest(text="'command' is required")
        command_payload = payload.get("payload")
        try:
            payload_json = (
                json.dumps(command_payload, ensure_ascii=False)
                if command_payload is not None
                else None
            )
        except (TypeError, ValueError):
            raise web.HTTPBadRequest(text="payload must be JSON-serializable")

        db.execute(
            "INSERT INTO standalone_commands(bot, command, payload, status, created_at) "
            "VALUES(?, ?, ?, 'pending', CURRENT_TIMESTAMP)",
            (key, command, payload_json),
        )
        row = db.query_one("SELECT last_insert_rowid()")
        command_id = row[0] if row else None

        try:
            await manager.ensure_running(key)
        except Exception as exc:
            logging.getLogger(__name__).warning(
                "Could not ensure %s running after command enqueue: %s",
                safe_key,
                self._safe_log_value(exc),
            )

        status = await manager.status(key)
        return self._json(
            {
                "queued": command_id,
                "standalone": status,
            },
            status=201,
        )

    # ------------------------------------------------------------------
    # Public endpoints (no auth required)
    # ------------------------------------------------------------------

    async def _handle_public_guild_stats(self, request: web.Request) -> web.Response:
        """Public endpoint for live guild stats (no auth required)."""
        headers = {
            "Access-Control-Allow-Origin": "https://deutsche-deadlock-community.de",
            "Access-Control-Allow-Methods": "GET",
            "Cache-Control": "public, max-age=30",
        }

        # Cache check (30 seconds)
        now = time.time()
        if self._public_stats_cache and (now - self._public_stats_cache_time) < 30:
            return web.json_response(self._public_stats_cache, headers=headers)

        # Get guild data
        if not self.bot.guilds:
            return web.json_response(
                {"error": "No guild data available"},
                status=503,
                headers=headers,
            )

        # Use first/primary guild
        guild = self.bot.guilds[0]

        # Count members in voice channels
        voice_count = sum(len(vc.members) for vc in guild.voice_channels if vc.members)

        # Build response
        try:
            import discord as _discord

            online_count = guild.approximate_presence_count or sum(
                1 for m in guild.members if m.status != _discord.Status.offline
            )
        except Exception:
            online_count = guild.approximate_presence_count or 0

        data = {
            "member_count": guild.member_count or len(guild.members),
            "online_count": online_count,
            "voice_count": voice_count,
            "cached_at": _dt.datetime.now(_dt.UTC).isoformat(),
        }

        # Update cache
        self._public_stats_cache = data
        self._public_stats_cache_time = now

        return web.json_response(data, headers=headers)

    async def _handle_public_patch_notes(self, request: web.Request) -> web.Response:
        """Public endpoint: all Deadlock patch notes in German, no auth required."""
        headers = {
            "Access-Control-Allow-Origin": "https://deutsche-deadlock-community.de",
            "Access-Control-Allow-Methods": "GET",
            "Cache-Control": "public, max-age=120",
        }

        import asyncio as _asyncio

        from service import db as _db

        def _detect_sections(content: str) -> list[str]:
            sections = []
            lower = content.lower()
            if any(kw in lower for kw in ["## allgemein", "## general"]):
                sections.append("allgemein")
            if "## items" in lower:
                sections.append("items")
            if any(kw in lower for kw in ["## helden", "## heroes"]):
                sections.append("helden")
            return sections

        def _fetch() -> list[dict]:
            rows = _db.query_all(
                """
                SELECT id, title, url, posted_at, translated_content
                FROM changelog_posts
                WHERE translated_content IS NOT NULL AND translated_content != ''
                ORDER BY id DESC
                """
            )
            return [
                {
                    "id": row["id"],
                    "title": row["title"] or "",
                    "url": row["url"] or "",
                    "posted_at": row["posted_at"] or "",
                    "translated_content": row["translated_content"] or "",
                    "sections": _detect_sections(row["translated_content"] or ""),
                }
                for row in rows
            ]

        try:
            loop = _asyncio.get_event_loop()
            patches = await loop.run_in_executor(None, _fetch)
            return web.json_response({"patches": patches, "total": len(patches)}, headers=headers)
        except Exception as exc:
            return web.json_response({"error": str(exc)}, status=500, headers=headers)

    async def _handle_public_cors(self, request: web.Request) -> web.Response:
        """CORS preflight handler for public endpoints."""
        return web.Response(
            headers={
                "Access-Control-Allow-Origin": "https://deutsche-deadlock-community.de",
                "Access-Control-Allow-Methods": "GET, OPTIONS",
                "Access-Control-Allow-Headers": "Content-Type",
                "Access-Control-Max-Age": "86400",
            }
        )


if TYPE_CHECKING:  # pragma: no cover - avoid runtime dependency cycle
    from main_bot import MasterBot
