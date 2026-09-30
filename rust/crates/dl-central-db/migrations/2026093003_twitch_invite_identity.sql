CREATE UNIQUE INDEX twitch_streamer_invites_user_unique_idx
    ON bot.twitch_streamer_invites (twitch_user_id)
    WHERE twitch_user_id IS NOT NULL;
