CREATE TABLE IF NOT EXISTS coaching.requests (
    request_uid TEXT PRIMARY KEY,
    bot_request_id INTEGER UNIQUE,
    website_request_id TEXT UNIQUE,
    discord_user_id BIGINT NOT NULL,
    discord_username TEXT,
    rank TEXT NOT NULL,
    subrank TEXT NOT NULL,
    hero TEXT,
    games_played TEXT,
    hours_played TEXT,
    availability TEXT,
    current_problems TEXT,
    ai_summary TEXT,
    ai_insights_json JSONB,
    status TEXT,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ,
    assigned_coach_id TEXT,
    assigned_coach_username TEXT,
    preferred_coach_id TEXT,
    reserved_until TIMESTAMPTZ,
    scheduled_slot TEXT,
    coachee_id TEXT,
    message_id BIGINT,
    channel_id BIGINT,
    role_assigned_at TIMESTAMPTZ,
    role_expires_at TIMESTAMPTZ,
    role_removed_at TIMESTAMPTZ,
    notify_discord_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS requests_discord_user_id_idx
    ON coaching.requests (discord_user_id);

CREATE INDEX IF NOT EXISTS requests_status_idx
    ON coaching.requests (status);

CREATE TABLE IF NOT EXISTS coaching.coaches (
    id TEXT PRIMARY KEY,
    discord_user_id BIGINT UNIQUE,
    discord_username TEXT,
    display_name TEXT,
    avatar_url TEXT,
    bio TEXT,
    specialties_json JSONB,
    availability_json JSONB,
    status TEXT,
    avg_rating DOUBLE PRECISION,
    total_reviews INTEGER,
    total_sessions INTEGER,
    twitch_url TEXT,
    website_coach_id TEXT,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS coaches_status_idx
    ON coaching.coaches (status);

CREATE TABLE IF NOT EXISTS coaching.coach_applications (
    id TEXT PRIMARY KEY,
    discord_user_id BIGINT NOT NULL,
    discord_username TEXT,
    display_name TEXT,
    application_text TEXT,
    experience_text TEXT,
    rank TEXT,
    specialties_json JSONB,
    availability_json JSONB,
    status TEXT,
    reviewed_by TEXT,
    reviewed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS coach_applications_discord_user_id_idx
    ON coaching.coach_applications (discord_user_id);

CREATE INDEX IF NOT EXISTS coach_applications_status_idx
    ON coaching.coach_applications (status);

CREATE TABLE IF NOT EXISTS coaching.bans (
    discord_user_id BIGINT PRIMARY KEY,
    banned_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    reason TEXT
);

CREATE TABLE IF NOT EXISTS coaching.coach_rotation (
    coach_id TEXT PRIMARY KEY,
    last_assigned_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS coaching.sessions (
    id TEXT PRIMARY KEY,
    request_uid TEXT,
    bot_request_id INTEGER,
    website_request_id TEXT,
    coach_id TEXT,
    discord_user_id BIGINT,
    discord_username TEXT,
    discord_channel_id BIGINT,
    discord_thread_id BIGINT,
    status TEXT,
    role_assigned_at TIMESTAMPTZ,
    role_reminder_at TIMESTAMPTZ,
    role_expires_at TIMESTAMPTZ,
    voice_channel_id BIGINT,
    voice_started_at TIMESTAMPTZ,
    voice_last_seen_at TIMESTAMPTZ,
    survey_sent_at TIMESTAMPTZ,
    scheduled_at TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ,
    reward_role_expires_at TIMESTAMPTZ,
    reward_role_removed_at TIMESTAMPTZ,
    coachee_id TEXT,
    bot_session_id TEXT
);

CREATE INDEX IF NOT EXISTS sessions_request_uid_idx
    ON coaching.sessions (request_uid);

CREATE INDEX IF NOT EXISTS sessions_bot_request_id_idx
    ON coaching.sessions (bot_request_id);

CREATE INDEX IF NOT EXISTS sessions_website_request_id_idx
    ON coaching.sessions (website_request_id);

CREATE INDEX IF NOT EXISTS sessions_discord_user_id_idx
    ON coaching.sessions (discord_user_id);

CREATE TABLE IF NOT EXISTS coaching.sessions_legacy (
    user_id BIGINT PRIMARY KEY,
    thread_id BIGINT,
    match_id TEXT,
    rank TEXT,
    subrank TEXT,
    hero TEXT,
    comment TEXT,
    step TEXT,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ,
    is_active BOOLEAN,
    voice_channel_id BIGINT,
    voice_started_at TIMESTAMPTZ,
    voice_last_seen_at TIMESTAMPTZ,
    survey_sent_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS coaching.surveys (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    rating INTEGER,
    feedback_text TEXT,
    improved_areas TEXT,
    unresolved_items TEXT,
    would_recommend BOOLEAN,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS surveys_session_id_idx
    ON coaching.surveys (session_id);

CREATE TABLE IF NOT EXISTS coaching.coach_reviews (
    id TEXT PRIMARY KEY,
    coach_id TEXT,
    session_id TEXT,
    user_display_name TEXT,
    rating INTEGER,
    feedback_text TEXT,
    improved_areas TEXT,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS coach_reviews_coach_id_idx
    ON coaching.coach_reviews (coach_id);

CREATE TABLE IF NOT EXISTS coaching.coachees (
    id TEXT PRIMARY KEY,
    discord_user_id BIGINT NOT NULL UNIQUE,
    discord_username TEXT,
    display_name TEXT,
    rank TEXT,
    main_heroes_json JSONB,
    current_focus TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS coaching.appointments (
    id TEXT PRIMARY KEY,
    coach_id TEXT,
    coachee_id TEXT,
    scheduled_at TIMESTAMPTZ NOT NULL,
    duration_minutes INTEGER,
    title TEXT,
    note TEXT,
    status TEXT,
    notify_created_at TIMESTAMPTZ,
    notify_reminder_at TIMESTAMPTZ,
    notify_cancelled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS appointments_coach_id_idx
    ON coaching.appointments (coach_id);

CREATE INDEX IF NOT EXISTS appointments_coachee_id_idx
    ON coaching.appointments (coachee_id);

CREATE INDEX IF NOT EXISTS appointments_scheduled_at_idx
    ON coaching.appointments (scheduled_at);

CREATE TABLE IF NOT EXISTS coaching.goals (
    id TEXT PRIMARY KEY,
    coachee_id TEXT,
    coach_id TEXT,
    session_id TEXT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT,
    sort_order INTEGER,
    target_date DATE,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS goals_coachee_id_idx
    ON coaching.goals (coachee_id);

CREATE TABLE IF NOT EXISTS coaching.milestones (
    id TEXT PRIMARY KEY,
    goal_id TEXT,
    title TEXT NOT NULL,
    description TEXT,
    achieved BOOLEAN,
    achieved_at TIMESTAMPTZ,
    sort_order INTEGER,
    created_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS milestones_goal_id_idx
    ON coaching.milestones (goal_id);

CREATE TABLE IF NOT EXISTS coaching.session_notes (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    coachee_id TEXT,
    coach_id TEXT,
    content TEXT,
    visibility TEXT,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS session_notes_session_id_idx
    ON coaching.session_notes (session_id);

CREATE TABLE IF NOT EXISTS scrim.participants (
    id INTEGER PRIMARY KEY,
    discord_id BIGINT,
    display_name TEXT NOT NULL,
    rank TEXT,
    rank_source TEXT NOT NULL,
    rank_verified BOOLEAN NOT NULL DEFAULT false,
    roles TEXT,
    availability TEXT,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS participants_discord_id_idx
    ON scrim.participants (discord_id);

CREATE TABLE IF NOT EXISTS scrim.teams (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    coach TEXT,
    discord_role_id BIGINT,
    discord_channel_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS scrim.team_members (
    team_id INTEGER NOT NULL,
    participant_id INTEGER NOT NULL,
    role TEXT,
    is_captain BOOLEAN NOT NULL DEFAULT false,
    is_bench BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (team_id, participant_id)
);

CREATE INDEX IF NOT EXISTS team_members_participant_id_idx
    ON scrim.team_members (participant_id);

CREATE TABLE IF NOT EXISTS scrim.matches (
    id INTEGER PRIMARY KEY,
    team_a_id INTEGER,
    team_b_id INTEGER,
    when_text TEXT,
    scheduled_at TIMESTAMPTZ,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS matches_team_a_id_idx
    ON scrim.matches (team_a_id);

CREATE INDEX IF NOT EXISTS matches_team_b_id_idx
    ON scrim.matches (team_b_id);
