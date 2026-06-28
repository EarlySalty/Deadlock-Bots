# Tournament-Removal REMOVE/KEEP Map (2026-06-28)

Scope: read-only discovery for removing the old in-bot tournament signup/admin surface and `!balance` from Deadlock-Bots. No source deletion, no code edits, no DB drops were done here.

Important split:

- REMOVE means old Deadlock-Bots-owned tournament functionality: `/turnier` admin/signup, `/api/turnier/*`, `/api/tournament/*`, `cogs.customgames.tournament_store`, `dl_tournament::store`, old public tournament website on `:8767`, and `!balance`.
- KEEP means Tierlist and Bridge/OAuth contracts consumed by `/home/naniadm/Documents/Deadlock-Turniere` or other public web surfaces.
- AMBIGUOUS means the name or route is tournament-adjacent, but removal needs a separate decision or live config check.

## Deadlock-Turniere konsumiert

Cross-read path: `/home/naniadm/Documents/Deadlock-Turniere/backend`.

- OAuth delegation base: `backend/config.py:198-210` sets `DISCORD_OAUTH_INTERNAL_API_BASE_URL` defaulting to `http://127.0.0.1:8766` and accepts token aliases including `TURNIER_INTERNAL_API_TOKEN`; this consumes Deadlock-Bots dashboard OAuth endpoints.
- OAuth initiate/consume endpoints: `backend/auth/discord_oauth.py:17-18`, `backend/auth/discord_oauth.py:88`, and `backend/auth/discord_oauth.py:115` call `/internal/v1/discord/initiate` and `/internal/v1/discord/consume-result`.
- Broker base: `backend/config.py:211-227` sets `DISCORD_MASTER_BROKER_BASE_URL`/`DISCORD_MASTER_BROKER_TOKEN`; this consumes Deadlock-Bots master-broker endpoints, but see AMBIGUOUS for the default-port mismatch.
- Broker actions: `backend/notifications/discord_notifier.py:168`, `:214`, `:227`, `:296`, `:335`, `:346`, `:355`, `:410`, `:462`, `:482`, and `:497` call `/internal/master/v1/discord/create-channel`, `/send-rich-message`, `/delete-channel`, `/send-message`, `/member/move-voice`, `/voice-channel/members`, and `/role/members`.
- SQLite Steam link bridge: `backend/steam/reader.py:1`, `backend/steam/reader.py:35-41` opens `STEAM_BRIDGE_DB_PATH` read-only and selects from `steam_links`.
- SQLite rank-role bridge: `backend/rank_reader.py:168-180` opens `STEAM_BRIDGE_DB_PATH` read-only and selects from `deadlock_subrank_roles`.
- SQLite Steam task bridge: `backend/match/steam_bridge.py:24-28`, `:39-46`, and `:123-128` reads/writes `steam_tasks` through `STEAM_BRIDGE_DB_PATH`.
- Deadlock-Turniere code search found no consumer of Deadlock-Bots `TURNIER_PUBLIC_PORT`, `:8767`, `/api/turnier/*`, or `/api/tournament/*`.
- `backend/config.py:265-266` `TURNIER_PUBLIC_URL` is the new Deadlock-Turniere frontend URL used by its OAuth flow, not Deadlock-Bots `TURNIER_PUBLIC_PORT`.

## REMOVE

