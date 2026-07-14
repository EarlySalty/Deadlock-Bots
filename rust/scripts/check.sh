#!/usr/bin/env bash
# Qualitäts-Gate für den Rust-Workspace: Formatierung, Lints, Tests.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> cargo fmt --check"
cargo fmt --all --check

echo "==> cargo clippy (-D warnings)"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test"
./scripts/central_test_db.sh cargo test --workspace

echo "OK — alle Checks grün."
