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

## Orchestrator state 2026-11-18 ~00:15 CET (main session paused on usage limit)

- Brain: gate round 8 BLOCK (invite reply token-budget fix not applied to twin prompts). Round 9 fixer #2 running in background via integrator thread (fixer #1 lost to usage limit, not a round). Design: help question 500 B, invite/LFG judges drop `reasoning`, reactive pitch reply 700 B, Twitch lines 800 B, dialog answers token budget binding + test that a token-truncated answer is never posted. No budget values changed.
- Supporter 3387bbad and Bots 1571fc3b: gates running, no verdict yet. Bots needs a new Brain pin + gate rerun after Brain changes.
- No round cap for this package (user, 2026-11-17).
- Wakeup invite-gates6 at 2026-11-18 00:47 CET, prompt file /tmp/claude-1000/-home-nathanael-Documents/fe420793-3a99-41d7-8475-079a7e54af4a/scratchpad/wake-gates6.txt. Do not start a second Brain fixer while round 9 is active.

Next steps:
1. When the Brain fixer finishes: review grant for new Brain head, rerun Brain gate.
2. On supporter/Bots BLOCK: one fresh fixer per branch, max two agents total.
3. On Brain ALLOW: re-pin Brain in Bots branch, rerun Bots gate.
4. All ALLOW: merge Brain, then Bots, then supporter (rebase/merge main, gate again), deploy Brain + dl-bot from origin/main, restart, live-prove Lachs replay (Brain reply + invite), greenlight works, unrelated chat silent, supporter live; clean up; English report.
5. Open user decision: invite reply cap option (recommended option 1, ban emoji/control chars, ~83 chars).

## Orchestrator state 2026-11-18 ~02:00 CET

- Brain round 9 fixer #2 failed on the Claude API rate limit (not a round). Uncommitted diff left in the Brain worktree: persona.rs, bot_conversation.rs, invite.rs, wire_enum.rs (+175/-119). Next fixer continues from it, does not discard it.
- No gate processes running. Supporter and Bots verdicts still to be read.
- Wakeup invite-gates7 armed, prompt wake-gates7.txt in the session scratchpad.
- 02:xx: integrator resumed after quota reset; Brain round 9 fixer + Bots round 4 fixer running, supporter fixer queued. invite-gates7 cancelled to avoid duplicate fixers.
- 02:02: integrator failed again on rate limit. Wakeup invite-gates8 armed (wake-gates8.txt) carrying the Bots and supporter findings.
- 02:52: integrator alive again (Brain 52914b4c + route-constant fix, Bots r4 fixer running). invite-gates8 cancelled.

## State 2026-11-18 (main session, quota stop)
- Brain round 9 (eaad1fd2): BLOCK (192 NITs, 67 promoted; classes: 600000 ms / 2 MiB limit literals in touched files, duplicated wire literals, stale GATE-RETRO/STATUS claims, dead surface from r9). Integrator started a fresh round-10 fixer; log copied into this task folder, 4 gate-learning ledger entries added.
- Supporter gate (06b51ebb): still running, log sup_gate_r9.log in scratchpad 256566d7.../scratchpad; watcher bmttzztzk waits for "exit".
- Next: when r10 fixer reports, grant new Brain head, run gate-review. On Brain ALLOW: re-pin Brain in Bots branch, commit this TODO, grant, gate Bots. Supporter: on BLOCK fresh fixer, on ALLOW wait. Then merge Brain, Bots, supporter; deploy; live proof (Lachs replay, greenlight, unrelated chat silent, supporter live); cleanup; English report. Max two agents in parallel.

## State 2026-11-18 07:05 CET
- Integrator delegate died on rate limit before its r10 fixer ran. Main session started the Brain r10 fixer itself (gpt-6.2-sol, high), worktree brain-invite-turn-20261116, base eaad1fd2.
- Supporter gate run on 06b51ebb ended exit 2 (lost to usage limit, not a round). Its partial BLOCK listed: roles.rs:395 assert that cannot fail, roles.rs:381 commit message claims wrong, roles.rs:82 grant loop has no tick bound, store.rs:8 lock timeout derivation, store.rs:86 55P03 only mapped for LOCK TABLE. Another process restarted the supporter gate at 07:00; if it BLOCKs on these, start a supporter fixer with them.
- Gate CLI login may expire today (2026-11-18); user runs `claude auth login` if gate fails on auth.
- 07:10: integrator delegate is alive again (limit resume); it reran the supporter gate and resumed its own Brain r10 fixer. My duplicate r10 fixer (gpt-6.2-sol) failed on Codex quota (until 2026-11-19 17:38) with no edits. Integrator owns the flow; main session only relays.

