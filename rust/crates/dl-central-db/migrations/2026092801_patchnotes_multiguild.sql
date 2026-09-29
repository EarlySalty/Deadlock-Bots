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
        CHECK (
            ((recovery_outcome IS NULL) = (recovery_checked_at IS NULL))
            AND (recovery_outcome <> 'partial' OR cardinality(sent_message_ids) > 0)
        ),
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

    IF TG_OP = 'INSERT' THEN
        IF NEW.status NOT IN ('pending', 'awaiting_approval')
           OR NEW.approved_by_user_id IS NOT NULL
           OR NEW.approved_at IS NOT NULL
           OR cardinality(NEW.sent_message_ids) > 0
           OR NEW.send_attempt_id IS NOT NULL
           OR NEW.send_started_at IS NOT NULL
           OR NEW.send_lease_expires_at IS NOT NULL
           OR NEW.recovery_outcome IS NOT NULL
           OR NEW.recovery_checked_at IS NOT NULL THEN
            RAISE EXCEPTION 'patchnotes dispatch must start without send or approval evidence'
                USING ERRCODE = '23514';
        END IF;

        IF configured_approval_mode = 'manual' AND NEW.status <> 'awaiting_approval' THEN
            RAISE EXCEPTION 'manual patchnotes dispatch must await approval'
                USING ERRCODE = '23514';
        END IF;

        IF configured_approval_mode = 'automatic' AND NEW.status = 'awaiting_approval' THEN
            RAISE EXCEPTION 'automatic patchnotes dispatch cannot await manual approval'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    IF NEW.status = 'sending' AND NEW.recovery_outcome IS NOT NULL THEN
        RAISE EXCEPTION 'new patchnotes send attempt cannot reuse a prior recovery result'
            USING ERRCODE = '23514';
    END IF;

    IF configured_approval_mode = 'manual'
       AND NEW.status IN ('sending', 'sent', 'retry', 'failed')
       AND NEW.approved_at IS NULL THEN
        RAISE EXCEPTION 'manual patchnotes dispatch requires approval before sending'
            USING ERRCODE = '23514';
    END IF;

    IF TG_OP = 'UPDATE' THEN
        IF (NEW.guild_id, NEW.patch_id, NEW.revision_hash)
           IS DISTINCT FROM (OLD.guild_id, OLD.patch_id, OLD.revision_hash) THEN
            RAISE EXCEPTION 'patchnotes dispatch identity is immutable'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.status = 'sending'
           AND OLD.status IS DISTINCT FROM NEW.status
           AND OLD.status NOT IN ('pending', 'retry') THEN
            RAISE EXCEPTION 'patchnotes send attempt must start from pending or reconciled retry'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.status = 'delivery_unknown'
           AND OLD.status IS DISTINCT FROM NEW.status
           AND OLD.status <> 'sending' THEN
            RAISE EXCEPTION 'unknown delivery requires an existing send attempt'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.status = 'sent'
           AND OLD.status IS DISTINCT FROM NEW.status
           AND OLD.status NOT IN ('sending', 'delivery_unknown') THEN
            RAISE EXCEPTION 'patchnotes dispatch can be sent only from an active or reconciled attempt'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.status = 'retry'
           AND OLD.status IS DISTINCT FROM NEW.status
           AND OLD.status NOT IN ('sending', 'delivery_unknown') THEN
            RAISE EXCEPTION 'patchnotes retry requires a reconciled send attempt'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.approved_at IS NULL THEN
            IF OLD.approved_by_user_id IS NOT NULL THEN
                RAISE EXCEPTION 'patchnotes approval identity requires an approval transition'
                    USING ERRCODE = '23514';
            END IF;

            IF OLD.status = 'awaiting_approval' AND NEW.status = 'pending' THEN
                IF configured_approval_mode <> 'manual'
                   OR NEW.approved_by_user_id IS NULL
                   OR NEW.approved_at IS NOT NULL THEN
                    RAISE EXCEPTION 'patchnotes approval must be an explicit manual approval transition'
                        USING ERRCODE = '23514';
                END IF;
                NEW.approved_at := clock_timestamp();
            ELSIF NEW.approved_by_user_id IS NOT NULL OR NEW.approved_at IS NOT NULL THEN
                RAISE EXCEPTION 'patchnotes approval evidence requires an awaiting-approval transition'
                    USING ERRCODE = '23514';
            END IF;
        ELSE
            IF NEW.approved_at IS DISTINCT FROM OLD.approved_at
               OR (NEW.approved_by_user_id IS DISTINCT FROM OLD.approved_by_user_id
                   AND NOT (OLD.approved_by_user_id IS NOT NULL
                            AND NEW.approved_by_user_id IS NULL)) THEN
                RAISE EXCEPTION 'patchnotes approval evidence cannot be changed or restored'
                    USING ERRCODE = '23514';
            END IF;
        END IF;

        IF OLD.status = 'sent' OR OLD.recovery_outcome = 'partial' THEN
            IF (to_jsonb(NEW) - 'approved_by_user_id')
               IS DISTINCT FROM (to_jsonb(OLD) - 'approved_by_user_id') THEN
                RAISE EXCEPTION 'confirmed patchnotes delivery evidence is immutable'
                    USING ERRCODE = '23514';
            END IF;
        END IF;

        IF cardinality(OLD.sent_message_ids) > 0
           AND NOT NEW.sent_message_ids @> OLD.sent_message_ids THEN
            RAISE EXCEPTION 'patchnotes sent message evidence cannot be removed'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.send_channel_id IS NOT NULL
           AND NEW.send_channel_id IS DISTINCT FROM OLD.send_channel_id
           AND (cardinality(OLD.sent_message_ids) > 0
                OR OLD.status IN ('sending', 'delivery_unknown', 'sent')
                OR (OLD.send_attempt_id IS NOT NULL AND (
                    OLD.recovery_outcome IS DISTINCT FROM 'not_delivered'
                    OR OLD.recovery_checked_at IS NULL
                ))) THEN
            RAISE EXCEPTION 'patchnotes dispatch channel cannot change before delivery is reconciled'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.status = 'delivery_unknown' AND NEW.status = 'sending' THEN
            RAISE EXCEPTION 'unknown patchnotes delivery must be reconciled before another send attempt'
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
                OR NEW.recovery_checked_at < OLD.send_started_at
                OR NEW.recovery_checked_at > clock_timestamp()
                OR NEW.status NOT IN ('retry', 'failed')
                OR (NEW.recovery_outcome = 'partial' AND NEW.status <> 'failed')
                OR NEW.recovery_outcome = 'delivered') THEN
            RAISE EXCEPTION 'patchnotes retry requires a checked not-delivered result or terminal partial result'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.status = 'delivery_unknown' AND NEW.status = 'sent'
           AND (NEW.recovery_outcome IS DISTINCT FROM 'delivered'
                OR NEW.recovery_checked_at IS NULL
                OR NEW.recovery_checked_at < OLD.send_started_at
                OR NEW.recovery_checked_at > clock_timestamp()) THEN
            RAISE EXCEPTION 'unknown delivery must be reconciled as delivered before marking sent'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.recovery_outcome IS NOT NULL
           AND OLD.recovery_outcome IS NULL
           AND (OLD.status NOT IN ('sending', 'delivery_unknown')
                OR NEW.recovery_checked_at IS NULL
                OR NEW.recovery_checked_at < OLD.send_started_at
                OR NEW.recovery_checked_at > clock_timestamp()) THEN
            RAISE EXCEPTION 'patchnotes recovery result requires a current send attempt check'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.recovery_outcome IS DISTINCT FROM OLD.recovery_outcome
           AND OLD.recovery_outcome IS NOT NULL
           AND NOT (
                OLD.status = 'retry'
                AND NEW.status = 'sending'
                AND OLD.recovery_outcome = 'not_delivered'
                AND NEW.recovery_outcome IS NULL
                AND NEW.recovery_checked_at IS NULL
                AND NEW.send_attempt_id IS DISTINCT FROM OLD.send_attempt_id
                AND NEW.send_started_at IS DISTINCT FROM OLD.send_started_at
           ) THEN
            RAISE EXCEPTION 'patchnotes recovery result is immutable once recorded'
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
               AND status IN ('sending', 'delivery_unknown', 'retry')
               AND approved_at IS NULL
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
REVOKE DELETE ON patchnotes.guild_dispatch FROM dl_patchnotes_dml;
GRANT SELECT, INSERT, UPDATE, DELETE ON patchnotes.guild_settings TO dl_patchnotes_dml;
GRANT SELECT, INSERT, UPDATE ON patchnotes.guild_dispatch TO dl_patchnotes_dml;
REVOKE EXECUTE ON FUNCTION patchnotes.validate_guild_dispatch_write() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION patchnotes.prevent_unapproved_manual_dispatch_mode() FROM PUBLIC;
