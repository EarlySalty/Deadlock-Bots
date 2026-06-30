CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE SCHEMA IF NOT EXISTS core;
CREATE SCHEMA IF NOT EXISTS coaching;
CREATE SCHEMA IF NOT EXISTS scrim;
CREATE SCHEMA IF NOT EXISTS steam;
CREATE SCHEMA IF NOT EXISTS turnier;
CREATE SCHEMA IF NOT EXISTS patchnotes;
CREATE SCHEMA IF NOT EXISTS activity;

CREATE TABLE IF NOT EXISTS core.users (
    discord_id BIGINT PRIMARY KEY,
    username TEXT,
    global_name TEXT,
    avatar TEXT,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    raw JSONB
);

CREATE TABLE IF NOT EXISTS core.steam_links (
    discord_id BIGINT NOT NULL REFERENCES core.users(discord_id) ON DELETE CASCADE,
    steam_id64 BIGINT NOT NULL,
    verified BOOLEAN NOT NULL DEFAULT false,
    linked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (discord_id, steam_id64)
);

CREATE INDEX IF NOT EXISTS steam_links_steam_id64_idx ON core.steam_links (steam_id64);
