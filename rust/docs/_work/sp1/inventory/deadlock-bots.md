# Inventar — Deadlock-Bots (`data/deadlock.sqlite3`)

DB: `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` — 124 Tabellen. Quelle: lebende DB + Rust-Consumer-Grep (crates+bin).
Gate: `rust/scripts/sp1_inventory_gate.py`. Schema-Vorschlaege nach Eltern-Spec §5.2 (OFFEN = in Phase 1 entscheiden).

## proposed_schema: core  (5 Tabellen)

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---|---|---|---|
| steam_links | 509 | 15 | dl-activity, dl-bot, dl-bridges, dl-central-db, dl-community, dl-db, dl-stats, dl-voice | Cross-SP #387: zentralisieren in core.steam_links (auch Steam-Bot SP3) |
| user_tags | 190 | 4 | dl-community, dl-voice |  |
| user_data | 0 | 5 | dl-community |  |
| user_mod_tags | 0 | 6 | dl-community |  |
| user_privacy | 0 | 5 | dl-activity, dl-community, dl-db, dl-voice |  |

## proposed_schema: coaching  (8 Tabellen)

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---|---|---|---|
| coaching_requests | 56 | 25 | dl-bot, dl-community, dl-db | Cross-SP #387: kanonische Form mit Website (id TEXT) vereinen |
| coaching_sessions | 43 | 21 | dl-community |  |
| coaching_surveys | 4 | 8 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| coaching_sessions_legacy | 2 | 15 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| coach_applications | 0 | 14 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| coaches | 0 | 15 | dl-bot, dl-community |  |
| coaching_bans | 0 | 4 | dl-community |  |
| coaching_coach_rotation | 0 | 2 | dl-community |  |

## proposed_schema: scrim  (4 Tabellen)

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---|---|---|---|
| scrim_participant | 29 | 12 | dl-community, dl-db, dl-squads |  |
| scrim_team_member | 24 | 5 | dl-db, dl-squads |  |
| scrim_team | 4 | 6 | dl-db, dl-squads |  |
| scrim_match | 2 | 7 | dl-db, dl-squads |  |

## proposed_schema: activity  (14 Tabellen)

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---|---|---|---|
| voice_session_log | 63844 | 13 | dl-activity, dl-community, dl-dashboard, dl-db, dl-stats, dl-voice | Hypertable-Kandidat (Zeit-Spalte: started_at) |
| user_co_players | 25178 | 7 | dl-activity, dl-community, dl-dashboard, dl-db, dl-stats |  |
| text_conversation_log | 4078 | 10 | dl-activity, dl-db, dl-stats | Hypertable-Kandidat (Zeit-Spalte: started_at) |
| member_events | 2946 | 9 | dl-activity, dl-community, dl-dashboard, dl-db, dl-stats, dl-twitch-invite-sync | Hypertable-Kandidat (Zeit-Spalte: timestamp) |
| message_activity | 1705 | 6 | dl-activity, dl-community, dl-dashboard, dl-db | Hypertable-Kandidat (Zeit-Spalte: last_message_at) |
| user_retention_messages | 1664 | 7 | dl-community, dl-dashboard | Hypertable-Kandidat (Zeit-Spalte: sent_at) |
| user_retention_tracking | 890 | 10 | dl-bot, dl-community, dl-dashboard | Hypertable-Kandidat (Zeit-Spalte: first_seen_at) |
| voice_stats | 864 | 4 | dl-activity, dl-bot, dl-community, dl-dashboard, dl-db, dl-stats, dl-voice | Hypertable-Kandidat (Zeit-Spalte: last_update) |
| user_activity_patterns | 787 | 10 | dl-activity, dl-community, dl-db | Hypertable-Kandidat (Zeit-Spalte: last_active_at) |
| text_stats | 578 | 4 | dl-activity, dl-bot, dl-db, dl-stats | Hypertable-Kandidat (Zeit-Spalte: last_update) |
| live_player_state | 526 | 12 | dl-activity, dl-community, dl-db, dl-voice | Hypertable-Kandidat (Zeit-Spalte: last_seen_ts) |
| voice_feedback_requests | 457 | 12 | dl-community, dl-voice | Hypertable-Kandidat (Zeit-Spalte: sent_at_ts) |
| member_leave_surveys | 95 | 17 | dl-community, dl-dashboard | Hypertable-Kandidat (Zeit-Spalte: left_at) |
| voice_feedback_responses | 31 | 6 | dl-community, dl-voice | Hypertable-Kandidat (Zeit-Spalte: received_at_ts) |

## proposed_schema: steam  (25 Tabellen)

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---|---|---|---|
| steam_launch_tokens | 13541 | 5 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| steam_rank_history | 3959 | 4 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386; BEHALTEN dauerhaft #385 (0 Konsumenten, Feature offen) |
| steam_tasks | 1000 | 11 | dl-bot |  |
| steam_cleanup_poll_state | 616 | 6 | dl-community |  |
| steam_friend_check_cache | 614 | 3 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| steam_friendship_miss_tracker | 421 | 8 | dl-community |  |
| steam_friend_requests | 389 | 6 | dl-community |  |
| steam_nudge_state | 384 | 7 | dl-community, dl-voice |  |
| steam_links_archive | 299 | 16 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| beta_invite_intent | 200 | 4 | dl-community |  |
| steam_rank_assignments | 182 | 3 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| beta_invite_panel_clicks | 181 | 4 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| steam_beta_invites | 157 | 14 | dl-community |  |
| beta_invite_audit | 129 | 7 | dl-community |  |
| beta_invite_tickets | 101 | 7 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| steam_links_leave_archive | 99 | 16 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| steam_flow_throttle | 93 | 2 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| beta_invite_friendship_auto_poll | 10 | 13 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| beta_invite_auto_failure_alerts | 3 | 3 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| steam_role_cleanup_pending | 2 | 6 | dl-community |  |
| beta_invite_pending_payments | 0 | 6 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| beta_invite_supporter_role_grants | 0 | 9 | — | 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| steam_presence_watchlist | 0 | 3 | dl-community |  |
| steam_quick_invites | 0 | 10 | dl-community |  |
| steam_rich_presence | 0 | 14 | dl-community |  |

