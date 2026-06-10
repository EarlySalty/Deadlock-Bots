-- GENERIERT von rust/scripts/dump_schema.py — nicht von Hand editieren.
-- Quelle: data/deadlock.sqlite3 · Stand: 2026-06-10

-- table: ai_moderation_cases
CREATE TABLE ai_moderation_cases (
    case_id TEXT PRIMARY KEY,
    guild_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    user_tag TEXT,
    original_content TEXT,
    attachments_json TEXT,
    ai_category TEXT,
    ai_confidence REAL,
    ai_reason TEXT,
    ai_raw_json TEXT,
    escalated_with_context INTEGER DEFAULT 0,
    action TEXT,
    mod_id INTEGER,
    mod_action_at TIMESTAMP,
    mod_deny_reason TEXT,
    mod_review_message_id INTEGER,
    log_message_id INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- table: ai_moderation_ragebait_hits
CREATE TABLE ai_moderation_ragebait_hits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    content_preview TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- table: beta_invite_audit
CREATE TABLE beta_invite_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                guild_id INTEGER,
                discord_id INTEGER NOT NULL,
                discord_name TEXT,
                steam_id64 TEXT NOT NULL,
                steam_profile TEXT NOT NULL,
                invited_at INTEGER NOT NULL
            );

-- table: beta_invite_auto_failure_alerts
CREATE TABLE beta_invite_auto_failure_alerts (
                discord_id INTEGER PRIMARY KEY,
                last_alert_at INTEGER NOT NULL,
                last_error TEXT
            );

-- table: beta_invite_friendship_auto_poll
CREATE TABLE beta_invite_friendship_auto_poll (
                discord_id INTEGER PRIMARY KEY,
                invite_record_id INTEGER NOT NULL,
                guild_id INTEGER NOT NULL,
                channel_id INTEGER NOT NULL,
                steam_id64 TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                started_at INTEGER NOT NULL,
                next_check_at INTEGER NOT NULL,
                attempts_completed INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 5,
                last_checked_at INTEGER,
                finished_at INTEGER,
                last_error TEXT
            );

-- table: beta_invite_intent
CREATE TABLE beta_invite_intent(
              discord_id INTEGER PRIMARY KEY,
              intent TEXT NOT NULL,
              decided_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              locked INTEGER NOT NULL DEFAULT 1
            );

