CREATE TABLE IF NOT EXISTS activity.member_events (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    occurred_at TIMESTAMPTZ,
    display_name TEXT,
    account_created_at TIMESTAMPTZ,
    join_position INTEGER,
    metadata JSONB
);

CREATE INDEX IF NOT EXISTS member_events_user_occurred_at_idx
    ON activity.member_events (user_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS member_events_guild_event_type_idx
    ON activity.member_events (guild_id, event_type);

CREATE TABLE IF NOT EXISTS activity.member_leave_surveys (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    left_at TIMESTAMPTZ NOT NULL,
    display_name TEXT,
    user_bucket TEXT NOT NULL,
    days_on_server INTEGER,
    survey_token TEXT NOT NULL,
    dm_status TEXT,
    reason_code TEXT,
    follow_up_question TEXT,
    follow_up_text TEXT,
    extra_text TEXT,
    responded_at TIMESTAMPTZ,
    web_submitted_at TIMESTAMPTZ,
    web_payload JSONB,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS member_leave_surveys_user_left_at_idx
    ON activity.member_leave_surveys (user_id, left_at DESC);

CREATE TABLE IF NOT EXISTS activity.message_activity (
    user_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT,
    message_count BIGINT,
    last_message_at TIMESTAMPTZ,
    first_message_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, guild_id)
);

CREATE INDEX IF NOT EXISTS message_activity_guild_last_message_idx
    ON activity.message_activity (guild_id, last_message_at DESC);

CREATE TABLE IF NOT EXISTS activity.text_conversation_log (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    guild_id BIGINT,
    channel_id BIGINT,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ NOT NULL,
    message_count INTEGER NOT NULL,
    points INTEGER NOT NULL,
    co_participant_ids JSONB,
    had_interaction BOOLEAN NOT NULL DEFAULT false
);

CREATE INDEX IF NOT EXISTS text_conversation_log_user_started_idx
    ON activity.text_conversation_log (user_id, started_at DESC);

CREATE TABLE IF NOT EXISTS activity.text_stats (
    user_id BIGINT PRIMARY KEY,
    total_messages BIGINT NOT NULL DEFAULT 0,
    total_points BIGINT NOT NULL DEFAULT 0,
    last_update TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS activity.user_activity_patterns (
    user_id BIGINT PRIMARY KEY,
    typical_hours JSONB,
    typical_days JSONB,
    activity_score_2w INTEGER,
    sessions_count_2w INTEGER,
    total_minutes_2w INTEGER,
    last_active_at TIMESTAMPTZ,
    last_analyzed_at TIMESTAMPTZ,
    last_pinged_at TIMESTAMPTZ,
    ping_count_30d INTEGER
);

CREATE INDEX IF NOT EXISTS user_activity_patterns_last_active_idx
    ON activity.user_activity_patterns (last_active_at DESC);

CREATE TABLE IF NOT EXISTS activity.user_co_players (
    user_id BIGINT NOT NULL,
    co_player_id BIGINT NOT NULL,
    sessions_together INTEGER,
    total_minutes_together INTEGER,
    last_played_together TIMESTAMPTZ,
    user_display_name TEXT,
    co_player_display_name TEXT,
    PRIMARY KEY (user_id, co_player_id)
);

CREATE INDEX IF NOT EXISTS user_co_players_last_played_idx
    ON activity.user_co_players (last_played_together DESC);

CREATE TABLE IF NOT EXISTS activity.user_retention_messages (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    message_type TEXT NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL,
    delivery_status TEXT NOT NULL,
    error_message TEXT
);

CREATE INDEX IF NOT EXISTS user_retention_messages_user_sent_idx
    ON activity.user_retention_messages (user_id, sent_at DESC);

CREATE TABLE IF NOT EXISTS activity.user_retention_tracking (
    user_id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL,
    last_active_at TIMESTAMPTZ NOT NULL,
    total_active_days INTEGER NOT NULL DEFAULT 0,
    avg_weekly_sessions DOUBLE PRECISION,
    last_miss_you_sent_at TIMESTAMPTZ,
    miss_you_count INTEGER NOT NULL DEFAULT 0,
    opted_out BOOLEAN NOT NULL DEFAULT false,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS user_retention_tracking_guild_active_idx
    ON activity.user_retention_tracking (guild_id, last_active_at DESC);

CREATE TABLE IF NOT EXISTS activity.voice_feedback_requests (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    guild_id BIGINT,
    channel_id BIGINT,
    channel_name TEXT,
    co_player_names TEXT,
    duration_seconds BIGINT,
    request_type TEXT,
    status TEXT,
    error_message TEXT,
    prompt_message_id BIGINT,
    sent_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS voice_feedback_requests_user_sent_idx
    ON activity.voice_feedback_requests (user_id, sent_at DESC);

CREATE TABLE IF NOT EXISTS activity.voice_feedback_responses (
    id BIGINT PRIMARY KEY,
    request_id BIGINT,
    user_id BIGINT NOT NULL,
    message_id BIGINT,
    content TEXT,
    received_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS voice_feedback_responses_request_idx
    ON activity.voice_feedback_responses (request_id);

CREATE TABLE IF NOT EXISTS activity.voice_session_log (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    guild_id BIGINT,
    channel_id BIGINT,
    channel_name TEXT,
    started_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ NOT NULL,
    duration_seconds BIGINT NOT NULL,
    points INTEGER NOT NULL,
    peak_users INTEGER,
    user_counts JSONB,
    display_name TEXT,
    co_player_ids JSONB
);

CREATE INDEX IF NOT EXISTS voice_session_log_user_started_idx
    ON activity.voice_session_log (user_id, started_at DESC);

CREATE TABLE IF NOT EXISTS moderation.ai_moderation_cases (
    case_id TEXT PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    message_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    user_tag TEXT,
    original_content TEXT,
    attachments JSONB,
    ai_category TEXT,
    ai_confidence DOUBLE PRECISION,
    ai_reason TEXT,
    ai_raw JSONB,
    escalated_with_context BOOLEAN,
    action TEXT,
    mod_id BIGINT,
    mod_action_at TIMESTAMPTZ,
    mod_deny_reason TEXT,
    mod_review_message_id BIGINT,
    log_message_id BIGINT,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS ai_moderation_cases_user_created_idx
    ON moderation.ai_moderation_cases (user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS ai_moderation_cases_message_idx
    ON moderation.ai_moderation_cases (guild_id, channel_id, message_id);

CREATE TABLE IF NOT EXISTS moderation.ai_moderation_ragebait_hits (
    id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    message_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    content_preview TEXT,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS ai_moderation_ragebait_hits_user_created_idx
    ON moderation.ai_moderation_ragebait_hits (guild_id, user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS moderation.security_guard_incidents (
    case_id TEXT PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    user_tag TEXT NOT NULL,
    action TEXT NOT NULL,
    reason TEXT NOT NULL,
    channel_count INTEGER,
    message_count INTEGER,
    attachment_count INTEGER,
    keyword_hit BOOLEAN,
    messages JSONB,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS security_guard_incidents_user_created_idx
    ON moderation.security_guard_incidents (user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS content.meta_announcements (
    id TEXT PRIMARY KEY,
    message TEXT,
    is_active BOOLEAN,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS meta_announcements_active_idx
    ON content.meta_announcements (is_active);

CREATE TABLE IF NOT EXISTS content.meta_reports (
    id TEXT PRIMARY KEY,
    build_id TEXT,
    reporter_id TEXT,
    reporter_name TEXT,
    reason TEXT,
    status TEXT,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS meta_reports_build_status_idx
    ON content.meta_reports (build_id, status);

CREATE TABLE IF NOT EXISTS patchnotes.changelog_posts (
    id BIGINT PRIMARY KEY,
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    posted_at TIMESTAMPTZ,
    raw_content TEXT,
    translated_content TEXT
);

CREATE INDEX IF NOT EXISTS changelog_posts_posted_at_idx
    ON patchnotes.changelog_posts (posted_at DESC);

CREATE TABLE IF NOT EXISTS patchnotes.deadlock_changelogs (
    id BIGINT PRIMARY KEY,
    title TEXT,
    url TEXT,
    posted_at TIMESTAMPTZ,
    content TEXT
);

CREATE INDEX IF NOT EXISTS deadlock_changelogs_posted_at_idx
    ON patchnotes.deadlock_changelogs (posted_at DESC);

CREATE TABLE IF NOT EXISTS patchnotes.meta_patch_notes (
    id TEXT PRIMARY KEY,
    title TEXT,
    content TEXT,
    version TEXT,
    created_at TIMESTAMPTZ
);
