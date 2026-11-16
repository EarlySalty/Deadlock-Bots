# TODO: invite lounge

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence, head d75ea823. Nothing merged or deployed. Alp's row is `Attempted` with the notice already sent by main; it is not polled.

## State (2026-11-16)
- Rounds 1 to 7 ended BLOCK. Round 7 was the final round the user approved (grant `invite-lounge-20261116-r7` on d75ea823, spent). The gate says: no further round, decision goes to the user.
- Round 7 log: `gate-r7.log` next to this file (ignored by git).
- claude-config main d4676c0 carries the OFFENE-URSACHEN rows for the tempvoice text match and the broker 502 mapping, and marks the room-request and privacy rows as fixed on this branch.

## Round 7 real findings
- `assets/faq_texts.toml` f5 is not what members see: `faq_publish.rs:56` `FAQ_ANSWER_F5` still promises an automatic invite. For linked members "nein" does not send a friend request (they are already Steam friends).
- Hint: `hint_owed` matches the anchor id. A second code-less ask during an in-flight hint replaces the anchor, keeps `hint_sent` (update_state 2086), and the first job returns without posting or releasing, so no hint at all. `send_hint` `?` on `hint_owed` skips `release_hint` on a load error. A hinted request completed by a plain code or edit keeps `hint_sent`, so a later code-less ask in another channel gets no hint.
- `asked()`: a check-in that was posted but came back Retry stays queued, so a "nein" to the visible question is ignored.
- `unclaim_check_in` leaves `status.check_in_at`, so a rejected or given-up check-in is never asked again and the request stays inert until expiry.
- `takes_answers` equals `is_open`: dead conjuncts in `answer_open` and `accepts_repost`; WaitingForCode chat messages now each cost a Brain call; README:55, 30, 37 misstate the phases.
- `privacy_kv_key` duplicates `cooldown_key`; `OutboxEntry::earlier_targets` has no bound; `finish_dispatch` treats a row erased during dispatch as an error; `CheckIn::Skip` restates `close_handled`; modglue 404 check duplicates `is_http_404`.
- Tests: no test reaches the window half of `is_closed`; no test guards the `hint_owed` check before the Brain call.
- Twins named by the reviewer outside the branch: `scrimglue.rs:2929` `channel_already_missing`, `lfg_panel.rs:1066` `is_not_found_error`, `concierge.rs:2052` `discord_channel_already_gone`.

## Next
Needs a user decision: another approved round after one fix pass, or a narrower scope. No gate run without it.
