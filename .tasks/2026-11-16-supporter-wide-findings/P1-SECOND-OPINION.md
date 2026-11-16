# P1 second opinion (Fable, 2026-11-16)

Branch feat/supporter-p1-20261111, HEAD c6e0b6c9, round 4 BLOCK. Read-only review of p1-r4-findings.txt, gate_hook.py, the review state and the branch history. Nothing was changed.

## Short answer

1. The branch does not converge because its surface grows faster than findings get fixed. The cross-crate HTTP, timeout, token and bounds rework is the driver. 2e410dde is only a small part of it.
2. Neither "fix everything as-is" nor "revert commits on top" reaches ALLOW with this gate. The only path I see that converges is a rebuild of the branch to the supporter-only footprint with a clean history. That conflicts with two written rules, so it is a user decision, not something the orchestrator may pick alone.
3. Of 566 BLOCKING lines, about 20 are correctness defects. 12 of those sit in supporter code, the rest are regressions the rework itself introduced and vanish with it.

## 1. Why it does not converge

Growth against origin/main:

| Head | Date | Files | Lines |
|---|---|---|---|
| 80499256 (feature complete, tier picker) | 11-12 | 14 | +2708 |
| ee9b4c45 | 11-13 | 45 | +5052 |
| 68e46b32 | 11-13 | 59 | +5998 |
| fd21d9b3 (round 3: 516 BLOCKING) | 11-15 | 66 | +7003 |
| c6e0b6c9 (round 4: 566 BLOCKING) | 11-15 | 97 | +8567 |

The jump comes from ee9b4c45 (35 files, "bound every touched HTTP client by config"), then b98b88e2, f9b8986a, 6b2279e7 and above all ecc82ea2 (55 files). 2e410dde (12 files, +208) added config keys for evidence downloads, tierlist and broker and is a late follower, not the origin.

Where the 679 round-4 lines sit:

| Bucket | Files | Lines | BLOCKING |
|---|---|---|---|
| Supporter feature files | 14 | 136 | 92 |
| Shared files the feature must touch (serversync.rs, main.rs, runtime_config.rs, settings_catalog, steam.rs) | 6 | 143 | 122 |
| Rework-only files in the diff | 55 | 341 | 294 |
| Files outside the diff (twins) | 24 | 59 | 58 |

So 62% of the blockers are in code the supporter goal does not need, and most of the "shared" bucket is rework too (44 of them in the runtime_config bounds work).

Three mechanisms in gate_hook.py make this self-feeding:

- **Fix commits widen the search to the whole repo.** Every commit whose subject starts with `fix` gets its Ursache block verified against HEAD, and every twin the reviewer finds anywhere in the repo becomes a hook BLOCKING line (`derived_root_cause_findings`, line 6313). The branch has about 45 fix commits with claims like "every timeout", "one source per value", "every Discord REST call". The repo holds at least 61 other timeout literals (timeout-literal-twins-20261115.txt). Each such claim is false by construction until the whole repo is reworked. 125 of the hook lines are these twins.
- **Each fixer pass adds files, which forces a full round.** `choose_phase` (line 4205) returns to an exhaustive phase1 when more than 3 new files appear or the diff grows past 1.5x. Round 4 was phase1 (state 554819bda82dac41.json). The state file does not keep round 3, but its 616 lines over the whole diff look the same. The branch grew by 31 files between the two rounds, so it could not get the cheap follow-up round where new findings outside the fix are demoted to NIT.
- **A full round has a noise floor per file.** The prompt says "assume the author was sloppy" and "when in doubt, BLOCKING low". The hook turns every `ok: false` on any of 7 input classes per unit into a BLOCKING line (224 lines), including nonsense such as URSACHE.md "mishandles concurrency". Round 3 gave 7.8 blockers per file, round 4 gave 5.8. At 97 files a zero is not reachable.

The 566 lines are far fewer problems. By my reading (an estimate, not a count) the 212 reviewer BLOCKING lines cover about 110 locations and roughly 70 classes: twitch.rs:419 header literal 10 times, serversync.rs:2397 7 times, privacy_gate.sql:17 5 times. Only 2 of them are not weighted low (vanity.rs:374, privacy_gate.sql:17); the brief's "115 not marked low" is those 2 plus 113 NIT lines.

The 349 hook lines are derived from the same reviewer answers. The 125 twin lines match reviewer findings I read. The 224 input-class lines carry no reason, and the raw reviewer answers are not in gate-p1-r4b.log, so they are unexamined. On the supporter files they cluster the way one judgement inherited by nested units would (links.rs max_overflow on 15 units around the tier row and URL limit; store.rs null_missing on 8 units around the 42P01 mapping; support_publish.rs stale_state around the stored goal). One cluster has no matching reviewer finding: error_path on links.rs `steam_link`, `checkout`, `tip`, `portal` (lines 202 to 228). The fixer must read those paths itself.

