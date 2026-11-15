# Invite lounge: silence never sends a friend request (2026-11-15)

## Trigger
Alp (kv `invite_lounge:requests/user:1037837117818142741`) posted a friend code in #deadlock-invite at 22:15 UTC on 2026-11-14 and then deleted the message. At 22:45 UTC the lounge posted the check-in without a reply reference, because the request was gone. At 23:15 UTC it sent a friend request on its own, and it ignored "Jo habe ich schon !" at 23:16 UTC. The state stayed `Pending` with `wait: Delivery`, so a "not accepted" notice would have gone out about 24 h later.

## User decision (binding)
1. Silence never triggers a friend request. Flow: the request, no staff reply, then the check-in. "No" sends the friend request. "Yes" gets a short OK and closes. No answer means nothing further happens.
2. A deleted request message drops the request silently.
3. A late answer is read as long as the request is open, whatever its state.
4. Audit all lounge message paths against the real channel history and fix each class of failure.

## Cause (four layers)
- **Mechanism (reproduced):**
  - `is_due` treated a room request as due once `ROOM_WAIT_SECONDS` (60 min) had passed. So `dispatch_due` sent the friend request after an unanswered check-in.
  - `record_check_in_answer` accepted answers only while `check_in_open` held (phase `Pending`, no `wait`, before dispatch), and only replies to or mentions of the bot (`replies_to_check_in`). The answer one minute after dispatch was therefore dropped.
  - Nothing checked whether the request message still existed.
  - Reproduced with the old test `raumbitte_mit_code_wartet_genau_eine_stunde` (sends at 3600 s) and Alp's state shape.
- **Why possible:**
  - The 60 min automatic invite came from a1843447 (2026-10-05, the first automatic lounge invites). It was kept when 86160190 (2026-11-06) added the check-in. The check-in was bolted on as an extra step instead of acting as the gate.
  - fc903455 and 27817d35 narrowed answer acceptance to the check-in window and to reply references, to stop false positives. That also cut off late answers and plain answers.
  - Deletion had no event path. The gateway published only new messages, so neither deletes nor edits reached the lounge.
- **Why unnoticed:**
  - Tests asserted the 60 min dispatch as correct behaviour.
  - `nachfrage_nimmt_nur_antworten_mit_bezug_zum_bot_an` asserted that a plain "ja" without a reply reference is ignored. Dropping plain answers was thereby codified as intended.
  - No metric covers "member answered but the lounge ignored it". These cases show up only in the channel history.
- **Twins, fixed together:**
  - The dispatch gate (`is_due`), the check-in (`check_in_due`) and the Steam follow-up (`follow_steam`, via the wait branch of `dispatch_request`).
  - The outbox notices: `send_notice` checks the request message first.
  - Answer intake in `Pending` and `Dispatching` (`answer_open`).
  - Edits, through the new `message_update` path. They had the same "new messages only" gap: a friend code edited into a request after the code hint was never seen.

