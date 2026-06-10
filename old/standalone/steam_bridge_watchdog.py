#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import logging
import os
import shlex
import signal
import sqlite3
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

LOG = logging.getLogger("steam_bridge_watchdog")
STOP_REQUESTED = False


def _default_db_path() -> Path:
    env_path = (os.getenv("DEADLOCK_DB_PATH") or "").strip()
    if env_path:
        return Path(env_path).expanduser()
    env_dir = (os.getenv("DEADLOCK_DB_DIR") or "").strip()
    if env_dir:
        return Path(env_dir).expanduser() / "deadlock.sqlite3"
    return Path(__file__).resolve().parents[1] / "data" / "deadlock.sqlite3"


def _default_state_path() -> Path:
    state_home = (os.getenv("XDG_STATE_HOME") or "").strip()
    if state_home:
        base = Path(state_home).expanduser()
    else:
        base = Path.home() / ".local" / "state"
    return base / "deadlock-bots" / "steam_bridge_watchdog.json"


def _parse_bool_env(name: str, default: bool = False) -> bool:
    raw = os.getenv(name)
    if raw is None:
        return default
    return raw.strip().lower() in {"1", "true", "yes", "y", "on"}


@dataclass(slots=True)
class HealthIssue:
    reason: str
    summary: str
    details: dict[str, Any]


def _extract_last_error_message(value: Any) -> str:
    if isinstance(value, dict):
        return str(value.get("message") or "").strip()
    if value:
        return str(value).strip()
    return ""


def _detect_health_issue(snapshot: dict[str, Any]) -> HealthIssue | None:
    runtime = snapshot.get("runtime", {}) if isinstance(snapshot, dict) else {}
    diagnostics = snapshot.get("diagnostics", {}) if isinstance(snapshot, dict) else {}

    logged_on = bool(runtime.get("logged_on", False))
    logging_in = bool(runtime.get("logging_in", False))
    steam_id64 = str(runtime.get("steam_id64") or "").strip()
    last_error_message = _extract_last_error_message(runtime.get("last_error"))
    recent_failed_friend_requests = int(diagnostics.get("recent_failed_friend_requests", 0) or 0)

    oldest_pending_friend_request_age = diagnostics.get("oldest_pending_friend_request_age_seconds")
    if oldest_pending_friend_request_age is not None:
        try:
            oldest_pending_friend_request_age = int(oldest_pending_friend_request_age)
        except (TypeError, ValueError):
            oldest_pending_friend_request_age = None

    details = {
        "logged_on": logged_on,
        "logging_in": logging_in,
        "steam_id64": steam_id64 or None,
        "last_error": last_error_message or None,
        "recent_failed_friend_requests": recent_failed_friend_requests,
        "oldest_pending_friend_request_age_seconds": oldest_pending_friend_request_age,
    }

    if not logging_in and not logged_on:
        return HealthIssue(
            reason="not_logged_in",
            summary="Bridge läuft, ist aber nicht bei Steam eingeloggt.",
            details=details,
        )

    if logged_on and not steam_id64:
        return HealthIssue(
            reason="missing_steam_id",
            summary="Bridge meldet Login, aber keine Steam-ID.",
            details=details,
        )

    stalled_friend_requests = (
        recent_failed_friend_requests >= 2
        and (oldest_pending_friend_request_age or 0) >= 120
        and last_error_message.lower() in {"noconnection", "not logged in", "request timed out"}
    )
    if stalled_friend_requests:
        return HealthIssue(
            reason="friend_requests_stalled",
            summary="Steam-Friend-Requests laufen in Timeouts und hängen fest.",
            details=details,
        )

    return None


def _load_snapshot(db_path: Path) -> tuple[int, dict[str, Any]]:
    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    try:
        row = conn.execute(
            "SELECT heartbeat, payload FROM standalone_bot_state WHERE bot=?",
            ("steam",),
        ).fetchone()
        if row is None:
            return 0, {}

        payload: dict[str, Any] = {}
        raw_payload = row["payload"]
        if raw_payload:
            try:
                payload = json.loads(raw_payload)
            except Exception:
                LOG.warning("Standalone state payload is not valid JSON")
                payload = {}

        now_ts = int(time.time())
        recent_failed_friend_requests_row = conn.execute(
            """
            SELECT COUNT(*) AS count
              FROM steam_tasks
             WHERE type='AUTH_SEND_FRIEND_REQUEST'
               AND status='FAILED'
               AND updated_at >= ?
            """,
            (now_ts - 900,),
        ).fetchone()
        oldest_pending_friend_request_row = conn.execute(
            """
            SELECT MIN(requested_at) AS oldest_requested_at
              FROM steam_friend_requests
             WHERE status='pending'
            """
        ).fetchone()

        diagnostics = {
            "recent_failed_friend_requests": int(recent_failed_friend_requests_row["count"] or 0)
            if recent_failed_friend_requests_row
            else 0,
            "oldest_pending_friend_request_age_seconds": None,
        }
        if (
            oldest_pending_friend_request_row
            and oldest_pending_friend_request_row["oldest_requested_at"] is not None
        ):
            diagnostics["oldest_pending_friend_request_age_seconds"] = max(
                0, now_ts - int(oldest_pending_friend_request_row["oldest_requested_at"])
            )

        payload["diagnostics"] = diagnostics
        return int(row["heartbeat"] or 0), payload
    finally:
        conn.close()


