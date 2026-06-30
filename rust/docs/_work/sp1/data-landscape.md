# SP1 Daten-Landschaft

Stand: 2026-06-30. Quelle sind die vier SQLite-Tabelleninventare und das Patchnotes-Inventar unter `rust/docs/_work/sp1/inventory/`. Dieses Dokument zaehlt `deadlock.sqlite3` nur einmal; `steam-bot.tables.json` ist die Steam-Bot-Konsumenten-Sicht auf dieselbe physische Datei, keine zweite Datenbank. Patchnotes hat keine eigene SQLite-Datei, nutzt aber zur Laufzeit ebenfalls die geteilte `deadlock.sqlite3`. Referenzen: Issues #385, #386, #387; Eltern-Spec §5.2 und §7.

## 1. Physische DB-Uebersicht

| Persistenz | Pfad / Quelle | Projekte/Sichten | Tabellen | Befund |
|---|---|---|---:|---|
| SQLite | `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` | Deadlock-Bots (`dl-bot`, `dl-web`) + Deadlock-Steam-Bot (`steam-bot`, `steam-core`) + Patchnotes | 124 | Eine physische DB. `deadlock-bots.tables.json` und `steam-bot.tables.json` haben denselben `db_path`, dieselben 124 Tabellennamen und identische Row-/Column-Sicht; nur 8 `proposed_schema`-Zuordnungen weichen zwischen den Inventar-Sichten ab. Der laufende Patchnotes-Service haelt dieselbe Datei offen und nutzt Tabellen daraus. #387: `steam_links` ist bereits physisch geteilt. |
| SQLite | `/home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db` | Turniere | 38 | Eigene Turnier-DB. `_sqlx_migrations` ist Migrations-/Tooling-Meta, kein fachliches Modell. |
| SQLite | `/home/naniadm/Documents/Website/builds/backend/deadlock.db` | Website/golden-coaching | 21 | Eigene Website-DB mit Coaching-Plattformtabellen und Website-Meta/Tierlist-Content. |
| Dateien | Patchnotes-Inventar `rust/docs/_work/sp1/inventory/patchnotes.md` | Patchnotes | 0 eigene SQLite | Keine eigene SQLite-Datei im Patchnotes-Repo. Relevant sind `data/patch_signal_history.ndjson` und Config-/Content-Dateien; DB-Zugriffe gehen gegen die geteilte `deadlock.sqlite3`. |

Die physische Gesamtlandschaft umfasst damit 183 SQLite-Tabellenzeilen nach Dedup der geteilten DB: 124 in `deadlock.sqlite3`, 38 in `tournament.db`, 21 in `deadlock.db`. Inventarweit gibt es 178 eindeutige Tabellennamen, weil einige fachliche Namen in mehreren DBs vorkommen, z.B. `coaching_requests`, `coaches` und `coach_applications`.

### Pflicht-Verifikation laufende Halter der geteilten `deadlock.sqlite3`

Scan am 2026-06-30 ueber `/proc/*/fd` gegen `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` inklusive `-wal` und `-shm`. Ergebnis: **vollstaendige aktuelle Halterliste, keine weiteren PIDs gefunden**.

| systemd-Unit | PID | Binary / Cmdline | Repo / WorkingDirectory | Offene DB-FDs |
|---|---:|---|---|---|
| `deadlock-patchnotes.service` | 5049 | `/usr/bin/python3.12`; Cmdline `/home/naniadm/Documents/Deadlock-Bots/.venv/bin/python main.py` | `/home/naniadm/Documents/Deadlock--Patchnotes-Bot` | `deadlock.sqlite3`, `deadlock.sqlite3-wal`, `deadlock.sqlite3-shm` |
| `steam-core.service` | 569292 | `/home/naniadm/Documents/Deadlock-Steam-Bot/rust/target/release/steam-core` | `/home/naniadm/Documents/Deadlock-Steam-Bot/rust` | `deadlock.sqlite3` mehrfach, `deadlock.sqlite3-wal` mehrfach, `deadlock.sqlite3-shm` |
| `deadlock-bot-rust.service` | 1404746 | `/home/naniadm/Documents/Deadlock-Bots/rust/target/release/dl-bot` | `/home/naniadm/Documents/Deadlock-Bots` | `deadlock.sqlite3` mehrfach, `deadlock.sqlite3-wal`, `deadlock.sqlite3-shm` |
| `steam-bot.service` | 1471159 | `/home/naniadm/Documents/Deadlock-Steam-Bot/rust/target/release/steam-bot` | `/home/naniadm/Documents/Deadlock-Steam-Bot/rust` | `deadlock.sqlite3`, `deadlock.sqlite3-wal`, `deadlock.sqlite3-shm` |
| `deadlock-web-rust.service` | 2400092 | `/home/naniadm/Documents/Deadlock-Bots/rust/target/release/dl-web (deleted)`; Cmdline `/home/naniadm/Documents/Deadlock-Bots/rust/target/release/dl-web` | `/home/naniadm/Documents/Deadlock-Bots` | `deadlock.sqlite3` mehrfach, `deadlock.sqlite3-wal`, `deadlock.sqlite3-shm` |

### Pflicht-Verifikation Steam-Bot Shared-DB

Ergebnis: **bestaetigt**. Der laufende Steam-Bot nutzt `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`; es gibt keine eigene Steam-Bot-DB in der Unit, im laufenden Prozess oder im Rust-Startcode.

- Unit-Suche: `systemctl --user list-units --all --no-pager | grep -i steam` zeigt `steam-bot.service` und `steam-core.service` aktiv; `systemctl list-units --all --no-pager | grep -i steam` lieferte keinen System-Unit-Treffer.
- Unit: `steam-bot.service`, FragmentPath `/home/naniadm/.config/systemd/user/steam-bot.service`, `ExecStart=/usr/bin/bash -lc %h/Documents/Deadlock-Steam-Bot/rust/deploy/run-steam-bot.sh`.
- Unit-Env: `/home/naniadm/.config/systemd/user/steam-bot.service` setzt `Environment=DEADLOCK_DB_PATH=%h/Documents/Deadlock-Bots/data/deadlock.sqlite3`.
- Wrapper: `/home/naniadm/Documents/Deadlock-Steam-Bot/rust/deploy/run-steam-bot.sh` setzt `STEAM_BOT_BIN=$HOME/Documents/Deadlock-Steam-Bot/rust/target/release/steam-bot` und startet per `exec "$STEAM_BOT_BIN" "$@"`.
- Laufzeit: `MainPID=1471159`; `/proc/1471159/exe -> /home/naniadm/Documents/Deadlock-Steam-Bot/rust/target/release/steam-bot`; offene FDs: `fd/9 -> .../data/deadlock.sqlite3`, `fd/10 -> .../data/deadlock.sqlite3-wal`, `fd/11 -> .../data/deadlock.sqlite3-shm`.
- Rust-Code: `/home/naniadm/Documents/Deadlock-Steam-Bot/rust/crates/steam-bot/src/main.rs` liest `DEADLOCK_DB_PATH` und faellt sonst auf `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` zurueck; `Db::open(&config.db_path)` oeffnet genau diesen Pfad.

Korrektur fuer die Konsolidierung: `steam-bot.tables.json` nicht als separate DB zaehlen. Consumers pro Tabelle werden aus Deadlock-Bots- und Steam-Bot-Inventar zusammengefuehrt. SP3 ist deshalb keine separate Datenmigration, sondern ein Code-Rewrite auf die zentrale DB; Steam-Daten werden in SP1 migriert. Das korrigiert Eltern-Spec §2/§10.

### Inventarabweichungen innerhalb der geteilten DB

Row-Counts und Spalten sind zwischen Deadlock-Bots- und Steam-Bot-Inventar identisch. Abweichungen gibt es nur bei `proposed_schema`; die Matrix weist beide Werte aus:

