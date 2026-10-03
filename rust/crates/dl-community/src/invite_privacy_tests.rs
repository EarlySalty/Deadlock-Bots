use super::*;
use chrono::{Duration, Utc};
use dl_central_db::{
    platform_connections::{upsert_twitch_connection, TwitchConnection},
    test_pool,
};

async fn fixture(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    upsert_twitch_connection(
        pool,
        42,
        &TwitchConnection {
            twitch_user_id: "111".into(),
            twitch_login: "eigener_kanal".into(),
            verified: true,
        },
    )
    .await?;
    let joined = Utc::now() - Duration::days(15);
    for (uid, join) in [(42_i64, 900_i64), (43, 901)] {
        sqlx::query("INSERT INTO activity.member_events(id,guild_id,user_id,event_type,occurred_at,metadata)
            VALUES($1,1,$2,'join',$3,$4)")
            .bind(join).bind(uid).bind(joined).bind(serde_json::json!({
                "invite_code":format!("Code{join}"),"inviter_id":"111",
                "inviter_name":"eigener_kanal","invite_url":format!("https://discord.gg/Code{join}"),
                "discord_joined_at":joined.to_rfc3339(),"join_source_bucket":"twitch"}))
            .execute(pool).await?;
        sqlx::query("INSERT INTO activity.twitch_invite_members(guild_id,user_id,first_join_id,first_joined_at,prior_member,voice_channel_id,voice_started_at) VALUES(1,$1,$2,$3,FALSE,99,$3)")
            .bind(uid).bind(join).bind(joined).execute(pool).await?;
        sqlx::query("INSERT INTO bot.twitch_invite_joins(join_id,guild_id,user_id,streamer_login,streamer_twitch_user_id,inviter_twitch_user_id,invite_code,joined_at,eligible,reason) VALUES($1,1,$2,'fremder_kanal','222','111',$3,$4,TRUE,'Invite von eigener_kanal')")
            .bind(join).bind(uid).bind(format!("Code{join}")).bind(joined).execute(pool).await?;
    }
    sqlx::query(
        "UPDATE bot.twitch_invite_joins SET status='qualified',qualified_at=$1 WHERE join_id=900",
    )
    .bind(joined + Duration::days(14))
    .execute(pool)
    .await?;
    dl_central_db::community_points::import_qualified_join_ledger(pool).await?;
    sqlx::query("INSERT INTO activity.twitch_invite_messages VALUES(777,1,42,clock_timestamp())")
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO activity.twitch_invite_evidence_queue(guild_id,user_id) VALUES(1,42) ON CONFLICT DO NOTHING").execute(pool).await?;
    sqlx::query("INSERT INTO bot.twitch_personal_invites(streamer_twitch_user_id,inviter_twitch_user_id,streamer_login,guild_id,channel_id,invite_code,invite_url) VALUES('222','111','fremder_kanal',1,99,'PersonalCode','https://discord.gg/PersonalCode'),('222','333','fremder_kanal',1,99,'FremdCode','https://discord.gg/FremdCode')").execute(pool).await?;
    sqlx::query("INSERT INTO bot.twitch_streamer_invite_code_history(source_login_snapshot,guild_id,invite_code,twitch_user_id,channel_id,valid_from,attribution_safe) VALUES('eigener_kanal',1,'HistoryCode','111',99,clock_timestamp(),TRUE),('fremder_kanal',1,'FremdHistory','333',99,clock_timestamp(),TRUE)").execute(pool).await?;
    sqlx::query(
        "INSERT INTO patchnotes.guild_settings(guild_id,updated_by_user_id) VALUES(1,42),(2,43)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[tokio::test]
async fn export_erasure_minimalnachweis_und_fremdzeilen() -> Result<(), Box<dyn std::error::Error>>
{
    let db = test_pool().await?;
    let pool = db.pool();
    fixture(pool).await?;
    let exported = crate::privacy::export_user_data(pool, 42, Utc::now().timestamp()).await?;
    let rows = &exported["tables"]["invite_privacy.bot.twitch_invite_joins"];
    assert!(rows.to_string().contains("900"));
    assert!(!rows.to_string().contains("\"user_id\":43"));
    assert!(!rows.to_string().contains("222"));
    let foreign_history:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(t)-'reason' FROM bot.twitch_invite_transitions t WHERE join_id=901 ORDER BY id").fetch_all(pool).await?;
    crate::privacy::delete_user_data(pool, 42, "user_request".into(), Utc::now().timestamp())
        .await?;
    for table in [
        "activity.twitch_invite_members",
        "activity.twitch_invite_messages",
        "activity.twitch_invite_evidence_queue",
        "bot.twitch_invite_joins",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table} WHERE user_id=42"))
                .fetch_one(pool)
                .await?,
            0
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM bot.twitch_invite_transitions WHERE join_id=900"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM community_points.ledger WHERE ref='qualified_invite:900' OR meta->>'join_id'='900'").fetch_one(pool).await?,0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT sum(points) FROM community_points.ledger WHERE streamer_twitch_user_id='222'"
        )
        .fetch_one(pool)
        .await?,
        50
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM bot.twitch_personal_invites WHERE inviter_twitch_user_id='111' OR streamer_twitch_user_id='111'").fetch_one(pool).await?,0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM bot.twitch_personal_invites WHERE inviter_twitch_user_id='333'"
        )
        .fetch_one(pool)
        .await?,
        1
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM bot.twitch_streamer_invite_code_history WHERE twitch_user_id='111'").fetch_one(pool).await?,0);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM bot.twitch_invite_joins WHERE user_id=43 AND inviter_twitch_user_id IS NULL").fetch_one(pool).await?,1);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM activity.member_events
        WHERE id=901 AND (metadata ? 'inviter_id' OR metadata ? 'invite_code' OR metadata ? 'inviter_name')")
        .fetch_one(pool).await?,0);
    assert_eq!(foreign_history,sqlx::query_scalar::<_,Value>("SELECT to_jsonb(t)-'reason' FROM bot.twitch_invite_transitions t WHERE join_id=901 ORDER BY id").fetch_all(pool).await?);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM bot.twitch_invite_transitions WHERE join_id=901 AND reason IS NOT NULL").fetch_one(pool).await?,0);
    let settings: Vec<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT guild_id,updated_by_user_id FROM patchnotes.guild_settings ORDER BY guild_id",
    )
    .fetch_all(pool)
    .await?;
    assert_eq!(settings, vec![(1, None), (2, Some(43))]);
    let exported = crate::privacy::export_user_data(pool, 42, Utc::now().timestamp()).await?;
    let proof = &exported["tables"]["invite_privacy.member_privacy"];
    assert_eq!(proof.as_array().unwrap().len(), 1);
    assert!(proof[0]["subject_hash"].is_string());
    assert!(proof[0].get("user_id").is_none());
    assert!(proof[0].get("voice_started_at").is_none());
    Ok(())
}

