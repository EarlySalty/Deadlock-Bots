CREATE SCHEMA IF NOT EXISTS community;

CREATE TABLE IF NOT EXISTS community.team_applications (
    id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL CHECK (guild_id > 0),
    applicant_user_id BIGINT NOT NULL CHECK (applicant_user_id > 0),
    applicant_name TEXT NOT NULL CHECK (char_length(applicant_name) BETWEEN 1 AND 80),
    kind TEXT NOT NULL CHECK (kind IN ('moderation', 'coach', 'caster', 'turnier', 'coder', 'sonstiges')),
    answers JSONB NOT NULL CHECK (jsonb_typeof(answers) = 'object'),
    status TEXT NOT NULL DEFAULT 'publishing' CHECK (status IN ('publishing', 'open', 'review', 'question', 'accepted', 'rejected')),
    moderator_message_id BIGINT CHECK (moderator_message_id > 0),
    reviewer_user_id BIGINT CHECK (reviewer_user_id > 0),
    status_note TEXT,
    status_dm_sent_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS team_applications_one_active_per_kind
    ON community.team_applications (guild_id, applicant_user_id, kind)
    WHERE status IN ('publishing', 'open', 'review', 'question');

CREATE INDEX IF NOT EXISTS team_applications_status_created_idx
    ON community.team_applications (status, created_at DESC);

CREATE TABLE IF NOT EXISTS community.team_application_discord_erasure_queue (
    application_id BIGINT PRIMARY KEY,
    moderator_message_id BIGINT NOT NULL CHECK (moderator_message_id > 0),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO core.privacy_field_registry(
    schema_name,
    table_name,
    column_name,
    data_category,
    retention_action,
    erasure_action,
    owner_service,
    reason
)
VALUES
    ('community', 'team_applications', 'applicant_user_id', 'user_id', 'retain_operational', 'delete_row_on_user_delete', 'dl-community', 'Discord user id of the applicant; the application row is deleted on user erasure'),
    ('community', 'team_applications', 'applicant_name', 'display_name', 'expire_with_domain', 'redact_on_user_delete', 'dl-community', 'display name captured when the application was submitted'),
    ('community', 'team_applications', 'answers', 'json_payload', 'expire_with_domain', 'redact_on_user_delete', 'dl-community', 'application answers entered by the applicant'),
    ('community', 'team_applications', 'reviewer_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'dl-community', 'Discord user id of the reviewing moderator'),
    ('community', 'team_applications', 'status_note', 'free_text', 'expire_with_domain', 'redact_on_user_delete', 'dl-community', 'moderator note sent to the applicant')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE SET
    data_category = EXCLUDED.data_category,
    retention_action = EXCLUDED.retention_action,
    erasure_action = EXCLUDED.erasure_action,
    owner_service = EXCLUDED.owner_service,
    reason = EXCLUDED.reason;
