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
- Why possible: the 60 min automatic invite came from a1843447 (2026-10-05, the first automatic lounge invites) and was kept when the check-in was added in 86160190 (2026-11-06); the check-in was bolted on as an extra step, not as the gate. Answer acceptance was narrowed in fc903455/27817d35 to the check-in window and reply references to stop false positives, which also cut off late and plain answers. Deletion had no event path: the gateway only published new messages, so neither deletes nor edits reached the lounge.
- Why unnoticed: tests asserted the 60 min dispatch as correct behaviour, and `nachfrage_nimmt_nur_antworten_mit_bezug_zum_bot_an` asserted that a plain "ja" without reply reference is ignored, so dropping plain answers was codified as intended. There is no metric for "member answered but the lounge ignored it"; the cases only show in the channel history.
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
| 8. Judge prompt asks about an "Einladung zur Deadlock-Lounge" | prompt text | Deadlock-Brain branch `fix/invite-judge-playtest-20261115`, see follow-up changes |

### Answer queue
- Answers are queued (`held_answers`) and judged in the dispatch task, so the Brain verdict for an answer never runs on the gateway loop. The code hint for a new request still runs inline in `record_message`, as before this change.
- Queue limit 5: a member answers the check-in with one to three short lines in the channel history; five covers a burst while the Brain is down, keeps the first answer at the head and bounds the state row. A sixth answer logs "Invite-Lounge verwirft Antwort, weil schon zu viele Antworten auf ein Brain-Urteil warten" (test `volle_antwortschlange_nimmt_keine_weitere_antwort_auf`).
- Before the Brain call the head answer is leased by compare-and-swap. The lease is twice the configured Brain timeout (`runtime.ai.brain_api_timeout_ms`, live 120 s, so 240 s): the Brain call plus the Discord and DB calls around it end inside it, so a tick and a message handler never judge the same answer twice (tests `geliehene_antwort_wird_nicht_doppelt_gewertet`, `leihfrist_deckt_das_doppelte_brain_zeitlimit`). Notice claims use the same lease; before, both used a fixed 120 s, equal to the live Brain timeout.
- A Brain failure keeps the answer and retries with `brain_backoff`: 1 min doubling, capped at 60 min, the same schedule as notices, since both wait on the same Brain. There is no attempt cap; the answer stays until the request window ends or the request closes (tests `antwort_ohne_brain_urteil_wird_spaeter_erneut_gewertet`, `brain_fehler_bei_der_antwort_verbraucht_sie_nicht`). A later message neither replaces a queued answer nor resets its backoff (test `gehaltene_antwort_geht_bei_einer_folgenachricht_nicht_verloren`).
- Users with queued answers are polled in every phase (test `gehaltene_antwort_im_versand_wird_beim_naechsten_takt_gewertet`, Postgres test `gehaltene_antwort_im_versand_zaehlt_als_offen`).
- An answer that repeats the stored friend code is judged as an answer (test `nein_mit_dem_freundescode_loest_die_einladung_aus`). A message phrased as a room request counts as an answer when it carries no code ("nein, kann mich jemand einladen?", test `nein_in_bittenform_wird_als_antwort_gewertet`) and as a repost when it carries a code.
- Logged drops on an open request: "Invite-Lounge wertet Nachricht des Mitglieds nicht als Antwort" with the reason ("Bitte zu oder anderer Kanal", "ohne Text"), "Invite-Lounge verwirft wartende Antwort, die Bitte ist nicht mehr offen" (head of the queue after the request closed, expired or was handled), "Invite-Lounge verwirft wartende Antworten" (request replaced), "Invite-Lounge verwirft wartende Antworten der gelöschten Bitte", "Invite-Lounge verwirft wartende Antworten, das Mitglied hat die Einladung schon" and the queue overflow line above. Closing steps (`close_handled`, `expire`, `finish_dispatch`) leave the queue in place, so its entries reach the "nicht mehr offen" line on the next poll.

