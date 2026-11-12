# Round 4 self-review

Reviewed all 220 numbered round-3 findings and the token-twin list before the
single final normal-round gate run. The ledger in
`.tasks/2026-11-12-steam-timeout-round4/finding-ledger.jsonl` retains each finding,
its source location, classification and disposition. It stays in this worktree
because this task restricts work to this checkout.

| Round-3 findings | Mechanism and evidence |
| --- | --- |
| 1, 2, 5, 6, 7, 8, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 55, 56, 57, 58, 59, 60, 62, 63, 64, 65, 66, 67, 68, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 83, 85, 86, 87, 88, 89, 94, 95, 96, 98, 103, 104, 105, 110, 112, 113, 115, 117, 118, 121, 124, 130, 131, 132, 134, 135, 141, 149, 150, 157, 163, 177, 197, 199, 208, 209, 211, 215, 219 | Explicit configuration ownership: The builder has no global lookup. Every call supplies Deadlines. Gateway providers pass for_operation(request_timeout) from their own provider config. Steam alone consumes injected Steam settings. Other components use catalog defaults and existing operation arguments. |
| 3, 4, 128, 142, 181, 204, 205 | Evidence correction: The round-3 disposition table and nonstandard green-test claims are withdrawn. The audit states the final defaults, operation semantics, consumer scope and deploy order. No live health or green check is inferred from missing evidence. |
| 9, 10, 100, 101, 165, 201 | Scrim startup: TurnierScrimClient::from_lookup uses explicit catalog defaults, its own RELAY_TIMEOUT and disabled-client error retention on construction failure. Its production expect is removed. |
| 19, 61, 137, 159, 168, 169, 180, 182, 198, 214, 216 | Construction twins: ImageFetcher and API ingest use the shared client with catalog defaults or ingest budget. The live golden test uses explicit defaults. Guard scans dl-brain and dl-knowledge in addition to the previous five consumers, and rejects convenience GETs, aliases, glob imports and grouped Client imports. |
| 42, 102, 109, 114, 116, 119, 127, 129, 133, 140, 143, 164, 173 | Package-directory tests: All shared-client test and component constructors pass explicit deadlines. Standard cargo-slot test --workspace uses package cwd without repository-root execution or deployment-config fixtures. |
| 43, 54, 136, 147, 153 | Failure messages: Steam publish-panel and admin failure paths reuse UNREACHABLE_MSG. Dashboard Twitch linking no longer promises recovery after one minute. |
| 69, 70, 82, 84, 166, 167, 175, 178, 184, 186, 187, 190, 195 | Unused wrapper surface: Client Default and head, Builder connect_timeout, and RequestBuilder form/body/headers are removed. Deadline validation is centralized; build/send share finish instead of duplicate request clamping. |
| 90, 91, 92, 93, 148, 154, 192, 193, 202 | Configuration boundaries: Steam parser and typed validation share bounds and catalog defaults. Tests reach both actual Steam endpoints, reject overflow and malformed values, and exercise effective connect/total relationships when either setting is missing. Generic operations reject zero and values above MAX_OPERATION_MS. |
| 97, 99, 203, 206, 207, 210, 212, 213, 218, 220 | Deployment compatibility: No live files touched. Missing keys work. The deploy note says deploy binaries before adding keys and requires no live config edit. Earlier live-state claims are withdrawn; health is an integrator verification after deployment. |
| 106, 107, 108, 111, 120, 122, 126, 139, 145, 162, 176, 183, 189 | Independent operation budgets: RequestBuilder::finish uses an explicit request budget when present and supplies the client default only when absent. Builder::timeout replaces the default with a bounded component budget. Steam invite and rankcheck use catalog 120 s, sync uses 180 s, Twitch can retain its validated 3600 s. Const assertions retain the external 90 s invite contract and operation upper bounds. |
| 123, 125, 146, 152, 156, 172, 200 | Operating-config latency: SteamBotClient::operating_config installs catalog STEAM_OPERATING_SECS on GET and PATCH while reusing the bounded no-proxy/no-redirect client. |
| 138, 144, 151, 161, 179, 185, 217 | Failure identity: Disabled clients retain the initialization cause. Request cloning retains that error and build/send share finish. AI classifiers distinguish initialization/configuration from transport. Steam logging retains the builder error. Startup panic and missing-key disabled states are eliminated through defaults. |
| 155, 160, 171, 188 | Preserved history: The task forbids force pushes. Prior messages remain historical; the new commit and audit explicitly supersede their bounds and override descriptions. No rewording or history rewrite is attempted. |
| 158, 170, 174, 194, 196 | Catalog and UI edge: Optional Steam fields remain nullable. Null selects the documented service defaults; the unchanged editor can represent absence without phantom invalid changes. Positive normalize cases cover null and the actual 120000 upper edge; malformed numeric inputs remain rejected. |
| 191 | Immutable values: Clients retain the explicitly passed value. Constructing a second client cannot mutate the first. No OnceLock, ambient config or current working directory participates in deadline selection. |

The HOOK-TWIN tokens resolve through the same changes: Steam timeout names have
one injected parser, all panel/admin failure text shares UNREACHABLE_MSG, sync
budgets share the catalog constant, and Steam operating-config retains the
internal-token header while reusing the existing client. Test API-key literals
are synthetic fixtures, not live credentials.

Edge decisions: missing optional Steam values use catalog defaults; null resets
restore absence; empty/non-numeric/zero/overflow values reject; the effective
connect budget must not exceed total; generic operation budgets must be positive
and at most the catalog maximum. Gateway providers validate configured request
budgets before constructing backends. A disabled request preserves the original
error during clone/build/send, and an existing client never observes later values.

No deployment, merge or live config edit is part of this handover.

## Validation results

- Changed Rust files pass rustfmt 1.97.1 with `--edition 2021 --check`.
- `cargo-slot clippy --workspace --all-targets --message-format=json --manifest-path rust/Cargo.toml` passed. Its existing diagnostics include no primary spans on changed lines.
- The complete workspace suite passed on ionos using `tools/test-db/test-db-run deadlock_test`, exporting CENTRAL_TEST_DSN and DEADLOCK_CENTRAL_DSN from the runner's DATABASE_URL, then `cargo-slot test --workspace --manifest-path rust/Cargo.toml`. Cargo used normal package working directories. The run ended with cargo-remote EXIT=0 and cargo-slot EXIT=0.
- Totals: 2464 passed, 0 failed, 72 ignored across 115 unit, integration and doc-test result summaries.

An initial local build observed an intermediate catalog artifact while editing;
the final source was then frozen and rechecked. A plain workspace test invocation
compiled successfully but stopped at four dl-activity tests because CENTRAL_TEST_DSN
was absent. The successful ionos run supplied the required throwaway database.
No test was moved to the repository root to resolve config paths.

Detailed logs: `/tmp/steam-timeout-round4-clippy-final.log` and
`/tmp/steam-timeout-round4-tests-db.log`. These are validation artifacts, not
claims about deployed services. The gate is run once after committing and pushing.
