CREATE TABLE patchnotes.guild_settings (
    guild_id BIGINT PRIMARY KEY CHECK (guild_id > 0),
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    channel_id BIGINT CHECK (channel_id IS NULL OR channel_id > 0),
    role_id BIGINT CHECK (role_id IS NULL OR role_id > 0),
    source_selection TEXT[] NOT NULL DEFAULT ARRAY['forum', 'steam']::TEXT[],
    section_selection TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    language TEXT NOT NULL DEFAULT 'de' CHECK (language IN ('de', 'en')),
    mention_strategy TEXT NOT NULL DEFAULT 'none'
        CHECK (mention_strategy IN ('none', 'role', 'everyone')),
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
        CHECK (
            (mention_strategy = 'role' AND role_id IS NOT NULL)
            OR (mention_strategy IN ('none', 'everyone') AND role_id IS NULL)
        )
);

CREATE TABLE patchnotes.guild_dispatch (
    guild_id BIGINT NOT NULL,
    patch_id BIGINT NOT NULL CHECK (patch_id > 0),
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$'),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'awaiting_approval', 'sending', 'delivery_unknown', 'sent', 'retry', 'failed', 'rejected', 'expired')),
    send_channel_id BIGINT CHECK (send_channel_id IS NULL OR send_channel_id > 0),
    sent_message_ids BIGINT[] NOT NULL DEFAULT ARRAY[]::BIGINT[],
    mention_strategy_snapshot TEXT
        CHECK (mention_strategy_snapshot IS NULL OR mention_strategy_snapshot IN ('none', 'role', 'everyone')),
    mention_role_id_snapshot BIGINT
        CHECK (mention_role_id_snapshot IS NULL OR mention_role_id_snapshot > 0),
    ping_message_id BIGINT CHECK (ping_message_id IS NULL OR ping_message_id > 0),
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
    CONSTRAINT patchnotes_guild_dispatch_mention_snapshot_check
        CHECK (
            (mention_strategy_snapshot IS NULL AND mention_role_id_snapshot IS NULL)
            OR (mention_strategy_snapshot = 'role' AND mention_role_id_snapshot IS NOT NULL)
            OR (mention_strategy_snapshot IN ('none', 'everyone') AND mention_role_id_snapshot IS NULL)
        ),
    CONSTRAINT patchnotes_guild_dispatch_ping_message_check
        CHECK (
            ping_message_id IS NULL
            OR (
                COALESCE(mention_strategy_snapshot IN ('role', 'everyone'), FALSE)
                AND ping_message_id = ANY(sent_message_ids)
            )
        ),
    CONSTRAINT patchnotes_guild_dispatch_sent_channel_check
        CHECK (
            (cardinality(sent_message_ids) = 0 OR send_channel_id IS NOT NULL)
            AND (status NOT IN ('sending', 'delivery_unknown', 'sent') OR send_channel_id IS NOT NULL)
        ),
    CONSTRAINT patchnotes_guild_dispatch_sent_status_check
        CHECK (
            status <> 'sent'
            OR (
                cardinality(sent_message_ids) > 0
                AND mention_strategy_snapshot IS NOT NULL
                AND (mention_strategy_snapshot = 'none' OR ping_message_id IS NOT NULL)
            )
        ),
    CONSTRAINT patchnotes_guild_dispatch_attempt_mention_snapshot_check
        CHECK (send_attempt_id IS NULL OR mention_strategy_snapshot IS NOT NULL),
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
            AND (recovery_outcome IS DISTINCT FROM 'partial' OR cardinality(sent_message_ids) > 0)
            AND (recovery_outcome IS DISTINCT FROM 'not_delivered' OR cardinality(sent_message_ids) = 0)
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
    configured_enabled BOOLEAN;
    configured_channel_id BIGINT;
    configured_mention_strategy TEXT;
    configured_role_id BIGINT;
BEGIN
    SELECT approval_mode, enabled, channel_id, mention_strategy, role_id
      INTO configured_approval_mode, configured_enabled, configured_channel_id,
           configured_mention_strategy, configured_role_id
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
           OR NEW.recovery_checked_at IS NOT NULL
           OR NEW.mention_strategy_snapshot IS NOT NULL
           OR NEW.mention_role_id_snapshot IS NOT NULL
           OR NEW.ping_message_id IS NOT NULL THEN
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

        IF NEW.status = 'sending'
           AND OLD.status IS DISTINCT FROM NEW.status
           AND (configured_enabled IS DISTINCT FROM TRUE
                OR configured_channel_id IS NULL
                OR NEW.send_channel_id IS DISTINCT FROM configured_channel_id) THEN
            RAISE EXCEPTION 'patchnotes send requires an enabled guild and its configured channel'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.status = 'sending' AND OLD.status IS DISTINCT FROM NEW.status THEN
            IF OLD.status = 'pending' THEN
                NEW.mention_strategy_snapshot := configured_mention_strategy;
                NEW.mention_role_id_snapshot := configured_role_id;
            ELSIF OLD.status = 'retry'
               AND OLD.mention_strategy_snapshot IS NOT NULL THEN
                NEW.mention_strategy_snapshot := OLD.mention_strategy_snapshot;
                NEW.mention_role_id_snapshot := OLD.mention_role_id_snapshot;
            ELSE
                RAISE EXCEPTION 'patchnotes send retry requires a frozen mention plan'
                    USING ERRCODE = '23514';
            END IF;
        ELSIF (NEW.mention_strategy_snapshot, NEW.mention_role_id_snapshot)
              IS DISTINCT FROM (OLD.mention_strategy_snapshot, OLD.mention_role_id_snapshot) THEN
            RAISE EXCEPTION 'patchnotes mention plan is immutable after the first send attempt'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.ping_message_id IS DISTINCT FROM OLD.ping_message_id
           AND (OLD.ping_message_id IS NOT NULL
                OR NEW.ping_message_id IS NULL
                OR OLD.status NOT IN ('sending', 'delivery_unknown')
                OR NEW.status NOT IN ('sending', 'delivery_unknown', 'sent', 'failed')
                OR NEW.mention_strategy_snapshot NOT IN ('role', 'everyone')
                OR NOT NEW.sent_message_ids @> ARRAY[NEW.ping_message_id]::BIGINT[]
                OR (
                    OLD.sent_message_ids @> ARRAY[NEW.ping_message_id]::BIGINT[]
                    AND NOT (
                        OLD.status = 'delivery_unknown'
                        AND NEW.status = 'sent'
                        AND NEW.recovery_outcome = 'delivered'
                        AND NEW.recovery_checked_at IS NOT NULL
                    )
                )) THEN
            RAISE EXCEPTION 'patchnotes ping evidence can only record one newly confirmed message'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.status = 'sending'
           AND NEW.status = 'sending'
           AND NEW.send_lease_expires_at IS DISTINCT FROM OLD.send_lease_expires_at THEN
            RAISE EXCEPTION 'active patchnotes send lease cannot be changed'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.status IN ('rejected', 'expired')
           AND OLD.status IS DISTINCT FROM NEW.status
           AND (OLD.status <> 'awaiting_approval'
                OR OLD.approved_at IS NOT NULL
                OR OLD.approved_by_user_id IS NOT NULL
                OR cardinality(OLD.sent_message_ids) > 0
                OR OLD.send_attempt_id IS NOT NULL
                OR OLD.send_started_at IS NOT NULL
                OR OLD.recovery_outcome IS NOT NULL) THEN
            RAISE EXCEPTION 'patchnotes approval request can only be rejected or expired before sending'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.sent_message_ids IS DISTINCT FROM OLD.sent_message_ids
           AND (OLD.status NOT IN ('sending', 'delivery_unknown')
                OR NEW.status NOT IN ('sending', 'delivery_unknown', 'sent', 'failed')
                OR OLD.recovery_outcome IS NOT NULL) THEN
            RAISE EXCEPTION 'patchnotes sent message evidence requires an active send attempt'
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

        IF OLD.status IN ('rejected', 'expired') AND NEW.status IS DISTINCT FROM OLD.status THEN
            RAISE EXCEPTION 'rejected or expired patchnotes request is terminal'
                USING ERRCODE = '23514';
        END IF;

        IF (OLD.status = 'failed' OR OLD.recovery_outcome = 'not_delivered')
           AND NEW.status IN ('pending', 'awaiting_approval') THEN
            RAISE EXCEPTION 'failed or reconciled patchnotes dispatch cannot return to an initial state'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.status = 'sent'
           AND OLD.status = 'sending'
           AND NEW.recovery_outcome IS NOT NULL THEN
            RAISE EXCEPTION 'live patchnotes send cannot carry a recovery outcome'
                USING ERRCODE = '23514';
        END IF;

        IF NEW.recovery_outcome = 'partial' AND NEW.status <> 'failed' THEN
            RAISE EXCEPTION 'partial patchnotes delivery must be recorded as terminal failure'
                USING ERRCODE = '23514';
        END IF;

        IF OLD.approved_at IS NULL THEN
            IF OLD.approved_by_user_id IS NOT NULL THEN
                RAISE EXCEPTION 'patchnotes approval identity requires an approval transition'
                    USING ERRCODE = '23514';
            END IF;

            IF OLD.status = 'awaiting_approval' AND NEW.status = 'pending' THEN
                IF configured_approval_mode = 'manual' THEN
                    IF NEW.approved_by_user_id IS NULL OR NEW.approved_at IS NOT NULL THEN
                        RAISE EXCEPTION 'patchnotes approval must be an explicit manual approval transition'
                            USING ERRCODE = '23514';
                    END IF;
                    NEW.approved_at := clock_timestamp();
                ELSIF NEW.approved_by_user_id IS NOT NULL OR NEW.approved_at IS NOT NULL THEN
                    RAISE EXCEPTION 'automatic patchnotes release cannot carry manual approval evidence'
                        USING ERRCODE = '23514';
                END IF;
            ELSIF NEW.approved_by_user_id IS NOT NULL OR NEW.approved_at IS NOT NULL THEN
                RAISE EXCEPTION 'patchnotes approval evidence requires an awaiting-approval transition'
                    USING ERRCODE = '23514';
            END IF;
        ELSE
            IF NEW.approved_at IS DISTINCT FROM OLD.approved_at THEN
                RAISE EXCEPTION 'patchnotes approval timestamp cannot be changed'
                    USING ERRCODE = '23514';
            END IF;

            IF NEW.approved_by_user_id IS DISTINCT FROM OLD.approved_by_user_id
               AND (OLD.approved_by_user_id IS NULL
                    OR NEW.approved_by_user_id IS NOT NULL
                    OR current_setting('patchnotes.privacy_erasure_user_id', true)
                       IS DISTINCT FROM OLD.approved_by_user_id::TEXT) THEN
                RAISE EXCEPTION 'patchnotes approval identity can only be anonymized by privacy erasure'
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

        IF OLD.status = 'sending' AND NEW.status IN ('retry', 'failed')
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

CREATE FUNCTION patchnotes.approve_dispatch(
    p_guild_id BIGINT,
    p_patch_id BIGINT,
    p_revision_hash TEXT,
    p_approver_user_id BIGINT
)
RETURNS BOOLEAN
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    changed_rows BIGINT;
BEGIN
    IF p_approver_user_id IS NULL OR p_approver_user_id <= 0 THEN
        RAISE EXCEPTION 'patchnotes approval requires a positive user id'
            USING ERRCODE = '23514';
    END IF;

    UPDATE patchnotes.guild_dispatch
       SET status = 'pending', approved_by_user_id = p_approver_user_id
     WHERE guild_id = p_guild_id
       AND patch_id = p_patch_id
       AND revision_hash = p_revision_hash
       AND status = 'awaiting_approval'
       AND approved_by_user_id IS NULL
       AND approved_at IS NULL;
    GET DIAGNOSTICS changed_rows = ROW_COUNT;
    RETURN changed_rows = 1;
END
$$;

CREATE FUNCTION patchnotes.anonymize_dispatch_approvals(p_user_id BIGINT)
RETURNS BIGINT
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    changed_rows BIGINT;
BEGIN
    IF p_user_id IS NULL OR p_user_id <= 0 THEN
        RAISE EXCEPTION 'patchnotes privacy erasure requires a positive user id'
            USING ERRCODE = '23514';
    END IF;

    PERFORM set_config('patchnotes.privacy_erasure_user_id', p_user_id::TEXT, true);
    UPDATE patchnotes.guild_dispatch
       SET approved_by_user_id = NULL
     WHERE approved_by_user_id = p_user_id;
    GET DIAGNOSTICS changed_rows = ROW_COUNT;
    PERFORM set_config('patchnotes.privacy_erasure_user_id', '', true);
    RETURN changed_rows;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'dl_patchnotes_dml') THEN
        BEGIN
            CREATE ROLE dl_patchnotes_dml NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN
            NULL;
        END;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'dl_patchnotes_privacy') THEN
        BEGIN
            CREATE ROLE dl_patchnotes_privacy NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN
            NULL;
        END;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA patchnotes TO dl_patchnotes_dml, dl_patchnotes_privacy;
