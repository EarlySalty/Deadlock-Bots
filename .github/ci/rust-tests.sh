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
ROOT="$(cd .. && pwd)"
runner=(bash "$ROOT/.github/ci/run-cargo-tests.sh")

# Preserve the established successful config CI commands and their targeted contracts.
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-core
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-bridges --lib steam_operating
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-dashboard --lib visual_brain_route_tests
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-dashboard --lib broker_evidence_requires_authenticated_fresh_envelope_and_pid
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-bridges --lib typed_toml_host_reaches_actual_twitch_constructor
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-bot --bin dl-bot typed_operating_snapshot_reaches_community_moderation_and_ai_constructors
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-bot --bin dl-bot dl_bot_baut_keine_llm_clients_mehr_selbst

# --skip matches substrings. Prove each exception still matches exactly its one test.
mkdir -p "$ROOT/.ci"
rustc +1.88.0 --edition=2021 -D warnings "$ROOT/.github/ci/test_inventory.rs" -o "$ROOT/.ci/test-inventory"
cargo +1.88.0 test --locked --workspace --all-features -j 2 -- --list --format terse > "$ROOT/.ci/rust-test-inventory.txt"
"$ROOT/.ci/test-inventory" < "$ROOT/.ci/rust-test-inventory.txt"
skips=()
while IFS= read -r name; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  skips+=(--skip "$name")
done < "$ROOT/.github/ci/external-rust-tests.txt"

# Include ignored DB/FFmpeg tests. External snapshot/live-API tests are not a pass.
# ETL contracts share the disposable database and must not truncate each other's rows.
"${runner[@]}" cargo +1.88.0 test --locked --workspace --all-features --no-fail-fast -j 2 -- \
  --include-ignored --test-threads=1 "${skips[@]}"

# Existing hybrid suite includes optional native model backends, not live LLM calls.
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-knowledge --all-features -- --test-threads=1
cargo +1.88.0 clippy --locked -j 2 -p dl-knowledge --all-features --all-targets -- -D warnings
# Existing schema/feature-specific contracts are mandatory, never silently skipped.
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --test-threads=1
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-central-db --features testing --test scrim_db_foundation -- --ignored --test-threads=1
"${runner[@]}" cargo +1.88.0 test --locked -j 2 -p dl-community --features testing privacy::tests::scrim_ --lib
