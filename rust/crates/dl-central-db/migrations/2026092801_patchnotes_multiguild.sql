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
        CHECK (array_position(section_selection, NULL) IS NULL),
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
        CHECK (status IN ('pending', 'awaiting_approval', 'sending', 'sent', 'retry', 'failed')),
    sent_message_ids BIGINT[] NOT NULL DEFAULT ARRAY[]::BIGINT[],
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    approved_by_user_id BIGINT CHECK (approved_by_user_id IS NULL OR approved_by_user_id > 0),
    approved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, patch_id),
    CONSTRAINT patchnotes_guild_dispatch_guild_fk
        FOREIGN KEY (guild_id)
        REFERENCES patchnotes.guild_settings (guild_id)
        ON DELETE RESTRICT,
    CONSTRAINT patchnotes_guild_dispatch_patch_fk
        FOREIGN KEY (patch_id)
        REFERENCES patchnotes.changelog_posts (id)
        ON DELETE RESTRICT,
    CONSTRAINT patchnotes_guild_dispatch_sent_message_ids_check
        CHECK (array_position(sent_message_ids, NULL) IS NULL AND 0 < ALL(sent_message_ids)),
    CONSTRAINT patchnotes_guild_dispatch_sent_status_check
        CHECK (status <> 'sent' OR cardinality(sent_message_ids) > 0),
    CONSTRAINT patchnotes_guild_dispatch_approval_check
        CHECK ((approved_by_user_id IS NULL) = (approved_at IS NULL))
);

CREATE INDEX patchnotes_guild_dispatch_retry_idx
    ON patchnotes.guild_dispatch (guild_id, next_attempt_at, updated_at)
    WHERE status IN ('pending', 'retry');

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
