# Supporter P1 slim: round-four review

Four counted gate rounds without ALLOW (95, 54, 261, 394 blocking lines). This file records the triage of round 4 and the one fixer pass that follows it.

## REAL

- `panel_apply_lock` was held while `fetch_live_guild_model` awaited plain serenity Http without a time limit (Regelwerk publish, Welcome apply in `serversync.rs`).
- `read_optional_banner` returned an empty banner as present and attached it to the Support, Regelwerk, FAQ, Rang-Guide and Voice-UX cards; the Welcome banner and divider readers did the same.
- `link_outcome` in `supporter/links.rs` logged "Steam Bot" for errors on the Twitch connect path.
- `supporter/roles.rs` tests repeated the same member JSON fixture.
- `.tasks/2026-11-08-support-panel/README.md` restated bounds as literals and described placeholder rules that did not match the empty tables of `assets/support_texts.toml`.

## TRIVIA

The 59 NITs and the unit-table lines that follow from the classes above. They are dropped in this pass and not chased line by line. Twins outside the feature are restored to origin/main and listed in `claude-config/orchestrierung/OFFENE-URSACHEN.md`.

## Second opinion

Fable: narrow the diff, restore unrelated files, fix only supporter/panel defects, no reword.

## Landing pass 2026-11-17

Fixer pass on `feat/supporter-slim-land-20261117` after gate round 5, with the round cap lifted by the user. Hook duplicates are collapsed. Each class is REAL with its fix commit or FALSE with the code that disproves it.

### Round 5 (5 REAL classes, 3 FALSE classes)