| Area | Datei:Zeile / Symbol | Begründung |
| --- | --- | --- |
| Rust crate | `rust/crates/dl-tournament/Cargo.toml:1` `dl-tournament` | Whole crate is the old tournament domain crate and should be removed as a unit. |
| Rust crate | `rust/crates/dl-tournament/src/lib.rs:1-11` module exports | Exports only old `balancer`, `balance_cmd`, `discord_ui`, `store`, and `web` tournament modules. |
| Rust crate | `rust/crates/dl-tournament/src/balance_cmd.rs:1` `!balance` port | Implements the old `!balance` prefix command layer that the user wants removed. |
| Rust crate | `rust/crates/dl-tournament/src/balancer.rs:1` team split algorithm | Only supports `!balance`/old team-balancer behavior. |
| Rust crate | `rust/crates/dl-tournament/src/discord_ui.rs:1-9` `TurnierUi` | Ports `cogs/customgames/turnier.py` user/admin Discord UI for the old in-bot tournament. |
| Rust crate | `rust/crates/dl-tournament/src/store.rs:144-184` `TournamentStore::ensure_schema` | Owns old tournament tables `customgames_tournament_*`, `tournament_periods`, and `turnier_auth_tokens`. |
| Rust crate | `rust/crates/dl-tournament/src/web.rs:1-12` `TurnierWeb` | Hosts the old public tournament website on `:8767`, which Deadlock-Turniere does not consume. |
| Rust workspace | `rust/Cargo.toml:20`, `rust/Cargo.toml:95` | Remove workspace member and workspace dependency for `crates/dl-tournament`. |
| Rust bot deps | `rust/bin/dl-bot/Cargo.toml:27` | Remove `dl-tournament` dependency after old bot tournament wiring is deleted. |
| Rust bot wiring | `rust/bin/dl-bot/src/main.rs:341-404` `TurnierRoleGlue`/`TurnierUi` | Registers old `/turnier` Discord interaction UI and tournament store. |
| Rust bot wiring | `rust/bin/dl-bot/src/main.rs:406-412`, `:598-599` `BalanceCommands::new`/`spawn` | Registers the old `!balance` command listener. |
| Rust bot wiring | `rust/bin/dl-bot/src/main.rs:613-629` auto-balance loop | Periodic auto-balance loop depends on `dl_tournament::store::TournamentStore` and old signups. |
| Rust bot glue | `rust/bin/dl-bot/src/modglue.rs:1928-2122` `BalanceGlue` | Discord adapter glue exists only for `dl_tournament::balance_cmd::BalancePort`. |
| Rust feature registry | `rust/bin/dl-bot/src/master.rs:31-32` `"tournament"`, `"team_balance"` | Remove or rename feature-status entries after the old commands disappear. |
| Rust web deps | `rust/bin/dl-web/Cargo.toml:15` | Remove `dl-tournament` dependency after `:8767` old tournament web is removed. |
| Rust web service | `rust/bin/dl-web/src/main.rs:60-75`, `rust/bin/dl-web/src/main.rs:99` | Remove old `TurnierWeb` listener on `TURNIER_PUBLIC_HOST:TURNIER_PUBLIC_PORT` and its `tokio::select!` branch. |
| Rust config | `rust/crates/dl-core/src/config.rs:34-35`, `:76`, `:121` `turnier_public` | Remove `TURNIER_PUBLIC_PORT` from Rust shared port config if no non-Bot consumer remains. |
| Rust dashboard deps | `rust/crates/dl-dashboard/Cargo.toml:14` | Remove `dl-tournament` dependency after old admin routes are removed. |
| Rust dashboard module | `rust/crates/dl-dashboard/src/lib.rs:38` `pub mod tournament` | Remove module export for old dashboard admin handlers. |
| Rust dashboard admin | `rust/crates/dl-dashboard/src/tournament.rs:1-459` | Whole file is old `/api/turnier/*` admin logic backed by `dl_tournament::store`. |
| Rust dashboard route | `rust/crates/dl-dashboard/src/web.rs:260` `/turnier` | Remove old admin SPA route; Bridge/OAuth routes in the same file are KEEP. |
| Rust dashboard API | `rust/crates/dl-dashboard/src/web.rs:348-364` `/api/turnier/*` | Remove old admin mutation/overview/bracket route block. |
| Rust dashboard page | `rust/crates/dl-dashboard/src/web.rs:478-497` `turnier_page` | Remove old `turnier.html` page renderer after `/turnier` is removed. |
| Rust dashboard config | `rust/crates/dl-dashboard/src/config.rs:66-67`, `:137-139` `tournament_default_guild` | Used for old Turnier admin routes via `TURNIER_PUBLIC_GUILD_ID`; remove with those routes. |
| Python package | `cogs/customgames/__init__.py:1` | Package becomes empty once old customgames tournament cog/store are removed. |
| Python store | `cogs/customgames/tournament_store.py:30-32`, `:91-149`, `:157-684` | Owns all old tournament tables and CRUD used by `/turnier`, public site, and `!balance`. |
| Python slash cog | `cogs/customgames/turnier.py:1-7`, `:21`, `:981-991`, `:1004-1137`, `:1247-1251`, `:1340` | Old `/turnier` signup/admin cog, persistent views, and auto-balance loop. |
| Python balancer | `cogs/deadlock_team_balancer.py:1-6`, `:19`, `:99-135`, `:564-581`, `:637-704`, `:734-928`, `:1029-1120`, `:1151-1153` | Old `!balance` command set plus tournament panel/status/list subcommands. |
| Python public cog | `cogs/turnier_public_cog.py:1`, `:22`, `:30-33`, `:58-59` | Starts/stops old public tournament website server on `:8767`. |
| Python public site | `service/turnier_public.py:1-9`, `:26`, `:34-36`, `:184-208`, `:308-343`, `:351-363`, `:438-694`, `:698-733` | Old public tournament website, OAuth token handoff, signup/team APIs, and bracket. |
| Python public asset | `service/static/turnier_public.html:1` | Static SPA for old `service/turnier_public.py` and Rust `dl_tournament::web`. |
| Python dashboard routes | `service/dashboard.py:451-465` route registration | Remove old `/api/tournament/*`, `/turnier`, and `/api/turnier/*` route registrations only. |
| Python dashboard admin | `service/dashboard.py:6065-6237` `_handle_tournament_*` | Legacy `/api/tournament/*` admin handlers import `cogs.customgames.tournament_store`. |
| Python dashboard page | `service/dashboard.py:6241-6268` `_load_turnier_html`/`_handle_turnier_page` | Serves old `turnier.html` dashboard SPA. |
| Python dashboard bracket | `service/dashboard.py:6270-6384` `_generate_bracket` | Only used by old `/api/turnier/bracket`. |
| Python dashboard API | `service/dashboard.py:6386-6590` `_handle_turnier_*` | Old `/api/turnier/*` admin handlers import `cogs.customgames.tournament_store`. |
| Python admin auth | `service/dashboard.py:1187-1203` `_check_turnier_auth` | Auth wrapper exists for old `/turnier` admin routes; see AMBIGUOUS for `turnier_only` role policy before deleting all related auth. |
| Python admin asset | `service/static/turnier.html:1`, `:285`, `:355-490` | Old admin SPA calls `/api/turnier/*`. |
| Python dashboard asset | `service/static/dashboard.html:1472`, `:1669-1692`, `:2009-2017`, `:2805-2835`, `:3837-3989` | Old embedded tournament dashboard tab calls `/api/tournament/*`. |
| Python loader note | `bot_core/cog_loader.py:257-305`, `cog_blocklist.json:1` | Auto-discovery means deleting the cog files is enough; no fixed loader list or blocklist entry is required. |
| Docs cleanup | `docs/voice-features.md:12`, `:31`, `:38` | User-facing docs still advertise `!balance` and should be updated with the removal. |
| Docs cleanup | `docs/tierlist-und-builds.md:30-31` | Docs incorrectly couple Tierlist with `turnier_public.py`/Tournament Store and should be corrected without removing Tierlist. |
| Rust docs cleanup | `rust/docs/00-rewrite-plan.md:15`, `:43`, `:56`, `:99`; `rust/docs/06-cutover-bot.md:35-51`; `rust/docs/07-dashboard-plan.md:53-57` | Rust planning docs still describe old `dl-tournament`, `:8767`, and Turnier-Admin as active migration targets. |

