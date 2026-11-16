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
| NIT 111, 112, 118, 132, 135 (live config, external branch, OFFENE-URSACHEN claims) | Facts outside the clone; checked at deploy time. The README no longer states live numbers; it states the derivation from the configured Brain timeout. |
| NIT 113 (release hint on transport error may double a hint) | Speculative; one extra hint within 24 h, no reachable harm. |
| NIT 115, 117, 119, 141 (rollback hazard of new fields on main binary) | Only on a rollback. README "Rollback" names the concrete step: before a main binary runs, clear or rewrite the rows that carry `held_answers`, `reposts`, `earlier` or (since fix pass 8) an outbox `earlier_targets`. Correction in round 6: the outbox `answer` field already exists on main. |
| NIT 116 | Same as I, fixed there. |
| NIT 121 (`retry_held_answer` empty-queue branch) | Defensive, harmless. |
| NIT 122, 124 (test `LEASE` literal) | Test parameter only; kept as a literal (see 4.7). |
| NIT 123 (privacy eraser does not list the namespace) | Real, and partly new on this branch (`held_answers`, outbox `answer` carry member text; Fable). Not half-fixed here: the eraser gap covers `friend_code` on main too. Filed in claude-config OFFENE-URSACHEN (6b25202), README "Privacy". Fixed in fix pass 8: the eraser and the export cover `invite_lounge:requests` and `invite_lounge:cooldown` (section 7). |
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

1. **A.** Any changed code on a `Pending` or `Dispatching` request restarts it at the new message (`restart_for_new_code`) with a fresh status and clears the claim token; `finish_dispatch` matches the token, so the late finish of the old call is a no-op. The restart is direct only when the new message is a direct request; a plain new code needs check-in and "nein". A non-request message changes code or request only when an open request lives in its channel (`open_request_in`). `dispatch_active` is the one "active" predicate for the replace rule.
2. **C/D/J/K.** `repost: Option<Repost>` replaced by `reposts` and `earlier` (last five each, no duplicates). Staff checks match any id; `on_gone` follows the newest repost in every phase; `GoneOutcome::Deferred` and the dead `drop_deleted` guard are gone.
3. **B.** `reread_waiting_request` hands a 404 to `drop_deleted`.
4. **E.** `release_hint` reloads, restores each hint field that still holds its claimed value, and retries on conflict.
5. **F.** `hint_in_history` runs in the dispatch task; an error there means "no earlier hint" and never drops the request. Class "Discord REST before the state write in `record_message`": no member left; the remaining `?` calls there are Postgres store calls (`actions_since`, `load`, `last_hint_at`, `known_code`, `record_answer`), and `channel_category` reads the cache.
6. **G.** `reply_body` sets `fail_if_not_exists: true`.
7. **H.** No new numbers. README states each bound's derivation and overrun mode (cut-off, lease, active window). The test `LEASE` literal stays (test parameter, NIT 122).
8. **I.** `waiting_users(created_since)` filters by request age in SQL.
9. **L/M/N.** `run_recorded` shared by `spawn_recorded`, the tick and the test helpers `handle_message` and `poll_due`; `queue_has_room`, `dispatched_since_request` and `accepts_repost` helpers; `note_repost`, `queue_held_answer` and `forget_request` return nothing; `dispatch_request` reads the clock after its load.
10. **O/P.** README rewritten without claims beyond the tests; the fix commit carries a four-layer block with a searched twin list.

