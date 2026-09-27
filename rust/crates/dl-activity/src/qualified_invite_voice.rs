use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow)]
struct VoiceClock {
    user_id: i64,
    first_joined_at: DateTime<Utc>,
    voice_channel_id: Option<i64>,
    voice_started_at: Option<DateTime<Utc>>,
    voice_observed_at: Option<DateTime<Utc>>,
    voice_qualified_at: Option<DateTime<Utc>>,
}

fn observe(
    clock: &mut VoiceClock,
    channel: Option<i64>,
    observed_at: DateTime<Utc>,
    explicit_transition: bool,
    excluded: &[i64],
) {
    if clock
        .voice_observed_at
        .is_some_and(|last| last > observed_at)
    {
        return;
    }
    let fresh = clock
        .voice_observed_at
        .is_some_and(|last| observed_at - last <= Duration::seconds(240));
    let same_channel = clock.voice_channel_id.is_some() && channel == clock.voice_channel_id;
    if fresh
        && (same_channel || explicit_transition)
        && clock
            .voice_channel_id
            .is_some_and(|id| !excluded.contains(&id))
    {
        if let Some(started_at) = clock
            .voice_started_at
            .filter(|start| *start >= clock.first_joined_at)
        {
            let ready_at = started_at + Duration::minutes(15);
            if ready_at <= observed_at && ready_at <= clock.first_joined_at + Duration::days(30) {
                clock.voice_qualified_at = Some(
                    clock
                        .voice_qualified_at
                        .map_or(ready_at, |old| old.min(ready_at)),
                );
            }
        }
    }
    if !same_channel || !fresh || explicit_transition {
        clock.voice_started_at = channel.map(|_| observed_at);
    }
    clock.voice_channel_id = channel;
    clock.voice_observed_at = Some(observed_at);
}

pub async fn reset_live_clocks(pool: &PgPool, guild_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE activity.twitch_invite_members SET voice_channel_id = NULL,
         voice_started_at = NULL, voice_observed_at = NULL WHERE guild_id = $1",
    )
    .bind(guild_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_snapshot(
    pool: &PgPool,
    guild_id: i64,
    members: &HashMap<u64, u64>,
    excluded: &[i64],
    observed_at: DateTime<Utc>,
    event_user: Option<u64>,
    explicit_transition: bool,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let clocks: Vec<VoiceClock> = sqlx::query_as(
        "SELECT user_id, first_joined_at, voice_channel_id, voice_started_at,
                voice_observed_at, voice_qualified_at
         FROM activity.twitch_invite_members m
         WHERE guild_id = $1 AND NOT prior_member AND left_at IS NULL
             AND first_joined_at IS NOT NULL AND current_joined_at = first_joined_at
             AND voice_qualified_at IS NULL
             AND first_joined_at + INTERVAL '720 hours' >= $3 - INTERVAL '4 minutes'
             AND ($2::bigint IS NULL OR user_id = $2)
             AND NOT EXISTS (SELECT 1 FROM core.user_privacy p WHERE p.user_id = m.user_id AND p.opted_out = TRUE)
         ORDER BY user_id FOR UPDATE",
    ).bind(guild_id).bind(event_user.and_then(|id| i64::try_from(id).ok())).bind(observed_at)
        .fetch_all(&mut *tx).await?;
    for mut clock in clocks {
        let channel = u64::try_from(clock.user_id)
            .ok()
            .and_then(|id| members.get(&id))
            .and_then(|id| i64::try_from(*id).ok())
            .filter(|id| !excluded.contains(id));
        observe(
            &mut clock,
            channel,
            observed_at,
            explicit_transition,
            excluded,
        );
        sqlx::query(
            "UPDATE activity.twitch_invite_members SET voice_channel_id = $3,
             voice_started_at = $4, voice_observed_at = $5, voice_qualified_at = $6
             WHERE guild_id = $1 AND user_id = $2",
        )
        .bind(guild_id)
        .bind(clock.user_id)
        .bind(clock.voice_channel_id)
        .bind(clock.voice_started_at)
        .bind(clock.voice_observed_at)
        .bind(clock.voice_qualified_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> VoiceClock {
        VoiceClock {
            user_id: 1,
            first_joined_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .expect("fixture")
                .with_timezone(&Utc),
            voice_channel_id: None,
            voice_started_at: None,
            voice_observed_at: None,
            voice_qualified_at: None,
        }
    }

    #[test]
    fn ongoing_voice_qualifies_without_a_leave_or_other_people() {
        let mut clock = clock();
        let start = clock.first_joined_at + Duration::days(1);
        for minute in 0..=15 {
            observe(
                &mut clock,
                Some(2),
                start + Duration::minutes(minute),
                false,
                &[],
            );
        }
        assert_eq!(
            clock.voice_qualified_at,
            Some(start + Duration::minutes(15))
        );
    }

    #[test]
    fn moves_short_sessions_and_unknown_gaps_do_not_accumulate() {
        let mut clock = clock();
        let start = clock.first_joined_at;
        for minute in 0..15 {
            observe(
                &mut clock,
                Some(2),
                start + Duration::minutes(minute),
                false,
                &[],
            );
        }
        observe(
            &mut clock,
            Some(3),
            start + Duration::minutes(14) + Duration::seconds(59),
            true,
            &[],
        );
        assert_eq!(clock.voice_qualified_at, None);
        observe(
            &mut clock,
            Some(3),
            start + Duration::minutes(20),
            false,
            &[],
        );
        assert_eq!(clock.voice_qualified_at, None);
        assert_eq!(clock.voice_started_at, Some(start + Duration::minutes(20)));
    }

    #[test]
    fn excluded_channels_cannot_create_proof_and_deadline_is_inclusive() {
        for delay in [0, 1] {
            let mut clock = clock();
            let start = clock.first_joined_at + Duration::days(30) - Duration::minutes(15)
                + Duration::seconds(delay);
            for minute in 0..=16 {
                observe(
                    &mut clock,
                    Some(2),
                    start + Duration::minutes(minute),
                    false,
                    &[],
                );
            }
            assert_eq!(clock.voice_qualified_at.is_some(), delay == 0);
        }
        let mut clock = clock();
        let start = clock.first_joined_at;
        for minute in 0..=20 {
            observe(
                &mut clock,
                Some(99),
                start + Duration::minutes(minute),
                false,
                &[99],
            );
        }
        assert!(clock.voice_qualified_at.is_none());
    }
}
