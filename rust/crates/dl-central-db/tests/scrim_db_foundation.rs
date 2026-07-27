#![cfg(feature = "testing")]

use std::{borrow::Cow, path::Path, time::Duration as StdDuration};

use dl_central_db::{connect_pool, testing::test_pool};
use sqlx::{migrate::Migrator, PgPool};

const PRE_FOUNDATION_VERSION: i64 = 2026072407;
const FOUNDATION_END_VERSION: i64 = 2026072418;

type ExistingResultRefRow = (i64, String, bool, String, bool, i32, bool, bool);

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn scrim_db_foundation_contract_tables_constraints_and_legacy_id_safety() {
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();

    assert_eq!(
        table_columns(pool, "scrim", "runtime_control").await,
        vec![
            "control_key",
            "mode",
            "epoch",
            "operational_writer",
            "actor_type",
            "actor_pseudonym",
            "actor_source",
            "request_id",
            "correlation_id",
            "decision_data",
            "created_at",
            "updated_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "runtime_control_history").await,
        vec![
            "id",
            "control_key",
            "mode",
            "epoch",
            "operational_writer",
            "actor_type",
            "actor_pseudonym",
            "actor_source",
            "request_id",
            "correlation_id",
            "decision_data",
            "created_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "command_receipts").await,
        vec![
            "id",
            "command_scope",
            "idempotency_key",
            "idempotency_generation",
            "payload_hash",
            "payload",
            "state",
            "lease_owner",
            "lease_until",
            "attempts",
            "next_attempt_at",
            "remote_system",
            "remote_message_id",
            "remote_task_id",
            "result_payload",
            "last_error_code",
            "last_error_hash",
            "received_at",
            "updated_at",
            "completed_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "inbox_events").await,
        vec![
            "id",
            "event_source",
            "source_event_id",
            "idempotency_key",
            "idempotency_generation",
            "payload_hash",
            "payload",
            "state",
            "lease_owner",
            "lease_until",
            "attempts",
            "next_attempt_at",
            "remote_system",
            "remote_message_id",
            "remote_task_id",
            "last_error_code",
            "last_error_hash",
            "received_at",
            "updated_at",
            "processed_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "outbox_effects").await,
        vec![
            "id",
            "effect_type",
            "idempotency_key",
            "idempotency_generation",
            "payload_hash",
            "payload",
            "state",
            "lease_owner",
            "lease_until",
            "attempts",
            "next_attempt_at",
            "remote_system",
            "remote_message_id",
            "remote_task_id",
            "command_receipt_id",
            "inbox_event_id",
            "last_error_code",
            "last_error_hash",
            "created_at",
            "updated_at",
            "delivered_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "audit_actor_pseudonyms").await,
        vec!["actor_type", "actor_ref", "actor_pseudonym", "created_at"],
    );
    assert_eq!(
        table_columns(pool, "scrim", "audit_events").await,
        vec![
            "id",
            "event_type",
            "entity_type",
            "entity_id",
            "actor_type",
            "actor_pseudonym",
            "actor_source",
            "request_id",
            "correlation_id",
            "before_data",
            "after_data",
            "decision_data",
            "metadata",
            "created_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "ai_runs").await,
        vec![
            "id",
            "run_kind",
            "subject_kind",
            "subject_id",
            "idempotency_key",
            "idempotency_generation",
            "input_hash",
            "state",
            "provider",
            "model",
            "lease_owner",
            "lease_until",
            "attempts",
            "next_attempt_at",
            "decision_ref_kind",
            "decision_ref_id",
            "lagebild_snapshot_id",
            "request_id",
            "correlation_id",
            "last_error_code",
            "last_error_hash",
            "created_at",
            "updated_at",
            "started_at",
            "finished_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "ai_decision_refs").await,
        vec![
            "id",
            "run_id",
            "decision_kind",
            "target_kind",
            "target_id",
            "decision_data",
            "confidence",
            "created_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "steam", "v1_workflows").await,
        vec![
            "id",
            "contract_version",
            "workflow_key",
            "workflow_type",
            "workflow_generation",
            "aggregate_kind",
            "aggregate_id",
            "state",
            "subject_kind",
            "subject_id",
            "payload",
            "created_at",
            "updated_at",
            "finished_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "steam", "v1_operations").await,
        vec![
            "id",
            "contract_version",
            "operation_id",
            "operation_type",
            "workflow_key",
            "workflow_generation",
            "aggregate_kind",
            "aggregate_id",
            "subject_kind",
            "subject_id",
            "idempotency_key",
            "idempotency_generation",
            "payload_hash",
            "payload",
            "state",
            "lease_owner",
            "lease_until",
            "attempts",
            "next_attempt_at",
            "legacy_task_id",
            "result_payload",
            "last_error_code",
            "last_error_hash",
            "created_at",
            "updated_at",
            "started_at",
            "finished_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "steam", "v1_operation_results").await,
        vec![
            "id",
            "operation_id",
            "operation_generation",
            "result_hash",
            "result_payload",
            "status",
            "created_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "steam", "v1_operation_events").await,
        vec![
            "id",
            "operation_id",
            "operation_generation",
            "event_type",
            "payload_hash",
            "payload",
            "created_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "steam", "v1_deliveries").await,
        vec![
            "id",
            "operation_id",
            "operation_generation",
            "delivery_type",
            "idempotency_key",
            "idempotency_generation",
            "remote_system",
            "remote_message_id",
            "remote_task_id",
            "payload_hash",
            "payload",
            "state",
            "attempts",
            "lease_owner",
            "lease_until",
            "created_at",
            "updated_at",
            "delivered_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "steam", "steam_tasks").await,
        vec![
            "id",
            "type",
            "payload",
            "status",
            "result",
            "error",
            "created_at",
            "updated_at",
            "started_at",
            "finished_at",
            "attempts",
            "bot_account_id",
        ],
        "legacy steam.steam_tasks compatibility stays intact",
    );
    assert_eq!(
        table_columns(pool, "scrim", "match_result_selections").await,
        vec![
            "match_id",
            "result_ref_id",
            "selected_at",
            "selected_by_user_id",
            "selected_by_display_name",
            "selection_reason",
            "created_at",
        ],
    );
    assert_eq!(
        table_columns(pool, "scrim", "match_result_selection_events").await,
        vec![
            "id",
            "event_type",
            "match_id",
            "result_ref_id",
            "old_result_ref_id",
            "new_result_ref_id",
            "actor_type",
            "actor_pseudonym",
            "actor_source",
            "before_data",
            "after_data",
            "created_at",
        ],
    );

    for table in [
        "participants",
        "teams",
        "matches",
        "match_request_batches",
        "match_requests",
    ] {
        assert_no_identity_default(pool, "scrim", table, "id").await;
    }

    assert_migration_helpers_are_not_public_api_and_constraints_are_validated(pool).await;
    assert_runtime_control_cas_audit_and_guards(pool).await;
    assert_audit_events_are_immutable_and_truncate_protected(pool).await;
    assert_idempotency_terminal_replay_and_lease_invariants(pool).await;
    assert_announcement_and_delivery_idempotency_is_permanent_and_immutable(pool).await;
    assert_error_fields_store_codes_not_raw_text(pool).await;
    assert_technical_fields_reject_bare_user_refs_and_display_names(pool).await;
    assert_command_lease_claim_is_single_consumer(pool).await;
    assert_result_refs_selection_supersede_and_delete_guards(pool).await;
    assert_match_result_ref_constraints_are_validated(pool).await;
    assert_replacement_request_candidate_need_consistency(pool).await;
    assert_scrim_request_workflow_constraints_and_links(pool).await;
    assert_steam_v1_idempotency_leases_and_result_guards(pool).await;
    assert_privacy_redaction_requires_deleted_marker_and_exact_target(pool).await;
    assert_lagebild_privacy_redaction_guards(pool).await;
    assert_privacy_registry_covers_new_user_id_display_name_and_json_fields(pool).await;
    assert_foundation_privacy_inventory_is_classified(pool).await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn match_result_ref_fetch_and_validation_statuses_stay_consistent() {
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool();
    seed_match(pool, 910_001, 910_002, 910_003).await;
    let superseded_by_ref_id = insert_result_ref(pool, 910_003, 910_001, 910_010, false)
        .await
        .expect("insert valid fetched result ref");

    sqlx::query(
        r#"INSERT INTO scrim.match_result_refs(
               match_id, steam_match_id, source_user_id, source_display_name,
               fetch_status, validation_status, voided_at, superseded_by_ref_id
           )
           VALUES
               ($1, 910011, '424242', 'Tester', 'pending', 'unvalidated', NULL, NULL),
               ($1, 910012, '424242', 'Tester', 'fetching', 'unvalidated', NULL, NULL),
               ($1, 910013, '424242', 'Tester', 'failed', 'unvalidated', NULL, NULL),
               ($1, 910014, '424242', 'Tester', 'fetched', 'ambiguous', NULL, NULL),
               ($1, 910015, '424242', 'Tester', 'fetched', 'rejected', NULL, NULL),
               ($1, 910016, '424242', 'Tester', 'fetched', 'void', now(), NULL),
               ($1, 910017, '424242', 'Tester', 'fetched', 'superseded', NULL, $2)"#,
    )
    .bind(910_003_i32)
    .bind(superseded_by_ref_id)
    .execute(pool)
    .await
    .expect("insert every allowed fetch/validation status pair");

    let fetched_unvalidated = sqlx::query(
        "INSERT INTO scrim.match_result_refs(
             match_id, steam_match_id, source_user_id, source_display_name,
             fetch_status, validation_status
         ) VALUES ($1, 910018, '424242', 'Tester', 'fetched', 'unvalidated')",
    )
    .bind(910_003_i32)
    .execute(pool)
    .await;
    let pending_valid = sqlx::query(
        "INSERT INTO scrim.match_result_refs(
             match_id, steam_match_id, source_user_id, source_display_name,
             fetch_status, validation_status
         ) VALUES ($1, 910019, '424242', 'Tester', 'pending', 'valid')",
    )
    .bind(910_003_i32)
    .execute(pool)
    .await;

    assert_eq!(
        (
            database_error_code(&fetched_unvalidated),
            database_error_code(&pending_valid),
        ),
        (Some("23514".to_string()), Some("23514".to_string()))
    );
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn scrim_privacy_registry_classifies_new_user_id_display_name_and_json_fields() {
    let db = test_pool().await.expect("create migrated test pool");
    assert_privacy_registry_covers_new_user_id_display_name_and_json_fields(db.pool()).await;
    assert_foundation_privacy_inventory_is_classified(db.pool()).await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn scrim_runtime_control_concurrent_transitions_serialize() {
    let db = test_pool().await.expect("create migrated test pool");
    let pool = db.pool().clone();
    let mut first_tx = pool.begin().await.expect("first runtime tx");
    let first: (bool, i64) = sqlx::query_as(
        "SELECT applied, current_epoch
           FROM scrim.transition_runtime_control(
                0, 'draining', 'turniere', '42', 'Tester', 'req:concurrent_1', 'corr:concurrent', '{}'::jsonb
           )",
    )
    .fetch_one(&mut *first_tx)
    .await
    .expect("first transition applies");
    assert_eq!(first, (true, 1));

    let second_pool = pool.clone();
    let second = tokio::spawn(async move {
        let mut tx = second_pool.begin().await.expect("second runtime tx");
        let row = sqlx::query_as::<_, (bool, i64)>(
            "SELECT applied, current_epoch
               FROM scrim.transition_runtime_control(
                     0, 'draining', 'turniere', '43', 'Tester2', 'req:concurrent_2', 'corr:concurrent', '{}'::jsonb
               )",
        )
        .fetch_one(&mut *tx)
        .await;
        if row.is_ok() {
            tx.commit().await.expect("commit second runtime tx");
        }
        row
    });
    wait_for_db_lock(&pool, "transition_runtime_control").await;
    first_tx.commit().await.expect("commit first runtime tx");
    let second = second
        .await
        .expect("second runtime task")
        .expect("second transition returns stale result, not unique violation");
    assert_eq!(second, (false, 1));
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn scrim_foundation_upgrades_real_2407_schema_without_identity_cutover() {
    let admin_dsn = test_dsn();
    let admin = connect_pool(&admin_dsn)
        .await
        .expect("connect admin database");
    let dbname = fresh_db_name("scrim_foundation_upgrade");
    create_fresh_db(&admin, &dbname).await;
    let db_dsn = swap_db(&admin_dsn, &dbname);
    let pool = connect_pool(&db_dsn)
        .await
        .expect("connect upgrade fixture database");

    run_migrations_between(&pool, i64::MIN, PRE_FOUNDATION_VERSION).await;
    seed_legacy_result_match(&pool, 720_001, 720_002, 720_003, 720_004).await;
    seed_legacy_match_with_steam_id_only(&pool, 720_011, 720_012, 720_013, 720_014).await;

    run_migrations_between(&pool, i64::MIN, FOUNDATION_END_VERSION).await;

    for table in [
        "participants",
        "teams",
        "matches",
        "match_request_batches",
        "match_requests",
    ] {
        assert_no_identity_default(&pool, "scrim", table, "id").await;
    }

    let selected: (Option<i64>, Option<i64>, Option<i32>, Option<String>) = sqlx::query_as(
        "SELECT result_ref_id, steam_match_id, winner_team_id, source
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(720_003_i32)
    .fetch_one(&pool)
    .await
    .expect("selected result view after upgrade");
    assert!(
        selected.0.is_some(),
        "legacy match result is backfilled to result_refs"
    );
    assert_eq!(selected.1, Some(720_004_i64));
    assert_eq!(selected.2, Some(720_001_i32));
    assert_eq!(selected.3.as_deref(), Some("result_ref"));

    let active: (Option<i64>, Option<i64>, Option<i32>, Option<String>) = sqlx::query_as(
        "SELECT result_ref_id, steam_match_id, winner_team_id, source
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(720_013_i32)
    .fetch_one(&pool)
    .await
    .expect("active steam-id-only match remains unselected");
    assert_eq!(active, (None, None, None, None));

    let active_refs: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT
           FROM scrim.match_result_refs
          WHERE match_id = $1
            AND (is_selected OR validation_status <> 'unvalidated')",
    )
    .bind(720_013_i32)
    .fetch_one(&pool)
    .await
    .expect("active ref count");
    assert_eq!(active_refs, 0);

    pool.close().await;
    drop_db(&admin, &dbname).await;
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn scrim_foundation_updates_existing_result_refs_before_inserting_missing_refs() {
    let admin_dsn = test_dsn();
    let admin = connect_pool(&admin_dsn)
        .await
        .expect("connect admin database");
    let dbname = fresh_db_name("scrim_foundation_existing_ref");
    create_fresh_db(&admin, &dbname).await;
    let db_dsn = swap_db(&admin_dsn, &dbname);
    let pool = connect_pool(&db_dsn)
        .await
        .expect("connect upgrade fixture database");

    run_migrations_between(&pool, i64::MIN, PRE_FOUNDATION_VERSION).await;
    seed_legacy_result_match(&pool, 721_001, 721_002, 721_003, 721_004).await;
    let existing_ref_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO scrim.match_result_refs(
             match_id,
             steam_match_id,
             source_user_id,
             source_display_name,
             fetch_status,
             fetched_at,
             winner_team_id,
             raw_result_json,
             normalized_result_json
          )
          VALUES (
             $1,
             $2,
             '424242',
             'Tester',
             'fetched',
             TIMESTAMPTZ '2026-07-24 12:00:00+00',
             $3,
             '{"raw":"keep"}'::jsonb,
             '{"normalized":"keep"}'::jsonb
          )
          RETURNING id::BIGINT"#,
    )
    .bind(721_003_i32)
    .bind(721_004_i64)
    .bind(721_002_i32)
    .fetch_one(&pool)
    .await
    .expect("existing fetched result ref");

    run_migrations_between(&pool, i64::MIN, FOUNDATION_END_VERSION).await;

    let refs: Vec<ExistingResultRefRow> = sqlx::query_as(
        r#"SELECT id::BIGINT,
                validation_status,
                is_selected,
                fetch_status,
                fetched_at = TIMESTAMPTZ '2026-07-24 12:00:00+00',
                winner_team_id,
                raw_result_json = '{"raw":"keep"}'::jsonb,
                normalized_result_json = '{"normalized":"keep"}'::jsonb
           FROM scrim.match_result_refs
          WHERE match_id = $1
          ORDER BY id"#,
    )
    .bind(721_003_i32)
    .fetch_all(&pool)
    .await
    .expect("backfilled refs");
    assert_eq!(
        refs,
        vec![(
            existing_ref_id,
            "valid".to_string(),
            false,
            "fetched".to_string(),
            true,
            721_002,
            true,
            true,
        )]
    );

    let selected_ref_id: i64 = sqlx::query_scalar(
        "SELECT result_ref_id
           FROM scrim.match_result_selections
          WHERE match_id = $1",
    )
    .bind(721_003_i32)
    .fetch_one(&pool)
    .await
    .expect("backfilled canonical selection");
    assert_eq!(selected_ref_id, existing_ref_id);

    pool.close().await;
    drop_db(&admin, &dbname).await;
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires CENTRAL_TEST_DSN; run via central_test_db.sh"]
async fn scrim_foundation_aborts_cross_match_steam_result_conflicts() {
    let admin_dsn = test_dsn();
    let admin = connect_pool(&admin_dsn)
        .await
        .expect("connect admin database");
    let dbname = fresh_db_name("scrim_foundation_cross_ref");
    create_fresh_db(&admin, &dbname).await;
    let db_dsn = swap_db(&admin_dsn, &dbname);
    let pool = connect_pool(&db_dsn)
        .await
        .expect("connect upgrade fixture database");

    run_migrations_between(&pool, i64::MIN, PRE_FOUNDATION_VERSION).await;
    seed_legacy_result_match(&pool, 722_001, 722_002, 722_003, 722_004).await;
    seed_match(&pool, 722_011, 722_012, 722_013).await;
    sqlx::query(
        "INSERT INTO scrim.match_result_refs(
             match_id, steam_match_id, source_user_id, source_display_name
         )
          VALUES ($1, $2, '424242', 'Tester')",
    )
    .bind(722_013_i32)
    .bind(722_004_i64)
    .execute(&pool)
    .await
    .expect("conflicting pre-foundation result ref");

    let result = run_migrations_between_result(&pool, i64::MIN, FOUNDATION_END_VERSION).await;
    let err = result.expect_err("cross-match result-ref conflict must abort migration");
    assert!(
        err.to_string()
            .contains("legacy steam_match_id conflicts with an existing result ref"),
        "unexpected migration error: {err}"
    );

    pool.close().await;
    drop_db(&admin, &dbname).await;
    admin.close().await;
}

async fn table_columns(pool: &PgPool, schema: &str, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT column_name
           FROM information_schema.columns
          WHERE table_schema = $1
            AND table_name = $2
          ORDER BY ordinal_position",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|err| panic!("columns for {schema}.{table}: {err}"))
}

async fn column_data_type(pool: &PgPool, schema: &str, table: &str, column: &str) -> String {
    sqlx::query_scalar(
        "SELECT data_type
           FROM information_schema.columns
          WHERE table_schema = $1
            AND table_name = $2
            AND column_name = $3",
    )
    .bind(schema)
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("data type for {schema}.{table}.{column}: {err}"))
}

async fn wait_for_db_lock(pool: &PgPool, query_fragment: &str) {
    let query_pattern = format!("%{query_fragment}%");
    tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            let waiting = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(
                    SELECT 1
                      FROM pg_stat_activity
                     WHERE datname = current_database()
                       AND pid <> pg_backend_pid()
                       AND state = 'active'
                       AND wait_event_type = 'Lock'
                       AND query LIKE $1
                )",
            )
            .bind(&query_pattern)
            .fetch_one(pool)
            .await
            .expect("pg_stat_activity lock wait");
            if waiting {
                return;
            }
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("DB lock wait for {query_fragment} not visible"));
}

async fn assert_no_identity_default(pool: &PgPool, schema: &str, table: &str, column: &str) {
    let identity: String = sqlx::query_scalar(
        "SELECT is_identity
           FROM information_schema.columns
          WHERE table_schema = $1
            AND table_name = $2
            AND column_name = $3",
    )
    .bind(schema)
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("identity flag for {schema}.{table}.{column}: {err}"));

    assert_eq!(identity, "NO", "identity cutover is deliberately deferred");
}

async fn assert_migration_helpers_are_not_public_api_and_constraints_are_validated(pool: &PgPool) {
    let helpers_exist: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
               FROM pg_proc AS proc
               JOIN pg_namespace AS ns
                 ON ns.oid = proc.pronamespace
              WHERE ns.nspname = 'scrim'
                AND proc.proname IN ('ensure_check_constraint', 'validate_constraints')
         )",
    )
    .fetch_one(pool)
    .await
    .expect("migration helper procedure lookup");
    assert!(
        !helpers_exist,
        "migration-only dynamic-SQL helpers must not remain callable in final schema"
    );

    let checked_constraints: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT
           FROM pg_constraint AS con
           JOIN pg_class AS rel
             ON rel.oid = con.conrelid
           JOIN pg_namespace AS ns
             ON ns.oid = rel.relnamespace
          WHERE ns.nspname IN ('scrim', 'steam')
            AND con.contype = 'c'
            AND (
                 con.conname LIKE '%_machine_code_check'
                 OR con.conname LIKE '%_domain_ref_check'
                 OR con.conname LIKE '%_actor_source_enum_check'
                 OR con.conname = 'audit_actor_pseudonyms_actor_ref_format_check'
            )",
    )
    .fetch_one(pool)
    .await
    .expect("technical check constraints count");
    assert!(
        checked_constraints > 20,
        "expected validated technical check constraints to remain after helper drop"
    );

    let unvalidated_constraints: Vec<(String, String)> = sqlx::query_as(
        "SELECT rel.relname::TEXT, con.conname::TEXT
           FROM pg_constraint AS con
           JOIN pg_class AS rel
             ON rel.oid = con.conrelid
           JOIN pg_namespace AS ns
             ON ns.oid = rel.relnamespace
          WHERE ns.nspname IN ('scrim', 'steam')
            AND con.contype = 'c'
            AND NOT con.convalidated
            AND (
                 con.conname LIKE '%_machine_code_check'
                 OR con.conname LIKE '%_domain_ref_check'
                 OR con.conname LIKE '%_actor_source_enum_check'
                 OR con.conname = 'audit_actor_pseudonyms_actor_ref_format_check'
            )
          ORDER BY rel.relname, con.conname",
    )
    .fetch_all(pool)
    .await
    .expect("technical unvalidated check constraints");
    assert!(
        unvalidated_constraints.is_empty(),
        "technical check constraints must stay validated after helper drop: {unvalidated_constraints:?}"
    );
}

async fn assert_runtime_control_cas_audit_and_guards(pool: &PgPool) {
    let runtime_kind: String = sqlx::query_scalar(
        "SELECT relkind::TEXT FROM pg_class WHERE oid = 'scrim.runtime_control'::regclass",
    )
    .fetch_one(pool)
    .await
    .expect("runtime relation kind");
    assert_eq!(runtime_kind, "v", "runtime_control must be a view");

    let runtime_comment: String =
        sqlx::query_scalar("SELECT obj_description('scrim.runtime_control'::regclass, 'pg_class')")
            .fetch_one(pool)
            .await
            .expect("runtime control table comment");
    assert!(runtime_comment.contains("Non-updatable current runtime state"));

    let mut forged = pool.begin().await.expect("forged runtime tx");
    sqlx::query("SELECT set_config('scrim.runtime_control_transition', '1', true)")
        .execute(&mut *forged)
        .await
        .expect("set forged runtime guc");
    let direct_update = sqlx::query(
        "UPDATE scrim.runtime_control
            SET mode = 'draining',
                operational_writer = 'turniere'
          WHERE control_key = 'scrim_runtime'",
    )
    .execute(&mut *forged)
    .await;
    assert!(
        database_error_code(&direct_update).is_some(),
        "forged runtime GUC must not authorize direct view update"
    );
    forged.rollback().await.expect("rollback forged runtime tx");

    let invalid_transition = sqlx::query(
        "SELECT applied, current_epoch
           FROM scrim.transition_runtime_control(
                0, 'turniere', 'turniere', '42', 'Tester', 'req:invalid', 'corr:invalid', '{}'::jsonb
           )",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&invalid_transition).as_deref(),
        Some("23514")
    );

    let applied: (bool, i64) = sqlx::query_as(
        "SELECT applied, current_epoch
           FROM scrim.transition_runtime_control(
                 0, 'draining', 'turniere', '42', 'Tester', 'req:rt', 'corr:rt', '{\"why\":\"test\"}'::jsonb
           )",
    )
    .fetch_one(pool)
    .await
    .expect("runtime transition CAS applies");
    assert_eq!(applied, (true, 1));

    let stale: (bool, i64) = sqlx::query_as(
        "SELECT applied, current_epoch
           FROM scrim.transition_runtime_control(
                 0, 'turniere', 'turniere', '42', 'Tester', 'req:stale', 'corr:stale', '{}'::jsonb
           )",
    )
    .fetch_one(pool)
    .await
    .expect("runtime transition CAS stale returns current epoch");
    assert_eq!(stale, (false, 1));

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT
           FROM scrim.audit_events
          WHERE event_type = 'runtime_control_transition'
            AND request_id = 'req:rt'",
    )
    .fetch_one(pool)
    .await
    .expect("runtime audit count");
    assert_eq!(audit_count, 1);

    let audit_actor: (String, String, bool, bool) = sqlx::query_as(
        "SELECT actor_pseudonym,
                actor_source,
                before_data ? 'actor_pseudonym',
                after_data ? 'actor_pseudonym'
           FROM scrim.audit_events
          WHERE event_type = 'runtime_control_transition'
            AND request_id = 'req:rt'",
    )
    .fetch_one(pool)
    .await
    .expect("runtime audit pseudonymous actor");
    let expected_pseudonym: String = sqlx::query_scalar(
        "SELECT actor_pseudonym
           FROM scrim.audit_actor_pseudonyms
          WHERE actor_type = 'user'
            AND actor_ref = '42'",
    )
    .fetch_one(pool)
    .await
    .expect("expected runtime actor pseudonym");
    assert_eq!(audit_actor.0, expected_pseudonym);
    assert!(audit_actor.0.starts_with("act_"));
    assert_eq!(audit_actor.1, "user");
    assert!(
        !audit_actor.2,
        "audit before_data must not keep raw actor fields"
    );
    assert!(
        !audit_actor.3,
        "audit after_data must not keep raw actor fields"
    );

    sqlx::query(
        "INSERT INTO scrim.runtime_control_history(
             control_key, mode, epoch, operational_writer, actor_type, actor_pseudonym,
             actor_source, request_id, correlation_id, decision_data
         ) VALUES (
             'scrim_runtime', 'turniere', 2, 'turniere', 'user',
             scrim.audit_actor_pseudonym('user', '43'), 'user', 'req:direct_rt',
             'corr:direct_rt', '{\"why\":\"direct\"}'::jsonb
         )",
    )
    .execute(pool)
    .await
    .expect("valid direct next-epoch insert is allowed");

    let direct_audit: (i64, bool, bool) = sqlx::query_as(
        "SELECT count(*)::BIGINT,
                COALESCE(bool_or(before_data ? 'actor_pseudonym'), false),
                COALESCE(bool_or(after_data ? 'actor_pseudonym'), false)
           FROM scrim.audit_events
          WHERE event_type = 'runtime_control_transition'
            AND request_id = 'req:direct_rt'",
    )
    .fetch_one(pool)
    .await
    .expect("direct runtime insert audit");
    assert_eq!(direct_audit.0, 1);
    assert_eq!((direct_audit.1, direct_audit.2), (false, false));

    let delete =
        sqlx::query("DELETE FROM scrim.runtime_control WHERE control_key = 'scrim_runtime'")
            .execute(pool)
            .await;
    assert!(database_error_code(&delete).is_some());

    let truncate = sqlx::query("TRUNCATE scrim.runtime_control")
        .execute(pool)
        .await;
    assert!(database_error_code(&truncate).is_some());

    let history_update = sqlx::query(
        "UPDATE scrim.runtime_control_history SET request_id = 'mutated' WHERE epoch = 1",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&history_update).as_deref(),
        Some("55000")
    );

    let history_delete = sqlx::query("DELETE FROM scrim.runtime_control_history WHERE epoch = 1")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&history_delete).as_deref(),
        Some("55000")
    );

    let skipped_epoch = sqlx::query(
        "INSERT INTO scrim.runtime_control_history(
             control_key, mode, epoch, operational_writer, actor_type, actor_pseudonym, actor_source
         ) VALUES (
             'scrim_runtime', 'draining', 4, 'turniere', 'user', scrim.audit_actor_pseudonym('user', '43'), 'user'
         )",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&skipped_epoch).as_deref(),
        Some("23514")
    );
}

async fn assert_audit_events_are_immutable_and_truncate_protected(pool: &PgPool) {
    let raw_user_insert = sqlx::query(
        "INSERT INTO scrim.audit_actor_pseudonyms(actor_type, actor_ref, actor_pseudonym)
         VALUES ('user', 'Alice', 'act_00000000000000000000000000000001')",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&raw_user_insert).as_deref(),
        Some("23514")
    );

    let raw_service_insert = sqlx::query(
        "INSERT INTO scrim.audit_actor_pseudonyms(actor_type, actor_ref, actor_pseudonym)
         VALUES ('service', 'Alice', 'act_00000000000000000000000000000002')",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&raw_service_insert).as_deref(),
        Some("23514")
    );

    for (sql, label) in [
        (
            "SELECT scrim.audit_actor_pseudonym('user', 'Alice')",
            "user display name",
        ),
        (
            "SELECT scrim.audit_actor_pseudonym('user', '')",
            "blank user ref",
        ),
        (
            "SELECT scrim.audit_actor_pseudonym('user', NULL)",
            "null user ref",
        ),
        (
            "SELECT scrim.audit_actor_pseudonym('service', 'Alice')",
            "service free text",
        ),
    ] {
        let result = sqlx::query_scalar::<_, String>(sql).fetch_one(pool).await;
        assert_eq!(
            database_error_code(&result).as_deref(),
            Some("23514"),
            "audit actor function must reject {label}"
        );
    }

    let service_pseudonym: String = sqlx::query_scalar(
        "SELECT scrim.audit_actor_pseudonym('service', 'service:privacy_contract')",
    )
    .fetch_one(pool)
    .await
    .expect("service domain ref audit actor pseudonym");
    assert!(service_pseudonym.starts_with("act_"));

    let expected_pseudonym: String =
        sqlx::query_scalar("SELECT scrim.audit_actor_pseudonym('user', '424242')")
            .fetch_one(pool)
            .await
            .expect("expected audit actor pseudonym");
    let audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO scrim.audit_events(event_type, entity_type, actor_type, actor_pseudonym, actor_source, before_data, after_data, decision_data)
           VALUES ('contract_test', 'match', 'user', $1, 'user', '{}'::jsonb, '{"after":true}'::jsonb, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(&expected_pseudonym)
    .fetch_one(pool)
    .await
    .expect("insert audit event");

    for sql in [
        "UPDATE scrim.audit_events SET metadata = '{}'::jsonb WHERE id = $1",
        "DELETE FROM scrim.audit_events WHERE id = $1",
    ] {
        let result = sqlx::query(sql).bind(audit_id).execute(pool).await;
        assert_eq!(database_error_code(&result).as_deref(), Some("55000"));
    }

    let actor: (String, String) = sqlx::query_as(
        "SELECT actor_pseudonym, actor_source
           FROM scrim.audit_events
          WHERE id = $1",
    )
    .bind(audit_id)
    .fetch_one(pool)
    .await
    .expect("pseudonymous audit actor");
    assert_eq!(actor, (expected_pseudonym.clone(), "user".to_string()));
    assert_ne!(actor.0, "424242");

    let repeated_pseudonym: String =
        sqlx::query_scalar("SELECT scrim.audit_actor_pseudonym('user', '424242')")
            .fetch_one(pool)
            .await
            .expect("repeat audit actor pseudonym");
    assert_eq!(repeated_pseudonym, expected_pseudonym);
    let raw_entity = sqlx::query(
        r#"INSERT INTO scrim.audit_events(event_type, entity_type, entity_id, actor_type, actor_pseudonym, actor_source)
           VALUES ('raw_entity_test', 'discord_user', '42', 'user', $1, 'user')"#,
    )
    .bind(&expected_pseudonym)
    .execute(pool)
    .await;
    assert_eq!(database_error_code(&raw_entity).as_deref(), Some("23514"));
    sqlx::query(
        r#"INSERT INTO scrim.audit_events(event_type, entity_type, entity_id, actor_type, actor_pseudonym, actor_source)
           VALUES ('pseudonym_entity_test', 'discord_user', $1, 'user', $1, 'user')"#,
    )
    .bind(&expected_pseudonym)
    .execute(pool)
    .await
    .expect("opaque personal audit entity id is allowed");
    let unknown_entity_type = sqlx::query(
        r#"INSERT INTO scrim.audit_events(event_type, entity_type, entity_id, actor_type, actor_pseudonym, actor_source)
           VALUES ('unknown_entity_test', 'discord_user_profile', '42', 'user', $1, 'user')"#,
    )
    .bind(&expected_pseudonym)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&unknown_entity_type).as_deref(),
        Some("23514")
    );
    sqlx::query("DELETE FROM scrim.audit_actor_pseudonyms WHERE actor_type = 'user' AND actor_ref = '424242'")
        .execute(pool)
        .await
        .expect("delete raw audit actor mapping");
    let regenerated_pseudonym: String =
        sqlx::query_scalar("SELECT scrim.audit_actor_pseudonym('user', '424242')")
            .fetch_one(pool)
            .await
            .expect("regenerate audit actor pseudonym");
    assert_ne!(regenerated_pseudonym, expected_pseudonym);

    let truncate = sqlx::query("TRUNCATE scrim.audit_events CASCADE")
        .execute(pool)
        .await;
    assert_eq!(database_error_code(&truncate).as_deref(), Some("55000"));
}

async fn assert_privacy_redaction_requires_deleted_marker_and_exact_target(pool: &PgPool) {
    let hash = vec![8_u8; 32];
    let guarded_44: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload)
         VALUES ('privacy_guard', 'target:44', $1, '{\"id\":\"44\"}'::jsonb)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert privacy guard 44 row");
    let guarded_42: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload)
         VALUES ('privacy_guard', 'target:42', $1, '{\"id\":\"42\"}'::jsonb)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert privacy guard 42 row");

    let mut forged_without_marker = pool.begin().await.expect("privacy forged tx");
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', '44', true),
             set_config('scrim.privacy_erasure_target_ref', '44', true)",
    )
    .execute(&mut *forged_without_marker)
    .await
    .expect("set forged privacy guc");
    let unauthorized = sqlx::query(
        "UPDATE scrim.command_receipts
            SET payload = scrim.jsonb_redact_user_ref(payload, '44')
          WHERE id = $1",
    )
    .bind(guarded_44)
    .execute(&mut *forged_without_marker)
    .await;
    assert_eq!(
        database_error_code(&unauthorized).as_deref(),
        Some("55000"),
        "GUC without deletion marker must not authorize privacy mutation"
    );
    forged_without_marker
        .rollback()
        .await
        .expect("rollback forged privacy tx");

    sqlx::query(
        "INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
         VALUES (42, true, now(), 'test', now())
         ON CONFLICT(user_id) DO UPDATE SET
             opted_out = true,
             deleted_at = excluded.deleted_at,
             reason = excluded.reason,
             updated_at = excluded.updated_at",
    )
    .execute(pool)
    .await
    .expect("insert deleted privacy marker");

    let mut authorized = pool.begin().await.expect("authorized privacy tx");
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', '42', true),
             set_config('scrim.privacy_erasure_target_ref', '42', true)",
    )
    .execute(&mut *authorized)
    .await
    .expect("set authorized privacy guc");
    sqlx::query(
        "UPDATE scrim.command_receipts
            SET payload = scrim.jsonb_redact_user_ref(payload, '42')
          WHERE id = $1",
    )
    .bind(guarded_42)
    .execute(&mut *authorized)
    .await
    .expect("authorized privacy redaction");
    authorized
        .commit()
        .await
        .expect("commit authorized privacy tx");

    let guarded_43: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload)
         VALUES ('privacy_guard', 'foreign:43', $1, '{\"id\":\"43\"}'::jsonb)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert privacy guard 43 row");
    let mut forged_foreign = pool.begin().await.expect("foreign privacy tx");
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', '42', true),
             set_config('scrim.privacy_erasure_target_ref', '43', true)",
    )
    .execute(&mut *forged_foreign)
    .await
    .expect("set foreign privacy guc");
    let foreign = sqlx::query(
        "UPDATE scrim.command_receipts
            SET payload = scrim.jsonb_redact_user_ref(payload, '43')
          WHERE id = $1",
    )
    .bind(guarded_43)
    .execute(&mut *forged_foreign)
    .await;
    assert_eq!(
        database_error_code(&foreign).as_deref(),
        Some("55000"),
        "marker for user 42 must not authorize redaction of user 43"
    );
    forged_foreign
        .rollback()
        .await
        .expect("rollback foreign privacy tx");

    let foreign_payload_id: String =
        sqlx::query_scalar("SELECT payload ->> 'id' FROM scrim.command_receipts WHERE id = $1")
            .bind(guarded_43)
            .fetch_one(pool)
            .await
            .expect("foreign payload after rejected redaction");
    assert_eq!(foreign_payload_id, "43");

    seed_match(pool, 880_001, 880_002, 880_003).await;
    let lineup_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO scrim.match_lineup_snapshots(
              match_id, snapshot_kind, lineup_payload, source, created_by_user_id, created_by_display_name
           ) VALUES (
              880003,
              'planned',
              '{"players":[{"id":"42","name":"Target"},{"id":"99","name":"Foreign"}]}'::jsonb,
              'turniere',
              '99',
              'ForeignCreator'
           )
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert adversarial lineup snapshot");

    let mut overredaction = pool.begin().await.expect("overredaction tx");
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', '42', true),
             set_config('scrim.privacy_erasure_target_ref', '42', true)",
    )
    .execute(&mut *overredaction)
    .await
    .expect("set privacy context for overredaction");
    let overredact_foreign_actor = sqlx::query(
        "UPDATE scrim.match_lineup_snapshots
            SET lineup_payload = scrim.jsonb_redact_user_ref(lineup_payload, '42'),
                created_by_user_id = 'redacted',
                created_by_display_name = 'redacted'
          WHERE id = $1",
    )
    .bind(lineup_id)
    .execute(&mut *overredaction)
    .await;
    assert_eq!(
        database_error_code(&overredact_foreign_actor).as_deref(),
        Some("55000"),
        "target inside lineup_payload must not authorize foreign actor redaction"
    );
    overredaction
        .rollback()
        .await
        .expect("rollback overredaction tx");

    let mut payload_only = pool.begin().await.expect("payload-only tx");
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', '42', true),
             set_config('scrim.privacy_erasure_target_ref', '42', true)",
    )
    .execute(&mut *payload_only)
    .await
    .expect("set privacy context for payload-only redaction");
    sqlx::query(
        "UPDATE scrim.match_lineup_snapshots
            SET lineup_payload = scrim.jsonb_redact_user_ref(lineup_payload, '42')
          WHERE id = $1",
    )
    .bind(lineup_id)
    .execute(&mut *payload_only)
    .await
    .expect("payload-only lineup redaction remains allowed");
    payload_only
        .commit()
        .await
        .expect("commit payload-only redaction");
    let lineup_actor: (String, String) = sqlx::query_as(
        "SELECT created_by_user_id, created_by_display_name
           FROM scrim.match_lineup_snapshots
          WHERE id = $1",
    )
    .bind(lineup_id)
    .fetch_one(pool)
    .await
    .expect("lineup actor after payload redaction");
    assert_eq!(
        lineup_actor,
        ("99".to_string(), "ForeignCreator".to_string())
    );
    let lineup_payload: String = sqlx::query_scalar(
        "SELECT lineup_payload::text
           FROM scrim.match_lineup_snapshots
          WHERE id = $1",
    )
    .bind(lineup_id)
    .fetch_one(pool)
    .await
    .expect("lineup payload after target redaction");
    assert!(
        !lineup_payload.contains("Target"),
        "target label in redacted player object must be scrubbed: {lineup_payload}"
    );
    assert!(
        lineup_payload.contains("Foreign"),
        "foreign player evidence must remain after target redaction: {lineup_payload}"
    );
}