| Tabelle | Abweichung |
|---|---|
| `deadlock_hero_builds` | proposed_schema OFFEN vs steam |
| `deadlock_party_members` | proposed_schema OFFEN vs steam |
| `deadlock_subrank_roles` | proposed_schema OFFEN vs steam |
| `live_player_state` | proposed_schema activity vs steam |
| `member_events` | proposed_schema activity vs OFFEN |
| `oauth_states` | proposed_schema OFFEN vs core |
| `user_privacy` | proposed_schema core vs OFFEN |
| `voice_stats` | proposed_schema activity vs OFFEN |

## 2. Tabelle x Projekt x proposed_schema-Matrix

`tot` ist aus den Inventar-Notizen abgeleitet: `ja` = als tot/Legacy oder OFFEN/tot markiert und ohne gemergte Consumer; `teilweise` = mindestens eine Sicht markiert tot/Legacy, aber andere gemergte Consumer existieren; `nein` = kein Tot-Hinweis; `Meta` = Tooling-Meta wie `_sqlx_migrations`.

### `deadlock.sqlite3` - geteilte physische DB
| Tabelle | Projekte/Sicht | Rows | merged consumers | proposed_schema | tot |
|---|---|---:|---|---|---|
| `ai_moderation_cases` | Deadlock-Bots + Steam-Bot | 36 | dl-moderation | OFFEN | teilweise |
| `ai_moderation_ragebait_hits` | Deadlock-Bots + Steam-Bot | 3 | dl-moderation | OFFEN | teilweise |
| `beta_invite_audit` | Deadlock-Bots + Steam-Bot | 129 | dl-community, steam-flows::betainvite::dispatch, steam-persistence, steam-persistence::betainvite | steam | nein |
| `beta_invite_auto_failure_alerts` | Deadlock-Bots + Steam-Bot | 3 | steam-persistence::betainvite | steam | teilweise |
| `beta_invite_friendship_auto_poll` | Deadlock-Bots + Steam-Bot | 10 | steam-flows::betainvite::friendship_poll, steam-persistence::betainvite | steam | teilweise |
| `beta_invite_intent` | Deadlock-Bots + Steam-Bot | 200 | dl-community, steam-persistence::betainvite | steam | nein |
| `beta_invite_panel_clicks` | Deadlock-Bots + Steam-Bot | 181 | - | steam | ja |
| `beta_invite_pending_payments` | Deadlock-Bots + Steam-Bot | 0 | steam-flows::supporter, steam-persistence::betainvite, steam-persistence::supporter | steam | teilweise |
| `beta_invite_supporter_role_grants` | Deadlock-Bots + Steam-Bot | 0 | steam-flows::supporter, steam-persistence::betainvite, steam-persistence::supporter | steam | teilweise |
| `beta_invite_tickets` | Deadlock-Bots + Steam-Bot | 101 | steam-persistence::betainvite | steam | teilweise |
| `changelog_entries` | Deadlock-Bots + Steam-Bot | 6290 | - | OFFEN | ja |
| `changelog_posts` | Deadlock-Bots + Steam-Bot + Patchnotes | 117 | dl-dashboard, Patchnotes::main | OFFEN | teilweise |
| `claimed_threads` | Deadlock-Bots + Steam-Bot | 0 | - | OFFEN | ja |
| `clip_contest_submissions` | Deadlock-Bots + Steam-Bot | 1 | - | OFFEN | ja |
| `clip_contests` | Deadlock-Bots + Steam-Bot | 1 | - | OFFEN | ja |
| `clip_fetch_history` | Deadlock-Bots + Steam-Bot | 2429 | - | OFFEN | ja |
| `clip_last_hashtags` | Deadlock-Bots + Steam-Bot | 0 | - | OFFEN | ja |
| `clip_submissions` | Deadlock-Bots + Steam-Bot | 25 | dl-community | OFFEN | teilweise |
| `clip_templates_global` | Deadlock-Bots + Steam-Bot | 5 | - | OFFEN | ja |
| `clip_templates_streamer` | Deadlock-Bots + Steam-Bot | 0 | - | OFFEN | ja |
| `clip_window_submissions` | Deadlock-Bots + Steam-Bot | 21 | dl-community | OFFEN | teilweise |
| `clip_windows` | Deadlock-Bots + Steam-Bot | 39 | dl-community | OFFEN | teilweise |
| `coach_applications` | Deadlock-Bots + Steam-Bot | 0 | - | coaching | ja |
| `coaches` | Deadlock-Bots + Steam-Bot | 0 | dl-bot, dl-community | coaching | teilweise |
| `coaching_bans` | Deadlock-Bots + Steam-Bot | 0 | dl-community | coaching | teilweise |
| `coaching_coach_rotation` | Deadlock-Bots + Steam-Bot | 0 | dl-community | coaching | teilweise |
| `coaching_requests` | Deadlock-Bots + Steam-Bot | 56 | dl-bot, dl-community, dl-db | coaching | teilweise |
| `coaching_sessions` | Deadlock-Bots + Steam-Bot | 43 | dl-community | coaching | teilweise |
| `coaching_sessions_legacy` | Deadlock-Bots + Steam-Bot | 2 | - | coaching | ja |
| `coaching_surveys` | Deadlock-Bots + Steam-Bot | 4 | - | coaching | ja |
| `customgames_tournament_signups` | Deadlock-Bots + Steam-Bot | 0 | - | OFFEN | ja |
| `customgames_tournament_teams` | Deadlock-Bots + Steam-Bot | 0 | - | OFFEN | ja |
| `deadlock_changelogs` | Deadlock-Bots + Steam-Bot + Patchnotes | 117 | Patchnotes::main | OFFEN | teilweise |
| `deadlock_hero_builds` | Deadlock-Bots + Steam-Bot | 71 | dl-dashboard, dl-tierlist, steam-core::task::handlers::builds, steam-core::task::handlers::builds::catalog, steam-persistence::builds | Deadlock-Bots=OFFEN / Steam-Bot=steam | nein |
| `deadlock_heroes` | Deadlock-Bots + Steam-Bot | 38 | dl-dashboard, dl-tierlist, steam-persistence::builds | OFFEN | nein |
| `deadlock_party_members` | Deadlock-Bots + Steam-Bot | 0 | dl-voice, steam-core, steam-core::steam::presence, steam-persistence, steam-persistence::party_members, steam-persistence::presence | Deadlock-Bots=OFFEN / Steam-Bot=steam | nein |
| `deadlock_subrank_roles` | Deadlock-Bots + Steam-Bot | 66 | steam-flows::rank, steam-persistence, steam-persistence::rank | Deadlock-Bots=OFFEN / Steam-Bot=steam | teilweise |
| `deadlock_voice_watch` | Deadlock-Bots + Steam-Bot | 0 | dl-community, dl-voice | OFFEN | teilweise |
| `discord_invite_codes` | Deadlock-Bots + Steam-Bot | 89 | - | OFFEN | ja |
| `dm_response_tracking` | Deadlock-Bots + Steam-Bot | 0 | dl-community | OFFEN | teilweise |
| `faq_chat_messages` | Deadlock-Bots + Steam-Bot | 44 | dl-community | OFFEN | teilweise |
| `faq_chat_sessions` | Deadlock-Bots + Steam-Bot | 13 | dl-community | OFFEN | teilweise |
| `hero_build_clones` | Deadlock-Bots + Steam-Bot | 68 | dl-bot, steam-core::task::handlers::builds, steam-core::task::handlers::builds::catalog, steam-core::task::handlers::builds::delete, steam-persistence, steam-persistence::builds | OFFEN | nein |
| `hero_build_sources` | Deadlock-Bots + Steam-Bot | 107 | dl-bot, steam-core::task::handlers::builds::catalog, steam-core::task::handlers::builds::convert, steam-core::task::handlers::builds::publish, steam-persistence, steam-persistence::builds | OFFEN | nein |
| `invite_snapshot_cache` | Deadlock-Bots + Steam-Bot | 1 | dl-discord | OFFEN | teilweise |
| `issue_reports` | Deadlock-Bots + Steam-Bot | 17 | dl-community | OFFEN | teilweise |
| `kv_store` | Deadlock-Bots + Steam-Bot + Patchnotes | 974 | dl-activity, dl-bot, dl-bridges, dl-community, dl-dashboard, dl-db, dl-twitch-invite-sync, dl-voice, steam-persistence::builds, Patchnotes::main | OFFEN | nein |
| `live_player_state` | Deadlock-Bots + Steam-Bot | 526 | dl-activity, dl-community, dl-db, dl-voice, steam-core, steam-core::steam::presence, steam-persistence, steam-persistence::presence | Deadlock-Bots=activity / Steam-Bot=steam | nein |
| `member_events` | Deadlock-Bots + Steam-Bot | 2946 | dl-activity, dl-community, dl-dashboard, dl-db, dl-stats, dl-twitch-invite-sync, steam-flows::leave_cleanup, steam-persistence, steam-persistence::miss_tracker | Deadlock-Bots=activity / Steam-Bot=OFFEN | nein |
| `member_leave_surveys` | Deadlock-Bots + Steam-Bot | 95 | dl-community, dl-dashboard | activity | teilweise |
| `message_activity` | Deadlock-Bots + Steam-Bot | 1705 | dl-activity, dl-community, dl-dashboard, dl-db | activity | teilweise |
| `notification_log` | Deadlock-Bots + Steam-Bot | 0 | dl-community | OFFEN | teilweise |
| `notification_queue` | Deadlock-Bots + Steam-Bot | 0 | dl-community | OFFEN | teilweise |
| `oauth_states` | Deadlock-Bots + Steam-Bot | 83 | dl-dashboard, steam-flows::field_crypto, steam-persistence::oauth_state | Deadlock-Bots=OFFEN / Steam-Bot=core | nein |
| `onboarding_pending_verify` | Deadlock-Bots + Steam-Bot | 289 | dl-bot, dl-community | OFFEN | teilweise |
| `persistent_views` | Deadlock-Bots + Steam-Bot | 5 | dl-community | OFFEN | teilweise |
| `reaction_role_dm_log` | Deadlock-Bots + Steam-Bot | 42 | dl-community, dl-dashboard, dl-db | OFFEN | teilweise |
| `reaction_role_mappings` | Deadlock-Bots + Steam-Bot | 1 | dl-community, dl-dashboard, dl-db | OFFEN | teilweise |
| `rename_global_state` | Deadlock-Bots + Steam-Bot | 1 | - | OFFEN | ja |
| `rename_requests` | Deadlock-Bots + Steam-Bot | 17447 | dl-voice | OFFEN | teilweise |
| `router_user_prefs` | Deadlock-Bots + Steam-Bot | 4 | dl-voice | OFFEN | teilweise |
| `schema_version` | Deadlock-Bots + Steam-Bot | 1 | - | OFFEN | ja |
| `scrim_match` | Deadlock-Bots + Steam-Bot | 2 | dl-db, dl-squads | scrim | teilweise |
| `scrim_participant` | Deadlock-Bots + Steam-Bot | 29 | dl-community, dl-db, dl-squads | scrim | teilweise |
| `scrim_team` | Deadlock-Bots + Steam-Bot | 4 | dl-db, dl-squads | scrim | teilweise |
| `scrim_team_member` | Deadlock-Bots + Steam-Bot | 24 | dl-db, dl-squads | scrim | teilweise |
| `security_guard_incidents` | Deadlock-Bots + Steam-Bot | 11 | dl-moderation | OFFEN | teilweise |
| `server_faq_logs` | Deadlock-Bots + Steam-Bot | 35 | dl-community | OFFEN | teilweise |
| `standalone_bot_state` | Deadlock-Bots + Steam-Bot | 2 | dl-bot, steam-core, steam-core::steam::heartbeat, steam-persistence, steam-persistence::bot_state | OFFEN | nein |
| `standalone_commands` | Deadlock-Bots + Steam-Bot | 25 | steam-core::command_loop, steam-persistence, steam-persistence::commands | OFFEN | teilweise |
| `steam_beta_invites` | Deadlock-Bots + Steam-Bot | 157 | dl-community, steam-flows::betainvite::interactions, steam-persistence::betainvite | steam | nein |
| `steam_cleanup_poll_state` | Deadlock-Bots + Steam-Bot | 616 | dl-community, steam-persistence::miss_tracker | steam | nein |
| `steam_flow_throttle` | Deadlock-Bots + Steam-Bot | 93 | steam-flows::leave_cleanup, steam-persistence::throttle | steam | teilweise |
| `steam_friend_check_cache` | Deadlock-Bots + Steam-Bot | 614 | steam-persistence, steam-persistence::friend_cache | steam | teilweise |
| `steam_friend_requests` | Deadlock-Bots + Steam-Bot | 389 | dl-community, steam-core::task::handlers::friends, steam-flows::friend_sync, steam-flows::link, steam-persistence, steam-persistence::friends | steam | nein |
| `steam_friendship_miss_tracker` | Deadlock-Bots + Steam-Bot | 421 | dl-community, steam-flows::friend_sync, steam-persistence::links, steam-persistence::miss_tracker | steam | nein |
| `steam_launch_tokens` | Deadlock-Bots + Steam-Bot | 13541 | steam-flows::link, steam-persistence::oauth_state | steam | teilweise |
| `steam_links` | Deadlock-Bots + Steam-Bot | 509 | dl-activity, dl-bot, dl-bridges, dl-central-db, dl-community, dl-db, dl-stats, dl-voice, steam-core::task::handlers::friends, steam-flows::betainvite::friendship_poll, steam-flows::friend_sync, steam-flows::leave_cleanup, steam-flows::link, steam-flows::link_commands, steam-flows::rank, steam-persistence, steam-persistence::betainvite, steam-persistence::friends, steam-persistence::links, steam-persistence::miss_tracker, steam-persistence::rank, steam-web::routes::events, steam-web::routes::link | core | nein |
| `steam_links_archive` | Deadlock-Bots + Steam-Bot | 299 | steam-persistence::links, steam-persistence::role_cleanup | steam | teilweise |
| `steam_links_leave_archive` | Deadlock-Bots + Steam-Bot | 99 | steam-flows::leave_cleanup, steam-persistence, steam-persistence::role_cleanup | steam | teilweise |
| `steam_nudge_state` | Deadlock-Bots + Steam-Bot | 384 | dl-community, dl-voice | steam | teilweise |
| `steam_presence_watchlist` | Deadlock-Bots + Steam-Bot | 0 | dl-community | steam | teilweise |
| `steam_quick_invites` | Deadlock-Bots + Steam-Bot | 0 | dl-community, steam-persistence::betainvite | steam | nein |
| `steam_rank_assignments` | Deadlock-Bots + Steam-Bot | 182 | steam-persistence, steam-persistence::rank | steam | teilweise |
| `steam_rank_history` | Deadlock-Bots + Steam-Bot | 3959 | steam-persistence::rank, steam-web::routes::rank | steam | teilweise |
| `steam_rich_presence` | Deadlock-Bots + Steam-Bot | 0 | dl-community | steam | teilweise |
| `steam_role_cleanup_pending` | Deadlock-Bots + Steam-Bot | 2 | dl-community, steam-flows::friend_sync, steam-persistence, steam-persistence::role_cleanup | steam | nein |
| `steam_tasks` | Deadlock-Bots + Steam-Bot | 1000 | dl-bot, steam-core, steam-core::api, steam-core::command_loop, steam-core::task, steam-core::task::runner, steam-flows::friend_sync, steam-persistence, steam-persistence::builds, steam-persistence::tasks | steam | nein |
| `streamer_link_intents` | Deadlock-Bots + Steam-Bot | 0 | dl-bot, dl-bridges | OFFEN | teilweise |
| `tempvoice_bans` | Deadlock-Bots + Steam-Bot | 27 | dl-community, dl-voice | OFFEN | teilweise |
| `tempvoice_interface` | Deadlock-Bots + Steam-Bot | 3 | dl-bot, dl-voice | OFFEN | teilweise |
| `tempvoice_lane_tag_filter` | Deadlock-Bots + Steam-Bot | 1 | dl-moderation, dl-voice | OFFEN | teilweise |
| `tempvoice_lanes` | Deadlock-Bots + Steam-Bot | 0 | dl-community, dl-voice | OFFEN | teilweise |
| `tempvoice_lurkers` | Deadlock-Bots + Steam-Bot | 0 | dl-community, dl-voice | OFFEN | teilweise |
| `tempvoice_owner_prefs` | Deadlock-Bots + Steam-Bot | 46 | dl-community, dl-voice | OFFEN | teilweise |
| `tempvoice_presets` | Deadlock-Bots + Steam-Bot | 13 | dl-voice | OFFEN | teilweise |
| `tempvoice_rank_pref` | Deadlock-Bots + Steam-Bot | 7 | dl-voice | OFFEN | teilweise |
| `tempvoice_staging_channels` | Deadlock-Bots + Steam-Bot | 6 | - | OFFEN | ja |
| `text_conversation_log` | Deadlock-Bots + Steam-Bot | 4078 | dl-activity, dl-db, dl-stats | activity | teilweise |
| `text_stats` | Deadlock-Bots + Steam-Bot | 578 | dl-activity, dl-bot, dl-db, dl-stats | activity | teilweise |
| `tierlist_build_votes` | Deadlock-Bots + Steam-Bot | 0 | dl-tierlist | OFFEN | teilweise |
| `tierlist_hero_meta` | Deadlock-Bots + Steam-Bot | 0 | dl-tierlist | OFFEN | teilweise |
| `tierlist_settings` | Deadlock-Bots + Steam-Bot | 5 | dl-tierlist | OFFEN | teilweise |
| `tierlist_snapshot_heroes` | Deadlock-Bots + Steam-Bot | 3420 | dl-tierlist | OFFEN | teilweise |
| `tierlist_snapshots` | Deadlock-Bots + Steam-Bot | 90 | dl-tierlist | OFFEN | teilweise |
| `tierlist_streamers` | Deadlock-Bots + Steam-Bot | 0 | dl-tierlist | OFFEN | teilweise |
| `tournament_periods` | Deadlock-Bots + Steam-Bot | 3 | - | OFFEN | ja |
| `turnier_auth_tokens` | Deadlock-Bots + Steam-Bot | 0 | - | OFFEN | ja |
| `twitch_streamer_invites` | Deadlock-Bots + Steam-Bot | 84 | dl-activity, dl-dashboard, dl-twitch-invite-sync | OFFEN | teilweise |
| `user_activity_patterns` | Deadlock-Bots + Steam-Bot | 787 | dl-activity, dl-community, dl-db | activity | teilweise |
| `user_co_players` | Deadlock-Bots + Steam-Bot | 25178 | dl-activity, dl-community, dl-dashboard, dl-db, dl-stats | activity | teilweise |
| `user_data` | Deadlock-Bots + Steam-Bot | 0 | dl-community | core | teilweise |
| `user_mod_tags` | Deadlock-Bots + Steam-Bot | 0 | dl-community | core | teilweise |
| `user_privacy` | Deadlock-Bots + Steam-Bot | 0 | dl-activity, dl-community, dl-db, dl-voice, steam-persistence | Deadlock-Bots=core / Steam-Bot=OFFEN | nein |
| `user_retention_messages` | Deadlock-Bots + Steam-Bot | 1664 | dl-community, dl-dashboard | activity | teilweise |
| `user_retention_tracking` | Deadlock-Bots + Steam-Bot | 890 | dl-bot, dl-community, dl-dashboard | activity | teilweise |
| `user_tags` | Deadlock-Bots + Steam-Bot | 190 | dl-community, dl-voice | core | teilweise |
| `voice_channel_anchors` | Deadlock-Bots + Steam-Bot | 34 | dl-community, dl-voice | OFFEN | teilweise |
| `voice_channel_settings` | Deadlock-Bots + Steam-Bot | 385 | dl-voice | OFFEN | teilweise |
| `voice_feedback_requests` | Deadlock-Bots + Steam-Bot | 457 | dl-community, dl-voice | activity | teilweise |
| `voice_feedback_responses` | Deadlock-Bots + Steam-Bot | 31 | dl-community, dl-voice | activity | teilweise |
| `voice_session_log` | Deadlock-Bots + Steam-Bot | 63844 | dl-activity, dl-community, dl-dashboard, dl-db, dl-stats, dl-voice | activity | teilweise |
| `voice_stats` | Deadlock-Bots + Steam-Bot | 864 | dl-activity, dl-bot, dl-community, dl-dashboard, dl-db, dl-stats, dl-voice, steam-flows::purge, steam-persistence, steam-persistence::friends | Deadlock-Bots=activity / Steam-Bot=OFFEN | nein |
| `watched_build_authors` | Deadlock-Bots + Steam-Bot | 10 | dl-bot, steam-persistence::builds | OFFEN | nein |

