DO $grant$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'twitchbot') THEN
        GRANT USAGE ON SCHEMA core TO twitchbot;
        GRANT SELECT ON core.dm_optout TO twitchbot;
    END IF;
END
$grant$;
