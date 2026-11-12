SET lock_timeout TO '5s';

CREATE TABLE IF NOT EXISTS core.supporter_sources (
    discord_id BIGINT NOT NULL CHECK (discord_id > 0),
    source TEXT NOT NULL CHECK (
        source IN ('stripe', 'boost', 'twitch_sub', 'twitch_bot_customer', 'manual')
    ),
    source_ref TEXT NOT NULL CHECK (source_ref ~ '[^[:space:]]'),
    monthly_cents INTEGER CHECK (monthly_cents IS NULL OR monthly_cents >= 0),
    valid_until TIMESTAMPTZ CHECK (valid_until IS NULL OR isfinite(valid_until)),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now() CHECK (isfinite(updated_at)),
    PRIMARY KEY (discord_id, source, source_ref)
);

CREATE INDEX IF NOT EXISTS supporter_sources_source_idx
    ON core.supporter_sources (source, valid_until);

COMMENT ON TABLE core.supporter_sources IS
    'One row per Discord member and supporter source. Each source owner writes and deletes only its own rows; any row with valid_until NULL or in the future makes the member a supporter.';
COMMENT ON COLUMN core.supporter_sources.discord_id IS
    'Discord user id of the supporter; positive.';
COMMENT ON COLUMN core.supporter_sources.source IS
    'Writer of the row: stripe (Steam bot billing), boost (dl-bot from guild members), twitch_sub and twitch_bot_customer (Twitch bot), manual (role holders imported at the cutover).';
COMMENT ON COLUMN core.supporter_sources.source_ref IS
    'Id of the proof inside the source, for example the Stripe subscription id, the Twitch channel, discord for a boost or role_cutover for the manual import; never empty.';
COMMENT ON COLUMN core.supporter_sources.monthly_cents IS
    'Monthly amount in euro cents the source pays; NULL when the source carries no amount (boost, Twitch, manual).';
COMMENT ON COLUMN core.supporter_sources.valid_until IS
    'End of the support from this source; NULL = active until the writer deletes the row. Finite.';
COMMENT ON COLUMN core.supporter_sources.updated_at IS
    'Time the writer last confirmed the row. Finite.';

CREATE OR REPLACE VIEW core.active_supporters AS
SELECT DISTINCT discord_id
  FROM core.supporter_sources
 WHERE valid_until IS NULL
    OR valid_until > now();

COMMENT ON VIEW core.active_supporters IS
    'Discord members with at least one active supporter source. Read by the dl-bot role sync, the only writer of the Discord supporter role, and by the Steam bot gate.';

INSERT INTO core.privacy_field_registry(
    schema_name,
    table_name,
    column_name,
    data_category,
    retention_action,
    erasure_action,
    owner_service,
    reason
)
VALUES
    ('core', 'supporter_sources', 'discord_id', 'user_id', 'retain_operational', 'manual_review', 'dl-bot',
     'Discord user id of a supporter; source writers recreate the row while a paid or boosted source is active, so erasure is decided together with the billing record'),
    ('core', 'supporter_sources', 'source_ref', 'domain_id', 'retain_operational', 'manual_review', 'dl-bot',
     'Id of the proof inside the source (subscription id, Twitch channel); removed together with the row')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE SET
    data_category = EXCLUDED.data_category,
    retention_action = EXCLUDED.retention_action,
    erasure_action = EXCLUDED.erasure_action,
    owner_service = EXCLUDED.owner_service,
    reason = EXCLUDED.reason;

DO $roles$
DECLARE writer text;
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON core.supporter_sources TO deadlock;
        GRANT SELECT ON core.active_supporters TO deadlock;
    END IF;
    FOREACH writer IN ARRAY ARRAY['twitchbot', 'twitchdash'] LOOP
        IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = writer) THEN
            EXECUTE format('GRANT USAGE ON SCHEMA core TO %I', writer);
            EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON core.supporter_sources TO %I', writer);
            EXECUTE format('GRANT SELECT ON core.active_supporters TO %I', writer);
        END IF;
    END LOOP;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'twitchdash')
       AND to_regclass('core.discord_platform_connections') IS NOT NULL THEN
        GRANT SELECT (platform, platform_user_id, discord_id, verified)
            ON core.discord_platform_connections TO twitchdash;
    END IF;
END
$roles$;
