CREATE TABLE bot.twitch_invite_qualification_status (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    last_completed_at TIMESTAMPTZ NOT NULL,
    last_successful_at TIMESTAMPTZ,
    evaluation_interval_seconds INTEGER NOT NULL
        CHECK (evaluation_interval_seconds BETWEEN 1 AND 86400),
    healthy BOOLEAN NOT NULL,
    CHECK (NOT healthy OR (last_successful_at IS NOT NULL AND last_successful_at = last_completed_at))
);

CREATE FUNCTION bot.record_twitch_invite_qualification(
    succeeded BOOLEAN,
    evaluation_interval INTEGER
) RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, bot
AS $$
DECLARE
    completed_at TIMESTAMPTZ := clock_timestamp();
BEGIN
    IF evaluation_interval NOT BETWEEN 1 AND 86400 THEN
        RAISE EXCEPTION 'invalid Twitch invite qualification interval';
    END IF;

    INSERT INTO bot.twitch_invite_qualification_status AS current_status (
        singleton,
        last_completed_at,
        last_successful_at,
        evaluation_interval_seconds,
        healthy
    ) VALUES (
        TRUE,
        completed_at,
        CASE WHEN succeeded THEN completed_at END,
        evaluation_interval,
        succeeded
    )
    ON CONFLICT (singleton) DO UPDATE SET
        last_completed_at = EXCLUDED.last_completed_at,
        last_successful_at = CASE
            WHEN EXCLUDED.healthy THEN EXCLUDED.last_completed_at
            ELSE current_status.last_successful_at
        END,
        evaluation_interval_seconds = EXCLUDED.evaluation_interval_seconds,
        healthy = EXCLUDED.healthy;
END;
$$;

REVOKE ALL ON FUNCTION bot.record_twitch_invite_qualification(BOOLEAN, INTEGER) FROM PUBLIC;

DO $roles$
DECLARE
    reader TEXT;
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        EXECUTE 'GRANT EXECUTE ON FUNCTION bot.record_twitch_invite_qualification(BOOLEAN, INTEGER) TO deadlock';
    END IF;

    FOREACH reader IN ARRAY ARRAY['twitchbot', 'twitchdash'] LOOP
        IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = reader) THEN
            EXECUTE format('GRANT SELECT ON bot.twitch_invite_qualification_status TO %I', reader);
        END IF;
    END LOOP;
END
$roles$;