New tests: `neuer_code_waehrend_des_versands_startet_die_bitte_neu`, `schlichter_neuer_code_bei_direkter_bitte_laedt_nicht_von_selbst`, `zahl_in_einem_anderen_kanal_aendert_die_offene_bitte_nicht`, `raumbitte_im_versand_mit_neuem_code_laedt_nicht_von_selbst`, `direkte_bitte_mit_neuem_code_im_laufenden_versand_bleibt_direkt`, `hinweisfreigabe_gibt_die_sperrzeit_auch_nach_neuer_bitte_frei`, `hilfe_an_einer_kopie_der_bitte_zaehlt`, `hilfe_an_der_geloeschten_ersten_bitte_zaehlt_nach_dem_folgen`, `geloeschte_kopien_werden_der_reihe_nach_verfolgt_und_zuletzt_verworfen`, `kopien_der_bitte_bleiben_begrenzt_und_ohne_doppelte`, `geloeschte_wartende_bitte_wird_beim_abgleich_still_verworfen`, `hinweisfreigabe_haelt_trotz_zwischenzeitlicher_schreibung`, `verlaufsfehler_vor_dem_hinweis_verliert_die_bitte_nicht`, `abgelaufene_leihe_wendet_kein_zweites_urteil_an`, `broker_antwort_ohne_zielnachricht_wird_nicht_ohne_bezug_gepostet`, `geloeschte_bitte_im_versand_folgt_der_neu_geposteten` (replaces the Deferred test).

Independent Opus review of 86ae2201, now c70faf7d (one pass, before the final gate): one blocking finding, fixed in the follow-up commit. A changed code from a non-request message in any lounge channel restarted the request there, so a match number in #frag-die-community invited a direct request's member at once and moved a room check-in under an unrelated message. On main the same mechanism moved only a `Pending` room request. Class fixed: a non-request message changes code or request only in the request's channel, for every open phase. Also fixed: the restart takes `direct` from the new message, so a stray number in the request's own channel never invites without a "nein"; README overclaims (always, test credit for the SQL window, finish no-op reason); the reviewer's NITs on `poll_due` (L twin), the dropped `note_repost` return (M) and `release_hint` after a new request (E).



Checks: dl-community 355 passed (4 DB-harness tests need `CENTRAL_TEST_DSN`), invite lounge with Postgres on ionos 134 passed; dl-discord, dl-broker, dl-bridges green; dl-bot only the 8 DB-harness `build_publisher` tests fail without `CENTRAL_TEST_DSN`. fmt clean, no clippy warning on branch lines.

## 5. Round 5 (final round) and the user decision of 2026-11-16

Round 5 ended BLOCK: 40 blocking lines (26 `[hook]` restatements) and 15 NITs (gate-r5.log). The user decided the friend-code rule ("FIX IT") and approved at most two more rounds (6 and 7) with one fix between them.

### Classification of the real round-5 findings

| Finding | Cause | Class | Fix |
|---|---|---|---|
| corrected code after `InvalidCode` dropped | regression (856e7939, now 204dec89) | a guard that narrows accepted inputs must list the inputs it now refuses | `awaits_corrected_code` keeps that request open for a correction; `restart_for_new_code` restarts it as a room request with the new code |
| stale `WaitingForCode` completed by a stray number | instance | "is the request open" written without the request window | one predicate `open_request` (phase, `is_fresh`, exact channel) for plain codes and for `continuing_direct`, the twin that let a stale direct request make a new room request direct |
| `record_history` stores any number as `friend_code` | instance | every `friend_code` writer that stores a code no request owns | backfill writes no code; a new request takes the message code, its own open request's code, the linked account, or none; `forget_request` clears it |
| `restart_at` carries `direct` the caller overwrites | regression (35e0a042, now ff234601) | rule kept in the caller instead of the shared function | `Request::restart_at` takes `direct` |
| `note_repost` guard in the callers | regression (856e7939) | same | `note_repost` checks `accepts_repost` and returns whether it noted |
| `REQUEST_COPY_LIMIT = 5` underived | spec | bound without stated reason and overrun mode | README states purpose and effect on staff detection |
| min-id bound computed twice | instance | one bound computed in two places | `Request::first_id`, passed to `answers_request` |
| README credits tests for paths they do not run | doc overclaim | test claims beyond the assertions | README states what each test asserts; `korrigierter_code_nach_ungueltigem_code_gehoert_zur_bitte` runs restart, check-in, "nein" and the invite |
| list formatting | NIT | | fixed |
| hook: heading `Ursache (four layers):` | spec | hook expects `Ursache:` | message-only rewrite below |

