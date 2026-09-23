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
    expect_error trivy fs --scanners secret --config "$PROBE/missing-config.yml" clean
    ;;
  *) echo 'usage: scanner-counterchecks.sh gitleaks|semgrep|trivy' >&2; exit 64 ;;
esac
printf '%s counterchecks: clean input passed; real findings and configuration errors blocked\n' "$1"
