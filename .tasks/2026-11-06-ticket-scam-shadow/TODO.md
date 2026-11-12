# TODO: ticket scam shadow (state 2026-11-12)

Worktree: /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107
Branch: feat/ticket-scam-shadow-20261107. origin is still at fbd40262; the local commits after it are NOT on the remote yet (tests unverified).
Gate counter: 2 counted BLOCK rounds. The final round was NOT run; see GATE-RETRO.md "Decision needed".
Backup of the pre-retro local history: ref `backup/ticket-scam-shadow-pre-retro` (8b038e44).

## What the branch does
- The collector gathers every message of the ticket opener until the text has been quiet for 45 s, at most 180 s. Images alone do not end the window. Up to `dl_ai::MAX_MULTIMODAL_IMAGES` images of at most `dl_ai::MAX_OPENAI_IMAGE_BYTES` each are kept; skipped images and text cut at the Brain question limit are counted and shown in the entry.
- One shadow entry per ticket opening in log channel 1374364800817303632. The transport (`faq_message_body` -> `brain_v2_body`) enforces the 4000-unit limit with a marker.
- Entry: Brain state with candidate, escalation reasons, intake counts, and for scam reports the screenshot review state, the reported accounts with guild membership, the policy action and `Bann-Entscheidung (Shadow, nur Log)`.
- No ban, no ping, no member-visible message. Bot messages in tickets (Ticket Tool) are dropped earlier by `gateway.rs` `is_user_message`.
- Screenshot review: `ScreenshotReviewSetup::from_config(switch, moderator)` gives Off, ModerationAiMissing or Active. Default off (serde default; the key is not written to `config/bot.toml`).
- Reported accounts come from the ticket text only: user mentions, Discord user IDs (no Steam64 range), `@name`, or a label line such as `Username: name` with exactly one ASCII name. Prose or non-ASCII after a label marks the report unclear.
- "would ban" needs exactly one distinct member that is not the opener, not staff, not a bot, no unclear or non-member ref, at most five refs, and a confirmed scam verdict above the policy threshold.
- Name lookups: a search with hits but no exact username match is "unklar", never "kein Mitglied".

## Stopped 2026-11-12 ~03:00 (quota emergency)
Done: retro fixer pass committed locally (3d9147dc tickets, ee5f5433 moderation, ef096693 members, config chore, docs commit on top). `cargo-slot check -p dl-community -p dl-moderation -p dl-ai -p dl-bot --tests` exit 0. Ursache preflight: all new commits pass; the five published ones still fail (as expected).
Not verified yet: unit tests. `cargo-slot test -p dl-community -p dl-moderation -p dl-bot --lib --bins` printed only failing dl-bot DB tests (build_publisher, self_invite, scrim_adapter, scrimglue: ionos DB baseline, untouched code); the output filter cut the `test result` lines, so dl-community and dl-moderation results are unknown.
Clippy on touched files: 219 lines in /tmp/claude-1000/-home-nathanael-Documents/8a85e52b-0943-4cac-a83e-4599f3e057da/scratchpad/clippy-touched.txt, first pages are the workspace baseline `double_must_use` from async_trait; not yet checked for lints on lines this branch changed.

Resume here:
a. `cd rust && cargo-slot test -p dl-community --lib 2>&1 | grep -E "^test result|FAILED"`, same for `-p dl-moderation --lib` and `-p dl-bot --bin dl-bot modglue`; baseline is 4 DB failures in dl-community. Fix failures in ticket_triage/faq/content_analyzer/modglue tests by amending the matching local commit (local history only).
b. Check clippy-touched.txt for lints on changed lines only (`git diff fbd40262 -U0`), fix those.
c. Run the Ursache preflight (scratchpad ursache_check.py), then a plain fast-forward upload of the branch to origin as its own simple git call.
d. Do NOT run the final gate: report to the user the retro (GATE-RETRO.md), fixed vs dropped, and decision a/b/c.

## Next steps
1. User decision on the five published commits that fail the gate hook's Ursache check (GATE-RETRO.md, options a/b/c).
2. Then one final `gate-review --repo /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107 --base origin/main`.
3. On ALLOW: merge to main, deploy dl-bot via its release wrapper, prove one shadow entry live in 1374364800817303632 with the switch off, then remove branch, backup ref and worktree.
4. Acceptance item: a live replay with a real scam screenshot needs the switch on, which sends member images to the moderation vision provider. Needs the user's approval; before switching on, rebuild `dl-community-points-sync` so it accepts the new key.
5. Open causes listed in GATE-RETRO.md move to `claude-config/orchestrierung/OFFENE-URSACHEN.md` once that checkout is clean.

## Decisions for the user
- Screenshot review sends ticket images to the moderation vision provider. No approval yet; stays off.
- The policy maps a confirmed scam to a timeout; the shadow entry calls it "would ban" per the brief. Decide before live bans.