async fn assert_lagebild_privacy_redaction_guards(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, created_at) VALUES(880011, 'Privacy Team', now())",
    )
    .execute(pool)
    .await
    .expect("insert lagebild privacy team");
    let snapshot_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO scrim.lagebild_snapshots(
              team_id, generated_for, source, status, lagebild_text, data_summary, error
           ) VALUES (
              880011,
              'privacy-test',
              'ai',
              'error',
              'Spieler 4242 braucht Review',
              '{"actor":"4242","display_name":"Target"}'::jsonb,
              'Fehler fuer 4242'
           )
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert lagebild privacy snapshot");
    sqlx::query(
        r#"INSERT INTO scrim.lagebild_evidences(
              snapshot_id, evidence_type, label, reference_id, payload
           ) VALUES (
              $1,
              'discord_message',
              'Evidence 4242',
              '4242',
              '{"actor":"4242","display_name":"Target"}'::jsonb
           )"#,
    )
    .bind(snapshot_id)
    .execute(pool)
    .await
    .expect("insert lagebild privacy evidence");
    let correction_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO scrim.lagebild_corrections(
              team_id, snapshot_id, role, author_user_id, author_display_name, message
           ) VALUES (
              880011,
              $1,
              'user',
              '4242',
              'Target',
              'Korrektur fuer 4242'
           )
           RETURNING id"#,
    )
    .bind(snapshot_id)
    .fetch_one(pool)
    .await
    .expect("insert lagebild privacy correction");

    let mut unauthorized = pool
        .begin()
        .await
        .expect("unauthorized lagebild privacy tx");
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', '4242', true),
             set_config('scrim.privacy_erasure_target_ref', '4242', true)",
    )
    .execute(&mut *unauthorized)
    .await
    .expect("set unauthorized lagebild privacy context");
    let blocked = sqlx::query(
        "UPDATE scrim.lagebild_corrections
            SET message = replace(message, '4242', 'redacted')
          WHERE id = $1",
    )
    .bind(correction_id)
    .execute(&mut *unauthorized)
    .await;
    assert_eq!(
        database_error_code(&blocked).as_deref(),
        Some("55000"),
        "Lagebild redaction requires a deleted privacy marker"
    );
    unauthorized
        .rollback()
        .await
        .expect("rollback unauthorized lagebild privacy tx");

    sqlx::query(
        "INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
         VALUES (4242, true, now(), 'test', now())
         ON CONFLICT(user_id) DO UPDATE SET
             opted_out = true,
             deleted_at = excluded.deleted_at,
             reason = excluded.reason,
             updated_at = excluded.updated_at",
    )
    .execute(pool)
    .await
    .expect("insert lagebild privacy marker");

    let mut authorized = pool.begin().await.expect("authorized lagebild privacy tx");
    sqlx::query(
        "SELECT
             set_config('scrim.privacy_erasure_user_id', '4242', true),
             set_config('scrim.privacy_erasure_target_ref', '4242', true)",
    )
    .execute(&mut *authorized)
    .await
    .expect("set authorized lagebild privacy context");
    sqlx::query(
        "UPDATE scrim.lagebild_snapshots
            SET lagebild_text = replace(lagebild_text, '4242', 'redacted'),
                data_summary = scrim.jsonb_redact_user_ref(data_summary, '4242'),
                error = replace(error, '4242', 'redacted')
          WHERE id = $1",
    )
    .bind(snapshot_id)
    .execute(&mut *authorized)
    .await
    .expect("authorized lagebild snapshot redaction");
    sqlx::query(
        "UPDATE scrim.lagebild_evidences
            SET label = replace(label, '4242', 'redacted'),
                reference_id = replace(reference_id, '4242', 'redacted'),
                payload = scrim.jsonb_redact_user_ref(payload, '4242')
          WHERE snapshot_id = $1",
    )
    .bind(snapshot_id)
    .execute(&mut *authorized)
    .await
    .expect("authorized lagebild evidence redaction");
    sqlx::query(
        "UPDATE scrim.lagebild_corrections
            SET author_user_id = 'redacted',
                author_display_name = 'redacted',
                message = replace(message, '4242', 'redacted')
          WHERE id = $1",
    )
    .bind(correction_id)
    .execute(&mut *authorized)
    .await
    .expect("authorized lagebild correction redaction");
    authorized
        .commit()
        .await
        .expect("commit authorized lagebild privacy tx");

    let remaining_refs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM (
                 SELECT lagebild_text || ' ' || COALESCE(error, '') || ' ' || data_summary::text AS text
                   FROM scrim.lagebild_snapshots
                  WHERE id = $1
                 UNION ALL
                 SELECT label || ' ' || COALESCE(reference_id, '') || ' ' || payload::text
                   FROM scrim.lagebild_evidences
                  WHERE snapshot_id = $1
                 UNION ALL
                 SELECT COALESCE(author_user_id, '') || ' ' || COALESCE(author_display_name, '') || ' ' || message
                   FROM scrim.lagebild_corrections
                  WHERE id = $2
                ) AS projected
          WHERE text LIKE '%4242%' OR text LIKE '%Target%'",
    )
    .bind(snapshot_id)
    .bind(correction_id)
    .fetch_one(pool)
    .await
    .expect("count remaining lagebild private refs");
    assert_eq!(remaining_refs, 0);
}

