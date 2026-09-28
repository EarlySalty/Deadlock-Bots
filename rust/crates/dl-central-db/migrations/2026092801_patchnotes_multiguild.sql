CREATE TABLE patchnotes.guild_settings (
    guild_id BIGINT PRIMARY KEY CHECK (guild_id > 0),
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    channel_id BIGINT CHECK (channel_id IS NULL OR channel_id > 0),
    role_id BIGINT CHECK (role_id IS NULL OR role_id > 0),
    source_selection TEXT[] NOT NULL DEFAULT ARRAY['forum', 'steam']::TEXT[],
    section_selection TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    language TEXT NOT NULL DEFAULT 'de' CHECK (language IN ('de', 'en')),
    mention_role BOOLEAN NOT NULL DEFAULT FALSE,
    approval_mode TEXT NOT NULL DEFAULT 'automatic' CHECK (approval_mode IN ('automatic', 'manual')),
    updated_by_user_id BIGINT CHECK (updated_by_user_id IS NULL OR updated_by_user_id > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT patchnotes_guild_settings_source_selection_check
        CHECK (
            array_position(source_selection, NULL) IS NULL
            AND source_selection <@ ARRAY['forum', 'steam']::TEXT[]
        ),
    CONSTRAINT patchnotes_guild_settings_section_selection_check
        CHECK (
            array_position(section_selection, NULL) IS NULL
            AND cardinality(section_selection) <= 100
            AND char_length(array_to_string(section_selection, '')) <= 4096
            AND array_to_string(section_selection, '') !~ '[[:cntrl:]]'
        ),
    CONSTRAINT patchnotes_guild_settings_enabled_channel_check
        CHECK (NOT enabled OR channel_id IS NOT NULL),
    CONSTRAINT patchnotes_guild_settings_role_mention_check
        CHECK (NOT mention_role OR role_id IS NOT NULL)
);

CREATE TABLE patchnotes.guild_dispatch (
    guild_id BIGINT NOT NULL,
    patch_id BIGINT NOT NULL CHECK (patch_id > 0),
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'awaiting_approval', 'sending', 'delivery_unknown', 'sent', 'retry', 'failed')),
    send_channel_id BIGINT CHECK (send_channel_id IS NULL OR send_channel_id > 0),
    sent_message_ids BIGINT[] NOT NULL DEFAULT ARRAY[]::BIGINT[],
    send_attempt_id UUID,
    send_started_at TIMESTAMPTZ,
    send_lease_expires_at TIMESTAMPTZ,
    recovery_outcome TEXT CHECK (recovery_outcome IN ('delivered', 'not_delivered', 'partial')),
    recovery_checked_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    approved_by_user_id BIGINT CHECK (approved_by_user_id IS NULL OR approved_by_user_id > 0),
    approved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, patch_id, revision_hash),
    CONSTRAINT patchnotes_guild_dispatch_guild_fk
        FOREIGN KEY (guild_id)
        REFERENCES patchnotes.guild_settings (guild_id)
        ON DELETE RESTRICT,
    CONSTRAINT patchnotes_guild_dispatch_patch_fk
        FOREIGN KEY (patch_id)
        REFERENCES patchnotes.changelog_posts (id)
        ON DELETE RESTRICT,
    CONSTRAINT patchnotes_guild_dispatch_message_ids_check
        CHECK (array_position(sent_message_ids, NULL) IS NULL AND 0 < ALL(sent_message_ids)),
    CONSTRAINT patchnotes_guild_dispatch_sent_channel_check
        CHECK (
            (cardinality(sent_message_ids) = 0 OR send_channel_id IS NOT NULL)
            AND (status NOT IN ('sending', 'delivery_unknown', 'sent') OR send_channel_id IS NOT NULL)
        ),
    CONSTRAINT patchnotes_guild_dispatch_sent_status_check
        CHECK (status <> 'sent' OR cardinality(sent_message_ids) > 0),
    CONSTRAINT patchnotes_guild_dispatch_sending_lease_check
        CHECK (
            (status = 'sending'
                AND send_attempt_id IS NOT NULL
                AND send_started_at IS NOT NULL
                AND send_lease_expires_at IS NOT NULL)
            OR (status <> 'sending' AND send_lease_expires_at IS NULL)
        ),
    CONSTRAINT patchnotes_guild_dispatch_unknown_delivery_check
        CHECK (
            status <> 'delivery_unknown'
            OR (send_attempt_id IS NOT NULL AND send_started_at IS NOT NULL AND send_channel_id IS NOT NULL)
        ),
    CONSTRAINT patchnotes_guild_dispatch_recovery_check
        CHECK ((recovery_outcome IS NULL) = (recovery_checked_at IS NULL)),
    CONSTRAINT patchnotes_guild_dispatch_approval_check
        CHECK (approved_by_user_id IS NULL OR approved_at IS NOT NULL)
);

