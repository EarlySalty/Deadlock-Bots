from __future__ import annotations

import asyncio
import json
import logging
import os
from ipaddress import ip_address
from typing import Any
from urllib.parse import urlsplit, urlunsplit

import aiohttp
import discord
from discord.ext import commands

from service.http_client import build_resilient_connector

log = logging.getLogger(__name__)

TWITCH_INTERNAL_API_BASE_PATH = "/internal/twitch/v1"
TWITCH_INTERNAL_TOKEN_HEADER = "X-Internal-Token"
TWITCH_IDEMPOTENCY_HEADER = "Idempotency-Key"
TWITCH_LIVE_BUTTON_LABEL = "Auf Twitch ansehen"


class TwitchLiveBridgeApiError(RuntimeError):
    """Raised when the Twitch internal API cannot be used safely."""


def _env_port(name: str, default: int) -> int:
    raw = (os.getenv(name) or "").strip()
    if not raw:
        return default
    try:
        value = int(raw)
    except ValueError:
        log.warning("Invalid %s=%r, using %s", name, raw, default)
        return default
    if value <= 0 or value > 65535:
        log.warning("Out-of-range %s=%r, using %s", name, raw, default)
        return default
    return value


def _env_float(name: str, default: float) -> float:
    raw = (os.getenv(name) or "").strip()
    if not raw:
        return default
    try:
        value = float(raw)
    except ValueError:
        log.warning("Invalid %s=%r, using %.1f", name, raw, default)
        return default
    return max(0.5, value)


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


def _normalize_base_url(value: str, *, allow_non_loopback: bool = False) -> str:
    raw = (value or "").strip()
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

    return urlunsplit(
        (
            (parsed.scheme or "http").lower(),
            parsed.netloc,
            path.rstrip("/"),
            "",
            "",
        )
    )


def _normalize_streamer_login(value: str) -> str:
    normalized = str(value or "").strip().lower()
    if not normalized:
        raise ValueError("streamer_login is required")
    return normalized


def _validate_referral_url(value: str) -> str:
    url = str(value or "").strip()
    parsed = urlsplit(url)
    if not url or parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ValueError("referral_url is invalid")
    return url


def _normalize_tracking_token(value: str) -> str:
    normalized = str(value or "").strip()
    if not normalized or len(normalized) > 128:
        raise ValueError("tracking_token is invalid")
    return normalized


def _normalize_button_label(value: str) -> str:
    normalized = str(value or "").strip() or TWITCH_LIVE_BUTTON_LABEL
    return normalized[:80]


def _coerce_positive_int(value: Any, *, field_name: str) -> int:
    try:
        parsed = int(str(value or "").strip())
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{field_name} is invalid") from exc
    if parsed <= 0:
        raise ValueError(f"{field_name} is invalid")
    return parsed