async fn assert_idempotency_terminal_replay_and_lease_invariants(pool: &PgPool) {
    let hash = vec![1_u8; 32];
    sqlx::query(
        "INSERT INTO scrim.command_receipts(
             command_scope, idempotency_key, payload_hash, payload, state, completed_at
         )
          VALUES ('match_release', 'idem:terminal', $1, '{}'::jsonb, 'completed', now())",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert terminal command receipt");

    let same_generation = sqlx::query(
        "INSERT INTO scrim.command_receipts(
             command_scope, idempotency_key, payload_hash, payload, state, completed_at
         )
          VALUES ('match_release', 'idem:terminal', $1, '{}'::jsonb, 'completed', now())",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&same_generation).as_deref(),
        Some("23505")
    );

    sqlx::query(
        "INSERT INTO scrim.command_receipts(
             command_scope, idempotency_key, idempotency_generation, payload_hash, payload, state
         )
          VALUES ('match_release', 'idem:terminal', 1, $1, '{}'::jsonb, 'received')",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("explicit new generation is allowed after terminal receipt");

    let second_active_generation = sqlx::query(
        "INSERT INTO scrim.command_receipts(
             command_scope, idempotency_key, idempotency_generation, payload_hash, payload, state
         )
          VALUES ('match_release', 'idem:terminal', 2, $1, '{}'::jsonb, 'received')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&second_active_generation).as_deref(),
        Some("23505")
    );

    let mutate_idempotency_hash = sqlx::query(
        "UPDATE scrim.command_receipts
            SET payload_hash = $1
          WHERE command_scope = 'match_release'
            AND idempotency_key = 'idem:terminal'
            AND idempotency_generation = 0",
    )
    .bind(vec![8_u8; 32])
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_idempotency_hash).as_deref(),
        Some("55000")
    );

    let mutate_command_payload = sqlx::query(
        "UPDATE scrim.command_receipts
            SET payload = '{\"changed\":true}'::jsonb
          WHERE command_scope = 'match_release'
            AND idempotency_key = 'idem:terminal'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_command_payload).as_deref(),
        Some("55000")
    );

    let reopen_terminal_command = sqlx::query(
        "UPDATE scrim.command_receipts
            SET state = 'received', completed_at = NULL
          WHERE command_scope = 'match_release'
            AND idempotency_key = 'idem:terminal'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_command).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO scrim.command_receipts(
             command_scope, idempotency_key, payload_hash, payload, state, completed_at
         )
          VALUES ('match_release', 'idem:cancelled', $1, '{}'::jsonb, 'cancelled', now())",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert cancelled command receipt");
    let reopen_cancelled_command = sqlx::query(
        "UPDATE scrim.command_receipts
            SET state = 'retry', completed_at = NULL, next_attempt_at = now()
          WHERE command_scope = 'match_release'
            AND idempotency_key = 'idem:cancelled'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_cancelled_command).as_deref(),
        Some("55000")
    );

    let delete_command = sqlx::query(
        "DELETE FROM scrim.command_receipts
          WHERE command_scope = 'match_release'
            AND idempotency_key = 'idem:terminal'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delete_command).as_deref(),
        Some("55000")
    );

    let truncate_command = sqlx::query("TRUNCATE scrim.command_receipts CASCADE")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_command).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO scrim.inbox_events(event_source, source_event_id, idempotency_key, payload_hash, payload)
         VALUES ('discord', 'evt:1', 'inbox:freeze', $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert inbox event");
    let mutate_inbox_identity = sqlx::query(
        "UPDATE scrim.inbox_events
            SET source_event_id = 'evt:mutated'
          WHERE event_source = 'discord'
            AND idempotency_key = 'inbox:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_inbox_identity).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "UPDATE scrim.inbox_events
            SET state = 'processed', processed_at = now()
          WHERE event_source = 'discord'
            AND idempotency_key = 'inbox:freeze'",
    )
    .execute(pool)
    .await
    .expect("mark inbox terminal");
    let reopen_terminal_inbox = sqlx::query(
        "UPDATE scrim.inbox_events
            SET state = 'received', processed_at = NULL
          WHERE event_source = 'discord'
            AND idempotency_key = 'inbox:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_inbox).as_deref(),
        Some("55000")
    );
    let delete_inbox = sqlx::query(
        "DELETE FROM scrim.inbox_events
          WHERE event_source = 'discord'
            AND idempotency_key = 'inbox:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(database_error_code(&delete_inbox).as_deref(), Some("55000"));
    let truncate_inbox = sqlx::query("TRUNCATE scrim.inbox_events CASCADE")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_inbox).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload)
         VALUES ('discord_message', 'outbox:freeze', $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert outbox effect");
    let mutate_outbox_payload = sqlx::query(
        "UPDATE scrim.outbox_effects
            SET payload = '{\"changed\":true}'::jsonb
          WHERE effect_type = 'discord_message'
            AND idempotency_key = 'outbox:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_outbox_payload).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "UPDATE scrim.outbox_effects
            SET state = 'delivered', delivered_at = now()
          WHERE effect_type = 'discord_message'
            AND idempotency_key = 'outbox:freeze'",
    )
    .execute(pool)
    .await
    .expect("mark outbox terminal");
    let reopen_terminal_outbox = sqlx::query(
        "UPDATE scrim.outbox_effects
            SET state = 'pending', delivered_at = NULL
          WHERE effect_type = 'discord_message'
            AND idempotency_key = 'outbox:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_outbox).as_deref(),
        Some("55000")
    );
    let delete_outbox = sqlx::query(
        "DELETE FROM scrim.outbox_effects
          WHERE effect_type = 'discord_message'
            AND idempotency_key = 'outbox:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delete_outbox).as_deref(),
        Some("55000")
    );
    let truncate_outbox = sqlx::query("TRUNCATE scrim.outbox_effects CASCADE")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_outbox).as_deref(),
        Some("55000")
    );

    let effect_receipt_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.effect_receipts(
             remote_system, remote_message_id, payload_hash, receipt_payload, status
         )
          VALUES ('discord', 'msg:effect_receipt', $1, '{}'::jsonb, 'confirmed')
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert immutable effect receipt");
    let mutate_effect_receipt = sqlx::query(
        "UPDATE scrim.effect_receipts
            SET receipt_payload = '{\"changed\":true}'::jsonb
          WHERE id = $1",
    )
    .bind(effect_receipt_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_effect_receipt).as_deref(),
        Some("55000")
    );
    let delete_effect_receipt = sqlx::query("DELETE FROM scrim.effect_receipts WHERE id = $1")
        .bind(effect_receipt_id)
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&delete_effect_receipt).as_deref(),
        Some("55000")
    );
    let truncate_effect_receipt = sqlx::query("TRUNCATE scrim.effect_receipts")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_effect_receipt).as_deref(),
        Some("55000")
    );

    let processing_without_lease = sqlx::query(
        "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload, state)
         VALUES ('lease', 'lease:no', $1, '{}'::jsonb, 'processing')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&processing_without_lease).as_deref(),
        Some("23514")
    );

    let lease_without_processing = sqlx::query(
        "INSERT INTO scrim.command_receipts(
             command_scope, idempotency_key, payload_hash, payload, state, lease_owner, lease_until
         )
          VALUES ('lease', 'lease:wrong_state', $1, '{}'::jsonb, 'received', 'worker:1', now() + interval '1 minute')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&lease_without_processing).as_deref(),
        Some("23514")
    );

    let retry_without_time = sqlx::query(
        "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload, state)
         VALUES ('lease', 'lease:retry_no_time', $1, '{}'::jsonb, 'retry')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&retry_without_time).as_deref(),
        Some("23514")
    );
}

