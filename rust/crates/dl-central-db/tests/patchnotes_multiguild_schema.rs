use std::path::Path;

fn migration() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/2026092801_patchnotes_multiguild.sql"),
    )
    .expect("Patchnotes-Multiguild-Migration fehlt")
}

#[test]
fn guild_settings_haben_sichere_defaults_und_begrenzte_werte() {
    let sql = migration();
    for expected in [
        "patchnotes.guild_settings",
        "guild_id BIGINT PRIMARY KEY CHECK (guild_id > 0)",
        "enabled BOOLEAN NOT NULL DEFAULT FALSE",
        "source_selection TEXT[] NOT NULL DEFAULT ARRAY['forum', 'steam']::TEXT[]",
        "language IN ('de', 'en')",
        "approval_mode IN ('automatic', 'manual')",
        "NOT enabled OR channel_id IS NOT NULL",
        "NOT mention_role OR role_id IS NOT NULL",
        "cardinality(section_selection) <= 100",
        "char_length(array_to_string(section_selection, '')) <= 4096",
        "array_to_string(section_selection, '') !~ '[[:cntrl:]]'",
    ] {
        assert!(
            sql.contains(expected),
            "Guild-Einstellungsvertrag fehlt: {expected}"
        );
    }
    assert!(!sql.contains("section_selection <@"));
}

#[test]
fn dispatch_ist_guild_und_revision_getrennt_und_schuetzt_recovery() {
    let sql = migration();
    for expected in [
        "patchnotes.guild_dispatch",
        "PRIMARY KEY (guild_id, patch_id, revision_hash)",
        "revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$')",
        "status IN ('pending', 'awaiting_approval', 'sending', 'delivery_unknown', 'sent', 'retry', 'failed')",
        "send_channel_id BIGINT",
        "sent_message_ids BIGINT[] NOT NULL DEFAULT ARRAY[]::BIGINT[]",
        "send_attempt_id UUID",
        "send_lease_expires_at TIMESTAMPTZ",
        "recovery_outcome TEXT CHECK (recovery_outcome IN ('delivered', 'not_delivered', 'partial'))",
        "REFERENCES patchnotes.guild_settings (guild_id)",
        "REFERENCES patchnotes.changelog_posts (id)",
        "manual patchnotes dispatch requires approval before sending",
        "unknown delivery must be reconciled before marking sent",
        "unknown patchnotes delivery must be reconciled before another send attempt",
        "new patchnotes send attempt cannot reuse a prior recovery result",
    ] {
        assert!(sql.contains(expected), "Dispatch-Vertrag fehlt: {expected}");
    }
}