## KEEP / Bridge

| Area | Datei:Zeile / Symbol | Begründung |
| --- | --- | --- |
| Tierlist Rust | `rust/bin/dl-web/src/main.rs:26-49`, `rust/bin/dl-web/src/main.rs:97` `dl-tierlist` on `:8771` | Explicit KEEP: this is the public Tierlist service, not the old tournament web. |
| Tierlist crate | `rust/crates/dl-tierlist/src/lib.rs:1-4`, `rust/crates/dl-tierlist/src/lib.rs:63-64` | Explicit KEEP: `dl-tierlist` implements `/api/tierlist` and `/api/tierlist/history`. |
| Tierlist Python | `cogs/tierlist_public_cog.py:1`, `:26`; `service/tierlist_public.py:22`, `:208-209`, `:427` | Explicit KEEP: live Python Tierlist and its dashboard-session validator are separate from tournament removal. |
| Tierlist DB | `service/db.py:560-617`, `rust/docs/db-schema.sql:1073-1118` `tierlist_*` | Explicit KEEP: these tables back Tierlist/build votes and are unrelated to old tournament signup. |
| Dashboard session | `service/dashboard.py:1090-1107` `validate_discord_session` | KEEP because Tierlist/public web validates dashboard Discord sessions through this function. |
| OAuth internal token guard | `service/dashboard.py:651-656`, `service/dashboard.py:671-682` | KEEP because Deadlock-Turniere authenticates internal OAuth calls with `X-Internal-Token`; the `turnier` token name is legacy naming, not old tournament admin. |
| Python OAuth routes | `service/dashboard.py:381-384`, `service/dashboard.py:2161-2212`, `service/dashboard.py:2334-2404` | KEEP because Deadlock-Turniere calls `/internal/v1/discord/initiate` and `/internal/v1/discord/consume-result`. |
| Rust OAuth routes | `rust/crates/dl-dashboard/src/web.rs:266-267`, `rust/crates/dl-dashboard/src/web.rs:794-850`, `rust/crates/dl-dashboard/src/web.rs:850-951` | KEEP in the Rust dashboard port for the same delegated Discord OAuth contract. |
| Legacy/specific OAuth delegation | `service/dashboard.py:387-392`, `service/dashboard.py:2506-2586`; `rust/crates/dl-dashboard/src/web.rs:269-275`, `:956-1010` | KEEP unless separately audited: user explicitly marked `/internal/turnier/v1/discord/authorize-url` and `/session` as Bridge/OAuth, even though current Deadlock-Turniere cross-read uses `/internal/v1/discord/*`. |
| Steam-link session bridge | `rust/crates/dl-dashboard/src/web.rs:277-279`, `:1057-1096` | KEEP: read-only/identity bridge around Discord session to Steam linking, not old tournament admin. |
| OAuth state DB | `service/db.py:1074-1084`, `service/db.py:1731-1825`; `rust/docs/db-schema.sql:645-646`, `:1542-1546` | KEEP because delegated OAuth stores/validates `oauth_states`. |
| Broker Python live | `bot_core/master_bot.py:207-225`, `bot_core/master_bot.py:614-627`; `service/master_broker.py:269-341` | KEEP because live Python master-broker serves `/internal/master/v1/discord/*` for external consumers. |
| Broker Rust port | `rust/bin/dl-bot/src/main.rs:542-565`; `rust/crates/dl-broker/src/lib.rs:141-224` | KEEP because Rust `dl-broker` is the master-broker replacement and is not old tournament admin. |
| Broker health | `rust/crates/dl-broker/src/lib.rs:143`, `rust/crates/dl-broker/src/handlers.rs:57-68`; `service/master_broker.py:269` | KEEP: internal broker health endpoint for service checks. |
| Broker create-channel | `rust/crates/dl-broker/src/lib.rs:185-186`, `rust/crates/dl-broker/src/handlers.rs:614-715`; `backend/notifications/discord_notifier.py:168` | KEEP because Deadlock-Turniere creates match text channels through the broker. |
| Broker delete-channel | `rust/crates/dl-broker/src/lib.rs:189-190`, `service/master_broker.py:297-299`; `backend/notifications/discord_notifier.py:227` | KEEP because Deadlock-Turniere deletes match channels through the broker. |
| Broker send-rich-message | `rust/crates/dl-broker/src/lib.rs:193-194`, `rust/crates/dl-broker/src/handlers.rs:855-890`; `backend/notifications/discord_notifier.py:214`, `:410`, `:462`, `:497` | KEEP because Deadlock-Turniere posts lobby announcements and match stats through the broker. |
| Broker send-message | `rust/crates/dl-broker/src/lib.rs:181-182`, `service/master_broker.py:287`; `backend/notifications/discord_notifier.py:296`, `:482` | KEEP because Deadlock-Turniere notifies players/casters through broker messages. |
| Broker move-voice | `rust/crates/dl-broker/src/lib.rs:213-214`, `rust/crates/dl-broker/src/handlers.rs:1211-1313`; `backend/notifications/discord_notifier.py:335` | KEEP because Deadlock-Turniere moves users to match voice channels through the broker. |
| Broker voice members | `rust/crates/dl-broker/src/lib.rs:217-218`, `rust/crates/dl-broker/src/handlers.rs:1315-1335`; `backend/notifications/discord_notifier.py:346` | KEEP because Deadlock-Turniere reads voice-channel membership through the broker. |
| Broker role members contract | `rust/crates/dl-broker/src/lib.rs:149-150`, `service/master_broker.py:272-274`; `backend/notifications/discord_notifier.py:355` | KEEP role-member lookup capability; see AMBIGUOUS for the current path spelling mismatch. |
| Steam links DB | `service/db.py:671-688`, `rust/docs/db-schema.sql:831-843`; `backend/steam/reader.py:35-41` | KEEP because Deadlock-Turniere reads verified Steam/rank links from `steam_links`. |
| Steam tasks DB | `service/db.py:874-887`, `service/db.py:1198-1199`, `rust/docs/db-schema.sql:950-951`; `backend/match/steam_bridge.py:24-28` | KEEP until replaced because Deadlock-Turniere writes/reads `steam_tasks` in the Bot DB. |
| Rank role DB | `rust/docs/db-schema.sql:433-434`, `rust/docs/db-schema.sql:1469-1471`; `backend/rank_reader.py:173-180` | KEEP because Deadlock-Turniere reads `deadlock_subrank_roles` for Discord rank role mapping. |
| Shared token env aliases | `backend/config.py:202-227`, `rust/bin/dl-bot/src/main.rs:542-546` | KEEP names such as `TURNIER_INTERNAL_API_TOKEN` and `MASTER_BROKER_TOKEN`; the `TURNIER_` prefix is legacy bridge naming, not old admin UI. |

