ALTER TABLE bot.twitch_streamer_invites ADD COLUMN twitch_user_id TEXT;
ALTER TABLE bot.twitch_streamer_invites ADD COLUMN channel_id BIGINT;
CREATE INDEX twitch_streamer_invites_user_idx ON bot.twitch_streamer_invites (twitch_user_id);

CREATE TABLE bot.twitch_personal_invites (
    streamer_twitch_user_id TEXT NOT NULL CHECK (streamer_twitch_user_id ~ '^[1-9][0-9]{0,19}$'),
    inviter_twitch_user_id TEXT NOT NULL CHECK (inviter_twitch_user_id ~ '^[1-9][0-9]{0,19}$'),
    streamer_login TEXT NOT NULL,
    guild_id BIGINT NOT NULL CHECK (guild_id > 0),
    channel_id BIGINT NOT NULL CHECK (channel_id > 0),
    invite_code TEXT NOT NULL,
    invite_url TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    revoked_at TIMESTAMPTZ,
    PRIMARY KEY (streamer_twitch_user_id, inviter_twitch_user_id),
    UNIQUE (guild_id, invite_code),
    CHECK (streamer_twitch_user_id <> inviter_twitch_user_id)
);

CREATE TABLE activity.twitch_invite_tracking (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
INSERT INTO activity.twitch_invite_tracking (singleton) VALUES (TRUE);

CREATE TABLE activity.twitch_invite_members (
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    first_join_id BIGINT,
    first_joined_at TIMESTAMPTZ,
    current_joined_at TIMESTAMPTZ,
    prior_member BOOLEAN NOT NULL,
    left_at TIMESTAMPTZ,
    voice_channel_id BIGINT,
    voice_started_at TIMESTAMPTZ,
    voice_observed_at TIMESTAMPTZ,
    voice_qualified_at TIMESTAMPTZ,
    PRIMARY KEY (guild_id, user_id)
);

CREATE INDEX twitch_invite_members_first_join_idx ON activity.twitch_invite_members (first_join_id);

INSERT INTO activity.twitch_invite_members (guild_id, user_id, first_joined_at, prior_member)
SELECT guild_id, user_id, MIN(seen_at), TRUE
FROM (
    SELECT guild_id, user_id, occurred_at AS seen_at FROM activity.member_events
    UNION ALL
    SELECT guild_id, user_id, first_message_at FROM activity.message_activity
    UNION ALL
    SELECT guild_id, user_id, started_at FROM activity.voice_session_log WHERE guild_id IS NOT NULL
) AS known_members
GROUP BY guild_id, user_id;

CREATE TABLE activity.twitch_invite_messages (
    message_id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX twitch_invite_messages_member_time_idx
    ON activity.twitch_invite_messages (guild_id, user_id, occurred_at, message_id);

CREATE TABLE bot.twitch_invite_joins (
    join_id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    streamer_login TEXT NOT NULL,
    streamer_twitch_user_id TEXT,
    inviter_twitch_user_id TEXT,
    invite_code TEXT NOT NULL,
    joined_at TIMESTAMPTZ NOT NULL,
    eligible BOOLEAN NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'qualified', 'expired')),
    qualified_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    reason TEXT,
    UNIQUE (guild_id, user_id, joined_at),
    CHECK ((status = 'qualified') = (qualified_at IS NOT NULL)),
    CHECK (qualified_at IS NULL OR (
        eligible AND qualified_at >= joined_at + INTERVAL '336 hours'
        AND qualified_at <= joined_at + INTERVAL '720 hours'
    ))
);
CREATE UNIQUE INDEX twitch_invite_joins_one_credit_idx
    ON bot.twitch_invite_joins (guild_id, user_id) WHERE status = 'qualified';
CREATE INDEX twitch_invite_joins_pending_idx
    ON bot.twitch_invite_joins (guild_id, join_id) WHERE status = 'pending';
CREATE INDEX twitch_invite_joins_changes_idx
    ON bot.twitch_invite_joins (guild_id, updated_at, join_id);
CREATE INDEX member_events_invite_lookup_idx
    ON activity.member_events (guild_id, occurred_at, id) WHERE event_type = 'join';

CREATE TABLE bot.twitch_invite_transitions (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    join_id BIGINT NOT NULL REFERENCES bot.twitch_invite_joins (join_id),
    status TEXT NOT NULL CHECK (status IN ('pending', 'qualified', 'expired')),
    occurred_at TIMESTAMPTZ NOT NULL,
    qualified_at TIMESTAMPTZ,
    reason TEXT,
    UNIQUE (join_id, status)
);

CREATE FUNCTION bot.guard_twitch_invite_join() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'Twitch invite joins cannot be deleted';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.status <> 'pending' THEN
            RAISE EXCEPTION 'Twitch invite joins must start pending';
        END IF;
    ELSE
        IF OLD.status <> 'pending' OR NEW.status NOT IN ('qualified', 'expired') THEN
            RAISE EXCEPTION 'Twitch invite terminal state is immutable';
        END IF;
        IF (to_jsonb(NEW) - ARRAY['status', 'qualified_at', 'updated_at', 'reason'])
            IS DISTINCT FROM (to_jsonb(OLD) - ARRAY['status', 'qualified_at', 'updated_at', 'reason']) THEN
            RAISE EXCEPTION 'Twitch invite attribution is immutable';
        END IF;
    END IF;
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END;
$$;
CREATE TRIGGER twitch_invite_join_guard
    BEFORE INSERT OR UPDATE OR DELETE ON bot.twitch_invite_joins
    FOR EACH ROW EXECUTE FUNCTION bot.guard_twitch_invite_join();

CREATE FUNCTION bot.audit_twitch_invite_join() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO bot.twitch_invite_transitions (join_id, status, occurred_at, qualified_at, reason)
    VALUES (NEW.join_id, NEW.status, NEW.updated_at, NEW.qualified_at, NEW.reason);
    RETURN NEW;
END;
$$;
CREATE TRIGGER twitch_invite_join_audit
    AFTER INSERT OR UPDATE ON bot.twitch_invite_joins
    FOR EACH ROW EXECUTE FUNCTION bot.audit_twitch_invite_join();

CREATE FUNCTION bot.guard_twitch_invite_history() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'Twitch invite history is append-only';
END;
$$;
CREATE TRIGGER twitch_invite_transition_guard
    BEFORE UPDATE OR DELETE ON bot.twitch_invite_transitions
    FOR EACH ROW EXECUTE FUNCTION bot.guard_twitch_invite_history();
CREATE TRIGGER twitch_invite_member_delete_guard
    BEFORE DELETE ON activity.twitch_invite_members
    FOR EACH ROW EXECUTE FUNCTION bot.guard_twitch_invite_history();
