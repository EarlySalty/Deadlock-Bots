from __future__ import annotations

import asyncio
import errno
import importlib
import logging
import os
import signal
import sys
import threading
import time
from collections.abc import Callable
from pathlib import Path

from bot_core.bootstrap import _load_env_robust, bootstrap_runtime
from bot_core.runtime_mode import ensure_gateway_start_allowed, resolve_runtime_mode

# Frühe Initialisierung, damit .env/Logging bereitstehen bevor Settings geladen werden.
bootstrap_runtime()

from bot_core import BotLifecycle, MasterBot, MasterControlCog  # noqa: E402

__all__ = ["MasterBot", "MasterControlCog", "BotLifecycle"]

_PID_FILE_BASE = Path(__file__).parent / "master_bot.pid"
_PID_LOCK_PATH: Path | None = None
_PID_RECOVERY_SUFFIX = ".recover"
_PID_LOCK_RETRY_DELAY_SECONDS = 0.05
_PID_LOCK_RETRY_ATTEMPTS = 120
_RECOVERY_LOCK_INVALID_GRACE_SECONDS = 1.0


def _effective_runtime_role() -> str:
    mode = resolve_runtime_mode()
    return mode.role


def _pid_file_path() -> Path:
    role = _effective_runtime_role()
    if role == "master":
        return _PID_FILE_BASE
    return _PID_FILE_BASE.with_name(f"{_PID_FILE_BASE.stem}.{role}{_PID_FILE_BASE.suffix}")


def _pid_exists(pid: int) -> bool:
    if pid <= 0:
        return False

    if os.name == "nt":
        try:
            import ctypes

            PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
            SYNCHRONIZE = 0x00100000
            WAIT_OBJECT_0 = 0x00000000
            WAIT_TIMEOUT = 0x00000102
            WAIT_FAILED = 0xFFFFFFFF

            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            open_process = kernel32.OpenProcess
            open_process.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
            open_process.restype = ctypes.c_void_p

            wait_for_single_object = kernel32.WaitForSingleObject
            wait_for_single_object.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
            wait_for_single_object.restype = ctypes.c_uint32

            close_handle = kernel32.CloseHandle
            close_handle.argtypes = [ctypes.c_void_p]
            close_handle.restype = ctypes.c_int

            access = PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE
            handle = open_process(access, 0, pid)
            if not handle:
                err = ctypes.get_last_error()
                if err in {87, 1168}:  # invalid parameter / element not found
                    return False
                if err == 5:  # access denied => process exists
                    return True
                return True  # fail closed: avoid starting a duplicate instance

            try:
                wait_result = wait_for_single_object(handle, 0)
                if wait_result == WAIT_TIMEOUT:
                    return True
                if wait_result == WAIT_OBJECT_0:
                    return False
                if wait_result == WAIT_FAILED:
                    return True
                return True
            finally:
                close_handle(handle)
        except Exception:
            return True

    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return True
    return True


def _read_pid_from_file(lock_file: Path) -> int | None:
    try:
        raw = lock_file.read_text(encoding="ascii").strip()
    except OSError:
        return None
    try:
        pid = int(raw)
    except (TypeError, ValueError):
        return None
    return pid if pid > 0 else None


def _try_create_pid_file(lock_file: Path, pid: int) -> bool:
    flags = os.O_CREAT | os.O_EXCL | os.O_WRONLY
    try:
        fd = os.open(os.fspath(lock_file), flags, 0o644)
    except OSError as exc:
        if exc.errno == errno.EEXIST:
            return False
        raise

    try:
        os.write(fd, f"{pid}\n".encode("ascii"))
    finally:
        os.close(fd)
    return True


def _release_file_if_owned(lock_file: Path, owner_pid: int) -> bool:
    if _read_pid_from_file(lock_file) != owner_pid:
        return False
    try:
        lock_file.unlink()
    except FileNotFoundError:
        return False
    except OSError:
        return False
    return True


def _recovery_lock_path(pid_file: Path) -> Path:
    return pid_file.with_name(f"{pid_file.name}{_PID_RECOVERY_SUFFIX}")


def _recovery_lock_is_stale(recovery_file: Path, holder_pid: int | None, current_pid: int) -> bool:
    if holder_pid == current_pid:
        return True
    if holder_pid:
        return not _pid_exists(holder_pid)
    try:
        age_seconds = time.time() - recovery_file.stat().st_mtime
    except OSError:
        return False
    return age_seconds >= _RECOVERY_LOCK_INVALID_GRACE_SECONDS


def _try_claim_recovery_lock(recovery_file: Path, current_pid: int) -> bool:
    if _try_create_pid_file(recovery_file, current_pid):
        return True

    holder_pid = _read_pid_from_file(recovery_file)
    if not _recovery_lock_is_stale(recovery_file, holder_pid, current_pid):
        return False

    try:
        recovery_file.unlink()
    except FileNotFoundError:
        return False
    except OSError:
        return False

    return _try_create_pid_file(recovery_file, current_pid)


