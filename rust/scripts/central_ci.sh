#!/usr/bin/env bash
# Vier-Säulen-CI fuer die zentrale Postgres/TimescaleDB-Foundation.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

run_step() {
  local label="$1"
  shift

  echo
  echo "== ${label} =="

  set +e
  "$@"
  local status=$?
  set -e

  if [ "$status" -ne 0 ]; then
    echo "FEHLER: ${label} fehlgeschlagen (Exit ${status})." >&2
    exit "$status"
  fi
}

cd "$ROOT"

run_step \
  "SÄULE 1: Compile-Vertrag offline (.sqlx ohne DB)" \
  env -u DATABASE_URL SQLX_OFFLINE=true cargo build --workspace

# SP0-Gate lintet SP0-Crates, nicht fremden Legacy-Code.
run_step \
  "SÄULE 1b: Lint offline (clippy -D warnings)" \
  env -u DATABASE_URL SQLX_OFFLINE=true cargo clippy -p dl-central-db -p dl-central-etl -p dl-central-migrate --all-targets -- -D warnings

run_step \
  "SÄULE 4: ETL-Negativtests ohne DB" \
  env -u DATABASE_URL SQLX_OFFLINE=true cargo test -p dl-central-etl

run_step \
  "SÄULE 3: Schema-Drift-Gate mit frischer Wegwerf-DB" \
  ./scripts/central_fresh_schema.sh

run_step \
  "SÄULE 2: Cross-Boundary-Integration mit eigener Wegwerf-DB" \
  ./scripts/central_test_db.sh cargo test -p dl-central-db --test integration_core -- --ignored

echo
echo "ALLE VIER SÄULEN GRÜN"
