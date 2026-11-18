# GATE-RETRO: Deadlock-Bots `fix/ai-gateway-refusal-reason-20261115` (package A2)

Written 2026-11-18 after round four without ALLOW. Gate file sha256 prefix `565177c4e865`, base `origin/main` 92d9654c, head 1014c510. Verdict files stay with the integrator outside the repo.

## 1. Why four rounds

| Round | Head | Blocking | Notes | What the findings were |
| --- | --- | --- | --- | --- |
| 1 | 70893c49 | many (184 ledger lines) | | New: the alarm had no delivery path, the reason was parsed out of text, status vocabulary in several copies. |
| 2 | fd2a00d9 | many (198 ledger lines) | | Incomplete fix and regression: helper and unit had no release step, pre-check could not fail, alarm latch errors, status read from hint text. |
| 3 | f0f438e4 | 34 (4 reviewer, 30 hook) | 152 | Regression of the round-2 fix (secret pipe inherited by children, release reported active while the gateway crash-loops, a legacy unit delivered) plus 26 instances of one class: a word or rule in several hand-written copies. |
| 4 | 1014c510 | 20 (1 reviewer, 19 hook) | 76 | 30 of 32 round-3 findings FIXED, 2 STILL. Reviewer finding: the README text I rewrote names the wrong unit set. The rest is the same copy class again, on lines the round-4 commits wrote or sat next to, plus commit texts that claim more than the code does. |

Honest reading:
- Rounds 1 to 3 were real defects of the change, each fix opened the next layer (alarm, then its delivery, then the installer that delivers it). The scope grew from "store the reason" to "ship and prove an alarm" because the brief demands a proven alarm.
- Round 4 is mostly my own miss: I closed the copies the gate named and searched only part of the touched lines for the next literal (`invalid_output` three lines below a line I changed, the unit name in a message next to `GATEWAY_UNIT`, the companion file names in the installer). Two commit messages claim a class closed that was not ("writers use the same names", the "known exception" about a ledger row that is never written on that path).
- Part is reviewer drift by construction: the hook's token search flags every second occurrence of a token in a touched file (for example `dl-infisical-env` in Cargo.toml, README and unit files). Those are not all one word that must be one constant. The retro separates them below.
- No round was started to see what is left; each followed a class fix and a self-review. The self-review before round 4 was too narrow.

## 2. Triage of every open finding (duplicates merged, numbers are the note numbers of gate4.txt)

REAL (to fix):
1. README.md:35-38 (blocking 1, notes 35, 45, 94): the command under "Units the installer does not restart" restarts the bot and the gateway the installer just restarted and proved. Misleading doc.
2. README.md:35 (notes 36, 45): claims `/opt/deadlock/bots/rollback` keeps the previous release; live it is a directory from 2026-10-12 and the installer never writes it. Misleading doc.
3. `bin/dl-ai-gateway/src/alarm.rs:166` (blocking 3, notes 37, 80, 96): readiness message repeats `dl-ai-gateway.service` next to `GATEWAY_UNIT`.
4. `crates/dl-ai/src/usage.rs:61` (blocking 18, note 79): `replay_pending` hard-codes the gateway unit name.
5. Installer companion names (blocking 4 to 9, notes 38, 40, 42, 48, 95): `dl-web`, `dl-infisical-env`, `BUILD-PROVENANCE.toml`, `dl-bot` typed again at :381, :416, :507-512, :526-531, :565; success line :603 and provenance keys :329 repeat the three rebuilt names; base provenance file name format written twice (:396, :538).
6. Installer :15 with :168 (blocking 10, note 41): the probe reads `GATEWAY_CONFIG`, the gateway reads the path in the delivered unit's `--config`; no test ties them.
7. Probe window (notes 39, 43): 60 s is not derived from the gateway's own pre-bind budget (helper handshake and `systemctl show`, each up to `notify_timeout_secs`), so a slow healthy start would be rolled back.
8. Installer tests (note 44): the success stub answers any path; weaker than its name.
9. `chat_provider.rs:1324`, `lib.rs:538` (blocking 14, 15, notes 53, 58, 59, 61, 68, 70, 71, 76, 77, 90): ledger code `invalid_output` still a literal next to lines moved to `FailureKind::ledger_code`.
10. `tool_chat/alarm.rs:174` (blocking 17, note 66): a row with NULL reason and status 402 or 412 becomes Class Billing with kind Unavailable; kind must follow the final reason. No test.
11. `dl-provider-refusal/src/lib.rs:302` (blocking 20, note 88) and :274 (note 89): a test literal that restates `HELPER_BINARY`, and an assert that cannot fail.
12. Helper `--contract` (notes 46, 47, 51): a credential that the send path rejects (blank token) passes; a state directory the owner cannot write (0500) passes; the test never reaches the credential read.
13. `chat_provider.rs:190` (notes 54, 60, 62, 93): a 403 the rule calls an account refusal becomes `Auth` and reads "authentication failed"; commit 1014c510 claims the hint shows the refusal.
14. `chat_backend.rs:34` (note 55): an unusable 2xx is stored with code `transport` and status 200 by the gateway, `invalid_json` or `invalid_output` by the other writers.
15. `chat_provider.rs:1265` (notes 56, 64): unbounded `response.text()` on a final non-2xx, now also for 401, 403, 429; the Fireworks path bounds the same read.
16. `chat_backend.rs:99` with `usage.rs:326` (note 72): the cached-token clamp exists in one of two writers of the same body.
17. `claude_plan.rs:252-260` (notes 73, 74, 83, 86, 91, 92): own plan-quota marker "limit reached" in front of the shared rule; reason words `http_529` and `http_429` written from text even when the stored status says otherwise (my round-4 change).
18. `usage.rs:1098` (note 82): `UsageProvider::chat_with_status` is production code nobody reaches.
19. `dl-provider-refusal/src/lib.rs:23` (note 85): the bare marker "budget" now applies to every metered provider's error message at 400 and 403; a request error that names `budget_tokens` would raise a billing alarm on its first row.
20. Commit texts (blocking 2, 13, 16, 19, note 63): claims that do not hold at HEAD ("writers use the same names"; the known-exception paragraph names a ledger row that path never writes). Fixed by making the claims true and correcting the statement in the new commit; the reviewed commits are not rewritten.
21. Test names stronger than the tests (note 87) and the tautological hint expectation (note 65): cheap to make honest.