#[tokio::test]
async fn wiedereinwilligung_verwirft_altwerte_und_erzeugt_keinen_zweiten_erstcredit(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_pool().await?;
    let pool = db.pool();
    fixture(pool).await?;
    crate::privacy::delete_user_data(pool, 42, "user_request".into(), Utc::now().timestamp())
        .await?;
    crate::privacy::set_opt_in(pool, 42, Utc::now().timestamp()).await?;
    upsert_twitch_connection(
        pool,
        42,
        &TwitchConnection {
            twitch_user_id: "111".into(),
            twitch_login: "eigener_kanal".into(),
            verified: true,
        },
    )
    .await?;
    let mut tx = pool.begin().await?;
    dl_central_db::platform_connections::lock_twitch_identity(&mut tx).await?;
    sqlx::query("INSERT INTO activity.twitch_invite_members(guild_id,user_id,first_join_id,first_joined_at,prior_member) VALUES(1,42,999,clock_timestamp(),FALSE)").execute(&mut *tx).await?;
    tx.commit().await?;
    let prior: bool = sqlx::query_scalar(
        "SELECT prior_member FROM activity.twitch_invite_members WHERE user_id=42 AND guild_id=1",
    )
    .fetch_one(pool)
    .await?;
    assert!(prior);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM bot.twitch_invite_joins WHERE user_id=42"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM activity.twitch_invite_messages WHERE user_id=42"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    Ok(())
}

