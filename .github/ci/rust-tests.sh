#!/usr/bin/env bash
# Called only by central_test_db.sh, with the working directory set to rust/.
set -euo pipefail
: "${CENTRAL_TEST_DSN:?Disposable database is mandatory}"
: "${DATABASE_URL:?Disposable SQLx DSN is mandatory}"
: "${DEADLOCK_CENTRAL_DSN:?Disposable runtime DSN is mandatory}"
if [[ "$CENTRAL_TEST_DSN" != postgres://deadlock:testpw@127.0.0.1:*/deadlock_test ]] \
  || [[ "$CENTRAL_TEST_DSN" != "$DATABASE_URL" ]] \
  || [[ "$CENTRAL_TEST_DSN" != "$DEADLOCK_CENTRAL_DSN" ]] \
  || [[ "$CENTRAL_TEST_DSN" == *:5432/deadlock_test ]]; then
  echo "Refusing tests outside the isolated database wrapper" >&2
  exit 1
fi

# Preserve the established successful config CI commands and their targeted contracts.
cargo +1.88.0 test --locked -j 2 -p dl-core
cargo +1.88.0 test --locked -j 2 -p dl-bridges --lib steam_operating
cargo +1.88.0 test --locked -j 2 -p dl-dashboard --lib visual_brain_route_tests
cargo +1.88.0 test --locked -j 2 -p dl-dashboard --lib broker_evidence_requires_authenticated_fresh_envelope_and_pid
cargo +1.88.0 test --locked -j 2 -p dl-bridges --lib typed_toml_host_reaches_actual_twitch_constructor
cargo +1.88.0 test --locked -j 2 -p dl-bot --bin dl-bot typed_operating_snapshot_reaches_community_moderation_and_ai_constructors
cargo +1.88.0 test --locked -j 2 -p dl-bot --bin dl-bot dl_bot_baut_keine_llm_clients_mehr_selbst

# Run ignored DB tests too. Only seven named external-data/live-API acceptance
# tests are excluded, not entire crates or all ignored tests. See SECURITY-CI.md.
cargo +1.88.0 test --locked --workspace --all-features -j 2 -- --include-ignored --test-threads=2 \
  --skip six_live_questions_against_approved_snapshot \
  --skip golden_retrieval_corpus \
  --skip golden_live_api \
  --skip deadlock_sqlite3_real_snapshot_loads_and_verifies \
  --skip tournament_snapshot_loads_and_verifies_real_data \
  --skip website_source_snapshot_loads_and_round_trips_real_data \
  --skip final_reconciliation_dry_run_real_copies_is_read_only

# Existing hybrid suite includes optional native model backends, not live LLM calls.
cargo +1.88.0 test --locked -j 2 -p dl-knowledge --all-features -- --test-threads=2
cargo +1.88.0 clippy --locked -j 2 -p dl-knowledge --all-features --all-targets -- -D warnings
# Existing schema/feature-specific contracts are mandatory, never silently skipped.
cargo +1.88.0 test --locked -j 2 -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --test-threads=1
cargo +1.88.0 test --locked -j 2 -p dl-central-db --features testing --test scrim_db_foundation -- --ignored --test-threads=1
cargo +1.88.0 test --locked -j 2 -p dl-community --features testing privacy::tests::scrim_ --lib