TRIVIA (stays, with reason):
- Token twins of `dl-infisical-env`, `dl-ai-gateway.service` and `BUILD-PROVENANCE.toml` in `README.md:11/13/45`, `Cargo.toml`, `Cargo.lock`, the unit files and test fixtures (hook twin lines 137 to 157): prose, manifests and systemd files cannot take a Rust constant; the unit file is tied by the tests of findings 3 and 6. Not one word in two code copies.
- `state.rs:122` (note 50): number word next to digits in the budget sentence; style only, the reviewer says the meaning is correct.
- `chat_provider.rs:1592` (note 57): cached tokens of a MiniMax token-plan answer in Anthropic field names; the reviewer states no reader is affected; speculative.
- `FailureKind::ALL`, `RefusalKind::ALL`, `Class::ALL`, `Outcome::ALL` not tied to the enum by the compiler (notes 69, 78, 84): no reachable failure today; Rust has no variant count without a new dependency, and the round-trip tests cover every listed variant. A new variant is a reviewed code change.
- `usage::code::INVALID_JSON` used as ledger code and as reason (notes 67, 81): it is one word on purpose; two constants with the same text would be the copy the gate forbids.
- `claude_plan.rs:256` (note 75): the whole CLI error text goes into the rule; the text of an error result is the CLI's message, not model output, and without a status the rule never says billing.
- `main.rs:91` (note 49): the contract makes the Botlogs channel a start gate; checked live, the installed helper config carries exactly that channel. The example config is outside this branch.
- Helper README budget text, `dl-insights-sync.service`, wrappers without status: logged by the hook as HOOK-BACKLOG, outside the branch.

## 3. Fix plan for the REAL findings (one clean pass, new commit, then one final round)

- Docs: README command restarts only `deadlock-web-rust.service`; drop the rollback-pointer claim and say what the installer really keeps (previous release under `releases/`, automatic rollback on a failed switch).
- One name each: `GATEWAY_UNIT` in the readiness message and in `replay_pending`; installer constants for the retained companions and the provenance file, one function for the base provenance name, success line and provenance keys built from `REBUILT`.
- Installer: test that the delivered unit passes `--config GATEWAY_CONFIG`; probe window derived from `alarm.notify_timeout_secs` of the installed config (two bounded children before the bind) plus a named margin; stub answers only the health path.
- Ledger words: `FailureKind::InvalidOutput.ledger_code()` at both writers; unusable 2xx maps to `InvalidOutput` in the gateway backend.
- Alarm edge: decide the reason first, the kind from that reason; test for 402 and 412 with NULL reason.
- Helper contract: token-shape check inside the shared credential reader, `ready()` proves the owner can write the state directory; test cases that reach the credential read (empty and blank file) and a 0500 directory.
- Rule: account-refused 401/403 keeps its hint (not `Auth`); "limit reached" joins the shared plan marker; claude_plan reason follows the stored status when there is one; bare "budget" marker removed; cached-token clamp inside the one reader; bounded error body read; `UsageProvider::chat_with_status` removed.
- Tests: no restated literals, no assert that cannot fail, names that say what is tested, literal expected hint.
- Commit message states which earlier claims were wrong.

