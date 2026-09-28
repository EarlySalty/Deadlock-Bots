use std::path::Path;

fn migration() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/2026092801_patchnotes_multiguild.sql"),
    )
    .expect("Patchnotes-Multiguild-Migration fehlt")
}

#[test]
fn guild_settings_haben_sichere_defaults_und_gueltige_werte() {
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
    ] {
        assert!(
            sql.contains(expected),
            "Guild-Einstellungsvertrag fehlt: {expected}"
        );
    }
}

#[test]
fn dispatch_ist_guild_getrennt_revisionsgebunden_und_fk_geschuetzt() {
    let sql = migration();
    for expected in [
        "patchnotes.guild_dispatch",
        "PRIMARY KEY (guild_id, patch_id)",
        "revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$')",
        "status IN ('pending', 'awaiting_approval', 'sending', 'sent', 'retry', 'failed')",
        "sent_message_ids BIGINT[] NOT NULL DEFAULT ARRAY[]::BIGINT[]",
        "REFERENCES patchnotes.guild_settings (guild_id)",
        "REFERENCES patchnotes.changelog_posts (id)",
        "approved_by_user_id",
        "next_attempt_at TIMESTAMPTZ",
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
async fn migration_ist_isoliert_anwendbar_und_trennt_guild_dispatch(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::test_pool().await?;
    let guild_one = 9_928_001_i64;
    let guild_two = 9_928_002_i64;
    let patch_id = 9_928_003_i64;
    let invalid_status_patch_id = 9_928_007_i64;

    for guild_id in [guild_one, guild_two] {
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
             (guild_id, patch_id, revision_hash, status, sent_message_ids)
         VALUES ($1, $3, $4, 'sending', ARRAY[9928004]::BIGINT[]),
                ($2, $3, $4, 'pending', ARRAY[]::BIGINT[])",
    )
    .bind(guild_one)
    .bind(guild_two)
    .bind(patch_id)
    .bind("a".repeat(64))
    .execute(db.pool())
    .await?;

    let rows: Vec<(i64, String, Vec<i64>)> = sqlx::query_as(
        "SELECT guild_id, status, sent_message_ids
           FROM patchnotes.guild_dispatch
          WHERE patch_id = $1
          ORDER BY guild_id",
    )
    .bind(patch_id)
    .fetch_all(db.pool())
    .await?;
    assert_eq!(
        rows,
        vec![
            (guild_one, "sending".to_string(), vec![9_928_004]),
            (guild_two, "pending".to_string(), Vec::new()),
        ]
    );

    let duplicate = sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash)
         VALUES ($1, $2, $3)",
    )
    .bind(guild_one)
    .bind(patch_id)
    .bind("b".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("Guild/Patch darf nur einen Dispatch-Datensatz haben");
    assert_eq!(
        duplicate
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23505")
    );

    for (guild_id, language) in [(9_928_005_i64, "fr"), (-1_i64, "de")] {
        let invalid = sqlx::query(
            "INSERT INTO patchnotes.guild_settings (guild_id, language)
             VALUES ($1, $2)",
        )
        .bind(guild_id)
        .bind(language)
        .execute(db.pool())
        .await
        .expect_err("ungueltige Guild-ID oder Sprache muss abgelehnt werden");
        assert_eq!(
            invalid
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("23514")
        );
    }

    let invalid_status = sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash, status)
         VALUES ($1, $2, $3, 'unknown')",
    )
    .bind(guild_one)
    .bind(invalid_status_patch_id)
    .bind("c".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("ungueltiger Dispatch-Status muss abgelehnt werden");
    assert_eq!(
        invalid_status
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23514")
    );

    let invalid_patch = sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch (guild_id, patch_id, revision_hash)
         VALUES ($1, $2, $3)",
    )
    .bind(guild_one)
    .bind(9_928_006_i64)
    .bind("d".repeat(64))
    .execute(db.pool())
    .await
    .expect_err("nicht existierender Patch muss abgelehnt werden");
    assert_eq!(
        invalid_patch
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23503")
    );

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

    Ok(())
}
