-- Partner challenges reuse the central activity/identity sources through the
-- existing local Twitch database roles. No access to messages or credentials.
DO $roles$
DECLARE reader text;
BEGIN
    FOREACH reader IN ARRAY ARRAY['twitchbot', 'twitchdash'] LOOP
        IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = reader) THEN
            EXECUTE format('GRANT USAGE ON SCHEMA bot, core, voice, activity, steam TO %I', reader);
            EXECUTE format('GRANT SELECT ON bot.twitch_invite_joins,
                core.steam_links, voice.deadlock_party_members,
                activity.live_player_state, steam.steam_tasks,
                activity.voice_session_log,
                activity.guild_member_directory, activity.member_events TO %I', reader);
        END IF;
    END LOOP;
END
$roles$;