## Audit of #deadlock-invite (Oct to 14 Nov 2026)
| Class | Examples | Fix |
|---|---|---|
| 1. Silence sent a friend request at 60 min, also to people humans had already served | wwqiz 11-10, Alp 11-14; LotusRitter and PapayaExpress 11-01; Mucki 11-10 "already invited" after Nani | Room requests become due only after a "no" (`invite_now`) or when the request is direct. Test `schweigen_nach_der_nachfrage_loest_nie_eine_einladung_aus` |
| 2. Plain answers without reply or mention were ignored | grootie 11-14 "nein habe ich nicht" (Nani had to step in), Gladston 11-08, Sloth 11-09 | A later message from the requester in the request channel is judged. Test `nachfrage_liest_auch_schlichte_folgenachrichten` |
| 3. Answers after dispatch were ignored | Alp 11-14 | `answer_open` covers `Pending` (with or without Steam wait) and `Dispatching`. "Yes" closes the request and purges queued notices; the OK goes out only if the bot asked. Test `spaetes_ja_schliesst_die_bitte_in_jedem_offenen_zustand` |
| 4. "Alles klar, kommt gleich." right before "FA ist raus" | INND8 11-10 (three bot messages in one minute) | "No" sets `invite_now` without a reply of its own; only the status notice goes out |
| 5. "Schau mal in deine Bibliothek" sent on unclear answers from people who were never invited | several, after check-ins | Unclear answers get the library hint only when Steam shows an invite; otherwise silence |
| 6. Check-in and notices for deleted requests | Alp 11-14 | `check_request` (result `RequestCheck`) runs before the check-in, the dispatch and the Steam follow-up, and `send_notice` checks the request before each notice. Any Discord 404 counts as gone. `on_gone` then decides. Tests `geloeschte_bitte_*`, `geloeschte_direkte_bitte_bekommt_keine_einladung`, `meldung_zu_geloeschter_bitte_wird_still_verworfen` |
| 7. A code edited into the request after the hint was never seen; the request stuck in `WaitingForCode` | Perry, Mental, Konsillis, OverTriXz, l0u (Oct), Toastbrotente 11-03, centipede 11-13 | The gateway `message_update` publishes `MessageEdit`. `record_edit` takes the code, the request becomes `Pending`, and the check-in clock restarts at the edit. Test `bearbeitete_bitte_uebernimmt_den_nachgereichten_code` |
| 8. The judge prompt asked about an "Einladung zur Deadlock-Lounge" | prompt text | Deadlock-Brain branch `fix/invite-judge-playtest-20261115`, see "Follow-up changes" |

### Answer intake
- **Where messages go.** A requester message that is not a direct bot request goes to `record_answer`, which loads the state and decides:
  - A room-phrased message with the stored code, posted before the check-in was asked, is not an answer. It is treated as a repost (see "Reposts and deleted requests").
  - After the check-in was asked, the same message is an answer. "Nein, kann mich jemand einladen? 123456789" sends the friend request (test `nein_in_bittenform_mit_gleichem_code_loest_die_einladung_aus`). It is also recorded as a repost in the same compare-and-swap.
  - A room-phrased message without a code is an answer (test `nein_in_bittenform_wird_als_antwort_gewertet`).
  - An answer that repeats the stored code is judged as an answer (test `nein_mit_dem_freundescode_loest_die_einladung_aus`).
- **New friend code.** A message with a new friend code counts as an answer only if all three hold: the check-in was asked, the invite has not started (`Pending`, no Steam wait, no dispatch since the request), and the answer queue has room (`takes_answer_code`).
  - The answer and the new code are then stored in one compare-and-swap, logged as "Invite-Lounge übernimmt neuen Freundescode aus der Antwort". "nein, mein Code ist 987654321" sends the friend request to 987654321 (test `neuer_code_in_der_antwort_geht_an_den_neuen_code`).
  - Otherwise the message is logged with reason "neuer Freundescode" and restarts the request with the new code, as on main. Tests: `neuer_code_vor_der_nachfrage_startet_die_bitte_neu`, `neuer_code_nach_der_einladung_startet_die_bitte_neu`, `neuer_code_in_der_antwort_braucht_platz_in_der_schlange`.
- **Queue.** Answers are queued in `held_answers` and judged in the dispatch task, never on the gateway loop.
  - The limit is 5. A member answers the check-in with one to three short lines in the channel history. Five covers a burst while the Brain is down, keeps the first answer at the head and bounds the state row.
  - A sixth answer logs "Invite-Lounge verwirft Antwort, weil schon zu viele Antworten auf ein Brain-Urteil warten" (test `volle_antwortschlange_nimmt_keine_weitere_antwort_auf`).
