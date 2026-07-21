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

INFISICAL_LOADER="${INFISICAL_LOADER:-/home/naniadm/.local/bin/dl-infisical-env}"
if [[ "${DL_INFISICAL_READY:-0}" != "1" ]]; then
  if [[ ! -x "$INFISICAL_LOADER" ]]; then
    echo "Infisical loader nicht gefunden oder nicht ausführbar: $INFISICAL_LOADER" >&2
    exit 1
  fi
  export DL_INFISICAL_READY=1
  exec "$INFISICAL_LOADER" --profile all -- "$0" "$@"
fi
unset DL_INFISICAL_READY
unset INFISICAL_SERVICE_TOKEN

# Das Infisical-GITHUB_TOKEN hat keinen Zugriff aufs 2nd-Brain-Repo und würde
# den funktionierenden git-credential-Helper des Users übersteuern (403 beim
# Wiki-Pull/-Push). Der Feeder braucht keine GitHub-API-Tokens — weg damit.
unset GITHUB_TOKEN GH_TOKEN

exec "$ROOT_DIR/rust/target/release/dl-brain-feeder"