Open question for the second opinion: is anything in the TRIVIA list a real defect, and is any REAL item scope creep that should stay out of this branch?

## 4. Second opinion (Fable, medium effort, read-only, 2026-11-18)

Asked on the retro and the triage with the round-4 finding list as evidence. Result, and what the fixer pass did with it:

Moved from TRIVIA to REAL:
- The `ALL` arrays of `FailureKind`, `RefusalKind`, `Class` and `Outcome` (notes 69, 78, 84): the retro's reason was wrong, a `macro_rules!` ties the list to the enum without a new dependency. Done: `worded_enum!` in dl-provider-refusal.
- Test fixtures in touched Rust files that restate names (installer fixtures): same shape as blocking finding 20. Done: fixtures use the constants; a test ties the delivered gateway unit to them.

REAL items done differently than planned:
- Item 13: `Auth` stays the variant for 401 and 403, because an untouched file classifies route-wide faults by the text "authentication failed"; `Auth` carries an optional hint only for an account refusal.
- Item 14: an unusable 2xx maps to `InvalidOutput`; the gateway answers 502 instead of 503; no claim of one code in every writer.
- Item 19: the bare marker "budget" is removed for every provider, Anthropic included, with a regression case for `budget_tokens`.
- Item 7: one `start_budget()` next to `AlarmConfig`, enforced by the gateway, and the installer waits that budget plus a margin; a dead gateway is rolled back after the budget, not after one minute.
- Item 17: plan quota is "usage limit" or "limit reached" without a billing marker, in the shared rule; the reason word follows the stored status; one table test through both plan mappers.
- Item 15: both body reads of `send_json_with_retry` are bounded.
- Item 5: the key names of the provenance file stay; writer and reader derive them from `REBUILT`.

Further sites named by the second opinion and closed: the status-to-kind rule shared as `limit_refusal` with the two status constants, `GATEWAY_BINARY` next to `GATEWAY_UNIT` from one literal, installer doubles (unchanged check, messages, release paths, `systemctl` path), the cached-token clamp in the one reader.

Stays TRIVIA after the second opinion, with its reason: `usage::code::INVALID_JSON` used for ledger code and reason (one word on purpose), the whole CLI text passed to the rule in claude_plan (the CLI text is not JSON; the risk named by the gate went away with the "budget" marker), prose and manifest token twins, the number word in the budget sentence, MiniMax cache fields, the Botlogs channel start gate (checked live).

Kept as stated limits, not claimed as closed: 529 is Overloaded in claude_plan and Unavailable on the API-key path because the kind drives retries; reason words still differ per writer; the service-to-unit map in `usage.rs` keeps its other names from main.

Corrections of earlier commit texts are in the commit that closes these findings.

## 5. Addendum 2026-11-18: coverage of section 2, and the round that ended without a verdict

Coverage. The heading of section 2 says "every open finding". Checked by a script that reads the numbered lines of gate4.txt (96 lines: 20 BLOCKING, 76 NIT) and the numbers cited in this file: 79 lines were cited by number, 17 were not. The 17, with the item that covers each:
- Blocking 11 and 12 (`install_discord_release.rs:396` and `:538`): item 5, which names both lines without their numbers.
- Notes 21 to 34: lines the hook derives from the reviewer's per-unit answer ("unit ... mishandles input class"), no finding text of their own. Mapped by unit, not by wording: 21 (README) to items 1 and 2; 22 to 26 (helper `contract_ready`, its tests, `secrets.rs::credential`, `state.rs::ready`) to item 12; 27 to 29 (`ChatProviderError`, `upstream`) to item 13; 30 to 32 (the chat providers' response read) to item 15; 33 and 34 (`tool_chat/alarm.rs`) to item 10.
- Note 52 (`state.rs:120`, the budget sentence): the TRIVIA bullet on `state.rs:122`.

Lost round. The final round was called on b8053f13 at about 10:08 and ended without a verdict when the reviewer's account limit was reached (exit 2, branch counter unchanged at 4). Its output holds one answer the hook rejected for a malformed reference. That answer named three twins in `install_discord_release.rs`:
- REAL: `restart()` restarts the bot and the gateway, and only the gateway was proven afterwards. 662a823d closed "a returned restart counts as a running service" for the gateway only.
- REAL: the owner's runtime directory was written twice in `as_owner`, although 4a4c3d4c lists a function for it.
- REAL: `HostFiles::keep` chained its steps with `?`, the form 1014c510 removed from the rollback.

All three are closed in the commit that follows 4a4c3d4c on this branch. What the bot proof shows and what it does not is stated in that commit. The answer is cut off after the installer file, so it says nothing about the other files.

## 6. Addendum 2026-11-18: claims of the commit texts tested at HEAD

Commands run from `rust/` at f6f1a09e. "Production part" means the file up to its `#[cfg(test)]` line.

