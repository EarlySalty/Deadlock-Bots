#!/usr/bin/env bash
# Wegwerfbarer Timescale-Testcontainer fuer zentrale DB-Integrationstests.
# Nutzt ein Throwaway-Passwort und hat keinen Bezug zur echten zentralen DB.
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 64
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="timescale/timescaledb:2.17.2-pg16"
NAME="dl-central-test-postgres-$$"
DB="deadlock"
USER="deadlock"
PASS="testpw"

cleanup() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

finish() {
  local status="${1:-$?}"
  trap - EXIT INT TERM
  cleanup
  exit "$status"
}
trap 'finish "$?"' EXIT
trap 'finish 130' INT
trap 'finish 143' TERM

docker run --rm -d \
  --name "$NAME" \
  -e POSTGRES_DB="$DB" \
  -e POSTGRES_USER="$USER" \
  -e POSTGRES_PASSWORD="$PASS" \
  -p "127.0.0.1:0:5432" \
  "$IMAGE" >/dev/null

PORT="$(docker port "$NAME" 5432/tcp | sed -E 's/.*:([0-9]+)$/\1/')"
if [[ ! "$PORT" =~ ^[0-9]+$ ]]; then
  echo "ungueltiger Host-Port fuer $NAME: ${PORT:-<leer>}" >&2
  exit 1
fi

CENTRAL_TEST_DSN="postgres://${USER}:${PASS}@127.0.0.1:${PORT}/${DB}"
export CENTRAL_TEST_DSN
export DEADLOCK_CENTRAL_DSN="$CENTRAL_TEST_DSN"
export TURNIER_TEST_DB_CONFIRM="throwaway-only"
export SQLX_OFFLINE=true

if ! command -v pg_isready >/dev/null 2>&1; then
  echo "pg_isready nicht gefunden; Host-DSN-Readiness kann nicht geprueft werden" >&2
  exit 1
fi

echo -n "warte auf zentrale Test-Postgres"
ready=0
for _ in $(seq 1 90); do
  count="$(docker logs "$NAME" 2>&1 | grep -c 'database system is ready to accept connections' || true)"
  if [ "${count:-0}" -ge 2 ] \
    && docker exec "$NAME" pg_isready -U "$USER" -d "$DB" >/dev/null 2>&1 \
    && pg_isready -h 127.0.0.1 -p "$PORT" -U "$USER" -d "$DB" >/dev/null 2>&1; then
    ready=1
    echo " ok"
    break
  fi
  echo -n "."
  sleep 1
done

if [ "$ready" -ne 1 ]; then
  echo " TIMEOUT" >&2
  docker logs "$NAME" 2>&1 | tail -50 >&2
  exit 1
fi

for _ in $(seq 1 15); do
  if docker logs "$NAME" 2>&1 | grep -q 'TimescaleDB background worker launcher connected'; then
    break
  fi
  sleep 1
done

echo "CENTRAL_TEST_DSN=postgres://${USER}:***@127.0.0.1:${PORT}/${DB}"

cd "$ROOT"
SQLX_OFFLINE=true DEADLOCK_CENTRAL_DSN="$CENTRAL_TEST_DSN" cargo run -p dl-central-migrate

set +e
"$@"
status=$?
set -e
finish "$status"
