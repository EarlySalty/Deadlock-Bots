#!/usr/bin/env bash
# CI-Drift-Gate: frische DB aus Migrationen muss dem erwarteten Schema-Vertrag entsprechen.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT"
./scripts/central_test_db.sh cargo test -p dl-central-db --test fresh_migrations_schema -- --ignored
./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test scrim_db_foundation -- --ignored
exec ./scripts/central_test_db.sh cargo test -p dl-community --features testing privacy::tests::scrim_ --lib
