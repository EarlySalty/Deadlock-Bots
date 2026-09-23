#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p "$ROOT/.ci"
PROBE="$(mktemp -d "$ROOT/.ci/sarif-probe.XXXXXX")"
trap 'rm -rf "$PROBE"' EXIT
mkdir "$PROBE/results"
printf '%s\n' '{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"CodeQL","rules":[]}},"results":[],"invocations":[{"executionSuccessful":true}]}]}' > "$PROBE/clean.json"
cp "$PROBE/clean.json" "$PROBE/results/result.sarif"
bash "$ROOT/.github/ci/check-sarif.sh" "$PROBE/results"
negative() {
  if bash "$ROOT/.github/ci/check-sarif.sh" "$PROBE/results" >/dev/null 2>&1; then
    echo "SARIF policy accepted invalid case: $1" >&2
    exit 1
  fi
}
cases=(
  '.runs = []'
  'del(.runs[0].results)'
  '.runs[0].tool.driver.name = "Unexpected tool"'
  '.runs[0].invocations[0].executionSuccessful = false'
  'del(.runs[0].invocations)'
  '.runs[0].invocations = []'
  'del(.runs[0].invocations[0].executionSuccessful)'
  '.runs[0].invocations[0].executionSuccessful = "true"'
  '.runs[0].invocations[0].toolExecutionNotifications = [{"level":"warning"}]'
  '.runs[0].invocations[0].toolExecutionNotifications = [{"level":"error"}]'
  '.runs[0].invocations[0].toolConfigurationNotifications = [{"level":"error"}]'
  '.runs[0].results = [{"level":"warning","ruleId":"probe"}]'
  '.runs[0].results = [{"level":"error","ruleId":"probe"}]'
  '.runs[0].results = [{"level":"unknown","ruleId":"probe"}]'
  '.runs[0].results = [{"ruleId":"probe"}]'
  '.runs[0].results = [{"level":"note","ruleId":"probe","properties":{"security-severity":"9.0"}}]'
  '.runs[0].tool.driver.rules = [{"id":"probe","properties":{"security-severity":"9.0"}}] | .runs[0].results = [{"level":"note","ruleId":"probe"}]'
)
for expression in "${cases[@]}"; do
  jq "$expression" "$PROBE/clean.json" > "$PROBE/results/result.sarif"
  negative "$expression"
done
printf 'invalid json' > "$PROBE/results/result.sarif"
negative 'malformed JSON'
rm "$PROBE/results/result.sarif"
negative 'empty results directory'
rmdir "$PROBE/results"
negative 'missing results directory'
printf 'SARIF counterchecks: 1 clean + %s negative cases passed\n' "$((${#cases[@]} + 3))"
