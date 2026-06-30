#!/usr/bin/env bash
# CI-Drift-Gate: frische DB aus Migrationen muss dem erwarteten Schema-Vertrag entsprechen.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT"
exec ./scripts/central_test_db.sh cargo test -p dl-central-db --test fresh_migrations_schema -- --ignored