### `tournament.db` - Turniere
| Tabelle | Projekte/Sicht | Rows | merged consumers | proposed_schema | tot |
|---|---|---:|---|---|---|
| `_sqlx_migrations` | Turniere | 2 | - | OFFEN | Meta |
| `audit_log` | Turniere | 181 | backend.db, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.operations_routes, backend.tournament.routes, backend.tournament.scheduler, turnier-api, turnier-engine, turnier-scheduler | turnier | nein |
| `bracket_matches` | Turniere | 25 | backend.admin.test_mode, backend.db, backend.draft.routes, backend.match.auto_lobby, backend.match.game_modes, backend.match.manager, backend.match.result_processor, backend.match.series_manager, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.mini_groups, backend.tournament.models, backend.tournament.operations_routes, backend.tournament.points, backend.tournament.routes, backend.tournament.scheduler, turnier-api, turnier-core, turnier-engine, turnier-match, turnier-scheduler | turnier | nein |
| `bracket_mini_group_teams` | Turniere | 0 | backend.db, backend.tournament.engine, backend.tournament.mini_groups, backend.tournament.routes, turnier-api, turnier-engine | turnier | nein |
| `bracket_mini_groups` | Turniere | 0 | backend.db, backend.tournament.engine, backend.tournament.mini_groups, backend.tournament.routes, turnier-api, turnier-engine | turnier | nein |
| `checkins` | Turniere | 0 | backend.db, backend.tournament.admin_routes, backend.tournament.engine, turnier-api, turnier-engine | turnier | nein |
| `discord_tasks` | Turniere | 29 | backend.db, backend.notifications.discord_notifier, turnier-discord | turnier | nein |
| `draft_actions` | Turniere | 0 | backend.db, backend.draft.engine, turnier-draft | turnier | nein |
| `draft_sessions` | Turniere | 0 | backend.db, backend.draft.engine, turnier-draft | turnier | nein |
| `group_matches` | Turniere | 4 | backend.admin.test_mode, backend.db, backend.match.auto_lobby, backend.match.game_modes, backend.match.manager, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.routes, turnier-api, turnier-engine, turnier-match | turnier | nein |
| `group_teams` | Turniere | 5 | backend.db, backend.match.manager, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.routes, turnier-api, turnier-engine, turnier-match | turnier | nein |
| `groups` | Turniere | 2 | backend.admin.test_mode, backend.db, backend.match.auto_lobby, backend.match.game_modes, backend.match.manager, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.models, backend.tournament.routes, turnier-api, turnier-core, turnier-engine, turnier-match | turnier | nein |
| `match_casters` | Turniere | 0 | backend.db, backend.match.manager, backend.tournament.admin_routes, turnier-match | turnier | nein |
| `match_games` | Turniere | 0 | backend.db, backend.match.series_manager, turnier-api, turnier-match | turnier | nein |
| `match_result_reports` | Turniere | 0 | backend.db, backend.tournament.operations_routes, turnier-api | turnier | nein |
| `match_results` | Turniere | 18 | backend.db, backend.match.manager, backend.match.result_processor, backend.tournament.admin_routes, turnier-api, turnier-core, turnier-match | turnier | nein |
| `player_points` | Turniere | 15 | backend.admin.test_mode, backend.db, backend.tournament.leaderboard_routes, backend.tournament.points, turnier-api, turnier-engine | turnier | nein |
| `rank_cache` | Turniere | 25 | backend.admin.test_mode, backend.db, backend.rank_reader, backend.tournament.leaderboard_routes, turnier-api, turnier-core, turnier-steam | turnier | nein |
| `sent_match_reminders` | Turniere | 0 | backend.db, backend.tournament.scheduler, turnier-scheduler | turnier | nein |
| `sent_start_reminders` | Turniere | 0 | backend.db, backend.tournament.scheduler, turnier-scheduler | turnier | nein |
| `sent_tournament_reminders` | Turniere | 4 | backend.db, backend.tournament.scheduler, turnier-scheduler | turnier | nein |
| `sessions` | Turniere | 65 | backend.admin.test_mode, backend.auth.discord_oauth, backend.auth.middleware, backend.db, backend.draft.routes, backend.tournament.admin_routes, backend.tournament.consent_routes, backend.tournament.leaderboard_routes, backend.tournament.routes, turnier-api, turnier-auth | turnier | nein |
| `team_applications` | Turniere | 0 | backend.db, backend.tournament.admin_routes, backend.tournament.routes, turnier-api | turnier | nein |
| `team_invitations` | Turniere | 0 | backend.db, backend.tournament.routes, turnier-api | turnier | nein |
| `team_members` | Turniere | 26 | backend.admin.test_mode, backend.db, backend.match.game_modes, backend.match.manager, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.leaderboard_routes, backend.tournament.operations_routes, backend.tournament.points, backend.tournament.routes, backend.tournament.scheduler, turnier-api, turnier-engine, turnier-match, turnier-scheduler | turnier | nein |
| `teams` | Turniere | 21 | backend.admin.test_mode, backend.db, backend.match.game_modes, backend.match.manager, backend.match.result_processor, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.leaderboard_routes, backend.tournament.models, backend.tournament.operations_routes, backend.tournament.points, backend.tournament.routes, backend.tournament.scheduler, turnier-api, turnier-automatik, turnier-core, turnier-engine, turnier-match, turnier-scheduler | turnier | nein |
| `tournament_casters` | Turniere | 0 | backend.db, backend.match.manager, backend.tournament.admin_routes, turnier-api, turnier-match | turnier | nein |
| `tournament_checkins` | Turniere | 23 | backend.admin.test_mode, backend.db, backend.tournament.admin_routes, backend.tournament.engine, backend.tournament.routes, turnier-api, turnier-engine | turnier | nein |
| `tournament_dm_optout` | Turniere | 0 | turnier-automatik, turnier-discord | turnier | nein |
| `tournament_presets` | Turniere | 0 | turnier-automatik | turnier | nein |
| `tournament_proposal_feedback` | Turniere | 0 | turnier-automatik | turnier | nein |
| `tournament_proposal_votes` | Turniere | 0 | turnier-automatik | turnier | nein |
| `tournament_proposals` | Turniere | 0 | turnier-automatik | turnier | nein |
| `tournament_signals` | Turniere | 0 | turnier-automatik | turnier | nein |
| `tournament_signups` | Turniere | 31 | backend.admin.test_mode, backend.db, backend.tournament.admin_routes, backend.tournament.consent_routes, backend.tournament.engine, backend.tournament.routes, backend.tournament.scheduler, turnier-api, turnier-engine, turnier-scheduler | turnier | nein |
| `tournaments` | Turniere | 4 | backend.admin.test_mode, backend.db, backend.match.auto_lobby, backend.match.game_modes, backend.match.manager, backend.match.series_manager, backend.tournament.admin_routes, backend.tournament.consent_routes, backend.tournament.engine, backend.tournament.leaderboard_routes, backend.tournament.operations_routes, backend.tournament.routes, backend.tournament.scheduler, turnier-api, turnier-engine, turnier-match, turnier-scheduler | turnier | nein |
| `user_consents` | Turniere | 26 | backend.admin.test_mode, backend.db, backend.tournament.consent_routes, backend.tournament.routes, turnier-api | turnier | nein |
| `user_profiles` | Turniere | 3 | backend.admin.test_mode, backend.db, backend.notifications.discord_notifier, backend.tournament.admin_routes, backend.tournament.consent_routes, backend.tournament.leaderboard_routes, backend.tournament.routes, backend.tournament.scheduler, turnier-api, turnier-discord, turnier-scheduler | turnier | nein |

