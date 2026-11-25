# Package D1b: gateway slot template and D1 runtime proof (Deadlock-Bots)

Read first: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-31-brain-zentral/RULES.md` and `AUFTRAG-D1.md` (context only, D1 is merged as 0741300f).

Worktree: create `/home/nathanael/.worktrees/bots-brain-zentral-d1b` from origin/main (0741300f or newer), branch `feat/brain-zentral-d1b`. Do not touch `/home/nathanael/repos/Deadlock-Bots` or other worktrees. No `git stash`.

## Background (verified 2026-11-01 13:45Z)

- The Brain (release 59fbd797, live) requests dl-ai-gateway slot `brain_bot_task` for every bot task (Discord concierge, voice hint, invite lounge, Twitch tasks). The live gateway config `/home/nathanael/.config/deadlock-bots/dl-ai-gateway.toml` now has `[slots.brain_bot_task]` (hand-added copy of `[slots.brain_answer]`), gateway PID 3369187.
- `install_discord_release` (rust/bin/dl-bot/examples/install_discord_release.rs, `install_gateway_config`) rewrites that live file from `config/dl-ai-gateway.toml` in the repo on every dl-bot install and restarts the gateway. The repo template only has `[slots.brain_answer]`, so the next install silently drops the slot again (happened 12:12Z today).

## Deliverables

1. `config/dl-ai-gateway.toml`: add `[slots.brain_bot_task]` with the same provider, model, timeout, tokens, attempts, backoff, deadline and fallback values as `brain_answer` (read them from the repo template, not from the live file; do not print the live file). No other config changes, no model changes.
2. Check the gateway accepts it: build `dl-ai-gateway` and run it with `--check-config --config <your repo template>`.
3. D1 runtime proof against the live Brain: the harness `/tmp/d1tools/d1_runtime_proof.rs` (uses `dl_brain::brain_api::BrainApiAnswerer` against `http://127.0.0.1:8788`, token from env `DISCORD_BRAIN_CLIENT_TOKEN`). Copy it as an example into your worktree (do not commit it), build it with `cargo-slot build --example ...` (see `/home/nathanael/Documents/claude-config/machines/v50671/RUST-BUILDS.md`), get the token only via `eval "$(/home/nathanael/Documents/Infisical/export_claude_secret.py --secret DISCORD_BRAIN_CLIENT_TOKEN)"` for the run and `unset` it afterwards. Run at least 6 concierge questions (German, about the Discord server, Deadlock patch facts, smalltalk, one off-topic). Report per question: outcome (Answered/Silent/error), latency, answer length; no tokens or secrets in the report. Expected: Answered for knowledge questions, Silent or short for smalltalk, no `provider_unavailable`. If a kernel defect appears (earlier runs showed `citation_invalid` on smalltalk and a channel read path ignoring the channel id), record the exact Brain journal line (`sudo -n journalctl -u brain-serve.service`) and report it as a Brain finding; do not patch around it in the bot.
4. Gate: `gate-review --repo /home/nathanael/.worktrees/bots-brain-zentral-d1b --base origin/main` until ALLOW. The gate blocks mechanically on new code comments, `.unwrap()` in production, Duration literals in timeout lines, hard-coded model names, more than 150 KB new lines. Every line in your diff is yours.
5. After ALLOW: publish to main with the refspec `HEAD:main` as a single git step per Bash call, verify with `git merge-base --is-ancestor` and its exit code, delete branch and worktree. No deploy; the orchestrator installs dl-bot.

Report in English: merge SHA, gate rounds and model, the proof table, Brain findings, MERGEPROTOKOLL[MS-1] line.
