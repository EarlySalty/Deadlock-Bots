# Invite lounge: silence never sends a friend request (2026-11-15)

## Trigger
Alp (kv `invite_lounge:requests/user:1037837117818142741`) posted a friend code in #deadlock-invite at 22:15 UTC on 2026-11-14 and then deleted the message. At 22:45 UTC the lounge posted the check-in without a reply reference, because the request was gone. At 23:15 UTC it sent a friend request on its own, and it ignored "Jo habe ich schon !" at 23:16 UTC. The state stayed `Pending` with `wait: Delivery`, so a "not accepted" notice would have gone out about 24 h later.

## User decision (binding)
1. Silence never triggers a friend request. Flow: the request, no staff reply, then the check-in. "No" sends the friend request. "Yes" gets a short OK and closes. No answer means nothing further happens.
2. A deleted request message drops the request silently.
3. A late answer is read as long as the request is open, whatever its state.
4. Audit all lounge message paths against the real channel history and fix each class of failure.
5. 2026-11-16 ("FIX IT"): a stored friend code belongs only to its own request; a new request without a code uses the linked Steam account or asks for the code with the hint, and never reuses a code from an earlier request or from chat; a `WaitingForCode` request expires with the 7-day window; a "ja" at any point ends the request, with no follow-up and no "nicht angenommen" notice. Rules in "Friend-code rule" below.

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

### Friend-code rule (user decision 2026-11-16)
- **A stored code belongs only to its own request.** `friend_code` is written in exactly these places, each for the request it belongs to:
  - a new request (`update_state`, replace branch): the code in the message, else the code of the member's own open request that this ask replaces (fresh, same channel, `Pending` or `Dispatching`), else the linked Steam account (`known_code`), else none;
  - the completion of a fresh `WaitingForCode` request in its channel by a plain code;
  - a restart for a new code (`restart_for_new_code`);
  - a new code in a check-in answer (`record_answer`, only while `takes_answer_code` holds);
  - a code edited into a fresh `WaitingForCode` request (`take_edited_code`).
  - `forget_request` (deleted request) clears it. The history backfill (`record_history`) no longer writes it; it records only `last_hint_at` of hinted requests.
- **A new request without a code never reuses an earlier code.** It takes the linked Steam account or asks for the code with the hint (tests `neue_bitte_ohne_code_nutzt_keinen_code_einer_frueheren_bitte`, `rueckblick_bewahrt_den_tipp_und_uebernimmt_keinen_code`, `raumbitte_ohne_code_verwendet_keinen_alten_code`). On 2026-11-16, 65 live rows held a code with no request (backfill or main writes); those codes are now never used.
- **A corrected code after the invalid-code notice belongs to that request.** While the request is `Attempted` with the `InvalidCode` notice, inside the window and in its channel (`awaits_corrected_code`), a plain or room message with a new code restarts it at that message as a room request with a fresh status. The friend request then follows the check-in and a "nein", so a number alone never sends one (test `korrigierter_code_nach_ungueltigem_code_gehoert_zur_bitte`, which runs restart, check-in, "nein" and the invite to the new code). A direct ask with the new code replaces the request and invites at once, as any direct request.
- **The request window bounds every open request.** One predicate, `open_request` (phase `WaitingForCode`, `Pending` or `Dispatching`, `is_fresh` inside the 7-day window, exact channel match), decides whether a plain message may complete or change a request (`takes_plain_code`) and whether a new ask continues a direct `WaitingForCode` request or carries its code. A `WaitingForCode` request older than the window is treated as expired: a number does not complete it (test `wartende_bitte_nach_dem_fenster_nimmt_keine_zahl_mehr_an`) and a new room request does not inherit its direct flag (test `alte_direkte_wartebitte_macht_eine_neue_raumbitte_nicht_direkt`). Its phase stays `WaitingForCode` in the row; 13 such live rows existed on 2026-11-16. A missing request channel counts as no match.
- **A plain message outside an open request changes nothing** and logs "Invite-Lounge übernimmt keinen Freundescode ohne offene Bitte in diesem Kanal" (tests `zahl_in_einem_anderen_kanal_aendert_die_offene_bitte_nicht`, `zahl_ohne_offene_bitte_wird_kein_freundescode`). Consequence: a code-less request whose member posts only a bare code in a different channel stays in `WaitingForCode` until an edit or a new request.
- **"Ja" ends the request in every open phase.** `apply_answer` with `Arrived` sets `Attempted`, clears the Steam wait and purges the request's queued notices, also after a friend request went out (`Pending` with `wait: Delivery`). While any answer is held unjudged (`answer_pending`), `dispatch_request` runs neither the Steam follow-up nor an invite, and `deliver_outbox` sends no notice, so a "ja" held through a Brain error is judged before a "nicht angenommen" notice could go out (test `ja_nach_der_freundschaftsanfrage_beendet_die_bitte_auch_bei_brain_fehler`). The cost is that notices and follow-ups wait until the held answer is judged: each retry follows the Brain backoff (capped at 60 min), and a held answer whose request leaves the 7-day window is discarded (`answer_open`), which ends the pause.
- **Alp's timeline replayed** (test `alps_verlauf_ohne_antwort_keine_anfrage_und_nach_ja_keine_absage`): a member with a stale stored code and a linked account posts a code-less room request; the request uses the linked code, the check-in goes out at 30 min, ticks at 60 min and 60 min 10 s send nothing, "Jo habe ich schon !" closes it with the OK, and a tick 25 h later with an expired Steam request sends no notice and no invite.

