UPDATE activity.journey_events
SET metadata = metadata - 'pate_id' - 'channel_id'
WHERE event_type = 'pate_matched'
  AND (metadata ? 'pate_id' OR metadata ? 'channel_id');

UPDATE activity.journey_user_state AS state
SET metadata = CASE
    WHEN EXISTS (
        SELECT 1
          FROM activity.journey_events AS event
         WHERE event.user_id = state.user_id
           AND event.guild_id = state.guild_id
           AND event.event_type = 'steckbrief_posted'
           AND event.metadata ->> 'channel_id' = state.metadata ->> 'channel_id'
    ) THEN state.metadata - 'pate_id'
    ELSE state.metadata - 'pate_id' - 'channel_id'
END
WHERE state.metadata ? 'pate_id';
