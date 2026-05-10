from __future__ import annotations

import asyncio
import datetime as _dt
import hashlib
import json
import logging
import os
import sys
import time
from pathlib import Path
from typing import TYPE_CHECKING, Any

import discord
import pytz
from discord.ext import commands

from bot_core.boot_profile import log_event, measure
from bot_core.bootstrap import _init_db_if_available, _log_secret_present
from bot_core.cog_loader import CogLoaderMixin
from bot_core.logging_setup import LoggingMixin
from bot_core.presence import PresenceMixin
from bot_core.runtime_mode import ensure_gateway_start_allowed, resolve_runtime_mode
from bot_core.standalone import StandaloneMixin
from service.config import settings
from service.http_client import build_resilient_connector
from service.master_broker import MasterBroker

try:
    from service.dashboard import DashboardServer
except Exception as _dashboard_import_error:
    DashboardServer = None  # type: ignore[assignment]
    logging.getLogger(__name__).warning("Dashboard module unavailable: %s", _dashboard_import_error)

__all__ = ["MasterBot"]

if TYPE_CHECKING:
    from bot_core.lifecycle import BotLifecycle


class MasterBot(LoggingMixin, CogLoaderMixin, PresenceMixin, StandaloneMixin, commands.Bot):
    """
    Master Discord Bot mit:
     - Auto-Discovery + Blocklist
     - Reload/Unload Helper
     - Presence/Voice Router
     - Standalone Manager Hooks
     - Dashboard als Cog
    """

    def __init__(self, lifecycle: BotLifecycle | None = None):
        self.runtime_mode = ensure_gateway_start_allowed(resolve_runtime_mode())

        intents = discord.Intents.default()
        intents.message_content = True
        intents.members = True
        intents.voice_states = True
        intents.guilds = True

        connector = build_resilient_connector()

        super().__init__(
            command_prefix=settings.command_prefix,
            intents=intents,
            description="Master Bot System - Verwaltet alle Bot-Funktionen",
            owner_id=settings.owner_id,
            case_insensitive=True,
            chunk_guilds_at_startup=False,
            max_messages=1000,
            member_cache_flags=discord.MemberCacheFlags.from_intents(intents),
            connector=connector,
        )

        self.lifecycle = lifecycle
        self.root_dir = Path(__file__).resolve().parent.parent
        self._boot_started_at = time.perf_counter()

        self.setup_logging()

        # Unterstütze zusätzliche Cog-Quellen (z.B. ausgelagerter Steam-Bot)
        self.cogs_dir = self.root_dir / "cogs"
        extra_dirs: list[Path] = []

        steam_env = (os.getenv("STEAM_COGS_DIR") or "").strip()
        if steam_env:
            extra_dirs.append(Path(steam_env).expanduser())

        default_external_candidates = [
            Path(os.path.expandvars(r"%USERPROFILE%"))
            / "Documents"
            / "Deadlock-Steam-Bot"
            / "cogs",
            self.root_dir.parent / "Deadlock-Steam-Bot" / "cogs",
        ]
        for default_external in default_external_candidates:
            if default_external.exists():
                extra_dirs.append(default_external)

        for raw in (os.getenv("EXTRA_COG_DIRS") or "").split(os.pathsep):
            item = raw.strip()
            if not item:
                continue
            extra_dirs.append(Path(item).expanduser())

        self.extra_cogs_dirs: list[Path] = []
        seen_dirs: set[Path] = set()
        for candidate in extra_dirs:
            try:
                resolved = candidate.resolve()
            except Exception:
                continue
            if not resolved.is_dir() or resolved in seen_dirs:
                continue
            seen_dirs.add(resolved)
            self.extra_cogs_dirs.append(resolved)
            parent = resolved.parent
            if parent and str(parent) not in sys.path:
                # Externe Cogs sollen bei Imports bevorzugt werden
                sys.path.insert(0, str(parent))

        # Falls das cogs-Paket bereits importiert wurde (z.B. während bootstrap),
        # erweitern wir den Suchpfad nachträglich um externe Cog-Verzeichnisse.
        try:  # pragma: no cover - runtime behavior
            import cogs as cogs_pkg

            for path in [self.cogs_dir, *self.extra_cogs_dirs]:
                try:
                    resolved = Path(path).resolve()
                except Exception:
                    continue
                if resolved.is_dir():
                    pstr = str(resolved)
                    if pstr not in cogs_pkg.__path__:
                        cogs_pkg.__path__.append(pstr)
        except Exception:
            # Wenn cogs noch nicht importiert wurde, ist nichts zu tun.
            pass

        blocklist_path = os.getenv("COG_BLOCKLIST_FILE")
        if blocklist_path:
            self.blocklist_path = Path(blocklist_path)
        else:
            self.blocklist_path = self.cogs_dir.parent / "cog_blocklist.json"
        self.blocked_namespaces = set()
        self._load_blocklist()

        self.cogs_list = []
        self.cog_status = {}
        self.auto_discover_cogs()

        tz = pytz.timezone("Europe/Berlin")
        self.startup_time = _dt.datetime.now(tz=tz)

        # Dashboard is now loaded as a Cog (cogs/dashboard_cog.py)
        self.dashboard: DashboardServer | None = None  # Set by DashboardCog
        self._dashboard_start_task: asyncio.Task[None] | None = (
            None  # Legacy, kept for compatibility
        )

        self.standalone_manager = None
        self.setup_standalone_manager()

        try:
            self.per_cog_unload_timeout = float(os.getenv("PER_COG_UNLOAD_TIMEOUT", "3.0"))
        except ValueError:
            self.per_cog_unload_timeout = 3.0

        # Central app-command sync guard (prevents concurrent sync storms).
        self._command_sync_lock = asyncio.Lock()
        self.master_broker: MasterBroker | None = None
        self._init_master_broker()

    @staticmethod
    def _env_bool(name: str, default: bool = False) -> bool:
        raw = os.getenv(name)
        if raw is None:
            return default
        return raw.strip().lower() in {"1", "true", "yes", "y", "on"}

    @staticmethod
    def _env_port(name: str, default: int) -> int:
        raw = (os.getenv(name) or "").strip()
        if not raw:
            return default
        try:
            parsed = int(raw)
        except ValueError:
            logging.warning("Invalid %s=%r, using %s", name, raw, default)
            return default
        if parsed <= 0 or parsed > 65535:
            logging.warning("Out-of-range %s=%r, using %s", name, raw, default)
            return default
        return parsed

    @staticmethod
    def _master_broker_token() -> str:
        for key in (
            "MASTER_BROKER_TOKEN",
            "MAIN_BOT_INTERNAL_TOKEN",
            "TWITCH_INTERNAL_API_TOKEN",
        ):
            value = (os.getenv(key) or "").strip()
            if value:
                return value
        return ""

    def _init_master_broker(self) -> None:
        if self.runtime_mode.role != "master":
            return

        token = self._master_broker_token()
        if not token:
            logging.warning(
                "Master broker disabled: missing token "
                "(MASTER_BROKER_TOKEN/MAIN_BOT_INTERNAL_TOKEN/TWITCH_INTERNAL_API_TOKEN)."
            )
            return

        host = (os.getenv("MASTER_BROKER_HOST") or "127.0.0.1").strip() or "127.0.0.1"
        port = self._env_port("MASTER_BROKER_PORT", 8770)
        try:
            self.master_broker = MasterBroker(self, token=token, host=host, port=port)
        except Exception as exc:
            logging.error("Master broker init failed: %s", exc, exc_info=True)
            self.master_broker = None

    async def _ensure_master_broker_view_resolver(self) -> bool:
        if self.runtime_mode.role != "master" or self.master_broker is None:
            return False

        resolver = getattr(self, "resolve_master_broker_view_spec", None)
        if callable(resolver):
            return True

        bridge_extension = "cogs.twitch.live_bridge"
        if bridge_extension in self.extensions:
            return callable(getattr(self, "resolve_master_broker_view_spec", None))

        try:
            await self.load_extension(bridge_extension)
        except commands.ExtensionAlreadyLoaded:
            pass  # Bridge extension loading is idempotent.
        except Exception as exc:
            logging.warning(
                "Could not load Twitch live bridge required by master broker: %s",
                exc,
                exc_info=True,
            )
            return False

        if bridge_extension in self.cogs_list:
            self.cogs_list = [name for name in self.cogs_list if name != bridge_extension]
        self.cog_status[bridge_extension] = "loaded"

        resolver = getattr(self, "resolve_master_broker_view_spec", None)
        if callable(resolver):
            logging.info("Loaded Twitch live bridge for master broker view resolution.")
            return True

        logging.warning(
            "Twitch live bridge loaded but master broker view resolver is still unavailable."
        )
        return False

    @staticmethod
    def _parse_id_list(raw: str) -> list[int]:
        ids: list[int] = []
        for token in raw.replace(",", " ").split():
            value = token.strip()
            if not value.isdigit():
                continue
            num = int(value)
            if num > 0 and num not in ids:
                ids.append(num)
        return ids

    def _command_sync_mode(self) -> str:
        raw = (os.getenv("COMMAND_SYNC_MODE") or "hybrid").strip().lower()
        if raw in {"disabled", "off", "none", "0", "false"}:
            return "disabled"
        if raw in {"always", "force", "legacy"}:
            return "always"
        return "hybrid"

    def _command_sync_state_path(self) -> Path:
        override = (os.getenv("COMMAND_SYNC_STATE_FILE") or "").strip()
        if override:
            return Path(override).expanduser()
        return self.root_dir / "logs" / "command_sync_state.json"

    def _read_command_sync_state(self) -> dict[str, Any]:
        path = self._command_sync_state_path()
        try:
            if not path.exists():
                return {}
            raw = path.read_text(encoding="utf-8")
            data = json.loads(raw)
            return data if isinstance(data, dict) else {}
        except Exception:
            logging.debug("Command sync state unreadable at %s", path, exc_info=True)
            return {}

    def _write_command_sync_state(self, state: dict[str, Any]) -> None:
        path = self._command_sync_state_path()
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                json.dumps(state, ensure_ascii=True, sort_keys=True, indent=2), encoding="utf-8"
            )
        except Exception:
            logging.warning("Failed to persist command sync state at %s", path, exc_info=True)

    @staticmethod
    def _normalize_command_sync_scope(scope: str) -> str:
        value = scope.strip().lower()
        if value in {"all", "both"}:
            return "both"
        if value in {"global", "globals"}:
            return "global"
        if value in {"guild", "guilds"}:
            return "guild"
        return "both"

    def _command_sync_guild_ids(self) -> list[int]:
        ids = self._parse_id_list(os.getenv("COMMAND_SYNC_GUILD_IDS", ""))
        if ids:
            return ids

        main_guild = (os.getenv("MAIN_GUILD_ID") or "").strip()
        if main_guild.isdigit():
            return [int(main_guild)]

        guild_id = int(getattr(settings, "guild_id", 0) or 0)
        return [guild_id] if guild_id > 0 else []

    @staticmethod
    def _command_sync_timeout_seconds() -> float | None:
        """
        Timeout for individual Discord app-command sync calls.

        Prevents long startup stalls (and perceived offline bot) when Discord
        responds with long retry windows for sync endpoints.
        """
        raw = (os.getenv("COMMAND_SYNC_TIMEOUT_SECONDS") or "").strip()
        if not raw:
            return 20.0
        try:
            parsed = float(raw)
        except ValueError:
            logging.warning(
                "Invalid COMMAND_SYNC_TIMEOUT_SECONDS=%r, using default 20s",
                raw,
            )
            return 20.0
        if parsed <= 0:
            return None
        return parsed

    def _command_sync_hash(self, guild_ids: list[int]) -> str:
        global_payload = [cmd.to_dict(self.tree) for cmd in self.tree.get_commands()]
        global_payload.sort(
            key=lambda c: (
                str(c.get("name", "")),
                str(c.get("type", "")),
                str(c.get("description", "")),
            )
        )

        guild_payload: dict[str, list[dict[str, Any]]] = {}
        for guild_id in guild_ids:
            obj = discord.Object(id=guild_id)
            payload = [cmd.to_dict(self.tree) for cmd in self.tree.get_commands(guild=obj)]
            payload.sort(
                key=lambda c: (
                    str(c.get("name", "")),
                    str(c.get("type", "")),
                    str(c.get("description", "")),
                )
            )
            guild_payload[str(guild_id)] = payload

        doc = {"global": global_payload, "guilds": guild_payload}
        raw = json.dumps(doc, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
        return hashlib.sha256(raw.encode("utf-8")).hexdigest()

    async def sync_app_commands(
        self,
        *,
        reason: str,
        scope: str = "both",
        force: bool = False,
    ) -> dict[str, Any]:
        normalized_scope = self._normalize_command_sync_scope(scope)
        include_global = normalized_scope in {"both", "global"}
        include_guilds = normalized_scope in {"both", "guild"}
        sync_timeout = self._command_sync_timeout_seconds()
        copy_global_to_guild = self._env_bool("COMMAND_SYNC_COPY_GLOBAL_TO_GUILD", True)

        async with self._command_sync_lock:
            mode = self._command_sync_mode()
            guild_ids = self._command_sync_guild_ids()

            try:
                current_hash = self._command_sync_hash(guild_ids)
            except Exception:
                logging.warning(
                    "Failed to build app-command hash; falling back to forced sync.", exc_info=True
                )
                current_hash = ""
                force = True
            state = self._read_command_sync_state()
            previous_hash = str(state.get("hash") or "")
            previous_scope = self._normalize_command_sync_scope(str(state.get("scope") or "both"))

            if mode == "disabled" and not force:
                logging.info(
                    "App-command sync disabled (reason=%s, scope=%s)", reason, normalized_scope
                )
                return {
                    "status": "skipped",
                    "scope": normalized_scope,
                    "mode": mode,
                    "force": force,
                    "skip_reason": "disabled",
                    "hash": current_hash,
                    "previous_hash": previous_hash,
                    "global_count": 0,
                    "guild_counts": {},
                    "errors": {},
                }

            if (
                mode == "hybrid"
                and not force
                and previous_hash
                and previous_hash == current_hash
                and previous_scope == normalized_scope
            ):
                logging.info(
                    "App-command sync skipped (unchanged hash=%s, reason=%s, scope=%s)",
                    current_hash[:12],
                    reason,
                    normalized_scope,
                )
                return {
                    "status": "skipped",
                    "scope": normalized_scope,
                    "mode": mode,
                    "force": force,
                    "skip_reason": "unchanged",
                    "hash": current_hash,
                    "previous_hash": previous_hash,
                    "global_count": 0,
                    "guild_counts": {},
                    "errors": {},
                }

            started = time.perf_counter()
            guild_counts: dict[str, int] = {}
            errors: dict[str, str] = {}
            global_count = 0

            if include_guilds:
                if guild_ids:
                    for guild_id in guild_ids:
                        guild_obj = discord.Object(id=guild_id)
                        try:
                            if copy_global_to_guild:
                                try:
                                    self.tree.copy_global_to(guild=guild_obj)
                                except Exception:
                                    logging.warning(
                                        "copy_global_to failed for guild %s; syncing guild-local commands only",
                                        guild_id,
                                        exc_info=True,
                                    )
                            if sync_timeout is None:
                                synced = await self.tree.sync(guild=guild_obj)
                            else:
                                synced = await asyncio.wait_for(
                                    self.tree.sync(guild=guild_obj),
                                    timeout=sync_timeout,
                                )
                            guild_counts[str(guild_id)] = len(synced)
                        except TimeoutError:
                            timeout_msg = (
                                f"timeout after {sync_timeout:.1f}s"
                                if sync_timeout is not None
                                else "timeout"
                            )
                            errors[f"guild:{guild_id}"] = timeout_msg
                            logging.warning(
                                "Guild app-command sync timed out for %s (reason=%s, timeout=%s)",
                                guild_id,
                                reason,
                                timeout_msg,
                            )
                        except Exception as exc:
                            errors[f"guild:{guild_id}"] = str(exc)
                            logging.warning(
                                "Guild app-command sync failed for %s (reason=%s): %s",
                                guild_id,
                                reason,
                                exc,
                            )
                else:
                    logging.info("No guild IDs configured for app-command guild sync.")

            if include_global:
                try:
                    if sync_timeout is None:
                        synced_global = await self.tree.sync()
                    else:
                        synced_global = await asyncio.wait_for(
                            self.tree.sync(),
                            timeout=sync_timeout,
                        )
                    global_count = len(synced_global)
                except TimeoutError:
                    timeout_msg = (
                        f"timeout after {sync_timeout:.1f}s"
                        if sync_timeout is not None
                        else "timeout"
                    )
                    errors["global"] = timeout_msg
                    logging.error(
                        "Global app-command sync timed out (reason=%s, timeout=%s)",
                        reason,
                        timeout_msg,
                    )
                except Exception as exc:
                    errors["global"] = str(exc)
                    logging.error("Global app-command sync failed (reason=%s): %s", reason, exc)

            success_count = (1 if include_global and "global" not in errors else 0) + len(
                guild_counts
            )
            if errors and success_count > 0:
                status = "partial"
            elif errors:
                status = "error"
            else:
                status = "synced"

            elapsed = time.perf_counter() - started
            if status == "synced":
                self._write_command_sync_state(
                    {
                        "hash": current_hash,
                        "updated_at": _dt.datetime.now(tz=_dt.UTC).isoformat(),
                        "reason": reason,
                        "scope": normalized_scope,
                        "mode": mode,
                    }
                )

            logging.info(
                "App-command sync %s (reason=%s, scope=%s, global=%d, guilds=%d, errors=%d, took=%.2fs)",
                status,
                reason,
                normalized_scope,
                global_count,
                len(guild_counts),
                len(errors),
                elapsed,
            )
            return {
                "status": status,
                "scope": normalized_scope,
                "mode": mode,
                "force": force,
                "hash": current_hash,
                "previous_hash": previous_hash,
                "global_count": global_count,
                "guild_counts": guild_counts,
                "errors": errors,
                "sync_timeout_seconds": sync_timeout,
                "elapsed_seconds": elapsed,
            }

    async def request_restart(self, reason: str = "unknown") -> bool:
        """
        Delegate a full-process restart to the lifecycle supervisor if available.
        """
        if not self.lifecycle:
            logging.warning("Restart requested (%s) aber kein Lifecycle vorhanden", reason)
            return False
        return await self.lifecycle.request_restart(reason=reason)

    async def setup_hook(self):
        logging.info(
            "Master Bot setup starting (role=%s, gateway=%s)...",
            self.runtime_mode.role,
            self.runtime_mode.discord_gateway_enabled,
        )

        secret_mode = (os.getenv("SECRET_LOG_MODE") or "off").lower()
        _log_secret_present(
            "Steam API Key", ["STEAM_API_KEY", "STEAM_WEB_API_KEY"], mode=secret_mode
        )
        _log_secret_present("Discord Token (Master)", ["DISCORD_TOKEN", "BOT_TOKEN"], mode="off")
        _log_secret_present(
            "Streamer OAuth Credentials",
            ["TWITCH_CLIENT_ID", "TWITCH_CLIENT_SECRET"],
            mode=secret_mode,
        )

        db_span = measure("db.init")
        _init_db_if_available()
        db_span.finish()

        await self._ensure_master_broker_view_resolver()

        if self.master_broker:
            broker_span = measure("broker.start")
            try:
                await self.master_broker.start()
            except Exception as exc:
                logging.warning(
                    "Master broker startup failed; continuing without broker: %s",
                    exc,
                    exc_info=True,
                )
                self.master_broker = None
                broker_span.finish(detail="disabled:start-failed")
            else:
                broker_span.finish(detail=self.master_broker.base_url)

        load_span = measure("cogs.load", detail=f"planned={len(self.cogs_list)}")
        await self.load_all_cogs()
        loaded_now = len([ext for ext in self.extensions.keys() if ext.startswith("cogs.")])
        load_span.finish(detail=f"loaded={loaded_now}")
        logging.info("Cogs geladen in %.2fs", time.perf_counter() - self._boot_started_at)

        if self._env_bool("COMMAND_SYNC_ON_START", True):
            sync_span = measure("slash.sync")
            scope = self._normalize_command_sync_scope(
                os.getenv("COMMAND_SYNC_START_SCOPE", "guild")
            )
            result = await self.sync_app_commands(reason="setup_hook", scope=scope, force=False)
            sync_span.finish(
                detail=(
                    f"status={result.get('status')} "
                    f"global={result.get('global_count', 0)} "
                    f"guilds={len(result.get('guild_counts', {}))}"
                )
            )
        else:
            logging.info("Startup app-command sync disabled via COMMAND_SYNC_ON_START.")

        logging.info("Master Bot setup completed")
        log_event("bot.setup_hook", time.perf_counter() - self._boot_started_at, "completed")

    async def close(self):
        logging.info("Master Bot shutting down...")

        if self.master_broker:
            try:
                await self.master_broker.stop()
            except Exception as exc:
                logging.error(f"Fehler beim Stoppen des Master-Brokers: {exc}")

        if self.dashboard:
            try:
                await self.dashboard.stop()
            except Exception as e:
                logging.error(f"Fehler beim Stoppen des Dashboards: {e}")

        if self.standalone_manager:
            try:
                await self.standalone_manager.shutdown()
            except Exception as exc:
                logging.error(f"Fehler beim Stoppen des Standalone-Managers: {exc}")

        to_unload = [ext for ext in list(self.extensions.keys()) if ext.startswith("cogs.")]
        if to_unload:
            logging.info(
                f"Unloading {len(to_unload)} cogs with timeout {self.per_cog_unload_timeout:.1f}s each ..."
            )
            _ = await self.unload_many(to_unload, timeout=self.per_cog_unload_timeout)

        try:
            timeout = float(os.getenv("DISCORD_CLOSE_TIMEOUT", "5"))
        except ValueError:
            timeout = 5.0
        try:
            await asyncio.wait_for(super().close(), timeout=timeout)
            logging.info("discord.Client.close() returned")
        except TimeoutError:
            logging.error(
                f"discord.Client.close() timed out after {timeout:.1f}s; continuing shutdown"
            )
        except Exception as e:
            logging.error(f"Error in discord.Client.close(): {e}")

        logging.info("Master Bot shutdown complete")