async fn assert_announcement_and_delivery_idempotency_is_permanent_and_immutable(pool: &PgPool) {
    let hash = vec![3_u8; 32];
    let published_announcement_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.announcement_drafts(
             scope, title, body, payload, payload_hash, idempotency_key,
             created_by_user_id, created_by_display_name, status, approved_at, published_at
         )
         VALUES (
             'global',
             'Title',
             'Body',
             '{}'::jsonb,
             scrim.announcement_effect_hash('global', NULL, 'Title', 'Body', '{}'::jsonb),
              'announcement:once',
             '42',
             'Tester',
             'published',
             now(),
             now()
         )
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert published announcement draft");

    let approval_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.announcement_approvals(
             draft_id, decision, decided_by_user_id, decided_by_display_name, decision_data
         )
         VALUES ($1, 'approved', '42', 'Tester', '{}'::jsonb)
         RETURNING id",
    )
    .bind(published_announcement_id)
    .fetch_one(pool)
    .await
    .expect("insert immutable announcement approval");
    let mutate_approval = sqlx::query(
        "UPDATE scrim.announcement_approvals
            SET decision_data = '{\"changed\":true}'::jsonb
          WHERE id = $1",
    )
    .bind(approval_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_approval).as_deref(),
        Some("55000")
    );
    sqlx::query(
        "WITH marker AS (
             INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
              VALUES (42, true, now(), 'test', now())
             ON CONFLICT(user_id) DO UPDATE SET
                 opted_out = true,
                 deleted_at = excluded.deleted_at,
                 reason = excluded.reason,
                 updated_at = excluded.updated_at
             RETURNING user_id
         ),
         privacy AS (
             SELECT
                  set_config('scrim.privacy_erasure_user_id', '42', true),
                  set_config('scrim.privacy_erasure_target_ref', '42', true)
               FROM marker
         )
         UPDATE scrim.announcement_approvals
             SET decided_by_user_id = 'redacted',
                 decided_by_display_name = 'redacted'
           FROM privacy
          WHERE id = $1",
    )
    .bind(approval_id)
    .execute(pool)
    .await
    .expect("privacy redaction remains allowed for approval actors");
    let delete_approval = sqlx::query("DELETE FROM scrim.announcement_approvals WHERE id = $1")
        .bind(approval_id)
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&delete_approval).as_deref(),
        Some("55000")
    );
    let truncate_approval = sqlx::query("TRUNCATE scrim.announcement_approvals")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_approval).as_deref(),
        Some("55000")
    );

    seed_match(pool, 901_001, 901_002, 901_003).await;
    let lineup_snapshot_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.match_lineup_snapshots(
             match_id, snapshot_kind, lineup_payload, source, created_by_user_id, created_by_display_name
         )
         VALUES ($1, 'planned', '{}'::jsonb, 'turniere', '42', 'Tester')
         RETURNING id",
    )
    .bind(901_003_i32)
    .fetch_one(pool)
    .await
    .expect("insert immutable lineup snapshot");
    let mutate_lineup_snapshot = sqlx::query(
        "UPDATE scrim.match_lineup_snapshots
            SET lineup_payload = '{\"changed\":true}'::jsonb
          WHERE id = $1",
    )
    .bind(lineup_snapshot_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_lineup_snapshot).as_deref(),
        Some("55000")
    );
    sqlx::query(
        "WITH marker AS (
             INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
              VALUES (42, true, now(), 'test', now())
             ON CONFLICT(user_id) DO UPDATE SET
                 opted_out = true,
                 deleted_at = excluded.deleted_at,
                 reason = excluded.reason,
                 updated_at = excluded.updated_at
             RETURNING user_id
         ),
         privacy AS (
             SELECT
                  set_config('scrim.privacy_erasure_user_id', '42', true),
                  set_config('scrim.privacy_erasure_target_ref', '42', true)
               FROM marker
         )
         UPDATE scrim.match_lineup_snapshots
             SET created_by_user_id = 'redacted',
                 created_by_display_name = 'redacted'
           FROM privacy
          WHERE id = $1",
    )
    .bind(lineup_snapshot_id)
    .execute(pool)
    .await
    .expect("privacy redaction remains allowed for lineup snapshot actors");
    let delete_lineup_snapshot =
        sqlx::query("DELETE FROM scrim.match_lineup_snapshots WHERE id = $1")
            .bind(lineup_snapshot_id)
            .execute(pool)
            .await;
    assert_eq!(
        database_error_code(&delete_lineup_snapshot).as_deref(),
        Some("55000")
    );
    let truncate_lineup_snapshot = sqlx::query("TRUNCATE scrim.match_lineup_snapshots")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_lineup_snapshot).as_deref(),
        Some("55000")
    );

    let replay = sqlx::query(
        "INSERT INTO scrim.announcement_drafts(
             scope, title, body, payload, payload_hash, idempotency_key,
             created_by_user_id, created_by_display_name, status
         )
         VALUES (
             'global',
             'Again',
             'Body',
             '{}'::jsonb,
             scrim.announcement_effect_hash('global', NULL, 'Again', 'Body', '{}'::jsonb),
              'announcement:once',
             '42',
             'Tester',
             'draft'
         )",
    )
    .execute(pool)
    .await;
    assert_eq!(database_error_code(&replay).as_deref(), Some("23505"));

    let mutate_announcement = sqlx::query(
        "UPDATE scrim.announcement_drafts
            SET payload_hash = $1
          WHERE idempotency_key = 'announcement:once'",
    )
    .bind(vec![4_u8; 32])
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_announcement).as_deref(),
        Some("55000")
    );

    let reopen_terminal_announcement = sqlx::query(
        "UPDATE scrim.announcement_drafts
            SET status = 'draft', published_at = NULL
          WHERE idempotency_key = 'announcement:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_announcement).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO scrim.announcement_drafts(
             scope, title, body, payload, payload_hash, idempotency_key, idempotency_generation,
             created_by_user_id, created_by_display_name, status
         )
         VALUES (
             'global',
             'Next',
             'Body',
             '{}'::jsonb,
             scrim.announcement_effect_hash('global', NULL, 'Next', 'Body', '{}'::jsonb),
              'announcement:once',
             1,
             '42',
             'Tester',
             'draft'
         )",
    )
    .execute(pool)
    .await
    .expect("announcement can advance generation after terminal status");

    sqlx::query(
        "UPDATE scrim.announcement_drafts
            SET title = 'Edited',
                body = 'Edited body',
                payload = '{\"edited\":true}'::jsonb,
                payload_hash = scrim.announcement_effect_hash('global', NULL, 'Edited', 'Edited body', '{\"edited\":true}'::jsonb)
          WHERE idempotency_key = 'announcement:once'
            AND idempotency_generation = 1",
    )
    .execute(pool)
    .await
    .expect("draft announcement content remains editable when hash follows content");

    sqlx::query(
        "UPDATE scrim.announcement_drafts
            SET status = 'approved', approved_at = now()
          WHERE idempotency_key = 'announcement:once'
            AND idempotency_generation = 1",
    )
    .execute(pool)
    .await
    .expect("approve editable announcement");

    let second_active_announcement = sqlx::query(
        "INSERT INTO scrim.announcement_drafts(
             scope, title, body, payload, payload_hash, idempotency_key, idempotency_generation,
             created_by_user_id, created_by_display_name, status
         )
         VALUES (
             'global',
             'Blocked',
             'Body',
             '{}'::jsonb,
             scrim.announcement_effect_hash('global', NULL, 'Blocked', 'Body', '{}'::jsonb),
              'announcement:once',
             2,
             '42',
             'Tester',
             'draft'
         )",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&second_active_announcement).as_deref(),
        Some("23505")
    );

    let mutate_announcement_payload = sqlx::query(
        "UPDATE scrim.announcement_drafts
            SET body = 'Changed after approval',
                payload_hash = scrim.announcement_effect_hash('global', NULL, 'Edited', 'Changed after approval', '{\"edited\":true}'::jsonb)
          WHERE idempotency_key = 'announcement:once'
            AND idempotency_generation = 1",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_announcement_payload).as_deref(),
        Some("55000")
    );

    let delete_announcement = sqlx::query(
        "DELETE FROM scrim.announcement_drafts
          WHERE idempotency_key = 'announcement:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delete_announcement).as_deref(),
        Some("55000")
    );
    let truncate_announcement = sqlx::query("TRUNCATE scrim.announcement_drafts CASCADE")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_announcement).as_deref(),
        Some("55000")
    );

    let bad_status_hash = sqlx::query(
        "INSERT INTO scrim.status_publication_approvals(
             target_kind, target_id, status_kind, payload, payload_hash
         )
          VALUES ('match', '901003', 'summary', '{}'::jsonb, $1)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&bad_status_hash).as_deref(),
        Some("23514")
    );

    sqlx::query(
        "INSERT INTO scrim.status_publication_approvals(
             target_kind, target_id, status_kind, payload, payload_hash
         )
         VALUES (
             'match',
              '901003',
             'summary',
             '{}'::jsonb,
              scrim.status_publication_effect_hash('match', '901003', 'summary', '{}'::jsonb)
         )",
    )
    .execute(pool)
    .await
    .expect("insert status publication approval");
    sqlx::query(
        "UPDATE scrim.status_publication_approvals
            SET payload = '{\"edited\":true}'::jsonb,
                payload_hash = scrim.status_publication_effect_hash('match', '901003', 'summary', '{\"edited\":true}'::jsonb)
          WHERE target_kind = 'match'
            AND target_id = '901003'
            AND status_kind = 'summary'",
    )
    .execute(pool)
    .await
    .expect("pending status publication content remains editable when hash follows content");
    sqlx::query(
        "UPDATE scrim.status_publication_approvals
            SET decision = 'approved',
                decided_by_user_id = '42',
                decided_by_display_name = 'Tester',
                decided_at = now()
          WHERE target_kind = 'match'
            AND target_id = '901003'
            AND status_kind = 'summary'",
    )
    .execute(pool)
    .await
    .expect("approve status publication");
    let mutate_status_payload = sqlx::query(
        "UPDATE scrim.status_publication_approvals
            SET payload = '{\"changed_after_approval\":true}'::jsonb,
                payload_hash = scrim.status_publication_effect_hash('match', '901003', 'summary', '{\"changed_after_approval\":true}'::jsonb)
          WHERE target_kind = 'match'
            AND target_id = '901003'
            AND status_kind = 'summary'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_status_payload).as_deref(),
        Some("55000")
    );
    sqlx::query(
        "UPDATE scrim.status_publication_approvals
            SET decision = 'published'
          WHERE target_kind = 'match'
            AND target_id = '901003'
            AND status_kind = 'summary'",
    )
    .execute(pool)
    .await
    .expect("publish status publication");
    let reopen_terminal_status = sqlx::query(
        "UPDATE scrim.status_publication_approvals
            SET decision = 'pending', decided_at = NULL
          WHERE target_kind = 'match'
            AND target_id = '901003'
            AND status_kind = 'summary'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_status).as_deref(),
        Some("55000")
    );
    let delete_status_publication = sqlx::query(
        "DELETE FROM scrim.status_publication_approvals
          WHERE target_kind = 'match'
            AND target_id = '901003'
            AND status_kind = 'summary'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delete_status_publication).as_deref(),
        Some("55000")
    );
    let truncate_status_publication =
        sqlx::query("TRUNCATE scrim.status_publication_approvals CASCADE")
            .execute(pool)
            .await;
    assert_eq!(
        database_error_code(&truncate_status_publication).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO steam.v1_operations(operation_type, idempotency_key, payload_hash, payload)
         VALUES ('fetch_match', 'operation:once', $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert steam operation");
    let operation_replay = sqlx::query(
        "INSERT INTO steam.v1_operations(operation_type, idempotency_key, payload_hash, payload)
         VALUES ('fetch_match', 'operation:once', $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&operation_replay).as_deref(),
        Some("23505")
    );
    let active_operation_generation = sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_type, idempotency_key, idempotency_generation, payload_hash, payload
         )
          VALUES ('fetch_match', 'operation:once', 1, $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&active_operation_generation).as_deref(),
        Some("23505")
    );
    sqlx::query(
        "UPDATE steam.v1_operations
            SET state = 'succeeded', finished_at = now()
          WHERE operation_type = 'fetch_match'
            AND idempotency_key = 'operation:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await
    .expect("mark operation terminal");
    let reopen_terminal_operation = sqlx::query(
        "UPDATE steam.v1_operations
            SET state = 'pending', finished_at = NULL
          WHERE operation_type = 'fetch_match'
            AND idempotency_key = 'operation:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_operation).as_deref(),
        Some("55000")
    );
    sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_type, idempotency_key, idempotency_generation, payload_hash, payload
         )
          VALUES ('fetch_match', 'operation:once', 1, $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("operation can advance generation after terminal status");
    let mutate_operation = sqlx::query(
        "UPDATE steam.v1_operations
            SET payload = '{\"changed\":true}'::jsonb
          WHERE operation_type = 'fetch_match'
            AND idempotency_key = 'operation:once'
            AND idempotency_generation = 1",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_operation).as_deref(),
        Some("55000")
    );
    let delete_operation = sqlx::query(
        "DELETE FROM steam.v1_operations
          WHERE operation_type = 'fetch_match'
            AND idempotency_key = 'operation:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delete_operation).as_deref(),
        Some("55000")
    );
    let truncate_operation = sqlx::query("TRUNCATE steam.v1_operations CASCADE")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_operation).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO steam.v1_deliveries(
             delivery_type, idempotency_key, payload_hash, remote_system, payload
         )
          VALUES ('discord_message', 'delivery:once', $1, 'discord', '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert delivery");

    let delivery_replay = sqlx::query(
        "INSERT INTO steam.v1_deliveries(
             delivery_type, idempotency_key, payload_hash, remote_system, payload
         )
          VALUES ('discord_message', 'delivery:once', $1, 'discord', '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delivery_replay).as_deref(),
        Some("23505")
    );

    let active_delivery_generation = sqlx::query(
        "INSERT INTO steam.v1_deliveries(
             delivery_type, idempotency_key, idempotency_generation, payload_hash, remote_system, payload
         )
          VALUES ('discord_message', 'delivery:once', 1, $1, 'discord', '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&active_delivery_generation).as_deref(),
        Some("23505")
    );

    sqlx::query(
        "UPDATE steam.v1_deliveries
            SET state = 'delivered', delivered_at = now()
          WHERE delivery_type = 'discord_message'
            AND idempotency_key = 'delivery:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await
    .expect("mark delivery terminal");

    let reopen_terminal_delivery = sqlx::query(
        "UPDATE steam.v1_deliveries
            SET state = 'pending', delivered_at = NULL
          WHERE delivery_type = 'discord_message'
            AND idempotency_key = 'delivery:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_delivery).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO steam.v1_deliveries(
             delivery_type, idempotency_key, idempotency_generation, payload_hash, remote_system, payload
         )
          VALUES ('discord_message', 'delivery:once', 1, $1, 'discord', '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("delivery can advance generation after terminal status");

    let mutate_delivery = sqlx::query(
        "UPDATE steam.v1_deliveries
            SET idempotency_key = 'delivery:mutated'
          WHERE delivery_type = 'discord_message'
            AND idempotency_key = 'delivery:once'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_delivery).as_deref(),
        Some("55000")
    );

    let delete_delivery = sqlx::query(
        "DELETE FROM steam.v1_deliveries
          WHERE delivery_type = 'discord_message'
            AND idempotency_key = 'delivery:once'
            AND idempotency_generation = 0",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delete_delivery).as_deref(),
        Some("55000")
    );
    let truncate_delivery = sqlx::query("TRUNCATE steam.v1_deliveries CASCADE")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_delivery).as_deref(),
        Some("55000")
    );

    let ai_run_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
         VALUES ('lagebild', 'team', '1', 'ai:freeze', $1)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert ai run");
    let runtime_decision_ref_id: i64 = sqlx::query_scalar(
        "SELECT id
           FROM scrim.runtime_control_history
          WHERE control_key = 'scrim_runtime'
          ORDER BY epoch
          LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("runtime history decision ref");
    sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash,
             decision_ref_kind, decision_ref_id
         )
         VALUES ('lagebild', 'team', '1', 'ai:valid_decision_ref', $1, 'runtime_control_history', $2)",
    )
    .bind(&hash)
    .bind(runtime_decision_ref_id.to_string())
    .execute(pool)
    .await
    .expect("insert ai run with valid non-personal decision ref");
    let decision_kind_without_id = sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash, decision_ref_kind
         )
          VALUES ('lagebild', 'team', '1', 'ai:decision_kind_without_id', $1, 'runtime_control_history')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&decision_kind_without_id).as_deref(),
        Some("23514")
    );
    let decision_id_without_kind = sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash, decision_ref_id
         )
          VALUES ('lagebild', 'team', '1', 'ai:decision_id_without_kind', $1, '1')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&decision_id_without_kind).as_deref(),
        Some("23514")
    );
    let raw_discord_decision_ref = sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash,
             decision_ref_kind, decision_ref_id
         )
          VALUES ('lagebild', 'team', '1', 'ai:decision_discord_user', $1, 'discord_user', '42')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&raw_discord_decision_ref).as_deref(),
        Some("23514")
    );
    let raw_steam_decision_ref = sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash,
             decision_ref_kind, decision_ref_id
         )
          VALUES ('lagebild', 'team', '1', 'ai:decision_steam_user', $1, 'steam_user', '7656119800000042')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&raw_steam_decision_ref).as_deref(),
        Some("23514")
    );
    let unknown_decision_ref_kind = sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash,
             decision_ref_kind, decision_ref_id
         )
          VALUES ('lagebild', 'team', '1', 'ai:decision_unknown_kind', $1, 'external_ticket', '1')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&unknown_decision_ref_kind).as_deref(),
        Some("23514")
    );
    let non_numeric_decision_ref_id = sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash,
             decision_ref_kind, decision_ref_id
         )
          VALUES ('lagebild', 'team', '1', 'ai:decision_non_numeric_id', $1, 'runtime_control_history', '42-user')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&non_numeric_decision_ref_id).as_deref(),
        Some("23514")
    );
    let missing_decision_ref = sqlx::query(
        "INSERT INTO scrim.ai_runs(
             run_kind, subject_kind, subject_id, idempotency_key, input_hash,
             decision_ref_kind, decision_ref_id
         )
          VALUES ('lagebild', 'team', '1', 'ai:decision_missing_ref', $1, 'match_result_ref', '999999999999')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&missing_decision_ref).as_deref(),
        Some("23503")
    );
    let invalid_ai_subject_kind = sqlx::query(
        "INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
         VALUES ('lagebild', 'user', '42', 'ai:invalid_subject_kind', $1)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&invalid_ai_subject_kind).as_deref(),
        Some("23514")
    );
    sqlx::query("INSERT INTO core.users(discord_id) VALUES (42) ON CONFLICT DO NOTHING")
        .execute(pool)
        .await
        .expect("core user for ai run privacy");
    sqlx::query(
        "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
         VALUES (42, '7656119800000042', 7656119800000042)
         ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("steam link for ai run privacy");
    sqlx::query(
        "INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
         VALUES (42, true, now(), 'test', now())
         ON CONFLICT(user_id) DO UPDATE SET
             opted_out = true,
             deleted_at = excluded.deleted_at,
             reason = excluded.reason,
             updated_at = excluded.updated_at",
    )
    .execute(pool)
    .await
    .expect("privacy marker for ai run redaction");
    let ai_discord_run_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
         VALUES ('lagebild', 'discord_user', '42', 'ai:discord_redact', $1)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert discord-user ai run");
    let ai_steam_run_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
         VALUES ('lagebild', 'steam_user', '7656119800000042', 'ai:steam_redact', $1)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert steam-user ai run");
    let ai_team_collision_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash)
         VALUES ('lagebild', 'team', '42', 'ai:team_collision', $1)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert team ai run collision row");
    let mutate_ai_subject_kind = sqlx::query(
        "UPDATE scrim.ai_runs
            SET subject_kind = 'discord_user'
          WHERE id = $1",
    )
    .bind(ai_team_collision_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_ai_subject_kind).as_deref(),
        Some("55000")
    );
    let redact_team_as_user = sqlx::query(
        "WITH privacy AS (
             SELECT
                   set_config('scrim.privacy_erasure_user_id', '43', true),
                   set_config('scrim.privacy_erasure_target_ref', '43', true)
         )
         UPDATE scrim.ai_runs
            SET subject_id = 'redacted'
           FROM privacy
          WHERE id = $1",
    )
    .bind(ai_team_collision_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&redact_team_as_user).as_deref(),
        Some("55000")
    );
    sqlx::query(
        "WITH privacy AS (
             SELECT
                 set_config('scrim.privacy_erasure_user_id', '42', true),
                 set_config('scrim.privacy_erasure_target_ref', '42', true)
         )
         UPDATE scrim.ai_runs
            SET subject_id = 'redacted'
           FROM privacy
          WHERE id = $1",
    )
    .bind(ai_discord_run_id)
    .execute(pool)
    .await
    .expect("discord-user ai run redaction allowed");
    sqlx::query(
        "WITH privacy AS (
             SELECT
                 set_config('scrim.privacy_erasure_user_id', '42', true),
                 set_config('scrim.privacy_erasure_target_ref', '7656119800000042', true)
         )
         UPDATE scrim.ai_runs
            SET subject_id = 'redacted'
           FROM privacy
          WHERE id = $1",
    )
    .bind(ai_steam_run_id)
    .execute(pool)
    .await
    .expect("steam-user ai run redaction allowed");
    let ai_privacy_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT subject_kind, subject_id
           FROM scrim.ai_runs
          WHERE id = ANY($1)
          ORDER BY subject_kind",
    )
    .bind([ai_discord_run_id, ai_steam_run_id, ai_team_collision_id])
    .fetch_all(pool)
    .await
    .expect("ai privacy rows after redaction");
    assert_eq!(
        ai_privacy_rows,
        vec![
            ("discord_user".to_string(), "redacted".to_string()),
            ("steam_user".to_string(), "redacted".to_string()),
            ("team".to_string(), "42".to_string()),
        ]
    );
    let ai_decision_ref_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.ai_decision_refs(
             run_id, decision_kind, target_kind, target_id, decision_data, confidence
          )
          VALUES ($1, 'classification', 'match', '1', '{}'::jsonb, 0.7000)
          RETURNING id",
    )
    .bind(ai_run_id)
    .fetch_one(pool)
    .await
    .expect("insert immutable ai decision ref");
    let raw_personal_ai_target = sqlx::query(
        "INSERT INTO scrim.ai_decision_refs(
             run_id, decision_kind, target_kind, target_id, decision_data, confidence
          )
          VALUES ($1, 'classification', 'discord_user', '42', '{}'::jsonb, 0.7000)",
    )
    .bind(ai_run_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&raw_personal_ai_target).as_deref(),
        Some("23514")
    );
    let non_numeric_ai_target = sqlx::query(
        "INSERT INTO scrim.ai_decision_refs(
             run_id, decision_kind, target_kind, target_id, decision_data, confidence
          )
          VALUES ($1, 'classification', 'match', '42-user', '{}'::jsonb, 0.7000)",
    )
    .bind(ai_run_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&non_numeric_ai_target).as_deref(),
        Some("23514")
    );
    let mutate_ai_decision_ref = sqlx::query(
        "UPDATE scrim.ai_decision_refs
            SET decision_data = '{\"changed\":true}'::jsonb
          WHERE id = $1",
    )
    .bind(ai_decision_ref_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_ai_decision_ref).as_deref(),
        Some("55000")
    );
    let delete_ai_decision_ref = sqlx::query("DELETE FROM scrim.ai_decision_refs WHERE id = $1")
        .bind(ai_decision_ref_id)
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&delete_ai_decision_ref).as_deref(),
        Some("55000")
    );
    let truncate_ai_decision_ref = sqlx::query("TRUNCATE scrim.ai_decision_refs")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_ai_decision_ref).as_deref(),
        Some("55000")
    );
    let mutate_ai_run = sqlx::query(
        "UPDATE scrim.ai_runs
            SET input_hash = $1
          WHERE run_kind = 'lagebild'
            AND idempotency_key = 'ai:freeze'",
    )
    .bind(vec![5_u8; 32])
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_ai_run).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "UPDATE scrim.ai_runs
            SET state = 'succeeded', finished_at = now()
          WHERE run_kind = 'lagebild'
            AND idempotency_key = 'ai:freeze'",
    )
    .execute(pool)
    .await
    .expect("mark ai run terminal");
    let reopen_terminal_ai = sqlx::query(
        "UPDATE scrim.ai_runs
            SET state = 'queued', finished_at = NULL
          WHERE run_kind = 'lagebild'
            AND idempotency_key = 'ai:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_ai).as_deref(),
        Some("55000")
    );
    let delete_ai = sqlx::query(
        "DELETE FROM scrim.ai_runs
          WHERE run_kind = 'lagebild'
            AND idempotency_key = 'ai:freeze'",
    )
    .execute(pool)
    .await;
    assert_eq!(database_error_code(&delete_ai).as_deref(), Some("55000"));
    let truncate_ai = sqlx::query("TRUNCATE scrim.ai_runs CASCADE")
        .execute(pool)
        .await;
    assert_eq!(database_error_code(&truncate_ai).as_deref(), Some("55000"));
}

async fn assert_error_fields_store_codes_not_raw_text(pool: &PgPool) {
    let hash = vec![3_u8; 32];
    for sql in [
        "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload, last_error_code)
         VALUES ('error_privacy', 'command:raw_user', $1, '{}'::jsonb, '42')",
        "INSERT INTO scrim.inbox_events(event_source, idempotency_key, payload_hash, payload, last_error_code)
         VALUES ('error_privacy', 'inbox:raw_user', $1, '{}'::jsonb, '42')",
        "INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload, last_error_code)
         VALUES ('error_privacy', 'outbox:raw_user', $1, '{}'::jsonb, '42')",
        "INSERT INTO scrim.ai_runs(run_kind, subject_kind, subject_id, idempotency_key, input_hash, last_error_code)
         VALUES ('error_privacy', 'match', '1', 'ai:raw_user', $1, '42')",
        "INSERT INTO steam.v1_operations(operation_type, idempotency_key, payload_hash, payload, last_error_code)
         VALUES ('error_privacy', 'steam:raw_user', $1, '{}'::jsonb, '42')",
    ] {
        let result = sqlx::query(sql).bind(&hash).execute(pool).await;
        assert_eq!(
            database_error_code(&result).as_deref(),
            Some("23514"),
            "raw user-looking error text must be rejected for {sql}"
        );
    }

    sqlx::query(
        "INSERT INTO scrim.command_receipts(
             command_scope, idempotency_key, payload_hash, payload, last_error_code, last_error_hash
         )
         VALUES ('error_privacy', 'command:safe_code', $1, '{}'::jsonb, 'err_remote_timeout', $1)",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("strict machine error code plus hash is accepted");
}

async fn assert_technical_fields_reject_bare_user_refs_and_display_names(pool: &PgPool) {
    let hash = vec![4_u8; 32];
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, created_at) VALUES (920001, 'Privacy Probe', now())",
    )
    .execute(pool)
    .await
    .expect("privacy probe team");

    for (sql, field) in [
        (
            "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload)
             VALUES ('privacy_probe', '42', $1, '{}'::jsonb)",
            "command idempotency_key",
        ),
        (
            "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload)
             VALUES ('DeleteMe', 'probe:scope', $1, '{}'::jsonb)",
            "command command_scope",
        ),
        (
            "INSERT INTO scrim.inbox_events(event_source, source_event_id, idempotency_key, payload_hash, payload)
             VALUES ('privacy_probe', 'DeleteMe', 'probe:inbox', $1, '{}'::jsonb)",
            "inbox source_event_id",
        ),
        (
            "INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload, remote_message_id)
             VALUES ('privacy_probe', 'probe:outbox', $1, '{}'::jsonb, '42')",
            "outbox remote_message_id",
        ),
        (
            "INSERT INTO scrim.replacement_needs(team_id, reason, created_by_user_id, created_by_display_name)
             VALUES (920001, 'DeleteMe', '99', 'OtherUser')",
            "replacement reason",
        ),
        (
            "INSERT INTO steam.v1_workflows(workflow_key, workflow_type)
             VALUES ('42', 'privacy_probe')",
            "workflow_key",
        ),
        (
            "INSERT INTO steam.v1_operations(operation_id, operation_type, idempotency_key, payload_hash, payload)
             VALUES ('42', 'privacy_probe', 'probe:steam_operation', $1, '{}'::jsonb)",
            "operation_id",
        ),
        (
            "INSERT INTO steam.v1_deliveries(delivery_type, idempotency_key, remote_system, payload_hash, payload)
             VALUES ('privacy_probe', 'DeleteMe', 'discord', $1, '{}'::jsonb)",
            "delivery idempotency_key",
        ),
    ] {
        let result = sqlx::query(sql).bind(&hash).execute(pool).await;
        assert_eq!(
            database_error_code(&result).as_deref(),
            Some("23514"),
            "bare user-looking value must be rejected for {field}"
        );
    }
}

async fn assert_command_lease_claim_is_single_consumer(pool: &PgPool) {
    let hash = vec![9_u8; 32];
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.command_receipts(command_scope, idempotency_key, payload_hash, payload)
         VALUES ('claim', 'claim:once', $1, '{}'::jsonb)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert claimable command");

    let mut tx1 = pool.begin().await.expect("tx1");
    let claim_1_sql = claim_command_sql("worker:1");
    let claimed_1: Option<i64> = sqlx::query_scalar(&claim_1_sql)
        .fetch_optional(&mut *tx1)
        .await
        .expect("worker 1 claim");
    assert_eq!(claimed_1, Some(id));

    let mut tx2 = pool.begin().await.expect("tx2");
    let claim_2_sql = claim_command_sql("worker:2");
    let claimed_2: Option<i64> = sqlx::query_scalar(&claim_2_sql)
        .fetch_optional(&mut *tx2)
        .await
        .expect("worker 2 skip locked claim");
    assert_eq!(claimed_2, None);

    tx2.rollback().await.expect("rollback tx2");
    tx1.rollback().await.expect("rollback tx1");
}

fn claim_command_sql(worker: &str) -> String {
    format!(
        "UPDATE scrim.command_receipts
            SET state = 'processing',
                lease_owner = '{worker}',
                lease_until = now() + interval '1 minute'
          WHERE id = (
              SELECT id
                FROM scrim.command_receipts
               WHERE command_scope = 'claim'
                  AND idempotency_key = 'claim:once'
                 AND state = 'received'
               ORDER BY id
               FOR UPDATE SKIP LOCKED
               LIMIT 1
          )
          RETURNING id"
    )
}