## 2. Path to ALLOW

**(a) Fix every finding as-is: will not converge.** The out-of-diff twins alone are 24 new files (dl-ai, dl-mcp, dl-verbinder, dl-token-secrets, dl-answer, visual_brain, scrimglue, faq, reaction_roles). Touching them triggers the next exhaustive phase1 over about 120 files, with the same floor. Several real findings exist only because the rework changed behaviour in crates nobody asked to change (uploads, coaching, tournament publish, admin config page).

**(b) Revert commits on top of the branch: will not converge either.** I checked this against the hook:

- The reverted fix commits stay in `base..head`. The reviewer must confirm for each that "the mechanism is gone at HEAD". After a revert it is back, so each one yields a root-cause BLOCK plus its twins.
- The next round would be phase2. Its required units are everything the revert delta touches, so the reviewer runs the input-class check on about 70 files that are identical to main, and those derived findings are not scope-limited.
- Commit-level reverts are not possible anyway. ee9b4c45 and ecc82ea2 mix rework with supporter changes (drop Plus, panel texts, store.rs, locks.rs).

**(c) What converges: rebuild to the supporter footprint with a clean history.** From origin/main, take the supporter content of c6e0b6c9 file by file (list below), as one or a few `feat(supporter)` commits. Corrections to code that was never on main are part of the feature, not fixes, so no fix subjects and no repo-wide twin search. Expected size: about 20 files, about +3500 lines. Then fix the in-scope findings from section 3 and run one exhaustive round.

This bounds the surface; it does not promise ALLOW in one round. The 14 feature files carry 92 BLOCKING lines today, about 6 per file, so expect the first round on the rebuilt branch to BLOCK with a few dozen lines. The difference is that the follow-up can then be a phase2 round: the fixer must add at most 3 new files, keep the diff under 1.5x and stay inside the code it changes, so new findings elsewhere are demoted to NIT. Fix commits in that pass bring the twin rule back, so each Ursache block must claim only what the commit does and list its twins inside the supporter files.

This needs an explicit user decision because it conflicts with:

- "No force push" (the exception covers message-only rewrites, this changes trees), or alternatively
- "Do not change branches to evade the cap" if it is done as a new branch.

It is also a deviation from the literal order "fix all". My view: the cap rule exists to stop agents from rerunning the gate, and here the round counter can stay as it is (the user already lifted the cap with "until finished"). Keep c6e0b6c9 as `backup/supporter-p1-wide-c6e0b6c9` so the rework is not lost; it can come back later as its own package with its own goal.

If the user refuses (c), the honest report is that (a) has no foreseeable end, not that it needs "a few more rounds".

### Keep list (content from c6e0b6c9)

The list is indicative. The supporter files at c6e0b6c9 do not compile against main as they are: they import `dl_bridges::http::{HttpLimits, internal_client}`, `dl_core::internal_tokens`, `dl_broker::payload` button limits (not on main) and `crate::guild_members::fetch_all` (branch-new). The rebuild has to adapt those call sites; see "Decisions inside the feature".

Whole files, with those imports adapted:
- `rust/bin/dl-bot/src/supporter.rs`, `supporter/links.rs`, `supporter/roles.rs`, `supporter/store.rs`
- `rust/bin/dl-bot/src/serversync/support_publish.rs`, `assets/support_texts.toml`
- migrations `20261111120000_supporter_sources`, `20261112120000_supporter_sources_erasure`, `20261113120000_supporter_sources_privacy_gate`, `20261115120000_plus_billing_events_subject_idx` (P2 depends on it)
- tests `supporter_sources_privacy_gate.rs`, `supporter_sources_twitch_rows.rs`, the `fresh_migrations_schema.rs` addition
- `dl-central-db/src/locks.rs` plus its export in `lib.rs` (store.rs uses `lock_users_privacy`)
- `dl-community/src/privacy.rs`: the `supporter_sources` TableSpec and its test only
- `.tasks/2026-11-11-supporter-panel/dry-run.md`, `.tasks/2026-11-08-support-panel/README.md`

Supporter hunks only:
- `dl-bot/src/main.rs`: supporter wiring (8 of 199 changed lines mention it)
- `dl-bot/src/serversync.rs`: the branch-new parts, which are the panel claim (`PanelClaim`, 4f148a5c) and `support_funding_refresh`, plus the supporter changes to `support_apply`. `support_apply` and `store_static_v2_message_id` already exist on main and share their write path with the regelwerk, faq, welcome and router panels. Leave that shared path as on main: no Serenity deadline rework, no welcome id change.
- `dl-core/src/runtime_config.rs` and `settings_catalog.tsv`: the supporter keys and rows (about 90 lines), not the named-range rework. `settings_catalog.rs` only as far as those rows need it.
- `dl-bridges/src/steam.rs`: the supporter calls (27 lines at 80499256), not the client rebuild.