#[test]
fn dienstrolle_erhaelt_nur_scoped_dml_und_keine_ddl_rechte() {
    let sql = migration();
    assert!(sql.contains("dl_patchnotes_dml"));
    assert!(sql.contains("NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE"));
    assert!(sql.contains("GRANT USAGE ON SCHEMA patchnotes TO dl_patchnotes_dml"));
    assert!(sql.contains("GRANT SELECT, INSERT, UPDATE, DELETE"));
    assert!(sql.contains(
        "REVOKE ALL ON TABLE patchnotes.guild_settings, patchnotes.guild_dispatch FROM PUBLIC"
    ));
    assert!(!sql.contains("GRANT ALL"));
    assert!(!sql.contains("GRANT CREATE"));
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn migration_ist_isoliert_anwendbar_und_erfuellt_guild_dispatch_vertraege(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::test_pool().await?;
    let guild_one = 9_928_001_i64;
    let guild_two = 9_928_002_i64;
    let manual_guild = 9_928_005_i64;
    let patch_id = 9_928_003_i64;
    let missing_patch_id = 9_928_006_i64;
    let invalid_status_patch_id = 9_928_007_i64;
    let hash_one = "a".repeat(64);
    let hash_two = "b".repeat(64);

    for guild_id in [guild_one, guild_two, manual_guild] {
        sqlx::query("INSERT INTO patchnotes.guild_settings (guild_id) VALUES ($1)")
            .bind(guild_id)
            .execute(db.pool())
            .await?;
    }

    let defaults: (bool, Vec<String>, Vec<String>, String, String) = sqlx::query_as(
        "SELECT enabled, source_selection, section_selection, language, approval_mode
           FROM patchnotes.guild_settings
          WHERE guild_id = $1",
    )
    .bind(guild_one)
    .fetch_one(db.pool())
    .await?;
    assert_eq!(
        defaults,
        (
            false,
            vec!["forum".to_string(), "steam".to_string()],
            Vec::new(),
            "de".to_string(),
            "automatic".to_string(),
        )
    );

    sqlx::query(
        "UPDATE patchnotes.guild_settings
            SET section_selection = ARRAY['A dynamic patch title', 'Another title']
          WHERE guild_id = $1",
    )
    .bind(guild_one)
    .execute(db.pool())
    .await?;

    for (guild_id, sections_sql) in [
        (9_928_008_i64, "array_fill('section'::text, ARRAY[101])"),
        (9_928_009_i64, "ARRAY[repeat('x', 4097)]"),
        (9_928_010_i64, "ARRAY['bad' || chr(1)]"),
    ] {
        sqlx::query(&format!(
            "INSERT INTO patchnotes.guild_settings (guild_id, section_selection) VALUES ($1, {sections_sql})"
        ))
        .bind(guild_id)
        .execute(db.pool())
        .await
        .expect_err("section selection must satisfy size and control-character bounds");
    }

    sqlx::query(
        "INSERT INTO patchnotes.changelog_posts (id, title, url)
         VALUES ($1, 'contract patch', 'https://example.invalid/contract-patch'),
                ($2, 'invalid status patch', 'https://example.invalid/invalid-status-patch')",
    )
    .bind(patch_id)
    .bind(invalid_status_patch_id)
    .execute(db.pool())
    .await?;

    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch
             (guild_id, patch_id, revision_hash, status, send_channel_id, sent_message_ids)
         VALUES ($1, $3, $4, 'sent', 9928004, ARRAY[9928005]::BIGINT[]),
                ($2, $3, $4, 'pending', 9928009, ARRAY[]::BIGINT[])",
    )
    .bind(guild_one)
    .bind(guild_two)
    .bind(patch_id)
    .bind(&hash_one)
    .execute(db.pool())
    .await?;

    let rows: Vec<(i64, String, Vec<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT guild_id, status, sent_message_ids, send_channel_id
           FROM patchnotes.guild_dispatch
          WHERE patch_id = $1 AND revision_hash = $2
          ORDER BY guild_id",
    )
    .bind(patch_id)
    .bind(&hash_one)
    .fetch_all(db.pool())
    .await?;
    assert_eq!(
        rows,
        vec![
            (
                guild_one,
                "sent".to_string(),
                vec![9_928_005],
                Some(9_928_004)
            ),
            (
                guild_two,
                "pending".to_string(),
                Vec::new(),
                Some(9_928_009)
            ),
        ]
    );
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch SET send_channel_id = 9928013
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_two)
    .bind(patch_id)
    .bind(&hash_one)
    .execute(db.pool())
    .await?;

    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash, status)
         VALUES ($1, $2, $3, 'pending')",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&hash_one)
    .execute(db.pool())
    .await
    .expect_err("same Guild/Patch/Revision must be unique");

    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash, status)
         VALUES ($1, $2, $3, 'pending')",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&hash_two)
    .execute(db.pool())
    .await?;
    let revisions: Vec<(String, String)> = sqlx::query_as(
        "SELECT revision_hash, status FROM patchnotes.guild_dispatch
          WHERE guild_id = $1 AND patch_id = $2 ORDER BY revision_hash",
    )
    .bind(guild_one)
    .bind(patch_id)
    .fetch_all(db.pool())
    .await?;
    assert_eq!(
        revisions,
        vec![
            (hash_one.clone(), "sent".to_string()),
            (hash_two.clone(), "pending".to_string()),
        ]
    );

    let recovery_hash = "f".repeat(64);
    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch
             (guild_id, patch_id, revision_hash, status, send_channel_id,
              send_attempt_id, send_started_at, send_lease_expires_at)
         VALUES ($1, $2, $3, 'sending', 9928014,
                 '00000000-0000-0000-0000-000000000001'::UUID, now(), now() + interval '1 minute')",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;

    let unapproved_mode_switch = sqlx::query(
        "UPDATE patchnotes.guild_settings SET approval_mode = 'manual' WHERE guild_id = $1",
    )
    .bind(guild_one)
    .execute(db.pool())
    .await
    .expect_err("manual mode cannot inherit an unapproved in-flight send");
    assert_eq!(
        error_code(&unapproved_mode_switch),
        Some("23514".to_string())
    );

    let invalid_patch = sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash)
         VALUES ($1, $2, $3)",
    )
    .bind(guild_one)
    .bind(missing_patch_id)
    .bind("c".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("dispatch must reject an unknown patch");
    assert_eq!(error_code(&invalid_patch), Some("23503".to_string()));

    let delete_referenced_patch =
        sqlx::query("DELETE FROM patchnotes.changelog_posts WHERE id = $1")
            .bind(patch_id)
            .execute(db.pool())
            .await
            .expect_err("a referenced patch cannot be deleted");
    assert_eq!(
        error_code(&delete_referenced_patch),
        Some("23503".to_string())
    );

    sqlx::query(
        "UPDATE patchnotes.guild_settings SET approval_mode = 'manual' WHERE guild_id = $1",
    )
    .bind(manual_guild)
    .execute(db.pool())
    .await?;
    let unapproved_send = sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch
             (guild_id, patch_id, revision_hash, status, send_channel_id,
              send_attempt_id, send_started_at, send_lease_expires_at)
         VALUES ($1, $2, $3, 'sending', 9928011,
                 '00000000-0000-0000-0000-000000000002'::UUID, now(), now() + interval '1 minute')",
    )
    .bind(manual_guild)
    .bind(patch_id)
    .bind("d".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("manual approval is required before sending");
    assert_eq!(error_code(&unapproved_send), Some("23514".to_string()));
    let unapproved_sent = sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch
             (guild_id, patch_id, revision_hash, status, send_channel_id, sent_message_ids)
         VALUES ($1, $2, $3, 'sent', 9928011, ARRAY[9928013]::BIGINT[])",
    )
    .bind(manual_guild)
    .bind(patch_id)
    .bind("e".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("manual approval is required before sent");
    assert_eq!(error_code(&unapproved_sent), Some("23514".to_string()));

    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch
             (guild_id, patch_id, revision_hash, status, approved_by_user_id, approved_at)
         VALUES ($1, $2, $3, 'awaiting_approval', 9928012, now())",
    )
    .bind(manual_guild)
    .bind(patch_id)
    .bind("d".repeat(64))
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'sending', send_channel_id = 9928011,
                send_attempt_id = '00000000-0000-0000-0000-000000000003'::UUID,
                send_started_at = now(), send_lease_expires_at = now() + interval '1 minute'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(manual_guild)
    .bind(patch_id)
    .bind("d".repeat(64))
    .execute(db.pool())
    .await?;

    let delivery_unknown = sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'delivery_unknown', send_lease_expires_at = NULL
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await
    .expect_err("active send lease cannot be recovered yet");
    assert_eq!(error_code(&delivery_unknown), Some("23514".to_string()));

    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET send_lease_expires_at = now() - interval '1 second'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'delivery_unknown', send_lease_expires_at = NULL
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    let direct_unknown_retry = sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'sending', send_lease_expires_at = now() + interval '1 minute'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await
    .expect_err("unknown delivery cannot start another attempt before reconciliation");
    assert_eq!(error_code(&direct_unknown_retry), Some("23514".to_string()));

    let blind_retry = sqlx::query(
        "UPDATE patchnotes.guild_dispatch SET status = 'retry'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await
    .expect_err("unknown delivery cannot be retried before reconciliation");
    assert_eq!(error_code(&blind_retry), Some("23514".to_string()));

    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'retry', recovery_outcome = 'not_delivered', recovery_checked_at = now()
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch SET recovery_outcome = 'partial'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    let partial_delivery_channel_change = sqlx::query(
        "UPDATE patchnotes.guild_dispatch SET send_channel_id = 9928016
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await
    .expect_err("partial delivery must keep its original channel for recovery");
    assert_eq!(
        error_code(&partial_delivery_channel_change),
        Some("23514".to_string())
    );
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch SET recovery_outcome = 'not_delivered'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    let stale_recovery_reuse = sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'sending', send_lease_expires_at = now() + interval '1 minute'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await
    .expect_err("new send attempt cannot reuse a prior recovery result");
    assert_eq!(error_code(&stale_recovery_reuse), Some("23514".to_string()));
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'sending', recovery_outcome = NULL, recovery_checked_at = NULL,
                send_attempt_id = '00000000-0000-0000-0000-000000000005'::UUID,
                send_started_at = now(), send_lease_expires_at = now() + interval '1 minute'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;

    let confirmed_delivery_hash = "c".repeat(64);
    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch
             (guild_id, patch_id, revision_hash, status, send_channel_id,
              send_attempt_id, send_started_at, send_lease_expires_at)
         VALUES ($1, $2, $3, 'sending', 9928014,
                 '00000000-0000-0000-0000-000000000004'::UUID,
                 now() - interval '2 minutes', now() - interval '1 minute')",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&confirmed_delivery_hash)
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'delivery_unknown', send_lease_expires_at = NULL
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&confirmed_delivery_hash)
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'sent', sent_message_ids = ARRAY[9928015]::BIGINT[],
                recovery_outcome = 'delivered', recovery_checked_at = now()
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&confirmed_delivery_hash)
    .execute(db.pool())
    .await?;
    let recovered: (String, Option<i64>, Vec<i64>) = sqlx::query_as(
        "SELECT status, send_channel_id, sent_message_ids
           FROM patchnotes.guild_dispatch
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&confirmed_delivery_hash)
    .fetch_one(db.pool())
    .await?;
    assert_eq!(
        recovered,
        ("sent".to_string(), Some(9_928_014), vec![9_928_015])
    );
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET send_lease_expires_at = now() - interval '1 second'
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'delivery_unknown', send_lease_expires_at = NULL
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch
            SET status = 'retry', recovery_outcome = 'not_delivered', recovery_checked_at = now()
          WHERE guild_id = $1 AND patch_id = $2 AND revision_hash = $3",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind(&recovery_hash)
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_settings SET approval_mode = 'manual' WHERE guild_id = $1",
    )
    .bind(guild_one)
    .execute(db.pool())
    .await?;

    let invalid_status = sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash, status)
         VALUES ($1, $2, $3, 'unknown')",
    )
    .bind(guild_one)
    .bind(invalid_status_patch_id)
    .bind("e".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("invalid status must be rejected");
    assert_eq!(error_code(&invalid_status), Some("23514".to_string()));

    let can_insert: bool = sqlx::query_scalar(
        "SELECT has_table_privilege('dl_patchnotes_dml', 'patchnotes.guild_dispatch', 'INSERT')",
    )
    .fetch_one(db.pool())
    .await?;
    let can_create: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege('dl_patchnotes_dml', 'patchnotes', 'CREATE')",
    )
    .fetch_one(db.pool())
    .await?;
    assert!(can_insert);
    assert!(!can_create);

    let mut dml_tx = db.pool().begin().await?;
    sqlx::query("SET LOCAL ROLE dl_patchnotes_dml")
        .execute(&mut *dml_tx)
        .await?;
    sqlx::query("INSERT INTO patchnotes.guild_settings (guild_id) VALUES (9928120)")
        .execute(&mut *dml_tx)
        .await?;
    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash)
         VALUES (9928120, $1, $2)",
    )
    .bind(patch_id)
    .bind("d".repeat(64))
    .execute(&mut *dml_tx)
    .await?;
    dml_tx.rollback().await?;

    Ok(())
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn patchnotes_identity_truncate_includes_fk_dependent_dispatch(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::test_pool().await?;
    sqlx::raw_sql(
        "TRUNCATE patchnotes.guild_dispatch, patchnotes.changelog_posts, patchnotes.deadlock_changelogs",
    )
        .execute(db.pool())
        .await?;
    Ok(())
}

fn error_code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(|code| code.into_owned())
}
