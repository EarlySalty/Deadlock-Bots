use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dl_broker::port::{PersonalInviteResult, QualifiedInviteInfo};
use dl_broker::{DiscordPort, PortError, TwitchInvitePort};
use sqlx::PgPool;

pub struct CentralTwitchInvites {
    pool: PgPool,
    discord: Arc<dyn DiscordPort>,
    personal_links_per_channel_max: i64,
}

impl CentralTwitchInvites {
    pub fn new(
        pool: PgPool,
        discord: Arc<dyn DiscordPort>,
        personal_links_per_channel_max: u32,
    ) -> Self {
        Self {
            pool,
            discord,
            personal_links_per_channel_max: i64::from(personal_links_per_channel_max),
        }
    }
}

#[async_trait::async_trait]
impl TwitchInvitePort for CentralTwitchInvites {
    async fn personal_invite(
        &self,
        streamer_login: &str,
        inviter_twitch_user_id: &str,
        guild_id: u64,
        channel_id: u64,
    ) -> Result<PersonalInviteResult, PortError> {
        let guild_id_i64 = i64::try_from(guild_id)
            .map_err(|_| PortError::Backend("guild id out of range".to_string()))?;
        let channel_id_i64 = i64::try_from(channel_id)
            .map_err(|_| PortError::Backend("channel id out of range".to_string()))?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| PortError::Backend(e.to_string()))?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(channel_id_i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| PortError::Backend(e.to_string()))?;

        if let Some((invite_url, code, stored_guild, stored_channel)) =
            sqlx::query_as::<_, (String, String, i64, i64)>(
                "SELECT invite_url, invite_code, guild_id, channel_id
                 FROM bot.twitch_personal_invites
                 WHERE streamer_login = $1 AND inviter_twitch_user_id = $2",
            )
            .bind(streamer_login)
            .bind(inviter_twitch_user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| PortError::Backend(e.to_string()))?
        {
            tx.commit()
                .await
                .map_err(|e| PortError::Backend(e.to_string()))?;
            return Ok(PersonalInviteResult {
                invite_url: Some(invite_url),
                code: Some(code),
                guild_id: u64::try_from(stored_guild).unwrap_or(guild_id),
                channel_id: u64::try_from(stored_channel).unwrap_or(channel_id),
                fallback: false,
            });
        }

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::bigint
             FROM bot.twitch_personal_invites
             WHERE guild_id = $1 AND channel_id = $2",
        )
        .bind(guild_id_i64)
        .bind(channel_id_i64)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| PortError::Backend(e.to_string()))?;
        if count >= self.personal_links_per_channel_max {
            tx.commit()
                .await
                .map_err(|e| PortError::Backend(e.to_string()))?;
            return Ok(PersonalInviteResult {
                invite_url: None,
                code: None,
                guild_id,
                channel_id,
                fallback: true,
            });
        }

        let reason = format!("twitch-viewer-invite:{streamer_login}:{inviter_twitch_user_id}");
        let invite = self.discord.create_invite(channel_id, &reason).await?;
        if invite.guild_id != guild_id {
            return Err(PortError::Backend("invite guild mismatch".to_string()));
        }

        sqlx::query(
            "INSERT INTO bot.twitch_personal_invites
             (streamer_login, inviter_twitch_user_id, guild_id, channel_id, invite_code, invite_url)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(streamer_login)
        .bind(inviter_twitch_user_id)
        .bind(guild_id_i64)
        .bind(channel_id_i64)
        .bind(&invite.code)
        .bind(&invite.invite_url)
        .execute(&mut *tx)
        .await
        .map_err(|e| PortError::Backend(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| PortError::Backend(e.to_string()))?;

        Ok(PersonalInviteResult {
            invite_url: Some(invite.invite_url),
            code: Some(invite.code),
            guild_id,
            channel_id,
            fallback: false,
        })
    }

    async fn qualified_invites_since(
        &self,
        since: &str,
    ) -> Result<Vec<QualifiedInviteInfo>, PortError> {
        let since = DateTime::parse_from_rfc3339(since)
            .map_err(|_| PortError::Backend("invalid since timestamp".to_string()))?
            .with_timezone(&Utc);
        let rows = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                DateTime<Utc>,
                String,
                Option<DateTime<Utc>>,
            ),
        >(
            "SELECT streamer_login, inviter_twitch_user_id, joined_at, status, qualified_at
             FROM activity.twitch_invite_qualifications
             WHERE joined_at >= $1
             ORDER BY joined_at, join_event_id",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| PortError::Backend(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(streamer_login, inviter_twitch_user_id, joined_at, status, qualified_at)| {
                    QualifiedInviteInfo {
                        streamer_login,
                        inviter_twitch_user_id,
                        joined_at: joined_at.to_rfc3339(),
                        status,
                        qualified_at: qualified_at.map(|value| value.to_rfc3339()),
                    }
                },
            )
            .collect())
    }
}

