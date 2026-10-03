-- Minimaler personenbezogener Erstmitgliedschaftsnachweis, ohne Aktivitätskopie.
CREATE TABLE activity.twitch_invite_member_privacy (
    guild_id BIGINT NOT NULL,
    subject_hash BYTEA NOT NULL CHECK (octet_length(subject_hash) = 32),
    prior_member BOOLEAN NOT NULL DEFAULT TRUE CHECK (prior_member),
    PRIMARY KEY (guild_id, subject_hash)
);

CREATE FUNCTION bot.invite_privacy_erasure_user() RETURNS BIGINT
LANGUAGE plpgsql STABLE SET search_path = pg_catalog AS $$
DECLARE
    raw TEXT := current_setting('community.invite_erasure_user_id', TRUE);
    uid BIGINT;
BEGIN
    IF raw IS NULL OR raw !~ '^[1-9][0-9]{0,18}$' THEN RETURN NULL; END IF;
    uid := raw::BIGINT;
    IF EXISTS (SELECT 1 FROM core.user_privacy WHERE user_id = uid AND opted_out) THEN
        RETURN uid;
    END IF;
    RETURN NULL;
END;
$$;

CREATE FUNCTION bot.invite_privacy_twitch_allowed(twitch_id TEXT, source_at TIMESTAMPTZ)
RETURNS BOOLEAN LANGUAGE sql STABLE SET search_path = pg_catalog AS $$
 SELECT NOT EXISTS (
     SELECT 1 FROM community_points.twitch_viewer_privacy_blocks b
      WHERE b.subject_hash = sha256(convert_to('community-points:twitch-viewer-privacy:v1:' || twitch_id, 'UTF8'))
 ) OR EXISTS (
     SELECT 1 FROM community_points.twitch_viewer_consents c
       JOIN core.discord_platform_connections l ON l.discord_id = c.discord_id
        AND l.platform = 'twitch' AND l.platform_user_id = twitch_id
      WHERE c.subject_hash = sha256(convert_to('community-points:twitch-viewer-privacy:v1:' || twitch_id, 'UTF8'))
        AND source_at >= c.activity_since
        AND NOT EXISTS (SELECT 1 FROM core.user_privacy p WHERE p.user_id = c.discord_id
                         AND (p.opted_out OR p.deleted_at IS NOT NULL))
 )
$$;

-- Keine Sperren in Rowtriggern: alle tatsächlichen Writer sperren vorher.
CREATE FUNCTION bot.validate_invite_privacy_write() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE
    floor TIMESTAMPTZ;
BEGIN
    IF TG_TABLE_NAME = 'twitch_personal_invites' THEN
        IF NOT bot.invite_privacy_twitch_allowed(NEW.streamer_twitch_user_id, NEW.created_at)
           OR NOT bot.invite_privacy_twitch_allowed(NEW.inviter_twitch_user_id, NEW.created_at) THEN
            RAISE EXCEPTION 'Twitch invite privacy blocks personal attribution' USING ERRCODE='23514';
        END IF;
    ELSIF TG_TABLE_NAME = 'twitch_streamer_invite_code_history' THEN
        IF NOT bot.invite_privacy_twitch_allowed(NEW.twitch_user_id, NEW.valid_from) THEN
            RETURN NULL;
        END IF;
    ELSE
        SELECT updated_at INTO floor FROM core.user_privacy
         WHERE user_id = NEW.user_id AND reason = 'user_opt_in';
        IF EXISTS (SELECT 1 FROM core.user_privacy WHERE user_id = NEW.user_id
                    AND (opted_out OR deleted_at IS NOT NULL)) THEN RETURN NULL; END IF;
        IF TG_TABLE_NAME = 'twitch_invite_members' THEN
            IF floor IS NOT NULL AND (
                 NEW.first_joined_at < floor OR NEW.current_joined_at < floor
                 OR NEW.left_at < floor OR NEW.voice_started_at < floor
                 OR NEW.voice_observed_at < floor OR NEW.voice_qualified_at < floor
            ) THEN RETURN NULL; END IF;
            IF EXISTS (SELECT 1 FROM activity.twitch_invite_member_privacy
                        WHERE guild_id = NEW.guild_id
                          AND subject_hash = sha256(convert_to('twitch-invites:discord-member-privacy:v1:' || NEW.user_id::TEXT, 'UTF8'))) THEN
                NEW.prior_member := TRUE;
                NEW.first_join_id := NULL;
                NEW.first_joined_at := NULL;
                NEW.voice_channel_id := NULL;
                NEW.voice_started_at := NULL;
                NEW.voice_observed_at := NULL;
                NEW.voice_qualified_at := NULL;
            END IF;
        ELSIF TG_TABLE_NAME = 'twitch_invite_messages' THEN
            IF floor IS NOT NULL AND NEW.occurred_at < floor THEN RETURN NULL; END IF;
        ELSIF TG_TABLE_NAME = 'twitch_invite_evidence_queue' THEN
            IF floor IS NOT NULL AND NEW.marked_at < floor THEN RETURN NULL; END IF;
        ELSIF TG_TABLE_NAME = 'twitch_invite_joins' THEN
            IF floor IS NOT NULL AND NEW.joined_at < floor THEN RETURN NULL; END IF;
            IF EXISTS (SELECT 1 FROM activity.twitch_invite_member_privacy
                        WHERE guild_id=NEW.guild_id
                          AND subject_hash=sha256(convert_to('twitch-invites:discord-member-privacy:v1:' || NEW.user_id::TEXT, 'UTF8'))) THEN
                RETURN NULL;
            END IF;
            IF NOT bot.invite_privacy_twitch_allowed(NEW.streamer_twitch_user_id,NEW.joined_at)
               OR NOT bot.invite_privacy_twitch_allowed(NEW.inviter_twitch_user_id,NEW.joined_at) THEN
                RETURN NULL;
            END IF;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER twitch_invite_member_privacy_write BEFORE INSERT OR UPDATE ON activity.twitch_invite_members
