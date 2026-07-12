CREATE TABLE IF NOT EXISTS voice.voice_pair_guard_locks (
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    blocked_user_id BIGINT NOT NULL,
    previous_allow BIGINT NOT NULL,
    previous_deny BIGINT NOT NULL,
    had_overwrite BOOLEAN NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    PRIMARY KEY (guild_id, channel_id, blocked_user_id)
);