### Answer intake
- **Where messages go.** A requester message that is not a direct bot request goes to `record_answer`, which loads the state and decides:
  - A room-phrased message with the stored code, posted before the check-in was asked, is not an answer. It is treated as a repost (see "Reposts and deleted requests").
  - After the check-in was asked, the same message is an answer. "Nein, kann mich jemand einladen? 123456789" sends the friend request (test `nein_in_bittenform_mit_gleichem_code_loest_die_einladung_aus`). It is also recorded as a repost in the same compare-and-swap.
  - A room-phrased message without a code is an answer (test `nein_in_bittenform_wird_als_antwort_gewertet`).
  - An answer that repeats the stored code is judged as an answer (test `nein_mit_dem_freundescode_loest_die_einladung_aus`).
- **New friend code.** A message with a new friend code counts as an answer only if all three hold: the check-in was asked, the invite has not started (`Pending`, no Steam wait, no dispatch since the request), and the answer queue has room (`takes_answer_code`).
  - The answer and the new code are then stored in one compare-and-swap, logged as "Invite-Lounge übernimmt neuen Freundescode aus der Antwort". "nein, mein Code ist 987654321" sends the friend request to 987654321 (test `neuer_code_in_der_antwort_geht_an_den_neuen_code`).
  - Otherwise the message is logged with reason "neuer Freundescode". In `update_state`, a changed code on a `Pending` or `Dispatching` request (room or direct, with or without Steam wait) restarts the request at the new message as `Pending` with a fresh status and clears `dispatch_started_at_millis` (`restart_for_new_code`, log "Invite-Lounge startet die Bitte mit dem neuen Freundescode neu"). The restarted request is direct only when the new message is itself a direct request ("Kannst du mich einladen? 987654321"); `Request::restart_at` takes the flag from its caller. A plain message with a new code ("mein Code ist 987654321") makes it a room request. Tests `schlichter_neuer_code_bei_direkter_bitte_laedt_nicht_von_selbst` and `raumbitte_im_versand_mit_neuem_code_laedt_nicht_von_selbst` assert only that the restart sends no friend request in the same pass; the later check-in and "nein" path for a restarted room request is run end to end by `korrigierter_code_nach_ungueltigem_code_gehoert_zur_bitte`.
  - A new request in another lounge channel is still a request and is handled as one.
  - During a running invite call, the restart drops that call's result: `finish_dispatch` matches its claim token (`dispatch_started_at_millis`), which the restart cleared, so the late finish changes nothing. A direct restart may become due and run a second bridge call for the new code while the old call is still in flight; that is the price of never tracking an uninvited code.
  - Tests: `neuer_code_vor_der_nachfrage_startet_die_bitte_neu`, `neuer_code_nach_der_einladung_startet_die_bitte_neu`, `neuer_code_waehrend_des_versands_startet_die_bitte_neu` (finish of the old call is a no-op, the next dispatch invites the new code), `direkte_bitte_mit_neuem_code_im_laufenden_versand_bleibt_direkt`. `neuer_code_in_der_antwort_braucht_platz_in_der_schlange` covers only the full-queue refusal.