Decision 4 ("ja" ends the request, no "nicht angenommen") had one path no finding named: a "ja" held through a Brain error let `follow_steam` settle `NotAccepted` and `deliver_outbox` send it. `answer_pending` now pauses the Steam follow-up, the invite and notice delivery while an answer is held. Alp replay: `alps_verlauf_ohne_antwort_keine_anfrage_und_nach_ja_keine_absage`.

### Message-only rewrite (lease-protected, own unmerged branch)

`git filter-branch --msg-filter` replaced `Ursache (four layers):` with `Ursache:`, updated with `--force-with-lease` against 2b339d08. Every tree is unchanged:

| Old commit | New commit | Tree (both) |
|---|---|---|
| 86ae2201 | c70faf7d | 7ba5f6c5 |
| c64783ea | ffb4f78d | 638fbdad |
| 35e0a042 | ff234601 | 12d66848 |
| 856e7939 | 204dec89 | 17f60bc5 |
| 2b339d08 | e6c0c494 | 84418eef |

Older references to the old SHAs in this folder mean the new commit with the same tree.

### Checks of fix pass 6
With Postgres on ionos (`test-db-run`): invite lounge 145 passed, dl-bot 374 passed. Without a DSN: dl-community 366 passed (the 4 failures are DB-harness tests), dl-bridges 58, dl-discord 74. `cargo fmt --check -p dl-community` clean; clippy reports nothing in the lounge files.


## 6. Lost run 6 on 49a80e2f and fix pass 7

The run on 49a80e2f gave no verdict (Opus reviewer quota; its log is kept outside git as `gate-r6-lost.log`, the leads are in the table below); it is not a round and its grant stayed unspent. The reviewer's notes named four leads and two NITs. Each was checked against the code:

| Lead | Holds | Cause | Fix |
|---|---|---|---|
| 1. A "ja" held while the invite call runs is dropped once `finish_dispatch` closes the request, and the closing notice goes out | yes, for the closing results (`Invite`, `AlreadyInvited`, `InvalidCode`, `Rejected`, none); the `Wait` results return to `Pending` | spec gap: "ja ends it" was checked only for open phases | `held_answer_open` keeps an already held answer judgeable on the fresh `Attempted` request in its channel (`judge_held`, `store_answer` only); the notice waits while the answer is held and a "ja" purges it. `finish_dispatch` is the only closer that can leave answers held. Twin: a "ja" removes the `InvalidCode` mark, so a later number does not restart the request |
| 2. `accepts_repost` checks "open" without the window; the open rule written in several places | yes: a stale `Dispatching` room request swallowed a new same-code room request as a repost | instance of the round-5 class "open written per site" | `open_request` is built from `fresh_request` and `request_in`; `answer_open`, `accepts_repost`, `take_edited_code`, `awaits_corrected_code`, `held_answer_open` use them. `note_ignored_answer` (no channel, it logs other channels) and `restart_for_new_code` (no window, a new code is a new request) keep a phase check with a stated reason |
| 3. `awaits_corrected_code` does not check the channel | yes: a room request with a new code in another channel restarted the corrected request there | the check sat in one caller (`takes_plain_code`), not in the predicate | `awaits_corrected_code` takes the channel |
| 4. `known_code` before the load drops the request on a DB error | yes; the same holds for `last_hint_at` on a first row (main had both, `known_code` only for direct asks) | an enrichment lookup was fatal | both lookups log and fall back (no linked code, no earlier hint) |
| NIT test hand-off tests:3723 | yes | `linked` passed for a message with its own code | `None` |
| NIT README:152 OFFENE-URSACHEN claim | yes, the row did not exist | | rows added in claude-config c602b3c; README names each row |

Unfinished checks of the lost run:
- `ACTIVE_DISPATCH_MILLIS` = 2 x `INVITE_FORWARD_TIMEOUT_SECS`, derived in code; the bridge call is hard bounded by the reqwest per-request timeout in `SteamBotClient::post_event`.
- `Dispatching` after a crash: predates the branch, exits listed in README "Limits to know", row in OFFENE-URSACHEN; no recovery mechanism in this pass.
- `friend_code` writers: unchanged list (README "Friend-code rule"), all six sites checked.
- The `graphify-first` hook blocks `git grep` in the reviewer sandbox: row in OFFENE-URSACHEN (c602b3c). The fix commit body lists the callers with line numbers instead.