class TwitchLiveInternalApiClient:
    """Small client for the Twitch worker's internal live-tracking endpoints."""

    def __init__(
        self,
        *,
        base_url: str,
        token: str,
        allow_non_loopback: bool = False,
        timeout_seconds: float = 10.0,
        session: aiohttp.ClientSession | None = None,
    ) -> None:
        self._base_url = _normalize_base_url(
            base_url,
            allow_non_loopback=allow_non_loopback,
        )
        self._token = str(token or "").strip()
        if not self._token:
            raise ValueError("token is required")
        self._timeout_seconds = max(0.5, float(timeout_seconds or 10.0))
        self._session = session
        self._owns_session = session is None

    @classmethod
    def from_env(
        cls,
        *,
        session: aiohttp.ClientSession | None = None,
    ) -> TwitchLiveInternalApiClient | None:
        token = (os.getenv("TWITCH_INTERNAL_API_TOKEN") or "").strip()
        if not token:
            return None

        base_url = (os.getenv("TWITCH_INTERNAL_API_BASE_URL") or "").strip()
        if not base_url:
            host = (os.getenv("TWITCH_INTERNAL_API_HOST") or "127.0.0.1").strip() or "127.0.0.1"
            port = _env_port("TWITCH_INTERNAL_API_PORT", 8776)
            base_url = f"http://{host}:{port}"

        allow_non_loopback = _env_bool("TWITCH_INTERNAL_API_ALLOW_NON_LOOPBACK", False)
        timeout_seconds = _env_float("TWITCH_INTERNAL_API_TIMEOUT_SEC", 10.0)
        return cls(
            base_url=base_url,
            token=token,
            allow_non_loopback=allow_non_loopback,
            timeout_seconds=timeout_seconds,
            session=session,
        )

    async def close(self) -> None:
        if self._owns_session and self._session is not None and not self._session.closed:
            await self._session.close()

    async def _ensure_session(self) -> aiohttp.ClientSession:
        if self._session is not None and not self._session.closed:
            return self._session
        connector = build_resilient_connector()
        timeout = aiohttp.ClientTimeout(total=self._timeout_seconds)
        self._session = aiohttp.ClientSession(connector=connector, timeout=timeout)
        self._owns_session = True
        return self._session

    async def _request_json(
        self,
        method: str,
        path: str,
        *,
        payload: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
    ) -> Any:
        session = await self._ensure_session()
        request_headers = {TWITCH_INTERNAL_TOKEN_HEADER: self._token}
        if headers:
            request_headers.update(headers)

        url = f"{self._base_url.rstrip('/')}/{path.lstrip('/')}"
        request_kwargs: dict[str, Any] = {"headers": request_headers}
        if payload is not None:
            request_kwargs["json"] = payload
        response = await session.request(method, url, **request_kwargs)
        try:
            text = await response.text()
        finally:
            response.release()

        try:
            body = json.loads(text) if text.strip() else {}
        except json.JSONDecodeError:
            body = None

        if 200 <= response.status < 300:
            return body if body is not None else {}

        message = ""
        if isinstance(body, dict):
            error = body.get("error")
            if isinstance(error, dict):
                message = str(error.get("message") or "").strip()
            if not message:
                message = str(body.get("message") or "").strip()
        if not message:
            message = f"Twitch internal API request failed with status {response.status}"
        raise TwitchLiveBridgeApiError(message)

    async def get_active_live_announcements(self) -> list[dict[str, Any]]:
        payload = await self._request_json(
            "GET",
            f"{TWITCH_INTERNAL_API_BASE_PATH}/live/active-announcements",
        )
        if not isinstance(payload, list):
            raise TwitchLiveBridgeApiError("active live announcements payload is invalid")

        required_keys = {
            "streamer_login",
            "message_id",
            "tracking_token",
            "referral_url",
            "button_label",
            "channel_id",
        }
        entries: list[dict[str, Any]] = []
        for item in payload:
            if not isinstance(item, dict) or not required_keys.issubset(item.keys()):
                raise TwitchLiveBridgeApiError("active live announcement entry is invalid")
            entries.append(dict(item))
        return entries

    async def record_live_link_click(
        self,
        *,
        streamer_login: str,
        tracking_token: str,
        discord_user_id: str | int,
        discord_username: str,
        guild_id: str | int | None,
        channel_id: str | int,
        message_id: str | int,
        source_hint: str,
        idempotency_key: str | None = None,
    ) -> dict[str, Any]:
        payload = {
            "streamer_login": _normalize_streamer_login(streamer_login),
            "tracking_token": _normalize_tracking_token(tracking_token),
            "discord_user_id": str(
                _coerce_positive_int(discord_user_id, field_name="discord_user_id")
            ),
            "discord_username": str(discord_username or "").strip(),
            "guild_id": (
                str(_coerce_positive_int(guild_id, field_name="guild_id"))
                if guild_id is not None and str(guild_id).strip()
                else None
            ),
            "channel_id": str(_coerce_positive_int(channel_id, field_name="channel_id")),
            "message_id": str(_coerce_positive_int(message_id, field_name="message_id")),
            "source_hint": str(source_hint or "").strip() or "discord_button",
        }
        extra_headers = (
            {TWITCH_IDEMPOTENCY_HEADER: str(idempotency_key).strip()}
            if idempotency_key is not None and str(idempotency_key).strip()
            else None
        )
        response = await self._request_json(
            "POST",
            f"{TWITCH_INTERNAL_API_BASE_PATH}/live/link-click",
            payload=payload,
            headers=extra_headers,
        )
        if not isinstance(response, dict):
            raise TwitchLiveBridgeApiError("live link click response is invalid")
        return response


class TwitchReferralLinkView(discord.ui.View):
    """Ephemeral view with the direct Twitch link."""

    def __init__(self, referral_url: str, *, button_label: str | None = None) -> None:
        super().__init__(timeout=60)
        self.add_item(
            discord.ui.Button(
                label=_normalize_button_label(button_label or TWITCH_LIVE_BUTTON_LABEL),
                style=discord.ButtonStyle.link,
                url=_validate_referral_url(referral_url),
            )
        )


