CREATE INDEX IF NOT EXISTS concierge_profiles_fallback_channel_idx
    ON bot.concierge_profiles (fallback_channel_id)
    WHERE fallback_channel_id IS NOT NULL;