### `deadlock.db` - Website/golden-coaching
| Tabelle | Projekte/Sicht | Rows | merged consumers | proposed_schema | tot |
|---|---|---:|---|---|---|
| `coach_applications` | Website | 0 | app.database, app.routers.coaching | coaching | nein |
| `coach_reviews` | Website | 0 | app.database, app.routers.coaching | coaching | nein |
| `coachees` | Website | 14 | app.database, app.routers.coaching_platform, tests.test_appointments | coaching | nein |
| `coaches` | Website | 5 | app.database, app.routers.auth, app.routers.coaching, app.routers.coaching_platform, tests.test_appointments | coaching | nein |
| `coaching_appointments` | Website | 0 | app.database, app.routers.coaching_platform, tests.test_appointments | coaching | nein |
| `coaching_goals` | Website | 0 | app.database, app.routers.coaching_platform, tests.test_appointments | coaching | nein |
| `coaching_milestones` | Website | 0 | app.database, app.routers.coaching_platform, tests.test_appointments | coaching | nein |
| `coaching_requests` | Website | 21 | app.database, app.routers.coaching, app.routers.coaching_platform, tests.test_appointments | coaching | nein |
| `coaching_sessions` | Website | 16 | app.database, app.routers.coaching, app.routers.coaching_platform, tests.test_appointments | coaching | nein |
| `coaching_surveys` | Website | 0 | app.database, app.routers.coaching | coaching | nein |
| `meta_announcements` | Website | 0 | app.database, app.routers.admin | OFFEN | nein |
| `meta_builds` | Website | 5 | app.database, app.routers.admin, app.routers.builds | OFFEN | nein |
| `meta_heroes` | Website | 38 | app.database, app.routers.heroes | OFFEN | nein |
| `meta_items` | Website | 0 | app.database, app.routers.items | OFFEN | teilweise |
| `meta_patch_notes` | Website | 1 | app.database, app.routers.patchnotes | OFFEN | nein |
| `meta_reports` | Website | 0 | app.database, app.routers.admin, app.routers.builds | OFFEN | nein |
| `meta_tier_history` | Website | 0 | app.database, app.routers.history | OFFEN | teilweise |
| `meta_tier_lists` | Website | 1 | app.database, app.routers.tierlists | OFFEN | nein |
| `meta_users` | Website | 5 | app.database, app.routers.admin, app.routers.auth, tests.test_appointments | OFFEN | nein |
| `meta_votes` | Website | 0 | app.database, app.routers.admin | OFFEN | teilweise |
| `session_notes` | Website | 0 | app.database, app.routers.coaching_platform, tests.test_appointments | coaching | nein |