-- table: beta_invite_panel_clicks
CREATE TABLE beta_invite_panel_clicks(
              discord_id INTEGER PRIMARY KEY,
              click_count INTEGER NOT NULL DEFAULT 1,
              first_clicked_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              last_clicked_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: beta_invite_pending_payments
CREATE TABLE beta_invite_pending_payments(
              discord_id INTEGER PRIMARY KEY,
              discord_name TEXT NOT NULL,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            , token TEXT, paid_at INTEGER, consumed_at INTEGER);

-- table: beta_invite_supporter_role_grants
CREATE TABLE beta_invite_supporter_role_grants(
              discord_id INTEGER PRIMARY KEY,
              role_id INTEGER NOT NULL,
              granted_at INTEGER NOT NULL,
              expires_at INTEGER NOT NULL,
              last_payment_at INTEGER NOT NULL,
              last_applied_at INTEGER,
              revoked_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: beta_invite_tickets
CREATE TABLE beta_invite_tickets(
              discord_id INTEGER PRIMARY KEY,
              guild_id INTEGER NOT NULL,
              channel_id INTEGER NOT NULL,
              status TEXT NOT NULL,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              closed_at INTEGER
            );

-- table: changelog_entries
CREATE TABLE changelog_entries(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              post_id INTEGER NOT NULL,
              section TEXT,
              subject TEXT,
              subject_type TEXT,
              change_text TEXT NOT NULL,
              FOREIGN KEY(post_id) REFERENCES changelog_posts(id) ON DELETE CASCADE
            );

-- table: changelog_posts
CREATE TABLE changelog_posts(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              title TEXT NOT NULL,
              url TEXT NOT NULL UNIQUE,
              posted_at TEXT,
              raw_content TEXT
            , translated_content TEXT);

-- table: claimed_threads
CREATE TABLE claimed_threads (
                    thread_id         BIGINT PRIMARY KEY,
                    assigned_user_id  BIGINT,
                    claimed_by_id     BIGINT,
                    created_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );

-- table: clip_contest_submissions
CREATE TABLE clip_contest_submissions(
                contest_id INTEGER NOT NULL,
                submission_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                PRIMARY KEY(contest_id, submission_id),
                FOREIGN KEY(contest_id) REFERENCES clip_contests(id) ON DELETE CASCADE,
                FOREIGN KEY(submission_id) REFERENCES clip_submissions(id) ON DELETE CASCADE
            );

-- table: clip_contests
CREATE TABLE clip_contests(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                guild_id INTEGER NOT NULL,
                name TEXT,
                start_ts INTEGER NOT NULL,
                end_ts   INTEGER NOT NULL,
                announce_channel_id INTEGER,
                status TEXT NOT NULL DEFAULT 'scheduled', -- scheduled|running|ended|published
                video_url TEXT,
                winner_user_id INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

-- table: clip_fetch_history
CREATE TABLE clip_fetch_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            streamer_login TEXT NOT NULL,
            fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            clips_found INTEGER DEFAULT 0,
            clips_new INTEGER DEFAULT 0,
            fetch_duration_ms INTEGER,
            error TEXT,
            FOREIGN KEY(streamer_login) REFERENCES twitch_streamers(twitch_login)
        );

-- table: clip_last_hashtags
CREATE TABLE clip_last_hashtags (
            streamer_login TEXT PRIMARY KEY,
            hashtags TEXT NOT NULL,
            last_used_at TEXT NOT NULL,
            FOREIGN KEY(streamer_login) REFERENCES twitch_streamers(twitch_login)
        );

-- table: clip_submissions
CREATE TABLE clip_submissions(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                guild_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                link TEXT NOT NULL,
                credit TEXT NOT NULL,
                permission TEXT NOT NULL,
                info TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

-- table: clip_templates_global
CREATE TABLE clip_templates_global (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            template_name TEXT NOT NULL UNIQUE,
            description_template TEXT NOT NULL,
            hashtags TEXT NOT NULL,
            category TEXT,
            usage_count INTEGER DEFAULT 0,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP,
            created_by TEXT
        );

-- table: clip_templates_streamer
CREATE TABLE clip_templates_streamer (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            streamer_login TEXT NOT NULL,
            template_name TEXT NOT NULL,
            description_template TEXT NOT NULL,
            hashtags TEXT NOT NULL,
            is_default INTEGER DEFAULT 0,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(streamer_login, template_name),
            FOREIGN KEY(streamer_login) REFERENCES twitch_streamers(twitch_login)
        );

-- table: clip_window_submissions
CREATE TABLE clip_window_submissions(
                window_id INTEGER NOT NULL,
                submission_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                PRIMARY KEY(window_id, submission_id),
                FOREIGN KEY(window_id) REFERENCES clip_windows(id) ON DELETE CASCADE,
                FOREIGN KEY(submission_id) REFERENCES clip_submissions(id) ON DELETE CASCADE
            );

-- table: clip_windows
CREATE TABLE clip_windows(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                guild_id INTEGER NOT NULL,
                start_ts INTEGER NOT NULL,
                end_ts   INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'running', -- running | dumped
                dump_sent_ts INTEGER,
                UNIQUE(guild_id, start_ts, end_ts)
            );

-- table: coach_applications
CREATE TABLE coach_applications (
              id TEXT PRIMARY KEY,
              discord_user_id INTEGER UNIQUE NOT NULL,
              discord_username TEXT,
              display_name TEXT,
              application_text TEXT,
              experience_text TEXT,
              rank TEXT,
              specialties_json TEXT DEFAULT '[]',
              availability_json TEXT DEFAULT '{}',
              status TEXT DEFAULT 'pending',
              reviewed_by INTEGER,
              reviewed_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: coaches
CREATE TABLE coaches (
              id TEXT PRIMARY KEY,
              discord_user_id INTEGER UNIQUE NOT NULL,
              discord_username TEXT,
              display_name TEXT,
              avatar_url TEXT,
              bio TEXT,
              specialties_json TEXT DEFAULT '[]',
              availability_json TEXT DEFAULT '{}',
              status TEXT DEFAULT 'pending',
              website_coach_id TEXT,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            , avg_rating REAL, total_reviews INTEGER DEFAULT 0, total_sessions INTEGER DEFAULT 0);

-- table: coaching_bans
CREATE TABLE coaching_bans (
              discord_user_id INTEGER PRIMARY KEY,
              banned_at INTEGER NOT NULL,
              expires_at INTEGER NOT NULL,
              reason TEXT
            );

-- table: coaching_requests
CREATE TABLE coaching_requests (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              discord_user_id INTEGER NOT NULL,
              discord_username TEXT,
              rank TEXT NOT NULL,
              subrank TEXT NOT NULL,
              hero TEXT,
              games_played TEXT,
              hours_played TEXT,
              availability TEXT,
              current_problems TEXT,
              ai_summary TEXT,
              ai_insights_json TEXT,
              status TEXT DEFAULT 'pending',
              message_id INTEGER,
              channel_id INTEGER,
              role_assigned_at INTEGER,
              role_expires_at INTEGER,
              role_removed_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            , scheduled_slot TEXT, assigned_coach_id TEXT, reserved_until INTEGER);

-- table: coaching_sessions
CREATE TABLE coaching_sessions (
          id TEXT PRIMARY KEY,
          request_id INTEGER,
          coach_id TEXT,
          discord_user_id INTEGER NOT NULL,
          discord_username TEXT,
          discord_channel_id INTEGER,
          discord_thread_id INTEGER,
          status TEXT DEFAULT 'active',
          role_assigned_at INTEGER,
          role_reminder_at INTEGER,
          role_expires_at INTEGER,
          voice_channel_id INTEGER,
          voice_started_at INTEGER,
          voice_last_seen_at INTEGER,
          survey_sent_at INTEGER,
          scheduled_at INTEGER,
          started_at INTEGER DEFAULT (strftime('%s','now')),
          completed_at INTEGER,
          created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        , reward_role_expires_at INTEGER, reward_role_removed_at INTEGER);

-- table: coaching_sessions_legacy
CREATE TABLE "coaching_sessions_legacy" (
                user_id     INTEGER PRIMARY KEY,
                thread_id   INTEGER,
                match_id    TEXT,
                rank        TEXT,
                subrank     TEXT,
                hero        TEXT,
                comment     TEXT,
                step        TEXT,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                is_active   INTEGER DEFAULT 1
            , voice_channel_id INTEGER, voice_started_at INTEGER, voice_last_seen_at INTEGER, survey_sent_at INTEGER);

-- table: coaching_surveys
CREATE TABLE coaching_surveys (
              id TEXT PRIMARY KEY,
              session_id TEXT UNIQUE,
              rating INTEGER CHECK(rating >= 0 AND rating <= 10),
              feedback_text TEXT,
              improved_areas TEXT,
              unresolved_items TEXT,
              would_recommend INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: customgames_tournament_signups
CREATE TABLE customgames_tournament_signups(
          guild_id INTEGER NOT NULL,
          user_id INTEGER NOT NULL,
          registration_mode TEXT NOT NULL CHECK (registration_mode IN ('solo', 'team')),
          rank TEXT NOT NULL,
          rank_value INTEGER NOT NULL,
          team_id INTEGER,
          assigned_by_admin INTEGER NOT NULL DEFAULT 0,
          created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
          updated_at DATETIME DEFAULT CURRENT_TIMESTAMP, rank_subvalue INTEGER NOT NULL DEFAULT 0, display_name TEXT,
          PRIMARY KEY(guild_id, user_id),
          FOREIGN KEY(team_id) REFERENCES customgames_tournament_teams(id) ON DELETE SET NULL
        );

-- table: customgames_tournament_teams
CREATE TABLE customgames_tournament_teams(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          guild_id INTEGER NOT NULL,
          name TEXT NOT NULL,
          name_key TEXT NOT NULL,
          created_by INTEGER,
          created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
          UNIQUE(guild_id, name_key)
        );

-- table: deadlock_changelogs
CREATE TABLE deadlock_changelogs (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT, url TEXT, posted_at TEXT, content TEXT);

-- table: deadlock_hero_builds
CREATE TABLE deadlock_hero_builds(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              hero_id INTEGER NOT NULL,
              build_id INTEGER NOT NULL,
              build_name TEXT NOT NULL,
              author_name TEXT NOT NULL,
              is_active INTEGER NOT NULL DEFAULT 1,
              sort_order INTEGER NOT NULL DEFAULT 100,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')), sync_status TEXT, sync_message TEXT, last_checked_at INTEGER, last_synced_at INTEGER, last_alerted_at INTEGER, source_version INTEGER, source_last_updated_ts INTEGER, clone_build_id INTEGER, clone_version INTEGER,
              UNIQUE(hero_id, build_id)
            );

-- table: deadlock_heroes
CREATE TABLE deadlock_heroes(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              hero_id INTEGER NOT NULL,
              name TEXT NOT NULL,
              origin_build_id INTEGER,
              is_active INTEGER NOT NULL DEFAULT 1,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')), target_build_name_override TEXT,
              UNIQUE(hero_id),
              UNIQUE(name)
            );

-- table: deadlock_party_members
CREATE TABLE deadlock_party_members(
              party_id TEXT NOT NULL,
              steam_id TEXT NOT NULL,
              party_size INTEGER,
              seen_at INTEGER NOT NULL,
              PRIMARY KEY (party_id, steam_id)
            );

-- table: deadlock_subrank_roles
CREATE TABLE deadlock_subrank_roles(
          guild_id INTEGER NOT NULL,
          rank_value INTEGER NOT NULL,
          subrank INTEGER NOT NULL,
          role_id INTEGER NOT NULL,
          role_name TEXT,
          updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
          PRIMARY KEY(guild_id, rank_value, subrank),
          UNIQUE(guild_id, role_id)
        );

-- table: deadlock_voice_watch
CREATE TABLE deadlock_voice_watch(
              steam_id TEXT PRIMARY KEY,
              guild_id INTEGER,
              channel_id INTEGER,
              updated_at INTEGER NOT NULL
            );

-- table: discord_invite_codes
CREATE TABLE discord_invite_codes (
            guild_id      INTEGER NOT NULL,
            invite_code   TEXT NOT NULL,
            created_at    TEXT DEFAULT CURRENT_TIMESTAMP,
            last_seen_at  TEXT DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (guild_id, invite_code)
        );

-- table: dm_response_tracking
CREATE TABLE dm_response_tracking (
                user_id TEXT PRIMARY KEY,
                last_dm_sent TIMESTAMP NOT NULL,
                response_count INTEGER DEFAULT 0,
                last_response TIMESTAMP,
                status TEXT DEFAULT 'pending'
            );

-- table: faq_chat_messages
CREATE TABLE faq_chat_messages(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              FOREIGN KEY(session_id) REFERENCES faq_chat_sessions(session_id)
            );

-- table: faq_chat_sessions
CREATE TABLE faq_chat_sessions(
              session_id TEXT PRIMARY KEY,
              user_id INTEGER NOT NULL,
              user_name TEXT,
              channel_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              expires_at DATETIME NOT NULL,
              status TEXT NOT NULL DEFAULT 'active',
              last_activity_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

-- table: hero_build_clones
CREATE TABLE hero_build_clones(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              origin_hero_build_id INTEGER NOT NULL,
              origin_build_id INTEGER,
              hero_id INTEGER NOT NULL,
              author_account_id INTEGER,
              source_language INTEGER,
              source_version INTEGER,
              source_last_updated_ts INTEGER,
              target_language INTEGER NOT NULL,
              target_name TEXT,
              target_description TEXT,
              status TEXT NOT NULL DEFAULT 'pending',
              status_info TEXT,
              uploaded_build_id INTEGER,
              uploaded_version INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              last_attempt_at INTEGER,
              attempts INTEGER NOT NULL DEFAULT 0,
              UNIQUE(origin_hero_build_id, target_language)
            );

-- table: hero_build_sources
CREATE TABLE hero_build_sources(
              hero_build_id INTEGER PRIMARY KEY,
              origin_build_id INTEGER,
              author_account_id INTEGER NOT NULL,
              hero_id INTEGER NOT NULL,
              language INTEGER NOT NULL,
              version INTEGER NOT NULL,
              name TEXT NOT NULL,
              description TEXT,
              tags_json TEXT,
              details_json TEXT,
              publish_ts INTEGER,
              last_updated_ts INTEGER,
              fetched_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              last_seen_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: invite_snapshot_cache
CREATE TABLE invite_snapshot_cache (
                    guild_id INTEGER NOT NULL PRIMARY KEY,
                    snapshot_json TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

-- table: issue_reports
CREATE TABLE issue_reports(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER,
              guild_id INTEGER,
              channel_id INTEGER,
              message_id INTEGER,
              category TEXT,
              title TEXT,
              description TEXT NOT NULL,
              status TEXT NOT NULL DEFAULT 'pending',
              ai_response TEXT,
              ai_model TEXT,
              ai_error TEXT,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              answered_at INTEGER
            );

-- table: kv_store
CREATE TABLE kv_store(
              ns TEXT NOT NULL,
              k  TEXT NOT NULL,
              v  TEXT NOT NULL,
              PRIMARY KEY(ns, k)
            );

-- table: live_player_state
CREATE TABLE live_player_state(
              steam_id TEXT PRIMARY KEY,
              last_gameid TEXT,
              last_server_id TEXT,
              last_seen_ts INTEGER,
              in_deadlock_now INTEGER DEFAULT 0,
              in_match_now_strict INTEGER DEFAULT 0
            , deadlock_stage TEXT, deadlock_minutes INTEGER, deadlock_localized TEXT, deadlock_hero TEXT, deadlock_party_hint TEXT, deadlock_updated_at INTEGER);

-- table: member_events
CREATE TABLE member_events(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              event_type TEXT NOT NULL,
              timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
              display_name TEXT,
              account_created_at DATETIME,
              join_position INTEGER,
              metadata TEXT
            );

-- table: member_leave_surveys
CREATE TABLE member_leave_surveys(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              left_at INTEGER NOT NULL,
              display_name TEXT,
              user_bucket TEXT NOT NULL,
              days_on_server INTEGER,
              survey_token TEXT UNIQUE NOT NULL,
              dm_status TEXT,
              reason_code TEXT,
              follow_up_question TEXT,
              follow_up_text TEXT,
              extra_text TEXT,
              responded_at INTEGER,
              web_submitted_at INTEGER,
              web_payload TEXT,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

-- table: message_activity
CREATE TABLE message_activity(
              user_id INTEGER NOT NULL,
              guild_id INTEGER NOT NULL,
              channel_id INTEGER,
              message_count INTEGER DEFAULT 1,
              last_message_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              first_message_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              PRIMARY KEY(user_id, guild_id)
            );

-- table: notification_log
CREATE TABLE notification_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT NOT NULL,
                rank TEXT NOT NULL,
                notification_time TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                count INTEGER DEFAULT 1
            );

-- table: notification_queue
CREATE TABLE notification_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT NOT NULL,
                guild_id TEXT NOT NULL,
                rank TEXT NOT NULL,
                queue_date TEXT NOT NULL,
                added_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                processed BOOLEAN DEFAULT FALSE
            );

-- table: oauth_states
CREATE TABLE oauth_states (
              state TEXT PRIMARY KEY,
              provider TEXT NOT NULL,
              flow_type TEXT NOT NULL,
              requesting_service TEXT,
              redirect_after TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              expires_at INTEGER NOT NULL,
              used INTEGER DEFAULT 0,
              metadata TEXT
            );

-- table: onboarding_pending_verify
CREATE TABLE onboarding_pending_verify (
                user_id INTEGER PRIMARY KEY,
                channel_id INTEGER NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

-- table: persistent_views
CREATE TABLE persistent_views (
                message_id TEXT PRIMARY KEY,
                channel_id TEXT NOT NULL,
                guild_id TEXT NOT NULL,
                view_type TEXT NOT NULL,
                user_id TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

-- table: rename_global_state
CREATE TABLE rename_global_state(
                id INTEGER PRIMARY KEY DEFAULT 1, -- Only one row expected
                last_rename_timestamp DATETIME DEFAULT (strftime('%Y-%m-%d %H:%M:%S', 'now', '-5 minutes')),
                next_worker_id INTEGER DEFAULT 1 -- 1=main bot, 2=worker bot
            );

-- table: rename_requests
CREATE TABLE rename_requests(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id INTEGER NOT NULL,
                new_name TEXT NOT NULL,
                reason TEXT,
                status TEXT NOT NULL DEFAULT 'PENDING', -- PENDING, PROCESSING, COMPLETED, FAILED
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                processed_at TIMESTAMP,
                retry_count INTEGER DEFAULT 0,
                last_error TEXT,
                assigned_worker_id INTEGER DEFAULT 0 -- 0=unassigned, 1=main bot, 2=worker bot
            );

-- table: router_user_prefs
CREATE TABLE router_user_prefs (
    user_id    INTEGER PRIMARY KEY,
    mode       TEXT NOT NULL CHECK(mode IN ('ranked', 'casual', 'street_brawl')),
    auto_join  INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- table: schema_version
CREATE TABLE schema_version(
              version INTEGER NOT NULL
            );

-- table: security_guard_incidents
CREATE TABLE security_guard_incidents (
                case_id      TEXT PRIMARY KEY,
                guild_id     INTEGER NOT NULL,
                user_id      INTEGER NOT NULL,
                user_tag     TEXT NOT NULL,
                action       TEXT NOT NULL,
                reason       TEXT NOT NULL,
                channel_count   INTEGER DEFAULT 0,
                message_count   INTEGER DEFAULT 0,
                attachment_count INTEGER DEFAULT 0,
                keyword_hit  INTEGER DEFAULT 0,
                messages_json TEXT,
                created_at   TEXT NOT NULL
            );

-- table: server_faq_logs
CREATE TABLE server_faq_logs(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              guild_id INTEGER,
              channel_id INTEGER,
              user_id INTEGER,
              question TEXT NOT NULL,
              answer TEXT,
              model TEXT,
              metadata TEXT
            );

-- table: standalone_bot_state
CREATE TABLE standalone_bot_state(
              bot TEXT PRIMARY KEY,
              heartbeat INTEGER NOT NULL,
              payload TEXT,
              updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

-- table: standalone_commands
CREATE TABLE standalone_commands(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              bot TEXT NOT NULL,
              command TEXT NOT NULL,
              payload TEXT,
              status TEXT NOT NULL DEFAULT 'pending',
              result TEXT,
              error TEXT,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              started_at DATETIME,
              finished_at DATETIME
            );

-- table: steam_beta_invites
CREATE TABLE steam_beta_invites(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              discord_id INTEGER NOT NULL,
              steam_id64 TEXT NOT NULL,
              account_id INTEGER,
              status TEXT NOT NULL,
              last_error TEXT,
              friend_requested_at INTEGER,
              friend_confirmed_at INTEGER,
              invite_sent_at INTEGER,
              last_notified_at INTEGER,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')), scheduled_invite_at INTEGER, dispatch_attempts INTEGER NOT NULL DEFAULT 0,
              UNIQUE(discord_id),
              UNIQUE(steam_id64)
            );

-- table: steam_cleanup_poll_state
CREATE TABLE steam_cleanup_poll_state(
              user_id INTEGER PRIMARY KEY,
              last_polled_at INTEGER NOT NULL,
              last_result TEXT NOT NULL,
              last_error TEXT,
              updated_at INTEGER NOT NULL
            , miss_count INTEGER NOT NULL DEFAULT 0);

-- table: steam_flow_throttle
CREATE TABLE steam_flow_throttle (
           key      TEXT    PRIMARY KEY,
           last_run INTEGER NOT NULL
         );

-- table: steam_friend_check_cache
CREATE TABLE steam_friend_check_cache(
      steam_id TEXT PRIMARY KEY,
      friend INTEGER NOT NULL,
      checked_at INTEGER NOT NULL
    );

-- table: steam_friend_requests
CREATE TABLE steam_friend_requests(
              steam_id TEXT PRIMARY KEY,
              status TEXT DEFAULT 'pending',
              requested_at INTEGER DEFAULT (strftime('%s','now')),
              last_attempt INTEGER,
              attempts INTEGER DEFAULT 0,
              error TEXT
            );

-- table: steam_friendship_miss_tracker
CREATE TABLE steam_friendship_miss_tracker(
              steam_id TEXT PRIMARY KEY,
              user_id INTEGER NOT NULL,
              miss_count INTEGER NOT NULL DEFAULT 0,
              last_polled_at INTEGER NOT NULL,
              last_seen_friend_at INTEGER,
              last_miss_at INTEGER,
              last_action_at INTEGER,
              updated_at INTEGER NOT NULL
            );

-- table: steam_launch_tokens
CREATE TABLE steam_launch_tokens (
           token       TEXT    PRIMARY KEY,
           user_id     INTEGER NOT NULL,
           created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now')),
           expires_at  INTEGER NOT NULL,
           consumed_at INTEGER
         );

-- table: steam_links
CREATE TABLE steam_links(
            user_id    INTEGER NOT NULL,
            steam_id   TEXT    NOT NULL,
            name       TEXT,
            verified   INTEGER DEFAULT 0,
            primary_account INTEGER DEFAULT 0,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP, legacy_ref TEXT, migrated_at INTEGER, deadlock_rank INTEGER, deadlock_rank_name TEXT, deadlock_subrank INTEGER, deadlock_badge_level INTEGER, deadlock_rank_updated_at INTEGER, is_steam_friend INTEGER DEFAULT 0,
            PRIMARY KEY(user_id, steam_id)
            );

-- table: steam_links_archive
CREATE TABLE steam_links_archive(
              user_id    INTEGER NOT NULL,
              steam_id   TEXT    NOT NULL,
              name       TEXT,
              verified   INTEGER DEFAULT 0,
              primary_account INTEGER DEFAULT 0,
              deadlock_rank INTEGER,
              deadlock_rank_name TEXT,
              deadlock_subrank INTEGER,
              deadlock_badge_level INTEGER,
              deadlock_rank_updated_at INTEGER,
              created_at DATETIME,
              updated_at DATETIME,
              left_at    INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              guild_id   INTEGER,
              leave_reason TEXT,
              display_name TEXT,
              PRIMARY KEY(user_id, steam_id)
            );

-- table: steam_links_leave_archive
CREATE TABLE steam_links_leave_archive(
                  user_id INTEGER NOT NULL,
                  steam_id TEXT NOT NULL,
                  name TEXT,
                  verified INTEGER DEFAULT 0,
                  primary_account INTEGER DEFAULT 0,
                  deadlock_rank INTEGER,
                  deadlock_rank_name TEXT,
                  deadlock_subrank INTEGER,
                  deadlock_badge_level INTEGER,
                  deadlock_rank_updated_at INTEGER,
                  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                  left_at INTEGER,
                  guild_id INTEGER,
                  leave_reason TEXT,
                  display_name TEXT,
                  PRIMARY KEY(user_id, steam_id)
                );

-- table: steam_nudge_state
CREATE TABLE steam_nudge_state(
          user_id     INTEGER PRIMARY KEY,
          notified_at DATETIME,
          opt_out     INTEGER DEFAULT 0,
          first_seen  DATETIME DEFAULT CURRENT_TIMESTAMP
        , message_id INTEGER, channel_id INTEGER, view_version INTEGER DEFAULT 0);

-- table: steam_presence_watchlist
CREATE TABLE steam_presence_watchlist(
              steam_id TEXT PRIMARY KEY,
              note TEXT,
              added_at INTEGER DEFAULT (strftime('%s','now'))
            );

-- table: steam_quick_invites
CREATE TABLE steam_quick_invites(
              token TEXT PRIMARY KEY,
              invite_link TEXT NOT NULL,
              invite_limit INTEGER DEFAULT 1,
              invite_duration INTEGER,
              created_at INTEGER NOT NULL,
              expires_at INTEGER,
              status TEXT DEFAULT 'available',
              reserved_by INTEGER,
              reserved_at INTEGER,
              last_seen INTEGER
            );

-- table: steam_rank_assignments
CREATE TABLE steam_rank_assignments (
               user_id  INTEGER NOT NULL,
               guild_id INTEGER NOT NULL,
               role_id  INTEGER NOT NULL,
               PRIMARY KEY (user_id, guild_id)
             );

-- table: steam_rich_presence
CREATE TABLE steam_rich_presence(
              steam_id TEXT PRIMARY KEY,
              app_id INTEGER,
              status TEXT,
              status_text TEXT,
              display TEXT,
              player_group TEXT,
              player_group_size INTEGER,
              connect TEXT,
              mode TEXT,
              map TEXT,
              party_size INTEGER,
              raw_json TEXT,
              last_update INTEGER,
              updated_at INTEGER
            );

-- table: steam_role_cleanup_pending
CREATE TABLE steam_role_cleanup_pending(
              user_id INTEGER PRIMARY KEY,
              reason TEXT NOT NULL,
              attempts INTEGER NOT NULL DEFAULT 0,
              last_error TEXT,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: steam_tasks
CREATE TABLE steam_tasks(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              type TEXT NOT NULL,
              payload TEXT,
              status TEXT NOT NULL DEFAULT 'PENDING',
              result TEXT,
              error TEXT,
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              started_at INTEGER,
              finished_at INTEGER
            , attempts INTEGER DEFAULT 0);

-- table: tempvoice_bans
CREATE TABLE tempvoice_bans (
                owner_id    BIGINT NOT NULL,
                banned_id   BIGINT NOT NULL,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (owner_id, banned_id)
            );

-- table: tempvoice_interface
CREATE TABLE tempvoice_interface (
                guild_id    INTEGER NOT NULL,
                channel_id  INTEGER NOT NULL,
                message_id  INTEGER NOT NULL,
                category_id INTEGER,
                lane_id     INTEGER,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (guild_id, message_id),
                UNIQUE(lane_id)
            );

-- table: tempvoice_lane_tag_filter
CREATE TABLE tempvoice_lane_tag_filter(
              channel_id INTEGER PRIMARY KEY,
              min_age_tag TEXT,
              required_tone_tag TEXT,
              deny_ragebaiter INTEGER DEFAULT 0,
              updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

-- table: tempvoice_lanes
CREATE TABLE tempvoice_lanes (
                channel_id  INTEGER PRIMARY KEY,
                guild_id    INTEGER NOT NULL,
                owner_id    INTEGER NOT NULL,
                base_name   TEXT NOT NULL,
                category_id INTEGER NOT NULL,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            , source_staging_id INTEGER, initial_owner_id INTEGER);

-- table: tempvoice_lurkers
CREATE TABLE tempvoice_lurkers (
                guild_id       INTEGER NOT NULL,
                channel_id     INTEGER NOT NULL,
                user_id        INTEGER NOT NULL,
                original_nick  TEXT,
                created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (channel_id, user_id)
            );

-- table: tempvoice_owner_prefs
CREATE TABLE tempvoice_owner_prefs (
                owner_id    INTEGER PRIMARY KEY,
                region      TEXT NOT NULL CHECK(region IN ('DE','EU')),
                updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

-- table: tempvoice_presets
CREATE TABLE tempvoice_presets (
                user_id     INTEGER NOT NULL,
                category_id INTEGER NOT NULL,
                name        TEXT NOT NULL,
                base_name   TEXT NOT NULL,
                "limit"     INTEGER NOT NULL,
                min_rank    TEXT NOT NULL DEFAULT 'unknown',
                region      TEXT NOT NULL DEFAULT 'EU' CHECK(region IN ('DE','EU')),
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (user_id, category_id, name)
            );

-- table: tempvoice_rank_pref
CREATE TABLE tempvoice_rank_pref (
                user_id    INTEGER PRIMARY KEY,
                rank       TEXT NOT NULL DEFAULT 'unknown',
                subrank    INTEGER NOT NULL DEFAULT 0,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

-- table: tempvoice_staging_channels
CREATE TABLE tempvoice_staging_channels (
                guild_id    INTEGER NOT NULL,
                channel_id  INTEGER NOT NULL,
                created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (guild_id, channel_id)
            );

-- table: text_conversation_log
CREATE TABLE text_conversation_log(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER,
              channel_id INTEGER,
              started_at DATETIME NOT NULL,
              ended_at DATETIME NOT NULL,
              message_count INTEGER NOT NULL DEFAULT 0,
              points INTEGER NOT NULL DEFAULT 0,
              co_participant_ids TEXT,
              had_interaction INTEGER NOT NULL DEFAULT 0
            );

-- table: text_stats
CREATE TABLE text_stats(
              user_id INTEGER PRIMARY KEY,
              total_messages INTEGER NOT NULL DEFAULT 0,
              total_points INTEGER NOT NULL DEFAULT 0,
              last_update DATETIME DEFAULT CURRENT_TIMESTAMP
            );

-- table: tierlist_build_votes
CREATE TABLE tierlist_build_votes(
              build_id INTEGER PRIMARY KEY,
              upvotes INTEGER NOT NULL DEFAULT 0,
              downvotes INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL
            );

-- table: tierlist_hero_meta
CREATE TABLE tierlist_hero_meta(
              hero_id INTEGER PRIMARY KEY,
              description TEXT NOT NULL DEFAULT '',
              updated_at INTEGER NOT NULL
            );

-- table: tierlist_settings
CREATE TABLE tierlist_settings(
              k TEXT PRIMARY KEY,
              v TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            );

-- table: tierlist_snapshot_heroes
CREATE TABLE tierlist_snapshot_heroes(
              snapshot_id INTEGER NOT NULL,
              hero_id INTEGER NOT NULL,
              matches INTEGER NOT NULL,
              wins INTEGER NOT NULL,
              losses INTEGER NOT NULL,
              winrate REAL NOT NULL,
              PRIMARY KEY(snapshot_id, hero_id),
              FOREIGN KEY(snapshot_id) REFERENCES tierlist_snapshots(id) ON DELETE CASCADE
            );

-- table: tierlist_snapshots
CREATE TABLE tierlist_snapshots(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              bucket TEXT NOT NULL,
              patch_id TEXT NOT NULL,
              patch_unix INTEGER NOT NULL,
              fetched_at INTEGER NOT NULL,
              UNIQUE(bucket, fetched_at)
            );

-- table: tierlist_streamers
CREATE TABLE tierlist_streamers(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              hero_id INTEGER NOT NULL,
              twitch_login TEXT NOT NULL,
              display_name TEXT NOT NULL,
              sort_order INTEGER NOT NULL DEFAULT 100,
              is_active INTEGER NOT NULL DEFAULT 1,
              created_at INTEGER NOT NULL,
              UNIQUE(hero_id, twitch_login)
            );

-- table: tournament_periods
CREATE TABLE tournament_periods(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              guild_id INTEGER NOT NULL,
              name TEXT NOT NULL,
              registration_start DATETIME NOT NULL,
              registration_end DATETIME NOT NULL,
              is_active INTEGER NOT NULL DEFAULT 1,
              team_size INTEGER NOT NULL DEFAULT 6,
              created_by INTEGER,
              created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

-- table: turnier_auth_tokens
CREATE TABLE turnier_auth_tokens(
              token TEXT PRIMARY KEY,
              user_id INTEGER NOT NULL,
              display_name TEXT NOT NULL,
              expires_at REAL NOT NULL
            );

-- table: user_activity_patterns
CREATE TABLE user_activity_patterns(
              user_id INTEGER PRIMARY KEY,
              typical_hours TEXT,
              typical_days TEXT,
              activity_score_2w INTEGER DEFAULT 0,
              sessions_count_2w INTEGER DEFAULT 0,
              total_minutes_2w INTEGER DEFAULT 0,
              last_active_at DATETIME,
              last_analyzed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
              last_pinged_at DATETIME,
              ping_count_30d INTEGER DEFAULT 0
            );

-- table: user_co_players
CREATE TABLE user_co_players(
              user_id INTEGER NOT NULL,
              co_player_id INTEGER NOT NULL,
              sessions_together INTEGER DEFAULT 1,
              total_minutes_together INTEGER DEFAULT 0,
              last_played_together DATETIME DEFAULT CURRENT_TIMESTAMP, user_display_name TEXT, co_player_display_name TEXT,
              PRIMARY KEY(user_id, co_player_id)
            );

-- table: user_data
CREATE TABLE user_data (
                user_id TEXT PRIMARY KEY,
                custom_interval INTEGER,
                paused_until TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

-- table: user_mod_tags
CREATE TABLE user_mod_tags(
              user_id INTEGER NOT NULL,
              mod_tag TEXT NOT NULL,
              set_by INTEGER NOT NULL,
              reason TEXT,
              set_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
              expires_at TIMESTAMP,
              PRIMARY KEY(user_id, mod_tag)
            );

-- table: user_privacy
CREATE TABLE user_privacy(
              user_id    INTEGER PRIMARY KEY,
              opted_out  INTEGER NOT NULL DEFAULT 0,
              deleted_at INTEGER,
              reason     TEXT,
              updated_at INTEGER DEFAULT (strftime('%s','now'))
            );

-- table: user_retention_messages
CREATE TABLE user_retention_messages(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                guild_id INTEGER NOT NULL,
                message_type TEXT NOT NULL, -- 'miss_you', 'welcome_back'
                sent_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                delivery_status TEXT NOT NULL DEFAULT 'sent', -- 'sent', 'failed', 'blocked'
                error_message TEXT
            );

-- table: user_retention_tracking
CREATE TABLE user_retention_tracking(
                user_id INTEGER PRIMARY KEY,
                guild_id INTEGER NOT NULL,
                first_seen_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                last_active_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                total_active_days INTEGER NOT NULL DEFAULT 0,
                avg_weekly_sessions REAL DEFAULT 0,
                last_miss_you_sent_at INTEGER,
                miss_you_count INTEGER NOT NULL DEFAULT 0,
                opted_out INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: user_tags
CREATE TABLE user_tags(
              user_id INTEGER NOT NULL,
              tag_key TEXT NOT NULL,
              tag_value TEXT NOT NULL,
              set_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
              PRIMARY KEY(user_id, tag_key)
            );

-- table: voice_channel_anchors
CREATE TABLE voice_channel_anchors (
                channel_id   INTEGER PRIMARY KEY,
                guild_id     INTEGER NOT NULL,
                user_id      INTEGER NOT NULL,
                rank_name    TEXT NOT NULL,
                rank_value   INTEGER NOT NULL,
                allowed_min  INTEGER NOT NULL,
                allowed_max  INTEGER NOT NULL,
                created_at   TEXT DEFAULT CURRENT_TIMESTAMP,
                updated_at   TEXT DEFAULT CURRENT_TIMESTAMP
            , anchor_subrank INTEGER DEFAULT 3, score_min INTEGER, score_max INTEGER);

-- table: voice_channel_settings
CREATE TABLE voice_channel_settings (
                channel_id  INTEGER PRIMARY KEY,
                guild_id    INTEGER NOT NULL,
                enabled     INTEGER NOT NULL DEFAULT 1,
                created_at  TEXT DEFAULT CURRENT_TIMESTAMP,
                updated_at  TEXT DEFAULT CURRENT_TIMESTAMP
            );

-- table: voice_feedback_requests
CREATE TABLE voice_feedback_requests(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER,
              channel_id INTEGER,
              channel_name TEXT,
              co_player_names TEXT,
              duration_seconds INTEGER,
              request_type TEXT DEFAULT 'first',
              status TEXT,
              error_message TEXT,
              prompt_message_id INTEGER,
              sent_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );

-- table: voice_feedback_responses
CREATE TABLE voice_feedback_responses(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              request_id INTEGER,
              user_id INTEGER NOT NULL,
              message_id INTEGER,
              content TEXT,
              received_at_ts INTEGER NOT NULL DEFAULT (strftime('%s','now')),
              FOREIGN KEY(request_id) REFERENCES voice_feedback_requests(id)
            );

-- table: voice_session_log
CREATE TABLE voice_session_log(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL,
              guild_id INTEGER,
              channel_id INTEGER,
              channel_name TEXT,
              started_at DATETIME NOT NULL,
              ended_at DATETIME NOT NULL,
              duration_seconds INTEGER NOT NULL DEFAULT 0,
              points INTEGER NOT NULL DEFAULT 0,
              peak_users INTEGER,
              user_counts_json TEXT
            , display_name TEXT, co_player_ids TEXT);

-- table: voice_stats
CREATE TABLE voice_stats(
              user_id       INTEGER PRIMARY KEY,
              total_seconds INTEGER NOT NULL DEFAULT 0,
              last_update   DATETIME DEFAULT CURRENT_TIMESTAMP
            , total_points INTEGER NOT NULL DEFAULT 0);

-- table: watched_build_authors
CREATE TABLE watched_build_authors (
            author_account_id INTEGER PRIMARY KEY NOT NULL,
            notes TEXT,
            is_active BOOLEAN NOT NULL DEFAULT 1,
            last_checked_at INTEGER,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
        , priority INTEGER, last_checked_status TEXT, last_checked_message TEXT);

-- index: idx_activity_patterns_last_active
CREATE INDEX idx_activity_patterns_last_active ON user_activity_patterns(last_active_at);

-- index: idx_activity_patterns_last_pinged
CREATE INDEX idx_activity_patterns_last_pinged ON user_activity_patterns(last_pinged_at);

-- index: idx_activity_patterns_score
CREATE INDEX idx_activity_patterns_score ON user_activity_patterns(activity_score_2w DESC);

-- index: idx_beta_invite_friendship_auto_poll_channel
CREATE INDEX idx_beta_invite_friendship_auto_poll_channel
              ON beta_invite_friendship_auto_poll(channel_id);

-- index: idx_beta_invite_friendship_auto_poll_due
CREATE INDEX idx_beta_invite_friendship_auto_poll_due
              ON beta_invite_friendship_auto_poll(active, next_check_at);

-- index: idx_beta_invite_pending_payments_created_at
CREATE INDEX idx_beta_invite_pending_payments_created_at ON beta_invite_pending_payments(created_at);

-- index: idx_beta_invite_pending_payments_token
CREATE INDEX idx_beta_invite_pending_payments_token ON beta_invite_pending_payments(token);

-- index: idx_beta_invite_supporter_role_grants_expires_at
CREATE INDEX idx_beta_invite_supporter_role_grants_expires_at ON beta_invite_supporter_role_grants(expires_at, revoked_at);

-- index: idx_beta_invite_tickets_channel
CREATE INDEX idx_beta_invite_tickets_channel ON beta_invite_tickets(channel_id);

-- index: idx_beta_invites_account
CREATE INDEX idx_beta_invites_account ON steam_beta_invites(account_id);

-- index: idx_beta_invites_status
CREATE INDEX idx_beta_invites_status ON steam_beta_invites(status);

-- index: idx_build_clones_hero
CREATE INDEX idx_build_clones_hero ON hero_build_clones(hero_id);

-- index: idx_build_clones_origin
CREATE INDEX idx_build_clones_origin ON hero_build_clones(origin_hero_build_id);

-- index: idx_build_clones_status
CREATE INDEX idx_build_clones_status ON hero_build_clones(status, target_language);

-- index: idx_build_sources_author
CREATE INDEX idx_build_sources_author ON hero_build_sources(author_account_id);

-- index: idx_build_sources_hero
CREATE INDEX idx_build_sources_hero ON hero_build_sources(hero_id);

-- index: idx_build_sources_origin
CREATE INDEX idx_build_sources_origin ON hero_build_sources(origin_build_id);

-- index: idx_changelog_entries_post
CREATE INDEX idx_changelog_entries_post ON changelog_entries(post_id);

-- index: idx_changelog_entries_subject
CREATE INDEX idx_changelog_entries_subject ON changelog_entries(subject);

-- index: idx_changelog_entries_subject_type
CREATE INDEX idx_changelog_entries_subject_type ON changelog_entries(subject_type);

-- index: idx_clip_contests_active
CREATE INDEX idx_clip_contests_active ON clip_contests(guild_id, start_ts, end_ts);

-- index: idx_clip_contests_guild
CREATE INDEX idx_clip_contests_guild ON clip_contests(guild_id);

-- index: idx_clip_fetch_history_streamer
CREATE INDEX idx_clip_fetch_history_streamer ON clip_fetch_history(streamer_login, fetched_at DESC);

-- index: idx_clip_submissions_guild
CREATE INDEX idx_clip_submissions_guild ON clip_submissions(guild_id);

-- index: idx_clip_templates_global_category
CREATE INDEX idx_clip_templates_global_category ON clip_templates_global(category);

-- index: idx_clip_templates_streamer_login
CREATE INDEX idx_clip_templates_streamer_login ON clip_templates_streamer(streamer_login);

-- index: idx_clip_windows_guild
CREATE INDEX idx_clip_windows_guild ON clip_windows(guild_id);

-- index: idx_co_players_co_player
CREATE INDEX idx_co_players_co_player ON user_co_players(co_player_id);

-- index: idx_co_players_user
CREATE INDEX idx_co_players_user ON user_co_players(user_id, sessions_together DESC);

-- index: idx_coach_applications_status
CREATE INDEX idx_coach_applications_status ON coach_applications(status);

-- index: idx_coaches_discord
CREATE INDEX idx_coaches_discord ON coaches(discord_user_id);

-- index: idx_coaching_requests_expires
CREATE INDEX idx_coaching_requests_expires ON coaching_requests(role_expires_at);

-- index: idx_coaching_requests_status
CREATE INDEX idx_coaching_requests_status ON coaching_requests(status, created_at);

-- index: idx_coaching_requests_user
CREATE INDEX idx_coaching_requests_user ON coaching_requests(discord_user_id);

-- index: idx_coaching_sessions_coach
CREATE INDEX idx_coaching_sessions_coach ON coaching_sessions(coach_id);

-- index: idx_coaching_sessions_expires
CREATE INDEX idx_coaching_sessions_expires ON coaching_sessions(role_expires_at);

-- index: idx_coaching_sessions_status
CREATE INDEX idx_coaching_sessions_status ON coaching_sessions(status);

-- index: idx_coaching_sessions_user
CREATE INDEX idx_coaching_sessions_user ON coaching_sessions(discord_user_id);

-- index: idx_coaching_sessions_voice
CREATE INDEX idx_coaching_sessions_voice ON coaching_sessions(voice_channel_id, survey_sent_at);

-- index: idx_coaching_surveys_session
CREATE INDEX idx_coaching_surveys_session ON coaching_surveys(session_id);

-- index: idx_coaching_thread
CREATE INDEX idx_coaching_thread ON "coaching_sessions_legacy" (thread_id);

-- index: idx_customgames_tournament_signups_guild
CREATE INDEX idx_customgames_tournament_signups_guild ON customgames_tournament_signups(guild_id, team_id);

-- index: idx_customgames_tournament_teams_guild
CREATE INDEX idx_customgames_tournament_teams_guild ON customgames_tournament_teams(guild_id, name_key);

-- index: idx_deadlock_hero_builds_build_id
CREATE INDEX idx_deadlock_hero_builds_build_id ON deadlock_hero_builds(build_id);

-- index: idx_deadlock_hero_builds_hero_active_sort
CREATE INDEX idx_deadlock_hero_builds_hero_active_sort ON deadlock_hero_builds(hero_id, is_active, sort_order);

-- index: idx_deadlock_hero_builds_sync_status
CREATE INDEX idx_deadlock_hero_builds_sync_status ON deadlock_hero_builds(sync_status, last_alerted_at);

-- index: idx_deadlock_heroes_active
CREATE INDEX idx_deadlock_heroes_active ON deadlock_heroes(is_active, hero_id);

-- index: idx_deadlock_heroes_hero
CREATE INDEX idx_deadlock_heroes_hero ON deadlock_heroes(hero_id);

-- index: idx_deadlock_party_members_party
CREATE INDEX idx_deadlock_party_members_party ON deadlock_party_members(party_id, seen_at);

-- index: idx_deadlock_party_members_steam
CREATE INDEX idx_deadlock_party_members_steam ON deadlock_party_members(steam_id, seen_at);

-- index: idx_deadlock_subrank_roles_guild
CREATE INDEX idx_deadlock_subrank_roles_guild
        ON deadlock_subrank_roles(guild_id);

-- index: idx_discord_invites_guild
CREATE INDEX idx_discord_invites_guild ON discord_invite_codes(guild_id);

-- index: idx_faq_messages_session
CREATE INDEX idx_faq_messages_session
            ON faq_chat_messages(session_id, created_at);

-- index: idx_faq_sessions_expires
CREATE INDEX idx_faq_sessions_expires
            ON faq_chat_sessions(expires_at, status);

-- index: idx_friend_requests_status
CREATE INDEX idx_friend_requests_status ON steam_friend_requests(status);

-- index: idx_hero_build_clones_origin
CREATE INDEX idx_hero_build_clones_origin
               ON hero_build_clones(origin_hero_build_id, target_language);

-- index: idx_hero_build_clones_status
CREATE INDEX idx_hero_build_clones_status
               ON hero_build_clones(status, hero_id);

-- index: idx_hero_build_clones_status_hero
CREATE INDEX idx_hero_build_clones_status_hero ON hero_build_clones(status, hero_id, created_at);

-- index: idx_hero_build_sources_author
CREATE INDEX idx_hero_build_sources_author
               ON hero_build_sources(author_account_id);

-- index: idx_hero_build_sources_hero
CREATE INDEX idx_hero_build_sources_hero
               ON hero_build_sources(hero_id);

-- index: idx_issue_reports_status
CREATE INDEX idx_issue_reports_status ON issue_reports(status, created_at);

-- index: idx_issue_reports_user
CREATE INDEX idx_issue_reports_user ON issue_reports(user_id, created_at DESC);

-- index: idx_leave_surveys_bucket
CREATE INDEX idx_leave_surveys_bucket ON member_leave_surveys(guild_id, user_bucket);

-- index: idx_leave_surveys_token
CREATE INDEX idx_leave_surveys_token ON member_leave_surveys(survey_token);

-- index: idx_leave_surveys_user
CREATE INDEX idx_leave_surveys_user ON member_leave_surveys(user_id, created_at DESC);

-- index: idx_member_events_guild
CREATE INDEX idx_member_events_guild ON member_events(guild_id, event_type, timestamp DESC);

-- index: idx_member_events_type
CREATE INDEX idx_member_events_type ON member_events(event_type, timestamp DESC);

-- index: idx_member_events_user
CREATE INDEX idx_member_events_user ON member_events(user_id, timestamp DESC);

-- index: idx_message_activity_guild
CREATE INDEX idx_message_activity_guild ON message_activity(guild_id, message_count DESC);

-- index: idx_message_activity_user
CREATE INDEX idx_message_activity_user ON message_activity(user_id, message_count DESC);

-- index: idx_mod_cases_created
CREATE INDEX idx_mod_cases_created ON ai_moderation_cases(created_at);

-- index: idx_mod_cases_user
CREATE INDEX idx_mod_cases_user ON ai_moderation_cases(user_id);

-- index: idx_oauth_states_expiry
CREATE INDEX idx_oauth_states_expiry ON oauth_states(expires_at, used);

-- index: idx_oauth_states_provider_flow
CREATE INDEX idx_oauth_states_provider_flow ON oauth_states(provider, flow_type, used);

-- index: idx_pending_payments_token
CREATE UNIQUE INDEX idx_pending_payments_token ON beta_invite_pending_payments(token);

-- index: idx_quick_invites_reserved
CREATE INDEX idx_quick_invites_reserved ON steam_quick_invites(reserved_by);

-- index: idx_quick_invites_status
CREATE INDEX idx_quick_invites_status ON steam_quick_invites(status, expires_at);

-- index: idx_ragebait_user_time
CREATE INDEX idx_ragebait_user_time ON ai_moderation_ragebait_hits(user_id, created_at);

-- index: idx_rename_requests_channel_status
CREATE INDEX idx_rename_requests_channel_status ON rename_requests(channel_id, status, id);

-- index: idx_rename_requests_status_created
CREATE INDEX idx_rename_requests_status_created ON rename_requests(status, created_at, id);

-- index: idx_retention_messages_user
CREATE INDEX idx_retention_messages_user ON user_retention_messages(user_id, sent_at);

-- index: idx_retention_tracking_guild
CREATE INDEX idx_retention_tracking_guild ON user_retention_tracking(guild_id);

-- index: idx_retention_tracking_last_active
CREATE INDEX idx_retention_tracking_last_active ON user_retention_tracking(last_active_at);

-- index: idx_standalone_commands_created
CREATE INDEX idx_standalone_commands_created ON standalone_commands(created_at);

-- index: idx_standalone_commands_status
CREATE INDEX idx_standalone_commands_status ON standalone_commands(bot, status, id);

-- index: idx_standalone_state_updated
CREATE INDEX idx_standalone_state_updated ON standalone_bot_state(updated_at);

-- index: idx_steam_beta_invites_status_schedule
CREATE INDEX idx_steam_beta_invites_status_schedule
              ON steam_beta_invites(status, scheduled_invite_at, updated_at);

-- index: idx_steam_cleanup_poll_state_polled
CREATE INDEX idx_steam_cleanup_poll_state_polled ON steam_cleanup_poll_state(last_polled_at, updated_at);

-- index: idx_steam_friendship_miss_tracker_polled
CREATE INDEX idx_steam_friendship_miss_tracker_polled ON steam_friendship_miss_tracker(last_polled_at, miss_count);

-- index: idx_steam_friendship_miss_tracker_user
CREATE INDEX idx_steam_friendship_miss_tracker_user ON steam_friendship_miss_tracker(user_id, last_polled_at);

-- index: idx_steam_launch_tokens_expires
CREATE INDEX idx_steam_launch_tokens_expires
           ON steam_launch_tokens(expires_at);

-- index: idx_steam_launch_tokens_user
CREATE INDEX idx_steam_launch_tokens_user
           ON steam_launch_tokens(user_id, expires_at);

-- index: idx_steam_links_archive_steam
CREATE INDEX idx_steam_links_archive_steam ON steam_links_archive(steam_id);

-- index: idx_steam_links_archive_user
CREATE INDEX idx_steam_links_archive_user ON steam_links_archive(user_id, left_at);

-- index: idx_steam_links_archive_user_left
CREATE INDEX idx_steam_links_archive_user_left
                  ON steam_links_archive(user_id, left_at);

-- index: idx_steam_links_friend
CREATE INDEX idx_steam_links_friend ON steam_links(is_steam_friend, user_id);

-- index: idx_steam_links_leave_archive_user_left
CREATE INDEX idx_steam_links_leave_archive_user_left
                  ON steam_links_leave_archive(user_id, left_at);

-- index: idx_steam_links_steam
CREATE INDEX idx_steam_links_steam ON steam_links(steam_id);

-- index: idx_steam_links_user
CREATE INDEX idx_steam_links_user ON steam_links(user_id);

-- index: idx_steam_links_verified
CREATE INDEX idx_steam_links_verified ON steam_links(verified, user_id);

-- index: idx_steam_nudge_optout
CREATE INDEX idx_steam_nudge_optout ON steam_nudge_state(opt_out);

-- index: idx_steam_role_cleanup_pending_updated
CREATE INDEX idx_steam_role_cleanup_pending_updated ON steam_role_cleanup_pending(updated_at);

-- index: idx_steam_tasks_attempts
CREATE INDEX idx_steam_tasks_attempts ON steam_tasks(attempts);

-- index: idx_steam_tasks_created
CREATE INDEX idx_steam_tasks_created ON steam_tasks(created_at);

-- index: idx_steam_tasks_status
CREATE INDEX idx_steam_tasks_status ON steam_tasks(status, id);

-- index: idx_steam_tasks_updated
CREATE INDEX idx_steam_tasks_updated ON steam_tasks(updated_at);

-- index: idx_steam_user
CREATE INDEX idx_steam_user   ON steam_links(user_id);

-- index: idx_tempvoice_lanes_guild
CREATE INDEX idx_tempvoice_lanes_guild ON tempvoice_lanes(guild_id, channel_id);

-- index: idx_text_log_started
CREATE INDEX idx_text_log_started ON text_conversation_log(started_at, user_id);

-- index: idx_text_log_user
CREATE INDEX idx_text_log_user ON text_conversation_log(user_id);

-- index: idx_text_stats_leaderboard
CREATE INDEX idx_text_stats_leaderboard ON text_stats(total_points DESC, total_messages DESC);

-- index: idx_tierlist_snapshot_heroes_snapshot
CREATE INDEX idx_tierlist_snapshot_heroes_snapshot
              ON tierlist_snapshot_heroes(snapshot_id);

-- index: idx_tierlist_snapshots_bucket_fetched
CREATE INDEX idx_tierlist_snapshots_bucket_fetched
              ON tierlist_snapshots(bucket, fetched_at DESC);

-- index: idx_tournament_periods_guild
CREATE INDEX idx_tournament_periods_guild
              ON tournament_periods(guild_id, is_active);

-- index: idx_user_mod_tags_active
CREATE INDEX idx_user_mod_tags_active ON user_mod_tags(user_id, expires_at);

-- index: idx_user_privacy_opted
CREATE INDEX idx_user_privacy_opted ON user_privacy(opted_out);

-- index: idx_voice_fb_req_status
CREATE INDEX idx_voice_fb_req_status ON voice_feedback_requests(status, sent_at_ts);

-- index: idx_voice_fb_req_type
CREATE INDEX idx_voice_fb_req_type ON voice_feedback_requests(request_type, sent_at_ts);

-- index: idx_voice_fb_req_user
CREATE INDEX idx_voice_fb_req_user ON voice_feedback_requests(user_id, sent_at_ts);

-- index: idx_voice_fb_resp_req
CREATE INDEX idx_voice_fb_resp_req ON voice_feedback_responses(request_id);

-- index: idx_voice_feedback_req_pending
CREATE INDEX idx_voice_feedback_req_pending ON voice_feedback_requests(status, sent_at_ts) WHERE status = 'pending';

-- index: idx_voice_log_display_name
CREATE INDEX idx_voice_log_display_name ON voice_session_log(display_name);

-- index: idx_voice_log_guild
CREATE INDEX idx_voice_log_guild ON voice_session_log(guild_id);

-- index: idx_voice_log_started
CREATE INDEX idx_voice_log_started ON voice_session_log(started_at);

-- index: idx_voice_log_started_user
CREATE INDEX idx_voice_log_started_user ON voice_session_log(started_at, user_id);

-- index: idx_voice_log_user
CREATE INDEX idx_voice_log_user ON voice_session_log(user_id);

-- index: idx_voice_stats_leaderboard
CREATE INDEX idx_voice_stats_leaderboard ON voice_stats(total_points DESC, total_seconds DESC);

-- index: idx_voice_stats_user_lookup
CREATE INDEX idx_voice_stats_user_lookup ON voice_stats(user_id, total_seconds, total_points);

-- index: uq_steam_links_steam_owner
CREATE UNIQUE INDEX uq_steam_links_steam_owner ON steam_links(steam_id) WHERE user_id != 0;

-- trigger: trg_cap_steam_tasks
CREATE TRIGGER trg_cap_steam_tasks
        AFTER INSERT ON steam_tasks
        BEGIN
          DELETE FROM steam_tasks
          WHERE id IN (
            SELECT id FROM steam_tasks
            WHERE status NOT IN ('PENDING','RUNNING')
            ORDER BY created_at ASC, id ASC
            LIMIT (
              SELECT CASE WHEN total > 1000 THEN total - 1000 ELSE 0 END
              FROM (SELECT COUNT(*) AS total FROM steam_tasks)
            )
          );
        END;

-- trigger: trg_steam_links_owner_guard_insert
CREATE TRIGGER trg_steam_links_owner_guard_insert
        BEFORE INSERT ON steam_links
        FOR EACH ROW
        WHEN NEW.user_id != 0
          AND EXISTS (
            SELECT 1
            FROM steam_links
            WHERE steam_id = NEW.steam_id
              AND user_id NOT IN (NEW.user_id, 0)
          )
        BEGIN
          SELECT RAISE(ABORT, 'steam_links ownership conflict');
        END;

-- trigger: trg_steam_links_owner_guard_update
CREATE TRIGGER trg_steam_links_owner_guard_update
        BEFORE UPDATE OF user_id, steam_id ON steam_links
        FOR EACH ROW
        WHEN NEW.user_id != 0
          AND (NEW.user_id != OLD.user_id OR NEW.steam_id != OLD.steam_id)
          AND EXISTS (
            SELECT 1
            FROM steam_links
            WHERE steam_id = NEW.steam_id
              AND user_id NOT IN (NEW.user_id, 0)
          )
        BEGIN
          SELECT RAISE(ABORT, 'steam_links ownership conflict');
        END;
