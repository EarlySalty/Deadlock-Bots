#!/usr/bin/env bash
# Prüft den typisierten Antwortport, ohne Bot oder Ingest-Dienste zu starten.
set -uo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
LOGS="$ROOT/.consumer-ci-reports/$(date -u +%Y%m%dT%H%M%S)-$(git -C "$ROOT" rev-parse --short=12 HEAD)"
mkdir -p "$LOGS"
CARGO_CACHE="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_CACHE="${RUSTUP_HOME:-$HOME/.rustup}"
TOOLCHAIN="stable"
RUSTUP="$HOME/.cargo/bin/rustup"
exec 8>/home/nathanael/Documents/.tasks/2026-10-02-offene-branches/locks/host-checks.lock || exit $?
flock -x 8 || exit $?
exec 9>/tmp/deadlock-cargo-release.lock || exit $?
flock -x 9 || exit $?
trap 'exec 9>&-; exec 8>&-' EXIT
host_probe() {
  local result
  while true; do
    ps -eo pid=,ppid=,stat=,comm= | awk '
      $3 !~ /^Z/ && ($4 == "cargo" || $4 == "rustc" || $4 == "clippy-driver" || $4 == "rustdoc") {
        print; busy = 1
      }
      END { exit busy ? 75 : 0 }
    ' >> "$LOGS/host-probe.log"
    result=$?
    if ((result == 0)); then return 0; fi
    if ((result != 75)); then return "$result"; fi
    sleep 30
  done
}
printf 'check\texit_code\n' > "$LOGS/results.tsv"
{
  git -C "$ROOT" rev-parse HEAD
  git -C "$ROOT" status --short
  sha256sum "$ROOT/rust/Cargo.lock"
  sha256sum "${BASH_SOURCE[0]}"
  "$RUSTUP" run "$TOOLCHAIN" cargo --version
  "$RUSTUP" run "$TOOLCHAIN" rustc --version
  free -m
  df -h "$HOME/.cache/rust-build"
} > "$LOGS/provenance.txt"
FAILED=0
run() {
  local label="$1"; shift
  local result=0
  if [[ "$1" == cargo ]]; then
    shift
    if [[ "$1" != fmt ]]; then
      host_probe || {
        result=$?
        FAILED=1
        printf '%s\t%s\n' "$label" "$result" >> "$LOGS/results.tsv"
        return "$result"
      }
      { free -m; df -h "$HOME/.cache/rust-build"; } >> "$LOGS/resources.log"
    fi
    set -- "$RUSTUP" run "$TOOLCHAIN" cargo --config "build.build-dir=\"$HOME/.cache/rust-build/{workspace-path-hash}\"" "$@"
  fi
  printf '%q ' "$@" > "$LOGS/$label.command"
  printf '\n' >> "$LOGS/$label.command"
  (cd "$ROOT" && env -i PATH="$HOME/.cargo/bin:$PATH" HOME="$HOME" CARGO_HOME="$CARGO_CACHE" RUSTUP_HOME="$RUSTUP_CACHE" CARGO_BUILD_JOBS=2 CARGO_NET_OFFLINE=true SQLX_OFFLINE=true LC_ALL=C.UTF-8 TZ=UTC "$@") > "$LOGS/$label.log" 2>&1 || result=$?
  printf '%s\t%s\n' "$label" "$result" >> "$LOGS/results.tsv"
  printf '%s exit=%s\n' "$label" "$result"
  if ((result != 0)); then FAILED=1; fi
}
run fmt cargo fmt --manifest-path rust/Cargo.toml -p dl-brain -- --check
run bot-fmt cargo fmt --manifest-path rust/Cargo.toml -p dl-bot -p dl-core -p dl-token-secrets -- --check
run token-secrets cargo test --manifest-path rust/Cargo.toml -p dl-token-secrets --all-targets --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run test cargo test --manifest-path rust/Cargo.toml -p dl-brain --all-targets --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run bot-brain cargo test --manifest-path rust/Cargo.toml -p dl-bot --bin dl-bot --features dl-central-db/testing brain_ --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run bot-mode cargo test --manifest-path rust/Cargo.toml -p dl-bot --bin dl-bot --features dl-central-db/testing typed_mode --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run core cargo test --manifest-path rust/Cargo.toml -p dl-core --all-targets --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run bot-shadow cargo test --manifest-path rust/Cargo.toml -p dl-bot --bin dl-bot --features dl-central-db/testing shadow_typed_probe --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run launcher cargo test --manifest-path rust/Cargo.toml -p dl-infisical-env --all-targets --locked --offline --jobs 2 -- --include-ignored --test-threads=2
run clippy cargo clippy --manifest-path rust/Cargo.toml -p dl-brain -p dl-core -p dl-token-secrets -p dl-bot --features dl-central-db/testing --all-targets --locked --offline --jobs 2 -- -D warnings
sha256sum "$ROOT/rust/Cargo.lock" > "$LOGS/lock-after.txt"
printf 'evidence=%s\n' "$LOGS"
exit "$FAILED"