## 3. Cross-Projekt/Cross-DB-Ueberschneidungen und Zentralisierung

### `steam_links` - bereits physisch geteilt (#387)

`steam_links` liegt in `deadlock.sqlite3` und wird von Deadlock-Bots sowie Steam-Bot gelesen/geschrieben. Die gelebte Form ist breiter als SP0 `core.steam_links(discord_id BIGINT, steam_id64 BIGINT, verified BOOL, linked_at)`: SQLite nutzt `user_id INTEGER`, `steam_id TEXT`, zusammengesetzten PK `(user_id, steam_id)`, Owner-Guard-Trigger, Partial-Unique-Index `uq_steam_links_steam_owner` und Steam-/Deadlock-Rangfelder.

- Aktuelle Spalten: `user_id, steam_id, name, verified, primary_account, created_at, updated_at, legacy_ref, migrated_at, deadlock_rank, deadlock_rank_name, deadlock_subrank, deadlock_badge_level, deadlock_rank_updated_at, is_steam_friend`.
- Erhaltenswerte Constraints: `PRIMARY KEY(user_id, steam_id)`, `UNIQUE(steam_id) WHERE user_id != 0`, Owner-Guard fuer Insert/Update gegen Mehrfachbesitz derselben Steam-ID.
- Kanonischer Vorschlag `core.steam_links`: `discord_id BIGINT NOT NULL` als semantischer Alias fuer `user_id`; `steam_id TEXT NOT NULL` statt `steam_id64 BIGINT`; `steam_id64 BIGINT GENERATED/optional` nur wenn parsebar; `steam_display_name TEXT`; `verified BOOL NOT NULL DEFAULT false`; `primary_account BOOL NOT NULL DEFAULT false`; `linked_at TIMESTAMPTZ` aus `created_at`; `updated_at TIMESTAMPTZ`; `legacy_ref TEXT`; `migrated_at TIMESTAMPTZ`; `deadlock_rank INTEGER`; `deadlock_subrank INTEGER`; `deadlock_badge_level INTEGER`; `deadlock_rank_name TEXT`; `deadlock_rank_updated_at TIMESTAMPTZ`; `is_steam_friend BOOL NOT NULL DEFAULT false`. Primaerschluessel/Unique/Owner-Guard bleiben erhalten.

