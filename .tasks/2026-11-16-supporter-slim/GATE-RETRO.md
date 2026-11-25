# Supporter P1 slim: round-four review

Four counted gate rounds without ALLOW (95, 54, 261, 394 blocking lines). This file records the triage of round 4, the landing pass on `feat/supporter-slim-land-20261117`, the gate runs after it and the fixer pass for the round-10 verdict.

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
| B26 port check by literal, port keys without catalog bounds | #367 #373 | 5404b9eb (validate through `PORT_RANGE`); its catalog port bounds were removed again in 08470994, see the 06b51ebb corrections |
| B23 index comparison blind to opclass and collation | #348 | 604c69ca |
| B24 third copy of the opt-out insert | #349 | 604c69ca |
| B25 CHECK on paid_at untested without the trigger | #350 | 604c69ca |

- FALSE #271 #276 #283 #288-291 #296 #305 #310 #314-319 #361 #394 (0-byte banner attached): `read_banner_file` in `serversync.rs` returns an error for an empty file, its test asserts "ist leer", and the welcome banner and divider readers go through it.
- FALSE #346 #347: `steam.rs` is restored to origin/main.
- FALSE #368: the runtime_config test asserts through `TWITCH_TOKEN_NAME`.
- FALSE #358 #365 #370 #371 #375 (`presence_status` text row, Brain image rows): `git diff origin/main...HEAD` had no hunk with these rows when this line was written. Stale since 08470994, which changes TSV lines 23, 159 and 162; see the 06b51ebb section.

### Corrections to earlier commit messages

- 765f5fb0 added a second `TOKEN_HEADER` constant; 5d037d4a removed it again.
- b6cd2b4b claimed one upload mapping without extra copies; it cloned each file into the tuple list and again per attempt. bae09016 keeps one copy per attempt, which `reqwest::multipart::Part` needs without a direct `bytes` dependency.
- 756dad7f claimed panel banners are read once; the bytes stayed in the banner output next to the attachment. Fixed in bae09016 for the FAQ, Regelwerk, Support, Rang guide and welcome banners, not for Voice UX (see the bae09016 correction in the 06b51ebb section).
- e05d5e31 claimed to compare each billing lookup index with its reference; `pg_get_indexdef` with a column number leaves out opclass and collation. Fixed in 604c69ca. Its optional_file permission test assumed a non-root user; fixed in 5404b9eb.
- 22a0cc1f claimed that each catalog bound comes from the loader range; the two port keys had none. 5404b9eb gave them catalog bounds from `PORT_RANGE`; 08470994 removed those again because both port rows are protected (see the 06b51ebb section).
- 2f6ed231 introduced the placeholder name cap that hid long names; removed in 76e263bf.
- e85a3ec5 claimed every serversync Discord URL went through `discord_api_url`; the rank guide legacy scan and delete still formatted `discord_api_base`. Fixed in d7bc8422.
- e85a3ec5 and 8adca784 stated that `discord_call_timeout()` bounds a retry call with its retries; it covered one request plus the wait budget, not five requests. Fixed in 2d237684.
- 2d237684 said the other `discord_call_timeout()` users bound one request each; with serenity's ratelimiter on, the cap covers one whole serenity call. Corrected in 5e24d534 and 65fcb08a.
- 5e24d534 said its test shows the cap with the ratelimiter on; the test hung on a silent proxy and never made the ratelimiter wait. Fixed in 65fcb08a.
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
- REAL 2 the 2d237684 claim that the other `discord_call_timeout()` users "bound one request each": wrong for production, where serenity's ratelimiter resends on 429 without a cap and its client has no request timeout. The cap covers one whole serenity call. 5e24d534 added a test against a silent proxy that could not tell a ratelimiter wait from a hang; 65fcb08a replaces it with an exhausted GuildMembers bucket, so the ratelimiter waits before the first request and the page timeout ends the call with zero requests sent (mutation checked).
- REAL 3 the link test gave the Steam Bot and the Twitch dashboard one token: 8dea0d4c (one token per service, mutation checked).
- REAL 4 the funding goal compared with itself: 8dea0d4c (`TEST_GOAL_CENTS` literal, mutation checked).
- REAL 5 SHAs from before the message reword in this file: mapped by tree hash to the current commits in this commit.
- NIT 6 tier ids "+5" and "05": 8dea0d4c (`tier_from_custom_id` matches against `tier_button_id`).
- REAL twin `rust/bin/dl-bot/src/vanity.rs:119` (the reviewer left out `src/`, and an earlier version of this file called it FALSE because of that): it formats its own `DISCORD_API_BASE` by hand (`vanity.rs:7`, `:119`, `:344`). `vanity.rs` is origin/main at HEAD and outside this branch; see Backlog outside this branch.

### Open

