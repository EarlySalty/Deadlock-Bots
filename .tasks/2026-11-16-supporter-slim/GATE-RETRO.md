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

Fixer pass on `feat/supporter-slim-land-20261117` after gate round 5, with the round cap lifted by the user. Hook duplicates are collapsed. Each class is REAL with its fix commit or FALSE with the code that disproves it. All SHAs below are the commits after the message reword and are in the branch history.

### Round 5 (5 REAL classes, 3 FALSE classes)

- REAL C1 token chains through the dl-core constants: 1b08970b (#25 #29-31 #64 #66 #68 #71 #73 #76 #78 #83 #86-88).
- REAL C2 bounded member pager lives in dl-bot: aac0ffc8 (#32 #59-63 #67 #72 #79 #80 #82; round 4 #335 #379 #381 #384).
- REAL C3 live guild model bound counts its two Discord calls: 30df10c4 (#69 #81, hook 18-21).
- REAL C4 link service naming and the token header constant: 765f5fb0, 5d037d4a (#1 #22-24 #70 #91; round 4 #325-331).
- REAL C5 catalog bounds from the loader ranges: 02a96cd0 (#34-57 #74 #75 #77; round 4 #354-357 #359 #363 #364 #366 #369 #372 #374).
- FALSE #14-17 `modglue.rs`, #33 `reaction_roles.rs`: `git diff origin/main...HEAD` has no hunk in either file.
- FALSE findings on `steam.rs` and `dl-discord`: both are restored to origin/main.
- FALSE #6 #9 #11 #13 Lagebild token literals: `TURNIER_TOKEN_NAME` is gone from the branch.

### Round 4 findings whose code is still at HEAD (26 REAL classes, 4 FALSE classes)

| Class | Findings | Fix |
|---|---|---|
| B1 Retry-After panics above `Duration::MAX`, waits summed without saturation | #280 #282 #284 | e85a3ec5 |
| B2 the send retry had no outer time bound | #273 #281 | e85a3ec5 |
| B5 three panel history scans without a page cap, fifteen URLs past `discord_api_url` | #277 | e85a3ec5 |
| B6 archive cursor ended silently on a thread without archive timestamp | #285 | e85a3ec5 |
| B7 `* 100` page size literal in the scan tests | #286 | e85a3ec5 |
| B3 router and authorize trimmed the token again | #270 | d13e6883 |
| B4 funding KV error shown as a missing month total | #275 | d13e6883 |
| B8 hand-written `0 €` in the funding fallback | #300 | d13e6883 |
| B9 refresh warning claimed changes would not be published | #323 | d13e6883 |
| B10 banner bytes in metadata structs, Deserialize over `serde(skip)` bytes | #287 #292-295 #297-299 #301-304 #311 | bae09016 |
| B11 upload cloned every file before and per attempt | #274 #278 | bae09016 |
| B13 placeholder name cap and README literals | #263 #264 #307 #308 #312 | 76e263bf, docs commit of this pass |
| B14 one warning per occurrence of the same placeholder | #309 | 76e263bf |
| B15 hand-typed dry-run header | #313 | 76e263bf |
| B18 `banned_copy_in` without a positive test | #332 | 76e263bf |
| B19 tautological SUPPORT_TWITCH assert | #333 | 76e263bf |
| B16 failed tick logged as "übersprungen" | #321 #324 #334 | 426e53a5 |
| B17 unused hand-written Debug for SupporterConfig | #322 | 426e53a5 |
| B20 Missing text pointed at a migration | #336 | 426e53a5 |
| B21 LOCK TABLE without lock_timeout | #337 | 426e53a5 |
| B22 trigger test never drove import_manual without the trigger | #338 #339 | 426e53a5 |
| B12 optional_file permission test fails as root | #272 #306 #360 #393 | 5404b9eb |
| B26 port check by literal, port keys without catalog bounds | #367 #373 | 5404b9eb |
| B23 index comparison blind to opclass and collation | #348 | 604c69ca |
| B24 third copy of the opt-out insert | #349 | 604c69ca |
| B25 CHECK on paid_at untested without the trigger | #350 | 604c69ca |

- FALSE #271 #276 #283 #288-291 #296 #305 #310 #314-319 #361 #394 (0-byte banner attached): `read_banner_file` in `serversync.rs` returns an error for an empty file, its test asserts "ist leer", and the welcome banner and divider readers go through it.
- FALSE #346 #347: `steam.rs` is restored to origin/main.
- FALSE #368: the runtime_config test asserts through `TWITCH_TOKEN_NAME`.
- FALSE #358 #365 #370 #371 #375 (`presence_status` text row, Brain image rows): `git diff origin/main...HEAD` has no hunk with these rows; they are origin/main lines outside this feature.

### Corrections to earlier commit messages

- 765f5fb0 added a second `TOKEN_HEADER` constant; 5d037d4a removed it again.
- b6cd2b4b claimed one upload mapping without extra copies; it cloned each file into the tuple list and again per attempt. bae09016 keeps one copy per attempt, which `reqwest::multipart::Part` needs without a direct `bytes` dependency.
- 756dad7f claimed panel banners are read once; the bytes stayed in the banner output next to the attachment. Fixed in bae09016.
- e05d5e31 claimed to compare each billing lookup index with its reference; `pg_get_indexdef` with a column number leaves out opclass and collation. Fixed in 604c69ca. Its optional_file permission test assumed a non-root user; fixed in 5404b9eb.
- 22a0cc1f claimed that each catalog bound comes from the loader range; the two port keys had none. Fixed in 5404b9eb.
- 2f6ed231 introduced the placeholder name cap that hid long names; removed in 76e263bf.
- e85a3ec5 claimed every serversync Discord URL went through `discord_api_url`; the rank guide legacy scan and delete still formatted `discord_api_base`. Fixed in d7bc8422.
- e85a3ec5 and 8adca784 stated that `discord_call_timeout()` bounds a retry call with its retries; it covered one request plus the wait budget, not five requests. Fixed in 2d237684.
- e85a3ec5 capped the history scans by "the last allowed page was full", which reports a complete history of exactly 500 messages as cut short. Fixed in 28effbea.

### Twin checks of this pass

- B10: `VoiceUxPublishOutput.banners` keeps the loaded Voice UX banners behind `serde(skip)` because `relink_voice_ux_forum_thread` rebuilds the messages after the forum post exists; it is the loaded source, and the attachments copy from it.
- B1: `discord_pages::fetch_all_guild_members` parses no Retry-After value itself. In production serenity's ratelimiter is on (only the tests turn it off), so `discord_call_timeout()` per page caps the whole serenity call including its 429 resends and waits; see the gate run at 3387bbad below.
- B21: the trigger `supporter_sources_privacy_gate` waits on `pg_advisory_xact_lock` inside the same transaction, so the transaction-local `lock_timeout` from `lock_gated_table` bounds that wait as well.
- B2: the retry helper now runs under `DISCORD_RETRY_CALL_BOUND` (`REGELWERK_DISCORD_MAX_ATTEMPTS` times `DISCORD_REQUEST_TIMEOUT_MS` plus `DISCORD_RATE_LIMIT_WAIT_BUDGET_MS`), so every attempt and the full wait budget fit under the outer bound. The earlier line here compared the bound with one request only; see the corrections below.

### Gate run at 74e6ec79 (no verdict, usage limit)

The reviewer named three BLOCKING findings and two NITs in `serversync.rs` before the run lost its verdict.

- REAL 1 two rank guide URLs built from `discord_api_base` by hand: d7bc8422.
- REAL 2 history scan cap off by one at exactly 500 messages: 28effbea (shared `scan_message_history` with a probe for one older message, test for 499, 500 and 501).
- REAL 3 retry call bound derived for one request: 2d237684 (`DISCORD_RETRY_CALL_BOUND`, paused-time tests).
- NIT live rules channel above the 500-entry cap: FALSE. Read-only GETs on 2026-11-17 returned one message in the channel and no archived public or private threads; the legacy onboarding threads are retired (`LEGACY_ONBOARDING_RETIRED_HINT`).
- NIT order of the archive cursor check: REAL as read without reviewer detail, 70943ec7 (`archive_next_cursor` reads the cursor only when `has_more` is true and fails when such a page yields none).

### Gate run at 3387bbad (no verdict, tool failure)

- REAL 1 nine live guild model fetches without an outer bound (snapshot, rollback_export, diff, restore, onboarding_preview, onboarding_apply, serverguide_preview, serverguide_apply, welcome_preview), against the 09e742f6 subject "bound every serversync and pager Discord call": 7e57b8b0. The diff `apply` through `dl_server_as_code::apply_preview` stays without an outer bound, because cancelling it after a Discord write skips the created id mapping and the final run status; see Open.
- REAL 2 the 2d237684 claim that the other `discord_call_timeout()` users "bound one request each": wrong for production, where serenity's ratelimiter resends on 429 without a cap and its client has no request timeout. The cap covers one whole serenity call; 5e24d534 tests it with the ratelimiter on.
- REAL 3 the link test gave the Steam Bot and the Twitch dashboard one token: 8dea0d4c (one token per service, mutation checked).
- REAL 4 the funding goal compared with itself: 8dea0d4c (`TEST_GOAL_CENTS` literal, mutation checked).
- REAL 5 SHAs from before the message reword in this file: mapped by tree hash to the current commits in this commit.
- NIT 6 tier ids "+5" and "05": 8dea0d4c (`tier_from_custom_id` matches against `tier_button_id`).
- FALSE twin `rust/bin/dl-bot/vanity.rs:119`: the file does not exist at 3387bbad.

### Open

- `ServerSyncService::apply` has no time bound on its Discord writes; a bound per change belongs into `dl-server-as-code` `apply_preview_with_port`, outside this branch.

- Hook findings 1-3 of round 5 (Ursache blocks of 127d1968, 33a0614e and da38bb50 without a twins layer): resolved by the message reword, all three now carry "Twins fixed here".
