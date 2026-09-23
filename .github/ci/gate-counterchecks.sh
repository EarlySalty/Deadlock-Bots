#!/usr/bin/env bash
set -euo pipefail
jobs=(actions format rust python secrets dependencies semgrep trivy codeql)
for job in "${jobs[@]}"; do export "CI_RESULT_${job}=success"; done
.ci/required-gate
count=0
for job in "${jobs[@]}"; do
  for status in failure cancelled skipped neutral timed_out ''; do
    export "CI_RESULT_${job}=$status"
    if .ci/required-gate >/dev/null 2>&1; then
      echo "Gate accepted $job=$status" >&2
      exit 1
    fi
    count=$((count + 1))
  done
  unset "CI_RESULT_${job}"
  if .ci/required-gate >/dev/null 2>&1; then
    echo "Gate accepted missing $job" >&2
    exit 1
  fi
  count=$((count + 1))
  export "CI_RESULT_${job}=success"
done
printf 'Gate counterchecks: 1 positive + %d negative process-exit checks passed\n' "$count"
