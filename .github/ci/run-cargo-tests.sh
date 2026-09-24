#!/usr/bin/env bash
# Preserve Cargo's exit status and reject successful commands that ran no tests.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ "$#" -eq 0 ]; then
  echo 'A test command is required' >&2
  exit 64
fi
mkdir -p "$ROOT/.ci"
REPORT="$(mktemp "$ROOT/.ci/cargo-test-output.XXXXXX")"
trap 'rm -f "$REPORT"' EXIT
status=0
"$@" 2>&1 | tee "$REPORT" || status=$?
if [ "$status" -ne 0 ]; then
  exit "$status"
fi
# Cargo/libtest's stable text protocol. A build-only or empty filtered run is not a test.
if ! awk '
  /^test result: ok\. [0-9]+ passed; [0-9]+ failed;/ {
    passed += $4;
    failed += $6;
    summaries += 1;
  }
  END { exit !(summaries > 0 && passed > 0 && failed == 0) }
' "$REPORT"; then
  echo 'Test command did not prove a nonempty passing Rust test suite' >&2
  exit 1
fi