async fn wait_for_identity_waiter(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND NOT granted
                 AND database=(SELECT oid FROM pg_database WHERE datname=current_database())
                 AND classid=hashtext('core.discord_platform_connections')::OID
                 AND objid=hashtext('twitch_reassignment')::OID)",
            )
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

async fn begin_erasure(pool: &PgPool) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    dl_central_db::lock_raw_event_retention_erasure(&mut tx).await?;
    dl_central_db::platform_connections::lock_twitch_identity(&mut tx).await?;
    dl_central_db::lock_user_privacy(&mut tx, 42).await?;
    dl_central_db::community_points::block_linked_viewer_imports(&mut tx, 42).await?;
    sqlx::query(
        "INSERT INTO core.user_privacy(user_id,opted_out,deleted_at,reason,updated_at)
        VALUES(42,TRUE,clock_timestamp(),'user_request',clock_timestamp())
        ON CONFLICT(user_id) DO UPDATE SET opted_out=TRUE,deleted_at=excluded.deleted_at,
            reason=excluded.reason,updated_at=excluded.updated_at",
    )
    .execute(&mut *tx)
    .await?;
    let mut relations: HashSet<&str> = HANDLED_COLUMNS.iter().map(|(table, _)| *table).collect();
    relations.insert(MEMBER_PRIVACY);
    relations.insert("core.discord_platform_connections");
    erase(&mut tx, 42, &relations).await?;
    Ok(tx)
}

#[derive(Clone, Copy, Debug)]
enum Writer {
    Queue,
    Message,
    Voice,
    Attribution,
    Personal,
    History,
}

async fn write_probe(
    tx: &mut Transaction<'_, Postgres>,
    writer: Writer,
) -> Result<u64, sqlx::Error> {
    let query = match writer {
        Writer::Queue => "INSERT INTO activity.twitch_invite_evidence_queue(guild_id,user_id) VALUES(1,42) ON CONFLICT(guild_id,user_id) DO UPDATE SET marked_at=clock_timestamp()",
        Writer::Message => "INSERT INTO activity.twitch_invite_messages VALUES(888,1,42,clock_timestamp())",
        Writer::Voice => "UPDATE activity.twitch_invite_members SET voice_channel_id=100,
            voice_observed_at=clock_timestamp() WHERE guild_id=1 AND user_id=42",
        Writer::Attribution => "INSERT INTO bot.twitch_invite_joins(join_id,guild_id,user_id,
            streamer_login,streamer_twitch_user_id,inviter_twitch_user_id,invite_code,joined_at,eligible)
            VALUES(902,1,44,'fremder_kanal','222','111','LateCode',clock_timestamp(),TRUE)",
        Writer::Personal => "INSERT INTO bot.twitch_personal_invites(streamer_twitch_user_id,
            inviter_twitch_user_id,streamer_login,guild_id,channel_id,invite_code,invite_url)
            VALUES('444','111','weiterer_kanal',1,99,'LatePersonal','https://discord.gg/LatePersonal')",
        Writer::History => "INSERT INTO bot.twitch_streamer_invite_code_history(source_login_snapshot,
            guild_id,invite_code,twitch_user_id,channel_id,valid_from,attribution_safe)
            VALUES('eigener_kanal',1,'LateHistory','111',99,clock_timestamp(),TRUE)",
    };
    Ok(sqlx::query(query).execute(&mut **tx).await?.rows_affected())
}

