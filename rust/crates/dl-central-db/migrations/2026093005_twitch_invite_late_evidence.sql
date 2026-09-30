-- Evidence is valid by its observed time, not by when its transaction commits.
-- Only new evidence wakes an expired deadline decision; there is no expiry sweep.
CREATE TABLE activity.twitch_invite_evidence_queue (
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    marked_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (guild_id, user_id)
);

CREATE FUNCTION bot.note_twitch_invite_evidence() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    evidence_at TIMESTAMPTZ;
    target RECORD;
BEGIN
    IF TG_TABLE_NAME = 'twitch_invite_messages' THEN
        evidence_at := NEW.occurred_at;
    ELSIF TG_TABLE_NAME = 'voice_session_log' THEN
        IF NEW.duration_seconds < 900 OR NEW.channel_id IS NULL
           OR NEW.ended_at < NEW.started_at + INTERVAL '15 minutes' THEN
            RETURN NEW;
        END IF;
        evidence_at := (NEW.started_at AT TIME ZONE 'UTC') + INTERVAL '15 minutes';
    ELSE
        IF NEW.voice_qualified_at IS NULL OR
           NEW.voice_qualified_at IS NOT DISTINCT FROM OLD.voice_qualified_at THEN
            RETURN NEW;
        END IF;
        evidence_at := NEW.voice_qualified_at;
    END IF;
    -- Same ordering as the evaluator: join first, queue second. The share lock
    -- prevents qualification/expiry from overtaking the evidence transaction.
    FOR target IN
        SELECT j.guild_id, j.user_id
        FROM bot.twitch_invite_joins j
        JOIN activity.twitch_invite_members m USING (guild_id, user_id)
        WHERE j.guild_id = NEW.guild_id AND j.user_id = NEW.user_id
          AND j.eligible AND NOT m.prior_member
          AND (j.status = 'pending' OR (j.status = 'expired' AND j.reason = 'deadline'))
          AND evidence_at >= j.joined_at
          AND evidence_at <= j.joined_at + INTERVAL '720 hours'
          AND (m.left_at IS NULL OR
               GREATEST(evidence_at, j.joined_at + INTERVAL '336 hours') < m.left_at)
          AND NOT EXISTS (SELECT 1 FROM core.user_privacy p
                          WHERE p.user_id = j.user_id AND p.opted_out = TRUE)
        ORDER BY j.join_id FOR SHARE OF j
    LOOP
        INSERT INTO activity.twitch_invite_evidence_queue (guild_id, user_id)
        VALUES (target.guild_id, target.user_id)
        ON CONFLICT (guild_id, user_id) DO UPDATE SET marked_at = clock_timestamp();
    END LOOP;
    RETURN NEW;
END;
$$;

CREATE TRIGGER twitch_invite_message_evidence
    AFTER INSERT ON activity.twitch_invite_messages
    FOR EACH ROW EXECUTE FUNCTION bot.note_twitch_invite_evidence();
CREATE TRIGGER twitch_invite_session_evidence
    AFTER INSERT ON activity.voice_session_log
    FOR EACH ROW EXECUTE FUNCTION bot.note_twitch_invite_evidence();
CREATE TRIGGER twitch_invite_live_voice_evidence
    AFTER UPDATE OF voice_qualified_at ON activity.twitch_invite_members
    FOR EACH ROW EXECUTE FUNCTION bot.note_twitch_invite_evidence();

CREATE OR REPLACE FUNCTION bot.guard_twitch_invite_join() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'Twitch invite joins cannot be deleted';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.status <> 'pending' THEN
            RAISE EXCEPTION 'Twitch invite joins must start pending';
        END IF;
    ELSE
        IF (
            (OLD.status = 'pending' AND NEW.status IN ('qualified', 'expired')) OR
            (OLD.status = 'expired' AND OLD.reason = 'deadline' AND NEW.status = 'qualified')
        ) IS NOT TRUE THEN
            RAISE EXCEPTION 'Twitch invite terminal state is immutable';
        END IF;
        IF (to_jsonb(NEW) - ARRAY['status', 'qualified_at', 'updated_at', 'reason'])
            IS DISTINCT FROM (to_jsonb(OLD) - ARRAY['status', 'qualified_at', 'updated_at', 'reason']) THEN
            RAISE EXCEPTION 'Twitch invite attribution is immutable';
        END IF;
        IF OLD.status = 'expired' AND NOT EXISTS (
            SELECT 1 FROM activity.twitch_invite_members m
            WHERE m.guild_id = NEW.guild_id AND m.user_id = NEW.user_id
              AND NOT m.prior_member
              AND (m.left_at IS NULL OR NEW.qualified_at < m.left_at)
              AND NOT EXISTS (SELECT 1 FROM core.user_privacy p
                              WHERE p.user_id = NEW.user_id AND p.opted_out = TRUE)
        ) THEN
            RAISE EXCEPTION 'Twitch invite membership or privacy forbids late qualification';
        END IF;
    END IF;
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END;
$$;

DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON activity.twitch_invite_evidence_queue TO deadlock;
    END IF;
END $$;