New tests: `gehaltenes_ja_waehrend_des_versands_schliesst_die_bitte_ohne_abschlussmeldung`, `ja_nach_ungueltigem_code_im_versand_laesst_keine_korrektur_mehr_zu`, `raumbitte_mit_neuem_code_in_anderem_kanal_korrigiert_den_ungueltigen_code_nicht`, `unlesbares_steam_konto_verliert_die_bitte_nicht`, `veraltete_bitte_im_versand_nimmt_keine_kopie_auf`. The first four fail with the old predicates (checked by reverting them). Checks: dl-community 371 passed, the same 4 DB-harness tests fail without `CENTRAL_TEST_DSN`; invite lounge 145 tests passed (without the Postgres tests); fmt clean; clippy reports nothing in the lounge files. The Postgres tests were not rerun: no SQL changed.

## 7. Round 6 on c43470fa and fix pass 8

Round 6 gave a verdict (BLOCK) and counts. Real findings, by cause (gate-learning):

| Findings | Cause | Class | Fix |
|---|---|---|---|
| 57, 58 hint never sent for a code-less ask after a hinted and closed request | regression of the round-5 fix (stored code no longer reused) | the hint is owed once per request, every path that ends a request must leave the next one hintable | `update_state` clears `hint_sent` and skips the 24 h cooldown when the new request follows a closed, expired or stale one (`follows_closed`); `forget_request` clears both hint fields; the cooldown applies only to a member with no stored request (history backfill) |
| 59, 109 room request after a closed or stale request dropped | instance of "open written per site without the window" | every replace decision uses the open rule | `is_closed` (not an open phase, or outside the window) makes the replace branch take the new room request |
| 65, 71, 76, 116 outbox entries of a replaced or restarted anchor still delivered | untested path | every anchor change drops the old anchor's entries | `update_state` drops entries of all old ids whenever the anchor changes (replace, restart, plain-code completion) |
| 69, 75, 87 "asked" counted from the claim, not the post; a rejected check-in stayed "asked" | spec gap | "asked" means the question is visible | `asked(state)`: claimed and no `CheckIn` entry waiting; `unclaim_check_in` on broker rejection or give-up; used by `apply_answer`, `takes_answer_code`, `record_answer`, `update_state` |
| 70 `CheckIn::Skip` left an inert `Pending` request | regression (the 60 min exit was removed) | every "a human answered" result closes like `close_handled` | `Skip` finishes the request without a notice and purges its outbox |
| 72 intake refused a new answer on a request closed while an answer was held | instance of the round-6 fix (judge side only) | judge and intake decide alike for a closed request with held answers | `intake_open` in `record_answer` |
| 82 a "ja" in `WaitingForCode` was dropped | spec gap | "ja at any point ends the request" | `takes_answers` covers every open phase; request messages and codes in `WaitingForCode` still go to `update_state` (`waits_for_code`) |
| 67 hint posted after the request no longer waited for its code | untested path | post-time effects re-check the state | `hint_owed` before the Brain call and before the post |
| 68 `allows_actions` DB error dropped the message | instance of the round-6 class "fatal lookup in record_message" | write-once gate value read once | cached in `actions_since` (`OnceLock`) after the first successful read; before that first read an error still drops the message and is logged |
| 88, 113 retry dedup looked only at the new anchor | untested path | dedup checks every target the notice could have landed on | `OutboxEntry::follow` keeps `earlier_targets`; `bot_replied_since` takes all targets |
| 77 privacy eraser and export missed `invite_lounge:*` | critic-miss of an earlier round (filed instead of fixed) | every personal-data writer is wired into erase and export | `privacy.rs` erase (`kv_invite_lounge`) and export (`invite_lounge`) |
| 83, 89 FAQ f5 promised an automatic invite | instance: a reader of the changed behaviour | user-facing text follows behaviour | `assets/faq_texts.toml` f5 rewritten |
| 84, 99, 105, 115 `reread_waiting_request` repeated the open rule | instance | | takes its request from `open_request` |
| 66 dead guard in `restart_for_new_code` | | | removed |
| 73 re-read cadence reused `RETRY_SECONDS` | | | `REREAD_SECONDS` with derivation in README |
| 94, 101 `is_http_404` doc comment narrower than the code | | | comment removed |
| 95 modglue panel edit matched "10008" in the error text | twin outside the branch; serenity 0.12.5 `HttpError` Display prints only the message, so the check never matched | typed status check | 404 status match like the other modglue sites |
| 96 tempvoice `looks_missing_message` string match | twin outside the branch; its port returns strings, not typed errors | | OFFENE-URSACHEN row (claude-config), not changed here |
| 93 a gone reply target is logged as a Discord error with 502 | design of the broker mapping, outside the lounge | | OFFENE-URSACHEN row; README states the gap |
| 80, 85, 90, 91 tests that did not run the rule they name | untested path | | `zweiter_dispatcher_verliert_den_anspruch_ohne_zweiten_versuch` (second dispatcher loses the CAS), reachable `(Pending, None)` fixture in `spaetes_ja_schliesst_die_bitte_in_jedem_offenen_zustand`, `hilfe_zwischen_frueherer_bitte_und_anker_zaehlt`; the Alp replay no longer claims the NotAccepted rule |
| 79 "no attempt cap" credited to one-retry tests | | | `gehaltene_antwort_hat_keine_versuchsgrenze` (twelve errors) |
| 60-64, 74, 78, 81, 86, 92, 97, 98, 100 README and TODO statements | | | rewritten against HEAD; "Live check" marked as pending |

