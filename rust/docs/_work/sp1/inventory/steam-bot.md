# Steam-Bot SP1 Phase 0 Inventory

- Projekt: Steam-Bot
- Live-DB: `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`
- DB-Tech: rusqlite
- Live-Pfad-Befund: Im Repo `/home/naniadm/Documents/Deadlock-Steam-Bot` wurden keine `*.sqlite3`/`*.db`-Dateien gefunden; `steam-bot.service` und `steam-core.service` setzen `DEADLOCK_DB_PATH` auf diese geteilte DB, und `/proc/<pid>/fd` zeigt offene FDs auf `deadlock.sqlite3`, `-wal`, `-shm`.
- Konsumenten-Scope: wortgenaue Suche in `/home/naniadm/Documents/Deadlock-Steam-Bot/rust/crates/*/src` (plus top-level `src/`, falls vorhanden).

## Issue #387: steam_links

- Befund: `steam_links` existiert mit 509 Rows.
- Spalten/Schluessel exakt: user_id INTEGER notnull=1 pk=1, steam_id TEXT notnull=1 pk=2, name TEXT notnull=0 pk=0, verified INTEGER notnull=0 pk=0, primary_account INTEGER notnull=0 pk=0, created_at DATETIME notnull=0 pk=0, updated_at DATETIME notnull=0 pk=0, legacy_ref TEXT notnull=0 pk=0, migrated_at INTEGER notnull=0 pk=0, deadlock_rank INTEGER notnull=0 pk=0, deadlock_rank_name TEXT notnull=0 pk=0, deadlock_subrank INTEGER notnull=0 pk=0, deadlock_badge_level INTEGER notnull=0 pk=0, deadlock_rank_updated_at INTEGER notnull=0 pk=0, is_steam_friend INTEGER notnull=0 pk=0.
- Weitere Schluessel/Guards: `PRIMARY KEY(user_id, steam_id)`, `uq_steam_links_steam_owner(steam_id) WHERE user_id != 0`, Owner-Guard-Trigger fuer Insert/Update.
- Abgleich gegen SP0 `core.steam_links`: `discord_id BIGINT` entspricht semantisch `user_id INTEGER`; `steam_id64 BIGINT` weicht vom aktuellen `steam_id TEXT` ab; `verified BOOL` ist INTEGER-bool-kompatibel; `linked_at TIMESTAMPTZ` entspricht am ehesten `created_at DATETIME`. Ziel bleibt `core.steam_links` als einzige Wahrheit fuer DL-Bot SP1 und Steam-Bot SP3.

## proposed_schema: core

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `oauth_states` | 83 | 9 | steam-flows::field_crypto, steam-persistence::oauth_state | Steam-Link-OAuth-State; proposed core auth/state, final namespace in SP1 klaeren |
| `steam_links` | 509 | 15 | steam-core::task::handlers::friends, steam-flows::betainvite::friendship_poll, steam-flows::friend_sync, steam-flows::leave_cleanup, steam-flows::link, steam-flows::link_commands, steam-flows::rank, steam-persistence, steam-persistence::betainvite, steam-persistence::friends, steam-persistence::links, steam-persistence::miss_tracker, steam-persistence::rank, steam-web::routes::events, steam-web::routes::link | Cross-SP #387: zentralisieren in core.steam_links (auch Steam-Bot SP3); Issue #387: Steam-Link-Aequivalent vorhanden. SQLite exakt: PK(user_id, steam_id); user_id INTEGER NOT NULL, steam_id TEXT NOT NULL, verified INTEGER DEFAULT 0; Unique Index uq_steam_links_steam_owner(steam_id) WHERE user_id != 0; Owner-Guard-Trigger fuer Insert/Update. Abgleich SP0 core.steam_links: discord_id BIGINT entspricht user_id; steam_id64 BIGINT weicht ab (hier steam_id TEXT); verified BOOL bool-kompatibel via INTEGER; linked_at TIMESTAMPTZ entspricht am ehesten created_at DATETIME. Ziel: core.steam_links als einzige Wahrheit fuer DL-Bot SP1 und Steam-Bot SP3. |
| `user_data` | 0 | 5 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `user_mod_tags` | 0 | 6 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `user_tags` | 190 | 4 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |

## proposed_schema: steam

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `beta_invite_audit` | 129 | 7 | steam-flows::betainvite::dispatch, steam-persistence, steam-persistence::betainvite |  |
| `beta_invite_auto_failure_alerts` | 3 | 3 | steam-persistence::betainvite |  |
| `beta_invite_friendship_auto_poll` | 10 | 13 | steam-flows::betainvite::friendship_poll, steam-persistence::betainvite |  |
| `beta_invite_intent` | 200 | 4 | steam-persistence::betainvite |  |
| `beta_invite_panel_clicks` | 181 | 4 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `beta_invite_pending_payments` | 0 | 6 | steam-flows::supporter, steam-persistence::betainvite, steam-persistence::supporter |  |
| `beta_invite_supporter_role_grants` | 0 | 9 | steam-flows::supporter, steam-persistence::betainvite, steam-persistence::supporter |  |
| `beta_invite_tickets` | 101 | 7 | steam-persistence::betainvite |  |
| `deadlock_hero_builds` | 71 | 18 | steam-core::task::handlers::builds, steam-core::task::handlers::builds::catalog, steam-persistence::builds | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| `deadlock_party_members` | 0 | 4 | steam-core, steam-core::steam::presence, steam-persistence, steam-persistence::party_members, steam-persistence::presence | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| `deadlock_subrank_roles` | 66 | 6 | steam-flows::rank, steam-persistence, steam-persistence::rank | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| `live_player_state` | 526 | 12 | steam-core, steam-core::steam::presence, steam-persistence, steam-persistence::presence | Hypertable-Kandidat (Zeit-Spalte: last_seen_ts) |
| `steam_beta_invites` | 157 | 14 | steam-flows::betainvite::interactions, steam-persistence::betainvite |  |
| `steam_cleanup_poll_state` | 616 | 6 | steam-persistence::miss_tracker |  |
| `steam_flow_throttle` | 93 | 2 | steam-flows::leave_cleanup, steam-persistence::throttle |  |
| `steam_friend_check_cache` | 614 | 3 | steam-persistence, steam-persistence::friend_cache |  |
| `steam_friend_requests` | 389 | 6 | steam-core::task::handlers::friends, steam-flows::friend_sync, steam-flows::link, steam-persistence, steam-persistence::friends |  |
| `steam_friendship_miss_tracker` | 421 | 8 | steam-flows::friend_sync, steam-persistence::links, steam-persistence::miss_tracker |  |
| `steam_launch_tokens` | 13541 | 5 | steam-flows::link, steam-persistence::oauth_state | Hypertable-Kandidat (Zeit-Spalte: created_at) |
| `steam_links_archive` | 299 | 16 | steam-persistence::links, steam-persistence::role_cleanup |  |
| `steam_links_leave_archive` | 99 | 16 | steam-flows::leave_cleanup, steam-persistence, steam-persistence::role_cleanup |  |
| `steam_nudge_state` | 384 | 7 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `steam_presence_watchlist` | 0 | 3 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `steam_quick_invites` | 0 | 10 | steam-persistence::betainvite |  |
| `steam_rank_assignments` | 182 | 3 | steam-persistence, steam-persistence::rank |  |
| `steam_rank_history` | 3959 | 4 | steam-persistence::rank, steam-web::routes::rank | Hypertable-Kandidat (Zeit-Spalte: captured_at) |
| `steam_rich_presence` | 0 | 14 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `steam_role_cleanup_pending` | 2 | 6 | steam-flows::friend_sync, steam-persistence, steam-persistence::role_cleanup |  |
| `steam_tasks` | 1000 | 11 | steam-core, steam-core::api, steam-core::command_loop, steam-core::task, steam-core::task::runner, steam-flows::friend_sync, steam-persistence, steam-persistence::builds, steam-persistence::tasks | Hypertable-Kandidat (Zeit-Spalte: created_at) |

