# TODO: invite lounge

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence. Nothing merged or deployed. Alp's row is `Attempted` with the notice already sent by main; it is not polled.

## State (2026-11-16)
- Rounds 1 to 5 ended BLOCK. The user decided the friend-code rule ("FIX IT") and approved gate rounds 6 and 7 with one fix between them.
- Fix pass 6 implements the decision (README "Friend-code rule", GATE-RETRO section 5). Commits 86ae2201 and c64783ea were rewritten message-only to the `Ursache:` heading with tree-hash proof (GATE-RETRO section 5).
- Each extra round needs a per-head grant in `gate_hook.py` `SUPPORTER_REVIEW_AUTHORIZATIONS`.

## Quota stop (2026-11-16, ~01:30)
- Head 49a80e2f (fix pass 6) is on the remote branch. Grant `invite-lounge-20261116-r6` sits in `/home/nathanael/Documents/.claude/gpt-workers/gate_hook.py` (tuple plus source note, nothing else changed in that file).
- Gate round 6 started at 00:59 in the background, output in `gate-r6.log` next to this file; no verdict yet when quota ran out. If the log holds a verdict, act on it. If it is empty and no gate process runs, the run was lost (not a round): restart the same `gate-review` command.
- Result of that run: no verdict ("kein Modell der Kette hat geurteilt"); the Opus reviewer stopped on quota. Not a round; the grant file `invite-lounge-20261116-r6.json` was not written, so the grant is unspent. Next: restart the same `gate-review` on 49a80e2f. The reviewer's partial notes in `gate-r6.log` (no findings yet) list its open checks: callers of each unit, `INVITE_FORWARD_TIMEOUT_SECS` vs `ACTIVE_DISPATCH_MILLIS` derivation, stuck `Dispatching` after a crash with `answer_open` taking answers there, all `friend_code` writers and README claims. It also reports that `graphify-first` blocks `git grep` inside the gate sandbox.
- This TODO edit is uncommitted; commit it with the next fix or drop it before the merge.

## Fix pass 7 (2026-11-16)
- The lost run's leads were checked and fixed (GATE-RETRO section 6). The lost log is kept (ignored by git) as `gate-r6-lost.log`; `gate-r6.log` is the real round 6.
- Grant `invite-lounge-20261116-r6` re-pointed to the fix pass 7 head.

## Next
1. Gate round 6 on the fix head. On BLOCK: one fresh fix, a new grant, round 7. If round 7 blocks, stop and report.
2. On ALLOW: update main with HEAD, deploy dl-bot, restart `deadlock-bot-rust.service`, live proof (README "Live check"), OFFENE-URSACHEN row, ledger retro, remove branch and worktree.
