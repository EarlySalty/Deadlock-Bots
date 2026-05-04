#!/usr/bin/env bash
# Kills any stale main_bot.py process before a fresh service start.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PID_FILE="$ROOT_DIR/master_bot.pid"

if [[ -f "$PID_FILE" ]]; then
    OLD_PID=$(cat "$PID_FILE" 2>/dev/null || true)
    if [[ -n "$OLD_PID" ]] && kill -0 "$OLD_PID" 2>/dev/null; then
        echo "Killing stale bot instance PID=$OLD_PID"
        kill "$OLD_PID" 2>/dev/null || true
        sleep 2
        kill -9 "$OLD_PID" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
fi

# Fallback: kill any remaining main_bot.py process from this venv
VENV_PYTHON="$ROOT_DIR/.venv/bin/python"
for pid in $(pgrep -f "$VENV_PYTHON" 2>/dev/null || true); do
    cmdline=$(cat "/proc/$pid/cmdline" 2>/dev/null | tr '\0' ' ' || true)
    if echo "$cmdline" | grep -q "main_bot\.py"; then
        echo "Fallback: killing stale bot PID=$pid"
        kill "$pid" 2>/dev/null || true
        sleep 2
        kill -9 "$pid" 2>/dev/null || true
    fi
done
