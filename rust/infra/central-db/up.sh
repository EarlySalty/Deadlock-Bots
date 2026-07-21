#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ -z "${DEADLOCK_CENTRAL_DSN:-}" ]]; then
  echo "DEADLOCK_CENTRAL_DSN fehlt in der Umgebung. Bitte vorher via Infisical laden; dieses Skript liest keine Secrets selbst." >&2
  exit 1
fi

url_decode() {
  local value="${1//+/ }"
  printf '%b' "${value//%/\\x}"
}

dsn_no_query="${DEADLOCK_CENTRAL_DSN%%\?*}"
dsn_authority_path="${dsn_no_query#*://}"
if [[ "$dsn_authority_path" != *"@"* ]]; then
  echo "DEADLOCK_CENTRAL_DSN enthaelt keine Userinfo." >&2
  exit 1
fi
dsn_userinfo="${dsn_authority_path%%@*}"
if [[ "$dsn_userinfo" != *":"* ]]; then
  echo "DEADLOCK_CENTRAL_DSN enthaelt kein Passwort." >&2
  exit 1
fi
POSTGRES_PASSWORD="$(url_decode "${dsn_userinfo#*:}")"
if [[ -z "$POSTGRES_PASSWORD" ]]; then
  echo "DEADLOCK_CENTRAL_DSN enthaelt kein Passwort." >&2
  exit 1
fi
export POSTGRES_PASSWORD

docker compose -f "$script_dir/docker-compose.yml" up -d

for _ in $(seq 1 30); do
  if docker exec deadlock-central-postgres pg_isready -U deadlock -d deadlock >/dev/null 2>&1; then
    echo "healthy"
    exit 0
  fi
  sleep 2
done

echo "timeout" >&2
exit 1
