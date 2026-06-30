CREATE TABLE IF NOT EXISTS activity.live_player_state (
    steam_id TEXT PRIMARY KEY,
    last_gameid TEXT,
    last_server_id TEXT,
    last_seen_at TIMESTAMPTZ,
    in_deadlock_now BOOLEAN,
    in_match_now_strict BOOLEAN,
    deadlock_stage TEXT,
    deadlock_minutes INTEGER,
    deadlock_localized TEXT,
    deadlock_hero TEXT,
    deadlock_party_hint TEXT,
    deadlock_updated_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS live_player_state_last_seen_idx
    ON activity.live_player_state (last_seen_at DESC);

CREATE INDEX IF NOT EXISTS live_player_state_deadlock_now_idx
    ON activity.live_player_state (in_deadlock_now, in_match_now_strict);

CREATE TABLE IF NOT EXISTS core.user_data (
    user_id BIGINT PRIMARY KEY,
    custom_interval INTEGER,
    paused_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS core.user_mod_tags (
    user_id BIGINT NOT NULL,
    mod_tag TEXT NOT NULL,
    set_by BIGINT NOT NULL,
    reason TEXT,
    set_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, mod_tag)
);

CREATE INDEX IF NOT EXISTS user_mod_tags_set_by_idx
    ON core.user_mod_tags (set_by);

CREATE INDEX IF NOT EXISTS user_mod_tags_expires_at_idx
    ON core.user_mod_tags (expires_at)
    WHERE expires_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS core.user_tags (
    user_id BIGINT NOT NULL,
    tag_key TEXT NOT NULL,
    tag_value TEXT NOT NULL,
    set_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, tag_key)
);

CREATE INDEX IF NOT EXISTS user_tags_tag_key_value_idx
    ON core.user_tags (tag_key, tag_value);
