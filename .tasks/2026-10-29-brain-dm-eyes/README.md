# Brain feedback reactions for concierge questions (2026-10-29)

## Problem
DM questions route through `dl_community::concierge` (log `Concierge: Antwort entschieden route=concierge source=brain_task`).
That path never reacted, so users saw nothing for 15 to 70 s. The old 👀/❌ feedback (0ff4cf74, 5fe14f9d) only lives in
`modglue.rs answer_discord_event`, which no longer sees DM questions because the concierge claims them
(`brain_direct_reaction_started` logged 0 times in 3 days).

## Change
- `ConciergePort` gains `remove_reaction` and `send_typing`; dl-bot glue implements them via `delete_reaction_me` and `broadcast_typing`.
- `handle_routed_message` passes the user message id as `feedback_message_id` through `AnswerOptions` and `AnswerBinding`.
- `answer_bound_decision` puts 👀 on the question right before the Brain request, keeps a typing indicator alive every 8 s while it runs
  (log `brain_concierge_reaction_started`).
- After delivery, `finish_brain_feedback` removes 👀 and adds ❌ only for technical failure: outcome Error, Timeout, or Uncertain without a
  delivered message (log `brain_concierge_reaction_finished`). Only for terminals with source `brain_task`; local answers get no reaction.
- NoAnswer (insufficient_evidence) on an explicit guild mention no longer posts the canned "Ich komme gerade nicht zuverlässig an mein Wissen" text.
  Error and Timeout still do, as before.
- `modglue.rs` BackendError branch left 👀 stuck and set no ❌; it now removes 👀, adds ❌ and logs `brain_direct_reaction_finished`.
- No double reaction: messages claimed by the concierge are skipped by the direct handler (`claimed_by_other_owner`), and Discord reaction PUTs are idempotent.

## Verification
- `cargo-slot clippy -p dl-community -p dl-bot --all-targets`: no new findings (only the repo-wide async_trait `double_must_use` baseline).
- `cargo fmt --check -p dl-community -p dl-bot`: clean.
- `cargo-slot test -p dl-community`: 356 passed, 3 failed (all `CENTRAL_TEST_DSN` harness, DB tests run only on ionos).
- `cargo-slot test -p dl-bot`: 260 passed, 86 failed (all DB harness), all modglue brain tests pass.
- New test `brain_question_shows_eyes_while_working_and_cross_only_on_failure` covers answered, insufficient and backend error.

## Live proof
See the final report in the T3 thread; check with
`journalctl --user -u deadlock-bot-rust.service | grep brain_concierge_reaction`.