## AMBIGUOUS

| Area | Datei:Zeile / Symbol | Begründung |
| --- | --- | --- |
| Broker port contract | `backend/config.py:211-217`; `rust/crates/dl-core/src/config.rs:38-39`, `:78`; `rust/bin/dl-bot/src/main.rs:556-557` | Deadlock-Turniere defaults broker base to `:8766`, while Deadlock-Bots master-broker defaults to `:8770`; live env likely resolves this, but secrets/env values were not read. |
| Broker role-members path | `backend/notifications/discord_notifier.py:355`; `rust/crates/dl-broker/src/lib.rs:149-150`; `service/master_broker.py:272-274` | Deadlock-Turniere calls `/discord/role/members`, while Bots exposes `/discord/role-members`; this is a Bridge mismatch, not a tournament-removal target. |
| `turnier_only` access level | `service/dashboard.py:1156-1185`, `:2444-2452`, `:2819-2821`; `rust/crates/dl-dashboard/src/config.rs:31-39`; `rust/crates/dl-dashboard/src/authority.rs:75-76`; `rust/crates/dl-dashboard/src/session.rs:232` | Looks old-admin-specific, but it is embedded in dashboard login/session semantics; remove only after deciding Community-Moderator dashboard access should disappear. |
| `TURNIER_MOD_ROLE_ID` | `service/dashboard.py:37`, `service/dashboard.py:1183`; `rust/crates/dl-dashboard/src/config.rs:14-15`, `:109-111` | Likely removable with `turnier_only`, but role policy is user/ops-facing rather than purely code-local. |
| Root redirect for turnier-only | `rust/crates/dl-dashboard/src/web.rs:452-466`; `service/dashboard.py:2819-2821` | Removing `/turnier` without changing this redirect creates a dead-end for existing `turnier_only` sessions. |
| `TURNIER_PUBLIC_GUILD_ID` env | `service/turnier_public.py:36`, `service/turnier_public.py:289-290`; `rust/crates/dl-dashboard/src/config.rs:137-139` | Remove for old Bot tournament, but audit systemd/env docs first so it is not confused with the new Deadlock-Turniere deployment. |
| `TURNIER_PUBLIC_URL` env | `backend/config.py:265-266`, `backend/auth/discord_oauth.py:86` | Do not remove from Deadlock-Turniere; same prefix as old Bot tournament but belongs to the new frontend OAuth completion URL. |
| `turnier_auth_tokens` table | `cogs/customgames/tournament_store.py:128-133`, `service/turnier_public.py:308-343`, `rust/docs/db-schema.sql:1142-1143` | Old public site only, but defer DB drop until after code removal and backup/retention decision. |
| Old tournament DB tables | `cogs/customgames/tournament_store.py:91-149`, `rust/crates/dl-tournament/src/store.rs:144-184`, `rust/docs/db-schema.sql:368-384`, `:1129-1143` | Tables appear old-only, but DB cleanup should be a later migration, not part of code removal. |
| Static `/turnier` references in Caddy/systemd | not found in repo code; requires ops config check | Cross-read did not check live reverse-proxy/systemd unit files; old `:8767` might still be routed externally. |
| Rust docs DB owner note | `rust/docs/01-db-contract.md:42`, `rust/docs/01-db-contract.md:47` | Mentions `dl-tournament` as owner for some DB groups; update carefully because builds/Tierlist tables must remain. |
| Archived docs | `docs/_archive/admin_commands.md:63-69`, `docs/_archive/DOKU_SPIELER.md:89`, `docs/_archive/BOT_DOKUMENTATION.md:34-42` | Archive content can stay historical or be annotated; not a runtime removal blocker. |

