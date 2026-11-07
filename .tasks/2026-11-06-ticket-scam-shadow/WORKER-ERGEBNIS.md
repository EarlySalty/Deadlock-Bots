# WORKER-ERGEBNIS: Ticket scam shadow (2026-11-07)

## Commits and branch

| Repo | Branch | Head | Remote |
|---|---|---|---|
| Deadlock-Bots | feat/ticket-scam-shadow-20261107 | 2ecf3c26 (after merge of origin/main 20c9bff4) | not pushed yet |

- `0edeeb35` is the feature commit (shadow triage, needs_mod, scam screenshot verdict).
- `2ecf3c26` fixes the two gate NITs: sorted dedup of resolved accounts and the matching test.
- `origin/main` (20c9bff4) is merged in without conflicts or force.

## Scope status

1. Collector keeps the opener's images (max 4, max 8 MiB each, same author only, Ticket Tool bot ignored). Done, unchanged.
2. One shadow entry per opening in log channel 1374364800817303632: Brain candidate, `needs_mod` with reasons, and for scam reports the analyzer plus verifier verdict, the resolved reported account, guild membership and `Aktion (Shadow): would ban <id>` / `kein Bann (...)`. Done, unchanged.
3. No ban, no ping, no member-visible message. Done: all output goes to the log channel through `FaqGlue::send_message`.

## Gate NITs from round 1

1. Length cap: `FaqGlue::send_message` builds `faq_message_body` -> `brain_v2_body`, which sets `BRAIN_V2_FLAG` (1 << 15) and enforces `BRAIN_V2_TEXT_LIMIT` 4000 UTF-16 units. The 3900 cap stays and fits Components V2. No code change. Existing dl-bot test `faq_message_body_deaktiviert_mentions` pins the V2 body.
2. Duplicate check: `shadow_action` now uses `sort_unstable` + `dedup`, so the same member named by ID and by name counts once regardless of order. Test `derselbe_gemeldete_account_ueber_id_und_name_zaehlt_einmal` covers one distinct non-opener member (would ban) and two distinct members (kein Bann, "mehrere Mitglieder genannt").
3. Data flow: ticket screenshots go to the moderation vision provider (`generate_multimodal` through the central dl_ai provider), same provider the scam check already uses. Flagged for the user; no approval exists yet for ticket screenshots.
4. Log volume: every opening produces one entry, including Brain NoAnswer cases. The brief requires exactly one entry per opening.
5. Screenshot content: `evaluate_reported_screenshots` uses an empty text and `[Bild ohne Text]` as prompt text, sends the images to the vision analyzer, and the verifier receives the images with trigger `ticket_scam_report`. Both system prompts instruct to judge the visible content and state that the screenshot format alone is no exoneration. No code change.

## Checks

- `cargo fmt -p dl-community -- --check`: exit 0.
- `cargo-slot test -p dl-community --lib`: 313 passed, 4 failed. The 4 failures are the known DB tests that need `CENTRAL_TEST_DSN` (ionos test instance, offline).
- `cargo-slot test -p dl-moderation`: 65 passed.
- `cargo-slot test -p dl-discord`: 67 passed plus 3 integration tests passed.
- `cargo-slot check -p dl-bot`: exit 0.
- `cargo-slot clippy -p dl-community -p dl-moderation -p dl-bot --no-deps`: exit 0. Only the workspace baseline lints remain (`double_must_use` from async_trait, `result_large_err`); ticket_triage.rs has only the async_trait baseline on the `ScreenshotReviewer` trait.

## Open items for the integrator and user

- Live replay of a real scam screenshot was not possible: no closed ticket with an image has been found, channel 1567507300216737894 contains text only.
- Ticket screenshots now reach the moderation vision provider. This needs explicit user approval before the shadow phase goes live.
- The moderation policy maps a confirmed scam to a timeout, while the log line follows the brief and says "would ban". Decide before switching to live bans.
- No merge to main, no deploy and no restart in this step.
