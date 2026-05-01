from __future__ import annotations

import asyncio
import logging
import os
import sys
import time
from collections import deque
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from datetime import UTC, date, datetime
from pathlib import Path
from typing import Any

__all__ = [
    "StandaloneBotConfig",
    "StandaloneBotManager",
    "StandaloneManagerError",
    "StandaloneConfigNotFound",
    "StandaloneAlreadyRunning",
    "StandaloneNotRunning",
]

log = logging.getLogger(__name__)


def _ts_from_monotonic(monotonic_value: float | None, fallback: float | None) -> float | None:
    if monotonic_value is None:
        return fallback
    offset = time.time() - time.monotonic()
    return monotonic_value + offset


def _iso(ts: float | None) -> str | None:
    if ts is None:
        return None
    return datetime.fromtimestamp(ts, tz=UTC).isoformat()


@dataclass(slots=True)
class StandaloneBotConfig:
    """
    Configuration describing how to launch and supervise a standalone helper bot.
    """

    key: str
    name: str
    script: Path
    workdir: Path
    description: str = ""
    args: list[str] = field(default_factory=list)
    env: dict[str, str] = field(default_factory=dict)
    executable: str | None = None
    python: str | None = None
    autostart: bool = False
    restart_on_crash: bool = True
    daily_restart_at: str | None = None  # format: "HH:MM" (local time)
    max_uptime_seconds: float | None = None
    max_log_lines: int = 200
    metrics_provider: Callable[[], Awaitable[dict[str, Any]]] | None = None
    tags: list[str] = field(default_factory=list)
    command_namespace: str | None = None

    def resolved_command(self) -> list[str]:
        interpreter = self.executable or self.python or sys.executable
        return [interpreter, str(self.script), *self.args]


@dataclass(slots=True)
class _RuntimeState:
    config: StandaloneBotConfig
    process: asyncio.subprocess.Process | None = None
    started_at_monotonic: float | None = None
    started_wall: float | None = None
    last_exit_wall: float | None = None
    returncode: int | None = None
    restart_attempts: int = 0
    stop_requested: bool = False
    reader_tasks: list[asyncio.Task] = field(default_factory=list)
    log_buffer: deque[dict[str, Any]] = field(init=False)
    last_scheduled_restart_day: date | None = None

    def __post_init__(self) -> None:
        self.log_buffer = deque(maxlen=self.config.max_log_lines)

    @property
    def running(self) -> bool:
        return self.process is not None and self.process.returncode is None


class StandaloneManagerError(RuntimeError):
    pass


class StandaloneConfigNotFound(StandaloneManagerError):
    pass


class StandaloneAlreadyRunning(StandaloneManagerError):
    pass


class StandaloneNotRunning(StandaloneManagerError):
    pass


