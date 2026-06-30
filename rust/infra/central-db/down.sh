#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Kein -v: `docker compose down -v` loescht das persistente Volume und ist tabu.
docker compose -f "$script_dir/docker-compose.yml" down
