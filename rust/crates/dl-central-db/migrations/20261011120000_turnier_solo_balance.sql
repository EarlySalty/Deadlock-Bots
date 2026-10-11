CREATE TABLE turnier.solo_balance_runs (
    tournament_id BIGINT PRIMARY KEY REFERENCES turnier.tournaments(id) ON DELETE CASCADE,
    balanced_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    team_count BIGINT NOT NULL CHECK (team_count > 0),
    team_size BIGINT NOT NULL CHECK (team_size > 1)
);

CREATE INDEX solo_balance_runs_order_idx ON turnier.solo_balance_runs (balanced_at DESC, tournament_id DESC);

CREATE TABLE turnier.solo_balance_waitlist (
    tournament_id BIGINT NOT NULL REFERENCES turnier.solo_balance_runs(tournament_id) ON DELETE CASCADE,
    signup_id BIGINT NOT NULL REFERENCES turnier.tournament_signups(id) ON DELETE CASCADE,
    discord_id BIGINT NOT NULL,
    position BIGINT NOT NULL CHECK (position > 0),
    PRIMARY KEY (tournament_id, discord_id),
    UNIQUE (tournament_id, position),
    UNIQUE (signup_id)
);
