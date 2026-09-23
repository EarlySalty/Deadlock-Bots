#!/usr/bin/env bash
# Immutable release archives; no remote install scripts and no global installation.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="$ROOT/.ci/bin"
mkdir -p "$DEST"
install_archive() {
  local tool="$1" url="$2" sha="$3" archive
  archive="$(mktemp "$ROOT/.ci/${tool}.XXXXXX.tar.gz")"
  curl --fail --silent --show-error --location --retry 3 "$url" --output "$archive"
  printf '%s  %s\n' "$sha" "$archive" | sha256sum --check --status
  tar -xzf "$archive" -C "$DEST" "$tool"
  rm "$archive"
  chmod 755 "$DEST/$tool"
}
for tool in "$@"; do
  case "$tool" in
    gitleaks)
      install_archive gitleaks https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_linux_x64.tar.gz 551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb ;;
    trivy)
      install_archive trivy https://github.com/aquasecurity/trivy/releases/download/v0.74.0/trivy_0.74.0_Linux-64bit.tar.gz 2ae6fe3ee734b7fdf11335663e18c75ea12dccc76062f09f164a3b0f8be4371a ;;
    actionlint)
      install_archive actionlint https://github.com/rhysd/actionlint/releases/download/v1.7.12/actionlint_1.7.12_linux_amd64.tar.gz 8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8 ;;
    zizmor)
      install_archive zizmor https://github.com/zizmorcore/zizmor/releases/download/v1.30.1/zizmor-x86_64-unknown-linux-gnu.tar.gz e65324f4430c2717591937edcec90ccbefaf14c174f8ec9415e03ca875b46e1a ;;
    *) printf 'Unknown tool: %s\n' "$tool" >&2; exit 64 ;;
  esac
done
