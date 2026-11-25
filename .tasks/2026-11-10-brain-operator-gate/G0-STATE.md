# G0 state (2026-11-11 14:20)

- Worktree /home/nathanael/.worktrees/Deadlock-Bots-access-gate-20261110, branch feat/access-gate-20261110, clean, rebased on origin/main 11f721bd. HEAD 7c7a1ef5 (4 commits). Not pushed.
- Migration 20261110180000 is still the next free one.
- Verified on this HEAD: fmt clean; clippy shows no findings in touched files (remaining warnings are pre-existing in other crates); `test-db-run deadlock_test -- cargo-slot test -p dl-bot -p dl-access-gate -p dl-core -p dl-community` all green (dl-bot 395 passed, includes gate and public MCP tests).
- Acceptance on a live snapshot (roles, 3479 members, 132 channels, read only via /mcp api_call), temporary probe test, removed again:
  - access for Moderator, Community Moderator (holds Admin via Scrim Match Leiter), Team Coach (staff without Admin), base member, Streamer: server-wide bits, channel bits in one restricted channel, role ids and top position match an independent Python computation of Discord's algorithm (ALL_MATCH True).
  - authorize: base + discord.event.write missing_right; Streamer + discord.event.write allowed; CM ban of Moderator hierarchy; internal read in a public channel place_ok false; wrong token 401; unmapped allowed for Moderator with Admin, refused (unmapped) for staff without Admin; role removal: allowed, allowed (cached), missing_right after cache time.
  - Note: no live holder of Moderator or Community Moderator lacks Administrator, so "without Administrator" used a Team Coach.
- Gate: rounds 1-3 BLOCK (earlier session, ledger written). Round 4 attempted three times (01:05, 09:20, 14:20): each time no model judged, "5-hour limit". No verdict yet.

Next step: `gate-review --repo /home/nathanael/.worktrees/Deadlock-Bots-access-gate-20261110 --base origin/main` right after a quota reset; then continue with steps 1-5 of TODO.md (fix round or push HEAD:main, ancestor check, cleanup, report, settle).

## 2026-11-11 gate attempt 14:20 (round 4, fourth try)
- No verdict again: Fable returned a BLOCK, but its JSON was cut off at char 33786 and rejected; Opus and Fable then hit the 5-hour limit (reset 19:20).
- Findings visible in the partial BLOCK answer:
  1. Bearer parsing exists three times: gate.rs:254-261, mcp.rs:213-219 and mcp.rs:224-230.
  2. The freshness rule exists twice: gate.rs:188 and gate.rs:134.
  3. discord_call has hard-coded retry timings (mcp.rs:719, 724) inside the configurable lookup timeout; the retry margin is not derived from it.
  4. wire_ids_and_names_follow_the_record_checks (tests.rs:838-867) never inserts into bot.access_gate_decisions, so drift from the migration CHECKs (lines 48-55) stays invisible.
  5. "Three contract mismatches": the details were in the truncated part.
- Correction from the user: runs without a verdict are not rounds. Cap not reached; still at round 4. Re-run the gate after the reset.

## 2026-11-11 22:30 takeover (new session, old threads settled)
- Round 4 fixes from GATE-R4-FINDINGS.md committed by a fresh fixer; rebased on origin/main 92d9654c. HEAD e5a617a2 (6 commits). Tests green before rebase (dl-bot 399, dl-community 314, dl-core 66, dl-access-gate 4); the rebase only pulled a dl-voice change.
- Gate round 4 running, output /tmp/claude-1000/-home-nathanael-Documents/1e50ee49-c45f-4227-a664-d7b910e9ec33/scratchpad/gate-r4.txt (`exit=` at the end).
- Next: on ALLOW publish HEAD to main, ancestor check, cleanup, report to f958016c, settle. On BLOCK: round 4 was the last; report findings to the user.
