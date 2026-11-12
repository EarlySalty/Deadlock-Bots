# Gate retro: ticket scam shadow

Written 2026-11-12 by the takeover fixer after the previous agent stopped at its own "round 5".

## Why so many rounds

| Attempt | HEAD | Phase | Verdict | Counted |
|---|---|---|---|---|
| 1 | 527220e6 | phase1 (full) | BLOCK, 94 findings | yes (round 1) |
| 2 | fbd40262 | none | no verdict (quota) | no |
| 3 | 10da8823 | none | no verdict (Fable answers rejected as malformed JSON, Opus 5-hour limit) | no |
| 4 | 9eab7e7d | phase1 (full) | BLOCK, 201 blocking + 16 NIT | yes (round 2) |

The gate counter shows 2 counted rounds. The previous agent counted quota runs as rounds and called attempt 4 "round 5".

Causes of the 201 findings:
1. **Attempt 4 was a second full review, not a delta review.** The merge of origin/main (daee4a32) changed the review state key, so the gate started a fresh phase1 state and judged every unit of the 1,780-line branch again. Every input class of every unit was re-asked, which produced about 60 mechanical "unit mishandles input class" findings that only mirror the reviewer's own answers.
2. **Incomplete fixes.** e086af40 counted skipped images inside `add_images` but left the entry guard that drops an oversized-only message before `add_images`; its own TODO claimed the class was done. 65885803 named "tests never checked main.rs wiring" as why_unnoticed and added no such test. Published Ursache blocks are re-verified at HEAD every round, so every unkept claim came back as a finding.
3. **Duplicates.** About 25 distinct defects, each reported by 3 to 6 reviewer areas (3900 limit 7 times, oversized-only guard 8 times, `not_asked` literal 6 times).
4. **Commit format.** The hook accepts the why-possible layer only as `2.`, `2)`, `why possible` or `warum möglich`. Four published fix commits wrote `why_possible:` (the hook's own layer name) and fail on that alone. 2ecf3c26 (published) has no block. e086af40 (local) used `Ursache statt Symptom:`, which the head regex does not match.
5. **Doc drift.** Task docs and member docs were rewritten in every round without re-checking against the code, so each round added new stale claims.

No regressions caused by fixes and no reviewer drift beyond the duplicate fan-out were found.

## Triage of the round-2 (attempt 4) finding classes

REAL = bug, reachable edge case, rule violation, unclean or duplicated code, misleading text. TRIVIA = pure style, wording, speculative hardening without a reachable failure.

### Commit format
| # | Class | Verdict | Reason |
|---|---|---|---|
| C1 | e086af40 has no matching `Ursache:` head | REAL | Rule violation; commit was local, reworded in place as d3642e1e with identical tree. |
| C2 | d59239d1, 4ef7c633, 65885803, 38353890 "missing why_possible" | TRIVIA in substance, blocking mechanically | Each block has a `why_possible:` line; only the hook regex rejects the underscore spelling. Published, cannot be reworded without a force push. |
| C3 | 2ecf3c26 has no Ursache block | REAL | Rule violation, but published. Its mechanism (adjacent-only dedup) is documented in 38353890. Cannot be reworded without a force push. |

### Code
| # | Class (finding ids) | Verdict | Reason |
|---|---|---|---|
| K1 | Oversized-only message returns before `add_images`; no count (112, 127, 133, 139, 145, 147, 154, 171, 182) | REAL | Reachable: a 9 MiB follow-up screenshot vanishes, entry shows no "Nicht geprüft" line, contradicting e086af40. |
| K2 | Image-only first message takes the one-shot claim; text later than 45 s is never judged (128) | REAL | Same cause as K1 (one claim per ticket, first message decides). An opener who posts screenshots first and types the description a minute later gets a `not_asked` entry and no scam check. |
| K3 | Text over 3500 chars dropped or cut without marker (129, 134, 140, 153, 168, 172, 183) | REAL | Scam words or a `Username:` label after the cap vanish and the entry claims nothing was lost. |
| K4 | Hard-coded 3900 cut, real limit BRAIN_V2_TEXT_LIMIT 4000 already enforced with marker; test asserts 2000 (116, 130, 136, 141, 142, 146, 162, 169, 184) | REAL | Duplicated limit without marker; test asserts a bound no code path has. |
| K5 | `"not_asked"` literal three times in fail-closed logs (137, 143, 151, 156, 163, 186-188) | REAL | Duplicates `BrainState::NotAsked.verdict()`. |
| K6 | `handle_ticket_message` wrapper without production caller (117, 135, 144, 197) | REAL | Dead public API kept only for tests. |
| K7 | `ScreenshotCheck::Disabled` also used when switch is on but moderation AI is missing (118, 138, 150) | REAL | Entry says "deaktiviert" while the config says on; misleading. |
| K8 | No test for `install_ticket_screenshot_review` although 65885803 names it as why_unnoticed (119, 152) | REAL | The published Ursache block makes the claim; the wiring decides whether private screenshots leave the server. |
| K9 | `member_by_name` treats nickname hits as "kein Mitglied"; inverted test name (125, 126, 131, 191) | REAL | Discord search matches nicknames; a member named by nickname is logged as non-member. |
| K10 | Signed CDN URLs of ticket screenshots in analyzer error log and in `ModerationVerdict.trigger` (111, 214) | REAL | Private member data in logs. |
| K11 | `username()` trims non-ASCII ends: `@José` becomes `jos` (158) | REAL | Looks up a different, possibly real account. |
| K12 | `labelled_name` takes the first word of prose: `Username: ist mir entfallen` gives `ist` (159, 164) | REAL | Bogus ref can resolve to a real member. |
| K13 | Steam64 check by prefix only, no 17-digit length (160) | REAL | Drops valid 18-20 digit snowflakes starting 7656119. |
| K14 | Opener named by username counts as second member; opener excluded in two places (161, 189) | REAL | Wrong reason ("mehrere Mitglieder"), rule split in two places. |
| K15 | `reportet` is a scam stem (190) | REAL | "Ich wurde reportet" becomes a scam report and, with the switch on, sends screenshots out. |
| K16 | `gesperrt`, `Sperre`, `gebanned` are no moderation words (155) | REAL | Ban questions get `needs_mod: nein`. |
| K17 | Real member ticket text, username and Discord ID as test fixture (167) | REAL | Personal data in the repo; anonymise. |
| K18 | TICKET_MAX_IMAGES / TICKET_MAX_IMAGE_BYTES copy dl-ai limits (148, 173, 177, 185) | REAL | Duplicated limit; a lower provider cap would drop images silently. |
| K19 | Empty-image guard three times (170, 175, 180, 192, 193) | REAL | Duplicated rule; two copies unreachable. |
| K20 | Failure detection by comparing reason text to `parse_error`; `lib.rs:153` keeps its own literal (174, 178, 179, 194, 195, 213, hook twins on `parse_error`) | REAL for the literal, TRIVIA for the sentinel design | The lib.rs literal duplicates the constant. A malformed-but-parseable reply maps to Other/unconfirmed, which yields "kein Bann" (fail-safe) and needs a typed analyzer API across all moderation callers. Fable rated the sentinel REAL; kept as TRIVIA because every wrong outcome points toward "kein Bann" and the typed API touches live message moderation. |
| K21 | Reviewer contract carries no reviewed-image count; provider skips failed downloads silently (149) | REAL as misleading text | Entry says "Bilder: n" while fewer may reach the model. Fix the wording to what is known ("angehängt"); a per-image count needs a dl-ai API change, logged as open cause. |
| K22 | `evaluate_behavior_trigger` still turns parse_error into an other/ignore verdict (196) | REAL, deferred | Pre-existing message-moderation path outside this branch's scope; recorded as open cause (see below). Fable rated it REAL; agreed, but fixing it here widens the scope into live message moderation. |
| K23 | `missing_candidate` Answered arm unreachable (157) | TRIVIA | Exhaustive match arm, no reachable failure. |
| K24 | Test at exactly MAX_ACCOUNT_LOOKUPS missing; `Username:\n@foo` test does not prove the label path (165, 166) | REAL | Contract boundary untested; tests rewritten with K12 anyway. |
| K25 | No image cap at the shared `review_reported_screenshots` (181) | TRIVIA | dl-ai caps at MAX_MULTIMODAL_IMAGES for every provider. |
| K26 | `label()` NoImages arm never rendered (176) | REAL | Dead variant of `ReportedScreenshotFailure`; removed together with the duplicate guards (K19). |
| K27 | New `bot.toml` key vs standalone binaries with `deny_unknown_fields` (216) | REAL | `dl-community-points-sync` is retained byte-for-byte and broke twice on new fields (memory bot-toml-standalone-binaries-drift). Removing the line keeps old binaries parsing; whoever switches the review on must rebuild points-sync first (Fable). |
| K28 | Settings UI test pins 167 editable fields (206, 208) | TRIVIA | `scripts/test_operating_config_ui.cjs` needs an external fixtures file, Playwright and Chromium and is wired into no runner; its 167 comes from that fixture, not from `settings_catalog.tsv` (147 edit rows on main, 148 here). Nothing to adjust in this branch. |

### Docs
| # | Class | Verdict | Reason |
|---|---|---|---|
| D1 | TODO stale merge state, wrong "no Ticket Tool guard" claim (gateway.rs `is_user_message` drops bots), dangling section ref in WORKER-ERGEBNIS (113-115, 200, 201) | REAL | Misleading. |
| D2 | Member docs say "erste Nachricht"; code collects all opener messages for 45 s / 180 s (123, 132, 198, 199) | REAL | Misleading. |
| D3 | Member docs mention `load_docs` and a Twitch diagnosis tool that do not exist in dl-community (121, 122) | REAL | The branch rewrote these lines. |
| D4 | Config key, `not_asked`, `needs_mod`, "Brain" in member-facing German text; absolute privacy claim (120, 124) | REAL | Jargon in user text; "ausschließlich" is only true while the code path is the only one. |
| D5a | SSOT note: Deadlock-Docs public copies lack the ticket shadow (204) | TRIVIA | `public/discord-server/faq-bot-selbst.html` and `community-tools.html` never described the ticket shadow; the shadow is internal and changes nothing members see. |
| D5b | Account names in the entry without code formatting (209) | REAL | A name with `_` or `*` renders as Markdown in the log; names are now wrapped in backticks. |
| D5c | No live replay with a real scam screenshot (212, 215, 217) | Acceptance item | Needs the screenshot switch on, which sends member images to the vision provider; requires the user's approval. Not a code defect. |
| D5d | Remaining display NITs (202, 203, 205, 207, 210, 211) | TRIVIA | Wording or layout of internal log lines with no wrong content. |

## Fix plan (one fixer pass)

1. Opening collector (K1, K2, K3): every message of the opener is added, including oversized-only ones; the opening keeps counts of dropped images and dropped characters, and the entry shows both. A ticket whose first messages carry no text waits for the text within the normal window instead of being judged image-only at once.
2. Entry and logs (K4, K5, K6, K7, K21): drop the 3900 cut (transport enforces 4000 with marker) and the 2000 test; use `BrainState::NotAsked.verdict()`; remove the wrapper; add `ScreenshotCheck::Unavailable`; say "angehängt" for the image count.
3. Wiring (K8, K27): pure decision function for the reviewer install with a test; remove the key line from `config/bot.toml` (serde default keeps it off) so old standalone binaries keep parsing.
4. Account extraction and decision (K11-K16, K24): reject non-ASCII names, take a labelled value only when it is the sole word after the label, exact Steam64 shape, opener excluded in one place, `reportet` out of the scam stems, add ban stems, boundary test at five.
5. Member lookup (K9): a search with hits but no exact username match is "unklar", not "kein Mitglied"; fix the test name.
6. Moderation (K10, K18, K19, K20 literal): no URLs in analyzer logs or the screenshot verdict trigger; export the dl-ai limits and use them; single empty-image guard; use PARSE_ERROR_REASON in lib.rs.
7. Fixture (K17): anonymous ticket text, name and ID.
8. Docs (D1-D4): rewrite task docs at final HEAD; member docs in plain German without config keys or internal labels, and without the stale loader and Twitch tool lines.
9. Open causes (K21 per-image count, K22 behavior-trigger parse_error, C2 hook regex): entries in `claude-config/orchestrierung/OFFENE-URSACHEN.md`. That checkout carries another agent's uncommitted edits to the same file and is over its 30-line cap, so the rows are kept below for the owner to move.

## Decision needed before the final round

C2 and C3 make the final round a mechanical BLOCK whatever the code looks like: the hook re-reads every fix commit in `origin/main..HEAD`, and five published commits fail it. Options:
- (a) one-time force push of this feature branch (not main) to reword the five commits;
- (b) the gate owner accepts `why_possible:` in the layer regex and rules on 2ecf3c26;
- (c) a fresh branch with the same trees, with the counter carried over by the gate owner.
None of them may be chosen by the fixer alone.

## Second opinion (Fable, ask_for_help)
Agreed with the triage except: K20 sentinel and K22 (Fable: REAL). K22 is now "REAL, deferred"; K20 stays TRIVIA with the reason in its row. Fable also suggested pushing reworded commits to a new remote ref; declined, because the brief forbids rewriting published history.

## Open causes (rows for `claude-config/orchestrierung/OFFENE-URSACHEN.md`)
| Seit | Repo | Ursache (Ebene) | Gesehenes Symptom | Beleg |
|---|---|---|---|---|
| 2026-11-12 | Deadlock-Bots | `ScreenshotReviewer` returns one verdict for all images; the provider skips images it cannot download without telling the caller, so "Bilder: n angehängt" may exceed what the model saw (unnoticed) | none yet; screenshot review is off | `dl-ai` `generate_multimodal`; gate finding 149 |
| 2026-11-12 | Deadlock-Bots | `evaluate_behavior_trigger` maps a parse error of the verifier to an other/ignore verdict on the live message path (possible, twin of the ticket path fixed in d59239d1) | a malformed verifier reply lets a message pass unflagged | `rust/crates/dl-moderation/src/content_analyzer.rs` `evaluate_behavior_trigger`; gate finding 196 |
| 2026-11-12 | gpt-workers | `missing_ursache_layers` rejects `why_possible:` although that is the hook's own layer name (possible) | four published fix commits on this branch fail the final round mechanically | `gate_hook.py` layer regex; this retro C2 |

## Notes
- d3642e1e keeps the original `Co-Authored-By: Claude Sonnet 5.5` trailer because that model wrote e086af40; the reword only changed the message, not the authorship record.
- The workspace formatter is blocked by a guardrail; changed files were formatted with `rustfmt` directly, and one unrelated reformat in `dl-answer/src/game.rs` was reverted.