FOR EACH ROW EXECUTE FUNCTION bot.validate_invite_privacy_write();
CREATE TRIGGER twitch_invite_message_privacy_write BEFORE INSERT ON activity.twitch_invite_messages
FOR EACH ROW EXECUTE FUNCTION bot.validate_invite_privacy_write();
CREATE TRIGGER twitch_invite_queue_privacy_write BEFORE INSERT OR UPDATE ON activity.twitch_invite_evidence_queue
FOR EACH ROW EXECUTE FUNCTION bot.validate_invite_privacy_write();
CREATE TRIGGER twitch_invite_join_privacy_write BEFORE INSERT ON bot.twitch_invite_joins
FOR EACH ROW EXECUTE FUNCTION bot.validate_invite_privacy_write();
CREATE TRIGGER twitch_personal_invite_privacy_write BEFORE INSERT OR UPDATE ON bot.twitch_personal_invites
FOR EACH ROW EXECUTE FUNCTION bot.validate_invite_privacy_write();
CREATE TRIGGER twitch_history_privacy_write BEFORE INSERT OR UPDATE ON bot.twitch_streamer_invite_code_history
FOR EACH ROW EXECUTE FUNCTION bot.validate_invite_privacy_write();

INSERT INTO core.privacy_field_registry(schema_name,table_name,column_name,data_category,retention_action,erasure_action,owner_service,reason) VALUES
 ('activity','twitch_invite_member_privacy','subject_hash','pseudonym','retain_operational','retain_hash_only','dl-community','Personenbezogener Erstmitgliedschaftsschutz: Hash, Guild und prior_member, ohne Zeit-/Voice-/Invitekopien'),
 ('activity','twitch_invite_members','user_id','user_id','retain_operational','delete_row_on_user_delete','dl-community','Vollständige Mitglieds-/Voicekopie löschen; ausschließlich getrennten Hash-Minimalnachweis erhalten'),
 ('activity','twitch_invite_messages','user_id','user_id','retain_operational','delete_row_on_user_delete','dl-community','Personenbezogene Nachrichtennachweise löschen'),
 ('activity','twitch_invite_evidence_queue','user_id','user_id','retain_operational','delete_row_on_user_delete','dl-community','Personenbezogenen ausstehenden Auswertungsauftrag löschen'),
 ('bot','twitch_invite_joins','user_id','user_id','retain_operational','delete_row_on_user_delete','dl-community','Eigene Attribution einschließlich erreichbarer Übergänge und Ledgerreferenzen entkoppeln'),
 ('bot','twitch_invite_joins','inviter_twitch_user_id','user_id','retain_operational','redact_on_user_delete','dl-community','Eigene Twitch-Zuschauerzuordnung vor Linklöschung entfernen, fremde Mitgliedschaft erhalten'),
 ('bot','twitch_invite_joins','streamer_twitch_user_id','user_id','retain_operational','redact_on_user_delete','dl-community','Eigene Twitch-Kanalzuordnung vor Linklöschung entfernen; keine pauschale Streamerausnahme'),
 ('bot','twitch_personal_invites','inviter_twitch_user_id','user_id','retain_operational','delete_row_on_user_delete','dl-community','Eigene persönliche Invitekopie löschen'),
 ('bot','twitch_personal_invites','streamer_twitch_user_id','user_id','retain_operational','delete_row_on_user_delete','dl-community','Eigene Kanal-/Invitekopie löschen; fremde Zuordnungen erhalten'),
 ('bot','twitch_streamer_invite_code_history','twitch_user_id','user_id','retain_operational','delete_row_on_user_delete','dl-community','Eigene historische Kanalidentitätskopie löschen; alte Herkunft bleibt durch Hash-/Consentgrenze gesperrt'),
 ('patchnotes','guild_dispatch','approved_by_user_id','user_id','retain_audit','redact_on_user_delete','dl-community','Vorhandene Anonymisierungsfunktion verwenden; Zustellnachweise erhalten'),
 ('patchnotes','guild_settings','updated_by_user_id','user_id','retain_operational','redact_on_user_delete','dl-community','Nur eigene nullable Editor-ID nullen; Guildkonfiguration erhalten');

