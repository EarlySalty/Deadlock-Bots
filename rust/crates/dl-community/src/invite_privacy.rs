//! Zielgebundener Privacyanschluss für Invitekopien und Patchnotes-Akteure.
//! Hashnachweise bleiben personenbezogen und werden ausdrücklich exportiert.
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, HashSet};

pub(crate) const HANDLED_COLUMNS: &[(&str, &str)] = &[
    ("activity.twitch_invite_members", "user_id"),
    ("activity.twitch_invite_messages", "user_id"),
    ("activity.twitch_invite_evidence_queue", "user_id"),
    ("bot.twitch_invite_joins", "user_id"),
    ("bot.twitch_invite_joins", "inviter_twitch_user_id"),
    ("bot.twitch_invite_joins", "streamer_twitch_user_id"),
    ("bot.twitch_personal_invites", "inviter_twitch_user_id"),
    ("bot.twitch_personal_invites", "streamer_twitch_user_id"),
    ("bot.twitch_streamer_invite_code_history", "twitch_user_id"),
    ("patchnotes.guild_dispatch", "approved_by_user_id"),
    ("patchnotes.guild_settings", "updated_by_user_id"),
];
const MEMBER_PRIVACY: &str = "activity.twitch_invite_member_privacy";

/// Aufruf vor Link-/USER_TABLES-Löschung, unter Identitäts- und Nutzerlock.
pub(crate) async fn erase(
    tx: &mut Transaction<'_, Postgres>,
    user: i64,
    relations: &HashSet<&str>,
) -> Result<BTreeMap<String, i64>, sqlx::Error> {
    let mut counts = BTreeMap::new();
    if relations.contains("bot.twitch_invite_joins") {
        sqlx::query("INSERT INTO activity.twitch_invite_member_privacy(guild_id,subject_hash)
            SELECT DISTINCT guild_id,sha256(convert_to('twitch-invites:discord-member-privacy:v1:' || $1::TEXT,'UTF8'))
              FROM activity.twitch_invite_members WHERE user_id=$1
            UNION SELECT DISTINCT guild_id,sha256(convert_to('twitch-invites:discord-member-privacy:v1:' || $1::TEXT,'UTF8'))
              FROM bot.twitch_invite_joins WHERE user_id=$1 ON CONFLICT DO NOTHING")
            .bind(user).execute(&mut **tx).await?;
        sqlx::query("SELECT set_config('community.invite_erasure_user_id',$1,TRUE)")
            .bind(user.to_string())
            .execute(&mut **tx)
            .await?;
        // Vor einer Identitätsscrub oder Joinlöschung alle erreichbaren Verweise lösen.
        sqlx::query("UPDATE community_points.ledger l SET ref='qualified_erased:' || gen_random_uuid()::TEXT,
                meta='{}'::JSONB,
                occurred_at=date_trunc('day',l.occurred_at AT TIME ZONE 'Europe/Berlin') AT TIME ZONE 'Europe/Berlin',
                created_at=date_trunc('day',l.created_at AT TIME ZONE 'Europe/Berlin') AT TIME ZONE 'Europe/Berlin'
             FROM bot.twitch_invite_joins j WHERE j.user_id=$1
               AND l.source='streamer_qualified_join' AND l.ref='qualified_invite:' || j.join_id::TEXT")
            .bind(user).execute(&mut **tx).await?;
        let affected = "SELECT join_id FROM bot.twitch_invite_joins WHERE user_id=$1
            OR streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch')
            OR inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch')";
        let deleted = sqlx::query(
            "DELETE FROM bot.twitch_invite_transitions WHERE join_id IN (SELECT join_id FROM bot.twitch_invite_joins WHERE user_id=$1)"
        )
        .bind(user)
        .execute(&mut **tx)
        .await?;
        counts.insert(
            "invite_privacy.transitions".into(),
            deleted.rows_affected() as i64,
        );
        // Fremde Status-/Zeitnachweise bleiben erhalten, nur persönlicher Freitext entfällt.
        sqlx::query(&format!(
            "UPDATE bot.twitch_invite_transitions SET reason=NULL WHERE reason IS NOT NULL
            AND join_id IN ({affected})"
        ))
        .bind(user)
        .execute(&mut **tx)
        .await?;
        sqlx::query(&format!("UPDATE activity.member_events SET metadata=metadata - ARRAY[
            'twitch_streamer_login','invite_code','invite_url','inviter_id','inviter_name','inviter_bot',
            'invite_channel_id','invite_channel_name','join_source_label'] WHERE id IN ({affected})"))
            .bind(user).execute(&mut **tx).await?;
        let deleted = sqlx::query("DELETE FROM bot.twitch_invite_joins WHERE user_id=$1")
            .bind(user)
            .execute(&mut **tx)
            .await?;
        counts.insert(
            "invite_privacy.joins".into(),
            deleted.rows_affected() as i64,
        );
        sqlx::query("UPDATE bot.twitch_invite_joins j SET
             streamer_login=CASE WHEN streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch') THEN '' ELSE streamer_login END,
             streamer_twitch_user_id=CASE WHEN streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch') THEN NULL ELSE streamer_twitch_user_id END,
             inviter_twitch_user_id=CASE WHEN inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch') THEN NULL ELSE inviter_twitch_user_id END,
             invite_code='',reason=NULL WHERE streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch')
             OR inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch')")
            .bind(user).execute(&mut **tx).await?;
        for table in [
            "activity.twitch_invite_evidence_queue",
            "activity.twitch_invite_messages",
            "activity.twitch_invite_members",
        ] {
            let deleted = sqlx::query(&format!("DELETE FROM {table} WHERE user_id=$1"))
                .bind(user)
                .execute(&mut **tx)
                .await?;
            counts.insert(
                format!("invite_privacy.{table}"),
                deleted.rows_affected() as i64,
            );
        }
        for (table,predicate) in [
            ("bot.twitch_personal_invites","streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch') OR inviter_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch')"),
            ("bot.twitch_streamer_invite_code_history","twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch')"),
        ] {
            let deleted=sqlx::query(&format!("DELETE FROM {table} WHERE {predicate}")).bind(user).execute(&mut **tx).await?;
            counts.insert(format!("invite_privacy.{table}"),deleted.rows_affected() as i64);
        }
        sqlx::query("DELETE FROM community_points.ledger WHERE source='streamer_qualified_join'
            AND streamer_twitch_user_id IN (SELECT platform_user_id FROM core.discord_platform_connections
                                             WHERE discord_id=$1 AND platform='twitch')")
            .bind(user).execute(&mut **tx).await?;
        sqlx::query("SELECT set_config('community.invite_erasure_user_id','',TRUE)")
            .execute(&mut **tx)
            .await?;
    }
    if relations.contains("patchnotes.guild_dispatch") {
        let count: i64 = sqlx::query_scalar("SELECT patchnotes.anonymize_dispatch_approvals($1)")
            .bind(user)
            .fetch_one(&mut **tx)
            .await?;
        counts.insert(
            "patchnotes.guild_dispatch.approved_by_user_id".into(),
            count,
        );
    }
    if relations.contains("patchnotes.guild_settings") {
        let result=sqlx::query("UPDATE patchnotes.guild_settings SET updated_by_user_id=NULL WHERE updated_by_user_id=$1")
            .bind(user).execute(&mut **tx).await?;
        counts.insert(
            "patchnotes.guild_settings.updated_by_user_id".into(),
            result.rows_affected() as i64,
        );
    }
    Ok(counts)
}

pub(crate) async fn export(
    pool: &PgPool,
    user: i64,
    relations: &HashSet<&str>,
) -> Result<BTreeMap<String, Value>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    dl_central_db::platform_connections::lock_twitch_identity(&mut tx).await?;
    dl_central_db::lock_user_privacy(&mut tx, user).await?;
    let mut out = BTreeMap::new();
    let ids: Vec<String> = if relations.contains("core.discord_platform_connections") {
        sqlx::query_scalar("SELECT platform_user_id FROM core.discord_platform_connections WHERE discord_id=$1 AND platform='twitch'")
            .bind(user).fetch_all(&mut *tx).await?
    } else {
        Vec::new()
    };
    for (table, column) in [
        ("activity.twitch_invite_members", "user_id"),
        ("activity.twitch_invite_messages", "user_id"),
        ("activity.twitch_invite_evidence_queue", "user_id"),
        ("patchnotes.guild_dispatch", "approved_by_user_id"),
        ("patchnotes.guild_settings", "updated_by_user_id"),
    ] {
        if relations.contains(table) {
            let rows: Vec<Value> = sqlx::query_scalar(&format!(
                "SELECT to_jsonb(t) FROM {table} t WHERE {column}=$1"
            ))
            .bind(user)
            .fetch_all(&mut *tx)
            .await?;
            out.insert(format!("invite_privacy.{table}"), Value::Array(rows));
        }
    }
    if relations.contains(MEMBER_PRIVACY) {
        let rows:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(p) FROM activity.twitch_invite_member_privacy p
            WHERE subject_hash=sha256(convert_to('twitch-invites:discord-member-privacy:v1:' || $1::TEXT,'UTF8'))")
            .bind(user).fetch_all(&mut *tx).await?;
        out.insert("invite_privacy.member_privacy".into(), Value::Array(rows));
    }
    if relations.contains("bot.twitch_invite_joins") {
        let rows:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(j) - ARRAY['user_id','streamer_twitch_user_id','inviter_twitch_user_id','joined_at','qualified_at','reason','join_id','created_at','updated_at'] ||
            jsonb_build_object('user_id',CASE WHEN user_id=$1 THEN user_id END,
                'join_id',CASE WHEN user_id=$1 THEN join_id END,
                'updated_at',CASE WHEN user_id=$1 THEN updated_at END,
                'streamer_twitch_user_id',CASE WHEN streamer_twitch_user_id=ANY($2) THEN streamer_twitch_user_id END,
                'inviter_twitch_user_id',CASE WHEN inviter_twitch_user_id=ANY($2) THEN inviter_twitch_user_id END,
                'joined_at',CASE WHEN user_id=$1 THEN joined_at END,
                'qualified_at',CASE WHEN user_id=$1 THEN qualified_at END)
            FROM bot.twitch_invite_joins j WHERE user_id=$1 OR streamer_twitch_user_id=ANY($2) OR inviter_twitch_user_id=ANY($2)")
            .bind(user).bind(&ids).fetch_all(&mut *tx).await?;
        out.insert(
            "invite_privacy.bot.twitch_invite_joins".into(),
            Value::Array(rows),
        );
    }
    if relations.contains("bot.twitch_personal_invites") {
        let rows:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(p) - ARRAY['streamer_twitch_user_id','inviter_twitch_user_id'] ||
            jsonb_build_object('streamer_twitch_user_id',CASE WHEN streamer_twitch_user_id=ANY($1) THEN streamer_twitch_user_id END,
                'inviter_twitch_user_id',CASE WHEN inviter_twitch_user_id=ANY($1) THEN inviter_twitch_user_id END)
            FROM bot.twitch_personal_invites p WHERE streamer_twitch_user_id=ANY($1) OR inviter_twitch_user_id=ANY($1)")
            .bind(&ids).fetch_all(&mut *tx).await?;
        out.insert(
            "invite_privacy.bot.twitch_personal_invites".into(),
            Value::Array(rows),
        );
    }
    if relations.contains("bot.twitch_streamer_invite_code_history") {
        let rows:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(h) FROM bot.twitch_streamer_invite_code_history h WHERE twitch_user_id=ANY($1)")
            .bind(&ids).fetch_all(&mut *tx).await?;
        out.insert(
            "invite_privacy.bot.twitch_streamer_invite_code_history".into(),
            Value::Array(rows),
        );
    }
    tx.commit().await?;
    Ok(out)
}

#[cfg(all(test, feature = "testing"))]
#[path = "invite_privacy_tests.rs"]
mod tests;
