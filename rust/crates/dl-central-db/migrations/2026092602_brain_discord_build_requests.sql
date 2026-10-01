-- Reserve a Discord interaction before invoking the non-idempotent Brain CLI.
-- A crashed process must not run the same interaction again after restart.
CREATE TABLE IF NOT EXISTS brain.discord_build_publish_requests (
    request_id text PRIMARY KEY,
    user_id text NOT NULL,
    result_text text,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    CONSTRAINT discord_build_request_id_format CHECK (request_id ~ '^discord:brain-build:v1:[0-9]+$'),
    CONSTRAINT discord_build_user_id_format CHECK (user_id ~ '^[0-9]+$')
);
