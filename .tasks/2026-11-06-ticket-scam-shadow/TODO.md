# TODO: ticket scam shadow (state 2026-11-12)

Worktree: /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107
Branch: feat/ticket-scam-shadow-20261107. Since 2026-11-12 the local history is uploaded as a plain fast-forward on top of fbd40262 (no rewrite).
Gate counter: 2 counted BLOCK rounds. The final round was NOT run; see GATE-RETRO.md "Decision needed".

DECISION (integrator, 2026-11-12 03:10 Berlin): option (c). No force push on any branch (CLAUDE.md). After tests and clippy pass, create `feat/ticket-scam-shadow-r2-20261112` from origin/main with the same final code as a few clean commits whose messages carry the full four-layer `Ursache:` block in the spelling the hook accepts, and upload it. Before the gate run, carry the counter over: set `block_rounds` in the new branch's file under `~/.local/state/gpt-workers/review-branch-rounds/` to at least 4, so the next verdict is the final round, and note in GATE-RETRO.md that this is a carry-over, not a reset. Run the final round only once the gate fix (fix/gate-round-four-20261112) is live. The `why_possible:` spelling rejection is a hook bug: add it as an open-cause row for the gate owner.
DECISION UPDATE (user, 2026-11-12 03:41 Berlin) REPLACES option (c): the user now allows a lease-protected force update of an own unmerged feature branch when only commit messages change. So keep `feat/ticket-scam-shadow-20261107`, no new branch, no counter carry-over. Reword the five published commits with the full four-layer `Ursache:` block, rebase the local fix commits on top, prove that every rewritten published commit has the same tree hash as its original, then lease-update the branch as its own simple git call. The next gate verdict is still the final round (round-four review already done); do not reset the counter. Run it only once the gate fix is live.
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
c. Do the message-only reword per DECISION UPDATE above, run the Ursache preflight (scratchpad ursache_check.py) on all commits, prove tree equality, then the lease update of the branch to origin as its own simple git call.
d. Do NOT run the final gate: report to the user the retro (GATE-RETRO.md), fixed vs dropped, and decision a/b/c.

## Amendment 2026-11-12: public scam reports (state after worker run)
Commits: 870f5cdb (fail-closed log field), 971aef9c (public report shadow path, screenshot account extraction), 4121a00f (German reason for a missing account, replay example).
- Trigger: a guild message outside the ticket category that pings role 1337518124647579661, or carries an image plus a scam word (`ticket_triage::is_public_report_trigger`). Further messages of the same author in the same channel join the report until 20 s quiet, at most 90 s (`faq_public_report.rs`).
- Same path as a ticket opening: `shadow_report_entry` with Brain task Ticket, `public_report_escalation` (always a moderation case), `ticket_scam_check`, member lookup and the same "would ban" rule.
- Screenshot account: the reported-screenshot verifier prompt asks for `reported_account` (user_id, username, display_name); `parse_screenshot_account` reads it. IDs and usernames go through the normal member lookup; a display name alone is listed but never a ban target.
- Shadow only: entry goes to 1374364800817303632 with a source link, verdict, reply candidate, reported account and ban decision. `moderation.ticket_screenshot_review` stays default false.
- Checks: `cargo-slot test -p dl-community --lib` 336 passed, 4 failed (ionos DB baseline: privacy, reaction_roles, two scrim_signup). `-p dl-moderation --lib` 68 passed. `cargo-slot check -p dl-bot --tests` exit 0. Clippy on dl-community with tests and examples: only baseline warnings (`double_must_use`, unused `evidence_server` that is also on origin/main).
- Replay (no Discord send, live Brain on 127.0.0.1:8788, switch off, placeholder image URLs): `cargo-slot build -p dl-community --example public_report_replay`, then run the binary with `DISCORD_BRAIN_PROBE_ENDPOINT`, `DISCORD_BRAIN_PROBE_TIMEOUT_MS` and the token from Infisical (`DISCORD_BRAIN_CLIENT_TOKEN`) on a JSON file with guild_id, channel_id, message_id, author_id and messages[{content, images[{url,size}]}]. Result for 1570393213364076626 plus 1570396144607957032: one entry, `Gemeldeter Account: keiner erkannt (im Text kein Konto genannt, Screenshot nicht ausgelesen, weil die Bildprüfung nicht gelaufen ist)`, `kein Bann (Bildprüfung deaktiviert)`.

Open points of the amendment:
1. Reply tone not met. The Brain task Ticket answers in its bot voice ("Das Team sieht sich den Fall an ... Welchen genauen Namen oder welche ID ...") and asks for a screenshot that was already attached. A casual human-mod reply that knows the outcome needs its own Brain task with an answer-shape rule in the Deadlock-Brain repo plus a rev bump; out of scope for this branch.
2. Message 1570396144607957032 came 11.5 minutes after the trigger, without ping or scam word. The live window (20 s quiet, 90 s max) does not collect it; only the replay merges both. Decide whether a longer window is wanted.
3. Screenshot account extraction is only tested with mocks. A real vision run needs the switch on, which sends member images to the moderation vision provider: user approval required.
4. The message-only reword with lease update from the DECISION UPDATE was not done; this worker's brief forbids any history rewrite.

## Next steps
1. User decision on the five published commits that fail the gate hook's Ursache check (GATE-RETRO.md, options a/b/c).
2. Then one final `gate-review --repo /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107 --base origin/main`.
3. On ALLOW: merge to main, deploy dl-bot via its release wrapper, prove one shadow entry live in 1374364800817303632 with the switch off, then remove branch, backup ref and worktree.
4. Acceptance item: a live replay with a real scam screenshot needs the switch on, which sends member images to the moderation vision provider. Needs the user's approval; before switching on, rebuild `dl-community-points-sync` so it accepts the new key.
5. Open causes listed in GATE-RETRO.md move to `claude-config/orchestrierung/OFFENE-URSACHEN.md` once that checkout is clean.

## Decisions for the user
- Screenshot review sends ticket images to the moderation vision provider. No approval yet; stays off.
- The policy maps a confirmed scam to a timeout; the shadow entry calls it "would ban" per the brief. Decide before live bans.