- **Queue.** Answers are queued in `held_answers` and judged in the dispatch task, never on the gateway loop.
  - The limit is 5. A member answers the check-in with one to three short lines in the channel history. Five covers a burst while the Brain is down, keeps the first answer at the head and bounds the state row.
  - A sixth answer logs "Invite-Lounge verwirft Antwort, weil schon zu viele Antworten auf ein Brain-Urteil warten" (test `volle_antwortschlange_nimmt_keine_weitere_antwort_auf`).
- **Leases.** Before the Brain call, the head answer is leased by compare-and-swap.
  - The leased work runs under a cut-off of 2 x the configured Brain timeout, enforced by `tokio::time::timeout` (`claim_cutoff`). For answers that work is `judge_answer` plus `already_invited` in `settle_answers`; for notices it is `send_notice` in `deliver_outbox`.
  - Cut-off derivation: the Brain call is bounded by the configured Brain timeout T. The Discord and DB steps around it (existence check, `already_invited`, the post) have no own bound, so they get the same budget again: cut-off = 2T. Overrun: the timeout aborts the work, the attempt counts as failed and retries with backoff.
  - Lease derivation: the lease must outlast the cut-off plus the closing load and compare-and-swap, so it is 2 x cut-off (`claim_lease_millis`, test `frist_und_leihfrist_folgen_dem_brain_zeitlimit`). If the closing steps still overrun, `store_answer` writes only while the queue head equals the leased copy (test `abgelaufene_leihe_wendet_kein_zweites_urteil_an`), and a notice re-claimed after expiry is caught by its idempotency key and `bot_replied_since`.
  - So a tick and a message handler do not judge the same leased answer twice (test `geliehene_antwort_wird_nicht_doppelt_gewertet`).
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
- **One request, several messages.** The request keeps its live anchor (`message_id`), the same-code room copies still visible (`reposts`) and the anchors that were deleted after a follow (`earlier`). Each list keeps the last five ids (`REQUEST_COPY_LIMIT`). The bound keeps the state row small for a member who reposts in a loop. Its consequence: staff help given only on a copy older than the last five is not seen, so the check-in asks, and a "ja" then closes the request; nothing is invited without a "nein". Reposts are recorded only for same-code room messages in the request channel while the request is `Pending` or `Dispatching` (`note_repost` checks `accepts_repost` itself). A duplicate id is not added twice (test `kopien_der_bitte_bleiben_begrenzt_und_ohne_doppelte`). Both lists are skipped when empty, so rows without them keep their exact stored text.
- **A repost does not move the request.** A same-code room message on a `Pending` or `Dispatching` room request is added to `reposts`; the anchor stays.
- **Staff help on any copy counts.** `request_answered` and `answers_request` match a staff reply to any id of the request (anchor, reposts, earlier) and read history back to the oldest of them. Tests: `doppelte_bitte_behaelt_den_anker_und_hilfe_von_hand_zaehlt`, `hilfe_an_einer_kopie_der_bitte_zaehlt`, `hilfe_an_der_geloeschten_ersten_bitte_zaehlt_nach_dem_folgen`.
- **`on_gone` decides when the anchor is gone.** Every existence check that finds a request message gone goes through `on_gone(state, checked_message_id)`: the two `check_request` Gone arms in `dispatch_request` and the 404 in `reread_waiting_request` (all through `drop_deleted`), and the Gone result in `settle_notice`.
  - The checked id is no longer the anchor: the request stays as it is.
  - A repost is left: the anchor moves to the newest repost, the old anchor goes to `earlier`, queued notices move along, and `created_at`, the phase and the check-in clock stay. This holds in `Dispatching` too, because `finish_dispatch` recognises its request by the claim token, not by the message id. Logged as "Invite-Lounge folgt der neu geposteten Bitte". Tests: `neu_gepostete_bitte_behaelt_die_nachfrage`, `gleicher_code_schiebt_die_nachfrage_nicht_auf`, `geloeschte_bitte_im_versand_folgt_der_neu_geposteten`, `geloeschte_kopien_werden_der_reihe_nach_verfolgt_und_zuletzt_verworfen`.
  - No repost left: the request is forgotten silently, logged as "Invite-Lounge verwirft gelöschte Bitte still" (test `geloeschte_wartende_bitte_wird_beim_abgleich_still_verworfen` for the re-read path).