class _TrackedTwitchButton(discord.ui.Button):
    def __init__(self, parent: TwitchLiveTrackingView, *, custom_id: str, label: str) -> None:
        super().__init__(
            label=_normalize_button_label(label),
            style=discord.ButtonStyle.primary,
            custom_id=custom_id,
        )
        self._parent_view = parent

    async def callback(self, interaction: discord.Interaction) -> None:  # type: ignore[override]
        await self._parent_view.handle_click(interaction)


class TwitchLiveTrackingView(discord.ui.View):
    """Persistent live-tracking button owned by the Deadlock master."""

    def __init__(
        self,
        *,
        cog: TwitchLiveBridgeCog,
        streamer_login: str,
        referral_url: str,
        tracking_token: str,
        button_label: str,
    ) -> None:
        super().__init__(timeout=None)
        self.cog = cog
        self.streamer_login = _normalize_streamer_login(streamer_login)
        self.referral_url = _validate_referral_url(referral_url)
        self.tracking_token = _normalize_tracking_token(tracking_token)
        self.button_label = _normalize_button_label(button_label)
        self.channel_id: int | None = None
        self.message_id: int | None = None

        custom_id = self.build_custom_id(self.streamer_login, self.tracking_token)
        self.add_item(_TrackedTwitchButton(self, custom_id=custom_id, label=self.button_label))

    @staticmethod
    def build_custom_id(streamer_login: str, tracking_token: str) -> str:
        login_part = "".join(ch for ch in streamer_login.lower() if ch.isalnum())[:24] or "stream"
        token_part = (tracking_token or "")[:32] or "track"
        return f"twitch-live:{login_part}:{token_part}"

    def bind_to_message(self, *, channel_id: int | None, message_id: int | None) -> None:
        self.channel_id = channel_id if channel_id and channel_id > 0 else None
        self.message_id = message_id if message_id and message_id > 0 else None

    async def handle_click(self, interaction: discord.Interaction) -> None:
        await self.cog.handle_tracking_click(interaction, self)