async fn assert_result_refs_selection_supersede_and_delete_guards(pool: &PgPool) {
    seed_match(pool, 900_001, 900_002, 900_003).await;
    seed_match(pool, 900_004, 900_005, 900_006).await;

    let selected_id = insert_result_ref(pool, 900_003, 900_001, 900_011, true)
        .await
        .expect("insert selected valid result");
    let second_selected_id = insert_result_ref(pool, 900_003, 900_001, 900_012, true)
        .await
        .expect("selected-valid uniqueness is deferred until cutover index review");

    let legacy_only_selection: Option<i64> = sqlx::query_scalar(
        "SELECT result_ref_id
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .fetch_one(pool)
    .await
    .expect("selected view ignores legacy is_selected truth");
    assert_eq!(
        legacy_only_selection, None,
        "is_selected flags must not make a result canonical"
    );

    sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
         VALUES ($1, $2, '42', 'Tester', 'contract_test')",
    )
    .bind(900_003_i32)
    .bind(selected_id)
    .execute(pool)
    .await
    .expect("insert canonical result selection");

    let canonical_selection: Option<i64> = sqlx::query_scalar(
        "SELECT result_ref_id
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .fetch_one(pool)
    .await
    .expect("selected view uses selection table");
    assert_eq!(canonical_selection, Some(selected_id));

    let selection_event: (String, Option<i64>, Option<i64>, bool, bool) = sqlx::query_as(
        "SELECT event_type,
                old_result_ref_id,
                new_result_ref_id,
                after_data ? 'selected_by_user_id',
                after_data ? 'selected_by_display_name'
           FROM scrim.match_result_selection_events
          WHERE match_id = $1
           ORDER BY id DESC
          LIMIT 1",
    )
    .bind(900_003_i32)
    .fetch_one(pool)
    .await
    .expect("selection insert event");
    assert_eq!(
        selection_event,
        (
            "selected".to_string(),
            None,
            Some(selected_id),
            false,
            false
        )
    );

    let incomplete_reselection = sqlx::query(
        "UPDATE scrim.match_result_selections
             SET result_ref_id = $2
           WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .bind(second_selected_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&incomplete_reselection).as_deref(),
        Some("23514")
    );

    let actor_only_update = sqlx::query(
        "UPDATE scrim.match_result_selections
            SET selected_by_display_name = 'Other Label'
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&actor_only_update).as_deref(),
        Some("55000")
    );

    let timestamp_only_update = sqlx::query(
        "UPDATE scrim.match_result_selections
            SET selected_at = selected_at + interval '1 second'
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&timestamp_only_update).as_deref(),
        Some("55000")
    );

    let match_id_update = sqlx::query(
        "UPDATE scrim.match_result_selections
            SET match_id = $2
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .bind(900_006_i32)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&match_id_update).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "UPDATE scrim.match_result_selections
            SET result_ref_id = $2,
                selected_by_user_id = '43',
                selected_by_display_name = 'Reselector',
                selection_reason = 'corrected_result_source',
                selected_at = selected_at + interval '1 second'
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .bind(second_selected_id)
    .execute(pool)
    .await
    .expect("controlled reselection to another valid result ref");

    let reselected_canonical: Option<i64> = sqlx::query_scalar(
        "SELECT result_ref_id
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .fetch_one(pool)
    .await
    .expect("selected view follows reselection");
    assert_eq!(reselected_canonical, Some(second_selected_id));

    let reselection_event: (String, Option<i64>, Option<i64>, bool, bool) = sqlx::query_as(
        "SELECT event_type,
                old_result_ref_id,
                new_result_ref_id,
                before_data ? 'selected_by_user_id',
                after_data ? 'selected_by_user_id'
           FROM scrim.match_result_selection_events
          WHERE match_id = $1
          ORDER BY id DESC
          LIMIT 1",
    )
    .bind(900_003_i32)
    .fetch_one(pool)
    .await
    .expect("reselection event");
    assert_eq!(
        reselection_event,
        (
            "reselected".to_string(),
            Some(selected_id),
            Some(second_selected_id),
            false,
            false,
        )
    );

    sqlx::query(
        "WITH marker AS (
             INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
                 VALUES (43, true, now(), 'test', now())
             ON CONFLICT(user_id) DO UPDATE SET
                 opted_out = true,
                 deleted_at = excluded.deleted_at,
                 reason = excluded.reason,
                 updated_at = excluded.updated_at
             RETURNING user_id
         ),
         privacy AS (
             SELECT
                  set_config('scrim.privacy_erasure_user_id', '43', true),
                  set_config('scrim.privacy_erasure_target_ref', '43', true)
               FROM marker
         )
         UPDATE scrim.match_result_selections
             SET selected_by_user_id = 'redacted',
                 selected_by_display_name = 'redacted'
           FROM privacy
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .execute(pool)
    .await
    .expect("privacy redaction remains allowed for canonical selection actor");
    let redaction_event: (String, Option<i64>, Option<i64>, bool, bool, bool, bool) =
        sqlx::query_as(
            "SELECT event_type,
                old_result_ref_id,
                new_result_ref_id,
                before_data ? 'selected_by_user_id',
                before_data ? 'selected_by_display_name',
                after_data ? 'selected_by_user_id',
                after_data ? 'selected_by_display_name'
           FROM scrim.match_result_selection_events
          WHERE match_id = $1
          ORDER BY id DESC
          LIMIT 1",
        )
        .bind(900_003_i32)
        .fetch_one(pool)
        .await
        .expect("selection redaction event");
    assert_eq!(
        redaction_event,
        (
            "redacted".to_string(),
            Some(second_selected_id),
            Some(second_selected_id),
            false,
            false,
            false,
            false,
        )
    );

    let mutate_selection_event = sqlx::query(
        "UPDATE scrim.match_result_selection_events
            SET after_data = '{\"changed\":true}'::jsonb
          WHERE match_id = $1",
    )
    .bind(900_003_i32)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_selection_event).as_deref(),
        Some("55000")
    );

    let delete_selection =
        sqlx::query("DELETE FROM scrim.match_result_selections WHERE match_id = $1")
            .bind(900_003_i32)
            .execute(pool)
            .await;
    assert_eq!(
        database_error_code(&delete_selection).as_deref(),
        Some("55000")
    );

    let truncate_selection = sqlx::query("TRUNCATE scrim.match_result_selections")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_selection).as_deref(),
        Some("55000")
    );

    let delete_selection_event =
        sqlx::query("DELETE FROM scrim.match_result_selection_events WHERE match_id = $1")
            .bind(900_003_i32)
            .execute(pool)
            .await;
    assert_eq!(
        database_error_code(&delete_selection_event).as_deref(),
        Some("55000")
    );

    let truncate_selection_event = sqlx::query("TRUNCATE scrim.match_result_selection_events")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_selection_event).as_deref(),
        Some("55000")
    );

    let reject_selected_failed = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET fetch_status = 'failed'
          WHERE id = $1",
    )
    .bind(second_selected_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reject_selected_failed).as_deref(),
        Some("55000")
    );

    let reject_selected_rejected = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'rejected'
          WHERE id = $1",
    )
    .bind(second_selected_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reject_selected_rejected).as_deref(),
        Some("55000")
    );

    let reject_selected_void = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'void', voided_at = now()
          WHERE id = $1",
    )
    .bind(second_selected_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reject_selected_void).as_deref(),
        Some("55000")
    );

    let reject_selected_null_winner = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET winner_team_id = NULL
          WHERE id = $1",
    )
    .bind(second_selected_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reject_selected_null_winner).as_deref(),
        Some("55000")
    );

    let second_selection = sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'contract_test')",
    )
    .bind(900_003_i32)
    .bind(second_selected_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&second_selection).as_deref(),
        Some("23505")
    );

    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET is_selected = false,
                selected_at = NULL,
                selected_by_user_id = NULL
          WHERE id = $1",
    )
    .bind(second_selected_id)
    .execute(pool)
    .await
    .expect("clear extra selected fixture");

    let unselected_same = insert_result_ref(pool, 900_003, 900_001, 900_013, false)
        .await
        .expect("insert unselected same match result");
    let unselected_other = insert_result_ref(pool, 900_006, 900_004, 900_014, false)
        .await
        .expect("insert unselected other match result");

    let prose_result_selection_reason = sqlx::query(
        "INSERT INTO scrim.match_result_refs(
             match_id, steam_match_id, source_user_id, source_display_name,
             fetch_status, normalized_result_json, validation_status, selection_reason
         ) VALUES (
              $1, $2, '424242', 'Tester', 'fetched', '{}'::jsonb, 'valid', 'manual pick for DeleteMe 42'
         )",
    )
    .bind(900_006_i32)
    .bind(900_021_i64)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&prose_result_selection_reason).as_deref(),
        Some("23514")
    );

    let prose_result_void_reason = sqlx::query(
        "INSERT INTO scrim.match_result_refs(
             match_id, steam_match_id, source_user_id, source_display_name,
             fetch_status, normalized_result_json, validation_status, void_reason
         ) VALUES (
              $1, $2, '424242', 'Tester', 'fetched', '{}'::jsonb, 'valid', 'voided because user 42 asked'
         )",
    )
    .bind(900_006_i32)
    .bind(900_022_i64)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&prose_result_void_reason).as_deref(),
        Some("23514")
    );

    let prose_selection_reason = sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'corrected for DeleteMe 42')",
    )
    .bind(900_006_i32)
    .bind(unselected_other)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&prose_selection_reason).as_deref(),
        Some("23514")
    );

    let null_winner_ref: i64 = sqlx::query_scalar(
        r#"INSERT INTO scrim.match_result_refs(
             match_id,
             steam_match_id,
             source_user_id,
             source_display_name,
             fetch_status,
             normalized_result_json,
             validation_status
         )
         VALUES (
             $1,
             $2,
              '424242',
             'Tester',
             'fetched',
             '{"contract":true}'::jsonb,
             'valid'
          )
          RETURNING id"#,
    )
    .bind(900_006_i32)
    .bind(900_015_i64)
    .fetch_one(pool)
    .await
    .expect("insert valid fetched result ref without winner");

    let null_winner_selection = sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'contract_test')",
    )
    .bind(900_006_i32)
    .bind(null_winner_ref)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&null_winner_selection).as_deref(),
        Some("23514")
    );

    let reject_selected_superseded = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(second_selected_id)
    .bind(unselected_same)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reject_selected_superseded).as_deref(),
        Some("55000")
    );

    let cross_match_selection = sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'contract_test')",
    )
    .bind(900_006_i32)
    .bind(unselected_same)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&cross_match_selection).as_deref(),
        Some("23514")
    );

    let cross_match_supersede = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(unselected_same)
    .bind(unselected_other)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&cross_match_supersede).as_deref(),
        Some("23514")
    );

    let mutable_source = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET source_display_name = 'Still mutable for privacy redaction'
          WHERE id = $1",
    )
    .bind(unselected_same)
    .execute(pool)
    .await;
    assert!(
        mutable_source.is_ok(),
        "source actor fields must remain redactable by privacy erasure"
    );

    let mutate_match_identity = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET steam_match_id = 900099
          WHERE id = $1",
    )
    .bind(unselected_same)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_match_identity).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET is_selected = false,
                validation_status = 'superseded',
                superseded_by_ref_id = $2,
                selected_at = NULL,
                selected_by_user_id = NULL
          WHERE id = $1",
    )
    .bind(selected_id)
    .bind(unselected_same)
    .execute(pool)
    .await
    .expect("old selection ref may be superseded after reselection");

    let cycle_a = insert_result_ref(pool, 900_003, 900_001, 900_016, false)
        .await
        .expect("insert cycle ref a");
    let cycle_b = insert_result_ref(pool, 900_003, 900_001, 900_017, false)
        .await
        .expect("insert cycle ref b");
    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(cycle_a)
    .bind(cycle_b)
    .execute(pool)
    .await
    .expect("first supersession edge is allowed");
    let direct_cycle = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(cycle_b)
    .bind(cycle_a)
    .execute(pool)
    .await;
    assert_eq!(database_error_code(&direct_cycle).as_deref(), Some("23514"));

    let indirect_a = insert_result_ref(pool, 900_003, 900_001, 900_018, false)
        .await
        .expect("insert indirect cycle ref a");
    let indirect_b = insert_result_ref(pool, 900_003, 900_001, 900_019, false)
        .await
        .expect("insert indirect cycle ref b");
    let indirect_c = insert_result_ref(pool, 900_003, 900_001, 900_020, false)
        .await
        .expect("insert indirect cycle ref c");
    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(indirect_a)
    .bind(indirect_b)
    .execute(pool)
    .await
    .expect("first indirect supersession edge is allowed");
    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(indirect_b)
    .bind(indirect_c)
    .execute(pool)
    .await
    .expect("second indirect supersession edge is allowed");
    let indirect_cycle = sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(indirect_c)
    .bind(indirect_a)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&indirect_cycle).as_deref(),
        Some("23514")
    );

    seed_match(pool, 900_501, 900_502, 900_503).await;
    let race_a = insert_result_ref(pool, 900_503, 900_501, 900_511, false)
        .await
        .expect("insert race ref a");
    let race_b = insert_result_ref(pool, 900_503, 900_501, 900_512, false)
        .await
        .expect("insert race ref b");
    let mut first_supersession_tx = pool.begin().await.expect("first supersession tx");
    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'superseded', superseded_by_ref_id = $2
          WHERE id = $1",
    )
    .bind(race_a)
    .bind(race_b)
    .execute(&mut *first_supersession_tx)
    .await
    .expect("first racing supersession edge holds match graph lock");
    let race_pool = pool.clone();
    let competing_supersession = tokio::spawn(async move {
        let mut tx = race_pool.begin().await.expect("competing supersession tx");
        let update = sqlx::query(
            "UPDATE scrim.match_result_refs
                SET validation_status = 'superseded', superseded_by_ref_id = $2
              WHERE id = $1",
        )
        .bind(race_b)
        .bind(race_a)
        .execute(&mut *tx)
        .await;

        if let Err(err) = update {
            let _ = tx.rollback().await;
            return Err(err);
        }

        tx.commit().await
    });
    wait_for_db_lock(pool, "UPDATE scrim.match_result_refs").await;
    first_supersession_tx
        .commit()
        .await
        .expect("commit first racing supersession edge");
    let competing_result = competing_supersession
        .await
        .expect("competing supersession task");
    assert_eq!(
        database_error_code(&competing_result).as_deref(),
        Some("23514")
    );
    let committed_edges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM scrim.match_result_refs
          WHERE id IN ($1, $2)
            AND superseded_by_ref_id IS NOT NULL",
    )
    .bind(race_a)
    .bind(race_b)
    .fetch_one(pool)
    .await
    .expect("count committed racing supersession edges");
    assert_eq!(committed_edges, 1, "at most one racing edge may commit");
    let graph_is_acyclic: bool = sqlx::query_scalar(
        "WITH RECURSIVE supersede_graph(root_id, id, superseded_by_ref_id, path, cycle) AS (
             SELECT id, id, superseded_by_ref_id, ARRAY[id], false
               FROM scrim.match_result_refs
              WHERE match_id = $1
                AND superseded_by_ref_id IS NOT NULL
             UNION ALL
             SELECT graph.root_id,
                    next_ref.id,
                    next_ref.superseded_by_ref_id,
                    graph.path || next_ref.id,
                    next_ref.id = ANY(graph.path)
               FROM supersede_graph AS graph
               JOIN scrim.match_result_refs AS next_ref
                 ON next_ref.id = graph.superseded_by_ref_id
              WHERE graph.superseded_by_ref_id IS NOT NULL
                AND NOT graph.cycle
         )
         SELECT NOT EXISTS(SELECT 1 FROM supersede_graph WHERE cycle)",
    )
    .bind(900_503_i32)
    .fetch_one(pool)
    .await
    .expect("final supersession graph acyclic");
    assert!(
        graph_is_acyclic,
        "racing supersessions must not leave a cycle"
    );

    seed_match(pool, 900_101, 900_102, 900_103).await;
    let defensive_view_ref = insert_result_ref(pool, 900_103, 900_101, 900_111, false)
        .await
        .expect("insert defensive view result ref");
    sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'contract_test')",
    )
    .bind(900_103_i32)
    .bind(defensive_view_ref)
    .execute(pool)
    .await
    .expect("insert defensive view selection");
    sqlx::query(
        "ALTER TABLE scrim.match_result_refs DISABLE TRIGGER prevent_selected_match_result_ref_invalidation",
    )
    .execute(pool)
    .await
    .expect("temporarily disable selected-ref invalidation trigger");
    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'rejected'
          WHERE id = $1",
    )
    .bind(defensive_view_ref)
    .execute(pool)
    .await
    .expect("simulate preexisting invalid selected ref for defensive view");
    sqlx::query(
        "ALTER TABLE scrim.match_result_refs ENABLE TRIGGER prevent_selected_match_result_ref_invalidation",
    )
    .execute(pool)
    .await
    .expect("re-enable selected-ref invalidation trigger");
    let defensive_view_selection: (
        Option<i64>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT result_ref_id, selected_at, selected_by_user_id
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(900_103_i32)
    .fetch_one(pool)
    .await
    .expect("defensive selected view filters invalid selected refs");
    assert_eq!(defensive_view_selection, (None, None, None));

    seed_match(pool, 900_201, 900_202, 900_203).await;
    let defensive_null_winner_ref = insert_result_ref(pool, 900_203, 900_201, 900_211, false)
        .await
        .expect("insert defensive null-winner result ref");
    sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'contract_test')",
    )
    .bind(900_203_i32)
    .bind(defensive_null_winner_ref)
    .execute(pool)
    .await
    .expect("insert defensive null-winner selection");
    sqlx::query(
        "ALTER TABLE scrim.match_result_refs DISABLE TRIGGER prevent_selected_match_result_ref_invalidation",
    )
    .execute(pool)
    .await
    .expect("temporarily disable selected-ref invalidation trigger");
    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET winner_team_id = NULL
          WHERE id = $1",
    )
    .bind(defensive_null_winner_ref)
    .execute(pool)
    .await
    .expect("simulate preexisting selected ref without winner for defensive view");
    sqlx::query(
        "ALTER TABLE scrim.match_result_refs ENABLE TRIGGER prevent_selected_match_result_ref_invalidation",
    )
    .execute(pool)
    .await
    .expect("re-enable selected-ref invalidation trigger");
    let defensive_null_winner_view: (
        Option<i64>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT result_ref_id, selected_at, selected_by_user_id
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(900_203_i32)
    .fetch_one(pool)
    .await
    .expect("defensive selected view filters null-winner selected refs");
    assert_eq!(defensive_null_winner_view, (None, None, None));

    seed_match(pool, 900_301, 900_302, 900_303).await;
    let locked_selection_ref = insert_result_ref(pool, 900_303, 900_301, 900_311, false)
        .await
        .expect("insert concurrency result ref");
    let mut selection_tx = pool.begin().await.expect("selection tx");
    sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'concurrency_lock')",
    )
    .bind(900_303_i32)
    .bind(locked_selection_ref)
    .execute(&mut *selection_tx)
    .await
    .expect("insert selection while holding referenced result lock");
    let update_pool = pool.clone();
    let invalidation = tokio::spawn(async move {
        sqlx::query(
            "UPDATE scrim.match_result_refs
                SET validation_status = 'rejected'
              WHERE id = $1",
        )
        .bind(locked_selection_ref)
        .execute(&update_pool)
        .await
    });
    wait_for_db_lock(pool, "UPDATE scrim.match_result_refs").await;
    selection_tx.commit().await.expect("commit selection tx");
    let invalidation_result = invalidation.await.expect("invalidation task");
    assert_eq!(
        database_error_code(&invalidation_result).as_deref(),
        Some("55000")
    );

    seed_match(pool, 900_401, 900_402, 900_403).await;
    let old_reselection_ref = insert_result_ref(pool, 900_403, 900_401, 900_411, false)
        .await
        .expect("insert old reselection ref");
    let new_reselection_ref = insert_result_ref(pool, 900_403, 900_401, 900_412, false)
        .await
        .expect("insert new reselection ref");
    sqlx::query(
        "INSERT INTO scrim.match_result_selections(
             match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
         )
          VALUES ($1, $2, '424242', 'Tester', 'initial_concurrency_selection')",
    )
    .bind(900_403_i32)
    .bind(old_reselection_ref)
    .execute(pool)
    .await
    .expect("insert reselection concurrency baseline");
    let mut reselection_tx = pool.begin().await.expect("reselection tx");
    sqlx::query(
        "UPDATE scrim.match_result_selections
            SET result_ref_id = $2,
                selected_by_user_id = '44',
                selected_by_display_name = 'Concurrent Reselector',
                selection_reason = 'concurrent_corrected_result',
                selected_at = selected_at + interval '1 second'
          WHERE match_id = $1",
    )
    .bind(900_403_i32)
    .bind(new_reselection_ref)
    .execute(&mut *reselection_tx)
    .await
    .expect("reselection locks new result ref");
    let invalidation_pool = pool.clone();
    let current_invalidation = tokio::spawn(async move {
        sqlx::query(
            "UPDATE scrim.match_result_refs
                SET validation_status = 'rejected'
              WHERE id = $1",
        )
        .bind(new_reselection_ref)
        .execute(&invalidation_pool)
        .await
    });
    wait_for_db_lock(pool, "UPDATE scrim.match_result_refs").await;
    reselection_tx
        .commit()
        .await
        .expect("commit reselection tx");
    let current_invalidation_result = current_invalidation
        .await
        .expect("current invalidation task");
    assert_eq!(
        database_error_code(&current_invalidation_result).as_deref(),
        Some("55000")
    );
    let reselected_after_race: Option<i64> = sqlx::query_scalar(
        "SELECT result_ref_id
           FROM scrim.selected_match_results
          WHERE match_id = $1",
    )
    .bind(900_403_i32)
    .fetch_one(pool)
    .await
    .expect("reselection survives concurrent invalidation");
    assert_eq!(reselected_after_race, Some(new_reselection_ref));
    sqlx::query(
        "UPDATE scrim.match_result_refs
            SET validation_status = 'void', voided_at = now()
          WHERE id = $1",
    )
    .bind(old_reselection_ref)
    .execute(pool)
    .await
    .expect("old ref may be voided after reselection commits");

    let parent_delete = sqlx::query("DELETE FROM scrim.matches WHERE id = $1")
        .bind(900_003_i32)
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&parent_delete).as_deref(),
        Some("23503")
    );

    let hard_delete = sqlx::query("DELETE FROM scrim.match_result_refs WHERE id = $1")
        .bind(unselected_same)
        .execute(pool)
        .await;
    assert_eq!(database_error_code(&hard_delete).as_deref(), Some("55000"));

    let truncate = sqlx::query("TRUNCATE scrim.match_result_refs CASCADE")
        .execute(pool)
        .await;
    assert_eq!(database_error_code(&truncate).as_deref(), Some("55000"));
}