### Existence check, reposts, edits, missed messages, hand-served requests
- Any Discord 404 (message or channel) counts as gone, one helper `is_http_404`.
- A Discord error on the existence check never drops anything. Before a friend request or a Steam follow-up the step is held 5 min (`RETRY_SECONDS`, the existing Steam retry interval) with "Invite-Lounge hält die Einladung zurück, bis die Bitte prüfbar ist", so no friend request goes out for a request that may be deleted (test `nicht_pruefbare_bitte_haelt_die_einladung_zurueck`). The check-in goes on, and its notice waits in the outbox until Discord answers (test `nicht_pruefbare_bitte_haelt_die_nachfrage_bis_discord_antwortet`).
- Deleting a room request and posting the same code again moves the `Pending` request and its queued notices to the new message; the check-in clock is kept (tests `neu_gepostete_bitte_behaelt_die_nachfrage`, `gleicher_code_schiebt_die_nachfrage_nicht_auf`).
- Catch-up on start, after `Lagged` on the message or the edit stream, and after a failed catch-up: every `WaitingForCode` request inside the window is read again from Discord (test `abgleich_findet_waehrend_der_pause_nachgereichten_code`), and for every open request the channel history after the request and the last seen message is read and the requester's messages go through the normal path (test `abgleich_liest_verpasste_antwort_nach`). A single user's failure is logged and skips only that user.
- A request a human served after the check-in sends nothing on a later "no" (test `nein_nach_einladung_von_hand_sendet_keine_anfrage`).
- The check-in fires any time from 30 min after the request while the request window is open; there is no upper cutoff, so downtime or a long Brain outage cannot skip it.

### Stored state format
Old rows still carry the removed field `check_in_message_id`. The text compare-and-swap would never match them, so `load` rewrites a row whose canonical form differs, guarded by the stored text, and logs "Invite-Lounge schreibt Zustand im aktuellen Format neu" (Postgres test `altzeile_wird_beim_laden_umgeschrieben_und_bleibt_schreibbar`). Eight live rows had the field on 2026-11-15. The serialized state has no maps, so its canonical text is stable.

## Alp's live state
Fixed by the code path: after deploy the next poll sees `wait: Delivery` with a past `check_at`, `request_gone` finds message 1571287459566981353 deleted (Discord 10008) and `drop_deleted` clears the request with log "Invite-Lounge verwirft gelöschte Bitte still". No manual DB change.

## Follow-up changes in this package
- Check-in in every lounge channel: it was limited to #deadlock-invite since e2cd593c (an implementation choice, no user rule). Without the 60 min invite, room requests in other channels such as #frag-die-community would have dead-ended; they now get the same question. Test `nachfrage_auch_ausserhalb_von_deadlock_invite`.
- Late "yes" while the invite call is in flight: `finish_dispatch` leaves the closed request alone, no "Invite ist raus" after the OK. Test `spaetes_ja_waehrend_des_versands_bleibt_geschlossen`.
- Judge prompt (Deadlock-Brain branch `fix/invite-judge-playtest-20261115`): asks about the Steam playtest invite or owning the game; an accepted friend request is `unclear`, so it cannot cancel the follow-up invite during the Steam wait. A non-verdict Brain reply counts as `unclear` (no retry loop); only Brain errors hold the answer.
- Gateway `message_update` mapping tested with a MESSAGE_UPDATE payload (`bearbeitete_nachricht_wird_mit_inhalt_weitergegeben`).

## Limits to know
- A request without an answer stays open for the request window (`REQUEST_WINDOW_SECONDS` = 7 days). Answers are read in every open phase (`Pending` with or without Steam wait, `Dispatching`) inside that window; a stale `Dispatching` request outside it takes no answers (test `veraltete_bitte_im_versand_nimmt_keine_antwort_mehr_an`).
- A "no" sends the friend request only after the check-in was asked and before any invite attempt, as the user decision says. A "no" written before the check-in is read but does not invite; the check-in still comes at 30 min and its answer decides. A "yes" closes the request in every open phase.
- Edits only revive `WaitingForCode` requests inside that window.
- A same-code repost while the invite call is in flight (`Dispatching`, seconds) is not retargeted, because `finish_dispatch` matches the request by message id. A deleted and reposted direct request starts a new direct request, as on main: an explicit ask is answered again.
- The catch-up covers requests the lounge already knows. A new request posted while the message stream lagged is not recovered; that gap predates this change (a1843447) and is tracked in `claude-config/orchestrierung/OFFENE-URSACHEN.md`.

## Dry run against live states (2026-11-15 00:20 UTC)
15 open states. Alp: `Pending`, `wait: Delivery`, next Steam check due, created after `actions_since` (1794031680), so the next poll drops it. 14 `WaitingForCode` states (13 older than 7 days, one 31 h in #deadlock-invite) stay inert; only an edit with a code inside the window revives one.

## Live check
- Journal of `deadlock-bot-rust.service` (user unit): "Invite-Lounge verwirft gelöschte Bitte still" for user 1037837117818142741.
- kv `invite_lounge:requests/user:1037837117818142741`: `request` absent, `last_result = "Bitte gelöscht; still beendet"`.
