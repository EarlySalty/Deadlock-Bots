#!/usr/bin/env bash
# Wöchentlicher Lauf des Phase-2-Feeders (dl-brain-feeder): liest read-only
# Aggregate aus der zentralen PG (+ optional Twitch-Analytics-PG), rendert den
# Wochen-Digest und schreibt ihn ins Second-Brain-Wiki. Kein LLM.
# Secrets wie beim Hauptbot aus Infisical; gebraucht wird DEADLOCK_CENTRAL_DSN
# (optional TWITCH_ANALYTICS_DSN, DL_BRAIN_WIKI_ROOT). Läuft als systemd-Oneshot.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_FILE="${INFISICAL_CONFIG_FILE:-$HOME/.config/deadlock-bots/infisical.conf}"

if [[ ! -f "$CONFIG_FILE" ]]; then
  echo "Missing Infisical config: $CONFIG_FILE" >&2
  exit 1
fi

# Nicht-geheime Verbindungsparameter (API-URL, Project-ID, Env-Name).
set -a
source "$CONFIG_FILE"
set +a

# Bootstrap-Token aus systemd-Credentials (kein Plaintext auf der Platte).
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

# Secrets injizieren (DEADLOCK_CENTRAL_DSN u. a.).
INFISICAL_EXPORT="$("$PYTHON_BIN" "$ROOT_DIR/scripts/export_infisical_env.py" --format shell)"
eval "$INFISICAL_EXPORT"

exec "$ROOT_DIR/rust/target/release/dl-brain-feeder"