## Build/Import-Bruchstellen, die beim Entfernen mit-angefasst werden müssen

- Rust workspace must remove `crates/dl-tournament` from `rust/Cargo.toml:20` and workspace dependency `rust/Cargo.toml:95`; otherwise Cargo still expects the crate.
- Rust `dl-bot` must remove `dl-tournament` dependency and all references in `rust/bin/dl-bot/src/main.rs:341-412`, `:598-629`, plus `rust/bin/dl-bot/src/modglue.rs:1928-2122`; otherwise `dl_tournament::...` imports fail.
- Rust `dl-web` must remove `dl-tournament` dependency and the `turnier_server` branch in `rust/bin/dl-web/src/main.rs:60-75`, `:99`; keep `dl-tierlist` branch `:26-49`.
- Rust `dl-dashboard` must remove `pub mod tournament`, `src/tournament.rs`, and route refs in `src/web.rs:348-364`; keep `/internal/v1/discord/*`, `/internal/turnier/v1/discord/*`, and broker-based auth helpers.
- Rust shared config should remove `turnier_public` port only after every caller of `cfg.ports.turnier_public` is gone; tests at `rust/crates/dl-core/src/config.rs:121` must change with it.
- Python auto-discovery loads files with `setup()` via `bot_core/cog_loader.py:257-305`; deleting `cogs/deadlock_team_balancer.py`, `cogs/turnier_public_cog.py`, and `cogs/customgames/turnier.py` is sufficient, no blocklist edit is required.
- Python imports of `cogs.customgames.tournament_store` remain in `cogs/deadlock_team_balancer.py:19`, `cogs/customgames/turnier.py:21`, `service/dashboard.py:6074-6579`, and `service/turnier_public.py:26`; all must disappear before deleting `tournament_store.py`.
- Python dashboard route table must remove old admin route registrations at `service/dashboard.py:451-465`; keep internal OAuth routes at `service/dashboard.py:381-392`.
- Static dashboard JS in `service/static/dashboard.html:3837-3989` calls `/api/tournament/*`; removing only backend routes leaves broken admin UI.
- Static admin SPA `service/static/turnier.html:285-490` calls `/api/turnier/*`; remove it with `/turnier`.
- Docs and runbook references to `:8767`, `dl-tournament`, and `!balance` must be updated so future cutovers do not re-enable the old service.
- Systemd/services should restart through `systemctl --user restart <service-name>` after actual removal, but this discovery did not restart anything.

