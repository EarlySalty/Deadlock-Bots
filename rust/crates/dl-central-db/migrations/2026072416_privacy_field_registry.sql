SET lock_timeout TO '2s';

CREATE OR REPLACE FUNCTION scrim.enforce_ai_run_decision_ref()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    ref_id BIGINT;
BEGIN
    IF NEW.decision_ref_kind IS NULL AND NEW.decision_ref_id IS NULL THEN
        RETURN NEW;
    END IF;

    IF NEW.decision_ref_kind IS NULL OR NEW.decision_ref_id IS NULL THEN
        RAISE EXCEPTION 'scrim ai run decision ref kind and id must be set together'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.decision_ref_id !~ '^[1-9][0-9]{0,17}$' THEN
        RAISE EXCEPTION 'scrim ai run decision ref id must be a positive numeric database id'
            USING ERRCODE = '23514';
    END IF;

    ref_id := NEW.decision_ref_id::BIGINT;

    CASE NEW.decision_ref_kind
        WHEN 'runtime_control_history' THEN
            PERFORM 1 FROM scrim.runtime_control_history WHERE id = ref_id;
        WHEN 'announcement_approval' THEN
            PERFORM 1 FROM scrim.announcement_approvals WHERE id = ref_id;
        WHEN 'status_publication_approval' THEN
            PERFORM 1 FROM scrim.status_publication_approvals WHERE id = ref_id;
        WHEN 'match_lineup_snapshot' THEN
            PERFORM 1 FROM scrim.match_lineup_snapshots WHERE id = ref_id;
        WHEN 'match_result_ref' THEN
            PERFORM 1 FROM scrim.match_result_refs WHERE id = ref_id;
        WHEN 'match_result_clarification' THEN
            PERFORM 1 FROM scrim.match_result_clarifications WHERE id = ref_id;
        ELSE
            RAISE EXCEPTION 'unsupported scrim ai run decision ref kind %', NEW.decision_ref_kind
                USING ERRCODE = '23514';
    END CASE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'scrim ai run decision ref %.% does not exist', NEW.decision_ref_kind, NEW.decision_ref_id
            USING ERRCODE = '23503';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS enforce_ai_run_decision_ref ON scrim.ai_runs;
CREATE TRIGGER enforce_ai_run_decision_ref
    BEFORE INSERT OR UPDATE OF decision_ref_kind, decision_ref_id ON scrim.ai_runs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.enforce_ai_run_decision_ref();