GRANT dl_patchnotes_privacy TO deadlock;
REVOKE ALL ON TABLE patchnotes.guild_settings, patchnotes.guild_dispatch FROM PUBLIC;
REVOKE DELETE ON patchnotes.guild_dispatch FROM dl_patchnotes_dml;
REVOKE UPDATE ON patchnotes.guild_dispatch FROM dl_patchnotes_dml;
GRANT SELECT, INSERT, UPDATE, DELETE ON patchnotes.guild_settings TO dl_patchnotes_dml;
GRANT SELECT, INSERT ON patchnotes.guild_dispatch TO dl_patchnotes_dml;
GRANT UPDATE (
    status,
    send_channel_id,
    sent_message_ids,
    ping_message_id,
    send_attempt_id,
    send_started_at,
    send_lease_expires_at,
    recovery_outcome,
    recovery_checked_at,
    attempts,
    next_attempt_at,
    last_error,
    updated_at
) ON patchnotes.guild_dispatch TO dl_patchnotes_dml;
REVOKE EXECUTE ON FUNCTION patchnotes.validate_guild_dispatch_write() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION patchnotes.prevent_unapproved_manual_dispatch_mode() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION patchnotes.approve_dispatch(BIGINT, BIGINT, TEXT, BIGINT) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION patchnotes.anonymize_dispatch_approvals(BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION patchnotes.approve_dispatch(BIGINT, BIGINT, TEXT, BIGINT)
    TO dl_patchnotes_dml;
GRANT EXECUTE ON FUNCTION patchnotes.anonymize_dispatch_approvals(BIGINT)
    TO dl_patchnotes_privacy;
