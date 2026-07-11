#!/usr/bin/env bash
set -euo pipefail

INFISICAL_READY_URL="${INFISICAL_READY_URL:-http://127.0.0.1:8080/api/status}"
INFISICAL_READY_TIMEOUT="${INFISICAL_READY_TIMEOUT:-300}"
INFISICAL_READY_INTERVAL="${INFISICAL_READY_INTERVAL:-2}"

deadline=$((SECONDS + INFISICAL_READY_TIMEOUT))

while true; do
  if curl -fsS "$INFISICAL_READY_URL" >/dev/null; then
    exit 0
  fi

  if ((SECONDS >= deadline)); then
    echo "Timed out waiting for Infisical readiness at $INFISICAL_READY_URL" >&2
    exit 1
  fi

  sleep "$INFISICAL_READY_INTERVAL"
done
