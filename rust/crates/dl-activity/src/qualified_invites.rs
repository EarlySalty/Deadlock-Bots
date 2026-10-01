use std::{collections::HashMap, time::Instant};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use crate::join_source::classify;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingInvite {
    pub join_id: i64,
    pub guild_id: i64,
    pub user_id: i64,
    pub joined_at: DateTime<Utc>,
    pub eligible: bool,
    pub left_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct MemberProof {
    pub joined_at: Option<DateTime<Utc>>,
    pub is_bot: bool,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct QualifiedInvite {
    pub join_id: String,
    pub streamer_login: String,
    pub inviter_twitch_user_id: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub status: String,
    pub qualified_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cursor {
    pub updated_at: DateTime<Utc>,
    pub join_id: i64,
}

#[derive(Debug, Serialize)]
pub struct InvitePage {
    pub invites: Vec<QualifiedInvite>,
    pub until: DateTime<Utc>,
    pub next_cursor: Option<Cursor>,
    pub next_since: Option<DateTime<Utc>>,
}

pub async fn lock_changes(conn: &mut PgConnection, guild_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('twitch-invites:' || $1::text, 0))")
        .bind(guild_id)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn remember_prior_member(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO activity.twitch_invite_members (guild_id, user_id, prior_member)
         VALUES ($1, $2, TRUE) ON CONFLICT (guild_id, user_id) DO NOTHING",
    )
    .bind(guild_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn remember_private_departure(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    user_id: i64,
    occurred_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO activity.twitch_invite_members
             (guild_id, user_id, prior_member, left_at)
         VALUES ($1, $2, TRUE, $3)
         ON CONFLICT (guild_id, user_id) DO UPDATE SET
             left_at = LEAST(activity.twitch_invite_members.left_at, EXCLUDED.left_at),
             current_joined_at = NULL",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(occurred_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn remember_member_event(
    tx: &mut Transaction<'_, Postgres>,
    join_id: i64,
    guild_id: i64,
    user_id: i64,
    event_type: &str,
    occurred_at: Option<DateTime<Utc>>,
    metadata: Option<&str>,
) -> Result<(), sqlx::Error> {
    if event_type == "join" {
        let metadata: Value = metadata
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or(Value::Null);
        let discord_joined_at = metadata
            .get("discord_joined_at")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let joined_at = discord_joined_at.or(occurred_at);
        let prior = discord_joined_at.is_none()
            || metadata.get("backfilled").and_then(Value::as_bool) == Some(true);
        sqlx::query(
            "INSERT INTO activity.twitch_invite_members
                 (guild_id, user_id, first_join_id, first_joined_at, current_joined_at, prior_member)
             SELECT $1, $2, $3, $4, $4, $5 OR $4 IS NULL OR $4 < started_at
             FROM activity.twitch_invite_tracking
             ON CONFLICT (guild_id, user_id) DO UPDATE
                 SET current_joined_at = EXCLUDED.current_joined_at",
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(join_id)
        .bind(joined_at)
        .bind(prior)
        .execute(&mut **tx)
        .await?;
    } else if matches!(event_type, "leave" | "ban") {
        sqlx::query(
            "INSERT INTO activity.twitch_invite_members
                 (guild_id, user_id, prior_member, left_at)
             VALUES ($1, $2, TRUE, $3)
             ON CONFLICT (guild_id, user_id) DO UPDATE SET
                 left_at = LEAST(activity.twitch_invite_members.left_at, EXCLUDED.left_at),
                 current_joined_at = NULL",
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(occurred_at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn record_message(
    pool: &PgPool,
    message_id: u64,
    guild_id: u64,
    user_id: u64,
    occurred_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let convert = |id| {
        i64::try_from(id)
            .map_err(|_| sqlx::Error::Protocol("Discord-ID außerhalb von BIGINT".into()))
    };
    sqlx::query(
        "INSERT INTO activity.twitch_invite_messages (message_id, guild_id, user_id, occurred_at)
         SELECT $1, $2, $3, $4
         WHERE $4 >= (SELECT started_at FROM activity.twitch_invite_tracking)
         AND NOT EXISTS (
             SELECT 1 FROM activity.twitch_invite_members m
             WHERE m.guild_id = $2 AND m.user_id = $3 AND (
                 m.prior_member OR (m.left_at IS NOT NULL AND $4 >= m.left_at)
                 OR $4 < m.first_joined_at OR $4 > m.first_joined_at + INTERVAL '720 hours'
             )
         ) AND NOT EXISTS (
             SELECT 1 FROM core.user_privacy WHERE user_id = $3 AND opted_out = TRUE
         )
         ON CONFLICT (message_id) DO NOTHING",
    )
    .bind(convert(message_id)?)
    .bind(convert(guild_id)?)
    .bind(convert(user_id)?)
    .bind(occurred_at)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct JoinSource {
    id: i64,
    user_id: i64,
    occurred_at: Option<DateTime<Utc>>,
    metadata: Option<Value>,
    first_joined_at: Option<DateTime<Utc>>,
    prior_member: bool,
}

pub async fn reconcile_attribution(pool: &PgPool, guild_id: i64) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    lock_changes(&mut tx, guild_id).await?;
    let website_values: Vec<(String, String)> =
        sqlx::query_as("SELECT k, v FROM bot.kv_store WHERE ns = 'website_invites'")
            .fetch_all(&mut *tx)
            .await?;
    let mut websites = HashMap::new();
    for (slug, raw) in website_values {
        if let Some(code) = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|value| value.get("code").and_then(Value::as_str).map(str::to_owned))
        {
            websites.insert(code.to_ascii_lowercase(), slug);
        }
    }
    let mut after_id = -1_i64;
    let mut inserted = 0;
    loop {
        let joins: Vec<JoinSource> = sqlx::query_as(
            "SELECT e.id, e.user_id, e.occurred_at, e.metadata,
                    m.first_joined_at, m.prior_member
             FROM activity.member_events e
             JOIN activity.twitch_invite_members m USING (guild_id, user_id)
             WHERE e.guild_id = $1 AND e.event_type = 'join' AND e.id > $2
               AND e.occurred_at >= (SELECT started_at FROM activity.twitch_invite_tracking)
               AND e.metadata->>'invite_code' IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM bot.twitch_invite_joins j WHERE j.join_id = e.id)
             ORDER BY e.id LIMIT 500",
        )
        .bind(guild_id)
        .bind(after_id)
        .fetch_all(&mut *tx)
        .await?;
        if joins.is_empty() {
            break;
        }
        for source in joins {
            after_id = source.id;
            let mut metadata = source.metadata.unwrap_or_else(|| json!({}));
            let Some(code) = metadata
                .get("invite_code")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            let actual_join = metadata
                .get("discord_joined_at")
                .and_then(Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc));
            let Some(joined_at) = actual_join.or(source.occurred_at) else {
                continue;
            };
            type OwnerCandidateRow = (Option<String>, Option<String>, Option<String>, bool);
            let owners: Vec<OwnerCandidateRow> = sqlx::query_as(
                "WITH channel_owners AS (
                         SELECT twitch_user_id,
                                MAX(source_login_snapshot) AS streamer_login,
                                BOOL_AND(attribution_safe) AS attribution_safe
                         FROM bot.twitch_streamer_invite_code_history
                         WHERE guild_id = $1 AND invite_code = $2
                           AND (valid_from IS NULL OR valid_from <= $3)
                           AND (valid_until IS NULL OR valid_until > $3)
                         GROUP BY twitch_user_id
                     )
                     SELECT COALESCE(current.streamer_login, owners.streamer_login),
                            owners.twitch_user_id, NULL::text,
                            owners.attribution_safe AND owners.twitch_user_id IS NOT NULL
                     FROM channel_owners AS owners
                     LEFT JOIN bot.twitch_streamer_invites AS current
                       ON current.twitch_user_id = owners.twitch_user_id
                      AND current.channel_id IS NOT NULL
                     UNION ALL
                     SELECT COALESCE(current.streamer_login, personal.streamer_login),
                            personal.streamer_twitch_user_id,
                            personal.inviter_twitch_user_id, TRUE
                     FROM bot.twitch_personal_invites AS personal
                     LEFT JOIN bot.twitch_streamer_invites AS current
                       ON current.twitch_user_id = personal.streamer_twitch_user_id
                      AND current.channel_id IS NOT NULL
                     WHERE personal.guild_id = $1 AND personal.invite_code = $2
                       AND personal.created_at <= $3
                       AND (personal.revoked_at IS NULL OR personal.revoked_at >= $3)",
            )
            .bind(guild_id)
            .bind(&code)
            .bind(joined_at)
            .fetch_all(&mut *tx)
            .await?;
            if owners.len() != 1 {
                continue;
            }
            let (login, streamer_id, inviter_id, attribution_safe) = &owners[0];
            if !attribution_safe {
                continue;
            }
            let (Some(login), Some(streamer_id)) = (login.as_ref(), streamer_id.as_ref()) else {
                continue;
            };
            metadata["twitch_streamer_login"] = json!(login);
            let lookup = HashMap::from([(code.to_ascii_lowercase(), login.clone())]);
            if classify(&metadata, &lookup, &websites).bucket != "twitch" {
                continue;
            }
            let eligible = !source.prior_member
                && actual_join.is_some()
                && source.first_joined_at == actual_join;
            inserted += sqlx::query(
                "INSERT INTO bot.twitch_invite_joins
                     (join_id, guild_id, user_id, streamer_login, streamer_twitch_user_id,
                      inviter_twitch_user_id, invite_code, joined_at, eligible)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT DO NOTHING",
            )
            .bind(source.id)
            .bind(guild_id)
            .bind(source.user_id)
            .bind(login)
            .bind(streamer_id)
            .bind(inviter_id)
            .bind(code)
            .bind(joined_at)
            .bind(eligible)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
    }
    tx.commit().await?;
    Ok(inserted)
}

pub async fn pending(pool: &PgPool, guild_id: i64) -> Result<Vec<PendingInvite>, sqlx::Error> {
    sqlx::query_as(
        "SELECT j.join_id, j.guild_id, j.user_id, j.joined_at, j.eligible, m.left_at
         FROM bot.twitch_invite_joins j
         JOIN activity.twitch_invite_members m USING (guild_id, user_id)
         WHERE j.guild_id = $1 AND j.status = 'pending'
         UNION ALL
         SELECT j.join_id, j.guild_id, j.user_id, j.joined_at, j.eligible, m.left_at
         FROM activity.twitch_invite_evidence_queue q
         JOIN bot.twitch_invite_joins j USING (guild_id, user_id)
         JOIN activity.twitch_invite_members m USING (guild_id, user_id)
         WHERE q.guild_id = $1 AND j.eligible AND j.status = 'expired' AND j.reason = 'deadline'
         ORDER BY joined_at, join_id",
    )
    .bind(guild_id)
    .fetch_all(pool)
    .await
}

async fn activity_at(
    conn: &mut PgConnection,
    invite: &PendingInvite,
    excluded_channels: &[i64],
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let deadline = (invite.joined_at + Duration::days(30)).min(now);
    sqlx::query_scalar(
        "WITH messages AS (
             SELECT occurred_at,
                    ROW_NUMBER() OVER (ORDER BY occurred_at, message_id) AS n,
                    (occurred_at AT TIME ZONE 'Europe/Berlin')::date AS day,
                    MIN((occurred_at AT TIME ZONE 'Europe/Berlin')::date) OVER () AS first_day
             FROM activity.twitch_invite_messages
             WHERE guild_id = $1 AND user_id = $2 AND occurred_at >= $3 AND occurred_at <= $4
         ), evidence AS (
             SELECT MIN(occurred_at) AS ready_at FROM messages WHERE n >= 5 AND day > first_day
             UNION ALL
             SELECT MIN(started_at + INTERVAL '15 minutes')
             FROM activity.voice_session_log
             WHERE guild_id = $1 AND user_id = $2 AND started_at >= $3
                 AND started_at + INTERVAL '15 minutes' <= $4
                 AND ended_at >= started_at + INTERVAL '15 minutes' AND ended_at <= $5
                 AND duration_seconds >= 900 AND channel_id IS NOT NULL
                 AND NOT (channel_id = ANY($6))
             UNION ALL
             SELECT voice_qualified_at FROM activity.twitch_invite_members
             WHERE guild_id = $1 AND user_id = $2 AND voice_qualified_at >= $3 AND voice_qualified_at <= $4
         ) SELECT MIN(ready_at) FROM evidence",
    )
    .bind(invite.guild_id)
    .bind(invite.user_id)
    .bind(invite.joined_at)
    .bind(deadline)
    .bind(now)
    .bind(excluded_channels)
    .fetch_one(conn)
    .await
}

pub fn qualifying_at(
    invite: &PendingInvite,
    activity_at: Option<DateTime<Utc>>,
    member: Option<&MemberProof>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if member.is_some_and(|proof| proof.is_bot) {
        return None;
    }
    let ready_at = timely_activity_at(invite, activity_at, now)?;
    let current_proof = member.is_some_and(|proof| {
        proof.joined_at == Some(invite.joined_at) && proof.checked_at >= ready_at
    });
    let historical_proof = invite.left_at.is_some_and(|left_at| left_at > ready_at);
    (current_proof || historical_proof).then_some(ready_at)
}

fn timely_activity_at(
    invite: &PendingInvite,
    activity_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if !invite.eligible {
        return None;
    }
    let ready_at = activity_at?.max(invite.joined_at + Duration::days(14));
    if ready_at > invite.joined_at + Duration::days(30) || ready_at > now {
        return None;
    }
    if invite.left_at.is_some_and(|left_at| ready_at >= left_at) {
        return None;
    }
    Some(ready_at)
}

pub async fn evaluate_one(
    pool: &PgPool,
    invite: &PendingInvite,
    member: Option<&MemberProof>,
    excluded_channels: &[i64],
    now: DateTime<Utc>,
) -> Result<Option<&'static str>, sqlx::Error> {
    let started = Instant::now();
    let mut tx = pool.begin().await?;
    lock_changes(&mut tx, invite.guild_id).await?;
    let current: Option<PendingInvite> = sqlx::query_as(
        "SELECT j.join_id, j.guild_id, j.user_id, j.joined_at, j.eligible, m.left_at
         FROM bot.twitch_invite_joins j
         JOIN activity.twitch_invite_members m USING (guild_id, user_id)
         WHERE j.join_id = $1 AND (j.status = 'pending' OR
             (j.status = 'expired' AND j.reason = 'deadline' AND EXISTS (
                 SELECT 1 FROM activity.twitch_invite_evidence_queue q
                 WHERE q.guild_id = j.guild_id AND q.user_id = j.user_id))) FOR UPDATE OF j",
    )
    .bind(invite.join_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(current) = current else {
        tx.commit().await?;
        return Ok(None);
    };
    let status: String =
        sqlx::query_scalar("SELECT status FROM bot.twitch_invite_joins WHERE join_id=$1")
            .bind(current.join_id)
            .fetch_one(&mut *tx)
            .await?;
    // Evidence writers take the join SHARE lock before marking this row.
    // A later writer therefore marks a new generation after this transaction.
    let marked: Option<bool> = sqlx::query_scalar(
        "SELECT TRUE FROM activity.twitch_invite_evidence_queue WHERE guild_id=$1 AND user_id=$2 AND $3 FOR UPDATE",
    ).bind(current.guild_id).bind(current.user_id).bind(current.eligible).fetch_optional(&mut *tx).await?;
    let opted_out: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM core.user_privacy WHERE user_id = $1 AND opted_out = TRUE)
             OR EXISTS (SELECT 1 FROM activity.twitch_invite_members WHERE guild_id=$2 AND user_id=$1 AND prior_member)",
    )
    .bind(current.user_id)
    .bind(current.guild_id)
    .fetch_one(&mut *tx)
    .await?;
    // Preserve the caller's clock while accounting for actual lock waits. A
    // completed evidence transaction must not look future-dated merely because
    // this evaluation began before it could acquire the join lock.
    let elapsed = Duration::from_std(started.elapsed()).map_err(|_| {
        sqlx::Error::Protocol("Evaluierungsdauer außerhalb des Zeitbereichs".into())
    })?;
    let now = now.checked_add_signed(elapsed).ok_or_else(|| {
        sqlx::Error::Protocol("Evaluierungszeit außerhalb des Zeitbereichs".into())
    })?;
    let evidence = if current.eligible && !opted_out {
        activity_at(&mut tx, &current, excluded_channels, now).await?
    } else {
        None
    };
    let qualified_at = qualifying_at(&current, evidence, member, now);
    let awaiting_membership = !opted_out
        && !member.is_some_and(|proof| proof.is_bot)
        && qualified_at.is_none()
        && timely_activity_at(&current, evidence, now).is_some();
    // Evidence may predate attribution, so its trigger could not yet find this
    // join. Retain a targeted retry before closing the visible deadline state.
    if awaiting_membership && marked.is_none() {
        sqlx::query(
            "INSERT INTO activity.twitch_invite_evidence_queue (guild_id,user_id)
             VALUES ($1,$2) ON CONFLICT (guild_id,user_id) DO NOTHING",
        )
        .bind(current.guild_id)
        .bind(current.user_id)
        .execute(&mut *tx)
        .await?;
    }
    let result = if qualified_at.is_some() {
        Some(("qualified", "activity_and_membership"))
    } else if !current.eligible || opted_out || member.is_some_and(|proof| proof.is_bot) {
        Some(("expired", "ineligible"))
    } else if current
        .left_at
        .is_some_and(|left_at| left_at <= current.joined_at + Duration::days(14))
    {
        Some(("expired", "left_before_retention"))
    } else if now >= current.joined_at + Duration::days(30) {
        Some(("expired", "deadline"))
    } else {
        None
    };
    let result = result.filter(|(next, _)| *next != status);
    if let Some((status, reason)) = result {
        sqlx::query(
            "UPDATE bot.twitch_invite_joins SET status = $2, qualified_at = $3, reason = $4
             WHERE join_id = $1",
        )
        .bind(current.join_id)
        .bind(status)
        .bind(qualified_at)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
    }
    // A valid activity timestamp may be newer than the REST membership proof.
    // Keep its marker until a fresh proof (or a definitive exclusion) resolves it.
    if marked.is_some() && !awaiting_membership {
        sqlx::query(
            "DELETE FROM activity.twitch_invite_evidence_queue WHERE guild_id=$1 AND user_id=$2",
        )
        .bind(current.guild_id)
        .bind(current.user_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(result.map(|(status, _)| status))
}

pub async fn list_page(
    pool: &PgPool,
    guild_id: i64,
    since: DateTime<Utc>,
    until: Option<DateTime<Utc>>,
    after: Option<&Cursor>,
    limit: u16,
) -> Result<InvitePage, sqlx::Error> {
    let mut tx = pool.begin().await?;
    lock_changes(&mut tx, guild_id).await?;
    let captured_at: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let until = until.unwrap_or(captured_at).min(captured_at);
    let limit = i64::from(limit.clamp(1, 1000));
    let mut invites: Vec<QualifiedInvite> = sqlx::query_as(
        "SELECT join_id::text AS join_id,
                COALESCE(current.streamer_login, joins.streamer_login) AS streamer_login,
                joins.inviter_twitch_user_id, joins.joined_at, joins.status,
                joins.qualified_at, joins.updated_at
         FROM bot.twitch_invite_joins AS joins
         LEFT JOIN bot.twitch_streamer_invites AS current
           ON current.twitch_user_id = joins.streamer_twitch_user_id
          AND current.channel_id IS NOT NULL
         WHERE joins.guild_id = $1 AND joins.updated_at >= $2 AND joins.updated_at <= $3
             AND ($4::timestamptz IS NULL OR (joins.updated_at, joins.join_id) > ($4, $5))
         ORDER BY joins.updated_at, joins.join_id LIMIT $6",
    )
    .bind(guild_id)
    .bind(since)
    .bind(until)
    .bind(after.map(|cursor| cursor.updated_at))
    .bind(after.map(|cursor| cursor.join_id))
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let has_more = invites.len() > limit as usize;
    invites.truncate(limit as usize);
    let next_cursor = if has_more {
        invites.last().map(|row| Cursor {
            updated_at: row.updated_at,
            join_id: row.join_id.parse().expect("Postgres BIGINT string"),
        })
    } else {
        None
    };
    Ok(InvitePage {
        invites,
        until,
        next_cursor,
        next_since: (!has_more).then_some(until),
    })
}

#[cfg(all(test, feature = "testing"))]
#[path = "qualified_invites_tests.rs"]
mod postgres_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PendingInvite, MemberProof) {
        let joined_at = DateTime::parse_from_rfc3339("2026-01-01T12:00:00Z")
            .expect("fixture timestamp")
            .with_timezone(&Utc);
        (
            PendingInvite {
                join_id: 1,
                guild_id: 1,
                user_id: 1,
                joined_at,
                eligible: true,
                left_at: None,
            },
            MemberProof {
                joined_at: Some(joined_at),
                is_bot: false,
                checked_at: joined_at + Duration::days(31),
            },
        )
    }

    #[test]
    fn retention_and_activity_must_both_be_ready() {
        let (invite, member) = fixture();
        let activity = Some(invite.joined_at + Duration::minutes(15));
        assert_eq!(
            qualifying_at(
                &invite,
                activity,
                Some(&member),
                invite.joined_at + Duration::days(13)
            ),
            None
        );
        assert_eq!(
            qualifying_at(&invite, activity, Some(&member), member.checked_at),
            Some(invite.joined_at + Duration::days(14))
        );
        assert_eq!(
            qualifying_at(&invite, None, Some(&member), member.checked_at),
            None
        );
    }

    #[test]
    fn delayed_evaluation_preserves_pre_deadline_qualification() {
        let (invite, member) = fixture();
        let deadline = invite.joined_at + Duration::days(30);
        assert_eq!(
            qualifying_at(&invite, Some(deadline), Some(&member), member.checked_at),
            Some(deadline)
        );
        assert_eq!(
            qualifying_at(
                &invite,
                Some(deadline + Duration::seconds(1)),
                Some(&member),
                member.checked_at
            ),
            None
        );
    }

    #[test]
    fn bots_prior_members_and_rejoins_do_not_qualify() {
        let (mut invite, mut member) = fixture();
        let activity = Some(invite.joined_at + Duration::days(1));
        member.is_bot = true;
        assert_eq!(
            qualifying_at(&invite, activity, Some(&member), member.checked_at),
            None
        );
        member.is_bot = false;
        invite.eligible = false;
        assert_eq!(
            qualifying_at(&invite, activity, Some(&member), member.checked_at),
            None
        );
        invite.eligible = true;
        member.joined_at = Some(invite.joined_at + Duration::days(2));
        assert_eq!(
            qualifying_at(&invite, activity, Some(&member), member.checked_at),
            None
        );
        invite.left_at = Some(invite.joined_at + Duration::days(14));
        assert_eq!(
            qualifying_at(&invite, activity, Some(&member), member.checked_at),
            None
        );
    }

    #[test]
    fn a_later_leave_does_not_erase_proven_qualification() {
        let (mut invite, member) = fixture();
        invite.left_at = Some(invite.joined_at + Duration::days(20));
        let activity = Some(invite.joined_at + Duration::days(15));
        assert_eq!(
            qualifying_at(&invite, activity, None, member.checked_at),
            activity
        );
        assert_eq!(
            qualifying_at(
                &invite,
                Some(invite.joined_at + Duration::days(21)),
                None,
                member.checked_at
            ),
            None
        );
    }
}