DO $roles$ BEGIN
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='deadlock') THEN
  GRANT SELECT,INSERT ON activity.twitch_invite_member_privacy TO deadlock;
 END IF;
END $roles$;

-- Normale Guardlogik unverändert; nur die geprüfte zielgebundene Erasure ergänzt.
CREATE OR REPLACE FUNCTION bot.guard_twitch_invite_join() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN

    IF bot.invite_privacy_erasure_user() IS NOT NULL THEN
        IF TG_OP = 'DELETE' AND OLD.user_id = bot.invite_privacy_erasure_user()
           AND EXISTS (SELECT 1 FROM activity.twitch_invite_member_privacy
                        WHERE guild_id=OLD.guild_id AND subject_hash=sha256(convert_to('twitch-invites:discord-member-privacy:v1:' || OLD.user_id::TEXT,'UTF8'))) THEN
            RETURN OLD;
        END IF;
        IF TG_OP = 'UPDATE' AND (
            OLD.streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')
            OR OLD.inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')
        ) AND (to_jsonb(NEW) - ARRAY['streamer_twitch_user_id','inviter_twitch_user_id','streamer_login','invite_code','reason'])
             IS NOT DISTINCT FROM (to_jsonb(OLD) - ARRAY['streamer_twitch_user_id','inviter_twitch_user_id','streamer_login','invite_code','reason'])
          AND (NEW.streamer_twitch_user_id IS NOT DISTINCT FROM OLD.streamer_twitch_user_id OR (
              NEW.streamer_twitch_user_id IS NULL AND OLD.streamer_twitch_user_id IN (
                  SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')))
          AND (NEW.inviter_twitch_user_id IS NOT DISTINCT FROM OLD.inviter_twitch_user_id OR (
              NEW.inviter_twitch_user_id IS NULL AND OLD.inviter_twitch_user_id IN (
                  SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')))
          AND (NEW.streamer_login = OLD.streamer_login OR (NEW.streamer_login = '' AND OLD.streamer_twitch_user_id IN (
              SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')))
          AND NEW.invite_code = '' AND NEW.reason IS NULL THEN
            RETURN NEW;
        END IF;
    END IF;
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

CREATE OR REPLACE FUNCTION bot.guard_twitch_invite_history() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND bot.invite_privacy_erasure_user() IS NOT NULL THEN
        IF TG_TABLE_NAME = 'twitch_invite_members' THEN
            IF OLD.user_id = bot.invite_privacy_erasure_user() AND EXISTS (
                SELECT 1 FROM activity.twitch_invite_member_privacy WHERE guild_id=OLD.guild_id
                 AND subject_hash=sha256(convert_to('twitch-invites:discord-member-privacy:v1:' || OLD.user_id::TEXT,'UTF8'))
            ) THEN RETURN OLD; END IF;
        ELSIF TG_TABLE_NAME = 'twitch_invite_transitions' THEN
            IF EXISTS (SELECT 1 FROM bot.twitch_invite_joins j WHERE j.join_id=OLD.join_id AND (
                j.user_id=bot.invite_privacy_erasure_user()
                OR j.streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')
                OR j.inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')
            )) THEN RETURN OLD; END IF;
        END IF;
    END IF;
    IF TG_OP='UPDATE' AND TG_TABLE_NAME='twitch_invite_transitions'
       AND bot.invite_privacy_erasure_user() IS NOT NULL AND NEW.reason IS NULL
       AND (to_jsonb(NEW)-'reason') IS NOT DISTINCT FROM (to_jsonb(OLD)-'reason')
       AND EXISTS (SELECT 1 FROM bot.twitch_invite_joins j WHERE j.join_id=OLD.join_id AND (
            j.streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')
            OR j.inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')
       )) THEN RETURN NEW; END IF;
    RAISE EXCEPTION 'Twitch invite history is append-only';
END;
$$;

-- Statementtrigger laufen vor allen Zeilenlocks, auch bei direkten Writeraufrufen.
-- Rustwriter erwerben denselben unveränderten Identitätskey vor Nutzerlocks.
CREATE FUNCTION core.lock_invite_privacy_statement() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(hashtext('core.discord_platform_connections'),hashtext('twitch_reassignment'));
    RETURN NULL;
END;
$$;
CREATE TRIGGER twitch_invite_members_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON activity.twitch_invite_members
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER twitch_invite_messages_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON activity.twitch_invite_messages
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER twitch_invite_evidence_queue_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON activity.twitch_invite_evidence_queue
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER voice_session_log_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON activity.voice_session_log
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER twitch_invite_joins_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON bot.twitch_invite_joins
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER twitch_personal_invites_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON bot.twitch_personal_invites
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER twitch_streamer_invite_code_history_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON bot.twitch_streamer_invite_code_history
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER twitch_invite_transitions_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON bot.twitch_invite_transitions
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();

CREATE TRIGGER twitch_streamer_invites_privacy_statement BEFORE INSERT OR UPDATE OR DELETE ON bot.twitch_streamer_invites
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();

-- Zentraler Actoranschluss; vorhandene Patchnotesfunktionen bleiben unverändert.
CREATE TRIGGER community_patchnotes_settings_identity
BEFORE INSERT OR UPDATE OR DELETE ON patchnotes.guild_settings
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER community_patchnotes_dispatch_identity
BEFORE INSERT OR UPDATE OR DELETE ON patchnotes.guild_dispatch
FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();

CREATE FUNCTION core.scrub_opted_out_patchnotes_actor() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE uid BIGINT;
BEGIN
    IF TG_TABLE_NAME='guild_dispatch' THEN uid:=NEW.approved_by_user_id;
    ELSE uid:=NEW.updated_by_user_id; END IF;
    IF uid IS NOT NULL AND EXISTS (
        SELECT 1 FROM core.user_privacy WHERE user_id=uid AND (opted_out OR deleted_at IS NOT NULL)
    ) THEN
        IF TG_TABLE_NAME='guild_dispatch' THEN
            PERFORM patchnotes.anonymize_dispatch_approvals(uid);
        ELSE
            UPDATE patchnotes.guild_settings SET updated_by_user_id=NULL
            WHERE guild_id=NEW.guild_id AND updated_by_user_id=uid;
        END IF;
    END IF;
    RETURN NULL;
END;
$$;
REVOKE ALL ON FUNCTION core.scrub_opted_out_patchnotes_actor() FROM PUBLIC;
CREATE TRIGGER community_patchnotes_settings_actor
AFTER INSERT OR UPDATE ON patchnotes.guild_settings
FOR EACH ROW EXECUTE FUNCTION core.scrub_opted_out_patchnotes_actor();
CREATE TRIGGER community_patchnotes_dispatch_actor
AFTER INSERT OR UPDATE ON patchnotes.guild_dispatch
FOR EACH ROW EXECUTE FUNCTION core.scrub_opted_out_patchnotes_actor();

-- Identitätsscrubs erzeugen keinen zweiten Übergang desselben fremden Status.
CREATE OR REPLACE FUNCTION bot.audit_twitch_invite_join() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='UPDATE' AND bot.invite_privacy_erasure_user() IS NOT NULL
       AND (to_jsonb(NEW)-ARRAY['streamer_twitch_user_id','inviter_twitch_user_id','streamer_login','invite_code','reason'])
           IS NOT DISTINCT FROM (to_jsonb(OLD)-ARRAY['streamer_twitch_user_id','inviter_twitch_user_id','streamer_login','invite_code','reason'])
       AND (OLD.streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')
            OR OLD.inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=bot.invite_privacy_erasure_user() AND platform='twitch')) THEN
        RETURN NEW;
    END IF;
    INSERT INTO bot.twitch_invite_transitions (join_id, status, occurred_at, qualified_at, reason)
    VALUES (NEW.join_id, NEW.status, NEW.updated_at, NEW.qualified_at, NEW.reason);
    RETURN NEW;
END;
$$;