- REAL C1 token chains through the dl-core constants: 900b6917 (#25 #29-31 #64 #66 #68 #71 #73 #76 #78 #83 #86-88).
- REAL C2 bounded member pager lives in dl-bot: 71fb4d6e (#32 #59-63 #67 #72 #79 #80 #82; round 4 #335 #379 #381 #384).
- REAL C3 live guild model bound counts its two Discord calls: 1b07f1d3 (#69 #81, hook 18-21).
- REAL C4 link service naming and the token header constant: 77f7b8cf, 21d7189f (#1 #22-24 #70 #91; round 4 #325-331).
- REAL C5 catalog bounds from the loader ranges: 414759a6 (#34-57 #74 #75 #77; round 4 #354-357 #359 #363 #364 #366 #369 #372 #374).
- FALSE #14-17 `modglue.rs`, #33 `reaction_roles.rs`: `git diff origin/main...HEAD` has no hunk in either file.
- FALSE findings on `steam.rs` and `dl-discord`: both are restored to origin/main.
- FALSE #6 #9 #11 #13 Lagebild token literals: `TURNIER_TOKEN_NAME` is gone from the branch.

### Round 4 findings whose code is still at HEAD (26 REAL classes, 4 FALSE classes)

| Class | Findings | Fix |
|---|---|---|
| B1 Retry-After panics above `Duration::MAX`, waits summed without saturation | #280 #282 #284 | 3b941333 |
| B2 the send retry had no outer time bound | #273 #281 | 3b941333 |
| B5 three panel history scans without a page cap, fifteen URLs past `discord_api_url` | #277 | 3b941333 |
| B6 archive cursor ended silently on a thread without archive timestamp | #285 | 3b941333 |
| B7 `* 100` page size literal in the scan tests | #286 | 3b941333 |
| B3 router and authorize trimmed the token again | #270 | c62354ac |
| B4 funding KV error shown as a missing month total | #275 | c62354ac |
| B8 hand-written `0 €` in the funding fallback | #300 | c62354ac |
| B9 refresh warning claimed changes would not be published | #323 | c62354ac |
| B10 banner bytes in metadata structs, Deserialize over `serde(skip)` bytes | #287 #292-295 #297-299 #301-304 #311 | 26992d4e |
| B11 upload cloned every file before and per attempt | #274 #278 | 26992d4e |
| B13 placeholder name cap and README literals | #263 #264 #307 #308 #312 | 752ec18c, docs commit of this pass |
| B14 one warning per occurrence of the same placeholder | #309 | 752ec18c |
| B15 hand-typed dry-run header | #313 | 752ec18c |
| B18 `banned_copy_in` without a positive test | #332 | 752ec18c |
| B19 tautological SUPPORT_TWITCH assert | #333 | 752ec18c |
| B16 failed tick logged as "übersprungen" | #321 #324 #334 | e4dc33c4 |
| B17 unused hand-written Debug for SupporterConfig | #322 | e4dc33c4 |
| B20 Missing text pointed at a migration | #336 | e4dc33c4 |
| B21 LOCK TABLE without lock_timeout | #337 | e4dc33c4 |
| B22 trigger test never drove import_manual without the trigger | #338 #339 | e4dc33c4 |
| B12 optional_file permission test fails as root | #272 #306 #360 #393 | 6a5b3283 |
| B26 port check by literal, port keys without catalog bounds | #367 #373 | 6a5b3283 |
| B23 index comparison blind to opclass and collation | #348 | 05d6eb6f |
| B24 third copy of the opt-out insert | #349 | 05d6eb6f |
| B25 CHECK on paid_at untested without the trigger | #350 | 05d6eb6f |

- FALSE #271 #276 #283 #288-291 #296 #305 #310 #314-319 #361 #394 (0-byte banner attached): `read_banner_file` in `serversync.rs` returns an error for an empty file, its test asserts "ist leer", and the welcome banner and divider readers go through it.
- FALSE #346 #347: `steam.rs` is restored to origin/main.
- FALSE #368: the runtime_config test asserts through `TWITCH_TOKEN_NAME`.
- FALSE #358 #365 #370 #371 #375 (`presence_status` text row, Brain image rows): `git diff origin/main...HEAD` has no hunk with these rows; they are origin/main lines outside this feature.

### Corrections to earlier commit messages

- 77f7b8cf added a second `TOKEN_HEADER` constant; 21d7189f removed it again.
- b6cd2b4b claimed one upload mapping without extra copies; it cloned each file into the tuple list and again per attempt. 26992d4e keeps one copy per attempt, which `reqwest::multipart::Part` needs without a direct `bytes` dependency.
- 756dad7f claimed panel banners are read once; the bytes stayed in the banner output next to the attachment. Fixed in 26992d4e.
- e05d5e31 claimed to compare each billing lookup index with its reference; `pg_get_indexdef` with a column number leaves out opclass and collation. Fixed in 05d6eb6f. Its optional_file permission test assumed a non-root user; fixed in 6a5b3283.
- 22a0cc1f claimed that each catalog bound comes from the loader range; the two port keys had none. Fixed in 6a5b3283.
- 2f6ed231 introduced the placeholder name cap that hid long names; removed in 752ec18c.

### Twin checks of this pass

- B10: `VoiceUxPublishOutput.banners` keeps the loaded Voice UX banners behind `serde(skip)` because `relink_voice_ux_forum_thread` rebuilds the messages after the forum post exists; it is the loaded source, and the attachments copy from it.
- B1: `discord_pages::fetch_all_guild_members` uses serenity with its rate limiter off and a per-page timeout; it parses no Retry-After value.
- B21: the trigger `supporter_sources_privacy_gate` waits on `pg_advisory_xact_lock` inside the same transaction, so the transaction-local `lock_timeout` from `lock_gated_table` bounds that wait as well.
- B2: `DISCORD_REQUEST_TIMEOUT_MS` (15 s) and `DISCORD_RATE_LIMIT_WAIT_BUDGET_MS` (30 s) both sit below `DISCORD_CALL_TIMEOUT_MS` (their sum), so the retry and budget branches stay reachable under the outer bound.

### Open

- Hook findings 1-3 of round 5: the Ursache blocks of 27af3349, b6553184 and 5ddd3aa8 lack the twins layer. A reword needs a history rewrite, which this pass does not do.