### `coaching_requests` - echter Cross-DB-Konflikt (#387)

`coaching_requests` existiert in `deadlock.sqlite3` und in der Website-DB `deadlock.db`, aber mit inkompatiblen Schluesseln und ergaenzenden Spaltensaetzen. Bot: `id INTEGER` plus Discord-Message/Rollen/Voice/Reward-/Scheduling-Felder. Website: `id TEXT` plus Plattformfelder `assigned_coach_username`, `bot_request_id`, `preferred_coach_id`, `notify_discord_at`.

- Bot-Spalten: `id, discord_user_id, discord_username, rank, subrank, hero, games_played, hours_played, availability, current_problems, ai_summary, ai_insights_json, status, message_id, channel_id, role_assigned_at, role_expires_at, role_removed_at, created_at, updated_at, scheduled_slot, assigned_coach_id, reserved_until, website_request_id, coachee_id`.
- Website-Spalten: `id, discord_user_id, discord_username, rank, subrank, hero, games_played, hours_played, availability, current_problems, ai_summary, ai_insights_json, status, created_at, updated_at, assigned_coach_id, assigned_coach_username, reserved_until, bot_request_id, preferred_coach_id, notify_discord_at`.
- Kanonischer Vorschlag `coaching.requests`: stabiler `request_uid TEXT PRIMARY KEY` als systemuebergreifender Schluessel; `bot_request_id INTEGER UNIQUE NULL`; `website_request_id TEXT UNIQUE NULL`; gemeinsame Felder `discord_user_id`, `discord_username`, `rank`, `subrank`, `hero`, `games_played`, `hours_played`, `availability`, `current_problems`, `ai_summary`, `ai_insights_json`, `status`, `created_at`, `updated_at`; Coach-/Claim-Felder `assigned_coach_id`, `assigned_coach_username`, `preferred_coach_id`, `reserved_until`, `scheduled_slot`, `coachee_id`; Discord-/Bot-Felder `message_id`, `channel_id`, `role_assigned_at`, `role_expires_at`, `role_removed_at`, `notify_discord_at`; optional Auditfelder fuer Quelle/Migrationsstatus. Bot-IDs bleiben als Legacy-/FK-Spalten erhalten, aber neue Writes adressieren `request_uid`.

### `coaches` und `coach_applications` - Spalten-Union, Wahrheit klaeren

Beide Namen existieren in `deadlock.sqlite3` und Website `deadlock.db`. Website hat aktive Coaching-/Auth-/Plattform-Consumer und 5 `coaches`-Rows; Bot-DB hat 0 Rows fuer `coaches` und `coach_applications`, aber Bot-/Community-Consumer bzw. Legacy-Notizen. Daher sollte Website/Plattform fuer Profile und Bewerbungen aktuell als operative Wahrheit gelten, waehrend die zentrale SP1-DB die Spalten-Union uebernimmt.

- `coaches` Bot-Spalten: `id, discord_user_id, discord_username, display_name, avatar_url, bio, specialties_json, availability_json, status, website_coach_id, created_at, updated_at, avg_rating, total_reviews, total_sessions`.
- `coaches` Website-Spalten: `id, discord_user_id, discord_username, display_name, avatar_url, bio, specialties_json, availability_json, status, avg_rating, total_reviews, total_sessions, created_at, updated_at, twitch_url`.
- Kanonisch `coaching.coaches`: `id TEXT PRIMARY KEY`, `discord_user_id BIGINT UNIQUE`, `discord_username`, `display_name`, `avatar_url`, `bio`, `specialties_json`, `availability_json`, `status`, `avg_rating`, `total_reviews`, `total_sessions`, `twitch_url`, `website_coach_id` optional/legacy, `created_at`, `updated_at`.
- `coach_applications` Bot-Spalten: `id, discord_user_id, discord_username, display_name, application_text, experience_text, rank, specialties_json, availability_json, status, reviewed_by, reviewed_at, created_at, updated_at`.
- `coach_applications` Website-Spalten: `id, discord_user_id, discord_username, display_name, application_text, experience_text, rank, specialties_json, availability_json, status, reviewed_by, reviewed_at, created_at, updated_at`.
- Kanonisch `coaching.coach_applications`: Spalten-Union mit `reviewed_by TEXT` als kanonisch; Bot-Integer-Werte werden als Text normalisiert oder in `reviewed_by_discord_id BIGINT` separiert, falls Query-Semantik das braucht.

## 4. Schema-Zuordnungs-Luecken

Physisch dedupliziert haben 82 Tabellen mindestens eine `OFFEN`-Zuordnung: 71 in `deadlock.sqlite3`, 10 in Website `deadlock.db`, 1 in `tournament.db`. Eltern-Spec §5.2 kennt nur `core`, `coaching`, `scrim`, `steam`, `turnier`, `patchnotes`, `activity`; die folgenden Cluster brauchen Phase-1-Entscheidung.