- **The post itself refuses a gone target.** Lounge notices are posted by the Brain through the broker (`send_channel_reply` -> `reply_body`). `reply_body` now sets `fail_if_not_exists: true`, so a request deleted between the existence check and the post makes the post fail instead of landing standalone (test `broker_antwort_ohne_zielnachricht_wird_nicht_ohne_bezug_gepostet`). The notice then retries, its existence check sees the deletion and `on_gone` decides. The broker logs this expected case as an error with HTTP 502. The only broker client that sends `reply_to_message_id` is the Brain's lounge notice delivery (searched across ~/repos and ~/Documents), so no other caller changes behaviour. Writers that keep `fail_if_not_exists: false` on purpose, none of them a lounge path: `bin/dl-bot/src/mcp.rs:1237` (operator MCP send), `bin/dl-bot/src/scrimglue.rs:4262` (scrim source reply), `crates/dl-community/src/voice_change_hint.rs:72` (voice hint). The broker runs inside the dl-bot binary, so this fix goes live with the dl-bot deploy.
- **Notices decide on the id they checked.** `settle_notice` uses the message id that the notice checked, taken from the claimed entry, not the reloaded one (test `meldung_zur_geloeschten_bitte_folgt_der_neu_geposteten`).
- **Held answers stay open.** A held answer stays open while the request is open, even after the anchor moved past it. So a "nein" held during a Brain error is still judged after the original was deleted and reposted, and the friend request goes out (test `gehaltenes_nein_folgt_der_neu_geposteten_bitte`).
- **Discord errors hold the step instead of dropping it.**
  - Before a friend request or a Steam follow-up, the step is held for 5 min (`RETRY_SECONDS`, the existing Steam retry interval), logged as "Invite-Lounge hält die Einladung zurück, bis die Bitte prüfbar ist". No friend request goes out for a request that may be deleted (test `nicht_pruefbare_bitte_haelt_die_einladung_zurueck`).
  - The check-in goes ahead, and its notice waits in the outbox until Discord answers (test `nicht_pruefbare_bitte_haelt_die_nachfrage_bis_discord_antwortet`).