- **Leases.** Before the Brain call, the head answer is leased by compare-and-swap.
  - The leased work runs under a cut-off of 2 x the configured Brain timeout, enforced by `tokio::time::timeout` (`claim_cutoff`). For answers that work is `judge_answer` plus `already_invited` in `settle_answers`; for notices it is `send_notice` in `deliver_outbox`.
  - The lease is 2 x the cut-off, with no floor (`claim_lease_millis`). Live, `runtime.ai.brain_api_timeout_ms` is 120 s, so the cut-off is 240 s and the lease is 480 s (test `frist_und_leihfrist_folgen_dem_brain_zeitlimit`).
  - A tick and a message handler therefore never judge the same answer twice (test `geliehene_antwort_wird_nicht_doppelt_gewertet`).
  - A timeout counts as a failed attempt and is logged as "Invite-Lounge bricht das Brain-Urteil nach der Frist ab" or "Invite-Lounge bricht die Meldung nach der Frist ab". Tests: `haengendes_brain_urteil_endet_nach_der_frist_und_bleibt_gehalten`, `haengende_meldung_endet_nach_der_frist_und_bleibt_in_der_ablage`.
  - Before this branch, only notice claims had a lease, a fixed 120 s; held answers had none.
- **Brain failures.** A Brain failure keeps the answer and retries with `brain_backoff`: 1 min, doubling, capped at 60 min. This is the same schedule as notices, since both wait on the same Brain.
  - There is no attempt cap. The answer stays until the request window ends or the request closes (tests `antwort_ohne_brain_urteil_wird_spaeter_erneut_gewertet`, `brain_fehler_bei_der_antwort_verbraucht_sie_nicht`).
  - A later message neither replaces a queued answer nor resets its backoff (test `gehaltene_antwort_geht_bei_einer_folgenachricht_nicht_verloren`).
- **Polling.** `pending_users` selects users with a `Pending` request or with queued answers, whatever the phase. Tests: `gehaltene_antwort_im_versand_wird_beim_naechsten_takt_gewertet`, and the Postgres test `gehaltene_antwort_im_versand_zaehlt_als_offen`.
- **Logged drops on an open request:**
  - "Invite-Lounge wertet Nachricht des Mitglieds nicht als Antwort", with one of the reasons "Bitte zu oder anderer Kanal", "neuer Freundescode" or "ohne Text".
  - "Invite-Lounge verwirft wartende Antwort, die Bitte ist nicht mehr offen" (the head of the queue after the request closed, expired or was handled).
  - "Invite-Lounge verwirft wartende Antworten" (request replaced).
  - "Invite-Lounge verwirft wartende Antworten der gelöschten Bitte".
  - "Invite-Lounge verwirft wartende Antworten, das Mitglied hat die Einladung schon".
  - The queue overflow line above.
  - The closing steps (`close_handled`, `expire`, `finish_dispatch`) leave the queue in place, so its entries reach the "nicht mehr offen" line on the next poll.

### Reposts and deleted requests
- **A repost does not move the request.** A same-code room message on a `Pending` or `Dispatching` room request is recorded as `repost` next to the request. The anchor (`request.message_id`) stays, so staff replies to the original keep counting for `request_answered` and `someone_on_it` (test `doppelte_bitte_behaelt_den_anker_und_hilfe_von_hand_zaehlt`). `repost` is skipped when empty, so rows without it keep their exact stored text.
- **`on_gone` decides when the anchor is gone.** When an existence check finds the anchor gone, `on_gone(state, checked_message_id)` decides. It is used at three sites: the two `check_request` Gone arms in `dispatch_request` (through `drop_deleted`) and the Gone result in `settle_notice`.
  - The checked id is no longer the anchor: the request stays as it is.
  - A repost is recorded: the anchor and its queued notices move to the repost, and `created_at` and the check-in clock stay. Logged as "Invite-Lounge folgt der neu geposteten Bitte". Tests: `neu_gepostete_bitte_behaelt_die_nachfrage`, `gleicher_code_schiebt_die_nachfrage_nicht_auf`.
  - No repost: the request is forgotten silently, logged as "Invite-Lounge verwirft gelöschte Bitte still".