CREATE INDEX patchnotes_guild_dispatch_retry_idx
    ON patchnotes.guild_dispatch (guild_id, next_attempt_at, updated_at)
    WHERE status IN ('pending', 'retry');

CREATE FUNCTION patchnotes.validate_guild_dispatch_write()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    configured_approval_mode TEXT;
BEGIN
    SELECT approval_mode
      INTO configured_approval_mode
      FROM patchnotes.guild_settings
     WHERE guild_id = NEW.guild_id
     FOR UPDATE;

    IF configured_approval_mode = 'manual'
       AND NEW.status IN ('sending', 'sent')
       AND (NEW.approved_by_user_id IS NULL OR NEW.approved_at IS NULL) THEN
        RAISE EXCEPTION 'manual patchnotes dispatch requires approval before sending'
            USING ERRCODE = '23514';
    END IF;

    IF TG_OP = 'INSERT' AND NEW.status = 'delivery_unknown' THEN
        RAISE EXCEPTION 'unknown delivery requires a prior sending attempt'
            USING ERRCODE = '23514';
    END IF;

    IF TG_OP = 'UPDATE' THEN
        IF OLD.send_channel_id IS NOT NULL
           AND NEW.send_channel_id IS DISTINCT FROM OLD.send_channel_id
           AND (cardinality(OLD.sent_message_ids) > 0
                OR OLD.status IN ('sending', 'delivery_unknown')
                OR OLD.recovery_outcome IS DISTINCT FROM 'not_delivered'
                OR OLD.recovery_checked_at IS NULL) THEN
            RAISE EXCEPTION 'patchnotes dispatch channel cannot change before delivery is reconciled'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.status IN ('sending', 'delivery_unknown')
           AND (NEW.send_attempt_id IS DISTINCT FROM OLD.send_attempt_id
                OR NEW.send_started_at IS DISTINCT FROM OLD.send_started_at
                OR NEW.send_channel_id IS DISTINCT FROM OLD.send_channel_id) THEN
            RAISE EXCEPTION 'patchnotes recovery must retain the original send attempt'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.status = 'sending' AND NEW.status = 'delivery_unknown'
           AND OLD.send_lease_expires_at > clock_timestamp() THEN
            RAISE EXCEPTION 'patchnotes send lease has not expired'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.status IN ('sending', 'delivery_unknown')
           AND NEW.status NOT IN ('sending', 'delivery_unknown', 'sent')
           AND (NEW.recovery_outcome IS NULL
                OR NEW.recovery_checked_at IS NULL
                OR NEW.recovery_outcome NOT IN ('not_delivered', 'partial')) THEN
            RAISE EXCEPTION 'patchnotes dispatch recovery must confirm no or partial delivery before retry'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.status = 'delivery_unknown' AND NEW.status = 'sent'
           AND (NEW.recovery_outcome IS NULL
                OR NEW.recovery_checked_at IS NULL
                OR NEW.recovery_outcome NOT IN ('delivered', 'partial')) THEN
            RAISE EXCEPTION 'unknown delivery must be reconciled before marking sent'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    RETURN NEW;
END
$$;

CREATE FUNCTION patchnotes.prevent_unapproved_manual_dispatch_mode()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    IF NEW.approval_mode = 'manual'
       AND EXISTS (
            SELECT 1
              FROM patchnotes.guild_dispatch
             WHERE guild_id = NEW.guild_id
               AND status = 'sending'
               AND (approved_by_user_id IS NULL OR approved_at IS NULL)
       ) THEN
        RAISE EXCEPTION 'manual approval mode conflicts with an unapproved active dispatch'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER patchnotes_guild_dispatch_validate_write_trg
    BEFORE INSERT OR UPDATE ON patchnotes.guild_dispatch
    FOR EACH ROW EXECUTE FUNCTION patchnotes.validate_guild_dispatch_write();

CREATE TRIGGER patchnotes_guild_settings_manual_approval_trg
    BEFORE UPDATE OF approval_mode ON patchnotes.guild_settings
    FOR EACH ROW EXECUTE FUNCTION patchnotes.prevent_unapproved_manual_dispatch_mode();

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'dl_patchnotes_dml') THEN
        BEGIN
            CREATE ROLE dl_patchnotes_dml NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN
            NULL;
        END;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA patchnotes TO dl_patchnotes_dml;
REVOKE ALL ON TABLE patchnotes.guild_settings, patchnotes.guild_dispatch FROM PUBLIC;
GRANT SELECT, INSERT, UPDATE, DELETE
    ON patchnotes.guild_settings, patchnotes.guild_dispatch TO dl_patchnotes_dml;
REVOKE EXECUTE ON FUNCTION patchnotes.validate_guild_dispatch_write() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION patchnotes.prevent_unapproved_manual_dispatch_mode() FROM PUBLIC;