async fn assert_match_result_ref_constraints_are_validated(pool: &PgPool) {
    let invalid_constraints: Vec<String> = sqlx::query_scalar(
        "SELECT conname
           FROM pg_constraint
          WHERE conrelid = 'scrim.match_result_refs'::regclass
            AND conname = ANY($1)
            AND NOT convalidated
          ORDER BY conname",
    )
    .bind([
        "match_result_refs_match_id_fkey",
        "match_result_refs_validation_status_check",
        "match_result_refs_clarification_state_check",
        "match_result_refs_selected_valid_check",
        "match_result_refs_void_status_check",
        "match_result_refs_supersede_status_check",
        "match_result_refs_supersede_not_self_check",
        "match_result_refs_superseded_ref_fkey",
        "match_result_refs_fetch_validation_status_check",
    ])
    .fetch_all(pool)
    .await
    .expect("validated result ref constraints");
    assert!(
        invalid_constraints.is_empty(),
        "result ref constraints must be validated: {invalid_constraints:?}"
    );

    let removed_constraints: Vec<String> = sqlx::query_scalar(
        "SELECT conname
           FROM pg_constraint
          WHERE conrelid = 'scrim.match_result_refs'::regclass
            AND conname = ANY($1)
          ORDER BY conname",
    )
    .bind([
        "match_result_refs_match_id_id_key",
        "match_result_refs_superseded_same_match_fkey",
    ])
    .fetch_all(pool)
    .await
    .expect("removed result ref constraints");
    assert!(
        removed_constraints.is_empty(),
        "live result ref table must not keep removed composite constraints: {removed_constraints:?}"
    );

    let live_result_ref_indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexname
           FROM pg_indexes
          WHERE schemaname = 'scrim'
            AND tablename = 'match_result_refs'
            AND indexname = ANY($1)
          ORDER BY indexname",
    )
    .bind([
        "match_result_refs_one_selected_valid_uidx",
        "match_result_refs_validation_idx",
    ])
    .fetch_all(pool)
    .await
    .expect("deferred result ref indexes");
    assert!(
        live_result_ref_indexes.is_empty(),
        "nonessential live result-ref indexes must be deferred until cutover: {live_result_ref_indexes:?}"
    );
}

async fn assert_replacement_request_candidate_need_consistency(pool: &PgPool) {
    let need_1 = insert_replacement_need(pool, 910_001).await;
    let need_2 = insert_replacement_need(pool, 910_002).await;
    let missing_identity = sqlx::query(
        "INSERT INTO scrim.replacement_candidates(need_id, candidate_data, score_data)
         VALUES ($1, '{}'::jsonb, '{}'::jsonb)",
    )
    .bind(need_1)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&missing_identity).as_deref(),
        Some("23514")
    );

    let candidate_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.replacement_candidates(need_id, discord_user_id, candidate_data, score_data)
         VALUES ($1, 4242, '{}'::jsonb, '{}'::jsonb)
         RETURNING id",
    )
    .bind(need_1)
    .fetch_one(pool)
    .await
    .expect("insert replacement candidate");

    let mismatch = sqlx::query(
        "INSERT INTO scrim.replacement_requests(
             need_id, candidate_id, requested_by_user_id, requested_by_display_name
         )
          VALUES ($1, $2, '424242', 'Tester')",
    )
    .bind(need_2)
    .bind(candidate_id)
    .execute(pool)
    .await;
    assert_eq!(database_error_code(&mismatch).as_deref(), Some("23503"));

    sqlx::query(
        "INSERT INTO scrim.replacement_requests(
             need_id, candidate_id, requested_by_user_id, requested_by_display_name
         )
          VALUES ($1, $2, '424242', 'Tester')",
    )
    .bind(need_1)
    .bind(candidate_id)
    .execute(pool)
    .await
    .expect("matching replacement request is allowed");
}

async fn assert_scrim_request_workflow_constraints_and_links(pool: &PgPool) {
    assert!(table_columns(pool, "scrim", "replacement_needs")
        .await
        .contains(&"match_request_id".to_string()));
    assert_eq!(
        table_columns(pool, "scrim", "match_request_reminder_effects").await,
        vec![
            "reminder_id",
            "outbox_effect_id",
            "created_at",
            "reconciled_at"
        ]
    );
    assert_eq!(
        table_columns(pool, "scrim", "status_publication_effects").await,
        vec![
            "status_publication_approval_id",
            "outbox_effect_id",
            "created_at",
            "reconciled_at"
        ]
    );
    assert_eq!(
        table_columns(pool, "scrim", "replacement_request_effects").await,
        vec![
            "replacement_request_id",
            "outbox_effect_id",
            "created_at",
            "reconciled_at"
        ]
    );
    let unreconciled_indexes: Vec<(String, String, Vec<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT source_table.relname::TEXT,
               index_class.relname::TEXT,
               array_agg(att.attname::TEXT ORDER BY key_columns.ord),
               pg_get_expr(idx.indpred, idx.indrelid)
          FROM pg_index idx
          JOIN pg_class index_class ON index_class.oid = idx.indexrelid
          JOIN pg_class source_table ON source_table.oid = idx.indrelid
          JOIN pg_namespace source_ns ON source_ns.oid = source_table.relnamespace
          JOIN unnest(idx.indkey) WITH ORDINALITY AS key_columns(attnum, ord) ON true
          JOIN pg_attribute att ON att.attrelid = source_table.oid
                               AND att.attnum = key_columns.attnum
         WHERE source_ns.nspname = 'scrim'
           AND index_class.relname IN (
               'match_request_reminder_effects_unreconciled_outbox_idx',
               'status_publication_effects_unreconciled_outbox_idx',
               'replacement_request_effects_unreconciled_outbox_idx'
           )
         GROUP BY source_table.relname, index_class.relname, idx.indpred, idx.indrelid
         ORDER BY source_table.relname, index_class.relname
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("unreconciled outbox index contracts");
    assert_eq!(
        unreconciled_indexes,
        vec![
            (
                "match_request_reminder_effects".to_string(),
                "match_request_reminder_effects_unreconciled_outbox_idx".to_string(),
                vec!["outbox_effect_id".to_string()],
                Some("(reconciled_at IS NULL)".to_string()),
            ),
            (
                "replacement_request_effects".to_string(),
                "replacement_request_effects_unreconciled_outbox_idx".to_string(),
                vec!["outbox_effect_id".to_string()],
                Some("(reconciled_at IS NULL)".to_string()),
            ),
            (
                "status_publication_effects".to_string(),
                "status_publication_effects_unreconciled_outbox_idx".to_string(),
                vec!["outbox_effect_id".to_string()],
                Some("(reconciled_at IS NULL)".to_string()),
            ),
        ]
    );
    let replacement_need_predicate: Option<String> = sqlx::query_scalar(
        r#"
        SELECT pg_get_expr(idx.indpred, idx.indrelid)
          FROM pg_index idx
          JOIN pg_class index_class ON index_class.oid = idx.indexrelid
          JOIN pg_namespace ns ON ns.oid = index_class.relnamespace
         WHERE ns.nspname = 'scrim'
           AND index_class.relname = 'replacement_needs_match_request_slot_uidx'
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("replacement need partial unique predicate");
    assert_eq!(
        replacement_need_predicate.as_deref(),
        Some("((match_request_id IS NOT NULL) AND (team_id IS NOT NULL) AND (participant_id IS NOT NULL) AND (slot_index IS NOT NULL))")
    );
    let link_table_fks: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT source_table.relname::TEXT,
               source_column.attname::TEXT,
               referenced_table.relname::TEXT
          FROM pg_constraint con
          JOIN pg_class source_table ON source_table.oid = con.conrelid
          JOIN pg_namespace source_ns ON source_ns.oid = source_table.relnamespace
          JOIN pg_class referenced_table ON referenced_table.oid = con.confrelid
          JOIN pg_attribute source_column ON source_column.attrelid = con.conrelid
                                         AND source_column.attnum = con.conkey[1]
         WHERE source_ns.nspname = 'scrim'
           AND source_table.relname IN (
               'match_request_reminder_effects',
               'status_publication_effects',
               'replacement_request_effects'
           )
           AND con.contype = 'f'
           AND array_length(con.conkey, 1) = 1
         ORDER BY source_table.relname, source_column.attname
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("outbox link table fks");
    assert_eq!(
        link_table_fks,
        vec![
            (
                "match_request_reminder_effects".to_string(),
                "outbox_effect_id".to_string(),
                "outbox_effects".to_string(),
            ),
            (
                "match_request_reminder_effects".to_string(),
                "reminder_id".to_string(),
                "match_request_reminders".to_string(),
            ),
            (
                "replacement_request_effects".to_string(),
                "outbox_effect_id".to_string(),
                "outbox_effects".to_string(),
            ),
            (
                "replacement_request_effects".to_string(),
                "replacement_request_id".to_string(),
                "replacement_requests".to_string(),
            ),
            (
                "status_publication_effects".to_string(),
                "outbox_effect_id".to_string(),
                "outbox_effects".to_string(),
            ),
            (
                "status_publication_effects".to_string(),
                "status_publication_approval_id".to_string(),
                "status_publication_approvals".to_string(),
            ),
        ]
    );

    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES (930001, 'Workflow A', now()), (930002, 'Workflow B', now())")
        .execute(pool)
        .await
        .expect("insert workflow teams");
    sqlx::query(
        "INSERT INTO scrim.participants(id, discord_id, display_name, rank_source, status, source, created_at, updated_at)
         VALUES (930101, 930101, 'Workflow User', 'manual', 'assigned', 'test', now(), now()),
                (930102, 930102, 'Workflow Other', 'manual', 'assigned', 'test', now(), now())",
    )
    .execute(pool)
    .await
    .expect("insert workflow participants");
    sqlx::query(
        "INSERT INTO scrim.match_request_batches(id, template, deadline_at, status, created_by_user_id, created_by_display_name)
         VALUES (930010, 'regular_scrim', now() + interval '1 day', 'open', '424242', 'Tester')",
    )
    .execute(pool)
    .await
    .expect("insert workflow batch");
    sqlx::query(
        r#"INSERT INTO scrim.match_requests(id, batch_id, team_a_id, team_b_id, status, slot_options)
         VALUES (930011, 930010, 930001, 930002, 'open', '[{"day":"sat","from":1200,"to":1320}]'::jsonb)"#,
    )
    .execute(pool)
    .await
    .expect("insert workflow request");

    let missing_slot = sqlx::query(
        "INSERT INTO scrim.replacement_needs(
             match_request_id, team_id, participant_id, reason, created_by_user_id, created_by_display_name
         )
         VALUES (930011, 930001, 930101, 'contract_test', '424242', 'Tester')",
    )
    .execute(pool)
    .await;
    assert_eq!(database_error_code(&missing_slot).as_deref(), Some("23514"));

    let need_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.replacement_needs(
             match_request_id, slot_index, team_id, participant_id, reason, created_by_user_id, created_by_display_name
         )
          VALUES (930011, 0, 930001, 930101, 'contract_test', '424242', 'Tester')
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert workflow replacement need");
    let duplicate_need = sqlx::query(
        "INSERT INTO scrim.replacement_needs(
             match_request_id, slot_index, team_id, participant_id, reason, created_by_user_id, created_by_display_name
         )
          VALUES (930011, 0, 930001, 930101, 'contract_test', '424242', 'Tester')",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&duplicate_need).as_deref(),
        Some("23505")
    );

    sqlx::query(
        "INSERT INTO scrim.replacement_candidates(need_id, participant_id, candidate_data, score_data, status)
         VALUES ($1, 930101, '{}'::jsonb, '{}'::jsonb, 'selected')",
    )
    .bind(need_id)
    .execute(pool)
    .await
    .expect("first selected candidate");
    let second_selected = sqlx::query(
        "INSERT INTO scrim.replacement_candidates(need_id, participant_id, candidate_data, score_data, status)
         VALUES ($1, 930102, '{}'::jsonb, '{}'::jsonb, 'selected')",
    )
    .bind(need_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&second_selected).as_deref(),
        Some("23505")
    );
    sqlx::query(
        "INSERT INTO scrim.replacement_candidates(need_id, discord_user_id, candidate_data, score_data, status)
         VALUES ($1, 930201, '{}'::jsonb, '{}'::jsonb, 'accepted'),
                ($1, 930202, '{}'::jsonb, '{}'::jsonb, 'accepted')",
    )
    .bind(need_id)
    .execute(pool)
    .await
    .expect("multiple accepted candidates stay allowed");

    sqlx::query(
        "INSERT INTO scrim.replacement_requests(need_id, discord_user_id, status, requested_by_user_id, requested_by_display_name)
         VALUES ($1, 930301, 'uncertain', '424242', 'Tester')",
    )
    .bind(need_id)
    .execute(pool)
    .await
    .expect("uncertain request without response time");
    sqlx::query(
        "INSERT INTO scrim.replacement_requests(need_id, discord_user_id, status, requested_by_user_id, requested_by_display_name)
         VALUES ($1, 930304, 'cancelled', '424242', 'Tester')",
    )
    .bind(need_id)
    .execute(pool)
    .await
    .expect("cancelled request without response time");
    sqlx::query(
        "INSERT INTO scrim.replacement_requests(need_id, discord_user_id, status, responded_at, requested_by_user_id, requested_by_display_name)
         VALUES ($1, 930305, 'cancelled', now(), '424242', 'Tester')",
    )
    .bind(need_id)
    .execute(pool)
    .await
    .expect("cancelled request after response remains legal");
    sqlx::query(
        "INSERT INTO scrim.replacement_requests(need_id, discord_user_id, status, responded_at, requested_by_user_id, requested_by_display_name)
         VALUES ($1, 930306, 'accepted', now(), '424242', 'Tester'),
                ($1, 930307, 'accepted', now(), '424242', 'Tester')",
    )
    .bind(need_id)
    .execute(pool)
    .await
    .expect("multiple accepted replacement requests stay legal before coach selection");
    let accepted_without_response_time = sqlx::query(
        "INSERT INTO scrim.replacement_requests(need_id, discord_user_id, status, requested_by_user_id, requested_by_display_name)
         VALUES ($1, 930308, 'accepted', '424242', 'Tester')",
    )
    .bind(need_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&accepted_without_response_time).as_deref(),
        Some("23514")
    );
    let selected_without_response_time = sqlx::query(
        "INSERT INTO scrim.replacement_requests(need_id, discord_user_id, status, requested_by_user_id, requested_by_display_name)
         VALUES ($1, 930302, 'selected', '424242', 'Tester')",
    )
    .bind(need_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&selected_without_response_time).as_deref(),
        Some("23514")
    );
    let replacement_request_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.replacement_requests(need_id, discord_user_id, status, responded_at, requested_by_user_id, requested_by_display_name)
         VALUES ($1, 930303, 'selected', now(), '424242', 'Tester')
         RETURNING id",
    )
    .bind(need_id)
    .fetch_one(pool)
    .await
    .expect("selected request with response time");

    sqlx::query(
        "INSERT INTO scrim.match_request_reminders(
             id, request_id, team_id, target_kind, missing_count, approved_by_user_id,
             approved_by_display_name, discord_channel_id, source_message_id, status
         )
          VALUES (930080, 930011, 930001, 'team', 1, '424242', 'Tester', 930400, 930401, 'queued'),
                 (930081, 930011, 930001, 'team', 1, '424242', 'Tester', 930400, 930401, 'uncertain')",
    )
    .execute(pool)
    .await
    .expect("queued and uncertain reminder states");
    sqlx::query(
        "UPDATE scrim.match_requests
            SET status_message_state = 'failed'
          WHERE id = 930011",
    )
    .execute(pool)
    .await
    .expect("failed status message state");
    sqlx::query(
        "UPDATE scrim.match_requests
            SET status_message_state = 'uncertain'
          WHERE id = 930011",
    )
    .execute(pool)
    .await
    .expect("uncertain status message state");

    let hash = vec![7_u8; 32];
    let reminder_effect_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload)
         VALUES ('discord_scrim_effect', 'effect:reminder', $1, '{}'::jsonb)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert reminder effect");
    sqlx::query(
        "INSERT INTO scrim.match_request_reminder_effects(reminder_id, outbox_effect_id)
         VALUES (930080, $1)",
    )
    .bind(reminder_effect_id)
    .execute(pool)
    .await
    .expect("link reminder effect");
    let reminder_reconciled_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT reconciled_at
           FROM scrim.match_request_reminder_effects
          WHERE reminder_id = 930080
            AND outbox_effect_id = $1",
    )
    .bind(reminder_effect_id)
    .fetch_one(pool)
    .await
    .expect("reminder effect reconciled_at default");
    assert_eq!(reminder_reconciled_at, None);
    let duplicate_reminder_link = sqlx::query(
        "INSERT INTO scrim.match_request_reminder_effects(reminder_id, outbox_effect_id)
         VALUES (930080, $1)",
    )
    .bind(reminder_effect_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&duplicate_reminder_link).as_deref(),
        Some("23505")
    );

    let status_publication_approval_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.status_publication_approvals(
             target_kind, target_id, status_kind, payload, payload_hash,
             decision, decided_by_user_id, decided_by_display_name, decided_at
         )
         VALUES (
             'match_request', '930011', 'match_status', '{}'::jsonb,
             scrim.status_publication_effect_hash('match_request', '930011', 'match_status', '{}'::jsonb),
             'approved', '424242', 'Tester', now()
         )
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert status publication approval");
    sqlx::query(
        "UPDATE scrim.status_publication_approvals
            SET decision = 'uncertain', decided_at = now()
          WHERE id = $1",
    )
    .bind(status_publication_approval_id)
    .execute(pool)
    .await
    .expect("status publication approval can become uncertain");
    let failed_status_publication_approval_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.status_publication_approvals(
             target_kind, target_id, status_kind, payload, payload_hash,
             decision, decided_by_user_id, decided_by_display_name, decided_at
         )
         VALUES (
             'match_request', '930011', 'match_status_retry', '{}'::jsonb,
             scrim.status_publication_effect_hash('match_request', '930011', 'match_status_retry', '{}'::jsonb),
             'failed', '424242', 'Tester', now()
         )
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("status publication approval can represent failed delivery");
    assert!(failed_status_publication_approval_id > 0);
    let status_effect_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload)
         VALUES ('discord_scrim_effect', 'effect:status', $1, '{}'::jsonb)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert status effect");
    sqlx::query(
        "INSERT INTO scrim.status_publication_effects(status_publication_approval_id, outbox_effect_id)
         VALUES ($1, $2)",
    )
    .bind(status_publication_approval_id)
    .bind(status_effect_id)
    .execute(pool)
    .await
    .expect("link status publication effect");
    let status_reconciled_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT reconciled_at
           FROM scrim.status_publication_effects
          WHERE status_publication_approval_id = $1
            AND outbox_effect_id = $2",
    )
    .bind(status_publication_approval_id)
    .bind(status_effect_id)
    .fetch_one(pool)
    .await
    .expect("status publication effect reconciled_at default");
    assert_eq!(status_reconciled_at, None);

    let replacement_effect_id: i64 = sqlx::query_scalar(
        "INSERT INTO scrim.outbox_effects(effect_type, idempotency_key, payload_hash, payload)
         VALUES ('discord_scrim_effect', 'effect:replacement', $1, '{}'::jsonb)
         RETURNING id",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert replacement effect");
    sqlx::query(
        "INSERT INTO scrim.replacement_request_effects(replacement_request_id, outbox_effect_id)
         VALUES ($1, $2)",
    )
    .bind(replacement_request_id)
    .bind(replacement_effect_id)
    .execute(pool)
    .await
    .expect("link replacement effect");
    let replacement_reconciled_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "UPDATE scrim.replacement_request_effects
            SET reconciled_at = now()
          WHERE replacement_request_id = $1
            AND outbox_effect_id = $2
      RETURNING reconciled_at",
    )
    .bind(replacement_request_id)
    .bind(replacement_effect_id)
    .fetch_one(pool)
    .await
    .expect("replacement request effect reconciled_at can be set");
    assert!(replacement_reconciled_at.is_some());
}