- **The code hint** runs in the dispatch task, not on the gateway loop, and so does the history lookup for an earlier hint (`hint_in_history`, only when no hint is stored). A history error there counts as "no earlier hint" and is logged as "Invite-Lounge: Verlauf vor dem Hinweis nicht lesbar, Hinweis geht trotzdem raus"; the request is stored either way (test `verlaufsfehler_vor_dem_hinweis_verliert_die_bitte_nicht`).
  - A failed reply or a Brain error releases the hint claim. `release_hint` reloads and resets the hint fields only while the claim it made still stands, retrying on a compare-and-swap conflict, so a write for the same user in between does not keep the hint blocked (tests `fehlgeschlagene_hinweis_antwort_gibt_den_hinweis_frei`, `hinweisfreigabe_haelt_trotz_zwischenzeitlicher_schreibung`).
  - The reply sets `fail_if_not_exists: true`, so a hint never lands without its request.
  - The claim (`hint_sent`) is set before the reply, as on main. A crash between the claim and the reply loses that one hint; this is unchanged main behaviour.
- **One composition for tests and production.** The gateway path (`spawn_recorded`) and the test-only `handle_message` both run `run_recorded` (hint, due dispatch, outbox delivery, each error logged).

### Edits and missed messages
- **Edits are re-read on an interval.** Every 5 min (`RETRY_SECONDS`) after the history backfill, one task re-reads the request message of each `WaitingForCode` request inside the window, one Discord GET per request (`reread_waiting_requests`, test `abgleich_findet_waehrend_der_pause_nachgereichten_code`). The window is applied in SQL (`waiting_users(created_since)`), so rows older than 7 days are not loaded (Postgres test `gehaltene_antwort_im_versand_zaehlt_als_offen` with a stale row). A deleted request goes to `drop_deleted` and is dropped silently. An error for one user is logged and skipped, and the next interval retries.
- **Lost messages are not recovered.** Messages and answers lost to downtime or to a `Lagged` gateway stream are not recovered, as on main; both `Lagged` arms only log. A lost answer behaves like silence: nothing is sent.
- **Hand-served requests.** A request a human served after the check-in sends nothing on a later "no" (test `nein_nach_einladung_von_hand_sendet_keine_anfrage`).
- **No upper cutoff for the check-in.** It fires any time from 30 min after the request while the request window is open, so downtime or a long Brain outage cannot skip it.
- **Active dispatch window.** A direct request without a new code that arrives while an invite call runs does not replace it for 2 x `INVITE_FORWARD_TIMEOUT_SECS` (`dispatch_active`, test `laufender_versand_bleibt_ueber_das_steam_zeitlimit_hinaus_stehen`). Derivation: the bridge call is bounded by `INVITE_FORWARD_TIMEOUT_SECS`, the store steps around it get the same budget again. Overrun: a direct request replaces the stuck dispatch, and the late finish is a no-op because the request is no longer `Dispatching` under that claim token (`finish_dispatch` requires both).


### Stored state format
Old rows still carry the removed field `check_in_message_id`. The text compare-and-swap would never match them. So `load` rewrites a row whose canonical form differs, guarded by the stored text, and logs "Invite-Lounge schreibt Zustand im aktuellen Format neu". `normalize` and `compare_exchange` share one text compare-and-swap (`swap_text`). Postgres test: `altzeile_wird_beim_laden_umgeschrieben_und_bleibt_schreibbar`. Eight live rows had the field on 2026-11-15. The serialized state has no maps, so its canonical text is stable.

## Deviations from the round-3 decisions
1. **Reposts after the check-in are recorded too.** D3 recorded reposts only before the check-in. After the check-in, a same-code room message is both an answer (D1) and a recorded repost, in one compare-and-swap. Without that, a deleted and reposted request after the check-in would be forgotten and a held "nein" would never invite (test `gehaltenes_nein_folgt_der_neu_geposteten_bitte`).
2. **A new code in an answer is taken only on an untouched invite with queue room.** Otherwise the message restarts the request with the new code (see "Answer intake"). Changing the code under a running Steam wait or dispatch without a restart would track progress against a code that was never invited.
3. **`finish_dispatch` matches its claim token.** It used to match the request by message id, which forced a gone anchor to wait during `Dispatching`. With the token, the anchor can follow a repost at any time and a restart for a new code turns the late finish into a no-op.
4. **Stale notices are dropped.** When a notice's checked id is gone but is no longer the anchor, the request is unchanged, and notices still bound to the deleted message are dropped.
5. **Following a repost takes one more tick on the dispatch path.** When `dispatch_request` follows a repost, the check-in or invite runs on the next tick (15 s), not in the same pass.

