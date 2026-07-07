CREATE TABLE IF NOT EXISTS bot.concierge_patenschaften (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL,
    pate_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL,
    released_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX IF NOT EXISTS concierge_patenschaften_user_active_idx
    ON bot.concierge_patenschaften (user_id)
    WHERE released_at IS NULL;

CREATE INDEX IF NOT EXISTS concierge_patenschaften_pate_active_idx
    ON bot.concierge_patenschaften (pate_id)
    WHERE released_at IS NULL;

ALTER TABLE activity.journey_events
    DROP CONSTRAINT IF EXISTS journey_events_event_type_chk;

ALTER TABLE activity.journey_events
    ADD CONSTRAINT journey_events_event_type_chk CHECK (
        event_type IN (
            'join',
            'screening_completed',
            'native_onboarding_completed',
            'weiche_changed',
            'steam_link',
            'invite_friend_request_sent',
            'invite_friend_request_accepted',
            'invite_sent',
            'invite_accepted',
            'first_message',
            'first_voice',
            'first_match',
            'squad_join',
            'opt_out',
            'streamer_contact_activation',
            'd7_activity',
            'd14_activity',
            'concierge_t0_sent',
            'concierge_reply',
            'concierge_tour_done',
            'steckbrief_posted',
            'nudge_sent',
            'pate_offered',
            'pate_matched',
            'opted_out',
            'congrats_sent'
        )
    );