async fn assert_steam_v1_idempotency_leases_and_result_guards(pool: &PgPool) {
    let hash = vec![2_u8; 32];
    assert_eq!(
        column_data_type(pool, "steam", "v1_workflows", "workflow_generation").await,
        "bigint"
    );
    assert_eq!(
        column_data_type(pool, "steam", "v1_operations", "idempotency_generation").await,
        "bigint"
    );
    assert_eq!(
        column_data_type(pool, "steam", "v1_deliveries", "idempotency_generation").await,
        "bigint"
    );

    let invalid_aggregate_kind = sqlx::query(
        "INSERT INTO steam.v1_workflows(workflow_key, workflow_type, aggregate_kind, aggregate_id)
         VALUES ('wf:invalid_kind', 'friend_request', 'discord_user', '42')",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&invalid_aggregate_kind).as_deref(),
        Some("23514")
    );
    let invalid_subject_kind = sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_type, idempotency_key, payload_hash, subject_kind, subject_id, payload
         )
          VALUES ('friend_request', 'invalid:subject_kind', $1, 'match', '42', '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&invalid_subject_kind).as_deref(),
        Some("23514")
    );

    sqlx::query("INSERT INTO core.users(discord_id) VALUES (42) ON CONFLICT DO NOTHING")
        .execute(pool)
        .await
        .expect("core user for steam v1 privacy");
    sqlx::query(
        "INSERT INTO core.steam_links(discord_id, steam_id, steam_id64)
         VALUES (42, '7656119800000042', 7656119800000042)
         ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("steam link for steam v1 privacy");

    sqlx::query(
        "INSERT INTO steam.v1_workflows(
             workflow_key, workflow_type, workflow_generation, aggregate_kind, aggregate_id,
             subject_kind, subject_id, state, finished_at
          )
          VALUES (
             'wf:generation', 'friend_request', 2147483648, 'player', '7656119800000042',
             'discord_user', '42', 'succeeded', now()
          )",
    )
    .execute(pool)
    .await
    .expect("insert terminal workflow with bigint generation");
    let duplicate_workflow_generation = sqlx::query(
        "INSERT INTO steam.v1_workflows(
             workflow_key, workflow_type, workflow_generation, aggregate_kind, aggregate_id,
             subject_kind, subject_id, state, finished_at
          )
          VALUES (
             'wf:generation', 'friend_request', 2147483648, 'player', '7656119800000042',
             'discord_user', '42', 'succeeded', now()
          )",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&duplicate_workflow_generation).as_deref(),
        Some("23505")
    );
    sqlx::query(
        "INSERT INTO steam.v1_workflows(
             workflow_key, workflow_type, workflow_generation, aggregate_kind, aggregate_id,
             subject_kind, subject_id
          )
          VALUES (
             'wf:generation', 'friend_request', 2147483649, 'player', '7656119800000042',
             'discord_user', '42'
          )",
    )
    .execute(pool)
    .await
    .expect("workflow can advance generation after terminal state");
    let mutate_workflow_identity = sqlx::query(
        "UPDATE steam.v1_workflows
            SET workflow_type = 'mutated'
          WHERE workflow_key = 'wf:generation'
            AND workflow_generation = 2147483649",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&mutate_workflow_identity).as_deref(),
        Some("55000")
    );
    sqlx::query(
        "WITH marker AS (
             INSERT INTO core.user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
             VALUES (42, true, now(), 'test', now())
             ON CONFLICT(user_id) DO UPDATE SET
                 opted_out = true,
                 deleted_at = excluded.deleted_at,
                 reason = excluded.reason,
                 updated_at = excluded.updated_at
             RETURNING user_id
         ),
         privacy AS (
             SELECT
                 set_config('scrim.privacy_erasure_user_id', '42', true),
                 set_config('scrim.privacy_erasure_target_ref', '7656119800000042', true)
               FROM marker
          )
          UPDATE steam.v1_workflows
             SET aggregate_id = 'redacted'
            FROM privacy
           WHERE workflow_key = 'wf:generation'
            AND workflow_generation = 2147483649",
    )
    .execute(pool)
    .await
    .expect("privacy redaction remains allowed for workflow aggregate id");
    sqlx::query(
        "INSERT INTO steam.v1_workflows(workflow_key, workflow_type, aggregate_kind, aggregate_id)
         VALUES ('wf:kind_collision', 'friend_request', 'match', '42')",
    )
    .execute(pool)
    .await
    .expect("insert aggregate kind collision workflow");
    let redact_match_aggregate = sqlx::query(
        "WITH privacy AS (
             SELECT
                 set_config('scrim.privacy_erasure_user_id', '42', true),
                 set_config('scrim.privacy_erasure_target_ref', '42', true)
         )
         UPDATE steam.v1_workflows
            SET aggregate_id = 'redacted'
           FROM privacy
          WHERE workflow_key = 'wf:kind_collision'",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&redact_match_aggregate).as_deref(),
        Some("55000"),
        "aggregate_kind=match must not redact as Discord user 42"
    );
    let second_active_workflow = sqlx::query(
        "INSERT INTO steam.v1_workflows(
             workflow_key, workflow_type, workflow_generation, aggregate_kind, aggregate_id
          )
           VALUES ('wf:generation', 'friend_request', 2147483650, 'player', '7656119800000042')",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&second_active_workflow).as_deref(),
        Some("23505")
    );
    sqlx::query(
        "UPDATE steam.v1_workflows
            SET state = 'succeeded', finished_at = now()
          WHERE workflow_key = 'wf:generation'
            AND workflow_generation = 2147483649",
    )
    .execute(pool)
    .await
    .expect("mark workflow terminal");
    let reopen_terminal_workflow = sqlx::query(
        "UPDATE steam.v1_workflows
            SET state = 'open', finished_at = NULL
          WHERE workflow_key = 'wf:generation'
            AND workflow_generation = 2147483649",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&reopen_terminal_workflow).as_deref(),
        Some("55000")
    );
    let delete_workflow = sqlx::query(
        "DELETE FROM steam.v1_workflows
          WHERE workflow_key = 'wf:generation'
            AND workflow_generation = 2147483649",
    )
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&delete_workflow).as_deref(),
        Some("55000")
    );
    let truncate_workflow = sqlx::query("TRUNCATE steam.v1_workflows CASCADE")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_workflow).as_deref(),
        Some("55000")
    );

    sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_type, workflow_key, idempotency_key, payload_hash, payload, state, finished_at
         )
          VALUES ('friend_request', 'wf:terminal', 'steam:idem', $1, '{}'::jsonb, 'succeeded', now())",
    )
    .bind(&hash)
    .execute(pool)
    .await
    .expect("insert terminal steam operation");

    let same_generation = sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_type, workflow_key, idempotency_key, payload_hash, payload, state, finished_at
         )
          VALUES ('friend_request', 'wf:terminal_dup', 'steam:idem', $1, '{}'::jsonb, 'succeeded', now())",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&same_generation).as_deref(),
        Some("23505")
    );

    let operation: (String, i64) = sqlx::query_as(
        "INSERT INTO steam.v1_operations(
             operation_id,
             operation_type,
             workflow_key,
             workflow_generation,
             aggregate_kind,
             aggregate_id,
             subject_kind,
             subject_id,
             idempotency_key,
             idempotency_generation,
             payload_hash,
             payload
          )
          VALUES (
             'op:contract_active',
             'friend_request',
             'wf:active',
             2147483648,
             'player',
             '7656119800000042',
             'discord_user',
             '42',
             'steam:idem',
             2147483648,
             $1,
             '{}'::jsonb
          )
          RETURNING operation_id, idempotency_generation",
    )
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("explicit steam generation allowed");

    let duplicate_operation_id = sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_id, operation_type, idempotency_key, payload_hash, payload
         )
          VALUES ('op:contract_active', 'friend_request', 'other:idem', $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&duplicate_operation_id).as_deref(),
        Some("23505")
    );

    let second_active_generation = sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_type, workflow_key, idempotency_key, idempotency_generation, payload_hash, payload
          )
           VALUES ('friend_request', 'wf:active_2', 'steam:idem', 2147483649, $1, '{}'::jsonb)",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&second_active_generation).as_deref(),
        Some("23505")
    );

    let lease_without_owner = sqlx::query(
        "INSERT INTO steam.v1_operations(
             operation_type, workflow_key, idempotency_key, payload_hash, payload, state, lease_until
         )
          VALUES ('friend_request', 'wf:bad_lease', 'bad:lease', $1, '{}'::jsonb, 'leased', now() + interval '1 minute')",
    )
    .bind(&hash)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&lease_without_owner).as_deref(),
        Some("23514")
    );

    let wrong_generation_result = sqlx::query(
        "INSERT INTO steam.v1_operation_results(
             operation_id, operation_generation, result_payload, status
         )
         VALUES ($1, $2, '{}'::jsonb, 'succeeded')",
    )
    .bind(&operation.0)
    .bind(operation.1 + 1)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&wrong_generation_result).as_deref(),
        Some("23503")
    );

    let result_id: i64 = sqlx::query_scalar(
        "INSERT INTO steam.v1_operation_results(
             operation_id, operation_generation, result_payload, status
         )
         VALUES ($1, $2, '{}'::jsonb, 'succeeded')
          RETURNING id",
    )
    .bind(&operation.0)
    .bind(operation.1)
    .fetch_one(pool)
    .await
    .expect("insert immutable steam result");

    let operation_event_id: i64 = sqlx::query_scalar(
        "INSERT INTO steam.v1_operation_events(
             operation_id, operation_generation, event_type, payload
         )
          VALUES ($1, $2, 'state_observed', '{}'::jsonb)
         RETURNING id",
    )
    .bind(&operation.0)
    .bind(operation.1)
    .fetch_one(pool)
    .await
    .expect("insert immutable steam operation event");
    let update_operation_event = sqlx::query(
        "UPDATE steam.v1_operation_events
            SET payload = '{\"changed\":true}'::jsonb
          WHERE id = $1",
    )
    .bind(operation_event_id)
    .execute(pool)
    .await;
    assert_eq!(
        database_error_code(&update_operation_event).as_deref(),
        Some("55000")
    );
    let delete_operation_event = sqlx::query("DELETE FROM steam.v1_operation_events WHERE id = $1")
        .bind(operation_event_id)
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&delete_operation_event).as_deref(),
        Some("55000")
    );

    let update_result =
        sqlx::query("UPDATE steam.v1_operation_results SET status = 'failed' WHERE id = $1")
            .bind(result_id)
            .execute(pool)
            .await;
    assert_eq!(
        database_error_code(&update_result).as_deref(),
        Some("55000")
    );

    let truncate_result = sqlx::query("TRUNCATE steam.v1_operation_results")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_result).as_deref(),
        Some("55000")
    );
    let truncate_operation_event = sqlx::query("TRUNCATE steam.v1_operation_events")
        .execute(pool)
        .await;
    assert_eq!(
        database_error_code(&truncate_operation_event).as_deref(),
        Some("55000")
    );
}

async fn assert_privacy_registry_covers_new_user_id_display_name_and_json_fields(pool: &PgPool) {
    for (schema, table, column) in expected_privacy_registry_fields() {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                   FROM core.privacy_field_registry
                  WHERE schema_name = $1
                    AND table_name = $2
                    AND column_name = $3
             )",
        )
        .bind(schema)
        .bind(table)
        .bind(column)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| panic!("privacy registry row for {schema}.{table}.{column}: {err}"));
        assert!(
            exists,
            "missing privacy classification for {schema}.{table}.{column}"
        );
    }

    let dangling_rows: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT
           FROM core.privacy_field_registry r
      LEFT JOIN information_schema.columns c
             ON c.table_schema = r.schema_name
            AND c.table_name = r.table_name
            AND c.column_name = r.column_name
          WHERE c.column_name IS NULL",
    )
    .fetch_one(pool)
    .await
    .expect("dangling privacy registry rows");
    assert_eq!(
        dangling_rows, 0,
        "privacy registry must not point at missing columns"
    );

    let manual_automated_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT schema_name, table_name, column_name, reason
           FROM core.privacy_field_registry
          WHERE erasure_action = 'manual_review'
            AND (
                 data_category IN ('user_id', 'display_name')
                 OR reason ILIKE '%automated%'
            )
          ORDER BY schema_name, table_name, column_name",
    )
    .fetch_all(pool)
    .await
    .expect("manual privacy actions for automated fields");
    assert!(
        manual_automated_rows.is_empty(),
        "automated privacy fields must not be manual_review: {manual_automated_rows:?}"
    );

    for &(schema, table, column, erasure_action) in expected_automated_erasure_actions() {
        let actual: String = sqlx::query_scalar(
            "SELECT erasure_action
               FROM core.privacy_field_registry
              WHERE schema_name = $1
                AND table_name = $2
                AND column_name = $3",
        )
        .bind(schema)
        .bind(table)
        .bind(column)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|err| {
            panic!("privacy erasure action for {schema}.{table}.{column}: {err}")
        });
        assert_eq!(
            actual, erasure_action,
            "unexpected erasure action for {schema}.{table}.{column}"
        );
    }

    let aggregate_categories: Vec<String> = sqlx::query_scalar(
        "SELECT data_category
           FROM core.privacy_field_registry
          WHERE schema_name = 'steam'
            AND table_name IN ('v1_workflows', 'v1_operations')
            AND column_name = 'aggregate_id'
          ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .expect("steam v1 aggregate privacy categories");
    assert_eq!(
        aggregate_categories,
        vec!["free_text".to_string(), "free_text".to_string()],
        "aggregate_id is a kind-aware domain aggregate, not an unconditional user_id"
    );

    let result_reason_classifications: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT table_name, column_name, data_category, erasure_action
           FROM core.privacy_field_registry
          WHERE schema_name = 'scrim'
            AND (table_name, column_name) IN (
                ('match_result_refs', 'selection_reason'),
                ('match_result_refs', 'void_reason'),
                ('match_result_selections', 'selection_reason')
            )
          ORDER BY table_name, column_name",
    )
    .fetch_all(pool)
    .await
    .expect("result reason privacy classifications");
    assert_eq!(
        result_reason_classifications,
        vec![
            (
                "match_result_refs".to_string(),
                "selection_reason".to_string(),
                "machine_code".to_string(),
                "retain_non_personal".to_string(),
            ),
            (
                "match_result_refs".to_string(),
                "void_reason".to_string(),
                "machine_code".to_string(),
                "retain_non_personal".to_string(),
            ),
            (
                "match_result_selections".to_string(),
                "selection_reason".to_string(),
                "machine_code".to_string(),
                "retain_non_personal".to_string(),
            ),
        ],
        "result reason fields must be bounded machine codes, not free text"
    );

    let reconciliation_timestamp_classifications: Vec<(String, String, String, String)> =
        sqlx::query_as(
            "SELECT table_name, data_category, retention_action, erasure_action
               FROM core.privacy_field_registry
              WHERE schema_name = 'scrim'
                AND (table_name, column_name) IN (
                    ('match_request_reminder_effects', 'reconciled_at'),
                    ('status_publication_effects', 'reconciled_at'),
                    ('replacement_request_effects', 'reconciled_at')
                )
              ORDER BY table_name",
        )
        .fetch_all(pool)
        .await
        .expect("reconciliation timestamp privacy classifications");
    assert_eq!(
        reconciliation_timestamp_classifications,
        vec![
            (
                "match_request_reminder_effects".to_string(),
                "machine_timestamp".to_string(),
                "retain_operational".to_string(),
                "retain_non_personal".to_string(),
            ),
            (
                "replacement_request_effects".to_string(),
                "machine_timestamp".to_string(),
                "retain_operational".to_string(),
                "retain_non_personal".to_string(),
            ),
            (
                "status_publication_effects".to_string(),
                "machine_timestamp".to_string(),
                "retain_operational".to_string(),
                "retain_non_personal".to_string(),
            ),
        ],
        "reconciliation markers are non-personal machine timestamps"
    );
}

async fn assert_foundation_privacy_inventory_is_classified(pool: &PgPool) {
    let inventory = foundation_privacy_inventory_columns(pool).await;
    assert!(
        !inventory.is_empty(),
        "foundation privacy inventory must be driven by live schema columns"
    );

    let mut missing = Vec::new();
    for (schema, table, column) in &inventory {
        if privacy_registry_has_column(pool, schema, table, column).await {
            continue;
        }
        if column_has_strict_privacy_constraint(pool, schema, table, column).await {
            continue;
        }
        missing.push(format!("{schema}.{table}.{column}"));
    }
    assert!(
        missing.is_empty(),
        "foundation text/json/hash/id columns need registry classification or a hard enum/code/pseudonym/hash constraint: {missing:?}"
    );

    let structural_gaps: Vec<(String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT r.schema_name, r.table_name, r.column_name, r.data_category
          FROM core.privacy_field_registry AS r
          JOIN information_schema.columns AS c
            ON c.table_schema = r.schema_name
           AND c.table_name = r.table_name
           AND c.column_name = r.column_name
         WHERE r.data_category IN ('machine_code', 'domain_id')
           AND NOT EXISTS (
                SELECT 1
                  FROM pg_constraint AS con
                  JOIN pg_class AS rel
                    ON rel.oid = con.conrelid
                  JOIN pg_namespace AS ns
                    ON ns.oid = rel.relnamespace
                  JOIN pg_attribute AS att
                    ON att.attrelid = rel.oid
                   AND att.attnum = ANY(con.conkey)
                 WHERE ns.nspname = r.schema_name
                   AND rel.relname = r.table_name
                   AND att.attname = r.column_name
                   AND con.contype = 'c'
                   AND (
                        pg_get_constraintdef(con.oid) LIKE '%ANY (ARRAY%'
                        OR pg_get_constraintdef(con.oid) LIKE '% IN (%'
                        OR pg_get_constraintdef(con.oid) LIKE '%~%'
                        OR pg_get_constraintdef(con.oid) LIKE '%scrim.is_machine_code%'
                        OR pg_get_constraintdef(con.oid) LIKE '%scrim.is_domain_ref%'
                        OR pg_get_constraintdef(con.oid) LIKE '%scrim.is_actor_source%'
                        OR pg_get_constraintdef(con.oid) LIKE '%scrim_runtime%'
                   )
           )
           AND NOT EXISTS (
                SELECT 1
                  FROM pg_constraint AS con
                  JOIN pg_class AS rel
                    ON rel.oid = con.conrelid
                  JOIN pg_namespace AS ns
                    ON ns.oid = rel.relnamespace
                  JOIN pg_attribute AS att
                    ON att.attrelid = rel.oid
                   AND att.attnum = ANY(con.conkey)
                 WHERE ns.nspname = r.schema_name
                   AND rel.relname = r.table_name
                   AND att.attname = r.column_name
                   AND con.contype = 'f'
           )
         ORDER BY r.schema_name, r.table_name, r.column_name
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("structural privacy registry support");
    assert!(
        structural_gaps.is_empty(),
        "machine_code/domain_id registry labels need live CHECK/FK support, not just registry claims: {structural_gaps:?}"
    );

    let manual_direct_ids: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT schema_name, table_name, column_name, data_category
           FROM core.privacy_field_registry
          WHERE erasure_action = 'manual_review'
            AND data_category IN ('user_id', 'display_name', 'domain_id', 'pseudonym')
          ORDER BY schema_name, table_name, column_name",
    )
    .fetch_all(pool)
    .await
    .expect("manual direct-id privacy classifications");
    assert!(
        manual_direct_ids.is_empty(),
        "direct ids and pseudonyms must not hide behind manual_review: {manual_direct_ids:?}"
    );

    let vague_audit_json: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT schema_name, table_name, column_name, reason
           FROM core.privacy_field_registry
          WHERE retention_action = 'retain_audit'
            AND erasure_action = 'manual_review'
            AND data_category = 'json_payload'
            AND reason NOT ILIKE '%strip%'
            AND reason NOT ILIKE '%typed column%'
            AND reason NOT ILIKE '%separated%'
          ORDER BY schema_name, table_name, column_name",
    )
    .fetch_all(pool)
    .await
    .expect("manual audit json reasons");
    assert!(
        vague_audit_json.is_empty(),
        "manual audit/decision JSON classifications must document raw actor stripping or typed-column separation: {vague_audit_json:?}"
    );
}

