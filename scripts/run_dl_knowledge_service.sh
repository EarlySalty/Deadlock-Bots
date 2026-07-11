#!/usr/bin/env bash
# Startet dl-knowledge mit dem deployten, committeten public-Snapshot.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_FILE="${INFISICAL_CONFIG_FILE:-$HOME/.config/deadlock-bots/infisical.conf}"

if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "Missing Infisical config: $CONFIG_FILE" >&2
  exit 1
fi

set -a
source "$CONFIG_FILE"
set +a

if [[ -n "${CREDENTIALS_DIRECTORY:-}" && -f "$CREDENTIALS_DIRECTORY/infisical-token" ]]; then
  INFISICAL_SERVICE_TOKEN="$(<"$CREDENTIALS_DIRECTORY/infisical-token")"
  export INFISICAL_SERVICE_TOKEN
fi

if [[ -z "${INFISICAL_SERVICE_TOKEN:-}" ]]; then
  echo "INFISICAL_SERVICE_TOKEN nicht gesetzt (systemd-creds?)." >&2
  exit 1
fi

if [[ -x "$ROOT_DIR/.venv/bin/python" ]]; then
  PYTHON_BIN="${PYTHON_BIN:-$ROOT_DIR/.venv/bin/python}"
else
  PYTHON_BIN="${PYTHON_BIN:-python3}"
fi

INFISICAL_EXPORT="$("$PYTHON_BIN" "$ROOT_DIR/scripts/export_infisical_env.py" --format shell)"
eval "$INFISICAL_EXPORT"

export RUST_LOG="${RUST_LOG:-info}"
export DL_DOCS_PATH="${DL_DOCS_PATH:-$HOME/.local/share/dl-knowledge/current/public/}"

cd "$ROOT_DIR"
exec "$ROOT_DIR/rust/target/release/dl-knowledge"
