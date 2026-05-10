#!/usr/bin/env bash
set -euo pipefail

requirements_file="${1:-}"

if [[ -z "$requirements_file" ]]; then
  echo "usage: $0 <requirements-lockfile>" >&2
  exit 64
fi

if [[ ! -f "$requirements_file" ]]; then
  echo "requirements file not found: $requirements_file" >&2
  exit 66
fi

python -m pip install --require-hashes -r "$requirements_file"