## proposed_schema: OFFEN  (68 Tabellen)

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---|---|---|---|
| rename_requests | 17447 | 10 | dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| changelog_entries | 6290 | 6 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386; BEHALTEN dauerhaft #385 (0 Konsumenten, Feature offen) |
| tierlist_snapshot_heroes | 3420 | 6 | dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| clip_fetch_history | 2429 | 7 | — | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| kv_store | 974 | 3 | dl-activity, dl-bot, dl-bridges, dl-community, dl-dashboard, dl-db, dl-twitch-invite-sync, dl-voice | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| voice_channel_settings | 385 | 5 | dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| onboarding_pending_verify | 289 | 3 | dl-bot, dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| changelog_posts | 117 | 6 | dl-dashboard | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| deadlock_changelogs | 117 | 5 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| hero_build_sources | 107 | 14 | dl-bot | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| tierlist_snapshots | 90 | 5 | dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| discord_invite_codes | 89 | 4 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| twitch_streamer_invites | 84 | 6 | dl-activity, dl-dashboard, dl-twitch-invite-sync | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| oauth_states | 83 | 9 | dl-dashboard | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| deadlock_hero_builds | 71 | 18 | dl-dashboard, dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| hero_build_clones | 68 | 19 | dl-bot | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| deadlock_subrank_roles | 66 | 6 | — | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| tempvoice_owner_prefs | 46 | 3 | dl-community, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| faq_chat_messages | 44 | 5 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| reaction_role_dm_log | 42 | 3 | dl-community, dl-dashboard, dl-db | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| clip_windows | 39 | 6 | dl-community | Vorschlag Namespace 'clips' (groesstenteils tot) |
| deadlock_heroes | 38 | 8 | dl-dashboard, dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| ai_moderation_cases | 36 | 20 | dl-moderation | Vorschlag neues Namespace 'moderation' |
| server_faq_logs | 35 | 9 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| voice_channel_anchors | 34 | 12 | dl-community, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| tempvoice_bans | 27 | 3 | dl-community, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| clip_submissions | 25 | 8 | dl-community | Vorschlag Namespace 'clips' (groesstenteils tot) |
| standalone_commands | 25 | 10 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| clip_window_submissions | 21 | 3 | dl-community | Vorschlag Namespace 'clips' (groesstenteils tot) |
| issue_reports | 17 | 15 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| faq_chat_sessions | 13 | 9 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| tempvoice_presets | 13 | 9 | dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| security_guard_incidents | 11 | 12 | dl-moderation | Vorschlag neues Namespace 'moderation' |
| watched_build_authors | 10 | 8 | dl-bot | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| tempvoice_rank_pref | 7 | 4 | dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| tempvoice_staging_channels | 6 | 3 | — | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| clip_templates_global | 5 | 8 | — | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| persistent_views | 5 | 6 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| tierlist_settings | 5 | 3 | dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| router_user_prefs | 4 | 4 | dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| ai_moderation_ragebait_hits | 3 | 7 | dl-moderation | Vorschlag neues Namespace 'moderation' |
| tempvoice_interface | 3 | 7 | dl-bot, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| tournament_periods | 3 | 9 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| standalone_bot_state | 2 | 4 | dl-bot | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| clip_contest_submissions | 1 | 3 | — | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| clip_contests | 1 | 10 | — | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| invite_snapshot_cache | 1 | 3 | dl-discord | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| reaction_role_mappings | 1 | 13 | dl-community, dl-dashboard, dl-db | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| rename_global_state | 1 | 3 | — | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| schema_version | 1 | 1 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| tempvoice_lane_tag_filter | 1 | 5 | dl-moderation, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| claimed_threads | 0 | 4 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| clip_last_hashtags | 0 | 3 | — | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| clip_templates_streamer | 0 | 8 | — | Vorschlag Namespace 'clips' (groesstenteils tot); 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| customgames_tournament_signups | 0 | 11 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| customgames_tournament_teams | 0 | 6 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
| deadlock_party_members | 0 | 4 | dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| deadlock_voice_watch | 0 | 4 | dl-community, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| dm_response_tracking | 0 | 5 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| notification_log | 0 | 5 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| notification_queue | 0 | 7 | dl-community | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| streamer_link_intents | 0 | 6 | dl-bot, dl-bridges | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden |
| tempvoice_lanes | 0 | 8 | dl-community, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| tempvoice_lurkers | 0 | 5 | dl-community, dl-voice | Vorschlag neues Namespace 'voice' (TempVoice/Voice-Config) |
| tierlist_build_votes | 0 | 4 | dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| tierlist_hero_meta | 0 | 3 | dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| tierlist_streamers | 0 | 7 | dl-tierlist | Vorschlag neues Namespace 'tierlist' (Tierlist/Builds) |
| turnier_auth_tokens | 0 | 4 | — | Vorschlag Catch-all-Namespace 'bot' (State/Infra) — in Phase 1 entscheiden; 0 Rust-Konsumenten (tot/Legacy) — migrieren, Drop spaeter #386 |
