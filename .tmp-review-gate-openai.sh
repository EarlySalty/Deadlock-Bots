#!/usr/bin/env bash
set -euo pipefail
eval "$(python3 /home/nathanael/Documents/Infisical/export_gpt_secret.py --secret OPENAI_API_KEY)"
export PATH="/home/nathanael/.worktrees/discord-toml-rollout-20260920/.gate-bin:/home/nathanael/.local/codex-current/node_modules/.bin:/usr/local/bin:/usr/bin:/bin"
set +e
python3 /home/nathanael/migration/extra/.claude/gpt-workers/review_gate.py \
  --repo /home/nathanael/.worktrees/discord-toml-rollout-20260920 \
  --base fb357052 \
  --head HEAD \
  --model gpt-6-astra \
  --effort high \
  --timeout 900
rc=$?
set -e
unset OPENAI_API_KEY
exit "$rc"