async fn assert_erased(pool: &PgPool) -> Result<(), sqlx::Error> {
    for table in [
        "activity.twitch_invite_members",
        "activity.twitch_invite_messages",
        "activity.twitch_invite_evidence_queue",
        "bot.twitch_invite_joins",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table} WHERE user_id=42"))
                .fetch_one(pool)
                .await?,
            0,
            "{table}"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM bot.twitch_invite_joins
        WHERE inviter_twitch_user_id='111' OR streamer_twitch_user_id='111'"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM bot.twitch_personal_invites
        WHERE inviter_twitch_user_id='111' OR streamer_twitch_user_id='111'"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM bot.twitch_streamer_invite_code_history
        WHERE twitch_user_id='111'"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn writer_vor_erasure_wird_vollstaendig_geloescht() -> Result<(), Box<dyn std::error::Error>>
{
    for writer in [
        Writer::Queue,
        Writer::Message,
        Writer::Voice,
        Writer::Attribution,
        Writer::Personal,
        Writer::History,
    ] {
        let db = test_pool().await?;
        let pool = db.pool();
        fixture(pool).await?;
        let mut held = pool.begin().await?;
        dl_central_db::platform_connections::lock_twitch_identity(&mut held).await?;
        assert_eq!(write_probe(&mut held, writer).await?, 1);
        let other = pool.clone();
        let erasure = tokio::spawn(async move {
            crate::privacy::delete_user_data(
                &other,
                42,
                "user_request".into(),
                Utc::now().timestamp(),
            )
            .await
        });
        wait_for_identity_waiter(pool).await?;
        assert!(
            !erasure.is_finished(),
            "{writer:?}: Erasure muss vor Zeilenlocks warten"
        );
        held.commit().await?;
        tokio::time::timeout(std::time::Duration::from_secs(10), erasure).await???;
        assert_erased(pool).await?;
    }
    Ok(())
}

#[tokio::test]
async fn erasure_vor_writer_blockiert_alle_invite_kopien() -> Result<(), Box<dyn std::error::Error>>
{
    for writer in [
        Writer::Queue,
        Writer::Message,
        Writer::Voice,
        Writer::Attribution,
        Writer::Personal,
        Writer::History,
    ] {
        let db = test_pool().await?;
        let pool = db.pool();
        fixture(pool).await?;
        let held = begin_erasure(pool).await?;
        let other = pool.clone();
        let pending = tokio::spawn(async move {
            let mut tx = other.begin().await?;
            // Der echte BEFORE-STATEMENT-Trigger muss ohne vorgeschalteten Rustlock warten.
            let result = write_probe(&mut tx, writer).await;
            match &result {
                Ok(_) => tx.commit().await?,
                Err(_) => tx.rollback().await?,
            }
            result
        });
        wait_for_identity_waiter(pool).await?;
        assert!(
            !pending.is_finished(),
            "{writer:?}: Writer muss vor Zeilenlocks warten"
        );
        held.commit().await?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), pending).await??;
        if matches!(writer, Writer::Personal) {
            let err =
                result.expect_err("Gesperrte persönliche Attribution muss ehrlich fehlschlagen");
            assert_eq!(
                err.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("23514")
            );
        } else {
            assert_eq!(result?, 0, "{writer:?}");
        }
        assert_erased(pool).await?;
    }
    Ok(())
}

