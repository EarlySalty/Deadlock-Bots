# TODO: invite lounge

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence. Nothing merged or deployed. Alp's row is `Attempted` with the notice already sent by main; it is not polled.

## State (2026-11-16)
- Rounds 1 to 7 ended BLOCK. User decision after round 7: one fix pass on five items, one round 8 (grant `invite-lounge-20261116-r8`), the lows as OFFENE-URSACHEN rows (claude-config ceedc6c).
- Fix pass 9 is on the branch (GATE-RETRO section 8). Round 8 log: `gate-r8.log` next to this file (ignored by git).

## Next
- Round 8 ALLOW: merge, deploy dl-bot, restart `deadlock-bot-rust.service`, live proof (service active, journal clean, Alp's row unchanged, no new message to Alp, FAQ f5 text), ledger retro, cleanup.
- Round 8 BLOCK: stop and report to the user; no further round without a decision.