Renamed tests: `gehaltenes_nein_folgt_der_neu_geposteten_bitte` is now `nein_vor_der_sichtbaren_nachfrage_laedt_nicht_ein_und_folgt_der_neu_geposteten_bitte` (the held "nein" no longer invites, the next one does); `raumbitte_mit_neuem_code_in_anderem_kanal_korrigiert_den_ungueltigen_code_nicht` is now `raumbitte_in_anderem_kanal_nach_ungueltigem_code_ist_eine_neue_bitte`; `raumbitte_nach_abgeschlossener_bitte_aendert_den_code_nicht` is now `raumbitte_nach_abgeschlossener_bitte_ist_eine_neue_bitte`.

New tests: `neue_bitte_ohne_code_nach_abgeschlossener_bitte_bekommt_den_hinweis`, `neue_bitte_ohne_code_nach_geloeschter_bitte_bekommt_den_hinweis`, `ja_waehrend_die_bitte_auf_den_code_wartet_beendet_sie_still`, `neue_bitte_verwirft_meldungen_der_alten_bitte`, `abgelehnte_oder_aufgegebene_nachfrage_gilt_nicht_als_gefragt`, `antwort_nach_dem_abschluss_waehrend_eine_antwort_wartet_wird_angenommen`, `hinweis_entfaellt_wenn_der_code_schon_da_ist`, `zustellpruefung_findet_die_antwort_an_der_frueheren_bitte`, `hilfe_zwischen_frueherer_bitte_und_anker_zaehlt`, `zweiter_dispatcher_verliert_den_anspruch_ohne_zweiten_versuch`, `gehaltene_antwort_hat_keine_versuchsgrenze`, `hilfe_kurz_vor_der_nachfrage_schliesst_die_bitte`, `code_fuer_die_wartende_bitte_gilt_nicht_als_ignorierte_antwort`. Mutation check: reverting each of the hint reset, the `forget_request` reset, `follows_closed` in the replace rule, the outbox purge, the visible-check-in rule, the `Skip` close, the `WaitingForCode` answers and the code guard in `record_answer` makes its test fail; removing only the first `hint_owed` check keeps `hinweis_entfaellt_wenn_der_code_schon_da_ist` green because the second check before the post still holds.

Checks of fix pass 8: dl-community 383 passed, the same 4 DB-harness tests fail without `CENTRAL_TEST_DSN`; invite lounge 157 tests passed; dl-discord 74 passed; fmt clean; clippy reports nothing in the changed files. Postgres tests (including the extended privacy erase test) were not run: they need `CENTRAL_TEST_DSN`.
