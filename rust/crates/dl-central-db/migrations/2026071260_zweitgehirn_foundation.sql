CREATE TABLE bot.action_outbox (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    action_type TEXT NOT NULL,
    user_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    template_id TEXT,
    anchor TEXT NOT NULL,
    idempotency_key TEXT UNIQUE NOT NULL,
    priority SMALLINT NOT NULL DEFAULT 0,
    scheduled_for TIMESTAMPTZ NOT NULL DEFAULT now(),
    status TEXT NOT NULL DEFAULT 'pending',
    suppress_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ,
    error TEXT,
    CHECK (btrim(action_type) <> ''),
    CHECK (btrim(anchor) <> ''),
    CHECK (btrim(idempotency_key) <> ''),
    CHECK (status IN ('pending', 'sent', 'failed', 'suppressed'))
);

CREATE INDEX action_outbox_pending_schedule_idx
    ON bot.action_outbox (priority DESC, scheduled_for, id)
    WHERE status = 'pending';

CREATE INDEX action_outbox_user_created_idx
    ON bot.action_outbox (user_id, created_at DESC);

CREATE VIEW activity.weekly_pulse AS
WITH bounds AS (
    SELECT now() - INTERVAL '7 days' AS period_start, now() AS period_end
),
guilds AS (
    SELECT guild_id FROM activity.voice_metadata_events
    UNION
    SELECT guild_id FROM activity.message_metadata_events
    UNION
    SELECT guild_id FROM activity.journey_events
    UNION
    SELECT guild_id FROM activity.lfg_watches
),
voice AS (
    SELECT
        events.guild_id,
        COUNT(DISTINCT events.user_id) AS voice_wau,
        ROUND(SUM(COALESCE(events.duration_seconds, 0)) / 60.0, 2) AS voice_minutes
    FROM activity.voice_metadata_events AS events
    CROSS JOIN bounds
    WHERE events.occurred_at >= bounds.period_start
      AND events.occurred_at < bounds.period_end
    GROUP BY events.guild_id
),
text AS (
    SELECT
        events.guild_id,
        COUNT(DISTINCT events.user_id) AS text_wau
    FROM activity.message_metadata_events AS events
    CROSS JOIN bounds
    WHERE events.occurred_at >= bounds.period_start
      AND events.occurred_at < bounds.period_end
    GROUP BY events.guild_id
),
members AS (
    SELECT
        events.guild_id,
        COUNT(DISTINCT events.user_id) AS new_members
    FROM activity.journey_events AS events
    CROSS JOIN bounds
    WHERE events.event_type = 'join'
      AND events.occurred_at >= bounds.period_start
      AND events.occurred_at < bounds.period_end
    GROUP BY events.guild_id
),
watches AS (
    SELECT
        watches.guild_id,
        COUNT(*) FILTER (
            WHERE watches.fired_at IS NULL AND watches.expires_at > bounds.period_end
        ) AS open_lfg_watches,
        COUNT(*) FILTER (
            WHERE watches.fired_at >= bounds.period_start
              AND watches.fired_at < bounds.period_end
        ) AS fired_lfg_watches
    FROM activity.lfg_watches AS watches
    CROSS JOIN bounds
    GROUP BY watches.guild_id
)
SELECT
    guilds.guild_id,
    bounds.period_start,
    bounds.period_end,
    COALESCE(voice.voice_wau, 0) AS voice_wau,
    COALESCE(text.text_wau, 0) AS text_wau,
    COALESCE(members.new_members, 0) AS new_members,
    COALESCE(watches.open_lfg_watches, 0) AS open_lfg_watches,
    COALESCE(watches.fired_lfg_watches, 0) AS fired_lfg_watches,
    COALESCE(voice.voice_minutes, 0) AS voice_minutes
FROM guilds
CROSS JOIN bounds
LEFT JOIN voice USING (guild_id)
LEFT JOIN text USING (guild_id)
LEFT JOIN members USING (guild_id)
LEFT JOIN watches USING (guild_id);

CREATE VIEW activity.at_risk_members AS
SELECT
    retention.user_id,
    retention.guild_id,
    retention.last_active_at,
    EXTRACT(DAY FROM now() - retention.last_active_at)::INTEGER AS inactive_days,
    patterns.last_pinged_at,
    COALESCE(patterns.ping_count_30d, 0) AS ping_count_30d
FROM activity.user_retention_tracking AS retention
LEFT JOIN activity.user_activity_patterns AS patterns USING (user_id)
WHERE retention.last_active_at >= now() - INTERVAL '21 days'
  AND retention.last_active_at <= now() - INTERVAL '7 days'
  AND NOT retention.opted_out
  AND (
      patterns.last_pinged_at IS NULL
      OR patterns.last_pinged_at <= now() - INTERVAL '14 days'
  )
  AND COALESCE(patterns.ping_count_30d, 0) < 2;
