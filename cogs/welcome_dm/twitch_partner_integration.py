from __future__ import annotations

import json
import logging
import os
import sys
from dataclasses import dataclass
from ipaddress import ip_address
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urlsplit, urlunsplit
from urllib.request import Request, urlopen

log = logging.getLogger("StreamerOnboarding.TwitchIntegration")

TWITCH_INTERNAL_API_BASE_PATH = "/internal/twitch/v1"
TWITCH_INTERNAL_TOKEN_HEADER = "X-Internal-Token"
DEFAULT_TWITCH_INTERNAL_API_HOST = "127.0.0.1"
DEFAULT_TWITCH_INTERNAL_API_PORT = 8776
TWITCH_INTERNAL_API_TOKEN_ENV_NAMES = (
    "TWITCH_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
)


class TwitchPartnerIntegrationUnavailable(RuntimeError):
    """Raised when the external Deadlock-Twitch-Bot integration cannot be used."""


@dataclass(frozen=True, slots=True)
class TwitchPartnerAuthState:
    twitch_login: str | None
    twitch_user_id: str | None
    authorized: bool


@dataclass(frozen=True, slots=True)
class _ExternalModules:
    repo_path: Path
    raid_auth_manager_cls: Any
    raid_integration_state_resolver_cls: Any
    default_redirect_uri: str


_EXTERNAL_MODULES: _ExternalModules | None = None
_AUTH_MANAGER: Any | None = None


def _normalize_login(value: object) -> str:
    return str(value or "").strip().lower()


def _env_port(name: str, default: int) -> int:
    raw = (os.getenv(name) or "").strip()
    if not raw:
        return default
    try:
        parsed = int(raw)
    except ValueError:
        log.warning("Invalid %s=%r, using %s", name, raw, default)
        return default
    if parsed <= 0 or parsed > 65535:
        log.warning("Out-of-range %s=%r, using %s", name, raw, default)
        return default
    return parsed


def _env_float(name: str, default: float) -> float:
    raw = (os.getenv(name) or "").strip()
    if not raw:
        return default
    try:
        return max(0.5, float(raw))
    except ValueError:
        log.warning("Invalid %s=%r, using %.1f", name, raw, default)
        return default


def _env_bool(name: str, default: bool = False) -> bool:
    raw = (os.getenv(name) or "").strip().lower()
    if not raw:
        return default
    if raw in {"1", "true", "yes", "on"}:
        return True
    if raw in {"0", "false", "no", "off"}:
        return False
    return default


def _is_loopback_host(host: str) -> bool:
    normalized = str(host or "").strip().lower().rstrip(".")
    if not normalized:
        return False
    if normalized == "localhost":
        return True
    try:
        return ip_address(normalized).is_loopback
    except ValueError:
        return False


def _normalize_internal_base_url(value: str, *, allow_non_loopback: bool = False) -> str:
    raw = str(value or "").strip()
    if not raw:
        raise ValueError("base_url is required")
    if "://" not in raw:
        raw = f"http://{raw}"

    parsed = urlsplit(raw)
    if not parsed.scheme or not parsed.netloc:
        raise ValueError("base_url is invalid")
    if parsed.username or parsed.password:
        raise ValueError("base_url must not contain credentials")

    host = (parsed.hostname or "").strip()
    if not host:
        raise ValueError("base_url is invalid")
    if not allow_non_loopback and not _is_loopback_host(host):
        raise ValueError("base_url host must resolve to loopback unless explicitly allowed")

    path = (parsed.path or "").rstrip("/")
    internal_base = TWITCH_INTERNAL_API_BASE_PATH.rstrip("/")
    if path == internal_base:
        path = ""
    elif path.endswith(internal_base):
        path = path[: -len(internal_base)]

    return urlunsplit(((parsed.scheme or "http").lower(), parsed.netloc, path.rstrip("/"), "", ""))