## State 2026-11-18 12:20 CET (replacement integrator session b2df737a)
- Brain gate round 11 on 02dab305 RUNNING since ~12:03 under grant `invite-turn-20261118-r11`, log /tmp/claude-1000/-home-nathanael-Documents/b2df737a-7d87-4354-96ef-62656dd80768/scratchpad/brain_gate_r11.log. Do not start a second Brain gate or grant on 02dab305.
- Session 71c000d8 (original integrator) was relaunched and is active on the supporter worktree. b2df737a's supporter fixer edited and reverted supporter.rs and supporter/roles.rs between ~12:05 and 12:16, made no commits and has stopped; diff against your own 12:13/12:15 snapshots. Its review notes: /tmp/claude-1000/-home-nathanael-Documents/b2df737a-7d87-4354-96ef-62656dd80768/scratchpad/supporter_fixer_state.md. b2df737a stays out of the supporter worktree.
- Deploy blocker: 08470994 makes the loader reject Brain image values it used to clamp; check live bot.toml against 1..=4 and 40..=95 before any dl-bot deploy.
- 12:31: b2df737a's Brain gate r11 on 02dab305 ended "Autorisierter Review-Branch hat sich geändert" (exit 2, tool failure, not a round): session 71c000d8 committed and published 30360694, b3d20dba, fc6a11d6 on the Brain branch during the run. Grant invite-turn-20261118-r11 (02dab305) is spent/stale. 71c000d8 owns Brain and supporter; b2df737a stands down and starts no further gate, fixer or grant. Next gate needs a new grant for the final Brain head from the owning integrator.

## State 2026-11-18 13:05 CET (integrator 71c000d8)
- Brain: round-11 fixes done, head fc6a11d6 (published). Gate round 12 running under grant invite-turn-20261118-r12, log /tmp/claude-1000/-home-nathanael-Documents/71c000d8-5c9e-4a24-9e44-e1abc3f0f505/scratchpad/brain_gate_r12.log.
- Supporter: all r9/r9b findings fixed, head 2be114ff (published, ionos DB suites green). Gate running under grant supporter-slim-land-20261118-r10, log sup_gate_r10.log in the same scratchpad.
- Live bot.toml (~/.config/deadlock-bots/bot.toml) sets neither brain_image_max_count nor brain_image_jpeg_quality, so 08470994 is no deploy blocker.
- Next: on Brain ALLOW re-pin Brain in this branch (Brain public API unchanged for Bots; new PublicScope optional), commit this TODO, grant, gate. On BLOCK: fresh fixer per branch.

## State 2026-11-18 14:45 CET (integrator 71c000d8)
- Brain round 12 on fc6a11d6: BLOCK, 18 promoted low-weight twins (request scope literals, in_turn twin, cross-crate caps, passive_help rule text, STATUS.md layers). Round-13 fixer done (head a467376e), then origin/main (TF1) merged in as d0e54069 (conflicts in two tests resolved; 1223 passed, 0 failed; clippy clean). Gate round 13 running under grant invite-turn-20261118-r13, log brain_gate_r13.log in scratchpad 71c000d8.
- Supporter gate on 2be114ff still running (started 12:55).
- Bots branch and supporter branch are both current with origin/main.