class TwitchLiveBridgeCog(commands.Cog):
    """Master-side bridge for Twitch live tracking views and restart rehydration."""

    def __init__(
        self,
        bot: commands.Bot,
        *,
        api_client: TwitchLiveInternalApiClient | None = None,
    ) -> None:
        self.bot = bot
        self._api_client = api_client
        self._owns_client = False
        self._previous_resolver: Any = None
        self._resolver_callback: Any = None
        self._resolver_installed = False
        self._restore_task: asyncio.Task[None] | None = None
        self._restore_retry_delays: tuple[float, ...] = (1.0, 2.0, 5.0, 10.0, 30.0)

    async def cog_load(self) -> None:
        if self._api_client is None:
            try:
                self._api_client = TwitchLiveInternalApiClient.from_env()
            except Exception as exc:
                log.warning("Twitch live bridge internal API config is invalid: %s", exc)
                self._api_client = None
            self._owns_client = self._api_client is not None

        if self._api_client is None:
            log.warning(
                "Twitch live bridge disabled: missing or unsafe Twitch internal API configuration."
            )
            return

        self._previous_resolver = getattr(self.bot, "resolve_master_broker_view_spec", None)
        self._resolver_callback = self.resolve_master_broker_view_spec
        self.bot.resolve_master_broker_view_spec = self._resolver_callback
        self._resolver_installed = True

        try:
            restored = await self._restore_active_announcements()
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            log.warning(
                "Twitch live bridge initial rehydration failed: %s. Scheduling retry.",
                exc,
            )
            self._restore_task = asyncio.create_task(
                self._restore_active_announcements_with_retry(),
                name="deadlock.twitch_live_bridge.rehydrate",
            )
        else:
            log.info("Twitch live bridge restored %s active announcement view(s)", restored)

    async def cog_unload(self) -> None:
        current = getattr(self.bot, "resolve_master_broker_view_spec", None)
        if self._resolver_installed and current is self._resolver_callback:
            if self._previous_resolver is None:
                try:
                    delattr(self.bot, "resolve_master_broker_view_spec")
                except AttributeError:
                    pass
            else:
                self.bot.resolve_master_broker_view_spec = self._previous_resolver
        self._resolver_installed = False
        self._resolver_callback = None

        if self._restore_task is not None:
            self._restore_task.cancel()
            try:
                await self._restore_task
            except asyncio.CancelledError:
                pass  # Awaiting a cancelled restore task during unload is expected.
            self._restore_task = None

        if self._owns_client and self._api_client is not None:
            await self._api_client.close()

    def resolve_master_broker_view_spec(self, view_spec: dict[str, Any]) -> discord.ui.View:
        if self._api_client is None:
            raise TwitchLiveBridgeApiError(
                "twitch live bridge is not configured for the internal API"
            )
        view_type = str(view_spec.get("type") or "").strip()
        if view_type != "twitch_live_tracking":
            raise ValueError("unsupported master broker view type")
        return TwitchLiveTrackingView(
            cog=self,
            streamer_login=str(view_spec.get("streamer_login") or ""),
            referral_url=str(view_spec.get("referral_url") or ""),
            tracking_token=str(view_spec.get("tracking_token") or ""),
            button_label=str(view_spec.get("button_label") or TWITCH_LIVE_BUTTON_LABEL),
        )

    async def _restore_active_announcements(self) -> int:
        if self._api_client is None:
            raise TwitchLiveBridgeApiError("twitch live bridge is not configured")

        announcements = await self._api_client.get_active_live_announcements()

        restored = 0
        for item in announcements:
            try:
                channel_id = _coerce_positive_int(item.get("channel_id"), field_name="channel_id")
                message_id = _coerce_positive_int(item.get("message_id"), field_name="message_id")
                view = self.resolve_master_broker_view_spec(
                    {
                        "type": "twitch_live_tracking",
                        "streamer_login": item.get("streamer_login"),
                        "tracking_token": item.get("tracking_token"),
                        "referral_url": item.get("referral_url"),
                        "button_label": item.get("button_label"),
                    }
                )
                bind_method = getattr(view, "bind_to_message", None)
                if callable(bind_method):
                    bind_method(channel_id=channel_id, message_id=message_id)
                self.bot.add_view(view, message_id=message_id)
                restored += 1
            except Exception as exc:
                log.warning("Skipping invalid Twitch live announcement during restore: %s", exc)
        return restored

    async def _restore_active_announcements_with_retry(self) -> None:
        delays = self._restore_retry_delays or (1.0,)
        attempt = 0
        while True:
            try:
                restored = await self._restore_active_announcements()
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                delay = delays[min(attempt, len(delays) - 1)]
                attempt += 1
                log.warning(
                    "Twitch live bridge rehydration retry %s failed: %s. Retrying in %.1fs.",
                    attempt,
                    exc,
                    delay,
                )
                await asyncio.sleep(delay)
                continue

            log.info("Twitch live bridge restored %s active announcement view(s)", restored)
            self._restore_task = None
            return

    async def handle_tracking_click(
        self,
        interaction: discord.Interaction,
        view: TwitchLiveTrackingView,
    ) -> None:
        try:
            if not interaction.response.is_done():
                await interaction.response.defer(ephemeral=True)
        except Exception:
            log.debug("Could not defer Twitch live interaction", exc_info=True)

        channel_source = interaction.channel_id or view.channel_id
        message_source = (
            getattr(getattr(interaction, "message", None), "id", None) or view.message_id
        )
        guild_source = interaction.guild_id

        if self._api_client is not None and channel_source and message_source:
            try:
                await self._api_client.record_live_link_click(
                    streamer_login=view.streamer_login,
                    tracking_token=view.tracking_token,
                    discord_user_id=interaction.user.id,
                    discord_username=str(interaction.user),
                    guild_id=guild_source,
                    channel_id=channel_source,
                    message_id=message_source,
                    source_hint="discord_button",
                    idempotency_key=f"twitch-live-click-{interaction.id}",
                )
            except Exception:
                log.exception(
                    "Twitch live bridge could not persist click (streamer=%s message=%s user=%s)",
                    view.streamer_login,
                    message_source,
                    getattr(interaction.user, "id", None),
                )

        content = f"Hier ist dein Twitch-Link für **{view.streamer_login}**."
        response_view = TwitchReferralLinkView(
            view.referral_url,
            button_label=view.button_label,
        )
        try:
            if interaction.response.is_done():
                await interaction.followup.send(content, view=response_view, ephemeral=True)
            else:
                await interaction.response.send_message(content, view=response_view, ephemeral=True)
        except Exception:
            log.exception("Twitch live bridge could not send referral response")


async def setup(bot: commands.Bot) -> None:
    await bot.add_cog(TwitchLiveBridgeCog(bot))