async fn foundation_privacy_inventory_columns(pool: &PgPool) -> Vec<(String, String, String)> {
    sqlx::query_as(
        r#"
        WITH new_tables(schema_name, table_name) AS (
            VALUES
                ('scrim', 'audit_actor_pseudonyms'),
                ('scrim', 'audit_events'),
                ('scrim', 'runtime_control_history'),
                ('scrim', 'command_receipts'),
                ('scrim', 'inbox_events'),
                ('scrim', 'outbox_effects'),
                ('scrim', 'effect_receipts'),
                ('scrim', 'announcement_drafts'),
                ('scrim', 'announcement_approvals'),
                ('scrim', 'status_publication_approvals'),
                ('scrim', 'replacement_needs'),
                ('scrim', 'replacement_candidates'),
                ('scrim', 'replacement_requests'),
                ('scrim', 'match_request_reminder_effects'),
                ('scrim', 'status_publication_effects'),
                ('scrim', 'replacement_request_effects'),
                ('scrim', 'match_lineup_snapshots'),
                ('scrim', 'lagebild_snapshots'),
                ('scrim', 'lagebild_evidences'),
                ('scrim', 'lagebild_corrections'),
                ('scrim', 'ai_runs'),
                ('scrim', 'ai_decision_refs'),
                ('scrim', 'match_result_selections'),
                ('scrim', 'match_result_selection_events'),
                ('scrim', 'match_result_clarifications'),
                ('steam', 'v1_workflows'),
                ('steam', 'v1_operations'),
                ('steam', 'v1_operation_results'),
                ('steam', 'v1_operation_events'),
                ('steam', 'v1_deliveries')
        ),
        added_columns(schema_name, table_name, column_name) AS (
            VALUES
                ('scrim', 'match_result_refs', 'external_result_ref'),
                ('scrim', 'match_result_refs', 'validation_status'),
                ('scrim', 'match_result_refs', 'clarification_state'),
                ('scrim', 'match_result_refs', 'clarification_payload'),
                ('scrim', 'match_result_refs', 'last_error'),
                ('scrim', 'match_result_refs', 'raw_result_json'),
                ('scrim', 'match_result_refs', 'normalized_result_json'),
                ('scrim', 'match_result_refs', 'selected_by_user_id'),
                ('scrim', 'match_result_refs', 'selection_reason'),
                ('scrim', 'match_result_refs', 'void_reason'),
                ('scrim', 'matches', 'join_code'),
                ('scrim', 'matches', 'lobby_code_source_user_id'),
                ('scrim', 'matches', 'lobby_code_source_display_name'),
                ('scrim', 'matches', 'lobby_code_message_ids'),
                ('scrim', 'matches', 'lobby_code_corrections'),
                ('scrim', 'matches', 'result_json'),
                ('scrim', 'match_requests', 'slot_options'),
                ('scrim', 'match_requests', 'team_query_message_ids'),
                ('scrim', 'match_requests', 'released_slot'),
                ('scrim', 'match_requests', 'released_by_user_id'),
                ('scrim', 'match_requests', 'released_by_display_name'),
                ('scrim', 'match_requests', 'override_reason'),
                ('scrim', 'match_requests', 'team_status_message_ids'),
                ('scrim', 'match_requests', 'status_message_last_error'),
                ('scrim', 'replacement_needs', 'match_request_id'),
                ('scrim', 'replacement_needs', 'slot_index'),
                ('scrim', 'match_request_reminder_effects', 'reminder_id'),
                ('scrim', 'match_request_reminder_effects', 'outbox_effect_id'),
                ('scrim', 'match_request_reminder_effects', 'reconciled_at'),
                ('scrim', 'status_publication_effects', 'status_publication_approval_id'),
                ('scrim', 'status_publication_effects', 'outbox_effect_id'),
                ('scrim', 'status_publication_effects', 'reconciled_at'),
                ('scrim', 'replacement_request_effects', 'replacement_request_id'),
                ('scrim', 'replacement_request_effects', 'outbox_effect_id'),
                ('scrim', 'replacement_request_effects', 'reconciled_at')
        ),
        scoped_columns AS (
            SELECT c.table_schema::TEXT AS schema_name,
                   c.table_name::TEXT AS table_name,
                   c.column_name::TEXT AS column_name
              FROM information_schema.columns AS c
              JOIN new_tables AS nt
                ON nt.schema_name = c.table_schema
               AND nt.table_name = c.table_name
             WHERE c.data_type IN ('text', 'bytea', 'ARRAY')
                OR c.udt_name = 'jsonb'
                OR c.column_name ILIKE '%user_id%'
                OR c.column_name ILIKE '%display_name%'
            UNION
            SELECT c.table_schema::TEXT AS schema_name,
                   c.table_name::TEXT AS table_name,
                   c.column_name::TEXT AS column_name
              FROM information_schema.columns AS c
              JOIN added_columns AS ac
                ON ac.schema_name = c.table_schema
               AND ac.table_name = c.table_name
               AND ac.column_name = c.column_name
        )
        SELECT schema_name, table_name, column_name
          FROM scoped_columns
         ORDER BY schema_name, table_name, column_name
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("foundation privacy inventory columns")
}

async fn privacy_registry_has_column(
    pool: &PgPool,
    schema: &str,
    table: &str,
    column: &str,
) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
               FROM core.privacy_field_registry
              WHERE schema_name = $1
                AND table_name = $2
                AND column_name = $3
         )",
    )
    .bind(schema)
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| panic!("privacy registry lookup for {schema}.{table}.{column}: {err}"))
}

async fn column_has_strict_privacy_constraint(
    pool: &PgPool,
    schema: &str,
    table: &str,
    column: &str,
) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
               FROM pg_constraint AS con
               JOIN pg_class AS rel
                 ON rel.oid = con.conrelid
               JOIN pg_namespace AS ns
                 ON ns.oid = rel.relnamespace
               JOIN pg_attribute AS att
                 ON att.attrelid = rel.oid
                AND att.attnum = ANY(con.conkey)
              WHERE ns.nspname = $1
                AND rel.relname = $2
                AND att.attname = $3
                AND con.contype = 'c'
                AND (
                     pg_get_constraintdef(con.oid) LIKE '%ANY (ARRAY%'
                     OR pg_get_constraintdef(con.oid) LIKE '% IN (%'
                     OR pg_get_constraintdef(con.oid) LIKE '%~%'
                     OR pg_get_constraintdef(con.oid) LIKE '%scrim.is_machine_code%'
                     OR pg_get_constraintdef(con.oid) LIKE '%scrim.is_domain_ref%'
                     OR pg_get_constraintdef(con.oid) LIKE '%scrim.is_actor_source%'
                     OR pg_get_constraintdef(con.oid) LIKE '%octet_length%'
                     OR pg_get_constraintdef(con.oid) LIKE '%scrim_runtime%'
                )
         )",
    )
    .bind(schema)
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|err| {
        panic!("strict privacy constraint lookup for {schema}.{table}.{column}: {err}")
    })
}

fn expected_automated_erasure_actions(
) -> &'static [(&'static str, &'static str, &'static str, &'static str)] {
    &[
        (
            "scrim",
            "audit_actor_pseudonyms",
            "actor_ref",
            "delete_row_on_user_delete",
        ),
        (
            "scrim",
            "command_receipts",
            "payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "command_receipts",
            "result_payload",
            "redact_on_user_delete",
        ),
        ("scrim", "inbox_events", "payload", "redact_on_user_delete"),
        (
            "scrim",
            "outbox_effects",
            "payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "effect_receipts",
            "receipt_payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "replacement_candidates",
            "candidate_data",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "replacement_candidates",
            "score_data",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "replacement_requests",
            "request_payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_lineup_snapshots",
            "lineup_payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_snapshots",
            "lagebild_text",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_snapshots",
            "data_summary",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_snapshots",
            "error",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_evidences",
            "label",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_evidences",
            "reference_id",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_evidences",
            "payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_corrections",
            "author_user_id",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_corrections",
            "author_display_name",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "lagebild_corrections",
            "message",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "matches",
            "lobby_code_source_user_id",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "matches",
            "lobby_code_source_display_name",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "matches",
            "lobby_code_corrections",
            "redact_on_user_delete",
        ),
        ("scrim", "matches", "result_json", "redact_on_user_delete"),
        (
            "scrim",
            "match_requests",
            "released_by_user_id",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_requests",
            "released_by_display_name",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_requests",
            "override_reason",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_requests",
            "status_message_last_error",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_result_refs",
            "last_error",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_result_refs",
            "raw_result_json",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_result_refs",
            "normalized_result_json",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_result_refs",
            "clarification_payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_result_clarifications",
            "request_payload",
            "redact_on_user_delete",
        ),
        (
            "scrim",
            "match_result_clarifications",
            "response_payload",
            "redact_on_user_delete",
        ),
        ("scrim", "ai_runs", "subject_id", "redact_on_user_delete"),
        (
            "steam",
            "v1_workflows",
            "aggregate_id",
            "redact_on_user_delete",
        ),
        (
            "steam",
            "v1_workflows",
            "subject_id",
            "redact_on_user_delete",
        ),
        ("steam", "v1_workflows", "payload", "redact_on_user_delete"),
        (
            "steam",
            "v1_operations",
            "aggregate_id",
            "redact_on_user_delete",
        ),
        (
            "steam",
            "v1_operations",
            "subject_id",
            "redact_on_user_delete",
        ),
        ("steam", "v1_operations", "payload", "redact_on_user_delete"),
        (
            "steam",
            "v1_operations",
            "result_payload",
            "redact_on_user_delete",
        ),
        (
            "steam",
            "v1_operation_results",
            "result_payload",
            "redact_on_user_delete",
        ),
        (
            "steam",
            "v1_operation_events",
            "payload",
            "redact_on_user_delete",
        ),
        ("steam", "v1_deliveries", "payload", "redact_on_user_delete"),
    ]
}

fn expected_privacy_registry_fields() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        ("scrim", "runtime_control", "actor_pseudonym"),
        ("scrim", "runtime_control", "decision_data"),
        ("scrim", "runtime_control_history", "actor_pseudonym"),
        ("scrim", "runtime_control_history", "decision_data"),
        ("scrim", "command_receipts", "payload"),
        ("scrim", "command_receipts", "result_payload"),
        ("scrim", "command_receipts", "last_error_code"),
        ("scrim", "command_receipts", "last_error_hash"),
        ("scrim", "inbox_events", "payload"),
        ("scrim", "inbox_events", "last_error_code"),
        ("scrim", "inbox_events", "last_error_hash"),
        ("scrim", "outbox_effects", "payload"),
        ("scrim", "outbox_effects", "last_error_code"),
        ("scrim", "outbox_effects", "last_error_hash"),
        ("scrim", "effect_receipts", "receipt_payload"),
        ("scrim", "audit_actor_pseudonyms", "actor_ref"),
        ("scrim", "audit_events", "entity_id"),
        ("scrim", "audit_events", "actor_pseudonym"),
        ("scrim", "audit_events", "before_data"),
        ("scrim", "audit_events", "after_data"),
        ("scrim", "audit_events", "decision_data"),
        ("scrim", "audit_events", "metadata"),
        ("scrim", "replacement_candidates", "discord_user_id"),
        ("scrim", "replacement_candidates", "candidate_data"),
        ("scrim", "replacement_candidates", "score_data"),
        ("scrim", "replacement_requests", "discord_user_id"),
        ("scrim", "replacement_requests", "request_payload"),
        ("scrim", "replacement_needs", "match_request_id"),
        ("scrim", "replacement_needs", "slot_index"),
        ("scrim", "match_request_reminder_effects", "reminder_id"),
        (
            "scrim",
            "match_request_reminder_effects",
            "outbox_effect_id",
        ),
        ("scrim", "match_request_reminder_effects", "reconciled_at"),
        (
            "scrim",
            "status_publication_effects",
            "status_publication_approval_id",
        ),
        ("scrim", "status_publication_effects", "outbox_effect_id"),
        ("scrim", "status_publication_effects", "reconciled_at"),
        (
            "scrim",
            "replacement_request_effects",
            "replacement_request_id",
        ),
        ("scrim", "replacement_request_effects", "outbox_effect_id"),
        ("scrim", "replacement_request_effects", "reconciled_at"),
        ("scrim", "match_lineup_snapshots", "lineup_payload"),
        ("scrim", "lagebild_snapshots", "generated_for"),
        ("scrim", "lagebild_snapshots", "source"),
        ("scrim", "lagebild_snapshots", "status"),
        ("scrim", "lagebild_snapshots", "lagebild_text"),
        ("scrim", "lagebild_snapshots", "data_summary"),
        ("scrim", "lagebild_snapshots", "model"),
        ("scrim", "lagebild_snapshots", "error"),
        ("scrim", "lagebild_evidences", "evidence_type"),
        ("scrim", "lagebild_evidences", "label"),
        ("scrim", "lagebild_evidences", "url"),
        ("scrim", "lagebild_evidences", "reference_id"),
        ("scrim", "lagebild_evidences", "payload"),
        ("scrim", "lagebild_corrections", "author_user_id"),
        ("scrim", "lagebild_corrections", "author_display_name"),
        ("scrim", "lagebild_corrections", "message"),
        ("scrim", "ai_runs", "subject_id"),
        ("scrim", "ai_runs", "decision_ref_kind"),
        ("scrim", "ai_runs", "decision_ref_id"),
        ("scrim", "ai_runs", "last_error_code"),
        ("scrim", "ai_runs", "last_error_hash"),
        ("scrim", "ai_decision_refs", "target_kind"),
        ("scrim", "ai_decision_refs", "target_id"),
        ("scrim", "ai_decision_refs", "decision_data"),
        ("scrim", "match_result_refs", "clarification_payload"),
        ("scrim", "match_result_refs", "last_error"),
        ("scrim", "match_result_refs", "raw_result_json"),
        ("scrim", "match_result_refs", "normalized_result_json"),
        ("scrim", "match_result_refs", "selected_by_user_id"),
        ("scrim", "match_result_refs", "selection_reason"),
        ("scrim", "match_result_refs", "void_reason"),
        ("scrim", "matches", "join_code"),
        ("scrim", "matches", "lobby_code_source_user_id"),
        ("scrim", "matches", "lobby_code_source_display_name"),
        ("scrim", "matches", "lobby_code_message_ids"),
        ("scrim", "matches", "lobby_code_corrections"),
        ("scrim", "matches", "result_json"),
        ("scrim", "match_requests", "slot_options"),
        ("scrim", "match_requests", "team_query_message_ids"),
        ("scrim", "match_requests", "released_slot"),
        ("scrim", "match_requests", "released_by_user_id"),
        ("scrim", "match_requests", "released_by_display_name"),
        ("scrim", "match_requests", "override_reason"),
        ("scrim", "match_requests", "team_status_message_ids"),
        ("scrim", "match_requests", "status_message_last_error"),
        ("scrim", "match_result_selections", "selected_by_user_id"),
        (
            "scrim",
            "match_result_selections",
            "selected_by_display_name",
        ),
        ("scrim", "match_result_selections", "selection_reason"),
        ("scrim", "match_result_selection_events", "actor_pseudonym"),
        (
            "scrim",
            "match_result_selection_events",
            "old_result_ref_id",
        ),
        (
            "scrim",
            "match_result_selection_events",
            "new_result_ref_id",
        ),
        ("scrim", "match_result_selection_events", "before_data"),
        ("scrim", "match_result_selection_events", "after_data"),
        ("scrim", "match_result_clarifications", "request_payload"),
        ("scrim", "match_result_clarifications", "response_payload"),
        ("steam", "v1_workflows", "payload"),
        ("steam", "v1_workflows", "aggregate_id"),
        ("steam", "v1_workflows", "subject_id"),
        ("steam", "v1_operations", "payload"),
        ("steam", "v1_operations", "aggregate_id"),
        ("steam", "v1_operations", "subject_id"),
        ("steam", "v1_operations", "last_error_code"),
        ("steam", "v1_operations", "last_error_hash"),
        ("steam", "v1_operations", "result_payload"),
        ("steam", "v1_operation_results", "result_payload"),
        ("steam", "v1_operation_events", "payload"),
        ("steam", "v1_deliveries", "payload"),
    ]
}

async fn seed_match(pool: &PgPool, team_a_id: i32, team_b_id: i32, match_id: i32) {
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES ($1, $2, now())")
        .bind(team_a_id)
        .bind(format!("Team {team_a_id}"))
        .execute(pool)
        .await
        .expect("insert team a");
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES ($1, $2, now())")
        .bind(team_b_id)
        .bind(format!("Team {team_b_id}"))
        .execute(pool)
        .await
        .expect("insert team b");
    sqlx::query(
        "INSERT INTO scrim.matches(id, team_a_id, team_b_id, status, created_at)
         VALUES ($1, $2, $3, 'scheduled', now())",
    )
    .bind(match_id)
    .bind(team_a_id)
    .bind(team_b_id)
    .execute(pool)
    .await
    .expect("insert match");
}

async fn seed_legacy_result_match(
    pool: &PgPool,
    team_a_id: i32,
    team_b_id: i32,
    match_id: i32,
    steam_match_id: i64,
) {
    seed_match(pool, team_a_id, team_b_id, match_id).await;
    sqlx::query(
        "UPDATE scrim.matches
            SET steam_match_id = $2,
                winner_team_id = $3,
                result_json = '{\"legacy\":true}'::jsonb,
                status = 'completed',
                updated_at = now()
          WHERE id = $1",
    )
    .bind(match_id)
    .bind(steam_match_id)
    .bind(team_a_id)
    .execute(pool)
    .await
    .expect("seed legacy match result columns");
}

async fn seed_legacy_match_with_steam_id_only(
    pool: &PgPool,
    team_a_id: i32,
    team_b_id: i32,
    match_id: i32,
    steam_match_id: i64,
) {
    seed_match(pool, team_a_id, team_b_id, match_id).await;
    sqlx::query(
        "UPDATE scrim.matches
            SET steam_match_id = $2,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(match_id)
    .bind(steam_match_id)
    .execute(pool)
    .await
    .expect("seed legacy steam id only");
}

async fn insert_result_ref(
    pool: &PgPool,
    match_id: i32,
    winner_team_id: i32,
    steam_match_id: i64,
    selected: bool,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        r#"INSERT INTO scrim.match_result_refs(
             match_id,
             steam_match_id,
             source_user_id,
             source_display_name,
             fetch_status,
             winner_team_id,
             normalized_result_json,
             validation_status,
             is_selected,
             selected_at,
             selected_by_user_id
         )
         VALUES (
             $1,
             $2,
              '424242',
             'Tester',
             'fetched',
             $3,
             '{"contract":true}'::jsonb,
             'valid',
             $4,
             CASE WHEN $4 THEN now() ELSE NULL END,
              CASE WHEN $4 THEN '424242' ELSE NULL END
          )
          RETURNING id"#,
    )
    .bind(match_id)
    .bind(steam_match_id)
    .bind(winner_team_id)
    .bind(selected)
    .fetch_one(pool)
    .await
}

async fn insert_replacement_need(pool: &PgPool, team_id: i32) -> i64 {
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES ($1, $2, now())")
        .bind(team_id)
        .bind(format!("Replacement Team {team_id}"))
        .execute(pool)
        .await
        .expect("insert replacement team");

    sqlx::query_scalar(
        "INSERT INTO scrim.replacement_needs(team_id, reason, created_by_user_id, created_by_display_name)
         VALUES ($1, 'contract_test', '424242', 'Tester')
         RETURNING id",
    )
    .bind(team_id)
    .fetch_one(pool)
    .await
    .expect("insert replacement need")
}

fn database_error_code<T>(result: &Result<T, sqlx::Error>) -> Option<String> {
    match result {
        Err(sqlx::Error::Database(db_error)) => db_error.code().map(|code| code.into_owned()),
        _ => None,
    }
}

fn test_dsn() -> String {
    std::env::var("CENTRAL_TEST_DSN")
        .expect("CENTRAL_TEST_DSN must be set for ignored DB tests; do not silently skip")
}

fn fresh_db_name(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    format!("dlcentral_{label}_{}_{}", std::process::id(), nanos)
}

fn swap_db(dsn: &str, db: &str) -> String {
    let (base, _old) = dsn.rsplit_once('/').expect("DSN contains database path");
    format!("{base}/{db}")
}

async fn create_fresh_db(admin: &PgPool, dbname: &str) {
    sqlx::query(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .execute(admin)
        .await
        .ok();
    sqlx::query(&format!("CREATE DATABASE {dbname}"))
        .execute(admin)
        .await
        .expect("create fresh database");
}

async fn drop_db(admin: &PgPool, dbname: &str) {
    sqlx::query(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .execute(admin)
        .await
        .ok();
}

async fn run_migrations_between(pool: &PgPool, min_version: i64, max_version: i64) {
    run_migrations_between_result(pool, min_version, max_version)
        .await
        .expect("run selected migrations");
}

async fn run_migrations_between_result(
    pool: &PgPool,
    min_version: i64,
    max_version: i64,
) -> Result<(), sqlx::migrate::MigrateError> {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let all = Migrator::new(migrations_path).await?;
    let selected = all
        .migrations
        .iter()
        .filter(|migration| migration.version >= min_version && migration.version <= max_version)
        .cloned()
        .collect::<Vec<_>>();
    let migrator = Migrator {
        migrations: Cow::Owned(selected),
        ..Migrator::DEFAULT
    };

    migrator.run(pool).await
}