## DB-Tabellen: später/separat, keine Drops in dieser Entfernung

Old tournament-only candidates after code removal:

- `customgames_tournament_teams` (`cogs/customgames/tournament_store.py:91-99`, `rust/crates/dl-tournament/src/store.rs:144-151`, `rust/docs/db-schema.sql:383-384`)
- `customgames_tournament_signups` (`cogs/customgames/tournament_store.py:101-114`, `rust/crates/dl-tournament/src/store.rs:153-166`, `rust/docs/db-schema.sql:368-380`)
- `tournament_periods` (`cogs/customgames/tournament_store.py:116-126`, `rust/crates/dl-tournament/src/store.rs:168-177`, `rust/docs/db-schema.sql:1129-1130`)
- `turnier_auth_tokens` (`cogs/customgames/tournament_store.py:128-133`, `rust/crates/dl-tournament/src/store.rs:179-184`, `rust/docs/db-schema.sql:1142-1143`)

Do not drop as part of this code removal:

- `steam_links` (`service/db.py:671-688`) because Deadlock-Turniere reads it through `backend/steam/reader.py:35-41`.
- `steam_tasks` (`service/db.py:874-887`) because Deadlock-Turniere uses it through `backend/match/steam_bridge.py:24-28`.
- `deadlock_subrank_roles` (`rust/docs/db-schema.sql:433-434`) because Deadlock-Turniere reads it through `backend/rank_reader.py:173-180`.
- `oauth_states` (`service/db.py:1074-1084`) because delegated OAuth uses `service/dashboard.py:2161-2404`.
- `tierlist_*` (`service/db.py:560-617`) because Tierlist is explicit KEEP.