def _internal_api_config() -> tuple[str, str, float] | None:
    token = ""
    for env_name in TWITCH_INTERNAL_API_TOKEN_ENV_NAMES:
        token = (os.getenv(env_name) or "").strip()
        if token:
            break
    if not token:
        return None

    base_url = (os.getenv("TWITCH_INTERNAL_API_BASE_URL") or "").strip()
    if not base_url:
        host = (
            os.getenv("TWITCH_INTERNAL_API_HOST") or DEFAULT_TWITCH_INTERNAL_API_HOST
        ).strip() or DEFAULT_TWITCH_INTERNAL_API_HOST
        port = _env_port("TWITCH_INTERNAL_API_PORT", DEFAULT_TWITCH_INTERNAL_API_PORT)
        base_url = f"http://{host}:{port}"

    allow_non_loopback = _env_bool("TWITCH_INTERNAL_API_ALLOW_NON_LOOPBACK", False)
    timeout_seconds = _env_float("TWITCH_INTERNAL_API_TIMEOUT_SEC", 5.0)
    try:
        normalized_base = _normalize_internal_base_url(
            base_url,
            allow_non_loopback=allow_non_loopback,
        )
    except ValueError as exc:
        raise TwitchPartnerIntegrationUnavailable(
            f"Twitch Internal API Base-URL ist ungueltig: {exc}"
        ) from exc
    return normalized_base, token, timeout_seconds


def _request_internal_api_json(path: str, query: dict[str, object | None]) -> dict[str, Any]:
    config = _internal_api_config()
    if config is None:
        raise TwitchPartnerIntegrationUnavailable("Kein Twitch-Internal-API-Token ist gesetzt.")

    base_url, token, timeout_seconds = config
    query_clean = {
        key: str(value) for key, value in query.items() if value is not None and str(value).strip()
    }
    query_string = urlencode(query_clean)
    url = f"{base_url.rstrip('/')}/{path.lstrip('/')}"
    if query_string:
        url = f"{url}?{query_string}"

    request = Request(url, headers={TWITCH_INTERNAL_TOKEN_HEADER: token}, method="GET")
    try:
        with urlopen(request, timeout=timeout_seconds) as response:  # noqa: S310
            raw = response.read().decode("utf-8")
    except HTTPError as exc:
        body = ""
        try:
            body = exc.read().decode("utf-8", errors="replace")
        except Exception:
            body = ""
        raise TwitchPartnerIntegrationUnavailable(
            f"Twitch Internal API antwortete mit HTTP {exc.code}: {body[:200]}"
        ) from exc
    except (TimeoutError, URLError, OSError) as exc:
        raise TwitchPartnerIntegrationUnavailable(
            f"Twitch Internal API ist nicht erreichbar: {exc}"
        ) from exc

    try:
        payload = json.loads(raw or "{}")
    except json.JSONDecodeError as exc:
        raise TwitchPartnerIntegrationUnavailable(
            "Twitch Internal API lieferte keine gueltige JSON-Antwort."
        ) from exc
    if not isinstance(payload, dict):
        raise TwitchPartnerIntegrationUnavailable(
            "Twitch Internal API lieferte ein unerwartetes Antwortformat."
        )
    return payload


def _prefer_internal_api() -> bool:
    return _internal_api_config() is not None


def _candidate_repo_paths() -> list[Path]:
    candidates: list[Path] = []

    configured = (os.getenv("DEADLOCK_TWITCH_BOT_DIR") or "").strip()
    if configured:
        candidates.append(Path(configured).expanduser())

    base = Path(__file__).resolve()
    candidates.extend(
        [
            base.parents[3] / "Deadlock-Twitch-Bot",
            base.parents[2].parent / "Deadlock-Twitch-Bot",
        ]
    )

    unique: list[Path] = []
    seen: set[Path] = set()
    for candidate in candidates:
        try:
            resolved = candidate.resolve()
        except Exception:
            continue
        if resolved in seen:
            continue
        seen.add(resolved)
        unique.append(resolved)
    return unique


