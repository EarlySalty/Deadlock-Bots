# TODO: invite lounge

Worktree: /home/nathanael/.worktrees/bots-invite-lounge-silence-20261115
Branch: fix/invite-lounge-silence. Nothing merged or deployed. Alp's row is `Attempted` with the notice already sent by main; it is not polled.

## State (2026-11-16)
- Rounds 1 to 8 ended BLOCK. Round 8 (grant `invite-lounge-20261116-r8` on 9af9c03f, spent) was the single round the user approved after round 7. No further round without a user decision.
- Round 8 log: `gate-r8.log` next to this file (ignored by git). 214 BLOCKING lines: 94 hook lines (mostly the parked twins: scrimglue 2929, concierge 2052, lfg_panel 1066, tempvoice 672, privacy_kv_key, CheckIn::Skip), 2 model highs, 118 model lows.

## Round 8 real findings
- High: a "nein" written after the check-in claim but before its post landed can still invite. Intake accepts on `may_be_asked`, and `apply_answer` reads `asked` at store time; nothing compares the answer id with the check-in reply id (`notice_landed` drops it).
- High: no lounge writer checks the privacy opt-out tombstone (`core.user_privacy`), while held answers store every requester line and send it to the Brain.
- `end_unasked_request` on the Retry give-up arm is unreachable (the request leaves the 7-day window first); the test builds an impossible state.
- `send_hint` releases after a reply that may have landed (same "posted but timed out" class as the check-in).
- `next_hint_claim` returns `now` without a previous stamp, so a fresh row can share a stamp with an in-flight job.
- `check_in` returns a `steam_progress` error without backoff, so it reruns every 15 s for up to 7 days.
- `confirm_check_in` pages history back to the oldest target inside the judgement cutoff.
- FAQ f5 promises the 30 min wait for every ask, but direct asks get the invite at once; a third f5 copy in `docs/onboarding-redesign/2026-07-05-server-faq-v2-texte.md` still says the bot invites automatically.
- README: "finish_dispatch is the only closer that can leave answers held" (end_unasked_request is another), cooldown and stamp sentences, privacy retention window.

## Next
User decision needed: another round with a fix pass (the parked twins block through the hook whatever the scope), or another plan.
