-- activity.lfg_watches: opt-in Match-Benachrichtigungen (one-shot, selbst-ablaufend)
CREATE TABLE IF NOT EXISTS activity.lfg_watches (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT      NOT NULL,
    guild_id        BIGINT      NOT NULL,
    mode            TEXT        NOT NULL CHECK (mode IN ('casual', 'ranked', 'street_brawl')),
    rank_min        INTEGER     CHECK (rank_min IS NULL OR rank_min BETWEEN 1 AND 11),
    rank_max        INTEGER     CHECK (rank_max IS NULL OR rank_max BETWEEN 1 AND 11),
    window_kind     TEXT        NOT NULL CHECK (window_kind IN ('now3h', 'today', 'weekend', 'week')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    fired_at        TIMESTAMPTZ,
    matched_post_id BIGINT,
    CHECK (rank_min IS NULL OR rank_max IS NULL OR rank_min <= rank_max)
);

-- Matcher-Query: scharfe, unabgelaufene Watches je Modus.
CREATE INDEX IF NOT EXISTS lfg_watches_arm_idx
    ON activity.lfg_watches (guild_id, mode, expires_at)
    WHERE fired_at IS NULL;

-- Ein scharfer Watch pro User (erneutes Aktivieren ersetzt via Upsert).
CREATE UNIQUE INDEX IF NOT EXISTS lfg_watches_one_armed_per_user_uidx
    ON activity.lfg_watches (guild_id, user_id)
    WHERE fired_at IS NULL;

-- Teil A: grobes Spielfenster des Gesuchs.
ALTER TABLE voice.lfg_posts
    ADD COLUMN IF NOT EXISTS play_window TEXT
    CHECK (play_window IS NULL OR play_window IN ('jetzt', 'heute_abend', 'wochenende', 'flexibel'));