def _resolve_repo_path() -> Path:
    for candidate in _candidate_repo_paths():
        if candidate.is_dir():
            return candidate
    searched = ", ".join(str(path) for path in _candidate_repo_paths()) or "<none>"
    raise TwitchPartnerIntegrationUnavailable(
        f"Deadlock-Twitch-Bot wurde nicht gefunden. Gepruefte Pfade: {searched}"
    )


def _load_external_modules() -> _ExternalModules:
    global _EXTERNAL_MODULES
    if _EXTERNAL_MODULES is not None:
        return _EXTERNAL_MODULES

    repo_path = _resolve_repo_path()
    repo_path_str = str(repo_path)
    if repo_path_str not in sys.path:
        sys.path.insert(0, repo_path_str)

    try:
        from bot.core.constants import TWITCH_RAID_REDIRECT_URI
        from bot.raid.auth import RaidAuthManager
        from bot.raid.integration_state import RaidIntegrationStateResolver
    except Exception as exc:
        raise TwitchPartnerIntegrationUnavailable(
            "Deadlock-Twitch-Bot konnte nicht geladen werden."
        ) from exc

    _EXTERNAL_MODULES = _ExternalModules(
        repo_path=repo_path,
        raid_auth_manager_cls=RaidAuthManager,
        raid_integration_state_resolver_cls=RaidIntegrationStateResolver,
        default_redirect_uri=str(TWITCH_RAID_REDIRECT_URI or "").strip(),
    )
    return _EXTERNAL_MODULES


def _auth_manager() -> Any:
    global _AUTH_MANAGER
    if _AUTH_MANAGER is not None:
        return _AUTH_MANAGER

    modules = _load_external_modules()
    client_id = (os.getenv("TWITCH_CLIENT_ID") or "").strip()
    client_secret = (os.getenv("TWITCH_CLIENT_SECRET") or "").strip()
    redirect_uri = (os.getenv("TWITCH_RAID_REDIRECT_URI") or modules.default_redirect_uri).strip()

    if not client_id or not client_secret or not redirect_uri:
        raise TwitchPartnerIntegrationUnavailable(
            "Twitch OAuth ist nicht konfiguriert "
            "(TWITCH_CLIENT_ID / TWITCH_CLIENT_SECRET / TWITCH_RAID_REDIRECT_URI)."
        )

    try:
        _AUTH_MANAGER = modules.raid_auth_manager_cls(
            client_id=client_id,
            client_secret=client_secret,
            redirect_uri=redirect_uri,
        )
    except TypeError:
        _AUTH_MANAGER = modules.raid_auth_manager_cls(client_id, client_secret, redirect_uri)
    except Exception as exc:
        raise TwitchPartnerIntegrationUnavailable(
            "RaidAuthManager aus Deadlock-Twitch-Bot konnte nicht initialisiert werden."
        ) from exc
    return _AUTH_MANAGER


def generate_discord_auth_url(discord_user_id: int) -> str:
    if _prefer_internal_api():
        payload = _request_internal_api_json(
            f"{TWITCH_INTERNAL_API_BASE_PATH}/raid/auth-url",
            {
                "login": f"discord:{int(discord_user_id)}",
                "discord_user_id": int(discord_user_id),
            },
        )
        auth_url = str(payload.get("auth_url") or "").strip()
        if not auth_url:
            raise TwitchPartnerIntegrationUnavailable(
                "Twitch Internal API hat keinen gueltigen Auth-Link geliefert."
            )
        return auth_url

    auth_url = str(
        _auth_manager().generate_discord_button_url(
            f"discord:{discord_user_id}",
            discord_user_id=discord_user_id,
        )
        or ""
    )
    auth_url = auth_url.strip()
    if not auth_url:
        raise TwitchPartnerIntegrationUnavailable(
            "Deadlock-Twitch-Bot hat keinen gueltigen Auth-Link geliefert."
        )
    return auth_url


