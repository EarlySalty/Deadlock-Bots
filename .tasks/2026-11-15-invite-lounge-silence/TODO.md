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

## User decision 2026-11-16
Relayed by the launching session, recorded as given: the user (owner) chose "A" = merge 7923f659 to main WITHOUT an ALLOW as the user's own exception, deploy, restart, live proof, park the open findings. No further gate round.

## Integration state 2026-11-16
- Checks on 7923f659 (merge tree equals branch tree): dl-community 389 passed, the same 4 DB-harness tests fail without `CENTRAL_TEST_DSN`; dl-discord 71 + 3 passed; dl-bot FAQ 13 passed, `service_faq_v2_repost_loescht_fremde_message_nicht` needs `CENTRAL_TEST_DSN`; `cargo fmt --check` reports no file changed by this branch; clippy reports only pre-existing warnings.
- Deploy blocked: live `/opt/deadlock/bots/current` runs 898df907 from the unmerged `feat/supporter-p1-slim-20261116` (deployed on user order before its gate ALLOW, migrations 20261110120000 to 20261115120000 applied), and that branch's prepared installer pins 898df907 as the previous release. A release from main would remove the supporter code from live. The supporter branch conflicts with this branch in `invite_lounge.rs`, `invite_lounge_tests.rs` and `dl-bridges/src/steam.rs`. Needs a user decision: deploy main anyway, let the supporter branch rebase onto main and carry this fix in its next release, or order a combined build.
- Open findings of round 8 are parked in claude-config `orchestrierung/OFFENE-URSACHEN.md`.