#[tokio::test]
async fn alte_herkunft_bleibt_gesperrt_neue_links_sind_nach_consent_zulaessig(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_pool().await?;
    let pool = db.pool();
    fixture(pool).await?;
    let old: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await?;
    crate::privacy::delete_user_data(pool, 42, "user_request".into(), Utc::now().timestamp())
        .await?;
    crate::privacy::set_opt_in(pool, 42, old.timestamp()).await?;
    // Opt-in allein darf keine Zuordnung freigeben.
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT bot.invite_privacy_twitch_allowed('111',clock_timestamp())"
        )
        .fetch_one(pool)
        .await?
    );
    upsert_twitch_connection(
        pool,
        42,
        &TwitchConnection {
            twitch_user_id: "111".into(),
            twitch_login: "eigener_kanal".into(),
            verified: true,
        },
    )
    .await?;
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT bot.invite_privacy_twitch_allowed('111',$1)")
            .bind(old)
            .fetch_one(pool)
            .await?
    );
    let blocked = sqlx::query(
        "INSERT INTO bot.twitch_streamer_invite_code_history(source_login_snapshot,
        guild_id,invite_code,twitch_user_id,valid_from,attribution_safe)
        VALUES('eigener_kanal',1,'Alt','111',$1,TRUE)",
    )
    .bind(old)
    .execute(pool)
    .await?;
    assert_eq!(blocked.rows_affected(), 0);
    let blocked = sqlx::query("INSERT INTO activity.twitch_invite_messages VALUES(889,1,42,$1)")
        .bind(old)
        .execute(pool)
        .await?;
    assert_eq!(blocked.rows_affected(), 0);
    let mut tx = pool.begin().await?;
    dl_central_db::platform_connections::lock_twitch_identity(&mut tx).await?;
    assert_eq!(write_probe(&mut tx, Writer::Personal).await?, 1);
    assert_eq!(write_probe(&mut tx, Writer::History).await?, 1);
    tx.commit().await?;
    // Normale Guards bleiben auch bei vorhandenen Privacy-Nachweisen wirksam.
    assert!(
        sqlx::query("DELETE FROM bot.twitch_invite_joins WHERE user_id=43")
            .execute(pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM activity.twitch_invite_members WHERE user_id=43")
            .execute(pool)
            .await
            .is_err()
    );
    Ok(())
}

async fn prepare_dispatch(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE patchnotes.guild_settings SET enabled=TRUE,channel_id=99,approval_mode='manual' WHERE guild_id=1")
        .execute(pool).await?;
    sqlx::query("INSERT INTO patchnotes.changelog_posts(id,title,url) VALUES(998,'Prüfpatch','https://example.org/privacy-patch')")
        .execute(pool).await?;
    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch(guild_id,patch_id,revision_hash,status)
        VALUES(1,998,repeat('a',64),'awaiting_approval')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn approve(tx: &mut Transaction<'_, Postgres>) -> Result<bool, sqlx::Error> {
    sqlx::query("SET LOCAL ROLE dl_patchnotes_dml")
        .execute(&mut **tx)
        .await?;
    sqlx::query_scalar("SELECT patchnotes.approve_dispatch(1,998,repeat('a',64),42)")
        .fetch_one(&mut **tx)
        .await
}

#[tokio::test]
async fn patchnotes_export_erasure_erhaelt_zustellung_und_guildkonfiguration(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_pool().await?;
    let pool = db.pool();
    fixture(pool).await?;
    prepare_dispatch(pool).await?;
    let mut tx = pool.begin().await?;
    assert!(approve(&mut tx).await?);
    tx.commit().await?;
    sqlx::query("UPDATE patchnotes.guild_dispatch SET status='sending',send_channel_id=99,
        send_attempt_id='00000000-0000-0000-0000-000000000003'::UUID,
        send_started_at=statement_timestamp(),send_lease_expires_at=statement_timestamp()+interval '10 minutes'
        WHERE guild_id=1 AND patch_id=998").execute(pool).await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch SET status='sent',sent_message_ids=ARRAY[997]::BIGINT[],
        send_lease_expires_at=NULL WHERE guild_id=1 AND patch_id=998",
    )
    .execute(pool)
    .await?;
    let exported = crate::privacy::export_user_data(pool, 42, Utc::now().timestamp()).await?;
    assert_eq!(
        exported["tables"]["invite_privacy.patchnotes.guild_dispatch"][0]["approved_by_user_id"],
        42
    );
    let delivery: Value = sqlx::query_scalar(
        "SELECT to_jsonb(d)-ARRAY['approved_by_user_id','updated_at']
        FROM patchnotes.guild_dispatch d WHERE guild_id=1 AND patch_id=998",
    )
    .fetch_one(pool)
    .await?;
    let settings: Value = sqlx::query_scalar(
        "SELECT to_jsonb(s)-ARRAY['updated_by_user_id','updated_at']
        FROM patchnotes.guild_settings s WHERE guild_id=1",
    )
    .fetch_one(pool)
    .await?;
    crate::privacy::delete_user_data(pool, 42, "user_request".into(), Utc::now().timestamp())
        .await?;
    assert_eq!(
        delivery,
        sqlx::query_scalar::<_, Value>(
            "SELECT to_jsonb(d)-ARRAY['approved_by_user_id','updated_at']
        FROM patchnotes.guild_dispatch d WHERE guild_id=1 AND patch_id=998"
        )
        .fetch_one(pool)
        .await?
    );
    assert_eq!(
        settings,
        sqlx::query_scalar::<_, Value>(
            "SELECT to_jsonb(s)-ARRAY['updated_by_user_id','updated_at']
        FROM patchnotes.guild_settings s WHERE guild_id=1"
        )
        .fetch_one(pool)
        .await?
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT approved_by_user_id FROM patchnotes.guild_dispatch
        WHERE guild_id=1 AND patch_id=998"
        )
        .fetch_one(pool)
        .await?,
        None
    );
    Ok(())
}

