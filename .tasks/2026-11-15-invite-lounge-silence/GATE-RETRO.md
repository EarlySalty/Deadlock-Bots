# Gate retro after round 4 (2026-11-15)

Branch `fix/invite-lounge-silence`, rounds 1 to 4 all BLOCK. Blocking findings per round (gate log counts, `[hook]` lines included): r1 107, r2 132, r3 111, r4 110. Fix commits: 5cf9e76d (initial), b671d17e and fe428ef8 and 7172ad65 (after r1/r2), 06760585 (after r2), 4dfe2f5e (after r3).

## 1. Why four rounds

- **Every fix round added a new mechanism, and each mechanism brought its own edges.** The user spec is small (ask, "nein" invites, "ja" says ok, deleted request stays silent). The branch grew an answer queue with leases, a Brain cut-off, a text-CAS normaliser, an edit path, a 5-minute re-read, a repost model and an off-loop hint. Round 4 findings sit almost entirely in mechanisms added in rounds 2 and 3:
  - `release_hint` lost its serialisation when 4dfe2f5e moved the hint off the gateway loop (regression, cluster E).
  - The single-slot `repost` from 4dfe2f5e is the source of clusters C and D and of the dead `Repost.created_at` (regression).
  - The answer-code rule from 4dfe2f5e ("deviation 2") claimed a restart that `update_state` only does for room requests in Pending (cluster A, incomplete fix of a class).
- **The README states absolute guarantees.** The reviewer checks each sentence literally. Every sentence of the form "never", "all three sites", "always restarts" that the code meets only partly became a BLOCK (r4: A, B, C, the test credited for a rule it does not run).
- **Twin lists were enumerated by memory, not by search.** 4dfe2f5e listed three `on_gone` sites; the re-read path is a fourth existence check (B). It listed Brain calls on the gateway loop; `has_hint` is a REST call on the same loop (F).
- **Reviewer drift is small.** Of 57 reviewer items, all but the NITs reproduce against the code. The 53 `[hook]` lines restate reviewer items as twin or input-class lines; they close when the clusters below close.
- **Scope creep:** moderate. The 5-minute re-read and the edit path were asked for by the round-1 audit (class 7), not by the user spec; they are kept because they are live failure classes from the channel history.

## 2. Triage

Clusters group the duplicates. Finding numbers refer to `gate-r4.log`.

### REAL

| Id | Findings | Defect | Reason it is real |
|---|---|---|---|
| A | 1-4, 8, 17, 29, 44, 54-61, 77, 86, 94, 99, hook README twins | A new friend code that is not taken as an answer overwrites `friend_code` (`update_state` ~1929) but restarts only a Pending room request (~1983). In Dispatching, or on a direct request in a Steam wait, follow-up and `already_invited` then track a code that was never invited; Delivery ends in `Finish(None)` silently, Retry sends an unrequested invite to the new code. | Reproduced by trace; contradicts README:52/104 and commit text. |
| B | 7, 24, 67, 80, 87, 95, 107 | `reread_waiting_request` treats a deleted request (`Ok(None)`) as no change. The request stays and is fetched every 5 min for 7 days. | Violates user rule 2 and the "all gone handling goes through on_gone" claim. |
| C | 3, 4, 20, 21, 27, 32, 37, 50, 62, 76, 84, 105, 74 | `request_answered`/`answers_request` match only the current anchor. Staff replies to the repost (before follow) or to the deleted original (after follow) do not count, so the check-in is asked and a later "nein" can invite a member staff already served. With more than one repost the single slot drops earlier ones (74). | Same mechanism the branch claims to fix ("staff replies to the original keep counting"). |
| D | 15, 36, 49, 66, 85, 104, 89 | `on_gone` defers on any Dispatching phase; `update_state` treats Dispatching as active only inside `ACTIVE_DISPATCH_MILLIS`. A crash mid-invite leaves notices deferred for 7 days. | Two rules for one question. |
| E | 22, 26, 71, 75, 108 | `release_hint` does one CAS against the claim snapshot. Since the hint runs in the dispatch task, any write for the user during the Brain call makes the release fail; the hint stays claimed for 24 h. | Regression from 4dfe2f5e; contradicts README:87. |
| F | 40, 58, 19 | `record_message` awaits `has_hint` (REST history paging) on the gateway loop, and its `?` returns before the state CAS: a Discord error drops the new request. | Same class 4dfe2f5e fixed for `send_brain_hint`. |
| G | 9, hook twin invite_lounge.rs:163, NITs 114/125/131 | Lounge notices are posted by the broker (`dl-broker/handlers.rs:683` -> `send_channel_reply` -> `reply_body`, `adapter.rs:310`) with `fail_if_not_exists: false`. A request deleted between the existence check in `send_notice` and the broker post gets a standalone notice. | Violates user rule 2 in a real (seconds-wide) window; the hint path already uses `true`. |
| H | 63, 65, 69, 83, 90, 91, 92 | The cut-off (2 x Brain timeout), lease (2 x cut-off) and active-dispatch window (2 x bridge timeout) carry no derivation. | Unexplained margins are a rule violation of the gate; the lease also delays recovery of a lost claim to 480 s. |
| I | 16, 79, 96 | `waiting_users` has no age bound; the 5-minute re-read loads every historical `WaitingForCode` row forever. | Unbounded work, bound lives in the callee. |
| J | 64, 68, 72, 73, 78, 82, 88, 98, 103, 106, 109, 14 | `Repost.created_at` is persisted, never read. | Dead persisted field. Resolved by the C redesign. |
| K | 70, 73, 81, 25 | `drop_deleted`'s Unchanged/Deferred return can never fire. | Dead guard. Resolved by C/D. |
| L | 93, NIT 136 | Test-only `handle_message` duplicates `spawn_recorded` with different error handling. | Two truths; tests assert a composition production does not run. |
| M | 100, 102, 101, NIT 120 | Queue-room rule written twice (`takes_answer_code`, `queue_held_answer`); "dispatched since this request" written twice (`invite_untouched`, `someone_on_it`); unused return values (`queue_held_answer`, `note_repost` in `record_answer`, `forget_request`). | Duplicated rules, dead returns. |
| N | 97, 41 | `dispatch_request` reads the clock before its load; an expiry during the load is not caught before the check-in. | Ordering regression against base. |
| O | 110, 61 | README credits `neuer_code_in_der_antwort_braucht_platz_in_der_schlange` for "restarts the request"; README:52/104 overclaim. | Misleading doc. Rewritten with A. |
| P | 5, 6, 10-13, 18, 23, 28-31, 33-35, 38, 39, 42, 43, 45-48, 51-53 (`[hook]` unit/input-class and root-cause lines) | Hook restatements of A-I per unit (concurrency, stale_state, null_missing, max_overflow, retry). | Close with their clusters; the Ursache block of the fix commit must list every twin site by search. |