- `ServerSyncService::apply` has no time bound on its Discord writes; a bound per change belongs into `dl-server-as-code` `apply_preview_with_port`, outside this branch.

### Resolved

- Hook findings 1-3 of round 5 (Ursache blocks of 127d1968, 33a0614e and da38bb50 without a twins layer): resolved by the message reword, all three now carry "Twins fixed here".

### Gate runs on 06b51ebb (no verdict, usage limit)

Two runs lost their verdict to the reviewer usage limit after the reviewers had named their findings. The settings catalog answer of the second run names its fifth finding ("several commit messages make claims about HEAD that are false") without the commits; that reviewer detail is lost. The corrections below are the false claims checked against HEAD, not the reviewer's own list.

- REAL 1 the static panel and welcome id loaders parsed the index suffix with `usize::from_str` (the reviewer called the welcome loader at `serversync.rs:1748` the Voice UX loader): e7bef533.
- REAL 2 TSV lines 23, 159 and 162 stated bounds as literal numbers: 08470994.
- REAL 3 the twitch_invites ranges twice and a one-way probe: 08470994.
- REAL 4 the placeholder test could not see a stale literal: 08470994.
- REAL 5 bounds on protected rows: 08470994.
- REAL 6 `TABLE_LOCK_TIMEOUT_MS` without a derivation, README read as a bound for the whole sync: a4d8882a.
- REAL 7 only `LOCK TABLE` mapped 55P03 to `LockTimeout`: a4d8882a.
- REAL 8 the zero-call assert of `rate_limiter_waits_are_bounded` could not fail: a4d8882a.
- REAL 9 the ratelimiter claims of 65fcb08a and 5e24d534: corrected below.
- REAL 10 nothing bounded one role sync tick: a4d8882a.
- REAL 11 false commit claims without a correction here: corrected below.
- NIT the `block_member` test wrapper: removed in a4d8882a.

Corrections to earlier commit messages:

- 65fcb08a said the roles.rs tests build Http with `ratelimiter_disabled(true)` and make no ratelimiter claim. At that commit `rate_limiter_waits_are_bounded` in roles.rs already installed `serenity::http::Ratelimiter` (`git show 65fcb08a:rust/bin/dl-bot/src/supporter/roles.rs`, line 382), and it claims that ratelimiter waits are bounded.
- 5e24d534 said every Discord test in dl-bot built Http with `ratelimiter_disabled(true)`. The same roles.rs test turned the ratelimiter on at that commit (line 382).
- e7bef533 recorded a twin search that only matched a parse within three lines after `strip_prefix(` or `split_once(`. The class search is every integer or text parse in the touched Rust files: `git diff --name-only origin/main...HEAD | grep '\.rs$' | xargs grep -nE '\.parse(::<[^>]*>)?\(\)|from_str(_radix)?\('`, 63 hits at a4d8882a:
  - Readers of strings the bot writes and stores: `serversync.rs:1019` (`static_v2_message_index`, the owner) and `support_publish.rs:202` (`SupportFunding::from_kv`), both strict since e7bef533, and the `bot.kv_store` id reads through `parse_discord_id` (`serversync.rs:5849`), strict since 7466fde5 through `parse_stored_discord_id`. An earlier version of this file filed `:5849` under rate limit headers.
  - Whole documents through serde, toml, `PgConnectOptions` or `IpAddr`: `serversync.rs:2995 7006 7060`, `support_publish.rs:146`, `privacy.rs:3488`, `bot_config.rs:280 344`, `runtime_config.rs:708`, `testing.rs:190 262`.
  - Discord rate limit headers: `serversync.rs:5808 5816`.
  - Channel and role ids from the server-as-code config and the live onboarding: `serversync.rs:6102 6130 6184 6523 6542 6640 6687 6702`.
  - Environment and config values written by people: `main.rs:110 138 161 167 255 1084 1857`, `bot_config.rs:219 233 245 249`, `runtime_config.rs:466`.
  - Settings catalog: `settings_catalog.rs:408` (TSV type column), `492` and `768` (paths from the TSV and the admin request), `540` (admin form value, already strict by its round trip), `548 553 596 601` (the catalog's own in-memory bound strings, no stored copy).
  - Privacy erasure matching of user ids in other bots' data, lenient on purpose so an erasure finds every spelling: `privacy.rs:1711 1725 1732 1884 1911`.
  - Test code: `serversync.rs:9067 9137 9141 11155`, `settings_catalog.rs:805 813 834 869 885 886 1170 1176 1183 1202 1205`.
  Backlog outside the touched files: `rust/bin/dl-bot/src/mcp/public.rs:897` (key suffix without a round trip) and `:913` (stored id without a round trip). The index parse at `:912` is strict through the round trip at `:914`; an earlier version of this file and e7bef533 called it lenient.
- 8dea0d4c checked its class (a reader that accepts more spellings than the bot writes) only in `supporter/links.rs` and `supporter.rs`. `load_static_v2_message_ids` and the welcome id loader in `serversync.rs` and `SupportFunding::from_kv` had the same shape; fixed in e7bef533.
- bae09016 said the Voice UX banner outputs are metadata only and the bytes stay in the payload attachment only. `VoiceUxPublishOutput.banners` (`voice_ux_publish.rs:82`) keeps the `LoadedBanner` bytes behind `serde(skip)`, and `voice_ux_publish.rs:812` clones them into each attachment. The B10 twin check above states this HEAD state.
- 507582ff said `RuntimeConfig::validate` enforces `matcher_max_ai_per_scan` and `matcher_scan_interval_hours` and that bot_config, operating_config and runtime_config share the new ranges. 3ccdf17d reverted that to origin/main, and until 789129be both matcher rows had no loader check and catalog-only bounds (`CATALOG_ONLY_BOUNDS`). Since 789129be `RuntimeConfig::validate` checks both rows against `MATCHER_MAX_AI_PER_SCAN_RANGE` and `MATCHER_SCAN_INTERVAL_HOURS_RANGE`, which the catalog takes as its bounds, and `CATALOG_ONLY_BOUNDS` is gone. `operating_config.rs` is origin/main, and the bot_config ranges are named again since 08470994.
- 22a0cc1f said its named ranges, `MATCHER_SCAN_INTERVAL_HOURS_RANGE` starting at 0, the u64 matcher reader, `DISCORD_STRING_OPTION_MAX_LENGTH` for the /brain question length and the admin_settings probe test are in place. 3ccdf17d reverted all of them: `RUNTIME_POSITIVE_SPAN_RANGE` and `DISCORD_STRING_OPTION_MAX_LENGTH` do not exist at HEAD, `MATCHER_SCAN_INTERVAL_HOURS_RANGE` exists again since 789129be as a dl-core loader range starting at 0, `admin_settings.rs`, `modglue.rs` and `dl-bridges` `matcher.rs` are origin/main, and `BRAIN_MAX_QUESTION_LEN_RANGE` ends at 65 536. The catalog probe test lives in `settings_catalog.rs` since 02a96cd0.
- 02a96cd0 said the rows that `BotConfig::validate` checks keep literal catalog bounds, `bot_config.rs` is not touched and keeps its literal id rule, `presence_status` is outside the class, the Brain image rows have no loader rule, and its placeholder test rejects bound literals in texts. Since 08470994 the bot_config rows use named consts in `bot_config.rs` for validate and the catalog, the id rule uses `DISCORD_ID_RANGE`, `PRESENCE_STATUS_MAX_CHARS` bounds `presence_status` in the loader and the catalog (and since c0e0e954 the catalog also applies the loader's blank rule through `presence_status_valid`), and `BRAIN_IMAGE_MAX_COUNT_RANGE` and `BRAIN_IMAGE_JPEG_QUALITY_RANGE` are loader rules. The 02a96cd0 placeholder test let TSV lines 23, 159 and 162 keep their literal bounds.
- 5404b9eb and the B26 row: 08470994 removed the catalog bounds of `runtime.start.mcp_port` and `runtime.bridges.twitch_port`, because both rows are protected and the admin form cannot write them. At HEAD only `RuntimeConfig::validate` checks the two ports against `PORT_RANGE` (`runtime_config.rs:540`).
- 32939e92 said the pager search "now finds only the helper"; 9fc9bca1 said `EMPTY_MEMBER_LIST` "covers every REST member reader (supporter role sync, serversync role assignments, modglue coach fallback and role members, vanity sweep, adapter role members)"; 09e742f6 said every Discord REST client comes from `serversync::discord_http_client` "(serversync, vanity snapshots, coaching post and edit)" and every Discord cursor loop stops through `dl_discord::adapter`; aac0ffc8 said the pager, its limit "and the fixture exist once, in dl-bot". c71e8e16 and 866f264e reverted `modglue.rs` and `vanity.rs` to origin/main, and `dl-discord` has no diff against origin/main. At HEAD the shared pager in `discord_pages.rs` serves the supporter role sync (`supporter/roles.rs:57`) and the serversync member scan (`serversync.rs:1231`); `modglue.rs:2799` and `:3601` and `vanity.rs:294` page by hand without a page timeout or stall guard, `adapter.rs:1072` keeps its own pager with the literal 1000, `vanity.rs:46` and `:83` build `reqwest::Client::new()`, and `advance_after` has no caller outside `discord_pages.rs`. See Backlog outside this branch.
- 8dea0d4c said in its why-unnoticed layer that every tier id in the tests came from `tier_button_id`. Before 8dea0d4c the stale-id list in `supporter/links.rs` already held hand-written ids ("support:tier:3", "support:tier:", "support:tier:x", "support:tier:-5"), and the button tests used the literals "support:tier:20" and "support:tier:50". The tests fed malformed ids, but none in a spelling that `u64::from_str` accepts and the writer never builds, such as "+50" or "050"; 8dea0d4c added those.
- ec1c5a0b said clippy 1.99 raises `double_must_use` inside the `#[async_trait]` expansion "at 80 trait sites of the workspace, 78 of them in origin/main code". That count came from a line filter that kept only the `#[async_trait::async_trait]` spelling and counted indentation variants twice. Counted by the first location of each warning in the clippy log over dl-bot, dl-core, dl-brain, dl-central-db and dl-community at ea11c14c, the lint fires at 91 trait sites: 87 in files outside the branch diff (among them `modglue.rs:253`, `:502` and `:2210`, which equal origin/main), 2 on origin/main lines of branch files (`serversync.rs:764`, `dl-brain/src/lib.rs:227`) and 2 on lines this branch added (`supporter.rs:97`, `supporter/links.rs:136`). Both branch warnings stay open as part of that workspace class.

### Fixer pass for the round-10 verdict (2026-11-18 to 2026-11-25)

Round 10 ended in BLOCK with 40 blocking lines, most of them hook twins. Each class below names its fix commit; every fix commit carries its own Ursache block with the twin search and its output.

- Stored Discord ids read leniently (BLOCKING 5-17, 27-29): 7466fde5, `parse_stored_discord_id` and `stored_discord_id` in `serversync.rs`, key replacement limited to the keys the id writer wrote.
- Member scan without a bound on the whole scan: bfefb444, bounded by the page count from `approximate_member_count`.
- Mode-000 banner test failing as root (BLOCKING 18): 6e12ecbc probes permission enforcement first.
- Placeholder lint skipping brace contents (BLOCKING 19): b749cb43.
- Banner bytes copied into outputs and attachments: 2fc07b43 moves them.
- Sync interval test equal to the default: 928303e6.
- Stripe tip rows without a schema bound on their validity: 47034252, CHECK `valid_until <= paid_at + 744 hours` in the not yet applied migration 20261116130000.
- Port rules through the literal 0 and invite range texts without an owner (BLOCKING 31, 34, 35): daed4a0e.
- Brain image edge and download size clamped silently by the reader (BLOCKING 32, 33, 38, 40): d4b86694. Its twin list named `modglue.rs:1812` and `:4231`; those hits came from a run over the working tree, and the stated command over `origin/main...HEAD` does not list `modglue.rs` (corrected in 789129be).
- Matcher rows with catalog-only bounds (BLOCKING 36): 789129be.
- Writable text rows without the loader's rule (BLOCKING 37, 39): c0e0e954.
- Retro and README claims (BLOCKING 1-4, 10, 26): this commit, see the corrections above and below.

Live checks, read-only:

- 2026-11-18 (round 11): the privacy trigger on `core.supporter_sources` has `tgenabled` 'O'; role `deadlock` has `rolbypassrls` true, `twitchdash` false; the stored panel option values and the 17 stored serversync message ids are canonical, so the strict readers reject none of them; the 35 serversync keys have no LIKE prefix collision; every tip row lies within 31 days of its payment.
- 2026-11-25: `_sqlx_migrations` ends at 20261116120000, so 20261116130000 is not applied and could still be changed; 2 tip rows, none with a NULL `valid_until` or one beyond 744 hours after `paid_at`; `/etc/deadlock-svc/master/bot.toml` sets none of the `brain_image_*` or matcher keys, and its `presence_status` "DM Support", `log_filter` "info", the five service ports and `personal_links_per_channel` 10 pass the loader rules of this pass.

### Backlog outside this branch

These twins sit in files that are origin/main at HEAD and belong to a repo review, not to this branch:

- `rust/bin/dl-bot/src/vanity.rs:7`, `:119`, `:344`: own `DISCORD_API_BASE` and hand-built Discord URLs on `reqwest::Client::new()` without a timeout (`:46`, `:83`).
- `rust/bin/dl-bot/src/modglue.rs:2799`, `:3601` and `vanity.rs:294`: member pagers by hand without a page timeout or stall guard; `rust/crates/dl-discord/src/adapter.rs:1072`: own pager with the literal page size 1000.
- `rust/bin/dl-bot/src/mcp/public.rs:897` and `:913`: stored key suffix and stored id parsed without a round trip.
- `rust/bin/dl-bot/src/twitch_invites_personal.rs:148`: the literal 1000 next to `DISCORD_GUILD_INVITE_LIMIT`.
- `rust/crates/dl-bridges/src/matcher.rs:361`: the reader's own `.max(0)` next to the dl-core matcher ranges.