def _load_state(path: Path) -> dict[str, Any]:
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return {}
    except Exception as exc:
        LOG.warning("Could not read watchdog state %s: %s", path, exc)
        return {}
    try:
        data = json.loads(raw)
    except Exception as exc:
        LOG.warning("Could not decode watchdog state %s: %s", path, exc)
        return {}
    return data if isinstance(data, dict) else {}


def _save_state(path: Path, state: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = path.with_suffix(path.suffix + ".tmp")
    tmp_path.write_text(json.dumps(state, indent=2, sort_keys=True), encoding="utf-8")
    tmp_path.replace(path)


def _restart_service(command: list[str], dry_run: bool) -> bool:
    LOG.warning("Restarting Steam bridge host service via: %s", shlex.join(command))
    if dry_run:
        return True
    try:
        completed = subprocess.run(command, check=False, capture_output=True, text=True)
    except Exception as exc:
        LOG.error("Restart command failed to execute: %s", exc)
        return False
    if completed.returncode != 0:
        stderr = (completed.stderr or "").strip()
        stdout = (completed.stdout or "").strip()
        LOG.error(
            "Restart command failed with exit code %s: %s%s",
            completed.returncode,
            stderr,
            f" | stdout={stdout}" if stdout else "",
        )
        return False
    return True


def _systemd_user_properties(service_name: str) -> dict[str, str]:
    if not service_name:
        return {}
    try:
        completed = subprocess.run(
            [
                "systemctl",
                "--user",
                "show",
                service_name,
                "--property=ActiveState,SubState,Result",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
    except Exception as exc:
        LOG.warning("Could not query host service %s: %s", service_name, exc)
        return {}
    if completed.returncode != 0:
        LOG.warning(
            "Could not query host service %s: %s",
            service_name,
            (completed.stderr or completed.stdout or "").strip(),
        )
        return {}
    props: dict[str, str] = {}
    for line in completed.stdout.splitlines():
        key, sep, value = line.partition("=")
        if sep:
            props[key] = value.strip()
    return props


def _reset_failed_host_service(service_name: str, dry_run: bool) -> None:
    if not service_name:
        return
    LOG.warning("Resetting failed host service state: %s", service_name)
    if dry_run:
        return
    completed = subprocess.run(
        ["systemctl", "--user", "reset-failed", service_name],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        LOG.error(
            "reset-failed failed for %s: %s",
            service_name,
            (completed.stderr or completed.stdout or "").strip(),
        )


def _handle_signal(signum: int, _frame: Any) -> None:
    global STOP_REQUESTED
    STOP_REQUESTED = True
    LOG.info("Received signal %s, stopping watchdog", signum)


def _run_once(
    *,
    db_path: Path,
    state_path: Path,
    restart_command: list[str],
    grace_period: int,
    heartbeat_max_age: int,
    restart_cooldown: int,
    host_service: str,
    dry_run: bool,
) -> int:
    state = _load_state(state_path)
    now = time.time()

    try:
        heartbeat, payload = _load_snapshot(db_path)
    except Exception as exc:
        LOG.error("Failed to query DB %s: %s", db_path, exc)
        return 2

    if not heartbeat:
        LOG.info("No standalone state for steam bridge yet")
        state.clear()
        _save_state(state_path, state)
        return 0

    heartbeat_age = now - heartbeat
    if heartbeat_age > heartbeat_max_age:
        LOG.warning("Steam bridge heartbeat too old: %.1fs", heartbeat_age)
        issue = HealthIssue(
            reason="stale_heartbeat",
            summary="Steam bridge heartbeat ist veraltet.",
            details={"heartbeat_age_seconds": int(heartbeat_age)},
        )
    else:
        issue = _detect_health_issue(payload)

    if issue is None:
        if state:
            LOG.info("Steam bridge watchdog state recovered")
        state = {}
        _save_state(state_path, state)
        return 0

    previous_reason = str(state.get("reason") or "")
    if previous_reason != issue.reason:
        state = {
            "reason": issue.reason,
            "summary": issue.summary,
            "details": issue.details,
            "first_seen_at": now,
            "last_restart_at": state.get("last_restart_at"),
        }
        _save_state(state_path, state)
        LOG.warning("Detected steam bridge issue %s: %s", issue.reason, issue.summary)
        return 1

    unhealthy_for = max(0.0, now - float(state.get("first_seen_at") or now))
    if unhealthy_for < grace_period:
        LOG.info(
            "Steam bridge still unhealthy (%s) for %.1fs, waiting for grace period",
            issue.reason,
            unhealthy_for,
        )
        return 1

    last_restart_at = float(state.get("last_restart_at") or 0.0)
    cooldown_left = (last_restart_at + restart_cooldown) - now
    if cooldown_left > 0:
        LOG.info(
            "Steam bridge issue %s persists, restart cooldown active for %.1fs",
            issue.reason,
            cooldown_left,
        )
        return 1

    host_props = _systemd_user_properties(host_service)
    active_state = host_props.get("ActiveState", "")
    result = host_props.get("Result", "")
    if active_state == "inactive" and result in {"", "success"}:
        LOG.info(
            "Host service %s is intentionally inactive (result=%s); not restarting",
            host_service,
            result or "none",
        )
        return 0
    if active_state in {"activating", "deactivating"}:
        LOG.info("Host service %s is %s; waiting", host_service, active_state)
        return 1
    if active_state == "failed":
        _reset_failed_host_service(host_service, dry_run=dry_run)

    state["last_restart_at"] = now
    state["last_restart_reason"] = issue.reason
    _save_state(state_path, state)

    if _restart_service(restart_command, dry_run=dry_run):
        return 10
    return 3


def main() -> int:
    parser = argparse.ArgumentParser(
        description="External watchdog for the Steam bridge host service."
    )
    parser.add_argument("--db-path", default=str(_default_db_path()))
    parser.add_argument("--state-path", default=str(_default_state_path()))
    parser.add_argument(
        "--restart-command",
        default=os.getenv(
            "STEAM_BRIDGE_WATCHDOG_RESTART_COMMAND",
            "systemctl --user restart deadlock-bot.service",
        ),
        help="Command used to restart the host service.",
    )
    parser.add_argument(
        "--host-service",
        default=os.getenv("STEAM_BRIDGE_WATCHDOG_HOST_SERVICE", "deadlock-bot.service"),
        help="systemd --user service supervised by this watchdog.",
    )
    parser.add_argument(
        "--interval", type=int, default=int(os.getenv("STEAM_BRIDGE_WATCHDOG_INTERVAL", "30"))
    )
    parser.add_argument(
        "--grace-period",
        type=int,
        default=int(os.getenv("STEAM_BRIDGE_WATCHDOG_GRACE_PERIOD", "180")),
    )
    parser.add_argument(
        "--heartbeat-max-age",
        type=int,
        default=int(os.getenv("STEAM_BRIDGE_WATCHDOG_HEARTBEAT_MAX_AGE", "90")),
    )
    parser.add_argument(
        "--restart-cooldown",
        type=int,
        default=int(os.getenv("STEAM_BRIDGE_WATCHDOG_RESTART_COOLDOWN", "600")),
    )
    parser.add_argument("--once", action="store_true", help="Run one check and exit.")
    parser.add_argument(
        "--dry-run", action="store_true", help="Log restart actions without executing them."
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        default=_parse_bool_env("STEAM_BRIDGE_WATCHDOG_VERBOSE", False),
    )
    args = parser.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    restart_command = shlex.split(args.restart_command)
    if not restart_command:
        LOG.error("Restart command is empty")
        return 2

    db_path = Path(args.db_path).expanduser()
    state_path = Path(args.state_path).expanduser()

    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    LOG.info(
        "Steam bridge watchdog started: db=%s, state=%s, interval=%ss",
        db_path,
        state_path,
        args.interval,
    )

    while not STOP_REQUESTED:
        exit_code = _run_once(
            db_path=db_path,
            state_path=state_path,
            restart_command=restart_command,
            grace_period=max(30, args.grace_period),
            heartbeat_max_age=max(30, args.heartbeat_max_age),
            restart_cooldown=max(60, args.restart_cooldown),
            host_service=str(args.host_service or "").strip(),
            dry_run=args.dry_run,
        )
        if args.once:
            return exit_code
        sleep_for = max(5, args.interval)
        for _ in range(sleep_for):
            if STOP_REQUESTED:
                break
            time.sleep(1)

    return 0


if __name__ == "__main__":
    sys.exit(main())