#[tokio::test]
async fn patchnotes_writer_und_erasure_in_beiden_sperrreihenfolgen(
) -> Result<(), Box<dyn std::error::Error>> {
    for writer_first in [true, false] {
        let db = test_pool().await?;
        let pool = db.pool();
        fixture(pool).await?;
        prepare_dispatch(pool).await?;
        if writer_first {
            let mut held = pool.begin().await?;
            dl_central_db::platform_connections::lock_twitch_identity(&mut held).await?;
            assert!(approve(&mut held).await?);
            sqlx::query(
                "UPDATE patchnotes.guild_settings SET updated_by_user_id=42 WHERE guild_id=1",
            )
            .execute(&mut *held)
            .await?;
            let other = pool.clone();
            let erasure = tokio::spawn(async move {
                crate::privacy::delete_user_data(
                    &other,
                    42,
                    "user_request".into(),
                    Utc::now().timestamp(),
                )
                .await
            });
            wait_for_identity_waiter(pool).await?;
            assert!(!erasure.is_finished());
            held.commit().await?;
            tokio::time::timeout(std::time::Duration::from_secs(10), erasure).await???;
        } else {
            let held = begin_erasure(pool).await?;
            let other = pool.clone();
            let writer = tokio::spawn(async move {
                let mut tx = other.begin().await?;
                assert!(approve(&mut tx).await?);
                sqlx::query(
                    "UPDATE patchnotes.guild_settings SET updated_by_user_id=42 WHERE guild_id=1",
                )
                .execute(&mut *tx)
                .await?;
                tx.commit().await
            });
            wait_for_identity_waiter(pool).await?;
            assert!(!writer.is_finished());
            held.commit().await?;
            tokio::time::timeout(std::time::Duration::from_secs(10), writer).await???;
        }
        assert_eq!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT approved_by_user_id FROM patchnotes.guild_dispatch
            WHERE guild_id=1 AND patch_id=998"
            )
            .fetch_one(pool)
            .await?,
            None
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT updated_by_user_id FROM patchnotes.guild_settings
            WHERE guild_id=1"
            )
            .fetch_one(pool)
            .await?,
            None
        );
        let state: (String, bool) = sqlx::query_as(
            "SELECT status,approved_at IS NOT NULL FROM patchnotes.guild_dispatch
            WHERE guild_id=1 AND patch_id=998",
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(state, ("pending".into(), true));
    }
    Ok(())
}

