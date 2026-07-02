use dl_central_db::test_pool;
use dl_server_as_code::{
    adopt_change, apply_preview, db, detect_and_record_drift, diff_models, ApplyOptions,
    BotMessageSpec, CategorySpec, ChannelKind, ChannelSpec, DocumentedException, DynamicNamespace,
    GuildModel, NamespaceMatch, ObjectKind, OverwriteKey, PermissionOverwriteSpec, RoleSpec,
    TargetKind,
};
use serenity::all::Http;
use sqlx::{PgPool, Row};

const GUILD_ID: u64 = 1289721245281292288;
const CATEGORY_ID: u64 = 700;
const CHANNEL_ID: u64 = 701;
const ROLE_ID: u64 = 800;

fn model(role_bits: u64) -> GuildModel {
    let mut model = GuildModel::new(GUILD_ID);
    model.categories.insert(
        CATEGORY_ID,
        CategorySpec {
            guild_id: GUILD_ID,
            category_id: CATEGORY_ID,
            name: "Chat".to_string(),
            position: 1,
        },
    );
    model.channels.insert(
        CHANNEL_ID,
        ChannelSpec {
            guild_id: GUILD_ID,
            channel_id: CHANNEL_ID,
            name: "allgemein".to_string(),
            kind: ChannelKind::Text,
            topic: Some(
                "PLATZHALTER rust/crates/dl-server-as-code/tests/db_workflow.rs:36".to_string(),
            ),
            position: 2,
            parent_category_id: Some(CATEGORY_ID),
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: Some(0),
            status: None,
        },
    );
    model.roles.insert(
        ROLE_ID,
        RoleSpec {
            guild_id: GUILD_ID,
            role_id: ROLE_ID,
            name: "Member".to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: role_bits,
            position: 1,
        },
    );
    model.overwrites.insert(
        OverwriteKey {
            channel_id: CHANNEL_ID,
            target_kind: TargetKind::Role,
            target_id: ROLE_ID,
        },
        PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id: CHANNEL_ID,
                target_kind: TargetKind::Role,
                target_id: ROLE_ID,
            },
            allow_bits: 1,
            deny_bits: 2,
        },
    );
    model
}

