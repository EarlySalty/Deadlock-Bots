# Gateway anthropic-workspace-id header

## State
- Branch fix/2026-11-03-gateway-workspace-header, commit 8474a872, on origin as branch. Not on main.
- Done: workspace_id_env in ProviderConfig, header sent when set (sensitive), startup fails naming the env var when empty, unit test present/absent, template config and live candidate /home/nathanael/.config/deadlock-bots/dl-ai-gateway.toml.api-first-20261103 updated.
- Probe under dl-infisical-env launcher: ANTHROPIC_API_KEY present, ANTHROPIC_WORKSPACE_ID present (no values printed).
- cargo test -p dl-ai: 150 passed. Clippy: no findings in touched files (pre-existing must_use in dl-ai lib/usage/transparency_log/backend/router, dl-changelog, dl-broker, dl-discord, dl-bridges). fmt: touched files clean; many pre-existing diffs (dl-verbinder, dl-activity, dl-voice and others).
- Gate round 1: BLOCK (log /tmp/ws-gate.txt).
  1. BLOCKING: rust/bin/dl-bot/examples/install_discord_release.rs:395 installs config/dl-ai-gateway.toml, rollback at :402 restores only binaries; old parser denies unknown field workspace_id_env, so a rolled-back gateway cannot start. Fix: back up and restore the previous gateway config together with the binaries on rollback.
  2. NIT: --check-config returns before the secret lookup; deployment must verify the snapshot value (done via launcher probe, mention in report).

- Round 1 fix: installer hard-links the previous gateway config to .dl-ai-gateway.toml.previous before replacing it; activate or restart failure restores it (or removes the new config when none existed); backup discarded after a successful restart. Test rollback_restores_or_removes_the_gateway_config; install_discord_release example tests 7 passed; cargo-slot check -p dl-bot --examples clean. Ledger line written.

- Gate round 2 on 40363ddb: [gpt-6.1-sol] ALLOW (log /tmp/ws-gate-r2.txt). NIT: workspace lookup and startup rejection lack regression tests (mod.rs:76). Retro ledger line written. Round 1 NIT stays open: --check-config does not verify the secret lookup; launcher probe covered it.
- Remaining for the integrator: publish to main, then worktree and branch cleanup. No deploy or restart done.

## Next steps
1. Load gate-learning skill, write ledger lines.
2. Fix rollback in install_discord_release.rs (restore prior config alongside binaries), new commit, cargo-slot check -p dl-bot --examples, publish branch to origin.
3. gate-review --repo /home/nathanael/.worktrees/bots-gateway-workspace-20261103 --base origin/main until ALLOW.
4. Publish HEAD to main as a plain git command. No deploy, no gateway restart.
5. git worktree remove and git branch -d after merge-base --is-ancestor exit 0.