#[tokio::test]
async fn patchnotes_direktwriter_mehrzeilen_und_funktionsrechte(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = test_pool().await?;
    let pool = db.pool();
    fixture(pool).await?;
    prepare_dispatch(pool).await?;
    crate::privacy::delete_user_data(pool, 42, "user_request".into(), Utc::now().timestamp())
        .await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE dl_patchnotes_dml")
        .execute(&mut *tx)
        .await?;
    // Auch bei vorgelagertem FOR UPDATE gilt der gemeinsame Writervertrag.
    dl_central_db::platform_connections::lock_twitch_identity(&mut tx).await?;
    sqlx::query("SELECT guild_id FROM patchnotes.guild_settings WHERE guild_id IN (1,2) ORDER BY guild_id FOR UPDATE")
        .fetch_all(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO patchnotes.guild_settings(guild_id,updated_by_user_id) VALUES(3,42),(4,42)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE patchnotes.guild_settings SET updated_by_user_id=42 WHERE guild_id IN (1,2)",
    )
    .execute(&mut *tx)
    .await?;
    // Der ursprüngliche Funktionsübergang bleibt pending und behält approved_at.
    assert!(approve(&mut tx).await?);
    tx.commit().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM patchnotes.guild_settings
        WHERE guild_id IN (1,2,3,4) AND updated_by_user_id IS NOT NULL"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    let state: (String, Option<i64>, bool) = sqlx::query_as(
        "SELECT status,approved_by_user_id,approved_at IS NOT NULL
        FROM patchnotes.guild_dispatch WHERE guild_id=1 AND patch_id=998",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(state, ("pending".into(), None, true));
    // Direkte INSERT/UPDATE-Übergänge werden ebenfalls zentral gescrubbt.
    sqlx::query("INSERT INTO patchnotes.guild_dispatch(guild_id,patch_id,revision_hash,status)
        VALUES(1,998,repeat('b',64),'awaiting_approval'),(1,998,repeat('c',64),'awaiting_approval')")
        .execute(pool).await?;
    sqlx::query(
        "UPDATE patchnotes.guild_dispatch SET status='pending',approved_by_user_id=42
        WHERE guild_id=1 AND patch_id=998 AND revision_hash IN (repeat('b',64),repeat('c',64))",
    )
    .execute(pool)
    .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM patchnotes.guild_dispatch
        WHERE approved_by_user_id=42"
        )
        .fetch_one(pool)
        .await?,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM patchnotes.guild_dispatch
        WHERE guild_id=1 AND patch_id=998 AND status='pending' AND approved_at IS NOT NULL"
        )
        .fetch_one(pool)
        .await?,
        3
    );
    let rights:(bool,bool,bool)=sqlx::query_as("SELECT
        has_function_privilege('dl_patchnotes_dml','core.scrub_opted_out_patchnotes_actor()','EXECUTE'),
        has_function_privilege('dl_patchnotes_dml','patchnotes.anonymize_dispatch_approvals(bigint)','EXECUTE'),
        has_function_privilege('dl_patchnotes_dml','patchnotes.approve_dispatch(bigint,bigint,text,bigint)','EXECUTE')")
        .fetch_one(pool).await?;
    assert_eq!(rights, (false, false, true));
    let owner_ok:bool=sqlx::query_scalar("SELECT p.prosecdef
        AND p.proconfig @> ARRAY['search_path=pg_catalog']
        AND has_function_privilege(p.proowner,'patchnotes.anonymize_dispatch_approvals(bigint)','EXECUTE')
        AND has_table_privilege(p.proowner,'core.user_privacy','SELECT')
        AND has_table_privilege(p.proowner,'patchnotes.guild_settings','UPDATE')
        FROM pg_proc p WHERE p.oid='core.scrub_opted_out_patchnotes_actor()'::regprocedure")
        .fetch_one(pool).await?;
    assert!(owner_ok);
    // Die bestehende Privacyrolle kann die unveränderte Funktion regulär nutzen.
    sqlx::query("UPDATE patchnotes.guild_settings SET approval_mode='manual' WHERE guild_id=2")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO patchnotes.guild_dispatch(guild_id,patch_id,revision_hash,status)
        VALUES(2,998,repeat('d',64),'awaiting_approval')",
    )
    .execute(pool)
    .await?;
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT patchnotes.approve_dispatch(2,998,repeat('d',64),43)"
        )
        .fetch_one(pool)
        .await?
    );
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL ROLE dl_patchnotes_privacy")
        .execute(&mut *tx)
        .await?;
    let changed: i64 = sqlx::query_scalar("SELECT patchnotes.anonymize_dispatch_approvals(43)")
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(changed, 1);
    tx.commit().await?;
    Ok(())
}
