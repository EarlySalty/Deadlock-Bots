#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
: "${1:?A SARIF results directory is mandatory}"
if [ ! -d "$1" ]; then
  echo 'Missing SARIF results directory' >&2
  exit 1
fi
mapfile -d '' -t reports < <(find "$1" -type f -name '*.sarif' -print0)
if [ "${#reports[@]}" -eq 0 ]; then
  echo 'No SARIF report: analysis cannot be successful' >&2
  exit 1
fi
for report in "${reports[@]}"; do
  if ! jq -e -f "$ROOT/.github/ci/sarif-policy.jq" "$report" >/dev/null; then
    printf 'Blocking findings, scanner errors or malformed SARIF: %s\n' "$report" >&2
    exit 1
  fi
done
printf 'Validated %s SARIF report(s): no blocking findings or scanner errors\n' "${#reports[@]}"
