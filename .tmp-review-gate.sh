#!/usr/bin/env bash
set -euo pipefail
eval "$(python3 /home/nathanael/Documents/Infisical/export_gpt_secret.py --secret ANTHROPIC_API_KEY)"
set +e
export PATH="/home/nathanael/.worktrees/discord-toml-rollout-20260920/.gate-bin:$PATH"
python3 /home/nathanael/migration/extra/.claude/gpt-workers/review_gate.py \
  --repo /home/nathanael/.worktrees/discord-toml-rollout-20260920 \
  --base fb357052 \
  --head HEAD \
  --model claude-opus-5 \
  --effort high \
  --timeout 900
rc=$?
set -e
unset ANTHROPIC_API_KEY
exit "$rc"