class StandaloneBotManager:
    """
    Supervises standalone bot processes (e.g. helper Discord bots with their own token).
    """

    def __init__(self) -> None:
        self._configs: dict[str, StandaloneBotConfig] = {}
        self._states: dict[str, _RuntimeState] = {}
        self._lock = asyncio.Lock()
        self._shutting_down = False
        self._manager_started_at: float = time.time()

    def register(self, config: StandaloneBotConfig) -> None:
        if config.key in self._configs:
            raise ValueError(f"Standalone bot '{config.key}' already registered")
        if not config.script.exists():
            raise FileNotFoundError(f"Standalone script does not exist: {config.script}")
        self._configs[config.key] = config
        self._states[config.key] = _RuntimeState(config=config)
        log.debug("Registered standalone bot %s -> %s", config.key, config.script)

    def config(self, key: str) -> StandaloneBotConfig:
        try:
            return self._configs[key]
        except KeyError as exc:
            raise StandaloneConfigNotFound(key) from exc

    def all_configs(self) -> list[StandaloneBotConfig]:
        return list(self._configs.values())

    async def start(self, key: str) -> dict[str, Any]:
        async with self._lock:
            state = self._states.get(key)
            if state is None:
                raise StandaloneConfigNotFound(key)
            if state.running:
                raise StandaloneAlreadyRunning(key)

            cmd = state.config.resolved_command()
            env = os.environ.copy()
            if state.config.env:
                env.update(state.config.env)

            log.info("Starting standalone bot %s: %s", key, " ".join(cmd))
            process = await asyncio.create_subprocess_exec(
                *cmd,
                cwd=str(state.config.workdir),
                env=env,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                stdin=asyncio.subprocess.PIPE,
            )

            state.process = process
            state.started_at_monotonic = time.monotonic()
            state.started_wall = time.time()
            state.returncode = None
            state.stop_requested = False
            state.restart_attempts = 0
            state.log_buffer.clear()

            stdout_task = asyncio.create_task(self._pump_stream(key, process.stdout, "stdout"))
            stderr_task = asyncio.create_task(self._pump_stream(key, process.stderr, "stderr"))
            waiter_task = asyncio.create_task(self._wait_for_exit(key, process))
            state.reader_tasks = [stdout_task, stderr_task, waiter_task]

            return self._status_for_state(state)

    async def stop(self, key: str, *, kill_after: float = 10.0) -> dict[str, Any]:
        async with self._lock:
            state = self._states.get(key)
            if state is None:
                raise StandaloneConfigNotFound(key)
            if not state.running:
                raise StandaloneNotRunning(key)

            process = state.process
            state.stop_requested = True
            assert process is not None

            log.info("Stopping standalone bot %s (pid=%s)", key, process.pid)
            process.terminate()
        try:
            await asyncio.wait_for(process.wait(), timeout=kill_after)
        except TimeoutError:
            log.warning(
                "Standalone bot %s did not terminate within %.1fs -> kill()",
                key,
                kill_after,
            )
            process.kill()
            await process.wait()
        finally:
            async with self._lock:
                await self._cleanup_tasks(key)
                state = self._states[key]
                state.process = None
                state.returncode = process.returncode
                state.last_exit_wall = time.time()

        return await self.status(key)

    async def restart(self, key: str) -> dict[str, Any]:
        try:
            await self.stop(key)
        except StandaloneNotRunning:
            log.debug("Restart requested but %s was not running; continuing", key)
        return await self.start(key)

    async def ensure_running(self, key: str) -> dict[str, Any]:
        async with self._lock:
            state = self._states.get(key)
            if state is None:
                raise StandaloneConfigNotFound(key)
            if state.running:
                return self._status_for_state(state)
        return await self.start(key)

    async def ensure_autostart(self) -> None:
        for config in self.all_configs():
            await self._maybe_restart_on_daily_schedule(config)
            await self._maybe_restart_on_max_uptime(config)
            if not config.autostart:
                continue
            try:
                await self.ensure_running(config.key)
            except StandaloneManagerError as exc:
                log.error("Failed to autostart %s: %s", config.key, exc)

    async def restart_all(
        self,
        *,
        autostart_only: bool = False,
        restart_running: bool = True,
    ) -> list[dict[str, Any]]:
        """
        Restart all registered standalone bots.

        Args:
            autostart_only: limit to configs with autostart=True.
            restart_running: if False, only start bots that are not running.
        """
        results: list[dict[str, Any]] = []
        for config in self.all_configs():
            if autostart_only and not config.autostart:
                continue
            try:
                if restart_running:
                    status = await self.restart(config.key)
                else:
                    status = await self.ensure_running(config.key)
            except StandaloneNotRunning:
                status = await self.start(config.key)
            except StandaloneManagerError as exc:
                status = {"key": config.key, "error": str(exc)}
                log.error("Restart for %s failed: %s", config.key, exc)
            results.append(status)
        return results

    async def _maybe_restart_on_daily_schedule(self, config: StandaloneBotConfig) -> None:
        schedule = config.daily_restart_at
        if not schedule:
            return
        try:
            hour_str, minute_str = schedule.split(":")
            hour = int(hour_str)
            minute = int(minute_str)
            if not (0 <= hour <= 23 and 0 <= minute <= 59):
                raise ValueError
        except ValueError:
            log.warning("Invalid daily_restart_at for %s: %s", config.key, schedule)
            return

        now = datetime.now()
        today = now.date()
        target_today = now.replace(hour=hour, minute=minute, second=0, microsecond=0)

        async with self._lock:
            state = self._states.get(config.key)
            if not state:
                return
            if state.last_scheduled_restart_day == today:
                return
            should_restart = now >= target_today
            # On fresh manager start: if we're already past today's scheduled time,
            # mark it as done without restarting (avoids premature restart on bot reboot).
            startup_grace = 30 * 60  # 30 minutes
            if (
                should_restart
                and state.last_scheduled_restart_day is None
                and (time.time() - self._manager_started_at) < startup_grace
            ):
                log.info(
                    "Scheduled daily restart for %s at %s -> skipped (manager just started, marking today as done)",
                    config.key,
                    schedule,
                )
                state.last_scheduled_restart_day = today
                return

        if not should_restart:
            return

        log.info("Scheduled daily restart for %s at %s -> restart now", config.key, schedule)
        try:
            await self.restart(config.key)
        except StandaloneManagerError as exc:
            log.warning("Daily restart for %s failed: %s", config.key, exc)
            return

        async with self._lock:
            state = self._states.get(config.key)
            if state:
                state.last_scheduled_restart_day = today

    async def _maybe_restart_on_max_uptime(self, config: StandaloneBotConfig) -> None:
        max_uptime = config.max_uptime_seconds
        if not max_uptime:
            return

        restart_needed = False
        uptime = None

        async with self._lock:
            state = self._states.get(config.key)
            if state and state.running:
                started_wall = state.started_wall or _ts_from_monotonic(
                    state.started_at_monotonic, None
                )
                if started_wall is not None:
                    uptime = time.time() - started_wall
                    restart_needed = uptime >= max_uptime

        if not restart_needed:
            return

        log.info(
            "Standalone bot %s exceeded max uptime %.0fs (uptime %.0fs) -> restart",
            config.key,
            max_uptime,
            uptime or 0.0,
        )
        try:
            await self.restart(config.key)
        except StandaloneManagerError as exc:
            log.warning("Max-uptime restart for %s failed: %s", config.key, exc)

    async def set_autostart(self, key: str, enabled: bool) -> dict[str, Any]:
        async with self._lock:
            state = self._states.get(key)
            if state is None:
                raise StandaloneConfigNotFound(key)
            state.config.autostart = bool(enabled)
            log.info("Standalone bot %s autostart -> %s", key, state.config.autostart)
            status = self._status_for_state(state)
        return status

    async def shutdown(self, *, kill_after: float = 10.0) -> None:
        self._shutting_down = True
        for config in self.all_configs():
            try:
                await self.stop(config.key, kill_after=kill_after)
            except StandaloneNotRunning:
                continue
            except StandaloneManagerError as exc:
                log.error("Failed to stop %s during shutdown: %s", config.key, exc)

    async def status(self, key: str) -> dict[str, Any]:
        async with self._lock:
            state = self._states.get(key)
            if state is None:
                raise StandaloneConfigNotFound(key)
            status = self._status_for_state(state)
        metrics: dict[str, Any] | None = None
        provider = state.config.metrics_provider
        if provider:
            try:
                metrics = await provider()
            except Exception as exc:  # noqa: BLE001
                log.warning("Metrics provider for %s failed: %s", key, exc)
        if metrics is not None:
            status["metrics"] = metrics
        return status

    async def snapshot(self) -> list[dict[str, Any]]:
        results: list[dict[str, Any]] = []
        for config in self.all_configs():
            info = await self.status(config.key)
            info["config"] = {
                "name": config.name,
                "description": config.description,
                "tags": config.tags,
                "autostart": config.autostart,
                "command_namespace": config.command_namespace,
            }
            results.append(info)
        return results

    async def logs(self, key: str, limit: int = 100) -> list[dict[str, Any]]:
        async with self._lock:
            state = self._states.get(key)
            if state is None:
                raise StandaloneConfigNotFound(key)
            tail = list(state.log_buffer)[-limit:]
        return tail

    async def _cleanup_tasks(self, key: str) -> None:
        state = self._states.get(key)
        if not state:
            return
        for task in state.reader_tasks:
            if not task.done():
                task.cancel()
        for task in state.reader_tasks:
            try:
                await task
            except asyncio.CancelledError:
                log.debug("Reader task for %s was cancelled during cleanup", key)
            except Exception as exc:  # noqa: BLE001
                log.debug("Reader task for %s raised during cleanup: %s", key, exc)
        state.reader_tasks.clear()

    async def _pump_stream(self, key: str, stream: asyncio.StreamReader | None, label: str) -> None:
        if stream is None:
            return
        while True:
            try:
                line = await stream.readline()
            except asyncio.CancelledError:
                return
            if not line:
                return
            text = line.decode("utf-8", errors="replace").rstrip()
            timestamp = datetime.now(UTC).isoformat()
            async with self._lock:
                state = self._states.get(key)
                if not state:
                    continue
                state.log_buffer.append({"ts": timestamp, "stream": label, "line": text})
            # log.debug("[%s][%s] %s", key, label, text)  <-- SPAM VERHINDERN
            if "[GC-Party]" in text or "[RP-DEBUG]" in text:
                log.info("[standalone:%s] %s", key, text)

    async def _wait_for_exit(self, key: str, process: asyncio.subprocess.Process) -> None:
        returncode = await process.wait()
        wall = time.time()
        async with self._lock:
            state = self._states.get(key)
            if not state:
                return
            state.returncode = returncode
            state.last_exit_wall = wall
            state.process = None
            state.reader_tasks = []
            exited_entry = {
                "ts": datetime.fromtimestamp(wall, tz=UTC).isoformat(),
                "stream": "manager",
                "line": f"Process exited with code {returncode}",
            }
            state.log_buffer.append(exited_entry)
            should_restart = (
                not self._shutting_down
                and not state.stop_requested
                and state.config.restart_on_crash
                and returncode not in (0, None)
            )
            state.stop_requested = False

        if should_restart:
            await self._schedule_restart(key)

    async def _schedule_restart(self, key: str) -> None:
        async with self._lock:
            state = self._states.get(key)
            if not state:
                return
            state.restart_attempts += 1
            attempt = state.restart_attempts
        delay = min(60.0, 5.0 * attempt)
        log.warning(
            "Standalone bot %s crashed (attempt %s) -> restart in %.1fs",
            key,
            attempt,
            delay,
        )
        await asyncio.sleep(delay)
        try:
            await self.start(key)
        except StandaloneAlreadyRunning:
            log.debug("Skip auto-restart for %s because it is already running", key)
        except StandaloneManagerError as exc:
            log.error("Automatic restart for %s failed: %s", key, exc)

    def _status_for_state(self, state: _RuntimeState) -> dict[str, Any]:
        started_wall = state.started_wall
        if started_wall is None and state.started_at_monotonic is not None:
            started_wall = _ts_from_monotonic(state.started_at_monotonic, None)
        uptime_seconds = None
        if state.running and started_wall is not None:
            uptime_seconds = max(0.0, time.time() - started_wall)

        return {
            "key": state.config.key,
            "running": state.running,
            "pid": state.process.pid if state.running and state.process else None,
            "started_at": _iso(started_wall),
            "uptime_seconds": uptime_seconds,
            "last_exit_at": _iso(state.last_exit_wall),
            "returncode": state.returncode,
            "restart_attempts": state.restart_attempts,
            "autostart": state.config.autostart,
        }
