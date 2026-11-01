CREATE TABLE IF NOT EXISTS coaching.request_opt_outs (
    bot_request_id INTEGER NOT NULL,
    coach_discord_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (bot_request_id, coach_discord_id),
    CONSTRAINT request_opt_outs_request_fk
        FOREIGN KEY (bot_request_id)
        REFERENCES coaching.requests (bot_request_id)
        ON DELETE CASCADE
        ON UPDATE CASCADE
);
