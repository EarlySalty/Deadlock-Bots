# Invite lounge KI intent: handover

State 2026-11-17, after the gate round 3 fixer.

- Branch `feat/invite-lounge-ki-intent-20261116`, HEAD: see `git log -1`. The bot pins Brain 0ba41619.
- Baseline: locally (no CENTRAL_TEST_DSN) 4 dl-community tests are red (reaction_roles runtime_gate, privacy runtime_gate, two scrim_signup) and every dl-bot test that needs the test database fails in the harness; on ionos with the throwaway database dl-community, dl-central-db and dl-bot are fully green.
- Workspace clippy with `-D warnings` stops at `#[async_trait]` double_must_use on the trait declarations in dl-brain and dl-changelog lib.rs. The branch changes neither those trait lines nor the async-trait version, and dl-changelog (no branch diff) fails the same way, so it comes from the toolchain. Plain clippy reports no warning on a line this branch changed.

## Done in round 3

- Turn path: every failed read fails the turn and the catch-up retries it; live order per author; start time repaired before each catch-up; catch-up window equals the request window.
- One home per rule: open phases, hidden-member predicate, Discord epoch, friend code validity (`friend_code_id` alone), turn bounding (`dl_brain::brain_api::bounded_turn`), notice mapping (exhaustive match), `http` workspace dependency, FAQ texts bound to `CHECK_IN_CHANNEL_ID` and `CHECK_IN_SECONDS` by a test.
- Tests that could not fail now fail on regressions: follow-up rerun, Skipped vs Failed, scripted Brain applies the production projection and panics on an unscripted turn, per-turn facts, reply-target detection, maximal turn validates.
- User texts and docs match the code: check-in rule, invite signposts to the lounge, neutral friend code placeholder, no em dashes in the rewritten FAQ lines.
- Rollback: `hint_sent` is written again (read with default), README states the procedure for confirmed/reply outbox rows.
- Privacy: no live friend code or member handles in tests or task docs.

## Open

1. Integrator: re-pin to the final Brain head, then gate round 3.
2. Integrator: run the README rollback query once on the live database before relying on it (it was not run against a database in this round).
