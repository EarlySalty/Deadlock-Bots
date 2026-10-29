# Brain feedback reactions for concierge questions (2026-10-29)

## Problem
DM questions route through `dl_community::concierge` (log `Concierge: Antwort entschieden route=concierge source=brain_task`).
That path never reacted, so users saw nothing for 15 to 70 s. The old 👀/❌ feedback (0ff4cf74, 5fe14f9d) only lives in
`modglue.rs answer_discord_event`, which no longer sees DM questions because the concierge claims them
(`brain_direct_reaction_started` logged 0 times in 3 days).

## Change
- `ConciergePort` gains `remove_reaction` and `send_typing`; dl-bot glue implements them via `delete_reaction_me` and `broadcast_typing`.
- `handle_routed_message` creates a `BrainFeedback` for the user message and passes it through `AnswerOptions` and `AnswerBinding`.
- `answer_bound_decision` marks the feedback started, puts 👀 on the question right before the Brain request and keeps a typing
  indicator alive every 8 s concurrently with the request (log `brain_concierge_reaction_started`).
- The feedback is marked failed only by explicit technical failures: Brain request error, and delivery failure or timeout of the answer.
  Privacy overrides and missing notice ids no longer count as failure.
- After the user action, `finish_brain_feedback` removes 👀 and adds ❌ only when failed (log `brain_concierge_reaction_finished`).
  Feedback that never started (local answers, controls) gets no reaction.
- Text is sent only for Answered; insufficient_evidence in DMs stays silent. The explicit guild mention fallback text for NoAnswer was kept
  as before, because existing tests pin it; this conflicts with the silence rule and is left for a product decision.
- No double reaction: messages claimed by the concierge are skipped by the direct handler (`claimed_by_other_owner`), and Discord reaction PUTs are idempotent.

## Verification
- `cargo-slot clippy -p dl-community -p dl-bot --all-targets --features dl-community/testing`: no new findings (only the repo-wide async_trait `double_must_use` baseline).
- `cargo fmt --check -p dl-community -p dl-bot`: clean.
- `cargo-slot test -p dl-community`: 356 passed, 3 failed (all `CENTRAL_TEST_DSN` harness).
- `test-db-run deadlock_test -- cargo-slot test -p dl-community --features testing`: 365 passed, 290 failed, all with
  `role "deadlock" does not exist` in migration 2026092801 on the ionos test DB (infrastructure baseline, unrelated).
- `cargo-slot test -p dl-bot modglue`: 75 passed.
- New test `brain_question_shows_eyes_while_working_and_cross_only_on_failure` covers answered, insufficient and backend error.

## Live proof
See the final report in the T3 thread; check with
`journalctl --user -u deadlock-bot-rust.service | grep brain_concierge_reaction`.
