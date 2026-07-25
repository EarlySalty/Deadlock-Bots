SET lock_timeout TO '2s';

ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_match_id_fkey;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_validation_status_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_clarification_state_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_selected_valid_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_void_status_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_supersede_status_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_supersede_not_self_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_superseded_ref_fkey;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_external_result_ref_domain_ref_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_selection_reason_machine_code_check;
ALTER TABLE scrim.match_result_refs VALIDATE CONSTRAINT match_result_refs_void_reason_machine_code_check;
ALTER TABLE scrim.match_result_selections VALIDATE CONSTRAINT match_result_selections_selection_reason_machine_code_check;
ALTER TABLE scrim.match_result_selection_events VALIDATE CONSTRAINT match_result_selection_events_actor_source_enum_check;
ALTER TABLE scrim.match_result_clarifications VALIDATE CONSTRAINT match_result_clarifications_remote_system_machine_code_check;
ALTER TABLE scrim.match_result_clarifications VALIDATE CONSTRAINT match_result_clarifications_remote_message_id_domain_ref_check;

DO $$
DECLARE
    invalid_constraints TEXT;
BEGIN
    SELECT string_agg(conname, ', ' ORDER BY conname)
      INTO invalid_constraints
      FROM pg_constraint
     WHERE conrelid = 'scrim.match_result_refs'::regclass
       AND conname IN (
           'match_result_refs_match_id_fkey',
           'match_result_refs_validation_status_check',
           'match_result_refs_clarification_state_check',
           'match_result_refs_selected_valid_check',
             'match_result_refs_void_status_check',
             'match_result_refs_supersede_status_check',
             'match_result_refs_supersede_not_self_check',
             'match_result_refs_superseded_ref_fkey',
             'match_result_refs_external_result_ref_domain_ref_check',
             'match_result_refs_selection_reason_machine_code_check',
             'match_result_refs_void_reason_machine_code_check'
          )
        AND NOT convalidated;

    IF invalid_constraints IS NOT NULL THEN
        RAISE EXCEPTION 'scrim match result constraints are not validated: %', invalid_constraints
            USING ERRCODE = '23514';
    END IF;
END;
$$;

DO $$
DECLARE
    invalid_constraints TEXT;
BEGIN
    SELECT string_agg(conname, ', ' ORDER BY conname)
      INTO invalid_constraints
      FROM pg_constraint
     WHERE conrelid = 'scrim.match_result_selections'::regclass
       AND conname IN (
           'match_result_selections_selection_reason_machine_code_check'
       )
       AND NOT convalidated;

    IF invalid_constraints IS NOT NULL THEN
        RAISE EXCEPTION 'scrim match result selection constraints are not validated: %', invalid_constraints
            USING ERRCODE = '23514';
    END IF;
END;
$$;

DO $$
DECLARE
    invalid_constraints TEXT;
BEGIN
    SELECT string_agg(conname, ', ' ORDER BY conname)
      INTO invalid_constraints
      FROM pg_constraint
     WHERE conrelid = 'scrim.match_result_selection_events'::regclass
       AND conname IN (
           'match_result_selection_events_actor_source_enum_check'
       )
       AND NOT convalidated;

    IF invalid_constraints IS NOT NULL THEN
        RAISE EXCEPTION 'scrim match result selection event constraints are not validated: %', invalid_constraints
            USING ERRCODE = '23514';
    END IF;
END;
$$;

DO $$
DECLARE
    invalid_constraints TEXT;
BEGIN
    SELECT string_agg(conname, ', ' ORDER BY conname)
      INTO invalid_constraints
      FROM pg_constraint
     WHERE conrelid = 'scrim.match_result_clarifications'::regclass
       AND conname IN (
           'match_result_clarifications_remote_system_machine_code_check',
           'match_result_clarifications_remote_message_id_domain_ref_check'
       )
       AND NOT convalidated;

    IF invalid_constraints IS NOT NULL THEN
        RAISE EXCEPTION 'scrim match result clarification constraints are not validated: %', invalid_constraints
            USING ERRCODE = '23514';
    END IF;
END;
$$;

RESET lock_timeout;