| Cluster | Vorschlag Namespace | Tabellen | Begruendung |
|---|---|---|---|
| TempVoice/Voice | `voice` | `deadlock_party_members`, `deadlock_subrank_roles`, `deadlock_voice_watch`, `rename_global_state`, `rename_requests`, `router_user_prefs`, `tempvoice_bans`, `tempvoice_interface`, `tempvoice_lane_tag_filter`, `tempvoice_lanes`, `tempvoice_lurkers`, `tempvoice_owner_prefs`, `tempvoice_presets`, `tempvoice_rank_pref`, `tempvoice_staging_channels`, `voice_channel_anchors`, `voice_channel_settings`, `voice_stats` | Eigene Domäne fuer dynamische Voice-Kanaele, Rename-Queue, Voice-Settings und Voice-Stats; nicht `activity`, weil viel Konfiguration/State enthalten ist. |
| Tierlist/Builds | `tierlist` | `deadlock_hero_builds`, `deadlock_heroes`, `hero_build_clones`, `hero_build_sources`, `tierlist_build_votes`, `tierlist_hero_meta`, `tierlist_settings`, `tierlist_snapshot_heroes`, `tierlist_snapshots`, `tierlist_streamers`, `watched_build_authors` | Hero-/Build-/Tierlist-Content und Sync-State; eigener Namespace vermeidet Vermischung mit `steam` oder `patchnotes`. |
| Moderation | `moderation` | `ai_moderation_cases`, `ai_moderation_ragebait_hits`, `security_guard_incidents` | AI-Moderation und SecurityGuard-Faelle sind fachlich Moderation, nicht Bot-Infra. |
| Bot-State/Infra | `bot` | `changelog_entries`, `claimed_threads`, `customgames_tournament_signups`, `customgames_tournament_teams`, `discord_invite_codes`, `dm_response_tracking`, `faq_chat_messages`, `faq_chat_sessions`, `invite_snapshot_cache`, `issue_reports`, `kv_store`, `notification_log`, `notification_queue`, `oauth_states`, `onboarding_pending_verify`, `persistent_views`, `reaction_role_dm_log`, `reaction_role_mappings`, `schema_version`, `server_faq_logs`, `standalone_bot_state`, `standalone_commands`, `streamer_link_intents`, `tournament_periods`, `turnier_auth_tokens`, `twitch_streamer_invites` | Technischer Bot-/Discord-/FAQ-/OAuth-/Invite-State; neuer Namespace `bot` oder alternativ `infra`. `kv_store` bleibt generisch, enthaelt aber u.a. Patchnotes-Keys im Namespace `patchnotes_bot`. Vorschlag: `bot`. |
| Patchnotes Runtime | `patchnotes` | `changelog_posts`, `deadlock_changelogs`; `kv_store`-Namespace `patchnotes_bot` | Aktiver `deadlock-patchnotes.service` liest/schreibt diese Tabellen in der geteilten `deadlock.sqlite3`; keine separate Patchnotes-DB. |
| Clips | `clips` | `clip_contest_submissions`, `clip_contests`, `clip_fetch_history`, `clip_last_hashtags`, `clip_submissions`, `clip_templates_global`, `clip_templates_streamer`, `clip_window_submissions`, `clip_windows` | Clip-Contest, Fetch-Historie, Templates und Submissions sind eine eigene Community-Content-Domaene. |
| Website Tierlist/Builds | `tierlist` | `meta_builds`, `meta_heroes`, `meta_items`, `meta_tier_history`, `meta_tier_lists`, `meta_votes` | Website-Meta fuer Builds, Heroes, Tierlists und Votes sollte mit Bot-Tierlist/Builds in denselben Namespace. |
| Website Patchnotes/Content | `patchnotes` | `meta_patch_notes` | `meta_patch_notes` gehoert zu `patchnotes`, auch wenn es aus Website-Meta kommt. |
| Website Admin/Reports | `content` | `meta_announcements`, `meta_reports` | Admin-Announcements und Reports sind Website-Content/Moderations-Meta; Vorschlag `content`, alternativ `moderation` fuer Reports. |
| Website Identity/Auth | `core` | `meta_users` | `meta_users` ist Auth-/Admin-Identitaet und passt besser zu `core` als zu Coaching. |
| Activity/Eventlog | `activity` | `member_events` | Event-/Zeitreihencharakter; `activity` ist bereits Eltern-Spec-konform. |
| Privacy/Core | `core` | `user_privacy` | User-Privacy/Deletion-Praferenz ist querschnittlich und sollte bei `core` liegen. |
| Turniere Tooling-Meta | `turnier_meta` | `_sqlx_migrations` | Tooling-Meta fuer sqlx; nicht als fachliche `turnier`-Tabelle modellieren. |

Entscheidungsliste nach physischer DB:

### OFFEN in `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`

| Tabelle | proposed_schema-Sicht | merged consumers | tot |
|---|---|---|---|
| `ai_moderation_cases` | OFFEN | dl-moderation | teilweise |
| `ai_moderation_ragebait_hits` | OFFEN | dl-moderation | teilweise |
| `changelog_entries` | OFFEN | - | ja |
| `changelog_posts` | OFFEN | dl-dashboard, Patchnotes::main | teilweise |
| `claimed_threads` | OFFEN | - | ja |
| `clip_contest_submissions` | OFFEN | - | ja |
| `clip_contests` | OFFEN | - | ja |
| `clip_fetch_history` | OFFEN | - | ja |
| `clip_last_hashtags` | OFFEN | - | ja |
| `clip_submissions` | OFFEN | dl-community | teilweise |
| `clip_templates_global` | OFFEN | - | ja |
| `clip_templates_streamer` | OFFEN | - | ja |
| `clip_window_submissions` | OFFEN | dl-community | teilweise |
| `clip_windows` | OFFEN | dl-community | teilweise |
| `customgames_tournament_signups` | OFFEN | - | ja |
| `customgames_tournament_teams` | OFFEN | - | ja |
| `deadlock_changelogs` | OFFEN | Patchnotes::main | teilweise |
| `deadlock_hero_builds` | Deadlock-Bots=OFFEN / Steam-Bot=steam | dl-dashboard, dl-tierlist, steam-core::task::handlers::builds, steam-core::task::handlers::builds::catalog, steam-persistence::builds | nein |
| `deadlock_heroes` | OFFEN | dl-dashboard, dl-tierlist, steam-persistence::builds | nein |
| `deadlock_party_members` | Deadlock-Bots=OFFEN / Steam-Bot=steam | dl-voice, steam-core, steam-core::steam::presence, steam-persistence, steam-persistence::party_members, steam-persistence::presence | nein |
| `deadlock_subrank_roles` | Deadlock-Bots=OFFEN / Steam-Bot=steam | steam-flows::rank, steam-persistence, steam-persistence::rank | teilweise |
| `deadlock_voice_watch` | OFFEN | dl-community, dl-voice | teilweise |
| `discord_invite_codes` | OFFEN | - | ja |
| `dm_response_tracking` | OFFEN | dl-community | teilweise |
| `faq_chat_messages` | OFFEN | dl-community | teilweise |
| `faq_chat_sessions` | OFFEN | dl-community | teilweise |
| `hero_build_clones` | OFFEN | dl-bot, steam-core::task::handlers::builds, steam-core::task::handlers::builds::catalog, steam-core::task::handlers::builds::delete, steam-persistence, steam-persistence::builds | nein |
| `hero_build_sources` | OFFEN | dl-bot, steam-core::task::handlers::builds::catalog, steam-core::task::handlers::builds::convert, steam-core::task::handlers::builds::publish, steam-persistence, steam-persistence::builds | nein |
| `invite_snapshot_cache` | OFFEN | dl-discord | teilweise |
| `issue_reports` | OFFEN | dl-community | teilweise |
| `kv_store` | OFFEN | dl-activity, dl-bot, dl-bridges, dl-community, dl-dashboard, dl-db, dl-twitch-invite-sync, dl-voice, steam-persistence::builds, Patchnotes::main | nein |
| `member_events` | Deadlock-Bots=activity / Steam-Bot=OFFEN | dl-activity, dl-community, dl-dashboard, dl-db, dl-stats, dl-twitch-invite-sync, steam-flows::leave_cleanup, steam-persistence, steam-persistence::miss_tracker | nein |
| `notification_log` | OFFEN | dl-community | teilweise |
| `notification_queue` | OFFEN | dl-community | teilweise |
| `oauth_states` | Deadlock-Bots=OFFEN / Steam-Bot=core | dl-dashboard, steam-flows::field_crypto, steam-persistence::oauth_state | nein |
| `onboarding_pending_verify` | OFFEN | dl-bot, dl-community | teilweise |
| `persistent_views` | OFFEN | dl-community | teilweise |
| `reaction_role_dm_log` | OFFEN | dl-community, dl-dashboard, dl-db | teilweise |
| `reaction_role_mappings` | OFFEN | dl-community, dl-dashboard, dl-db | teilweise |
| `rename_global_state` | OFFEN | - | ja |
| `rename_requests` | OFFEN | dl-voice | teilweise |
| `router_user_prefs` | OFFEN | dl-voice | teilweise |
| `schema_version` | OFFEN | - | ja |
| `security_guard_incidents` | OFFEN | dl-moderation | teilweise |
| `server_faq_logs` | OFFEN | dl-community | teilweise |
| `standalone_bot_state` | OFFEN | dl-bot, steam-core, steam-core::steam::heartbeat, steam-persistence, steam-persistence::bot_state | nein |
| `standalone_commands` | OFFEN | steam-core::command_loop, steam-persistence, steam-persistence::commands | teilweise |
| `streamer_link_intents` | OFFEN | dl-bot, dl-bridges | teilweise |
| `tempvoice_bans` | OFFEN | dl-community, dl-voice | teilweise |
| `tempvoice_interface` | OFFEN | dl-bot, dl-voice | teilweise |
| `tempvoice_lane_tag_filter` | OFFEN | dl-moderation, dl-voice | teilweise |
| `tempvoice_lanes` | OFFEN | dl-community, dl-voice | teilweise |
| `tempvoice_lurkers` | OFFEN | dl-community, dl-voice | teilweise |
| `tempvoice_owner_prefs` | OFFEN | dl-community, dl-voice | teilweise |
| `tempvoice_presets` | OFFEN | dl-voice | teilweise |
| `tempvoice_rank_pref` | OFFEN | dl-voice | teilweise |
| `tempvoice_staging_channels` | OFFEN | - | ja |
| `tierlist_build_votes` | OFFEN | dl-tierlist | teilweise |
| `tierlist_hero_meta` | OFFEN | dl-tierlist | teilweise |
| `tierlist_settings` | OFFEN | dl-tierlist | teilweise |
| `tierlist_snapshot_heroes` | OFFEN | dl-tierlist | teilweise |
| `tierlist_snapshots` | OFFEN | dl-tierlist | teilweise |
| `tierlist_streamers` | OFFEN | dl-tierlist | teilweise |
| `tournament_periods` | OFFEN | - | ja |
| `turnier_auth_tokens` | OFFEN | - | ja |
| `twitch_streamer_invites` | OFFEN | dl-activity, dl-dashboard, dl-twitch-invite-sync | teilweise |
| `user_privacy` | Deadlock-Bots=core / Steam-Bot=OFFEN | dl-activity, dl-community, dl-db, dl-voice, steam-persistence | nein |
| `voice_channel_anchors` | OFFEN | dl-community, dl-voice | teilweise |
| `voice_channel_settings` | OFFEN | dl-voice | teilweise |
| `voice_stats` | Deadlock-Bots=activity / Steam-Bot=OFFEN | dl-activity, dl-bot, dl-community, dl-dashboard, dl-db, dl-stats, dl-voice, steam-flows::purge, steam-persistence, steam-persistence::friends | nein |
| `watched_build_authors` | OFFEN | dl-bot, steam-persistence::builds | nein |

