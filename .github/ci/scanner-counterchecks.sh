#!/usr/bin/env bash
# Harmless, generated scanner fixtures only. Never compiled, committed or deployed.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p "$ROOT/.ci"
PROBE="$(mktemp -d "$ROOT/.ci/scanner-probe.XXXXXX")"
trap 'rm -rf "$PROBE"' EXIT
cd "$PROBE"

expect_finding() {
  local status=0
  "$@" > finding.log 2>&1 || status=$?
  if [ "$status" -ne 1 ]; then
    printf 'Expected finding exit 1, received %s\n' "$status" >&2
    cat finding.log >&2
    exit 1
  fi
}
expect_error() {
  local status=0
  "$@" > error.log 2>&1 || status=$?
  if [ "$status" -eq 0 ]; then
    echo 'Scanner configuration failure was incorrectly successful' >&2
    exit 1
  fi
}
case "${1:-}" in
  gitleaks)
    gitleaks version
    printf 'This file contains no credentials.\n' > clean.txt
    gitleaks dir --redact --exit-code 1 --config "$ROOT/.gitleaks.toml" . >/dev/null 2>&1
    # Synthetic nonfunctional pattern, deliberately assembled only in the temporary tree.
    printf 'github_token = "%s%s"\n' 'ghp_' 'Ab3Cd4Ef5Gh6Ij7Kl8Mn9Op0Qr1St2Uv3Wx4' > credential.txt
    expect_finding gitleaks dir --redact --exit-code 1 --config "$ROOT/.gitleaks.toml" --report-format json --report-path findings.json .
    jq -e 'any(.[]; .RuleID == "github-pat")' findings.json >/dev/null
    expect_error gitleaks dir --redact --config "$PROBE/missing-config.toml" .
    ;;
  semgrep)
    semgrep --version
    printf 'fn safe() { let _ = 1 + 1; }\n' > clean.rs
    args=(scan --config "$ROOT/.semgrep/rules" --severity ERROR --severity WARNING --error --strict --metrics off --disable-version-check --no-git-ignore)
    semgrep "${args[@]}" clean.rs >/dev/null 2>&1
    printf '%s\n' 'fn probe() { let _ = reqwest::Client::builder().danger_accept_invalid_certs(true); }' > probe.rs
    expect_finding semgrep "${args[@]}" --json --output findings.json probe.rs
    jq -e '(.errors | length) == 0 and any(.results[]; (.check_id | endswith("rust.tls.certificate-validation-disabled")) and .extra.severity == "ERROR")' findings.json >/dev/null
    expect_error semgrep scan --config "$PROBE/missing-config.yml" --strict --error clean.rs
    ;;
  trivy)
    trivy --version
    mkdir clean unsafe
    printf 'This file contains no credentials.\n' > clean/plain.txt
    trivy fs --scanners secret --exit-code 1 clean >/dev/null 2>&1
    printf 'github_token = "%s%s"\n' 'ghp_' 'Ab3Cd4Ef5Gh6Ij7Kl8Mn9Op0Qr1St2Uv3Wx4' > unsafe/credential.txt
    expect_finding trivy fs --scanners secret --exit-code 1 --format json --output findings.json unsafe
    jq -e 'any(.Results[]?; any(.Secrets[]?; .RuleID == "github-pat"))' findings.json >/dev/null
    # Real HIGH configuration check: root is unsafe in this generated, never-built Dockerfile.
    printf 'FROM alpine:3.21\nUSER root\n' > unsafe/Dockerfile
    expect_finding trivy fs --scanners misconfig --severity HIGH,CRITICAL --exit-code 1 --format json --output config-findings.json unsafe
    jq -e 'any(.Results[]?; any(.Misconfigurations[]?; .Severity == "HIGH" or .Severity == "CRITICAL"))' config-findings.json >/dev/null
    # Never installed: an old dependency in a custom-named requirements fixture.
    printf 'requests==2.19.1\n' > unsafe/semgrep-requirements.txt
    expect_finding trivy fs --scanners vuln --severity HIGH,CRITICAL \
      --file-patterns 'pip:(^|/)(python|semgrep)-requirements\.txt$' \
      --exit-code 1 --list-all-pkgs --format json --output pip-findings.json unsafe
    jq -e 'any(.Results[]?; .Type == "pip" and (.Packages | length) > 0 and any(.Vulnerabilities[]?; .PkgName == "requests" and (.Severity == "HIGH" or .Severity == "CRITICAL")))' pip-findings.json >/dev/null
    expect_error trivy fs --scanners secret --config "$PROBE/missing-config.yml" clean
    ;;
  trivy-inventory)
    policy="$ROOT/.github/ci/trivy-inventory.jq"
    jq -n '
      def package($target; $type):
        {Target: $target, Class: "lang-pkgs", Type: $type,
         Packages: [{Name: "fixture", Version: "1.0.0"}]};
      {SchemaVersion: 2, Results: [
        package("rust/Cargo.lock"; "cargo"),
        package(".github/eslint-security/package-lock.json"; "npm"),
        package(".github/ci/python-requirements.txt"; "pip"),
        package(".github/ci/semgrep-requirements.txt"; "pip"),
        {Target: ".clusterfuzzlite/Dockerfile", Class: "config", Type: "dockerfile",
         MisconfSummary: {Successes: 1, Failures: 0}}
      ]}' > inventory-clean.json
    jq -e -s -f "$policy" inventory-clean.json >/dev/null
    jq -n '{ID: "fixture-root", Identifier: {UID: "fixture-uid"}, Relationship: "root", AnalyzedBy: "cargo", DependsOn: ["fixture@1.0.0"]}' > workspace-root.json
    jq --slurpfile root workspace-root.json '.Results[0].Packages += $root' inventory-clean.json > inventory-workspace.json
    jq -e -s -f "$policy" inventory-workspace.json >/dev/null
    mutations=(
      '.Results[0].Packages = $root'
      '.Results[2].Packages += $root'
      '.Results[0].Packages += ($root | map(del(.Identifier)))'
      '.Results[0].Packages += ($root | map(.AnalyzedBy = "unknown"))'
      '.Results[0].Packages += ($root | map(.DependsOn = []))'
      '.Results[0].Packages += ($root | map(.Name = "unversioned-real-package"))'
      'empty'
      '[]'
      'null'
      'true'
      '{}, .'
      '., .'
      'del(.SchemaVersion)'
      '.SchemaVersion = 999'
      'del(.Results)'
      '.Results = []'
      '.Results = {}'
      'del(.Results[0])'
      'del(.Results[1])'
      'del(.Results[2])'
      'del(.Results[3])'
      'del(.Results[4])'
      '.Results += [.Results[2]]'
      '.Results[2].Type = "unknown"'
      '.Results[2].Class = "config"'
      'del(.Results[2].Packages)'
      '.Results[2].Packages = []'
      '.Results[2].Packages = [null]'
      '.Results[2].Packages[0].Name = ""'
      'del(.Results[2].Packages[0].Version)'
      'del(.Results[4].MisconfSummary)'
      '.Results[4].MisconfSummary = {Successes: 0, Failures: 0}'
      '.Results[4].MisconfSummary = {Successes: "yes", Failures: "no"}'
      '.Results[4].MisconfSummary = {Successes: -1, Failures: 2}'
      '.Results[4].MisconfSummary = {Successes: 0.5, Failures: 0}'
    )
    for mutation in "${mutations[@]}"; do
      jq --slurpfile root workspace-root.json "$mutation" inventory-clean.json > inventory-bad.json
      if jq -e -s -f "$policy" inventory-bad.json >/dev/null 2>&1; then
        printf 'Trivy inventory incorrectly accepted: %s\n' "$mutation" >&2
        exit 1
      fi
    done
    printf 'Trivy inventory counterchecks: 2 positive + %s negative cases passed\n' "${#mutations[@]}"
    exit 0
    ;;
  *) echo 'usage: scanner-counterchecks.sh gitleaks|semgrep|trivy|trivy-inventory' >&2; exit 64 ;;
esac
printf '%s counterchecks: clean input passed; real findings and configuration errors blocked\n' "$1"
