# WORKER-ERGEBNIS: Brain on support tickets (2026-11-06)

## Commits and branches

| Repo | Branch | Head SHA | Remote |
|---|---|---|---|
| Deadlock-Brain | fix/brain-ticket-capability-20261106 | 7af8a54e42b6953f9afacbd9a3d4d14bc590c4b6 | pushed, contains origin/main de49aa64 |
| Deadlock-Bots | fix/brain-ticket-capability-20261106 | 44b43385506c1d7e50b8ac99f9310eade387d740 | pushed (new branch) |

- Bots `f0949787` is the WIP commit, `44b43385` is the merge of origin/main (Cargo.lock and dl-brain/Cargo.toml rev `7af8a54e42b6953f9afacbd9a3d4d14bc590c4b6` for brain-client and brain-contracts).
- Brain `9d3c71ce` adds the ticket parity test, `7af8a54e` merges origin/main into the branch.

## Scope status

1. Bots WIP committed, origin/main merged without force, main features kept (`ConversationReply::Answered { text, actions }`, `sent_at_unix`). Done.
2. `project_bounded_with_unverified` does not exist in the branch. Production code has no "Ungeprüft:" stripping. Bots map Answered to Answer and everything else to NoAnswer. Done.
   - Intake acknowledgements without citations still come back Unverified (see decision below).
3. Concierge and Faq routing in `answer_discord_task_with_history` is identical to origin/main (`answer_query`, 1800). Only the Ticket branch (`answer_ticket`) is new. Done.
4. Probe against a local Brain: NOT run, see blockers.
5. Checks: see below.
6. This file, plus the grant location below.

## Decision: Unverified ticket intake acknowledgements

No Brain change. The probe could not be run, so there is no evidence that intake acknowledgements come back Unverified in practice. A narrow uncited exception would also need a safe rule for "intake without factual claims"; `uncited_smalltalk_allowed` (brain-contracts discord_task.rs) only covers concierge smalltalk with empty evidence and no conversation. Without a verified case, the Brain keeps `Unverified` and the bot keeps `NoAnswer`. Revisit after the probe runs on a working Brain.

## Checks

Brain (`cargo-slot`, verify profile):
- `cargo-slot fmt --all -- --check` in `rust/`: exit 0.
- `cargo-slot clippy -p brain-contracts -p brain-kernel -p brain-api -p brain-serve --all-targets -- -D warnings`: exit 0 (log /tmp/brain-clippy.log).
- `cargo-slot test -p brain-contracts -p brain-kernel -p brain-api -p brain-serve`: NOT run. cargo-slot routes all test jobs of these crates to ionos (DB test rule). ionos (`naniadm@100.89.130.17`) unreachable, exit 78. Blocker.

Bots (`cargo-slot`):
- `cargo-slot check -p dl-brain -p dl-community --all-targets`: exit 0.
- Warnings (dead code, `block_next_port_call`, `evidence_server`, `gated_knowledge_server`, `gated_two_response_knowledge_server`, `gate_first_call`): same call counts as origin/main in faq.rs and answer_support.rs, so pre-existing for the non-feature test build.
- `cargo-slot fmt --all -- --check`: exit 1, but only in files this branch does not touch (dl-verbinder, dl-activity, dl-answer, dl-changelog, dl-dashboard, dl-voice). dl-brain and dl-community are clean. Pre-existing on main, not in scope.
- `cargo-slot test -p dl-brain`: 37 passed (earlier run, before the merge commit; no later code change in dl-brain).
- `cargo-slot test -p dl-community`: earlier run 278 passed, 4 failed, all because `CENTRAL_TEST_DSN` is missing (DB tests, ionos only).

Known baseline failures (brain-api Twitch-title x2, brain-serve Ranked-Lane panel) could not be re-measured because the Brain test jobs need ionos.

## Probe

NOT run. Blockers:
- A local brain-serve needs the Postgres role `brain_serve` (peer auth via `/run/postgresql`). The local user cannot assume it, and no secret was read to work around that.
- The provider is external AI. The real opening texts contain member data; sending them needs an explicit go, or a loopback provider from the central router.
- The existing example `dl-brain/examples/discord_task_probe.rs` uses synthetic texts (Steam link, skinscam, report with ID, ban, seven items question), not the five real openings. It needs `DISCORD_BRAIN_PROBE_ENDPOINT`, `DISCORD_BRAIN_PROBE_TIMEOUT_MS`, `DISCORD_BRAIN_CLIENT_TOKEN`, `DISCORD_BRAIN_PROBE_CHANNEL_ID`.

Acceptance is therefore open: no support redirect, intake acknowledgement for the 4 moderation cases.

## Production grant for discord_ticket

- Config key: `bot_task_consumers` in the brain-serve config (`rust/crates/brain-serve/src/config.rs` line ~40, validated at line ~917). The production file is not in the repo. Locate it on v50671 from the running brain-serve unit before editing; do not copy secrets.
- Current dl-bot entry (from the test fixture shape): `{"actor_id":"dl-bot","channel":"discord","capabilities":["discord_concierge","discord_faq"]}`. Add `"discord_ticket"` to that list.
- Bots side: `BotCapability::DiscordTicket` = `discord_ticket` (brain-contracts bot_tasks.rs).
- Grant and Brain deploy are integrator steps, after gate-review on both repos.

## Not done (out of scope or blocked)

- No Discord posts, no deploys, no restarts, no merge to main.
- No gate-review run yet (integrator step).