## proposed_schema: OFFEN

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `ai_moderation_cases` | 36 | 20 | [] | Vorschlag neues Namespace 'moderation'; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `ai_moderation_ragebait_hits` | 3 | 7 | [] | Vorschlag neues Namespace 'moderation'; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `changelog_entries` | 6290 | 6 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `changelog_posts` | 117 | 6 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `claimed_threads` | 0 | 4 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_contest_submissions` | 1 | 3 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_contests` | 1 | 10 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_fetch_history` | 2429 | 7 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_last_hashtags` | 0 | 3 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_submissions` | 25 | 8 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_templates_global` | 5 | 8 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_templates_streamer` | 0 | 8 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_window_submissions` | 21 | 3 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `clip_windows` | 39 | 6 | [] | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `customgames_tournament_signups` | 0 | 11 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `customgames_tournament_teams` | 0 | 6 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `deadlock_changelogs` | 117 | 5 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `deadlock_heroes` | 38 | 8 | steam-persistence::builds | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| `deadlock_voice_watch` | 0 | 4 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `discord_invite_codes` | 89 | 4 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `dm_response_tracking` | 0 | 5 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `faq_chat_messages` | 44 | 5 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `faq_chat_sessions` | 13 | 9 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `hero_build_clones` | 68 | 19 | steam-core::task::handlers::builds, steam-core::task::handlers::builds::catalog, steam-core::task::handlers::builds::delete, steam-persistence, steam-persistence::builds | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| `hero_build_sources` | 107 | 14 | steam-core::task::handlers::builds::catalog, steam-core::task::handlers::builds::convert, steam-core::task::handlers::builds::publish, steam-persistence, steam-persistence::builds | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| `invite_snapshot_cache` | 1 | 3 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `issue_reports` | 17 | 15 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `kv_store` | 974 | 3 | steam-persistence::builds | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden |
| `member_events` | 2946 | 9 | steam-flows::leave_cleanup, steam-persistence, steam-persistence::miss_tracker | Hypertable-Kandidat (Zeit-Spalte: timestamp); OFFEN: Schema-Zuordnung in SP1 klaeren |
| `notification_log` | 0 | 5 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `notification_queue` | 0 | 7 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `onboarding_pending_verify` | 289 | 3 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `persistent_views` | 5 | 6 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `reaction_role_dm_log` | 42 | 3 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `reaction_role_mappings` | 1 | 13 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `rename_global_state` | 1 | 3 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `rename_requests` | 17447 | 10 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386; Hypertable-Kandidat (Zeit-Spalte: created_at) |
| `router_user_prefs` | 4 | 4 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `schema_version` | 1 | 1 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `security_guard_incidents` | 11 | 12 | [] | Vorschlag neues Namespace 'moderation'; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `server_faq_logs` | 35 | 9 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `standalone_bot_state` | 2 | 4 | steam-core, steam-core::steam::heartbeat, steam-persistence, steam-persistence::bot_state | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden |
| `standalone_commands` | 25 | 10 | steam-core::command_loop, steam-persistence, steam-persistence::commands | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden |
| `streamer_link_intents` | 0 | 6 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_bans` | 27 | 3 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_interface` | 3 | 7 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_lane_tag_filter` | 1 | 5 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_lanes` | 0 | 8 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_lurkers` | 0 | 5 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_owner_prefs` | 46 | 3 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_presets` | 13 | 9 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_rank_pref` | 7 | 4 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tempvoice_staging_channels` | 6 | 3 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tierlist_build_votes` | 0 | 4 | [] | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tierlist_hero_meta` | 0 | 3 | [] | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tierlist_settings` | 5 | 3 | [] | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tierlist_snapshot_heroes` | 3420 | 6 | [] | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tierlist_snapshots` | 90 | 5 | [] | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tierlist_streamers` | 0 | 7 | [] | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `tournament_periods` | 3 | 9 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `turnier_auth_tokens` | 0 | 4 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `twitch_streamer_invites` | 84 | 6 | [] | Vorschlag Catch-all-Namespace 'bot' (State/Infra) - in Phase 1 entscheiden; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `user_privacy` | 0 | 5 | steam-persistence | OFFEN: Schema-Zuordnung in SP1 klaeren |
| `voice_channel_anchors` | 34 | 12 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `voice_channel_settings` | 385 | 5 | [] | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `voice_stats` | 864 | 4 | steam-flows::purge, steam-persistence, steam-persistence::friends | Hypertable-Kandidat (Zeit-Spalte: last_update); OFFEN: Schema-Zuordnung in SP1 klaeren |
| `watched_build_authors` | 10 | 8 | steam-persistence::builds | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |

## proposed_schema: activity

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `member_leave_surveys` | 95 | 17 | [] | Hypertable-Kandidat (Zeit-Spalte: left_at); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `message_activity` | 1705 | 6 | [] | Hypertable-Kandidat (Zeit-Spalte: last_message_at); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `text_conversation_log` | 4078 | 10 | [] | Hypertable-Kandidat (Zeit-Spalte: started_at); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `text_stats` | 578 | 4 | [] | Hypertable-Kandidat (Zeit-Spalte: last_update); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `user_activity_patterns` | 787 | 10 | [] | Hypertable-Kandidat (Zeit-Spalte: last_active_at); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `user_co_players` | 25178 | 7 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `user_retention_messages` | 1664 | 7 | [] | Hypertable-Kandidat (Zeit-Spalte: sent_at); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `user_retention_tracking` | 890 | 10 | [] | Hypertable-Kandidat (Zeit-Spalte: first_seen_at); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `voice_feedback_requests` | 457 | 12 | [] | Hypertable-Kandidat (Zeit-Spalte: sent_at_ts); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `voice_feedback_responses` | 31 | 6 | [] | Hypertable-Kandidat (Zeit-Spalte: received_at_ts); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `voice_session_log` | 63844 | 13 | [] | Hypertable-Kandidat (Zeit-Spalte: started_at); 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |

## proposed_schema: coaching

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `coach_applications` | 0 | 14 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `coaches` | 0 | 15 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `coaching_bans` | 0 | 4 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `coaching_coach_rotation` | 0 | 2 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `coaching_requests` | 56 | 25 | [] | Cross-SP #387: kanonische Form mit Website (id TEXT) vereinen; 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `coaching_sessions` | 43 | 21 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `coaching_sessions_legacy` | 2 | 15 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `coaching_surveys` | 4 | 8 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |

## proposed_schema: scrim

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `scrim_match` | 2 | 7 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `scrim_participant` | 29 | 12 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `scrim_team` | 4 | 6 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |
| `scrim_team_member` | 24 | 5 | [] | 0 Steam-Bot-Rust-Konsumenten (aus Steam-Bot-Sicht tot/Legacy) - migrieren, Drop spaeter Issue #386 |

