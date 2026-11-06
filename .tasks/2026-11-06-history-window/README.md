# History window: send message timestamps to the Brain

Goal: the Brain keeps history turns older than one hour out of the prompt and
only fetches them on demand (Brain branch `feat/history-window-20261106`,
task notes in Deadlock-Brain `.tasks/2026-11-06-history-window/README.md`).
dl-bot supplies the timestamps.

## Change

- `dl-brain`: `DialogEntry` gains `sent_at_unix: Option<i64>`;
  `bounded_dialog` forwards it as `DialogTurn.sent_at_unix` (non-positive
  values are dropped). Turn and character caps are unchanged.
- `dl-community` concierge (`recent_dialog_tx`) and FAQ history read
  `EXTRACT(EPOCH FROM created_at)::BIGINT` next to the content. Row limits
  (concierge 12, FAQ 5) and the user filter are unchanged; no new data leaves
  the bot except the timestamp of turns it already sent.
- Brain crates pinned to Deadlock-Brain main `7d97ae43acb46520735987a5258adf45bdf7f5d5`.

## Deploy order

`DialogTurn` is `deny_unknown_fields` in the Brain. The Brain release with
this change must be live before this dl-bot, otherwise concierge and FAQ
tasks with history are rejected.

## Unchanged on purpose

- Direct mentions (`BrainConversations`, 10 minute TTL) send no timestamps.
- Earlier-message images keep the 10 minute follow-up TTL.

## Checks

- `cargo-slot check --workspace --all-targets`: ok.
- `cargo fmt --check -p dl-brain -p dl-community`: ok.
- `cargo-slot clippy -p dl-brain -p dl-community -- -D warnings` fails with
  `clippy::double_must_use` on existing `async_trait` traits in dl-brain and
  dl-changelog; the same failure on origin/main 490d17bb.
- `cargo-slot test -p dl-brain`: 36 passed.
- `cargo-slot test -p dl-community`: 281 passed, 4 failed, all needing
  `CENTRAL_TEST_DSN` (privacy, reaction roles, scrim signup). The SQL change
  is not covered by a DB run here because the test DB host is offline.
