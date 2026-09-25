#!/usr/bin/env bash
# Test the typed command port; never launch dl-bot, dl-knowledge or an ingester.
set -uo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
LOGS="$ROOT/.consumer-ci-reports"
mkdir -p "$LOGS/test-home"
CARGO_CACHE="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_CACHE="${RUSTUP_HOME:-$HOME/.rustup}"
printf 'check\texit_code\n' > "$LOGS/results.tsv"
{ git -C "$ROOT" rev-parse HEAD; cargo --version; rustc --version; } > "$LOGS/provenance.txt"
FAILED=0
run() {
  local label="$1"; shift
  local result=0
  (cd "$ROOT" && env -i PATH="$PATH" HOME="$LOGS/test-home" CARGO_HOME="$CARGO_CACHE" RUSTUP_HOME="$RUSTUP_CACHE" CARGO_BUILD_JOBS=2 CARGO_NET_OFFLINE=true SQLX_OFFLINE=true LC_ALL=C.UTF-8 TZ=UTC "$@") > "$LOGS/$label.log" 2>&1 || result=$?
  printf '%s\t%s\n' "$label" "$result" >> "$LOGS/results.tsv"
  printf '%s exit=%s\n' "$label" "$result"
  if ((result != 0)); then FAILED=1; tail -n 60 "$LOGS/$label.log"; fi
}
run fmt cargo fmt --manifest-path rust/Cargo.toml -p dl-brain -- --check
run test cargo test --manifest-path rust/Cargo.toml -p dl-brain --all-targets --locked --offline
run clippy cargo clippy --manifest-path rust/Cargo.toml -p dl-brain --all-targets --locked --offline -- -D warnings
exit "$FAILED"