Two corrections of f6f1a09e and of section 5 above:
- Both say 1014c510 moved the rollback to "attempt every step". Wrong commit. `git log -S'fn every(' --format=%h -- bin/dl-bot/examples/install_discord_release.rs` prints `662a823d`: that commit wrote `every` and put the rollback on it. 1014c510 put `restart()` on it (its text: "`restart()` attempts both restarts and reports both errors"). `keep()` was missed by both.
- f6f1a09e describes a sweep of the other files without the command. The sweep: for each `.rs` file of `git diff --name-only origin/main...HEAD`, string literals of four or more characters that occur twice or more in the production part and at least once on a line this branch added. Result, 20 values:
  - `bin/dl-ai-gateway/src/alarm.rs`: `AI_GATEWAY_ALARM_FAILED` (two log events), `PATH` and `HOME` (environment of the two child commands).
  - `bin/dl-unit-failure-notify/src/main.rs`: `event_mode`, `devfeed_429`, `provider_refusal`, `install` (clap attribute strings, which take literals).
  - `crates/dl-ai/src/chat_provider.rs`: `LLM provider error: {}` (two `#[error]` attributes of this branch plus one from main), `minimax`, `mistral`, `Authorization`, `Bearer {}` (same count on main and at HEAD: 7, 8, 3, 3).
  - `crates/dl-ai/src/tool_chat/anthropic_http.rs`: the JSON keys `system`, `tools`, `type`, `text`, and `cache_control`, which this branch writes at two places (the last tool and the system block).
  - `crates/dl-ai/src/tool_chat/claude_plan.rs`: the JSON key `usage`. `crates/dl-ai/src/tool_chat/types.rs`: `Option::is_none` in two serde attributes.
  - `crates/dl-provider-refusal/src/lib.rs`: `billing`, once as the reason code `BILLING` and once as a text marker searched in provider messages.
  These stay as they are. No commit of this branch may be read as claiming one source for them; f6f1a09e's wording "provider names that are on main" covers four of the 20 values, not the list.

Claims of the earlier fix commits, each with its test:
- dcedab49, "`HELPER_BINARY` names the helper once": `grep -rn '"dl-unit-failure-notify"' --include=*.rs --include=Cargo.toml bin crates` prints two lines, the package name in `bin/dl-unit-failure-notify/Cargo.toml:2` and the constant in `crates/dl-provider-refusal/src/lib.rs:35`. The helper's test compares its package name with the constant.
- 4a4c3d4c, "`GATEWAY_BINARY` and `GATEWAY_UNIT` come from one literal": `grep -rn '"dl-ai-gateway' --include=*.rs bin crates` prints one line, `crates/dl-provider-refusal/src/lib.rs:9`.
- 1014c510, "`HEALTH_PATH` names the route for the gateway router and for the installer's probe": `grep -rn '"/healthz"' --include=*.rs bin/dl-ai-gateway crates/dl-ai crates/dl-provider-refusal bin/dl-unit-failure-notify` prints one line, the constant. dl-bot's own `/healthz` route in `bin/dl-bot/src/mcp.rs` is another service and not touched.
- 1014c510, "the writers in usage.rs, chat_provider.rs, lib.rs, chat_backend.rs and anthropic_http.rs use the same names": a grep of the production parts of these five files for the ledger code words as string literals prints the constant definitions (`usage.rs:16-18`, `chat_provider.rs:171`) and the word `transport` twice: `chat_provider.rs:1463` (a log word of `request_error_kind`, on main at `:1330`) and `anthropic_http.rs:219` (a reason, on main at `:204`, the same word as the ledger code of `FailureKind::Unavailable` in `backend.rs:31`). Neither is a ledger code written at that place, and neither line is changed by this branch. Stated, not closed.
- dcedab49 and 4a4c3d4c on status numbers: a grep of the production parts of `anthropic_http.rs`, `chat_backend.rs`, `claude_plan.rs`, `tool_chat/alarm.rs` and `chat_provider.rs` for `402`, `412`, `429`, `529` prints three lines, all comments in `chat_provider.rs` (`:27`, `:272`, `:1391`).
- dcedab49, "round-trip test for every variant": the tests iterate `RefusalKind::ALL`, `Class::ALL` and `Outcome::ALL` (`dl-provider-refusal/src/lib.rs:306`, `:312`, `:319`), and since 4a4c3d4c `worded_enum!` emits `ALL` from the same list as the enum.

Not retested here:
- f0f438e4, the migration version "above every version applied in production and every version on the open branches": a statement of 2026-11-17. It is checked against production again before the migration is applied.
- fd2a00d9, "Every status mapper ... uses status_reason" and dcedab49, "billing has the kind Limit in every mapper": replaced by the narrower statements of 4a4c3d4c ("Not claimed" paragraph).