async fn insert_desired(pool: &PgPool, model: &GuildModel) -> anyhow::Result<()> {
    for category in model.categories.values() {
        sqlx::query(
            "INSERT INTO server_config.desired_categories (guild_id, category_id, name, position)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(category.guild_id as i64)
        .bind(category.category_id as i64)
        .bind(&category.name)
        .bind(category.position)
        .execute(pool)
        .await?;
    }
    for channel in model.channels.values() {
        sqlx::query(
            "INSERT INTO server_config.desired_channels
             (guild_id, channel_id, name, channel_type, topic, position, parent_category_id,
              nsfw, bitrate, user_limit, rate_limit_per_user, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(channel.guild_id as i64)
        .bind(channel.channel_id as i64)
        .bind(&channel.name)
        .bind(channel.kind.as_db())
        .bind(&channel.topic)
        .bind(channel.position)
        .bind(channel.parent_category_id.map(|id| id as i64))
        .bind(channel.nsfw)
        .bind(channel.bitrate)
        .bind(channel.user_limit)
        .bind(channel.rate_limit_per_user)
        .bind(&channel.status)
        .execute(pool)
        .await?;
    }
    for role in model.roles.values() {
        sqlx::query(
            "INSERT INTO server_config.desired_roles
             (guild_id, role_id, name, color, hoist, mentionable, managed, permissions_bitmask, position)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(role.guild_id as i64)
        .bind(role.role_id as i64)
        .bind(&role.name)
        .bind(role.color)
        .bind(role.hoist)
        .bind(role.mentionable)
        .bind(role.managed)
        .bind(role.permissions_bitmask as i64)
        .bind(role.position)
        .execute(pool)
        .await?;
    }
    for overwrite in model.overwrites.values() {
        sqlx::query(
            "INSERT INTO server_config.desired_permission_overwrites
             (guild_id, channel_id, target_type, target_id, allow_bits, deny_bits)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(overwrite.guild_id as i64)
        .bind(overwrite.key.channel_id as i64)
        .bind(overwrite.key.target_kind.as_db())
        .bind(overwrite.key.target_id as i64)
        .bind(overwrite.allow_bits as i64)
        .bind(overwrite.deny_bits as i64)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn snapshot_persistiert_jeden_lauf_neu_und_laedt_modell() -> anyhow::Result<()> {
    let db = test_pool().await?;
    let first = db::persist_snapshot_model(&db, &model(7), "fixture").await?;
    let second = db::persist_snapshot_model(&db, &model(7), "fixture").await?;

    assert_ne!(first.snapshot_id, second.snapshot_id);
    assert_eq!(first.categories, 1);
    assert_eq!(first.channels, 1);
    assert_eq!(first.roles, 1);
    assert_eq!(first.overwrites, 1);

    let loaded = db::load_snapshot_model(&db, first.snapshot_id).await?;
    assert_eq!(loaded, model(7));
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn desired_bulk_persist_ist_idempotent_und_schreibt_registries() -> anyhow::Result<()> {
    let db = test_pool().await?;
    let mut expanded = model(7);
    expanded.categories.insert(
        798,
        CategorySpec {
            guild_id: GUILD_ID,
            category_id: 798,
            name: "Stale Kategorie".to_string(),
            position: 99,
        },
    );
    expanded.channels.insert(
        799,
        ChannelSpec {
            guild_id: GUILD_ID,
            channel_id: 799,
            name: "stale".to_string(),
            kind: ChannelKind::Text,
            topic: None,
            position: 3,
            parent_category_id: Some(CATEGORY_ID),
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            status: None,
        },
    );
    expanded.roles.insert(
        801,
        RoleSpec {
            guild_id: GUILD_ID,
            role_id: 801,
            name: "Stale Rolle".to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: 123,
            position: 99,
        },
    );
    expanded.overwrites.insert(
        OverwriteKey {
            channel_id: 799,
            target_kind: TargetKind::Role,
            target_id: 801,
        },
        PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id: 799,
                target_kind: TargetKind::Role,
                target_id: 801,
            },
            allow_bits: 8,
            deny_bits: 16,
        },
    );
    expanded.bot_messages.insert(
        (CHANNEL_ID, "panel-main".to_string()),
        BotMessageSpec {
            guild_id: GUILD_ID,
            channel_id: CHANNEL_ID,
            message_key: "panel-main".to_string(),
            message_kind: "panel".to_string(),
            message_id: Some(900),
            content_hash: Some("hash-a".to_string()),
        },
    );
    let namespaces = vec![
        DynamicNamespace {
            namespace_id: None,
            namespace_key: "tickets".to_string(),
            system_name: "TicketTool".to_string(),
            match_rule: NamespaceMatch::NamePrefix("ticket-".to_string()),
        },
        DynamicNamespace {
            namespace_id: None,
            namespace_key: "stale_namespace".to_string(),
            system_name: "old".to_string(),
            match_rule: NamespaceMatch::NamePrefix("old-".to_string()),
        },
    ];
    let exceptions = vec![
        DocumentedException {
            exception_id: None,
            exception_key: "user-ban-1".to_string(),
            object_kind: ObjectKind::PermissionOverwrite,
            channel_id: Some(CHANNEL_ID),
            target_kind: Some(TargetKind::Member),
            target_id: Some(500),
            allow_bits: Some(0),
            deny_bits: Some(2),
            reason: "PLATZHALTER rust/crates/dl-server-as-code/tests/db_workflow.rs:204"
                .to_string(),
        },
        DocumentedException {
            exception_id: None,
            exception_key: "stale-exception".to_string(),
            object_kind: ObjectKind::PermissionOverwrite,
            channel_id: Some(799),
            target_kind: Some(TargetKind::Member),
            target_id: Some(501),
            allow_bits: Some(0),
            deny_bits: Some(4),
            reason: "PLATZHALTER rust/crates/dl-server-as-code/tests/db_workflow.rs:217"
                .to_string(),
        },
    ];
    db::persist_desired_model(&db, &expanded, &namespaces, &exceptions).await?;

    let mut base = model(11);
    base.bot_messages.insert(
        (CHANNEL_ID, "panel-main".to_string()),
        BotMessageSpec {
            guild_id: GUILD_ID,
            channel_id: CHANNEL_ID,
            message_key: "panel-main".to_string(),
            message_kind: "panel".to_string(),
            message_id: Some(901),
            content_hash: Some("hash-b".to_string()),
        },
    );
    db::persist_desired_model(&db, &base, &namespaces[..1], &exceptions[..1]).await?;
    db::persist_desired_model(&db, &base, &namespaces[..1], &exceptions[..1]).await?;

    let loaded = db::load_desired_model(&db, GUILD_ID).await?;
    assert_eq!(
        loaded
            .roles
            .get(&ROLE_ID)
            .map(|role| role.permissions_bitmask),
        Some(11)
    );
    assert!(!loaded.categories.contains_key(&798));
    assert!(!loaded.channels.contains_key(&799));
    assert!(!loaded.roles.contains_key(&801));
    assert!(!loaded.overwrites.contains_key(&OverwriteKey {
        channel_id: 799,
        target_kind: TargetKind::Role,
        target_id: 801,
    }));
    assert_eq!(
        loaded
            .bot_messages
            .get(&(CHANNEL_ID, "panel-main".to_string()))
            .and_then(|message| message.message_id),
        Some(901)
    );

    let loaded_namespaces = db::load_dynamic_namespaces(&db, GUILD_ID).await?;
    assert_eq!(loaded_namespaces.len(), 1);
    assert_eq!(loaded_namespaces[0].namespace_key, "tickets");
    let loaded_exceptions = db::load_documented_exceptions(&db, GUILD_ID).await?;
    assert_eq!(loaded_exceptions.len(), 1);
    assert_eq!(loaded_exceptions[0].exception_key, "user-ban-1");
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn preview_apply_ist_dry_run_per_default_und_prueft_hash() -> anyhow::Result<()> {
    let db = test_pool().await?;
    let desired = model(7);
    let actual = model(3);
    let diff = diff_models(&desired, &actual, &[], &[])?;
    let preview = db::persist_diff_preview(&db, None, &diff, None).await?;
    let http = Http::new("fake-token-for-dry-run");

    let report = apply_preview(
        &db,
        preview.preview_id,
        &preview.diff_hash,
        ApplyOptions::default(),
        &http,
        "server-as-code test dry-run",
    )
    .await?;

    assert!(report.dry_run);
    assert_eq!(report.applied_changes, 0);
    assert!(report
        .change_results
        .iter()
        .all(|result| result.status == "dry_run"));

    let result = apply_preview(
        &db,
        preview.preview_id,
        "falscher-hash",
        ApplyOptions::default(),
        &http,
        "server-as-code test hash mismatch",
    )
    .await;
    let Err(err) = result else {
        anyhow::bail!("hash mismatch muss blockieren");
    };
    assert!(err.to_string().contains("Diff-Hash stimmt nicht"));
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn apply_verweigert_manipuliertes_diff_json_trotz_bestaetigtem_hash() -> anyhow::Result<()> {
    let db = test_pool().await?;
    let desired = model(7);
    let actual = model(3);
    let diff = diff_models(&desired, &actual, &[], &[])?;
    let preview = db::persist_diff_preview(&db, None, &diff, None).await?;
    let mut tampered = diff.clone();
    tampered.changes.clear();
    sqlx::query(
        "UPDATE server_config.diff_previews
            SET diff_json = $2::text::jsonb
          WHERE preview_id = $1",
    )
    .bind(preview.preview_id)
    .bind(serde_json::to_string(&tampered)?)
    .execute(&*db)
    .await?;
    let http = Http::new("fake-token-for-dry-run");

    let result = apply_preview(
        &db,
        preview.preview_id,
        &preview.diff_hash,
        ApplyOptions::default(),
        &http,
        "server-as-code test tamper",
    )
    .await;

    let Err(err) = result else {
        anyhow::bail!("manipuliertes diff_json muss blockieren");
    };
    assert!(err.to_string().contains("Diff-Hash stimmt nicht"));
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn drift_erzeugt_events_ohne_auto_revert_bei_leerer_whitelist() -> anyhow::Result<()> {
    let db = test_pool().await?;
    insert_desired(&db, &model(1)).await?;
    let snapshot = db::persist_snapshot_model(&db, &model(3), "fixture").await?;

    let record = detect_and_record_drift(&db, snapshot.snapshot_id, None).await?;

    assert_eq!(record.preview.diff.changes.len(), 1);
    assert_eq!(record.drift_event_ids.len(), 1);
    let row = sqlx::query(
        "SELECT auto_revert_eligible, severity
           FROM server_config.drift_events
          WHERE drift_event_id = $1",
    )
    .bind(record.drift_event_ids[0])
    .fetch_one(&*db)
    .await?;
    assert!(!row.try_get::<bool, _>("auto_revert_eligible")?);
    assert_eq!(row.try_get::<String, _>("severity")?, "warning");
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn drift_dedupliziert_offene_events_und_resolved_verschwundene_drifts() -> anyhow::Result<()>
{
    let db = test_pool().await?;
    insert_desired(&db, &model(1)).await?;

    let mut last_seen_verlauf = Vec::new();
    for _ in 0..3 {
        let snapshot = db::persist_snapshot_model(&db, &model(3), "fixture").await?;
        let record = detect_and_record_drift(&db, snapshot.snapshot_id, None).await?;
        assert_eq!(record.drift_event_ids.len(), 1);
        let last_seen: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT last_seen_at FROM server_config.drift_events WHERE guild_id = $1",
        )
        .bind(GUILD_ID as i64)
        .fetch_one(&*db)
        .await?;
        last_seen_verlauf.push(last_seen);
    }
    // Wiederholter gleicher Drift verlängert das Sichtungsfenster des EINEN Events.
    assert!(last_seen_verlauf[2] > last_seen_verlauf[0]);

    let (total_events, open_events): (i64, i64) = sqlx::query_as(
        "SELECT count(*) AS total_events,
                count(*) FILTER (WHERE resolved_at IS NULL) AS open_events
           FROM server_config.drift_events
          WHERE guild_id = $1",
    )
    .bind(GUILD_ID as i64)
    .fetch_one(&*db)
    .await?;
    assert_eq!(total_events, 1);
    assert_eq!(open_events, 1);

    let snapshot = db::persist_snapshot_model(&db, &model(1), "fixture").await?;
    let record = detect_and_record_drift(&db, snapshot.snapshot_id, None).await?;
    assert_eq!(record.drift_event_ids.len(), 0);

    let (open_events, resolved_events): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE resolved_at IS NULL) AS open_events,
                count(*) FILTER (WHERE resolved_at IS NOT NULL) AS resolved_events
           FROM server_config.drift_events
          WHERE guild_id = $1",
    )
    .bind(GUILD_ID as i64)
    .fetch_one(&*db)
    .await?;
    assert_eq!(open_events, 0);
    assert_eq!(resolved_events, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn adopt_uebernimmt_live_aenderung_ins_soll_modell() -> anyhow::Result<()> {
    let db = test_pool().await?;
    insert_desired(&db, &model(1)).await?;
    let snapshot = db::persist_snapshot_model(&db, &model(3), "fixture").await?;
    let desired = db::load_desired_model(&db, GUILD_ID).await?;
    let actual = db::load_snapshot_model(&db, snapshot.snapshot_id).await?;
    let diff = diff_models(&desired, &actual, &[], &[])?;
    let change = diff
        .changes
        .iter()
        .find(|change| change.object.object_id == ROLE_ID)
        .ok_or_else(|| anyhow::anyhow!("role drift"))?;

    adopt_change(&db, snapshot.snapshot_id, change, None).await?;
    let adopted = db::load_desired_model(&db, GUILD_ID).await?;

    assert_eq!(
        adopted
            .roles
            .get(&ROLE_ID)
            .map(|role| role.permissions_bitmask),
        Some(3)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "braucht CENTRAL_TEST_DSN/DATABASE_URL/DEADLOCK_CENTRAL_DSN"]
async fn adopt_entfernt_kindzeilen_beim_entfernen_eines_soll_channels() -> anyhow::Result<()> {
    let db = test_pool().await?;
    insert_desired(&db, &model(1)).await?;
    sqlx::query(
        "INSERT INTO server_config.desired_bot_messages
         (guild_id, channel_id, message_key, message_kind, message_id, expected_hash)
         VALUES ($1, $2, 'panel-main', 'panel', $3, 'hash-a')",
    )
    .bind(GUILD_ID as i64)
    .bind(CHANNEL_ID as i64)
    .bind(900_i64)
    .execute(&*db)
    .await?;

    let mut actual = model(1);
    actual.channels.clear();
    actual.overwrites.clear();
    let snapshot = db::persist_snapshot_model(&db, &actual, "fixture").await?;
    let desired = db::load_desired_model(&db, GUILD_ID).await?;
    let actual = db::load_snapshot_model(&db, snapshot.snapshot_id).await?;
    let diff = diff_models(&desired, &actual, &[], &[])?;
    let change = diff
        .changes
        .iter()
        .find(|change| {
            change.object.kind == dl_server_as_code::ObjectKind::Channel
                && change.object.object_id == CHANNEL_ID
        })
        .ok_or_else(|| anyhow::anyhow!("channel create drift"))?;

    adopt_change(&db, snapshot.snapshot_id, change, None).await?;

    let (channels, overwrites, bot_messages): (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM server_config.desired_channels WHERE guild_id = $1 AND channel_id = $2) AS channels,
            (SELECT count(*) FROM server_config.desired_permission_overwrites WHERE guild_id = $1 AND channel_id = $2) AS overwrites,
            (SELECT count(*) FROM server_config.desired_bot_messages WHERE guild_id = $1 AND channel_id = $2) AS bot_messages",
    )
    .bind(GUILD_ID as i64)
    .bind(CHANNEL_ID as i64)
    .fetch_one(&*db)
    .await?;
    assert_eq!((channels, overwrites, bot_messages), (0, 0, 0));
    Ok(())
}