## Alp's live state
The old main binary finished Alp's request before this fix went live. Row on 2026-11-16 (redacted): `request.phase = Attempted`, `direct: false`, notices `CheckIn`, `FriendRequest`, `NotAccepted`, no outbox, no held answers. The "nicht angenommen" notice was already sent by main. An `Attempted` request is not polled (`pending_users` selects `Pending` rows and rows with held answers), so after the deploy nothing more goes to Alp from this row. No manual DB change. The timeline itself is replayed by `alps_verlauf_ohne_antwort_keine_anfrage_und_nach_ja_keine_absage` and `ja_nach_der_freundschaftsanfrage_beendet_die_bitte_auch_bei_brain_fehler`.

## Follow-up changes in this package
- **Check-in in all lounge channels.** e2cd593c had limited it to #deadlock-invite; that was an implementation choice, not a user rule. Without the 60 min invite, room requests in other channels such as #frag-die-community would dead-end, so they now get the same question. Test `nachfrage_auch_ausserhalb_von_deadlock_invite`.
- **Late "yes" while the invite call is in flight.** `finish_dispatch` leaves the closed request alone, so no "Invite ist raus" follows the OK. Test `spaetes_ja_waehrend_des_versands_bleibt_geschlossen`.
- **Judge prompt** (Deadlock-Brain 76e122f3, on Brain `origin/main` since 2026-11-15):
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
- **A room request while the member's earlier request is closed** (`Attempted` without the invalid-code notice, or `Expired`) is not taken as a new request, as on main, and its code is not stored (test `raumbitte_nach_abgeschlossener_bitte_aendert_den_code_nicht`). A direct ask always starts a new request. Tracked in `claude-config/orchestrierung/OFFENE-URSACHEN.md`.
- **Missed requests.** A new request posted while the message stream lagged, or while the bot was down, is not recovered. That gap predates this change (a1843447) and is tracked in `claude-config/orchestrierung/OFFENE-URSACHEN.md`.

## Rollback
A main binary does not know `held_answers`, `reposts`, `earlier` or the outbox `answer` field. It drops them on load, and its text compare-and-swap never matches a row that carries them, so such a request would stop moving. Before running a main binary again, list the rows in `bot.kv_store` namespace `invite_lounge:requests` that carry any of these fields and either clear their open request or rewrite them without the fields. Rows without them are read unchanged.

## Privacy
The lounge state keeps member text (queued answers in `held_answers`, the outbox `answer`) for up to the request window. The privacy eraser does not list the `invite_lounge:*` namespaces yet; tracked in `claude-config/orchestrierung/OFFENE-URSACHEN.md` instead of a partial fix here.

## Dry run against live states (2026-11-16, redacted aggregate)
242 rows in `invite_lounge:requests`.
- 65 rows: no request, stored code. Not polled; the code is never used by a later request.
- 163 rows `Attempted` (11 inside the window, including Alp). Not polled. None carries the `InvalidCode` notice, so no row is open for a corrected code.
- 14 rows `WaitingForCode`: 1 inside the window in #deadlock-invite, 13 older than 7 days (2 with a stale stored code). The old ones stay inert; a number does not complete them.
- No row has held answers or a `Pending` phase.

## Live check
- Journal of `deadlock-bot-rust.service` (user unit) after the restart: no lounge line for user 1037837117818142741.
- kv `invite_lounge:requests/user:1037837117818142741` unchanged against the pre-deploy row (phase `Attempted`, same notices, no outbox).
