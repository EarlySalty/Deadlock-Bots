# TODO: invite lounge, stopped after the final gate round

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence, HEAD 856e7939 (pushed). Nothing merged or deployed. Alp's request is still in the live state.

## State

Rounds 1 to 4 ended BLOCK. After round 4 came the round-four review: the retro, the Fable opinion, one fixer pass (86ae2201) and an independent Opus review with follow-ups (c64783ea, 35e0a042, 856e7939).

The final round 5 also ended BLOCK: 40 blocking findings, of which 26 are [hook] restatements, plus 15 NITs. The log is in gate-r5.log, next to this file and gitignored. Under the rules no further round runs without a user decision.

## Remaining real findings (round 5)

1. **Friend-code rule.** One rule decides which message may set the stored code, and whether a later direct request without a code reuses it. Three paths still disagree:
   - The 856e7939 guard drops a corrected code that a member posts after an `Attempted` request, for example after the invalid-code notice. A later "Kannst du mich nochmal einladen?" then invites the stale invalid code. On main the corrected code was taken, so this is a regression.
   - `open_request_in` treats a `WaitingForCode` request of any age as open. A stray number completes a direct request that is weeks old and invites it. The behaviour predates the branch, but the branch claims it is fixed.
   - The `record_history` backfill (invite_lounge.rs:1797-1801) stores any number as `friend_code` when no request exists. This twin is not covered.
2. **Low findings:**
   - `Request::restart_at` still carries `direct`; the caller overwrites it.
   - The `note_repost` guard lives in the callers instead of the shared function.
   - `REQUEST_COPY_LIMIT = 5` has no derivation.
   - The min-id bound is computed twice, in `request_answered` and `answers_request`.
   - The README credits two tests for "needs check-in and nein", but they only assert no invite.
   - README list formatting.
3. **Hook:** commits 86ae2201 and c64783ea use the heading "Ursache (four layers):", but the hook expects `Ursache:`. Fixing that needs a message-only rewrite of the branch, the allowed lease-protected exception, with a tree-hash proof.

## Decision needed from the user

The friend-code rule. Recommendation: a stored code belongs only to its request. A new request without a code uses the linked Steam account (`known_code`) or asks for the code with the hint, and never reuses a code from an earlier request or from chat. A `WaitingForCode` request also expires with the request window. This one rule closes all three paths of finding 1.

After the decision, the next step needs the user's explicit approval for further gate rounds. Then: one fixer pass, then a gate round.