Decisions inside the feature:
- HTTP client: supporter keeps its own client bound by `supporter_request_timeout_ms`, as at 80499256, with redirects off and no proxy for the token-carrying calls. No dependency on `dl-bridges/src/http.rs`. This satisfies the mechanical hard-coded-timeout pre-check.
- Tokens: read through what main already offers (state of 783b83ef), no `dl-core/src/internal_tokens.rs`.
- Button limits: keep them local to support_publish.rs and links.rs as one constant each; no new shared home in dl-broker.
- Member listing for roles.rs: either keep `guild_members.rs` as a small supporter-owned helper (one of the allowed new files) or use main's existing member loop. Do not unify the other member loops.

### Back to origin/main (drop from the branch)

`dl-bot`: mcp.rs, mcp/self_invite.rs, modglue.rs, scrim_adapter.rs, turnierglue.rs, twitch_invite_sync.rs, vanity.rs, serversync/twitch_link.rs. Binaries: dl-community-points-sync, dl-unit-failure-notify, dl-web. Crates: dl-brain, dl-bridges (http.rs, matcher.rs, steam_operating.rs, twitch.rs), dl-broker, dl-community (coaching, invite_lounge and tests, privacy_ui, scout_privacy), dl-core (bot_config, duration, internal_tokens, operating_config), dl-dashboard, dl-discord, dl-server-as-code, dl-stats, dl-tierlist, dl-voice, dl-webcore. Also `docs/admin-betriebskatalog.md`, `tests/migration_session_timeouts_scoped.rs` (515-line SQL tokenizer, 22 findings; the supporter migrations already use SET LOCAL) and `URSACHE.md` (12 findings, almost all about the rework).

Main defects the rework found and that stay open after the cut need rows in `orchestrierung/OFFENE-URSACHEN.md` before the round, not a memory note: unbounded 429 wait in dl-bot mcp.rs and dl-mcp main.rs, the vanity directory ignoring privacy opt-out, the scrim trigger lock order (2026072411:193), the session-level lock_timeout in 20261021120000, internal tokens on redirect-following clients, the timeout literal list.

## 3. Triage

Tiers follow ABLAUF: REAL-A is correctness, data, privacy, concurrency. REAL-B is real by rule (duplicated limit, stale doc, test that cannot fail, dead code) but without a failure today. TRIVIA has no consequence.

### REAL-A in supporter scope (fix these in any path)

| Location | Defect |
|---|---|
| privacy_gate.sql:17 | Insert path waits on the privacy lock behind a row lock the same transaction holds; 40P01 against delete_user_data. Not fixable in P1 alone: the P2 writer (apply_billing_event) must take `lock_user_privacy` first, and the residual needs a committed OFFENE-URSACHEN row. 9db742e7 claims that row exists; it does not. |
| privacy_gate.sql:8 | Lock-free shortcut lets any UPDATE through that keeps the key and does not extend valid_until, so an opted-out member's monthly_cents can still be rewritten. |
| store.rs:118 | `import_manual` does not enforce its one-time contract; the marker insert result is discarded, overlapping ticks import twice. |
| store.rs:41 | Every 42P01 from any query becomes "supporter_sources fehlt", drops the cause and silences the role loop. |
| store.rs:68 | `lock_users_privacy` before the boost upsert adds a round trip per booster and prevents nothing (BEFORE INSERT trigger fires first); the commit claim is wrong. Remove or justify. |
| serversync.rs:2397 | Funding fetch failure: refresh publishes the stored goal, apply uses the configured goal. Two rules for one fallback; a goal-only change errors. |
| serversync.rs:2341, 4418 (round 3) | Funding refresh fetches before taking the panel claim. Verify at HEAD. |
| links.rs:476 with runtime_config.rs:18 | All tiers go into one action row; only a bare `5` keeps it under the Discord row limit. Tie it to the limit or split rows. |
| links.rs:26, 374 | Link URL limit compared in bytes, not characters. |
| supporter.rs:81 | TWITCH_DASHBOARD_RECEIVER serves two receivers; the supporter sender shares the token list of the master dashboard routes. Moot if tokens go back to main's sources, then re-check. |
| supporter_sources_privacy_gate.rs:271 | Test commits `CREATE ROLE twitchdash` into the shared test cluster and changes every later migration run. |
| plus_billing_events_subject_idx.sql:7 | Text indexes store the whole serialized object when the path holds an object. Check against the real Stripe payloads before deploy. |