CREATE TABLE IF NOT EXISTS core.privacy_field_registry (
    schema_name TEXT NOT NULL,
    table_name TEXT NOT NULL,
    column_name TEXT NOT NULL,
    data_category TEXT NOT NULL CHECK (data_category IN ('user_id', 'display_name', 'json_payload', 'free_text', 'domain_id', 'machine_code', 'content_hash', 'pseudonym')),
    retention_action TEXT NOT NULL CHECK (retention_action IN ('retain_operational', 'retain_audit', 'expire_with_domain', 'manual_review')),
    erasure_action TEXT NOT NULL CHECK (erasure_action IN ('redact_on_user_delete', 'delete_row_on_user_delete', 'retain_hash_only', 'retain_non_personal', 'manual_review')),
    owner_service TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (schema_name, table_name, column_name)
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
    ('scrim', 'runtime_control', 'actor_pseudonym', 'pseudonym', 'retain_audit', 'retain_hash_only', 'turniere-cutover', 'runtime ownership actor pseudonym'),
    ('scrim', 'runtime_control', 'decision_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere-cutover', 'immutable cutover decision context; writer strips raw actor fields and remaining free-form context requires human review before disclosure'),
    ('scrim', 'runtime_control_history', 'actor_pseudonym', 'pseudonym', 'retain_audit', 'retain_hash_only', 'turniere-cutover', 'runtime ownership actor pseudonym'),
    ('scrim', 'runtime_control_history', 'actor_source', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere-cutover', 'explicit actor source enum, not a raw actor id'),
    ('scrim', 'runtime_control_history', 'request_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'turniere-cutover', 'request correlation id; callers must not put user-entered text here'),
    ('scrim', 'runtime_control_history', 'correlation_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'turniere-cutover', 'correlation id; callers must not put user-entered text here'),
    ('scrim', 'runtime_control_history', 'decision_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere-cutover', 'immutable cutover decision context; writer strips raw actor fields and remaining free-form context requires human review before disclosure'),
    ('scrim', 'command_receipts', 'command_scope', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'bounded command namespace, not user-entered free text'),
    ('scrim', 'command_receipts', 'idempotency_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'idempotency reference; callers must not put user-entered text here'),
    ('scrim', 'command_receipts', 'payload_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'turniere', 'payload digest only'),
    ('scrim', 'command_receipts', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'idempotent command payload with automated nested user-id projection/redaction'),
    ('scrim', 'command_receipts', 'lease_owner', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'worker lease id, not user-entered free text'),
    ('scrim', 'command_receipts', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'remote system code'),
    ('scrim', 'command_receipts', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'remote message id; direct user ids are not stored here'),
    ('scrim', 'command_receipts', 'remote_task_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'remote task id; direct user ids are not stored here'),
    ('scrim', 'command_receipts', 'result_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'idempotent command result with automated nested user-id projection/redaction'),
    ('scrim', 'command_receipts', 'last_error_code', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'strict machine error code; raw error text is not stored'),
    ('scrim', 'command_receipts', 'last_error_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'turniere', 'raw error digest only'),
    ('scrim', 'inbox_events', 'event_source', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'bounded event source code'),
    ('scrim', 'inbox_events', 'source_event_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'source event id; direct user ids are not stored here'),
    ('scrim', 'inbox_events', 'idempotency_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'idempotency reference; callers must not put user-entered text here'),
    ('scrim', 'inbox_events', 'payload_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'turniere', 'payload digest only'),
    ('scrim', 'inbox_events', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'external event payload with automated nested user-id projection/redaction'),
    ('scrim', 'inbox_events', 'lease_owner', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'worker lease id, not user-entered free text'),
    ('scrim', 'inbox_events', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'remote system code'),
    ('scrim', 'inbox_events', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'remote message id; direct user ids are not stored here'),
    ('scrim', 'inbox_events', 'remote_task_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'remote task id; direct user ids are not stored here'),
    ('scrim', 'inbox_events', 'last_error_code', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'strict machine error code; raw error text is not stored'),
    ('scrim', 'inbox_events', 'last_error_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'turniere', 'raw error digest only'),
    ('scrim', 'outbox_effects', 'effect_type', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'bounded outbound effect type'),
    ('scrim', 'outbox_effects', 'idempotency_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'idempotency reference; callers must not put user-entered text here'),
    ('scrim', 'outbox_effects', 'payload_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'dl-bots-discord-adapter', 'payload digest only'),
    ('scrim', 'outbox_effects', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'dl-bots-discord-adapter', 'outbound effect payload with automated nested user-id projection/redaction'),
    ('scrim', 'outbox_effects', 'lease_owner', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'worker lease id, not user-entered free text'),
    ('scrim', 'outbox_effects', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote system code'),
    ('scrim', 'outbox_effects', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote message id; direct user ids are not stored here'),
    ('scrim', 'outbox_effects', 'remote_task_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote task id; direct user ids are not stored here'),
    ('scrim', 'outbox_effects', 'last_error_code', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'strict machine error code; raw error text is not stored'),
    ('scrim', 'outbox_effects', 'last_error_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'dl-bots-discord-adapter', 'raw error digest only'),
    ('scrim', 'effect_receipts', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote system code'),
    ('scrim', 'effect_receipts', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote message id; direct user ids are not stored here'),
    ('scrim', 'effect_receipts', 'remote_task_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote task id; direct user ids are not stored here'),
    ('scrim', 'effect_receipts', 'payload_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'dl-bots-discord-adapter', 'payload digest only'),
    ('scrim', 'effect_receipts', 'receipt_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'dl-bots-discord-adapter', 'remote delivery receipt with automated nested user-id projection/redaction'),
    ('scrim', 'audit_actor_pseudonyms', 'actor_ref', 'user_id', 'retain_audit', 'delete_row_on_user_delete', 'turniere', 'private raw audit actor mapping; user rows are positive Discord IDs, system/service rows are bounded machine/domain refs'),
    ('scrim', 'audit_events', 'event_type', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'audit event type code'),
    ('scrim', 'audit_events', 'entity_type', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'checked audit entity kind'),
    ('scrim', 'audit_events', 'entity_id', 'domain_id', 'retain_audit', 'retain_hash_only', 'turniere', 'entity id is constrained by entity_type; personal entity kinds require an opaque act_ pseudonym, never raw Discord or Steam ids'),
    ('scrim', 'audit_events', 'actor_pseudonym', 'pseudonym', 'retain_audit', 'retain_hash_only', 'turniere', 'opaque audit actor pseudonym'),
    ('scrim', 'audit_events', 'actor_source', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'explicit actor source enum, not a raw actor id'),
    ('scrim', 'audit_events', 'request_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'turniere', 'request correlation id; callers must not put user-entered text here'),
    ('scrim', 'audit_events', 'correlation_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'turniere', 'correlation id; callers must not put user-entered text here'),
    ('scrim', 'audit_events', 'before_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'immutable audit before payload; audit writers strip raw actor fields and disclosure requires human review'),
    ('scrim', 'audit_events', 'after_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'immutable audit after payload; audit writers strip raw actor fields and disclosure requires human review'),
    ('scrim', 'audit_events', 'decision_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'immutable audit decision payload; audit writers strip raw actor fields and disclosure requires human review'),
    ('scrim', 'audit_events', 'metadata', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'immutable audit metadata; audit writers strip raw actor fields and disclosure requires human review'),
    ('scrim', 'announcement_drafts', 'block_key', 'domain_id', 'expire_with_domain', 'retain_non_personal', 'dl-bots-discord-adapter', 'announcement block id, not user-entered free text'),
    ('scrim', 'announcement_drafts', 'title', 'free_text', 'expire_with_domain', 'manual_review', 'dl-bots-discord-adapter', 'announcement title may contain user-entered text'),
    ('scrim', 'announcement_drafts', 'body', 'free_text', 'expire_with_domain', 'manual_review', 'dl-bots-discord-adapter', 'announcement body may contain user-entered text'),
    ('scrim', 'announcement_drafts', 'payload', 'json_payload', 'expire_with_domain', 'manual_review', 'dl-bots-discord-adapter', 'announcement structured payload'),
    ('scrim', 'announcement_drafts', 'payload_hash', 'content_hash', 'expire_with_domain', 'retain_hash_only', 'dl-bots-discord-adapter', 'announcement content digest only'),
    ('scrim', 'announcement_drafts', 'idempotency_key', 'domain_id', 'expire_with_domain', 'retain_non_personal', 'dl-bots-discord-adapter', 'idempotency reference; callers must not put user-entered text here'),
    ('scrim', 'announcement_drafts', 'created_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'draft creator id'),
    ('scrim', 'announcement_drafts', 'created_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'draft creator label'),
    ('scrim', 'announcement_drafts', 'approved_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'draft approver id'),
    ('scrim', 'announcement_drafts', 'approved_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'draft approver label'),
    ('scrim', 'announcement_drafts', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote system code'),
    ('scrim', 'announcement_drafts', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'remote message id; direct user ids are not stored here'),
    ('scrim', 'announcement_approvals', 'decided_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'approval actor id'),
    ('scrim', 'announcement_approvals', 'decided_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'approval actor label'),
    ('scrim', 'announcement_approvals', 'decision_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'approval decision payload; actor ids are stored in typed columns and disclosure requires human review'),
    ('scrim', 'status_publication_approvals', 'target_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'turniere', 'target_kind constrains this to a Scrim domain entity, never a user target'),
    ('scrim', 'status_publication_approvals', 'status_kind', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'status publication kind code'),
    ('scrim', 'status_publication_approvals', 'payload', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'status publication payload; actor ids are stored in typed columns and disclosure requires human review'),
    ('scrim', 'status_publication_approvals', 'payload_hash', 'content_hash', 'retain_audit', 'retain_hash_only', 'turniere', 'status publication content digest only'),
    ('scrim', 'status_publication_approvals', 'decided_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'status decision actor id'),
    ('scrim', 'status_publication_approvals', 'decided_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'status decision actor label'),
    ('scrim', 'status_publication_approvals', 'decision_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'status decision payload; actor ids are stored in typed columns and disclosure requires human review'),
    ('scrim', 'status_publication_approvals', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'remote system code'),
    ('scrim', 'status_publication_approvals', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'remote message id; direct user ids are not stored here'),
    ('scrim', 'replacement_needs', 'needed_role', 'machine_code', 'expire_with_domain', 'retain_non_personal', 'turniere', 'replacement role code'),
    ('scrim', 'replacement_needs', 'reason', 'machine_code', 'expire_with_domain', 'retain_non_personal', 'turniere', 'bounded replacement reason code; human details belong in managed payloads'),
    ('scrim', 'replacement_needs', 'request_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'request correlation id; callers must not put user-entered text here'),
    ('scrim', 'replacement_needs', 'correlation_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'correlation id; callers must not put user-entered text here'),
    ('scrim', 'replacement_needs', 'participant_id', 'user_id', 'expire_with_domain', 'delete_row_on_user_delete', 'turniere', 'replacement target participant'),
    ('scrim', 'replacement_needs', 'created_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'replacement need creator id'),
    ('scrim', 'replacement_needs', 'created_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'replacement need creator label'),
    ('scrim', 'replacement_candidates', 'participant_id', 'user_id', 'expire_with_domain', 'delete_row_on_user_delete', 'turniere', 'candidate participant'),
    ('scrim', 'replacement_candidates', 'discord_user_id', 'user_id', 'expire_with_domain', 'delete_row_on_user_delete', 'turniere', 'candidate discord id'),
    ('scrim', 'replacement_candidates', 'candidate_data', 'json_payload', 'expire_with_domain', 'redact_on_user_delete', 'turniere', 'candidate payload with automated nested Discord and linked-Steam erasure; DSAR HashOnly mode exposes only row id and payload digest'),
    ('scrim', 'replacement_candidates', 'score_data', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'candidate scoring payload with automated nested Discord and linked-Steam erasure; DSAR HashOnly mode exposes only row id and payload digest'),
    ('scrim', 'replacement_requests', 'participant_id', 'user_id', 'expire_with_domain', 'delete_row_on_user_delete', 'turniere', 'requested participant'),
    ('scrim', 'replacement_requests', 'discord_user_id', 'user_id', 'expire_with_domain', 'delete_row_on_user_delete', 'turniere', 'requested discord id'),
    ('scrim', 'replacement_requests', 'request_payload', 'json_payload', 'expire_with_domain', 'redact_on_user_delete', 'turniere', 'replacement request payload with automated nested Discord and linked-Steam erasure; requester-only erasure scrubs payload even when the row survives'),
    ('scrim', 'replacement_requests', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'remote system code'),
    ('scrim', 'replacement_requests', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'remote message id; direct user ids are not stored here'),
    ('scrim', 'replacement_requests', 'requested_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'replacement requester id'),
    ('scrim', 'replacement_requests', 'requested_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'replacement requester label'),
    ('scrim', 'matches', 'join_code', 'free_text', 'retain_operational', 'manual_review', 'turniere', 'legacy lobby secret; safe actor DSAR omits it and disclosure requires manual handling outside the projection'),
    ('scrim', 'matches', 'lobby_code_source_user_id', 'user_id', 'retain_operational', 'redact_on_user_delete', 'turniere', 'lobby-code source actor id with safe actor DSAR projection'),
    ('scrim', 'matches', 'lobby_code_source_display_name', 'display_name', 'retain_operational', 'redact_on_user_delete', 'turniere', 'lobby-code source actor label redacted together with actor id'),
    ('scrim', 'matches', 'lobby_code_message_ids', 'json_payload', 'retain_operational', 'retain_non_personal', 'turniere', 'Discord message-id map; non-personal but actor DSAR exposes only a hash to avoid adjacent context leakage'),
    ('scrim', 'matches', 'lobby_code_corrections', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'lobby-code correction history with automated nested actor-id/display-name erasure; DSAR HashOnly mode exposes only row id and payload digest so lobby codes and notes stay hidden'),
    ('scrim', 'matches', 'result_json', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'legacy match result payload with automated nested Discord, Steam64 and Steam account-id erasure; DSAR projection keeps target result data and redacts foreign players'),
    ('scrim', 'match_requests', 'slot_options', 'json_payload', 'retain_operational', 'manual_review', 'turniere', 'legacy slot option payload; actor DSAR exposes only a hash because payload may contain free-form scheduling context'),
    ('scrim', 'match_requests', 'team_query_message_ids', 'json_payload', 'retain_operational', 'retain_non_personal', 'turniere', 'Discord query message-id map; non-personal but actor DSAR exposes only a hash'),
    ('scrim', 'match_requests', 'released_slot', 'json_payload', 'retain_operational', 'manual_review', 'turniere', 'released slot payload; actor DSAR exposes only a hash because payload may include foreign/free-form context'),
    ('scrim', 'match_requests', 'released_by_user_id', 'user_id', 'retain_operational', 'redact_on_user_delete', 'turniere', 'match request release actor id with safe actor DSAR projection'),
    ('scrim', 'match_requests', 'released_by_display_name', 'display_name', 'retain_operational', 'redact_on_user_delete', 'turniere', 'match request release actor label redacted together with actor id'),
    ('scrim', 'match_requests', 'override_reason', 'free_text', 'retain_operational', 'redact_on_user_delete', 'turniere', 'legacy release override free text; erasure scrubs exact Discord, Steam64 and account-id target refs while writers remain compatible'),
    ('scrim', 'match_requests', 'team_status_message_ids', 'json_payload', 'retain_operational', 'retain_non_personal', 'turniere', 'Discord status message-id map; non-personal but actor DSAR exposes only a hash'),
    ('scrim', 'match_requests', 'status_message_last_error', 'free_text', 'retain_operational', 'redact_on_user_delete', 'turniere', 'legacy status-message error text; erasure scrubs exact Discord, Steam64 and account-id target refs while writers remain compatible'),
    ('scrim', 'match_lineup_snapshots', 'lineup_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'lineup snapshot payload with subject-only DSAR projection and nested user-id redaction'),
    ('scrim', 'match_lineup_snapshots', 'request_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'request correlation id; callers must not put user-entered text here'),
    ('scrim', 'match_lineup_snapshots', 'correlation_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'correlation id; callers must not put user-entered text here'),
    ('scrim', 'match_lineup_snapshots', 'created_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'lineup snapshot actor id'),
    ('scrim', 'match_lineup_snapshots', 'created_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'lineup snapshot actor label'),
    ('scrim', 'ai_runs', 'run_kind', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'AI run kind code'),
    ('scrim', 'ai_runs', 'subject_id', 'free_text', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'AI run subject id; personal only when subject_kind is discord_user or steam_user and handled by kind-aware erasure'),
    ('scrim', 'ai_runs', 'idempotency_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'idempotency reference; callers must not put user-entered text here'),
    ('scrim', 'ai_runs', 'input_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'dl-bots-ai', 'AI input digest only'),
    ('scrim', 'ai_runs', 'provider', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'provider code'),
    ('scrim', 'ai_runs', 'model', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'model code'),
    ('scrim', 'ai_runs', 'lease_owner', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'worker lease id, not user-entered free text'),
    ('scrim', 'ai_runs', 'decision_ref_kind', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'explicit allowlist of non-personal Scrim foundation decision-domain records'),
    ('scrim', 'ai_runs', 'decision_ref_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'positive numeric id whose existence is verified against the allowlisted decision-domain table; raw user refs and unknown kinds are rejected'),
    ('scrim', 'ai_runs', 'request_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'request correlation id; callers must not put user-entered text here'),
    ('scrim', 'ai_runs', 'correlation_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'correlation id; callers must not put user-entered text here'),
    ('scrim', 'ai_runs', 'last_error_code', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'strict machine error code; raw error text is not stored'),
    ('scrim', 'ai_runs', 'last_error_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'dl-bots-ai', 'raw error digest only'),
    ('scrim', 'ai_decision_refs', 'decision_kind', 'machine_code', 'retain_audit', 'retain_non_personal', 'dl-bots-ai', 'AI decision kind code'),
    ('scrim', 'ai_decision_refs', 'target_kind', 'machine_code', 'retain_audit', 'retain_non_personal', 'dl-bots-ai', 'checked AI target kind; personal target kinds are not allowed'),
    ('scrim', 'ai_decision_refs', 'target_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'dl-bots-ai', 'checked numeric domain target id; personal target kinds are structurally unavailable'),
    ('scrim', 'ai_decision_refs', 'decision_data', 'json_payload', 'retain_audit', 'manual_review', 'dl-bots-ai', 'immutable AI decision payload; target ids are separated into checked domain fields and disclosure requires human review'),
    ('scrim', 'match_result_refs', 'external_result_ref', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'external result reference; direct user ids are not stored here'),
    ('scrim', 'match_result_refs', 'source_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'result ref source actor id'),
    ('scrim', 'match_result_refs', 'source_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'result ref source actor label'),
    ('scrim', 'match_result_refs', 'last_error', 'free_text', 'retain_operational', 'redact_on_user_delete', 'turniere', 'legacy raw result-fetch error text; DSAR hashes/redacts it and erasure scrubs exact Discord, Steam64 and Steam account-id target refs while writers migrate to strict error codes'),
    ('scrim', 'match_result_refs', 'clarification_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'result clarification payload with automated nested erasure; DSAR HashOnly mode exposes only row id and payload digest'),
    ('scrim', 'match_result_refs', 'raw_result_json', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'raw result payload with automated nested Discord, Steam64 and Steam account-id erasure; generic DSAR projection keeps target result data and redacts foreign players'),
    ('scrim', 'match_result_refs', 'normalized_result_json', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'normalized result payload backing selected view with automated nested Discord, Steam64 and Steam account-id erasure'),
    ('scrim', 'match_result_refs', 'selected_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'selected result actor id'),
    ('scrim', 'match_result_refs', 'selection_reason', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'bounded selection reason code; human details belong in clarification payloads'),
    ('scrim', 'match_result_refs', 'void_reason', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'bounded void reason code; human details belong in clarification payloads'),
    ('scrim', 'match_result_selections', 'selected_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'canonical result selection actor id'),
    ('scrim', 'match_result_selections', 'selected_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'canonical result selection actor label'),
    ('scrim', 'match_result_selections', 'selection_reason', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'bounded canonical selection reason code; human details belong in clarification payloads'),
    ('scrim', 'match_result_selection_events', 'actor_pseudonym', 'pseudonym', 'retain_audit', 'retain_hash_only', 'turniere', 'selection history actor pseudonym'),
    ('scrim', 'match_result_selection_events', 'old_result_ref_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'turniere', 'previous canonical result ref id for immutable reselection history'),
    ('scrim', 'match_result_selection_events', 'new_result_ref_id', 'domain_id', 'retain_audit', 'retain_non_personal', 'turniere', 'new canonical result ref id for immutable selection history'),
    ('scrim', 'match_result_selection_events', 'actor_source', 'machine_code', 'retain_audit', 'retain_non_personal', 'turniere', 'explicit actor source enum, not a raw actor id'),
    ('scrim', 'match_result_selection_events', 'before_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'immutable selection history before payload; trigger strips raw actor fields'),
    ('scrim', 'match_result_selection_events', 'after_data', 'json_payload', 'retain_audit', 'manual_review', 'turniere', 'immutable selection history after payload; trigger strips raw actor fields'),
    ('scrim', 'match_result_clarifications', 'request_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'result clarification request with automated nested erasure; DSAR HashOnly mode exposes only row id and payload digest'),
    ('scrim', 'match_result_clarifications', 'response_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'turniere', 'result clarification response with automated nested erasure; DSAR HashOnly mode exposes only row id and payload digest'),
    ('scrim', 'match_result_clarifications', 'requested_by_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'turniere', 'clarification requester id'),
    ('scrim', 'match_result_clarifications', 'requested_by_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'turniere', 'clarification requester label'),
    ('scrim', 'match_result_clarifications', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'turniere', 'remote system code'),
    ('scrim', 'match_result_clarifications', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'remote message id; direct user ids are not stored here'),
    ('steam', 'v1_workflows', 'workflow_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'workflow key; callers must not put user-entered text here'),
    ('steam', 'v1_workflows', 'workflow_type', 'machine_code', 'retain_operational', 'retain_non_personal', 'steam-core', 'workflow type code'),
    ('steam', 'v1_workflows', 'aggregate_id', 'free_text', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'domain aggregate id; only aggregate_kind=player is a Steam personal ref and is handled by kind-aware erasure'),
    ('steam', 'v1_workflows', 'subject_id', 'user_id', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'workflow subject may be a Discord user id'),
    ('steam', 'v1_workflows', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'workflow payload with automated nested user-id projection/redaction'),
    ('steam', 'v1_operations', 'operation_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'operation id; generated op: ids are preferred and direct user ids are not stored here'),
    ('steam', 'v1_operations', 'operation_type', 'machine_code', 'retain_operational', 'retain_non_personal', 'steam-core', 'operation type code'),
    ('steam', 'v1_operations', 'workflow_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'workflow key; callers must not put user-entered text here'),
    ('steam', 'v1_operations', 'aggregate_id', 'free_text', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'domain aggregate id; only aggregate_kind=player is a Steam personal ref and is handled by kind-aware erasure'),
    ('steam', 'v1_operations', 'subject_id', 'user_id', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'operation subject may be a Discord user id'),
    ('steam', 'v1_operations', 'idempotency_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'idempotency reference; callers must not put user-entered text here'),
    ('steam', 'v1_operations', 'payload_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'steam-core', 'payload digest only'),
    ('steam', 'v1_operations', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'operation payload with automated nested user-id projection/redaction'),
    ('steam', 'v1_operations', 'lease_owner', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'worker lease id, not user-entered free text'),
    ('steam', 'v1_operations', 'result_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'operation result payload with automated nested user-id projection/redaction'),
    ('steam', 'v1_operations', 'last_error_code', 'machine_code', 'retain_operational', 'retain_non_personal', 'steam-core', 'strict machine error code; raw error text is not stored'),
    ('steam', 'v1_operations', 'last_error_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'steam-core', 'raw error digest only'),
    ('steam', 'v1_operation_results', 'operation_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'operation id; direct user ids are not stored here'),
    ('steam', 'v1_operation_results', 'result_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'steam-core', 'operation result digest only'),
    ('steam', 'v1_operation_results', 'result_payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'durable operation result with automated nested user-id projection/redaction'),
    ('steam', 'v1_operation_events', 'operation_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'operation id; direct user ids are not stored here'),
    ('steam', 'v1_operation_events', 'event_type', 'machine_code', 'retain_operational', 'retain_non_personal', 'steam-core', 'operation event type code'),
    ('steam', 'v1_operation_events', 'payload_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'steam-core', 'event payload digest only'),
    ('steam', 'v1_operation_events', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'operation event payload with automated nested user-id projection/redaction'),
    ('steam', 'v1_deliveries', 'operation_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'operation id; direct user ids are not stored here'),
    ('steam', 'v1_deliveries', 'delivery_type', 'machine_code', 'retain_operational', 'retain_non_personal', 'steam-core', 'delivery type code'),
    ('steam', 'v1_deliveries', 'idempotency_key', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'idempotency reference; callers must not put user-entered text here'),
    ('steam', 'v1_deliveries', 'remote_system', 'machine_code', 'retain_operational', 'retain_non_personal', 'steam-core', 'remote system code'),
    ('steam', 'v1_deliveries', 'remote_message_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'remote message id; direct user ids are not stored here'),
    ('steam', 'v1_deliveries', 'remote_task_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'remote task id; direct user ids are not stored here'),
    ('steam', 'v1_deliveries', 'payload_hash', 'content_hash', 'retain_operational', 'retain_hash_only', 'steam-core', 'payload digest only'),
    ('steam', 'v1_deliveries', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'steam-core', 'delivery payload with automated nested user-id projection/redaction'),
    ('steam', 'v1_deliveries', 'lease_owner', 'domain_id', 'retain_operational', 'retain_non_personal', 'steam-core', 'worker lease id, not user-entered free text')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE
    SET data_category = EXCLUDED.data_category,
        retention_action = EXCLUDED.retention_action,
        erasure_action = EXCLUDED.erasure_action,
        owner_service = EXCLUDED.owner_service,
        reason = EXCLUDED.reason;

DROP PROCEDURE IF EXISTS scrim.validate_constraints(REGCLASS, TEXT[]);
DROP PROCEDURE IF EXISTS scrim.ensure_check_constraint(REGCLASS, TEXT, TEXT);

RESET lock_timeout;