pub fn spawn_qualification_worker(
    pool: PgPool,
    staging_channel_ids: HashSet<i64>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if let Err(error) =
                evaluate_qualifications(&pool, &staging_channel_ids, Utc::now()).await
            {
                tracing::warn!(%error, "Twitch-Invite-Qualifikation fehlgeschlagen");
            }
        }
    })
}

pub async fn evaluate_qualifications(
    pool: &PgPool,
    staging_channel_ids: &HashSet<i64>,
    now: DateTime<Utc>,
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, i64, i64, DateTime<Utc>)>(
        "SELECT join_event_id, user_id, guild_id, joined_at
         FROM activity.twitch_invite_qualifications
         WHERE status = 'pending'
         ORDER BY joined_at, join_event_id",
    )
    .fetch_all(pool)
    .await?;
    let mut changed = 0usize;

    for (join_event_id, user_id, guild_id, joined_at) in rows {
        let fourteen_days = joined_at + chrono::Duration::days(14);
        let thirty_days = joined_at + chrono::Duration::days(30);

        let member_at_threshold: Option<String> = sqlx::query_scalar(
            "SELECT event_type
             FROM activity.member_events
             WHERE user_id = $1
               AND guild_id = $2
               AND event_type IN ('join', 'leave')
               AND occurred_at <= $3
             ORDER BY occurred_at DESC, id DESC
             LIMIT 1",
        )
        .bind(user_id)
        .bind(guild_id)
        .bind(fourteen_days)
        .fetch_optional(pool)
        .await?;

        let next_status = if now >= fourteen_days && member_at_threshold.as_deref() != Some("join")
        {
            Some("expired")
        } else if now >= fourteen_days {
            let activity_end = std::cmp::min(now, thirty_days);
            let voice_rows = sqlx::query_as::<_, (Option<i64>, i64)>(
                "SELECT channel_id, duration_seconds
                 FROM activity.voice_session_log
                 WHERE user_id = $1
                   AND guild_id = $2
                   AND started_at >= $3
                   AND ended_at <= $4
                   AND duration_seconds >= 900
                   AND NOT EXISTS (
                       SELECT 1
                       FROM voice.tempvoice_staging_channels staging
                       WHERE staging.guild_id = activity.voice_session_log.guild_id
                         AND staging.channel_id = activity.voice_session_log.channel_id
                   )",
            )
            .bind(user_id)
            .bind(guild_id)
            .bind(joined_at)
            .bind(activity_end)
            .fetch_all(pool)
            .await?;
            let voice_ok = voice_rows.into_iter().any(|(channel_id, duration)| {
                duration >= 900
                    && channel_id
                        .map(|id| !staging_channel_ids.contains(&id))
                        .unwrap_or(false)
            });

            let (message_count, message_days) = sqlx::query_as::<_, (i64, i64)>(
                "SELECT COUNT(*)::bigint,
                        COUNT(DISTINCT ((occurred_at AT TIME ZONE 'UTC')::date))::bigint
                 FROM activity.message_metadata_events
                 WHERE user_id = $1
                   AND guild_id = $2
                   AND occurred_at >= $3
                   AND occurred_at <= $4",
            )
            .bind(user_id)
            .bind(guild_id)
            .bind(joined_at)
            .bind(activity_end)
            .fetch_one(pool)
            .await?;
            if voice_ok || (message_count >= 5 && message_days >= 2) {
                Some("qualified")
            } else if now >= thirty_days {
                Some("expired")
            } else {
                None
            }
        } else {
            None
        };

        let Some(next_status) = next_status else {
            sqlx::query(
                "UPDATE activity.twitch_invite_qualifications
                 SET last_evaluated_at = $2
                 WHERE join_event_id = $1 AND status = 'pending'",
            )
            .bind(join_event_id)
            .bind(now)
            .execute(pool)
            .await?;
            continue;
        };

        let mut tx = pool.begin().await?;
        let updated = if next_status == "qualified" {
            sqlx::query(
                "UPDATE activity.twitch_invite_qualifications
                 SET status = 'qualified', qualified_at = $2, last_evaluated_at = $2
                 WHERE join_event_id = $1 AND status = 'pending'",
            )
            .bind(join_event_id)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        } else {
            sqlx::query(
                "UPDATE activity.twitch_invite_qualifications
                 SET status = 'expired', expired_at = $2, last_evaluated_at = $2
                 WHERE join_event_id = $1 AND status = 'pending'",
            )
            .bind(join_event_id)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        };
        if updated == 1 {
            sqlx::query(
                "INSERT INTO activity.twitch_invite_qualification_transitions
                 (join_event_id, from_status, to_status, occurred_at)
                 VALUES ($1, 'pending', $2, $3)",
            )
            .bind(join_event_id)
            .bind(next_status)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            changed += 1;
        }
        tx.commit().await?;
    }

    Ok(changed)
}
