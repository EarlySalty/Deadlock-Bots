# TODO: invite lounge

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence. Nothing merged or deployed. Alp's row is `Attempted` with the notice already sent by main; it is not polled.

## State (2026-11-16)
- Rounds 1 to 5 ended BLOCK. The user decided the friend-code rule ("FIX IT") and approved gate rounds 6 and 7 with one fix between them.
- Fix pass 6 implements the decision (README "Friend-code rule", GATE-RETRO section 5). Commits 86ae2201 and c64783ea were rewritten message-only to the `Ursache:` heading with tree-hash proof (GATE-RETRO section 5).
- Each extra round needs a per-head grant in `gate_hook.py` `SUPPORTER_REVIEW_AUTHORIZATIONS`.

## Next
1. Gate round 6 on the fix head. On BLOCK: one fresh fix, a new grant, round 7. If round 7 blocks, stop and report.
2. On ALLOW: update main with HEAD, deploy dl-bot, restart `deadlock-bot-rust.service`, live proof (README "Live check"), OFFENE-URSACHEN row, ledger retro, remove branch and worktree.
