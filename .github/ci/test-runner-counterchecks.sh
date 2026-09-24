#!/usr/bin/env bash
# Validate result accounting without invoking Cargo or starting any database.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
runner=(bash "$ROOT/.github/ci/run-cargo-tests.sh")
negative() {
  local status=0
  "${runner[@]}" "$@" >/dev/null 2>&1 || status=$?
  if [ "$status" -eq 0 ]; then
    echo 'Test result wrapper incorrectly accepted an incomplete or failing run' >&2
    exit 1
  fi
}
"${runner[@]}" printf 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n'
negative true
negative printf 'test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out\n'
negative printf 'test result: ok. 0 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out\n'
negative printf 'test result: ok. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n'
negative printf 'test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n'
negative bash -c 'printf "test result: ok. 1 passed; 0 failed;\n"; exit 101'
negative bash -c 'exit 143'
negative
printf 'Test runner counterchecks: 1 positive + 8 negative cases passed\n'
