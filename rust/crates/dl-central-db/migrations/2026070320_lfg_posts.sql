CREATE TABLE IF NOT EXISTS voice.lfg_posts (
    guild_id BIGINT NOT NULL,
    forum_channel_id BIGINT NOT NULL,
    thread_id BIGINT UNIQUE,
    starter_message_id BIGINT,
    lane_id BIGINT UNIQUE,
    owner_id BIGINT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('casual', 'ranked', 'street_brawl')),
    rank_min INTEGER,
    rank_max INTEGER,
    requested_slots INTEGER NOT NULL CHECK (requested_slots BETWEEN 1 AND 5),
    status TEXT NOT NULL CHECK (status IN ('creating', 'open', 'closed', 'expired')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ,
    last_render_hash TEXT,
    last_post_edit_at TIMESTAMPTZ,
    CHECK (rank_min IS NULL OR rank_min BETWEEN 1 AND 11),
    CHECK (rank_max IS NULL OR rank_max BETWEEN 1 AND 11),
    CHECK (rank_min IS NULL OR rank_max IS NULL OR rank_min <= rank_max)
);

CREATE INDEX IF NOT EXISTS lfg_posts_owner_id_idx
    ON voice.lfg_posts (owner_id);

CREATE UNIQUE INDEX IF NOT EXISTS lfg_posts_owner_active_uidx
    ON voice.lfg_posts (owner_id)
    WHERE status IN ('creating', 'open');

CREATE INDEX IF NOT EXISTS lfg_posts_status_expires_at_idx
    ON voice.lfg_posts (status, expires_at);