### REAL-A caused by the rework (gone after the cut; under path (a) each must be fixed)

- adapter.rs:1436, main.rs:591: every Discord multipart upload (scrim recordings up to 50 MiB, evidence) now under the 15 s generic limit.
- modglue.rs:4561, turnierglue.rs:362-398: non-idempotent POSTs under the new timeout; duplicate coaching card, deleted tournament card with a stored id.
- mcp.rs:617-625: new bail errors are swallowed as "no permission"; exports report success with threads missing.
- duration.rs:12 and :4: NaN retry_after gives a zero wait (four immediate retries against a 429); tiny values give Duration::ZERO.
- settings_catalog.rs:348 with tsv:102, 118, 139, 140: minutes and days fields bounded by a seconds range, and the admin help documents the defect.
- coaching.rs:128, dl-web main.rs:40: internal tokens sent on clients that follow redirects and honour proxies.
- operating_config.rs:71: admin config page probe went from 3 s to 15 s.
- twitch_link.rs:43: interactive click budget went from 2.5 s to 15 s.
- runtime_config.rs:61, :64, :471; invite_lounge.rs:532; scrim_adapter.rs:41, :54; scrimglue.rs:1179: leases and waits with no margin against the now configurable timeouts.
- broker idempotency.rs:31: reader accepts values the validator rejects.
- vanity.rs:374: opted-out member with an existing row is marked absent on every sweep; dashboard totals wrong. The branch added this gate.

### Pre-existing on main, outside the goal (OFFENE-URSACHEN rows, not this branch)

serversync.rs:4672, 4369, 4914, 5058, 6194: every static panel repost writes message ids and metadata in separate statements. This path is on main and shared by five panels, so a support-only fix would leave four twins; fix it as its own package. dl-mcp main.rs:371-395 unbounded 429 wait; scrim trigger 2026072411:193; 20261021120000 session lock_timeout; twitch_link.rs:79 unchecked authorize URL; the 61 timeout literals; hand-copied cursor pagers in invite_lounge and reaction_roles.

### REAL-B (real by rule, cheap, mostly moot after the cut)

- Duplicated values: X-Internal-Token literal (twitch.rs:419 and 5 more, steam_operating.rs:143), 429 fallback in three places, TextDisplay 4000 in five publishers, custom_id 100 in dl-changelog, 131072 twice, Twitch API default base three times.
- Tests that cannot fail or miss the rule they name: links.rs:836, links.rs:922, roles.rs:309, twitch_link.rs:268, serversync.rs:12957, twitch.rs:1037, notifier main.rs:463, runtime_config.rs:1204/1321/1352, settings_catalog.rs:748/784/889. The first three and serversync.rs:12957 are supporter scope and stay to fix.
- Dead or unreachable code: http.rs:88 `current_client` and insights_sync `sync_insights` (no production caller), rename_queue.rs:30 cap, support_publish.rs:1100 guard, dl-brain re-exports.
- Stale docs and commit claims: URSACHE.md:13/46/52/53/58/81, dashboard.rs:6 "20-Sekunden-Timeout", tierlist lib.rs:35, tsv:60/61 help scope, web.rs:147 "one broker client", twitch_link.rs:3 added doc comment (no-comment rule), admin-betriebskatalog.md:5/21.
- Supporter scope, keep: support_publish.rs:1285 (three banned-word lists) and links.rs:59 (reply texts never scanned).

### TRIVIA

- All 113 NIT lines.
- Hook lines "unit mishandles input class" on documents and TSV rows (URSACHE.md, README.md, admin-betriebskatalog.md, settings_catalog.tsv). A document has no concurrency behaviour.
- Counts inside commit messages (migration_session_timeouts_scoped.rs:3/:4 "14 of 15", "six applied migrations").
- Tokenizer corner cases in a test helper (non-ASCII dollar tags, EXECUTE format of a SET).
- "Margin nobody derived" where no failing schedule is shown (notifier main.rs:44, :49).

## Notes for the fixer and the next round

- The state file is fresh until 2026-11-19 00:13 UTC. Any history rewrite or rebuild makes the next round a full phase1, which is what path (c) wants.
- Under (c) the migration 20261113120000 and the release binaries change again; rebuild dl-central-migrate and release-p1-sha256.txt from the merged head.
- P2's sibling must be re-pinned to the new P1 head, and the P2 timeout contract (12 s checkout under P1's supporter limit) re-checked.
- Gate observations, not work for this task: derived input-class findings on non-code units, and a fix-commit twin rule that searches the whole repo, belong in OFFENE-URSACHEN for the gate owner.