- **Notices decide on the id they checked.** `settle_notice` uses the message id that the notice checked, taken from the claimed entry, not the reloaded one (test `meldung_zur_geloeschten_bitte_folgt_der_neu_geposteten`).
- **Held answers stay open.** A held answer stays open while the request is open, even after the anchor moved past it. So a "nein" held during a Brain error is still judged after the original was deleted and reposted, and the friend request goes out (test `gehaltenes_nein_folgt_der_neu_geposteten_bitte`).
- **Discord errors never drop anything.**
  - Before a friend request or a Steam follow-up, the step is held for 5 min (`RETRY_SECONDS`, the existing Steam retry interval), logged as "Invite-Lounge hält die Einladung zurück, bis die Bitte prüfbar ist". No friend request goes out for a request that may be deleted (test `nicht_pruefbare_bitte_haelt_die_einladung_zurueck`).
  - The check-in goes ahead, and its notice waits in the outbox until Discord answers (test `nicht_pruefbare_bitte_haelt_die_nachfrage_bis_discord_antwortet`).
- **The code hint** (`send_brain_hint`) runs in the dispatch task, not on the gateway loop.
  - A failed reply or a Brain error releases the hint claim (test `fehlgeschlagene_hinweis_antwort_gibt_den_hinweis_frei`).
  - The reply sets `fail_if_not_exists: true`, so a hint never lands without its request.
  - The claim (`hint_sent`) is set before the reply, as on main. A crash between the claim and the reply loses that one hint; this is unchanged main behaviour.
  - Looking up an existing hint in the channel history (`has_hint`) still runs inline in `record_message`.

### Edits and missed messages
- **Edits are re-read on an interval.** Every 5 min (`RETRY_SECONDS`) after the history backfill, one task re-reads the request message of each `WaitingForCode` request inside the window, one Discord GET per request (`reread_waiting_requests`, test `abgleich_findet_waehrend_der_pause_nachgereichten_code`). An error for one user is logged and skipped, and the next interval retries.
- **Lost messages are not recovered.** Messages and answers lost to downtime or to a `Lagged` gateway stream are not recovered, as on main; both `Lagged` arms only log. A lost answer behaves like silence: nothing is sent.
- **Hand-served requests.** A request a human served after the check-in sends nothing on a later "no" (test `nein_nach_einladung_von_hand_sendet_keine_anfrage`).
- **No upper cutoff for the check-in.** It fires any time from 30 min after the request while the request window is open, so downtime or a long Brain outage cannot skip it.
- **Active dispatch window.** A direct request that arrives while an invite call runs does not replace it for 2 x `INVITE_FORWARD_TIMEOUT_SECS` (240 s), twice the Steam bridge timeout (test `laufender_versand_bleibt_ueber_das_steam_zeitlimit_hinaus_stehen`).

### Stored state format
Old rows still carry the removed field `check_in_message_id`. The text compare-and-swap would never match them. So `load` rewrites a row whose canonical form differs, guarded by the stored text, and logs "Invite-Lounge schreibt Zustand im aktuellen Format neu". `normalize` and `compare_exchange` share one text compare-and-swap (`swap_text`). Postgres test: `altzeile_wird_beim_laden_umgeschrieben_und_bleibt_schreibbar`. Eight live rows had the field on 2026-11-15. The serialized state has no maps, so its canonical text is stable.

