from __future__ import annotations

import asyncio
import json
import logging
import os
import time
from typing import Any

import aiohttp
import discord

from bot_core.boot_profile import log_event

_STEAM_LOG_CHANNEL_ID = 1374364800817303632


class PresenceMixin:
    """Presence, Ready-Tasks und Voice-Routing."""

    _steam_bridge_internal_self_heal_enabled: bool = (
        os.getenv("STEAM_BRIDGE_INTERNAL_SELF_HEAL") or ""
    ).strip().lower() in {"1", "true", "yes", "y", "on"}

    def active_cogs(self) -> list[str]:
        """Aktuell geladene Extensions (runtime), nur 'cogs.'-Namespace."""
        return sorted([ext for ext in self.extensions.keys() if ext.startswith("cogs.")])

    async def update_presence(self):
        """Presence immer anhand der echten Runtime-Anzahl setzen."""
        pfx = os.getenv("COMMAND_PREFIX", "!")
        try:
            if not self.is_ready() or getattr(self, "ws", None) is None:
                logging.debug("Presence-Update übersprungen – Bot noch nicht bereit")
                return
            activity = discord.Activity(
                type=discord.ActivityType.watching,
                name=f"{len(self.active_cogs())} Cogs | {pfx}help",
            )
            await self.change_presence(activity=activity)
        except Exception as exc:
            logging.exception("Konnte Presence nicht aktualisieren: %s", exc)

    async def _start_dashboard_background(self) -> None:
        if not self.dashboard:
            return
        try:
            logging.info("Dashboard HTTP server startup task running...")
            await self.dashboard.start()
            logging.info("Dashboard HTTP server startup completed.")
        except RuntimeError as e:
            logging.error(
                f"Dashboard konnte nicht gestartet werden: {e}. Laeuft bereits ein anderer Prozess?"
            )
        except Exception as e:
            logging.error(f"Dashboard konnte nicht gestartet werden: {e}")

    async def on_voice_state_update(
        self,
        member: discord.Member,
        before: discord.VoiceState,
        after: discord.VoiceState,
    ):
        """
        Voice Event Router - verteilt Voice State Updates parallel an alle Handler-Cogs.
        Verhindert sequenzielle Abarbeitung (40% schneller!).
        """
        # Sammle alle Voice-Handler aus den Cogs (mit Metadaten für Error-Logging)
        handler_info = []
        for cog_name, cog in self.cogs.items():
            if hasattr(cog, "on_voice_state_update"):
                listener_pairs = getattr(type(cog), "__cog_listeners__", [])
                if any(
                    event_name == "on_voice_state_update" and method_name == "on_voice_state_update"
                    for event_name, method_name in listener_pairs
                ):
                    continue
                handler = cog.on_voice_state_update
                if callable(handler):
                    handler_info.append((cog_name, handler))

        if not handler_info:
            return

        # Führe alle Handler PARALLEL aus (nicht sequenziell wie discord.py Default!)
        tasks = [(cog_name, handler(member, before, after)) for cog_name, handler in handler_info]
        results = await asyncio.gather(*[task for _, task in tasks], return_exceptions=True)

        # Log Fehler mit korrektem Cog-Namen (Race-Safe!)
        for (cog_name, _), result in zip(tasks, results, strict=False):
            if isinstance(result, Exception):
                logging.error(f"Voice handler error in {cog_name}: {result}", exc_info=result)

    async def on_ready(self):
        logging.info(f"Bot logged in as {self.user} (ID: {self.user.id})")
        logging.info(f"Connected to {len(self.guilds)} guilds")

        try:
            started = getattr(self, "_boot_started_at", None)
            if started is not None:
                log_event(
                    "discord.ready",
                    time.perf_counter() - started,
                    f"guilds={len(self.guilds)}",
                )
        except Exception:
            logging.getLogger(__name__).debug("BootProfile ready log failed", exc_info=True)

        await self.update_presence()

        runtime_loaded = self.active_cogs()
        logging.info(f"Loaded cogs (runtime): {len(runtime_loaded)}")
        logging.info(f"Loaded cogs: {len(runtime_loaded)}/{len(self.cogs_list)}")

        # TempVoice Log (neu)
        try:
            tv_core = self.get_cog("TempVoiceCore")
            if tv_core:
                cnt = len(getattr(tv_core, "created_channels", set()))
                logging.info(f"TempVoiceCore bereit • verwaltete Lanes: {cnt}")
            tv_if = self.get_cog("TempVoiceInterface")
            if tv_if:
                logging.info("TempVoiceInterface bereit • Interface-View registriert")
        except Exception as e:
            logging.getLogger().debug("TempVoice Ready-Log fehlgeschlagen (ignoriert): %r", e)

        # Performance-Info loggen
        voice_handlers = sum(
            1 for cog in self.cogs.values() if hasattr(cog, "on_voice_state_update")
        )
        if voice_handlers > 0:
            logging.info(f"Voice Event Router aktiv: {voice_handlers} Handler (parallel)")

        asyncio.create_task(self.hourly_health_check())
        if self.standalone_manager:
            asyncio.create_task(self._bootstrap_standalone_autostart())

    async def queue_channel_rename(
        self, channel_id: int, new_name: str, reason: str = "Automated Rename"
    ):
        rename_cog = self.get_cog("RenameManagerCog")
        if rename_cog:
            rename_cog.queue_local_rename_request(channel_id, new_name, reason)
        else:
            logging.error(
                "RenameManagerCog nicht geladen. Rename fuer Channel %s zu '%s' kann nicht verarbeitet werden.",
                channel_id,
                new_name,
            )

    # State für Steam-Bridge-Health-Self-Heal
    _steam_bridge_unhealthy_since: float | None = None
    _steam_bridge_unhealthy_reason: str | None = None
    _steam_bridge_restart_cooldown_until: float = 0.0

    @staticmethod
    def _extract_steam_bridge_health_issue(snapshot: dict[str, Any]) -> dict[str, Any] | None:
        runtime = snapshot.get("runtime", {}) if isinstance(snapshot, dict) else {}
        diagnostics = snapshot.get("diagnostics", {}) if isinstance(snapshot, dict) else {}
        now = time.time()

        logged_on = bool(runtime.get("logged_on", False))
        logging_in = bool(runtime.get("logging_in", False))
        steam_id64 = str(runtime.get("steam_id64") or "").strip()
        last_error = runtime.get("last_error")
        if isinstance(last_error, dict):
            last_error_message = str(last_error.get("message") or "").strip()
        elif last_error:
            last_error_message = str(last_error).strip()
        else:
            last_error_message = ""

        recent_failed_friend_requests = int(
            diagnostics.get("recent_failed_friend_requests", 0) or 0
        )
        oldest_pending_friend_request_age = diagnostics.get(
            "oldest_pending_friend_request_age_seconds"
        )
        if oldest_pending_friend_request_age is not None:
            try:
                oldest_pending_friend_request_age = int(oldest_pending_friend_request_age)
            except (TypeError, ValueError):
                oldest_pending_friend_request_age = None

        if not logging_in and not logged_on:
            return {
                "reason": "not_logged_in",
                "summary": "Bridge läuft, ist aber nicht bei Steam eingeloggt.",
                "details": {
                    "logged_on": logged_on,
                    "logging_in": logging_in,
                    "steam_id64": steam_id64 or None,
                    "last_error": last_error_message or None,
                    "recent_failed_friend_requests": recent_failed_friend_requests,
                    "oldest_pending_friend_request_age_seconds": oldest_pending_friend_request_age,
                    "detected_at": now,
                },
            }

        if logged_on and not steam_id64:
            return {
                "reason": "missing_steam_id",
                "summary": "Bridge meldet Login, aber keine Steam-ID.",
                "details": {
                    "logged_on": logged_on,
                    "logging_in": logging_in,
                    "last_error": last_error_message or None,
                    "recent_failed_friend_requests": recent_failed_friend_requests,
                    "oldest_pending_friend_request_age_seconds": oldest_pending_friend_request_age,
                    "detected_at": now,
                },
            }

        stalled_friend_requests = (
            recent_failed_friend_requests >= 2
            and (oldest_pending_friend_request_age or 0) >= 120
            and last_error_message.lower() in {"noconnection", "not logged in", "request timed out"}
        )
        if stalled_friend_requests:
            return {
                "reason": "friend_requests_stalled",
                "summary": "Steam-Friend-Requests laufen in Timeouts und hängen fest.",
                "details": {
                    "logged_on": logged_on,
                    "logging_in": logging_in,
                    "steam_id64": steam_id64 or None,
                    "last_error": last_error_message or None,
                    "recent_failed_friend_requests": recent_failed_friend_requests,
                    "oldest_pending_friend_request_age_seconds": oldest_pending_friend_request_age,
                    "detected_at": now,
                },
            }

        return None

    async def _check_steam_bot_health(self) -> str | None:
        """Prüft den Rust-steam-bot (Discord-Flows + Web-API, Port 8783) per /health.

        Gibt einen Issue-Text zurück oder None wenn alles in Ordnung ist.
        """
        base = os.getenv("STEAM_BOT_API_URL", "http://127.0.0.1:8783").rstrip("/")
        try:
            async with aiohttp.ClientSession() as session:
                async with session.get(
                    f"{base}/health",
                    timeout=aiohttp.ClientTimeout(total=5),
                ) as resp:
                    if resp.status != 200:
                        return f"steam-bot /health HTTP {resp.status}"
        except Exception as exc:
            return f"steam-bot nicht erreichbar: {exc}"
        return None

    async def _check_steam_bridge_login_health(self) -> None:
        """Sauberer Self-Heal für die Steam Bridge anhand von Runtime-State und Task-Diagnostik."""
        if not self._steam_bridge_internal_self_heal_enabled:
            return
        if not self.standalone_manager:
            return
        try:
            state_info = await self.standalone_manager.status("steam")
        except Exception:
            return

        if not state_info.get("running"):
            self._steam_bridge_unhealthy_since = None
            self._steam_bridge_unhealthy_reason = None
            return  # Prozess läuft nicht – ensure_autostart kümmert sich darum

        # Runtime- und Diagnostik-Status aus DB lesen
        try:
            from service import db

            def _get_state():
                row = db.query_one(
                    "SELECT heartbeat, payload FROM standalone_bot_state WHERE bot=?",
                    ("steam",),
                )
                if not row:
                    return 0, {}
                payload = {}
                if row["payload"]:
                    try:
                        payload = json.loads(row["payload"])
                    except Exception:  # noqa: S110
                        pass
                now_ts = int(time.time())
                recent_failed_friend_requests_row = db.query_one(
                    """
                    SELECT COUNT(*) AS count
                      FROM steam_tasks
                     WHERE type='AUTH_SEND_FRIEND_REQUEST'
                       AND status='FAILED'
                       AND updated_at >= ?
                    """,
                    (now_ts - 900,),
                )
                oldest_pending_friend_request_row = db.query_one(
                    """
                    SELECT MIN(requested_at) AS oldest_requested_at
                      FROM steam_friend_requests
                     WHERE status='pending'
                    """,
                )
                diagnostics = {
                    "recent_failed_friend_requests": int(
                        recent_failed_friend_requests_row["count"] or 0
                    )
                    if recent_failed_friend_requests_row
                    else 0,
                    "oldest_pending_friend_request_age_seconds": None,
                }
                if (
                    oldest_pending_friend_request_row
                    and oldest_pending_friend_request_row["oldest_requested_at"] is not None
                ):
                    diagnostics["oldest_pending_friend_request_age_seconds"] = max(
                        0,
                        now_ts - int(oldest_pending_friend_request_row["oldest_requested_at"]),
                    )
                payload["diagnostics"] = diagnostics
                return int(row["heartbeat"] or 0), payload

            heartbeat, payload = await asyncio.to_thread(_get_state)
        except Exception as exc:
            logging.debug("Steam login health check: DB-Fehler %s", exc)
            return

        # Heartbeat muss frisch sein (sonst Bridge schreibt gerade noch nicht)
        if not heartbeat or time.time() - heartbeat > 90:
            self._steam_bridge_unhealthy_since = None
            self._steam_bridge_unhealthy_reason = None
            return

        issue = self._extract_steam_bridge_health_issue(payload)
        if issue is None:
            if self._steam_bridge_unhealthy_since is not None:
                logging.info("Steam Bridge Health-Check wieder OK")
            self._steam_bridge_unhealthy_since = None
            self._steam_bridge_unhealthy_reason = None
            return

        now = time.time()
        reason = str(issue.get("reason") or "unknown")
        if self._steam_bridge_unhealthy_reason != reason:
            self._steam_bridge_unhealthy_since = now
            self._steam_bridge_unhealthy_reason = reason
            logging.warning(
                "Steam Bridge Health-Check: Problem erkannt (%s): %s",
                reason,
                issue.get("summary") or "ohne Beschreibung",
            )
            return

        if self._steam_bridge_unhealthy_since is None:
            self._steam_bridge_unhealthy_since = now
            return

        unhealthy_for_seconds = now - self._steam_bridge_unhealthy_since
        grace_period = 180
        if unhealthy_for_seconds < grace_period:
            return

        if now < self._steam_bridge_restart_cooldown_until:
            return

        self._steam_bridge_restart_cooldown_until = now + 600
        self._steam_bridge_unhealthy_since = None
        self._steam_bridge_unhealthy_reason = None

        details = issue.get("details", {}) if isinstance(issue.get("details"), dict) else {}
        minutes_down = int(unhealthy_for_seconds / 60)
        last_error = details.get("last_error")
        error_info = f" (letzter Fehler: `{last_error}`)" if last_error else ""
        logging.warning(
            "Steam Bridge health check: %s seit %d Min%s – starte sauberen Neustart",
            reason,
            minutes_down,
            error_info or "",
        )

        channel = self.get_channel(_STEAM_LOG_CHANNEL_ID)
        owner_id = getattr(self, "owner_id", None)
        ping = f"<@{owner_id}>" if owner_id else ""

        if channel:
            await channel.send(
                f"{ping} ⚠️ **Steam Bridge Self-Heal**: `{reason}` seit {minutes_down} Min.{error_info} – starte sauberen Neustart..."
            )

        try:
            await self.standalone_manager.restart("steam")
            logging.info("Steam Bridge Neustart durch Self-Heal ausgelöst: %s", reason)
            if channel:
                await channel.send(
                    "🔄 Steam Bridge Neustart wurde gestartet. Login und Session werden komplett neu aufgebaut."
                )
        except Exception as exc:
            logging.error("Steam Bridge Neustart fehlgeschlagen: %s", exc)
            if channel:
                await channel.send(f"❌ Neustart fehlgeschlagen: `{exc}`")

    async def hourly_health_check(self):
        critical_check_interval = 3600  # 1h
        last_critical_check = 0.0

        while not self.is_closed():
            try:
                await asyncio.sleep(300)
                current = asyncio.get_running_loop().time()

                if self.standalone_manager:
                    try:
                        await self.standalone_manager.ensure_autostart()
                    except Exception as exc:
                        logging.warning(
                            "Standalone Manager Autostart-Pruefung fehlgeschlagen: %s",
                            exc,
                        )

                try:
                    await self._check_steam_bridge_login_health()
                except Exception as exc:
                    logging.warning("Steam Bridge Login-Health-Check fehlgeschlagen: %s", exc)

                if current - last_critical_check >= critical_check_interval:
                    issues = []

                    if not self.get_cog("TempVoiceCore"):
                        issues.append("TempVoiceCore not loaded")
                    if not self.get_cog("TempVoiceInterface"):
                        issues.append("TempVoiceInterface not loaded")
                    # Steam-Linking lebt seit dem Cutover im Rust-steam-bot
                    # (Port 8783), nicht mehr im geblockten Python-Cog.
                    steam_bot_issue = await self._check_steam_bot_health()
                    if steam_bot_issue:
                        issues.append(steam_bot_issue)

                    if issues:
                        logging.warning(f"Critical Health Check: Issues found: {issues}")
                    else:
                        logging.debug("Critical Health Check: Core cogs operational")

                    last_critical_check = current

            except Exception as e:
                logging.error(f"Health check error: {e}")
