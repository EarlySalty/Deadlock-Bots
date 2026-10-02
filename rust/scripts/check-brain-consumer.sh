#!/usr/bin/env bash
# Prüft den typisierten Antwortport, ohne Bot oder Ingest-Dienste zu starten.
set -uo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
LOGS="$ROOT/.consumer-ci-reports"
mkdir -p "$LOGS"
CARGO_CACHE="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_CACHE="${RUSTUP_HOME:-$HOME/.rustup}"
printf 'check\texit_code\n' > "$LOGS/results.tsv"
{ git -C "$ROOT" rev-parse HEAD; cargo --version; rustc --version; } > "$LOGS/provenance.txt"
FAILED=0
run() {
  local label="$1"; shift
  local result=0
  if [[ "$1" == cargo ]]; then
    shift
    set -- cargo --config "build.build-dir=\"$HOME/.cache/rust-build/{workspace-path-hash}\"" "$@"
  fi
  (cd "$ROOT" && env -i PATH="$PATH" HOME="$HOME" CARGO_HOME="$CARGO_CACHE" RUSTUP_HOME="$RUSTUP_CACHE" CARGO_BUILD_JOBS=2 CARGO_NET_OFFLINE=true SQLX_OFFLINE=true LC_ALL=C.UTF-8 TZ=UTC "$@") > "$LOGS/$label.log" 2>&1 || result=$?
  printf '%s\t%s\n' "$label" "$result" >> "$LOGS/results.tsv"
  printf '%s exit=%s\n' "$label" "$result"
  if ((result != 0)); then FAILED=1; tail -n 60 "$LOGS/$label.log"; fi
}
run fmt cargo fmt --manifest-path rust/Cargo.toml -p dl-brain -- --check
run bot-fmt cargo fmt --manifest-path rust/Cargo.toml -p dl-bot -p dl-core -p dl-token-secrets -- --check
run token-secrets cargo test --manifest-path rust/Cargo.toml -p dl-token-secrets --all-targets --locked --offline --jobs 2 -- --test-threads=2
run test cargo test --manifest-path rust/Cargo.toml -p dl-brain --all-targets --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run bot-brain cargo test --manifest-path rust/Cargo.toml -p dl-bot --bin dl-bot brain_ --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run bot-mode cargo test --manifest-path rust/Cargo.toml -p dl-bot --bin dl-bot typed_mode --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run core cargo test --manifest-path rust/Cargo.toml -p dl-core --lib --locked --offline --jobs 2 -- --test-threads=2
run bot-shadow cargo test --manifest-path rust/Cargo.toml -p dl-bot --bin dl-bot shadow_typed_probe --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run clippy cargo clippy --manifest-path rust/Cargo.toml -p dl-brain --all-targets --locked --offline --jobs 2 -- -D warnings
exit "$FAILED"