### TRIVIA (kept, with reason)

| Findings | Reason |
|---|---|
| NIT 111, 112, 118, 132, 135 (live config, external branch, OFFENE-URSACHEN claims) | Facts outside the clone; checked at deploy time. Live `brain_api_timeout_ms` is read during the live proof and the README number corrected then. |
| NIT 113 (release hint on transport error may double a hint) | Speculative; one extra hint within 24 h, no reachable harm. |
| NIT 115, 117, 119, 141 (rollback hazard of new fields on main binary) | Only on a rollback. README "Rollback" names the concrete step: before a main binary runs, clear or rewrite the rows that carry `held_answers`, `reposts`, `earlier` or an outbox `answer`. |
| NIT 116 | Same as I, fixed there. |
| NIT 121 (`retry_held_answer` empty-queue branch) | Defensive, harmless. |
| NIT 122, 124 (test `LEASE` literal) | Test parameter only; derived anyway in the H fix. |
| NIT 123 (privacy eraser does not list the namespace) | Real, and partly new on this branch (`held_answers`, outbox `answer` carry member text; Fable). Not half-fixed here: the eraser gap covers `friend_code` on main too. Filed in claude-config OFFENE-URSACHEN (6b25202), README "Privacy". |
| NIT 126, 133 (test naming, repeated `moving_clock`) | Style only. |
| NIT 127, 129, 134, 137 (test precision, fake fidelity) | No rule is unguarded by them; test-only. |
| NIT 128 (Postgres tests behind `--features testing`) | Run in the fixer pass with the test DB. |
| NIT 130 (check_in repeats someone_on_it's history scan) | Predates the branch; cost only. |
| NIT 138, 139 (adapter doc comment, changelog variant) | Predate the branch, doc wording. |
| NIT 140 (MESSAGE_UPDATE without `type`) | Checked on live logs after deploy; the 5-minute re-read covers a miss. |

## 3. Fable second opinion (claude-fable-5-2, medium)

- Agreed with A, C and D as real. Proposed matching `finish_dispatch` by its claim token instead of the message id, which removes the `Deferred` arm entirely.
- Warned against flipping `fail_if_not_exists` for every broker reply without checking callers. Checked: the Brain's lounge notice is the only broker client that sends `reply_to_message_id` (search over ~/repos and ~/Documents), so the flip changes only this path.
- H needs stated bounds and the failure mode on overrun, not new numbers.
- NIT 123 is real for the new text fields; NIT 141 needs a concrete pre-rollback step.

## 4. Fix pass as done

1. **A.** Any changed code on a `Pending` or `Dispatching` request restarts it at the new message (`restart_for_new_code`) and clears the claim token; `finish_dispatch` matches the token, so the late finish of the old call is a no-op. `dispatch_active` is the one "active" predicate for the replace rule.
2. **C/D/J/K.** `repost: Option<Repost>` replaced by `reposts` and `earlier` (last five each, no duplicates). Staff checks match any id; `on_gone` follows the newest repost in every phase; `GoneOutcome::Deferred` and the dead `drop_deleted` guard are gone.
3. **B.** `reread_waiting_request` hands a 404 to `drop_deleted`.
4. **E.** `release_hint` reloads and resets only while its own claim stands, retrying on conflict.
5. **F.** `hint_in_history` runs in the dispatch task; an error there means "no earlier hint" and never drops the request.
6. **G.** `reply_body` sets `fail_if_not_exists: true`.
7. **H.** No new numbers. README states each bound's derivation and overrun mode (cut-off, lease, active window). The test `LEASE` literal stays (test parameter, NIT 122).
8. **I.** `waiting_users(created_since)` filters by request age in SQL.
9. **L/M/N.** `run_recorded` shared by `spawn_recorded` and the test `handle_message`; `queue_has_room` and `dispatched_since_request` helpers; dead returns dropped; `dispatch_request` reads the clock after its load.
10. **O/P.** README rewritten without claims beyond the tests; the fix commit carries a four-layer block with a searched twin list.

New tests: `neuer_code_waehrend_des_versands_startet_die_bitte_neu`, `neuer_code_bei_direkter_bitte_im_steam_warten_geht_an_den_neuen_code`, `hilfe_an_einer_kopie_der_bitte_zaehlt`, `hilfe_an_der_geloeschten_ersten_bitte_zaehlt_nach_dem_folgen`, `geloeschte_kopien_werden_der_reihe_nach_verfolgt_und_zuletzt_verworfen`, `kopien_der_bitte_bleiben_begrenzt_und_ohne_doppelte`, `geloeschte_wartende_bitte_wird_beim_abgleich_still_verworfen`, `abgleich_liest_keine_bitten_ausserhalb_des_fensters`, `hinweisfreigabe_haelt_trotz_zwischenzeitlicher_schreibung`, `verlaufsfehler_vor_dem_hinweis_verliert_die_bitte_nicht`, `abgelaufene_leihe_wendet_kein_zweites_urteil_an`, `broker_antwort_ohne_zielnachricht_wird_nicht_ohne_bezug_gepostet`, `geloeschte_bitte_im_versand_folgt_der_neu_geposteten` (replaces the Deferred test).

Checks: dl-community 355 passed (4 DB-harness tests need `CENTRAL_TEST_DSN`), invite lounge with Postgres on ionos 134 passed; dl-discord, dl-broker, dl-bridges green; dl-bot only the 8 DB-harness `build_publisher` tests fail without `CENTRAL_TEST_DSN`. fmt clean, no clippy warning on branch lines.

## 5. Original fix plan (before Fable)


1. **A, code belongs to its request.** In `update_state`, a new friend code outside an accepted answer either restarts the request at the new message (any Pending request, direct or room, with or without Steam wait, and a stale Dispatching) or, while an invite call is active, is not stored and is logged. One predicate `dispatch_active(state, now_millis)` decides "active" for `update_state` and `on_gone`. Tests: new code in active Dispatching (room and direct) keeps the old code; new code on a direct request in a Steam wait restarts as a direct request and invites the new code.
2. **C/D/J/K, request identity.** Replace `repost: Option<Repost>` with the list of the request's other message ids (reposts still visible plus earlier anchors that were deleted), bounded. Staff checks (`request_answered`, `answers_request`) match any id of the request. `on_gone` follows the newest remaining repost and keeps the deleted anchor in the list; `finish_dispatch` recognises its request by any id, so `Deferred` is no longer needed and `drop_deleted` loses its dead guard. The field is new on this branch and not deployed, so no stored row carries the old shape.
3. **B.** `reread_waiting_request` hands `Ok(None)` to the same gone path (`drop_deleted`).
4. **E.** `release_hint` reloads and resets the hint fields only while the claim it made still stands, retrying on CAS conflict.
5. **F.** The history check moves into the dispatch task (`send_hint`); an error there counts as "no hint known".
6. **G.** `reply_body` sets `fail_if_not_exists: true`: a broker reply whose target is gone fails instead of posting standalone. The lounge notice then retries, its existence check sees the deletion and `on_gone` decides.
7. **H.** Derive bounds in one place: cut-off = Brain timeout for the Brain call plus the same budget for the unbounded Discord/DB steps around it; lease = cut-off plus a named tail for the closing CAS; active dispatch = bridge timeout plus the same tail. README states each derivation and the failure mode when exceeded.
8. **I.** `waiting_users` filters by request age in SQL.
9. **L/M/N.** One shared `run_recorded` for production and tests; one queue-room helper; one "dispatched since request" helper; drop unused returns; read the clock after the load in `dispatch_request`.
10. **O/P.** README rewritten for A to I without absolute claims beyond what tests show; fix commit gets a four-layer Ursache block whose twin list comes from search (`grep` for `message_id ==`, `request.message_id`, `message_content(`, `compare_exchange(.*claimed`, `.await?` in `record_message`).
