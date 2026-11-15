# Invite lounge: silence never sends a friend request (2026-11-15)

## Trigger
Alp (kv `invite_lounge:requests/user:1037837117818142741`) posted a friend code in #deadlock-invite at 22:15 UTC on 2026-11-14 and deleted the message. The lounge posted the check-in at 22:45 UTC without a reply reference (the request was gone), sent a friend request by itself at 23:15 UTC and ignored "Jo habe ich schon !" at 23:16 UTC. The state stayed `Pending` with `wait: Delivery` and would have posted a "not accepted" notice about 24 h later.

## User decision (binding)
1. Silence never triggers a friend request. Request without staff reply, then the check-in. "No" sends the friend request, "yes" gets a short OK and closes. No answer: nothing more.
2. A deleted request message drops the request silently.
3. A late answer is read while the request is open, in every state.
4. Audit every lounge message path against the real channel history and fix the classes.

## Cause (four layers)
- Mechanism (reproduced): `is_due` treated a room request as due once `ROOM_WAIT_SECONDS` (60 min) passed, so `dispatch_due` sent the friend request after an unanswered check-in. `record_check_in_answer` only accepted answers while `check_in_open` held (phase `Pending`, no `wait`, before dispatch) and only replies or mentions to the bot (`replies_to_check_in`), so the answer one minute after dispatch was dropped. Nothing checked whether the request message still existed. Reproduced with the old tests `raumbitte_mit_code_wartet_genau_eine_stunde` (send at 3600 s) and Alp's state shape.
- Why possible: the 60 min automatic invite came from the first lounge design (86160190) and was kept when the check-in was added; the check-in was bolted on as an extra step, not as the gate. Answer acceptance was narrowed in fc903455/27817d35 to the check-in window and reply references to stop false positives, which also cut off late and plain answers. Deletion had no event path: the gateway only published new messages, so neither deletes nor edits reached the lounge.
- Why unnoticed: tests asserted the 60 min dispatch as correct behaviour, and every reply path was tested only with `reply_message_id` set. There is no metric for "member answered but the lounge ignored it"; the cases only show in the channel history.
- Twins: every decision on an open request. Fixed together: the dispatch gate (`is_due`), the check-in (`check_in_due`), the Steam follow-up (`follow_steam` via the wait branch), every outbox notice (`send_notice` checks the request message first), answer intake for all open phases (`answer_open`), and edits (new `message_update` path). Same "only new messages" gap for edits: friend codes edited into a request after the code hint.

## Audit of #deadlock-invite (Oct to 14 Nov 2026)
| Class | Examples | Fix |
|---|---|---|
| 1. Silence sent a friend request at 60 min, also to people humans had served | wwqiz 11-10, Alp 11-14; LotusRitter and PapayaExpress 11-01; Mucki 11-10 "already invited" after Nani | Room requests are due only after a "no" (`invite_now`) or when direct. Test `schweigen_nach_der_nachfrage_loest_nie_eine_einladung_aus` |
| 2. Plain answers without reply or mention ignored | grootie 11-14 "nein habe ich nicht" (Nani had to step in), Gladston 11-08, Sloth 11-09 | Any later message of the requester in the request channel is judged. Test `nachfrage_liest_auch_schlichte_folgenachrichten` |
| 3. Answers after dispatch ignored | Alp 11-14 | `answer_open` covers `Pending` (with or without Steam wait) and `Dispatching`; "yes" closes, purges queued notices, OK only if the bot asked. Test `spaetes_ja_schliesst_die_bitte_in_jedem_offenen_zustand` |
| 4. "Alles klar, kommt gleich." right before "FA ist raus" | INND8 11-10 (three bot messages in a minute) | "No" sets `invite_now` without its own reply; only the status notice goes out |
| 5. "Schau mal in deine Bibliothek" for unclear answers of people never invited | several after check-ins | Unclear answers get the library hint only when Steam shows an invite; otherwise silence |
| 6. Check-in and notices for deleted requests | Alp 11-14 | `request_gone` before check-in, dispatch and Steam follow-up; `send_notice` returns `Gone`; request and its notices are dropped. Tests `geloeschte_bitte_*`, `meldung_zu_geloeschter_bitte_wird_still_verworfen` |
| 7. Code edited into the request after the hint never seen, request stuck in `WaitingForCode` | Perry, Mental, Konsillis, OverTriXz, l0u (Oct), Toastbrotente 11-03, centipede 11-13 | Gateway `message_update` publishes `MessageEdit`; `record_edit` takes the code, request becomes `Pending`, check-in clock restarts at the edit. Test `bearbeitete_bitte_uebernimmt_den_nachgereichten_code` |
| 8. Judge prompt asks about an "Einladung zur Deadlock-Lounge" | prompt text | Brain package, see open points |

A Brain failure while judging an answer holds it (`held_answer`) and retries after 60 s instead of dropping it (test `antwort_ohne_brain_urteil_wird_spaeter_erneut_gewertet`). A Discord error on the existence check skips the step instead of dropping (test `nicht_pruefbare_bitte_setzt_den_schritt_nur_aus`). The check-in window is now 30 min to 6 h after the request.

## Alp's live state
Fixed by the code path: after deploy the next poll sees `wait: Delivery` with a past `check_at`, `request_gone` finds message 1571287459566981353 deleted (Discord 10008) and `drop_deleted` clears the request with log "Invite-Lounge verwirft gelöschte Bitte still". No manual DB change.

## Follow-up changes in this package
- Check-in in every lounge channel: it was limited to #deadlock-invite since e2cd593c (an implementation choice, no user rule). Without the 60 min invite, room requests in other channels such as #frag-die-community would have dead-ended; they now get the same question. Test `nachfrage_auch_ausserhalb_von_deadlock_invite`.
- Late "yes" while the invite call is in flight: `finish_dispatch` leaves the closed request alone, no "Invite ist raus" after the OK. Test `spaetes_ja_waehrend_des_versands_bleibt_geschlossen`.
- Judge prompt (Deadlock-Brain branch `fix/invite-judge-playtest-20261115`): asks about the Steam playtest invite or owning the game; an accepted friend request is `unclear`, so it cannot cancel the follow-up invite during the Steam wait. A non-verdict Brain reply counts as `unclear` (no retry loop); only Brain errors hold the answer.
- Gateway `message_update` mapping tested with a MESSAGE_UPDATE payload (`bearbeitete_nachricht_wird_mit_inhalt_weitergegeben`).

## Limits to know
- A request without an answer stays open for the request window (`REQUEST_WINDOW_SECONDS` = 7 days). A "no" from the requester in that channel within the window still sends the friend request; that is spec point 3.
- Edits only revive `WaitingForCode` requests inside that window.

## Dry run against live states (2026-11-15 00:20 UTC)
15 open states. Alp: `Pending`, `wait: Delivery`, next Steam check due, created after `actions_since` (1794031680), so the next poll drops it. 14 `WaitingForCode` states (13 older than 7 days, one 31 h in #deadlock-invite) stay inert; only an edit with a code inside the window revives one.

## Live check
- Journal of `deadlock-bot-rust.service` (user unit): "Invite-Lounge verwirft gelöschte Bitte still" for user 1037837117818142741.
- kv `invite_lounge:requests/user:1037837117818142741`: `request` absent, `last_result = "Bitte gelöscht; still beendet"`.
