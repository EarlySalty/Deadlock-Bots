-- Paket D (Community-Streamer-Brücke): wöchentlicher Clip-Contest mit
-- Community-Voting und Einreichung von Twitch-Clips über den Broker.
-- Rein additiv: neue Spalten mit Default bzw. nullable, neue Tabellen.

-- ── Twitch-Einsendungen ────────────────────────────────────────────────────
-- Twitch-Clips haben keinen Discord-Einsender: user_id bleibt dort NULL, die
-- Identität steht in streamer_twitch_user_id (Twitch-User-ID als Text, wie in
-- bot.twitch_personal_invites). Discord-Einsendungen behalten user_id.
ALTER TABLE clips.clip_submissions ALTER COLUMN user_id DROP NOT NULL;
ALTER TABLE clips.clip_submissions
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'discord';
ALTER TABLE clips.clip_submissions
    ADD COLUMN IF NOT EXISTS streamer_twitch_user_id TEXT;
ALTER TABLE clips.clip_submissions
    ADD COLUMN IF NOT EXISTS streamer_login TEXT;
ALTER TABLE clips.clip_submissions
    ADD COLUMN IF NOT EXISTS submitted_by_twitch_user_id TEXT;
ALTER TABLE clips.clip_submissions
    ADD COLUMN IF NOT EXISTS title TEXT;
ALTER TABLE clips.clip_submissions
    ADD COLUMN IF NOT EXISTS idempotency_key TEXT;

ALTER TABLE clips.clip_submissions
    ADD CONSTRAINT clip_submissions_source_check
    CHECK (source IN ('discord', 'twitch'));
ALTER TABLE clips.clip_submissions
    ADD CONSTRAINT clip_submissions_twitch_identity_check
    CHECK (
        source <> 'twitch'
        OR (
            user_id IS NULL
            AND streamer_twitch_user_id IS NOT NULL
            AND streamer_twitch_user_id ~ '^[1-9][0-9]{0,19}$'
            AND (submitted_by_twitch_user_id IS NULL
                 OR submitted_by_twitch_user_id ~ '^[1-9][0-9]{0,19}$')
            AND idempotency_key IS NOT NULL
        )
    );

CREATE UNIQUE INDEX IF NOT EXISTS clip_submissions_idempotency_key_idx
    ON clips.clip_submissions (idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS clip_submissions_streamer_idx
    ON clips.clip_submissions (streamer_twitch_user_id)
    WHERE streamer_twitch_user_id IS NOT NULL;

ALTER TABLE clips.clip_window_submissions ALTER COLUMN user_id DROP NOT NULL;

-- ── Voting je Wochenfenster ────────────────────────────────────────────────
-- Eine Zeile je Fenster. Der Scheduler liest seinen Zustand nur von hier:
-- pending (Stimmzettel eingefroren) -> publishing (Post beansprucht) -> open
-- -> closing (Ergebnis gespeichert, Posts/DM laufen) -> closed. skipped, wenn
-- das Fenster keine gültigen Clips hatte.
CREATE TABLE IF NOT EXISTS clips.clip_votings (
    window_id BIGINT PRIMARY KEY REFERENCES clips.clip_windows(id) ON DELETE CASCADE,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'publishing', 'open', 'closing', 'closed', 'skipped')),
    message_id BIGINT,
    publish_claimed_at TIMESTAMPTZ,
    voting_start_at TIMESTAMPTZ,
    voting_end_at TIMESTAMPTZ,
    result_post_claimed_at TIMESTAMPTZ,
    result_message_id BIGINT,
    curator_dm_claimed_at TIMESTAMPTZ,
    curator_dm_sent_at TIMESTAMPTZ,
    closed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (status NOT IN ('open', 'closing', 'closed')
           OR (message_id IS NOT NULL AND voting_start_at IS NOT NULL
               AND voting_end_at IS NOT NULL)),
    CHECK (voting_end_at IS NULL OR voting_end_at > voting_start_at)
);

CREATE INDEX IF NOT EXISTS clip_votings_due_idx
    ON clips.clip_votings (guild_id, status, voting_end_at);

-- Eingefrorener Stimmzettel: Nummer (1..25) je Clip.
CREATE TABLE IF NOT EXISTS clips.clip_voting_entries (
    window_id BIGINT NOT NULL REFERENCES clips.clip_votings(window_id) ON DELETE CASCADE,
    position SMALLINT NOT NULL CHECK (position BETWEEN 1 AND 25),
    submission_id BIGINT NOT NULL REFERENCES clips.clip_submissions(id) ON DELETE CASCADE,
    PRIMARY KEY (window_id, position),
    UNIQUE (window_id, submission_id)
);

-- Eine Stimme je Mitglied und Woche, änderbar bis Voting-Ende. Paket C liest
-- hier die Stimmabgabe (2 Punkte je Wähler und Woche, nur status = closed).
CREATE TABLE IF NOT EXISTS clips.clip_votes (
    window_id BIGINT NOT NULL REFERENCES clips.clip_votings(window_id) ON DELETE CASCADE,
    voter_user_id BIGINT NOT NULL,
    submission_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (window_id, voter_user_id),
    FOREIGN KEY (window_id, submission_id)
        REFERENCES clips.clip_voting_entries(window_id, submission_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS clip_votes_submission_idx
    ON clips.clip_votes (window_id, submission_id);

CREATE INDEX IF NOT EXISTS clip_votes_voter_idx
    ON clips.clip_votes (voter_user_id);

-- Top 3 je Woche. Einsender ist entweder ein Discord-Mitglied (user_id) oder
-- ein Partner-Streamer (streamer_twitch_user_id). Paket C liest Platz,
-- Einsender und Woche für die Punkte 100/60/40.
CREATE TABLE IF NOT EXISTS clips.clip_contest_results (
    window_id BIGINT NOT NULL REFERENCES clips.clip_windows(id) ON DELETE CASCADE,
    place SMALLINT NOT NULL CHECK (place BETWEEN 1 AND 3),
    guild_id BIGINT NOT NULL,
    week_start_at TIMESTAMPTZ NOT NULL,
    week_end_at TIMESTAMPTZ NOT NULL,
    submission_id BIGINT REFERENCES clips.clip_submissions(id) ON DELETE SET NULL,
    source TEXT NOT NULL CHECK (source IN ('discord', 'twitch')),
    user_id BIGINT,
    streamer_twitch_user_id TEXT,
    streamer_login TEXT,
    votes INTEGER NOT NULL CHECK (votes >= 0),
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (window_id, place)
);

CREATE INDEX IF NOT EXISTS clip_contest_results_decided_idx
    ON clips.clip_contest_results (decided_at);

CREATE INDEX IF NOT EXISTS clip_contest_results_user_idx
    ON clips.clip_contest_results (user_id)
    WHERE user_id IS NOT NULL;
