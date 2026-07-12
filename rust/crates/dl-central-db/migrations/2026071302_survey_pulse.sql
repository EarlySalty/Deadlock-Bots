CREATE TABLE bot.survey_waves (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    config_snapshot JSONB NOT NULL
);

CREATE TABLE bot.survey_responses (
    wave_id BIGINT NOT NULL REFERENCES bot.survey_waves(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL,
    satisfaction SMALLINT CHECK (satisfaction BETWEEN 1 AND 5),
    events TEXT[],
    freitext TEXT,
    answered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (wave_id, user_id)
);

CREATE INDEX survey_responses_user_idx
    ON bot.survey_responses (user_id, answered_at DESC);

CREATE VIEW bot.survey_wave_summary AS
WITH invitations AS (
    SELECT
        (outbox.payload ->> 'wave_id')::BIGINT AS wave_id,
        COUNT(DISTINCT outbox.user_id) AS invited_count
    FROM bot.action_outbox AS outbox
    WHERE outbox.action_type = 'survey_pulse'
    GROUP BY (outbox.payload ->> 'wave_id')::BIGINT
),
response_totals AS (
    SELECT
        responses.wave_id,
        COUNT(*) AS response_count,
        AVG(responses.satisfaction) AS average_satisfaction
    FROM bot.survey_responses AS responses
    GROUP BY responses.wave_id
),
event_totals AS (
    SELECT
        responses.wave_id,
        event_name,
        COUNT(*) AS event_count
    FROM bot.survey_responses AS responses
    CROSS JOIN LATERAL unnest(COALESCE(responses.events, ARRAY[]::TEXT[])) AS event_name
    GROUP BY responses.wave_id, event_name
),
event_json AS (
    SELECT
        wave_id,
        jsonb_object_agg(event_name, event_count ORDER BY event_name) AS event_counts
    FROM event_totals
    GROUP BY wave_id
)
SELECT
    waves.id AS wave_id,
    waves.started_at,
    waves.config_snapshot,
    COALESCE(invitations.invited_count, 0) AS invited_count,
    COALESCE(response_totals.response_count, 0) AS response_count,
    CASE
        WHEN COALESCE(invitations.invited_count, 0) = 0 THEN 0::NUMERIC
        ELSE ROUND(
            COALESCE(response_totals.response_count, 0)::NUMERIC
            / invitations.invited_count,
            4
        )
    END AS response_rate,
    response_totals.average_satisfaction,
    COALESCE(event_json.event_counts, '{}'::JSONB) AS event_counts
FROM bot.survey_waves AS waves
LEFT JOIN invitations ON invitations.wave_id = waves.id
LEFT JOIN response_totals ON response_totals.wave_id = waves.id
LEFT JOIN event_json ON event_json.wave_id = waves.id;
