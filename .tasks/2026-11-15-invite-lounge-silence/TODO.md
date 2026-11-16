# TODO: invite lounge

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence. Nothing merged or deployed. Alp's row is `Attempted` with the notice already sent by main; it is not polled.

## State (2026-11-16)
- Rounds 1 to 6 ended BLOCK. The user decided the friend-code rule ("FIX IT") and approved gate rounds 6 and 7 with one fix between them; round 7 is the last.
- Round 6 ran on c43470fa (fix pass 7); its log is `gate-r6.log` next to this file. The earlier run on 49a80e2f was lost without a verdict and is kept as `gate-r6-lost.log`. Both logs are ignored by git.
- Fix pass 8 answers round 6 (GATE-RETRO section 7). The fix head is the commit that carries this TODO version.
- Each extra round needs a per-head grant in `gate_hook.py` `SUPPORTER_REVIEW_AUTHORIZATIONS`: `invite-lounge-20261116-r6` (spent on c43470fa) and `invite-lounge-20261116-r7` for the fix pass 8 head.

## Next
1. Gate round 7 on the fix pass 8 head, log `gate-r7.log`. On BLOCK: stop and report the findings to the user; no further round.
2. On ALLOW: update main with HEAD, deploy dl-bot, restart `deadlock-bot-rust.service`, live proof (README "Live check"), ledger retro, remove branch and worktree.