## State 2026-11-18 ~15:35 CET (integrator 71c000d8, quota stop)
- Brain gate round 13 on d0e54069 (grant invite-turn-20261118-r13): still running at stop time. Log /tmp/claude-1000/-home-nathanael-Documents/71c000d8-5c9e-4a24-9e44-e1abc3f0f505/scratchpad/brain_gate_r13.log. If it ended BLOCK: copy log to verdicts/round-13.log, fresh fixer with a brief built like /tmp/claude-1000/-home-nathanael-Documents/71c000d8-5c9e-4a24-9e44-e1abc3f0f505/scratchpad/brain_fixer_r13_brief.md (classes from the promoted lines), new grant for the new head, gate again. If ALLOW: merge Brain to main (check origin/main ancestor first, merge origin/main into the branch if it moved, never rebase: the retro cites SHAs).
- Supporter gate on 2be114ff: BLOCK (30 promoted low-weight twins: lenient stored-id parse, retro/README claims). Log committed as .tasks/2026-11-16-supporter-slim/verdicts/round-10.log. Round-11 fixer stopped on quota after publishing 2871acb5, 7466fde5, bfefb444, 6e12ecbc; uncommitted placeholder-check change in support_publish.rs. Its step list: /home/nathanael/.worktrees/bots-supporter-land-20261117/.tasks/2026-11-16-supporter-slim/TODO.md. Brief: /tmp/claude-1000/-home-nathanael-Documents/71c000d8-5c9e-4a24-9e44-e1abc3f0f505/scratchpad/supporter_fixer_r11_brief.md. Next: fresh fixer finishes that TODO, then grant + gate.
- Bots branch: unchanged (c8862e04 + this TODO). After Brain merges: re-pin Brain to the Brain main commit, commit, grant, gate.
- Then: merge Bots and supporter, deploy Brain and dl-bot from origin/main, restart, live proof (Lachs replay, greenlight, unrelated chat silent, supporter live), cleanup.

## State 2026-11-18 15:12 CET (main session)
- Integrator 71c000d8 and delegate 466be772 both stopped on Claude quota; no claude-account relaunch job is queued for them. Claude account at 97% (5h, free from 17:00) and 98% (7d, free from 2026-11-24 21:00).
- Brain gate round 13 on d0e54069 still running (gate_hook pid 3379926, started ~14:27). Its verdict decides the next step listed in the 15:35 block above.
- Restart a single fresh integrator from this TODO once quota allows; do not start a second while one is alive.

## State 2026-11-18 (after compact, quota 7d 100%)
- Brain gate r13 on d0e54069: TOOL FAILURE, not a round (reviewer answers rejected for invalid JSON / bad caller refs, then "claude failed exit 2"). Grant invite-turn-20261118-r13 is spent. Log: /tmp/claude-1000/-home-nathanael-Documents/71c000d8-5c9e-4a24-9e44-e1abc3f0f505/scratchpad/brain_gate_r13.log
- The rejected answers already say BLOCK on low-weight items; hand these to a fresh round-14 fixer instead of a blind rerun:
  1. docs/public-retrieval-api.md:11: the 400 pointer reaches only retrieval.rs:56-62 and the 503 "ausgefallene Belegsuche" pointer misses retrieval.rs:185-189; GATE-RETRO.md:48 and a467376e claim every status is bound (correction line). Also doc line 5 overstates encoder coverage for /v1/task; 413/429/504 are stated in two places.
  2. diagnostics.rs: no test reaches the nested query.request_id branch of IdEnvelope (:43, :82-83) nor the Task branch of bind (:88-89, stable_request_id); add tests that can fail.
- Next: copy log to verdicts/round-13-toolfail.log, fresh Opus 5.5 high fixer with the two items, new grant for the new head, gate. Supporter and Bots steps unchanged from the block above.

## State 2026-11-18 17:02 CET (main session)
- Brain gate round 13 on d0e54069 ended without a verdict: the reviewer's answer was not valid JSON, the retry then failed (`review_gate: claude failed`, exit 2). Tool failure, not a counted round. A draft BLOCK in the log (scratchpad brain_gate_r13.log of session 71c000d8) names only low-weight doc defects in docs/public-retrieval-api.md: test pointers on line 11 that miss their status branch, an overstated /v1/task encoder claim on line 5, and 413/429/504 stated twice.
- Fresh integrator 466be772 relaunch failed on the rate limit. Claude account now at 7d 100% (5h 3%). claude-account will resume this session once there is quota again.
- Next: one fresh integrator from this TODO. Have its Brain fixer clear the three doc points above, self-review, re-grant the new head, rerun the Brain gate, then follow the 15:35 block (supporter r11 finish, Bots re-pin, merge, deploy, live proof).