## Deviations from the round-3 decisions
1. **Reposts after the check-in are recorded too.** D3 recorded reposts only before the check-in. After the check-in, a same-code room message is both an answer (D1) and a recorded repost, in one compare-and-swap. Without that, a deleted and reposted request after the check-in would be forgotten and a held "nein" would never invite (test `gehaltenes_nein_folgt_der_neu_geposteten_bitte`).
2. **A new code in an answer is taken only on an untouched invite with queue room.** Otherwise the message restarts the request with the new code, as on main. Changing the code under a running Steam wait or dispatch would track progress against a code that was never invited.
3. **A gone request with a repost waits while the invite call runs.** During `Dispatching`, the anchor is not moved, because `finish_dispatch` matches the request by message id. The notice is rescheduled instead (test `geloeschte_bitte_im_versand_wartet_auf_den_abschluss`).
4. **Stale notices are dropped.** When a notice's checked id is gone but is no longer the anchor, the request is unchanged, and notices still bound to the deleted message are dropped.
5. **Following a repost takes one more tick on the dispatch path.** When `dispatch_request` follows a repost, the check-in or invite runs on the next tick (15 s), not in the same pass.

## Alp's live state
Fixed by the code path. After the deploy, the next poll sees `wait: Delivery` with a past `check_at`. `check_request` finds message 1571287459566981353 gone (Discord 404), and `drop_deleted` hands it to `on_gone`. There is no repost, so the request is cleared with the log "Invite-Lounge verwirft gelöschte Bitte still". No manual DB change.

## Follow-up changes in this package
- **Check-in in all lounge channels.** e2cd593c had limited it to #deadlock-invite; that was an implementation choice, not a user rule. Without the 60 min invite, room requests in other channels such as #frag-die-community would dead-end, so they now get the same question. Test `nachfrage_auch_ausserhalb_von_deadlock_invite`.
- **Late "yes" while the invite call is in flight.** `finish_dispatch` leaves the closed request alone, so no "Invite ist raus" follows the OK. Test `spaetes_ja_waehrend_des_versands_bleibt_geschlossen`.
- **Judge prompt** (Deadlock-Brain branch `fix/invite-judge-playtest-20261115`):
  - It asks about the Steam playtest invite or owning the game.
  - An accepted friend request counts as `unclear`, so it cannot cancel the follow-up invite during the Steam wait.
  - A non-verdict Brain reply counts as `unclear`, so there is no retry loop; only Brain errors hold the answer.
- **Gateway `message_update` mapping** is tested with a MESSAGE_UPDATE payload (`bearbeitete_nachricht_wird_mit_inhalt_weitergegeben`).

## Limits to know
- **Request window.** A request without an answer stays open for the request window (`REQUEST_WINDOW_SECONDS` = 7 days). Answers are read in `Pending` (with or without Steam wait) and `Dispatching` inside that window. A stale `Dispatching` request outside the window takes no answers (test `veraltete_bitte_im_versand_nimmt_keine_antwort_mehr_an`).
- **When "no" invites.** A "no" sends the friend request only after the check-in was asked and before any invite attempt, as the user decision says.
  - A "no" written before the check-in is read but does not invite. The check-in still comes at 30 min, and its answer decides.
  - A "yes" closes the request in `Pending` (with or without Steam wait) and in `Dispatching`.
- **Edits** revive only `WaitingForCode` requests inside that window.
- **Deleted and reposted direct requests** start a new direct request, as on main: an explicit ask is answered again.
- **Missed requests.** A new request posted while the message stream lagged, or while the bot was down, is not recovered. That gap predates this change (a1843447) and is tracked in `claude-config/orchestrierung/OFFENE-URSACHEN.md`.

## Dry run against live states (2026-11-15 00:20 UTC)
15 open states.
- Alp: `Pending`, `wait: Delivery`, next Steam check due, created after `actions_since` (1794031680). The next poll drops it.
- The other 14 are `WaitingForCode`: 13 are older than 7 days, and one is 31 h old in #deadlock-invite. They stay inert; only an edit with a code inside the window revives one.

## Live check
- Journal of `deadlock-bot-rust.service` (user unit): "Invite-Lounge verwirft gelöschte Bitte still" for user 1037837117818142741.
- kv `invite_lounge:requests/user:1037837117818142741`: `request` absent, `last_result = "Bitte gelöscht; still beendet"`.
