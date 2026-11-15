# TODO: invite lounge, final gate round (round 5)

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence

## State

- Rounds 1 to 4 ended BLOCK. The round-four review is done: GATE-RETRO.md, the Fable second opinion, one clean fixer pass (86ae2201 and follow-ups after an independent Opus review), and gate-learning ledger lines for round 4.
- claude-config 6b25202 added the privacy row (eraser misses `invite_lounge:*`) to OFFENE-URSACHEN. Worktree /home/nathanael/.worktrees/cc-lounge-ursachen-20261115 stays for the row correction after the merge.
- Only one final gate round remains. A run without a verdict is not a round; rerun it with the same command:
  `(gate-review --repo /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115 --base origin/main; echo "EXIT=$?") > <scratchpad>/gate-r5.log 2>&1`

## On ALLOW
1. `git -C /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115 push origin HEAD:main`.
2. Deploy dl-bot through the deploy wrapper (memory dl-bot-build-und-deploy) and restart the user unit `deadlock-bot-rust.service`. The broker runs inside dl-bot, so the `fail_if_not_exists` fix goes live with it.
3. Live proof:
   - The journal shows "Invite-Lounge verwirft gelöschte Bitte still" for user 1037837117818142741.
   - The redacted kv row `invite_lounge:requests/user:1037837117818142741` shows `request` absent.
4. In claude-config, correct the OFFENE-URSACHEN Lagged row (8c92b9b) to the merge SHA. Write the ledger retro line. Delete the branch, the remote branch and both worktrees.

## On BLOCK
Stop. Report the retro, the remaining findings and the decision needed to the user. Do not start another round.
