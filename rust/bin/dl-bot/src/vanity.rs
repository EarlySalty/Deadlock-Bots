use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use std::collections::BTreeSet;
use std::time::Duration;

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const MEMBER_DIRECTORY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const VANITY_ATTRIBUTION_GRACE: Duration = Duration::from_secs(90);

type TaskResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Deserialize)]
struct VanityResponse {
    code: Option<String>,
    uses: Option<i32>,
}

#[derive(Debug)]
struct VanitySnapshot {
    code: Option<String>,
    uses: i32,
    captured_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct DirectoryMember {
    user: DirectoryUser,
    joined_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DirectoryUser {
    id: String,
    bot: Option<bool>,
}

pub fn spawn_vanity_snapshots(
    pool: PgPool,
    token: String,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut gateway = dispatcher.subscribe_gateway();
    let client = reqwest::Client::new();
    tokio::spawn(async move {
        let mut guilds = BTreeSet::new();
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                event = gateway.recv() => {
                    match event {
                        Ok(dl_discord::GatewayEvent::CacheReady { guild_ids }) => {
                            guilds.extend(guild_ids);
                        }
                        Ok(dl_discord::GatewayEvent::Ready { .. }) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                            tracing::warn!(missed, "Vanity-Snapshot Gateway-Events verpasst");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = interval.tick(), if !guilds.is_empty() => {
                    for guild_id in guilds.iter().copied().collect::<Vec<_>>() {
                        if let Err(err) = poll_one(&pool, &client, &token, guild_id).await {
                            tracing::debug!(%err, guild_id, "Vanity-URL-Snapshot fehlgeschlagen");
                        }
                    }
                }
            }
        }
    })
}

pub fn spawn_member_directory_sweep(
    pool: PgPool,
    token: String,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut gateway = dispatcher.subscribe_gateway();
    let client = reqwest::Client::new();
    tokio::spawn(async move {
        let mut guilds = BTreeSet::new();
        let start = tokio::time::Instant::now() + MEMBER_DIRECTORY_INTERVAL;
        let mut interval = tokio::time::interval_at(start, MEMBER_DIRECTORY_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                event = gateway.recv() => {
                    match event {
                        Ok(dl_discord::GatewayEvent::CacheReady { guild_ids }) => {
                            guilds.extend(guild_ids);
                            run_directory_sweeps(&pool, &client, &token, &guilds).await;
                        }
                        Ok(dl_discord::GatewayEvent::Ready { .. }) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                            tracing::warn!(missed, "Member-Directory Gateway-Events verpasst");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = interval.tick(), if !guilds.is_empty() => {
                    run_directory_sweeps(&pool, &client, &token, &guilds).await;
                }
            }
        }
    })
}

async fn poll_one(
    pool: &PgPool,
    client: &reqwest::Client,
    token: &str,
    guild_id: u64,
) -> TaskResult<()> {
    let response = client
        .get(format!("{DISCORD_API_BASE}/guilds/{guild_id}/vanity-url"))
        .header("Authorization", format!("Bot {token}"))
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    if !response.status().is_success() {
        return Err(format!("discord status {}", response.status()).into());
    }
    let body = response.json::<VanityResponse>().await?;
    let Some(uses) = body.uses else {
        return Ok(());
    };
    let code = body.code.filter(|code| !code.is_empty());
    let guild_id = i64::try_from(guild_id)?;
    let previous = latest_vanity_snapshot(pool, guild_id).await?;
    let changed = previous
        .as_ref()
        .map(|snapshot| snapshot.uses != uses || snapshot.code != code)
        .unwrap_or(true);
    let captured_at = Utc::now();

    if changed {
        sqlx::query(
            r#"
            INSERT INTO activity.vanity_uses_snapshots(guild_id, uses, code, captured_at)
            VALUES($1, $2, $3, $4)
            "#,
        )
        .bind(guild_id)
        .bind(uses)
        .bind(&code)
        .bind(captured_at)
        .execute(pool)
        .await?;

        if let Some(previous) = previous.filter(|snapshot| uses > snapshot.uses) {
            let delta = uses - previous.uses;
            apply_vanity_delta_to_recent_joins(
                pool,
                guild_id,
                vanity_attribution_window_start(previous.captured_at),
                captured_at,
                delta,
                code.as_deref(),
            )
            .await?;
        }
    }

    purge_old_vanity_snapshots(pool).await?;
    Ok(())
}

async fn latest_vanity_snapshot(
    pool: &PgPool,
    guild_id: i64,
) -> Result<Option<VanitySnapshot>, sqlx::Error> {
    let Some(row) = sqlx::query(
        r#"
        SELECT code, uses, captured_at
          FROM activity.vanity_uses_snapshots
         WHERE guild_id = $1
         ORDER BY captured_at DESC
         LIMIT 1
        "#,
    )
    .bind(guild_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(VanitySnapshot {
        code: row.get("code"),
        uses: row.get("uses"),
        captured_at: row.get("captured_at"),
    }))
}

fn vanity_attribution_window_start(previous_captured_at: DateTime<Utc>) -> DateTime<Utc> {
    previous_captured_at - VANITY_ATTRIBUTION_GRACE
}

async fn apply_vanity_delta_to_recent_joins(
    pool: &PgPool,
    guild_id: i64,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    delta: i32,
    code: Option<&str>,
) -> Result<(), sqlx::Error> {
    // Vanity-Attribution ist eine Naeherung: Discord liefert den uses-Anstieg
    // erst beim Poller, daher kann die Markierung bis zum Poll-Intervall warten.
    let rows = sqlx::query(
        r#"
        WITH candidates AS (
            SELECT id
              FROM activity.member_events
             WHERE guild_id = $1
               AND event_type = 'join'
               AND occurred_at >= $2
               AND occurred_at <= $3
               AND COALESCE(metadata->>'join_source_kind', 'unknown') IN ('unknown', 'server_discovery')
             ORDER BY occurred_at DESC NULLS LAST, id DESC
             LIMIT $4
        )
        UPDATE activity.member_events e
           SET metadata = COALESCE(e.metadata, '{}'::jsonb) ||
               jsonb_strip_nulls(jsonb_build_object(
                   'join_source_bucket', 'vanity',
                   'join_source_kind', 'vanity',
                   'join_source_label', 'Vanity-Link',
                   'join_source_confidence', 'medium',
                   'join_source_reason', 'vanity_uses_delta',
                   'vanity_uses_delta', $5,
                   'invite_code', $6::TEXT,
                   'invite_url', CASE
                       WHEN $6::TEXT IS NULL OR $6::TEXT = '' THEN NULL
                       ELSE 'https://discord.gg/' || $6::TEXT
                   END
               ))
          FROM candidates c
         WHERE e.id = c.id
         RETURNING e.id
        "#,
    )
    .bind(guild_id)
    .bind(window_start)
    .bind(window_end)
    .bind(i64::from(delta))
    .bind(delta)
    .bind(code)
    .fetch_all(pool)
    .await?;
    tracing::debug!(
        guild_id,
        delta,
        attributed = rows.len(),
        "Vanity-Uses-Delta Join-Events zugeordnet"
    );
    Ok(())
}

async fn purge_old_vanity_snapshots(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM activity.vanity_uses_snapshots WHERE captured_at < now() - interval '90 days'",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn run_directory_sweeps(
    pool: &PgPool,
    client: &reqwest::Client,
    token: &str,
    guilds: &BTreeSet<u64>,
) {
    for guild_id in guilds.iter().copied().collect::<Vec<_>>() {
        if let Err(err) = sync_member_directory(pool, client, token, guild_id).await {
            tracing::warn!(%err, guild_id, "Guild-Member-Directory-Sweep fehlgeschlagen");
        }
    }
}

async fn sync_member_directory(
    pool: &PgPool,
    client: &reqwest::Client,
    token: &str,
    guild_id: u64,
) -> TaskResult<()> {
    let guild_id_i64 = i64::try_from(guild_id)?;
    let sweep_started_at = Utc::now();
    let mut after = None;
    let mut seen = 0usize;

    loop {
        let page = fetch_member_page(client, token, guild_id, after).await?;
        if page.is_empty() {
            break;
        }
        for member in &page {
            upsert_directory_member(pool, guild_id_i64, member, sweep_started_at).await?;
        }
        seen += page.len();
        after = page
            .last()
            .and_then(|member| member.user.id.parse::<u64>().ok());
        if page.len() < 1000 || after.is_none() {
            break;
        }
    }

    let absent = sqlx::query(
        r#"
        UPDATE activity.guild_member_directory
           SET present = FALSE,
               synced_at = $2
         WHERE guild_id = $1
           AND synced_at < $2
        "#,
    )
    .bind(guild_id_i64)
    .bind(sweep_started_at)
    .execute(pool)
    .await?
    .rows_affected();

    tracing::info!(
        guild_id,
        seen,
        absent,
        "Guild-Member-Directory synchronisiert"
    );
    Ok(())
}

async fn fetch_member_page(
    client: &reqwest::Client,
    token: &str,
    guild_id: u64,
    after: Option<u64>,
) -> TaskResult<Vec<DirectoryMember>> {
    let mut url = format!("{DISCORD_API_BASE}/guilds/{guild_id}/members?limit=1000");
    if let Some(after) = after {
        url.push_str("&after=");
        url.push_str(&after.to_string());
    }
    let response = client
        .get(url)
        .header("Authorization", format!("Bot {token}"))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(format!("discord status {}", response.status()).into());
    }
    Ok(response.json::<Vec<DirectoryMember>>().await?)
}

async fn upsert_directory_member(
    pool: &PgPool,
    guild_id: i64,
    member: &DirectoryMember,
    synced_at: DateTime<Utc>,
) -> TaskResult<()> {
    let user_id_u64 = member.user.id.parse::<u64>()?;
    let user_id = i64::try_from(user_id_u64)?;
    let joined_at = member
        .joined_at
        .as_deref()
        .and_then(parse_discord_timestamp);
    let account_created_at = discord_snowflake_created_at(user_id_u64);
    sqlx::query(
        r#"
        INSERT INTO activity.guild_member_directory(
            guild_id, user_id, joined_at, account_created_at, is_bot, present, synced_at
        )
        VALUES($1, $2, $3, $4, $5, TRUE, $6)
        ON CONFLICT (guild_id, user_id) DO UPDATE SET
            joined_at = EXCLUDED.joined_at,
            account_created_at = EXCLUDED.account_created_at,
            is_bot = EXCLUDED.is_bot,
            present = TRUE,
            synced_at = EXCLUDED.synced_at
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(joined_at)
    .bind(account_created_at)
    .bind(member.user.bot.unwrap_or(false))
    .bind(synced_at)
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_discord_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn discord_snowflake_created_at(id: u64) -> Option<DateTime<Utc>> {
    let millis = i64::try_from(id >> 22)
        .ok()?
        .checked_add(DISCORD_EPOCH_MS)?;
    DateTime::from_timestamp_millis(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing_member_ids(mut present: Vec<i64>, mut seen: Vec<i64>) -> Vec<i64> {
        present.sort_unstable();
        seen.sort_unstable();
        present
            .into_iter()
            .filter(|id| seen.binary_search(id).is_err())
            .collect()
    }

    #[test]
    fn snowflake_zu_account_created_at() {
        let snowflake = 1_000_u64 << 22;
        let created = discord_snowflake_created_at(snowflake).expect("timestamp");

        assert_eq!(created.to_rfc3339(), "2015-01-01T00:00:01+00:00");
    }

    #[test]
    fn vanity_fenster_enthaelt_kulanz_vor_letztem_snapshot() {
        let previous = DateTime::parse_from_rfc3339("2026-07-06T12:00:00Z")
            .expect("ts")
            .with_timezone(&Utc);

        assert_eq!(
            vanity_attribution_window_start(previous).to_rfc3339(),
            "2026-07-06T11:58:30+00:00"
        );
    }

    #[test]
    fn sweep_diff_markiert_nur_nicht_gesehene_present_member() {
        assert_eq!(
            missing_member_ids(vec![10, 20, 30], vec![20, 40]),
            vec![10, 30]
        );
    }
}