def get_auth_state(discord_user_id: int) -> TwitchPartnerAuthState:
    if _prefer_internal_api():
        payload = _request_internal_api_json(
            f"{TWITCH_INTERNAL_API_BASE_PATH}/raid/auth-state",
            {"discord_user_id": int(discord_user_id)},
        )
        return TwitchPartnerAuthState(
            twitch_login=_normalize_login(payload.get("twitch_login")) or None,
            twitch_user_id=str(payload.get("twitch_user_id") or "").strip() or None,
            authorized=bool(payload.get("authorized")),
        )

    modules = _load_external_modules()
    manager = _auth_manager()
    discord_id = str(discord_user_id)

    try:
        resolver = modules.raid_integration_state_resolver_cls(
            auth_manager=manager,
            token_error_handler=getattr(manager, "token_error_handler", None),
        )
        state = resolver.resolve_auth_state(discord_id)
        return TwitchPartnerAuthState(
            twitch_login=_normalize_login(state.twitch_login) or None,
            twitch_user_id=str(state.twitch_user_id or "").strip() or None,
            authorized=bool(state.authorized),
        )
    except Exception as exc:
        raise TwitchPartnerIntegrationUnavailable(
            "Autorisierungsstatus aus Deadlock-Twitch-Bot konnte nicht gelesen werden."
        ) from exc


def check_onboarding_blocklist(
    *,
    discord_user_id: int | None = None,
    twitch_login: str | None = None,
) -> tuple[bool, str | None]:
    if _prefer_internal_api():
        payload = _request_internal_api_json(
            f"{TWITCH_INTERNAL_API_BASE_PATH}/raid/block-state",
            {
                "discord_user_id": discord_user_id,
                "twitch_login": _normalize_login(twitch_login) or None,
            },
        )
        if bool(payload.get("partner_opt_out")):
            blocked_login = (
                _normalize_login(payload.get("twitch_login"))
                or _normalize_login(twitch_login)
                or "unbekannt"
            )
            return True, f"manual_partner_opt_out=1 fuer {blocked_login}"
        if bool(payload.get("token_blacklisted")):
            blocked_user_id = str(payload.get("twitch_user_id") or "").strip() or "unbekannt"
            return True, f"twitch_token_blacklist fuer {blocked_user_id}"
        if bool(payload.get("raid_blacklisted")):
            blocked_login = (
                _normalize_login(payload.get("twitch_login"))
                or _normalize_login(twitch_login)
                or "unbekannt"
            )
            return True, f"twitch_raid_blacklist fuer {blocked_login}"
        return False, None

    modules = _load_external_modules()
    normalized_login = _normalize_login(twitch_login) or None
    normalized_discord_id = str(discord_user_id) if discord_user_id is not None else None

    try:
        resolver = modules.raid_integration_state_resolver_cls(
            auth_manager=_AUTH_MANAGER,
            token_error_handler=getattr(_AUTH_MANAGER, "token_error_handler", None),
        )
        state = resolver.resolve_block_state(
            discord_user_id=normalized_discord_id,
            twitch_login=normalized_login,
        )
        if state.partner_opt_out:
            blocked_login = _normalize_login(state.twitch_login) or normalized_login or "unbekannt"
            return True, f"manual_partner_opt_out=1 fuer {blocked_login}"
        if state.token_blacklisted:
            blocked_user_id = str(state.twitch_user_id or "").strip() or "unbekannt"
            return True, f"twitch_token_blacklist fuer {blocked_user_id}"
        if state.raid_blacklisted:
            blocked_login = _normalize_login(state.twitch_login) or normalized_login or "unbekannt"
            return True, f"twitch_raid_blacklist fuer {blocked_login}"
    except Exception as exc:
        raise TwitchPartnerIntegrationUnavailable(
            "Opt-out-/Blacklist-Status aus Deadlock-Twitch-Bot konnte nicht gelesen werden."
        ) from exc

    return False, None


__all__ = [
    "TwitchPartnerAuthState",
    "TwitchPartnerIntegrationUnavailable",
    "check_onboarding_blocklist",
    "generate_discord_auth_url",
    "get_auth_state",
]