### OFFEN in `/home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db`

| Tabelle | proposed_schema-Sicht | merged consumers | tot |
|---|---|---|---|
| `_sqlx_migrations` | OFFEN | - | Meta |

### OFFEN in `/home/naniadm/Documents/Website/builds/backend/deadlock.db`

| Tabelle | proposed_schema-Sicht | merged consumers | tot |
|---|---|---|---|
| `meta_announcements` | OFFEN | app.database, app.routers.admin | nein |
| `meta_builds` | OFFEN | app.database, app.routers.admin, app.routers.builds | nein |
| `meta_heroes` | OFFEN | app.database, app.routers.heroes | nein |
| `meta_items` | OFFEN | app.database, app.routers.items | teilweise |
| `meta_patch_notes` | OFFEN | app.database, app.routers.patchnotes | nein |
| `meta_reports` | OFFEN | app.database, app.routers.admin, app.routers.builds | nein |
| `meta_tier_history` | OFFEN | app.database, app.routers.history | teilweise |
| `meta_tier_lists` | OFFEN | app.database, app.routers.tierlists | nein |
| `meta_users` | OFFEN | app.database, app.routers.admin, app.routers.auth, tests.test_appointments | nein |
| `meta_votes` | OFFEN | app.database, app.routers.admin | teilweise |

## 5. SP-Auswirkungen

- SP1 migriert die zentrale `deadlock.sqlite3`-Datenlandschaft inklusive Steam-Tabellen; `steam_links` ist schon heute eine physisch geteilte Tabelle und muss verlustfrei in `core.steam_links` ueberfuehrt werden.
- SP3 (Steam-Bot) bekommt **keine separate Datenmigration**. Der Steam-Bot muss auf die zentrale SP1-DB und die neuen kanonischen Tabellen/Views umgeschrieben werden.
- SP5 (Patchnotes) ist bereits Consumer/Writer der geteilten `deadlock.sqlite3`: `changelog_posts`, `deadlock_changelogs` und `kv_store(ns='patchnotes_bot')`. SP5 ist daher Python-zu-Rust-Port plus Umstellung auf die zentrale SP1-DB/kanonische Tabellen, **keine separate Patchnotes-DB**.
- Eltern-Spec §2/§10 muss entsprechend korrigiert werden: Steam-Bot ist Daten-Consumer/Writer derselben physischen DB, kein eigenes Persistenz-Silo.
- Eltern-Spec §5.2/§7 reicht fuer alle existierenden Tabellen nicht aus: `voice`, `tierlist`, `moderation`, `bot`, `clips` und ggf. `content`/`turnier_meta` muessen explizit entschieden oder in bestehende Namespaces integriert werden.
- #385/#386 bleiben relevant fuer Behalten-vs-Drop: `tot` markierte Tabellen sollten migriert werden, aber Drop erst nach separatem Legacy-/Consumer-Gate.

## 6. Patchnotes

Patchnotes hat **keine eigene SQLite-Datei**, ist aber nicht rein file-basiert: der laufende `deadlock-patchnotes.service` nutzt die geteilte `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`. Das Inventar verweist zusaetzlich auf `data/patch_signal_history.ndjson` plus Config-/Content-Dateien; SP1/SP5 muss die DB-Consumer in der zentralen DB erhalten.

| Tabelle in `deadlock.sqlite3` | Patchnotes-Nutzung | Evidenz |
|---|---|---|
| `changelog_posts` | read/write/DDL | `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/main.py` selektiert, erstellt, aktualisiert und inseriert diese Tabelle. |
| `deadlock_changelogs` | read/write/DDL, Legacy-Backfill | `main.py` erstellt die Tabelle bei Bedarf und schreibt Legacy-Backfills. |
| `kv_store` | read/write ueber Namespace `patchnotes_bot` | `main.py` nutzt `deadlock_db.get_kv/set_kv`; `service/db.py` mappt diese Calls auf `kv_store(ns,k,v)`. |

Inventarhinweis komprimiert: Inventar - Patchnotes; Scope: `/home/naniadm/Documents/Deadlock--Patchnotes-Bot`; Scan: `*.db`, `*.sqlite`, `*.sqlite3`, `*.db3`, `*.json`, `*.yaml`, `*.yml`, `*.csv` unter dem Repo plus alle Dateien unter `data/`; ausgeschlossen wurden `.git`, `node_modules`, `__pycache__`, `venv`, `.venv` und Cache-Verzeichnisse.; Ergebnis: Im Patchnotes-Repo existiert keine SQLite-Datei. Deshalb wurde keine `patchnotes.tables.json` erzeugt; das SP1-Gate entfaellt.; Persistenz-Artefakte im Repo; \| Pfad \| Typ \| Zweck \| Einordnung \|; \|---\|---\|---\|---\|; \| `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/.github/dependabot.yml` \| YAML \| GitHub Dependabot-Konfiguration. \| statisch/Config \|

## 7. Verweise

- Issue #385: Tabellen dauerhaft behalten trotz 0 Konsumenten/Feature offen.
- Issue #386: Legacy-/tot-Tabellen migrieren, Drop erst nach spaeterem Gate.
- Issue #387: Cross-SP-Konsolidierung, insbesondere `steam_links`, `coaching_requests`, `coaches`, `coach_applications`.
- Eltern-Spec §5.2: bisherige Namespace-Liste `core/coaching/scrim/steam/turnier/patchnotes/activity`.
- Eltern-Spec §7: Migrations-/Konsolidierungsregeln; wegen geteilter Steam-DB mit obiger SP3-Korrektur anwenden.