def _acquire_pid_lock() -> None:
    """Verhindert dass zwei Instanzen gleichzeitig laufen.

    Erzeugt die PID-Datei atomar, um TOCTOU-Races beim Start zu vermeiden.
    Bei stale PID-Dateien wird eine koordinierte Recovery durchgeführt.
    """
    global _PID_LOCK_PATH
    pid_file = _pid_file_path()
    recovery_file = _recovery_lock_path(pid_file)
    current_pid = os.getpid()

    for _ in range(_PID_LOCK_RETRY_ATTEMPTS):
        if _try_create_pid_file(pid_file, current_pid):
            _PID_LOCK_PATH = pid_file
            return

        old_pid = _read_pid_from_file(pid_file)
        if old_pid == current_pid:
            _PID_LOCK_PATH = pid_file
            return

        if old_pid and _pid_exists(old_pid):
            logging.critical(
                "Master Bot läuft bereits als PID %s. "
                "Zweite Instanz wird NICHT gestartet (verhindert Token-Race-Conditions). "
                "Beende PID %s zuerst oder lösche %s manuell.",
                old_pid,
                old_pid,
                pid_file,
            )
            sys.exit(1)

        if not _try_claim_recovery_lock(recovery_file, current_pid):
            time.sleep(_PID_LOCK_RETRY_DELAY_SECONDS)
            continue

        try:
            owner_pid = _read_pid_from_file(pid_file)
            if owner_pid and owner_pid != current_pid and _pid_exists(owner_pid):
                logging.critical(
                    "Master Bot läuft bereits als PID %s. "
                    "Zweite Instanz wird NICHT gestartet (verhindert Token-Race-Conditions). "
                    "Beende PID %s zuerst oder lösche %s manuell.",
                    owner_pid,
                    owner_pid,
                    pid_file,
                )
                sys.exit(1)

            if owner_pid:
                logging.warning(
                    "Stale PID-File gefunden (PID %s nicht mehr aktiv) -> wird überschrieben",
                    owner_pid,
                )
            else:
                logging.warning("Ungültiges PID-File gefunden (%s) -> wird überschrieben", pid_file)

            try:
                pid_file.unlink()
            except FileNotFoundError:
                pass  # PID file is already gone; cleanup can continue.
            except OSError as exc:
                logging.getLogger(__name__).debug("PID lock cleanup failed: %r", exc)
        finally:
            _release_file_if_owned(recovery_file, current_pid)

        time.sleep(_PID_LOCK_RETRY_DELAY_SECONDS)

    logging.critical("PID lock acquisition timed out for %s", pid_file)
    raise SystemExit(1)


def _release_pid_lock() -> None:
    global _PID_LOCK_PATH
    pid_file = _PID_LOCK_PATH
    if pid_file is None:
        return
    try:
        _release_file_if_owned(pid_file, os.getpid())
    except Exception as exc:
        logging.getLogger(__name__).debug("PID lock release failed: %r", exc)
    finally:
        _PID_LOCK_PATH = None


def _load_fresh_token() -> str:
    """
    Reload settings (and .env) so restarts pick up a rotated Discord token.
    """
    _load_env_robust()
    config_module = importlib.reload(importlib.import_module("service.config"))
    token = config_module.settings.discord_token.get_secret_value()
    if not token:
        raise SystemExit("DISCORD_TOKEN fehlt in ENV/.env")
    return token


def _install_signal_handlers(
    loop: asyncio.AbstractEventLoop, lifecycle: BotLifecycle
) -> Callable[[], None]:
    shutdown_started = False
    kill_timer: threading.Timer | None = None

    try:
        kill_after = float(os.getenv("KILL_AFTER_SECONDS", "2"))
    except ValueError:
        kill_after = 2.0

    def _arm_kill_timer() -> None:
        nonlocal kill_timer
        try:
            kill_timer = threading.Timer(
                kill_after,
                lambda: (
                    logging.error(f"Kill watchdog fired after {kill_after:.1f}s -> os._exit(2)"),
                    os._exit(2),
                ),
            )
            kill_timer.daemon = True
            kill_timer.start()
        except Exception as exc:  # pragma: no cover - defensive
            logging.getLogger(__name__).debug("Kill-Timer konnte nicht gestartet werden: %r", exc)

    def _cancel_kill_timer() -> None:
        nonlocal kill_timer
        if not kill_timer:
            return
        try:
            kill_timer.cancel()
        except Exception as exc:  # pragma: no cover - defensive
            logging.getLogger(__name__).debug("Kill-Timer konnte nicht gestoppt werden: %r", exc)
        kill_timer = None

    def _handle(signum, frame) -> None:  # pragma: no cover - system dependent
        nonlocal shutdown_started
        if shutdown_started:
            logging.error("Second signal received -> hard exit now.")
            os._exit(1)

        shutdown_started = True
        logging.info("Received signal %s, shutting down gracefully...", signum)
        _arm_kill_timer()
        try:
            loop.call_soon_threadsafe(
                asyncio.create_task,
                lifecycle.request_stop(reason=f"signal {signum}"),
            )
        except RuntimeError:
            os._exit(0)

    try:
        signal.signal(signal.SIGINT, _handle)
        signal.signal(signal.SIGTERM, _handle)
    except Exception as exc:
        logging.getLogger(__name__).debug(
            "Signal-Handler Registrierung teilweise fehlgeschlagen (OS?): %r", exc
        )

    return _cancel_kill_timer


async def main() -> None:
    try:
        mode = ensure_gateway_start_allowed()
    except RuntimeError as exc:
        logging.critical("%s", exc)
        raise SystemExit(2) from exc

    logging.info(
        "Runtime mode active: role=%s discord_gateway_enabled=%s",
        mode.role,
        mode.discord_gateway_enabled,
    )
    if not mode.discord_gateway_enabled:
        logging.info(
            "Discord Gateway disabled for role=%s. Skipping discord.Client login.",
            mode.role,
        )
        return

    lifecycle = BotLifecycle(token_loader=_load_fresh_token)
    loop = asyncio.get_running_loop()
    cancel_watchdog = _install_signal_handlers(loop, lifecycle)

    try:
        await lifecycle.run_forever()
    finally:
        cancel_watchdog()
        logging.info("Lifecycle beendet")


if __name__ == "__main__":
    _acquire_pid_lock()
    try:
        asyncio.run(main())
    finally:
        _release_pid_lock()
