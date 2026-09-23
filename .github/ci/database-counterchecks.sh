#!/usr/bin/env bash
# Failure-path probes use command shims, never a real database or Docker daemon.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p "$ROOT/.ci"
PROBE="$(mktemp -d "$ROOT/.ci/database-probe.XXXXXX")"
trap 'rm -rf "$PROBE"' EXIT
printf '#!/usr/bin/env bash\nexit 93\n' > "$PROBE/docker"
printf '#!/usr/bin/env bash\nprintf unexpected > "%s/cargo-called"\nexit 0\n' "$PROBE" > "$PROBE/cargo"
chmod 755 "$PROBE/docker" "$PROBE/cargo"
export PATH="$PROBE:$PATH"
negative() {
  local status=0
  "$@" > "$PROBE/output" 2>&1 || status=$?
  if [ "$status" -eq 0 ] || [ -e "$PROBE/cargo-called" ]; then
    echo 'Missing database prerequisite reached Cargo or was accepted' >&2
    exit 1
  fi
}
negative bash "$ROOT/rust/scripts/central_test_db.sh" true
unset CENTRAL_TEST_DSN DATABASE_URL DEADLOCK_CENTRAL_DSN
negative bash "$ROOT/.github/ci/rust-tests.sh"
export CENTRAL_TEST_DSN='postgres://deadlock:testpw@127.0.0.1:54321/deadlock_test'
negative bash "$ROOT/.github/ci/rust-tests.sh"
export DATABASE_URL="$CENTRAL_TEST_DSN"
negative bash "$ROOT/.github/ci/rust-tests.sh"
export DEADLOCK_CENTRAL_DSN='postgres://deadlock:testpw@127.0.0.1:54322/deadlock_test'
negative bash "$ROOT/.github/ci/rust-tests.sh"
export CENTRAL_TEST_DSN='postgres://deadlock:testpw@127.0.0.1:5432/deadlock_test'
export DATABASE_URL="$CENTRAL_TEST_DSN" DEADLOCK_CENTRAL_DSN="$CENTRAL_TEST_DSN"
negative bash "$ROOT/.github/ci/rust-tests.sh"
printf 'Database counterchecks: unavailable Docker and 5 missing/unsafe DSN cases blocked before Cargo\n'
