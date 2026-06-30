//! Aktivitäts-Analyzer — Port des Kerns von `cogs/user_activity_analyzer.py`.
//!
//! Zwei Loops:
//! - alle 6 h: Aktivitätsmuster der letzten 14 Tage aus `voice_session_log`
//!   → `user_activity_patterns` (typische Stunden/Wochentage, Score) —
//!   idempotent überschreibend.
//! - alle 10 min: wer sitzt gerade mit wem im Voice → `user_co_players`
//!   (+1 Session, +10 Minuten, bidirektional, Namen werden mitgepflegt).
//!
//! Bewusst NICHT portiert: der 6-h-Co-Spieler-Backfill des Originals — er
//! addierte alle 6 Stunden die kompletten 2-Wochen-Aggregate ERNEUT auf
//! `user_co_players` (Doppelzählung zusätzlich zum 10-min-Tracker, stiller
//! Akkumulations-Bug). Der 10-min-Pfad ist der korrekte Schreiber.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Utc};
use sqlx::PgPool;

use crate::db::{
    discord_id_to_i64, i64_to_i32, i64_to_u64, lock_member_events, next_member_event_id_in_tx,
    validate_json_text, ActivityDbResult,
};

pub const ANALYZE_INTERVAL: Duration = Duration::from_secs(6 * 3600);
pub const CO_PLAYER_INTERVAL: Duration = Duration::from_secs(600);
pub const CO_PLAYER_MINUTES_PER_TICK: i64 = 10;
pub const WINDOW_DAYS: i64 = 14;
pub const MEMBER_BACKFILL_RETRY_INTERVAL: Duration = Duration::from_secs(5);
pub const BACKFILL_JOIN_SOURCE_LABEL_PLACEHOLDER: &str = "Vor Tracking (rückwirkend)";

// ── Pure Muster-Berechnung (Referenzwerte aus CPython im Test) ─────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityPattern {
    /// Top-3-Stunden (0–23), nach Häufigkeit, Insertion-Order als Tie-Break.
    pub typical_hours: Vec<u32>,
    /// Top-3-Wochentage (0=Mo … 6=So).
    pub typical_days: Vec<u32>,
    pub sessions_count: i64,
    pub total_minutes: i64,
    pub last_active: Option<NaiveDateTime>,
}

/// Wie `_analyze_single_user`: Python-dict-Insertion-Order + stabile
/// Sortierung nach Häufigkeit bestimmen die Tie-Breaks.
pub fn analyze_sessions(sessions: &[(Option<NaiveDateTime>, i64)]) -> ActivityPattern {
    let mut hour_order: Vec<(u32, i64)> = Vec::new();
    let mut day_order: Vec<(u32, i64)> = Vec::new();
    let mut total_minutes = 0i64;
    let mut last_active: Option<NaiveDateTime> = None;

    for (started_at, duration_seconds) in sessions {
        total_minutes += duration_seconds / 60;
        let Some(started_at) = started_at else {
            continue;
        };
        let hour = started_at.hour();
        match hour_order.iter_mut().find(|(h, _)| *h == hour) {
            Some((_, count)) => *count += 1,
            None => hour_order.push((hour, 1)),
        }
        let day = started_at.weekday().num_days_from_monday();
        match day_order.iter_mut().find(|(d, _)| *d == day) {
            Some((_, count)) => *count += 1,
            None => day_order.push((day, 1)),
        }
        if last_active.map(|last| *started_at > last).unwrap_or(true) {
            last_active = Some(*started_at);
        }
    }
    // Stabile Sortierung absteigend nach Häufigkeit (wie Pythons sorted)
    hour_order.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    day_order.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

    ActivityPattern {
        typical_hours: hour_order.iter().take(3).map(|(h, _)| *h).collect(),
        typical_days: day_order.iter().take(3).map(|(d, _)| *d).collect(),
        sessions_count: sessions.len() as i64,
        total_minutes,
        last_active,
    }
}

// ── Analyzer ───────────────────────────────────────────────────────────────

/// Voice-Sicht für den Co-Spieler-Tracker: Gruppen von (user_id, display_name)
/// pro Kanal mit ≥ 2 Nicht-Bots.
#[async_trait::async_trait]
pub trait VoiceGroups: Send + Sync {
    async fn channel_groups(&self) -> Vec<Vec<(u64, String)>>;
}

pub struct ActivityAnalyzer {
    pub pool: PgPool,
    pub voice: Arc<dyn VoiceGroups>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillMember {
    pub guild_id: u64,
    pub user_id: u64,
    pub display_name: String,
    pub joined_at: Option<String>,
    pub account_created_at: Option<String>,
    pub is_bot: bool,
}

#[async_trait::async_trait]
pub trait MemberBackfillPort: Send + Sync {
    async fn cache_ready(&self) -> bool {
        true
    }
    async fn current_members(&self) -> Vec<BackfillMember>;
}

impl ActivityAnalyzer {
    pub fn new(pool: PgPool, voice: Arc<dyn VoiceGroups>) -> Arc<Self> {
        Arc::new(Self { pool, voice })
    }

    /// 6-h-Lauf: Muster aller aktiven User der letzten 14 Tage.
    pub async fn analyze_all(&self) -> usize {
        let cutoff = Utc::now() - chrono::Duration::days(WINDOW_DAYS);
        let sessions = sqlx::query!(
            r#"
            SELECT user_id,
                   started_at,
                   duration_seconds AS "duration_seconds!"
            FROM activity.voice_session_log
            WHERE started_at >= $1
            ORDER BY user_id, started_at
            "#,
            cutoff,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let mut grouped: HashMap<u64, Vec<(Option<NaiveDateTime>, i64)>> = HashMap::new();
        for row in sessions {
            let Some(user_id) = i64_to_u64(row.user_id, "user_id") else {
                continue;
            };
            grouped
                .entry(user_id)
                .or_default()
                .push((Some(row.started_at.naive_utc()), row.duration_seconds));
        }

        let analyzed = grouped.len();
        for (user_id, user_sessions) in grouped {
            let pattern = analyze_sessions(&user_sessions);
            self.persist_pattern(user_id, &pattern).await;
        }
        tracing::info!(analyzed, "Aktivitätsmuster aktualisiert");
        analyzed
    }

    async fn persist_pattern(&self, user_id: u64, pattern: &ActivityPattern) {
        let result = self.persist_pattern_inner(user_id, pattern).await;
        if let Err(err) = result {
            tracing::warn!(%err, user_id, "Pattern-Persist fehlgeschlagen");
        }
    }

    async fn persist_pattern_inner(
        &self,
        user_id: u64,
        pattern: &ActivityPattern,
    ) -> ActivityDbResult<()> {
        let user_id = discord_id_to_i64(user_id, "user_id")?;
        let typical_hours =
            serde_json::to_string(&pattern.typical_hours).unwrap_or_else(|_| "[]".to_string());
        let typical_days =
            serde_json::to_string(&pattern.typical_days).unwrap_or_else(|_| "[]".to_string());
        let last_active = pattern
            .last_active
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
        let sessions_count = i64_to_i32(pattern.sessions_count, "sessions_count_2w")?;
        let total_minutes = i64_to_i32(pattern.total_minutes, "total_minutes_2w")?;

        sqlx::query!(
            r#"
            INSERT INTO activity.user_activity_patterns(
                user_id, typical_hours, typical_days, activity_score_2w,
                sessions_count_2w, total_minutes_2w, last_active_at, last_analyzed_at
            )
            VALUES($1, $2::text::jsonb, $3::text::jsonb, $4, $5, $6, $7, now())
            ON CONFLICT(user_id) DO UPDATE SET
                typical_hours = EXCLUDED.typical_hours,
                typical_days = EXCLUDED.typical_days,
                activity_score_2w = EXCLUDED.activity_score_2w,
                sessions_count_2w = EXCLUDED.sessions_count_2w,
                total_minutes_2w = EXCLUDED.total_minutes_2w,
                last_active_at = EXCLUDED.last_active_at,
                last_analyzed_at = now()
            "#,
            user_id,
            typical_hours,
            typical_days,
            sessions_count,
            sessions_count,
            total_minutes,
            last_active,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 10-min-Lauf: aktuelle Voice-Paarungen → user_co_players (+1/+10 min).
    pub async fn track_co_players(&self) {
        let groups = self.voice.channel_groups().await;
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    let (a_id, a_name) = group[i].clone();
                    let (b_id, b_name) = group[j].clone();
                    self.record_pair(a_id, &a_name, b_id, &b_name).await;
                }
            }
        }
    }

    async fn record_pair(&self, user_id: u64, user_name: &str, co_id: u64, co_name: &str) {
        if user_id == co_id {
            return;
        }
        let pairs = [
            (user_id, co_id, user_name.to_string(), co_name.to_string()),
            (co_id, user_id, co_name.to_string(), user_name.to_string()),
        ];
        let result = self.record_pairs_inner(&pairs).await;
        if let Err(err) = result {
            tracing::warn!(%err, user_id, co_id, "Co-Player-Persist fehlgeschlagen");
        }
    }

    async fn record_pairs_inner(
        &self,
        pairs: &[(u64, u64, String, String)],
    ) -> ActivityDbResult<()> {
        let minutes = i64_to_i32(CO_PLAYER_MINUTES_PER_TICK, "total_minutes_together")?;
        for (uid, co_uid, uid_name, co_name) in pairs {
            let uid = discord_id_to_i64(*uid, "user_id")?;
            let co_uid = discord_id_to_i64(*co_uid, "co_player_id")?;
            sqlx::query!(
                r#"
                INSERT INTO activity.user_co_players(
                    user_id, co_player_id, sessions_together, total_minutes_together,
                    last_played_together, user_display_name, co_player_display_name
                )
                VALUES($1, $2, 1, $3, now(), $4, $5)
                ON CONFLICT(user_id, co_player_id) DO UPDATE SET
                    sessions_together = COALESCE(activity.user_co_players.sessions_together, 0) + 1,
                    total_minutes_together =
                        COALESCE(activity.user_co_players.total_minutes_together, 0)
                        + EXCLUDED.total_minutes_together,
                    last_played_together = now(),
                    user_display_name =
                        COALESCE(EXCLUDED.user_display_name, activity.user_co_players.user_display_name),
                    co_player_display_name =
                        COALESCE(EXCLUDED.co_player_display_name, activity.user_co_players.co_player_display_name)
                "#,
                uid,
                co_uid,
                minutes,
                uid_name,
                co_name,
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Top-Mitspieler (für LFG/Empfehlungen).
    pub async fn top_co_players(&self, user_id: u64, limit: i64) -> Vec<(u64, i64)> {
        let Ok(user_id) = discord_id_to_i64(user_id, "user_id") else {
            return Vec::new();
        };
        sqlx::query!(
            r#"
            SELECT co_player_id,
                   COALESCE(sessions_together, 0)::INT AS "sessions_together!"
            FROM activity.user_co_players
            WHERE user_id = $1
            ORDER BY COALESCE(sessions_together, 0) DESC,
                     COALESCE(total_minutes_together, 0) DESC
            LIMIT $2
            "#,
            user_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .filter_map(|row| {
                    i64_to_u64(row.co_player_id, "co_player_id")
                        .map(|id| (id, i64::from(row.sessions_together)))
                })
                .collect()
        })
        .unwrap_or_default()
    }
}

/// Website-Unterseiten-Slugs (wie `dl-dashboard::server_stats`) für den
/// Website-Override in `classify`.
const WEBSITE_SLUGS: [&str; 6] = [
    "landing",
    "streamer",
    "mitspieler",
    "coaching",
    "helden",
    "guides",
];

/// Lädt die Lookups für `classify`: `invite_code → streamer_login` (aus
/// `twitch_streamer_invites`) und `invite_code → website-slug` (aus dem
/// `website_invites`-KV).
async fn load_lookups(pool: &PgPool) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut twitch = HashMap::new();
    let twitch_rows = sqlx::query!(
        r#"
        SELECT streamer_login, invite_code
        FROM bot.twitch_streamer_invites
        "#
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for row in twitch_rows {
        let login = row.streamer_login.trim().to_lowercase();
        let code = row.invite_code.unwrap_or_default().trim().to_lowercase();
        if !login.is_empty() && !code.is_empty() {
            twitch.entry(code).or_insert(login);
        }
    }

    let mut website = HashMap::new();
    let website_rows = sqlx::query!(
        r#"
        SELECT k, v
        FROM bot.kv_store
        WHERE ns = 'website_invites'
        "#
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for row in website_rows {
        let slug = if row.k == "main" {
            "landing".to_string()
        } else {
            row.k.trim().to_lowercase()
        };
        if !WEBSITE_SLUGS.contains(&slug.as_str()) {
            continue;
        }
        if let Some(code) = serde_json::from_str::<serde_json::Value>(&row.v)
            .ok()
            .and_then(|p| {
                p.get("code")
                    .and_then(|c| c.as_str())
                    .map(|c| c.trim().to_string())
            })
            .filter(|c| !c.is_empty())
        {
            website.entry(code.to_lowercase()).or_insert(slug);
        }
    }
    (twitch, website)
}

/// Verfeinert die rohen Join-Metadaten über `classify` (Twitch-/Website-
/// Override) und schreibt das Ergebnis zurück in die Felder.
fn apply_classify(
    mut meta: serde_json::Value,
    tw: &HashMap<String, String>,
    web: &HashMap<String, String>,
) -> serde_json::Value {
    let c = crate::join_source::classify(&meta, tw, web);
    if let serde_json::Value::Object(ref mut m) = meta {
        m.insert(
            "join_source_bucket".into(),
            serde_json::Value::from(c.bucket),
        );
        m.insert("join_source_kind".into(), serde_json::Value::from(c.kind));
        m.insert("join_source_label".into(), serde_json::Value::from(c.label));
        if let Some(login) = c.twitch_login {
            m.insert(
                "twitch_streamer_login".into(),
                serde_json::Value::from(login),
            );
        }
        if let Some(code) = c.invite_code {
            m.insert("invite_code".into(), serde_json::Value::from(code));
        }
        if let Some(url) = c.invite_url {
            m.insert("invite_url".into(), serde_json::Value::from(url));
        }
    }
    meta
}

/// member_events-Writer: join/leave/ban/unban persistieren. Joins tragen jetzt
/// die volle Beitrittsquellen-Klassifikation (Invite-Snapshot-Diff aus dem
/// Gateway → `classify`-Verfeinerung). Bots und Privacy-Opt-out übersprungen.
pub fn spawn_member_events(
    pool: PgPool,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_members();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Err(err) = handle_member_event(&pool, event).await {
                        tracing::warn!(%err, "member_events-Insert fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

async fn handle_member_event(
    pool: &PgPool,
    event: dl_discord::MemberEvent,
) -> ActivityDbResult<()> {
    use dl_discord::MemberEvent as M;
    match event {
        M::Join {
            guild_id,
            user_id,
            display_name,
            account_created_at,
            join_position,
            is_bot,
            metadata,
        } => {
            if is_bot {
                return Ok(());
            }
            let (tw, web) = load_lookups(pool).await;
            let refined = apply_classify(metadata, &tw, &web);
            let meta_str = serde_json::to_string(&refined).unwrap_or_else(|_| "{}".to_string());
            let created = DateTime::from_timestamp(account_created_at, 0);
            let join_position = join_position
                .map(|value| i64_to_i32(value, "join_position"))
                .transpose()?;
            insert_member_event(
                pool,
                MemberEventInsert {
                    guild_id,
                    user_id,
                    event_type: "join".to_string(),
                    display_name: Some(display_name),
                    occurred_at: Some(Utc::now()),
                    account_created_at: created,
                    join_position,
                    metadata_json: Some(meta_str),
                    skip_if_join_exists: false,
                },
            )
            .await
            .map(|_| ())
        }
        M::Remove {
            guild_id,
            user_id,
            display_name,
            is_bot,
        } => {
            if is_bot {
                return Ok(());
            }
            insert_simple_event(pool, guild_id, user_id, "leave", Some(display_name)).await
        }
        M::Ban {
            guild_id,
            user_id,
            display_name,
            is_bot,
        } => {
            if is_bot {
                return Ok(());
            }
            insert_simple_event(pool, guild_id, user_id, "ban", Some(display_name)).await
        }
        M::Unban {
            guild_id,
            user_id,
            display_name,
            is_bot,
        } => {
            if is_bot {
                return Ok(());
            }
            insert_simple_event(pool, guild_id, user_id, "unban", Some(display_name)).await
        }
        M::ScreeningCompleted { .. } => Ok(()),
    }
}

async fn insert_simple_event(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
    event_type: &str,
    display_name: Option<String>,
) -> ActivityDbResult<()> {
    insert_member_event(
        pool,
        MemberEventInsert {
            guild_id,
            user_id,
            event_type: event_type.to_string(),
            display_name,
            occurred_at: Some(Utc::now()),
            account_created_at: None,
            join_position: None,
            metadata_json: None,
            skip_if_join_exists: false,
        },
    )
    .await
    .map(|_| ())
}

struct MemberEventInsert {
    guild_id: u64,
    user_id: u64,
    event_type: String,
    display_name: Option<String>,
    occurred_at: Option<DateTime<Utc>>,
    account_created_at: Option<DateTime<Utc>>,
    join_position: Option<i32>,
    metadata_json: Option<String>,
    skip_if_join_exists: bool,
}

async fn insert_member_event(pool: &PgPool, event: MemberEventInsert) -> ActivityDbResult<bool> {
    let user_id = discord_id_to_i64(event.user_id, "user_id")?;
    let guild_id = discord_id_to_i64(event.guild_id, "guild_id")?;
    let metadata_json = event
        .metadata_json
        .map(|raw| validate_json_text(raw, "metadata", "{}"))
        .transpose()?;

    let mut tx = pool.begin().await?;
    lock_member_events(&mut tx).await?;

    let opted_out = sqlx::query!(
        r#"
        SELECT opted_out AS "opted_out!"
        FROM core.user_privacy
        WHERE user_id = $1
          AND opted_out = TRUE
        "#,
        user_id,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if opted_out.is_some() {
        tx.commit().await?;
        return Ok(false);
    }

    if event.skip_if_join_exists {
        let exists = sqlx::query!(
            r#"
            SELECT 1 AS "exists!"
            FROM activity.member_events
            WHERE user_id = $1
              AND guild_id = $2
              AND event_type = 'join'
            LIMIT 1
            "#,
            user_id,
            guild_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if exists.is_some() {
            tx.commit().await?;
            return Ok(false);
        }
    }

    let id = next_member_event_id_in_tx(&mut tx).await?;
    sqlx::query!(
        r#"
        INSERT INTO activity.member_events(
            id, user_id, guild_id, event_type, occurred_at, display_name,
            account_created_at, join_position, metadata
        )
        VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9::text::jsonb)
        "#,
        id,
        user_id,
        guild_id,
        event.event_type,
        event.occurred_at,
        event.display_name,
        event.account_created_at,
        event.join_position,
        metadata_json,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

async fn record_message_activity(
    pool: &PgPool,
    user_id: u64,
    guild_id: u64,
    channel_id: u64,
) -> ActivityDbResult<()> {
    let user_id = discord_id_to_i64(user_id, "user_id")?;
    let guild_id = discord_id_to_i64(guild_id, "guild_id")?;
    let channel_id = discord_id_to_i64(channel_id, "channel_id")?;

    let opted_out = sqlx::query!(
        r#"
        SELECT opted_out AS "opted_out!"
        FROM core.user_privacy
        WHERE user_id = $1
          AND opted_out = TRUE
        "#,
        user_id,
    )
    .fetch_optional(pool)
    .await?;
    if opted_out.is_some() {
        return Ok(());
    }

    sqlx::query!(
        r#"
        INSERT INTO activity.message_activity(
            user_id, guild_id, channel_id, message_count, last_message_at, first_message_at
        )
        VALUES($1, $2, $3, 1, now(), now())
        ON CONFLICT(user_id, guild_id) DO UPDATE SET
            message_count = COALESCE(activity.message_activity.message_count, 0) + 1,
            last_message_at = now(),
            channel_id = EXCLUDED.channel_id
        "#,
        user_id,
        guild_id,
        channel_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_optional_utc(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
        }
    }
    tracing::warn!(
        raw,
        "member_events-Backfill-Zeitstempel konnte nicht geparst werden"
    );
    None
}

pub async fn backfill_member_joins(
    pool: &PgPool,
    members: Vec<BackfillMember>,
) -> ActivityDbResult<usize> {
    let mut inserted = 0usize;
    for member in members {
        if member.is_bot {
            continue;
        }
        let metadata = serde_json::json!({
            "join_source_bucket": "unknown",
            "join_source_kind": "backfilled",
            "join_source_label": BACKFILL_JOIN_SOURCE_LABEL_PLACEHOLDER,
            "join_source_confidence": "none",
            "backfilled": true,
        });
        let metadata = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
        let changed = insert_member_event(
            pool,
            MemberEventInsert {
                guild_id: member.guild_id,
                user_id: member.user_id,
                event_type: "join".to_string(),
                display_name: Some(member.display_name),
                occurred_at: parse_optional_utc(member.joined_at.as_deref()),
                account_created_at: parse_optional_utc(member.account_created_at.as_deref()),
                join_position: None,
                metadata_json: Some(metadata),
                skip_if_join_exists: true,
            },
        )
        .await?;
        if changed {
            inserted += 1;
        }
    }
    Ok(inserted)
}

pub fn spawn_member_backfill(
    pool: PgPool,
    port: Arc<dyn MemberBackfillPort>,
) -> tokio::task::JoinHandle<()> {
    spawn_member_backfill_with_delays(pool, port, Duration::ZERO, MEMBER_BACKFILL_RETRY_INTERVAL)
}

pub fn spawn_member_backfill_with_delays(
    pool: PgPool,
    port: Arc<dyn MemberBackfillPort>,
    initial_delay: Duration,
    retry_delay: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if !initial_delay.is_zero() {
            tokio::time::sleep(initial_delay).await;
        }
        loop {
            if !port.cache_ready().await {
                tracing::debug!("member_events Startup-Backfill wartet auf Gateway-Cache");
                tokio::time::sleep(retry_delay).await;
                continue;
            }
            let members = port.current_members().await;
            if members.is_empty() {
                tracing::debug!("member_events Startup-Backfill wartet auf Cache-Member");
                tokio::time::sleep(retry_delay).await;
                continue;
            }
            match backfill_member_joins(&pool, members).await {
                Ok(inserted) => {
                    tracing::info!(inserted, "member_events Startup-Backfill abgeschlossen")
                }
                Err(err) => tracing::warn!(%err, "member_events Startup-Backfill fehlgeschlagen"),
            }
            break;
        }
    })
}

/// message_activity-Writer: Nachrichten-Zähler je User×Guild (wie der
/// on_message-Tracker des Originals; Privacy-Opt-out wird respektiert).
/// Grundlage u. a. für die Leave-Survey-Einstufung (Bucket A/B/C).
pub fn spawn_message_activity(
    pool: PgPool,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let Some(guild_id) = event.guild_id else {
                        continue;
                    };
                    let (user_id, channel_id) = (event.author_id, event.channel_id);
                    let result =
                        record_message_activity(&pool, user_id, guild_id, channel_id).await;
                    if let Err(err) = result {
                        tracing::warn!(%err, "message_activity-Upsert fehlgeschlagen");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub fn spawn(analyzer: Arc<ActivityAnalyzer>) -> Vec<tokio::task::JoinHandle<()>> {
    let pattern = analyzer.clone();
    let pattern_task = tokio::spawn(async move {
        loop {
            pattern.analyze_all().await;
            tokio::time::sleep(ANALYZE_INTERVAL).await;
        }
    });
    let co_players = analyzer;
    let co_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(CO_PLAYER_INTERVAL).await;
            co_players.track_co_players().await;
        }
    });
    vec![pattern_task, co_task]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "testing")]
    use std::sync::Mutex as StdMutex;

    fn dt(s: &str) -> Option<NaiveDateTime> {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
    }

    #[test]
    fn muster_wie_python() {
        let sessions = vec![
            (dt("2026-06-01 18:30:00"), 3600),
            (dt("2026-06-01 20:10:00"), 1800),
            (dt("2026-06-02 18:05:00"), 900),
            (dt("2026-06-03 21:00:00"), 7200),
            (dt("2026-06-08 18:00:00"), 600),
            (dt("2026-06-08 21:30:00"), 300),
        ];
        let pattern = analyze_sessions(&sessions);
        assert_eq!(pattern.typical_hours, vec![18, 21, 20]);
        assert_eq!(pattern.typical_days, vec![0, 1, 2]);
        assert_eq!(pattern.sessions_count, 6);
        assert_eq!(pattern.total_minutes, 240);
        assert_eq!(pattern.last_active, dt("2026-06-08 21:30:00"));
    }

    #[cfg(feature = "testing")]
    struct MockVoice {
        groups: StdMutex<Vec<Vec<(u64, String)>>>,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl VoiceGroups for MockVoice {
        async fn channel_groups(&self) -> Vec<Vec<(u64, String)>> {
            self.groups.lock().expect("lock").clone()
        }
    }

    #[cfg(feature = "testing")]
    async fn setup() -> Result<
        (dl_central_db::TestDb, Arc<ActivityAnalyzer>, Arc<MockVoice>),
        Box<dyn std::error::Error>,
    > {
        let db = dl_central_db::testing::test_pool().await?;
        let voice = Arc::new(MockVoice {
            groups: StdMutex::new(Vec::new()),
        });
        let analyzer = ActivityAnalyzer::new(db.pool().clone(), voice.clone());
        Ok((db, analyzer, voice))
    }

    #[cfg(feature = "testing")]
    async fn member_event_count(
        pool: &PgPool,
        user_id: i64,
        event_type: &str,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM activity.member_events
            WHERE user_id = $1
              AND event_type = $2
            "#,
            user_id,
            event_type,
        )
        .fetch_one(pool)
        .await?;
        Ok(row.count)
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn member_writer_klassifiziert_und_gated() -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        sqlx::query!(
            r#"
            INSERT INTO bot.twitch_streamer_invites(streamer_login, guild_id, invite_code)
            VALUES('coolstreamer', 1, 'ABC123')
            "#
        )
        .execute(pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out)
            VALUES(999, TRUE)
            "#
        )
        .execute(pool)
        .await?;

        let join = |uid: u64, is_bot: bool| dl_discord::MemberEvent::Join {
            guild_id: 1,
            user_id: uid,
            display_name: "X".into(),
            account_created_at: 1_700_000_000,
            join_position: Some(42),
            is_bot,
            metadata: serde_json::json!({
                "join_source_bucket": "personal",
                "join_source_kind": "invite_link",
                "invite_code": "ABC123",
            }),
        };

        handle_member_event(pool, join(10, false)).await?;
        handle_member_event(pool, join(999, false)).await?;
        handle_member_event(pool, join(11, true)).await?;
        handle_member_event(
            pool,
            dl_discord::MemberEvent::Ban {
                guild_id: 1,
                user_id: 12,
                display_name: "Y".into(),
                is_bot: false,
            },
        )
        .await?;

        let bucket = sqlx::query!(
            r#"
            SELECT metadata->>'join_source_bucket' AS "bucket!"
            FROM activity.member_events
            WHERE user_id = 10
            "#
        )
        .fetch_one(pool)
        .await?
        .bucket;
        let ban_type = sqlx::query!(
            r#"
            SELECT event_type AS "event_type!"
            FROM activity.member_events
            WHERE user_id = 12
            "#
        )
        .fetch_one(pool)
        .await?
        .event_type;

        assert_eq!(bucket, "twitch");
        assert_eq!(member_event_count(pool, 999, "join").await?, 0);
        assert_eq!(member_event_count(pool, 11, "join").await?, 0);
        assert_eq!(ban_type, "ban");
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn analyze_schreibt_patterns() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, analyzer, _voice) = setup().await?;
        let recent = Utc::now() - chrono::Duration::days(2);
        sqlx::query!(
            r#"
            INSERT INTO activity.voice_session_log(
                id, user_id, started_at, ended_at, duration_seconds, points
            )
            VALUES(1, 100, $1, $1, 1800, 0)
            "#,
            recent,
        )
        .execute(&analyzer.pool)
        .await?;

        assert_eq!(analyzer.analyze_all().await, 1);
        let row = sqlx::query!(
            r#"
            SELECT COALESCE(sessions_count_2w, 0)::INT AS "sessions_count!",
                   COALESCE(total_minutes_2w, 0)::INT AS "total_minutes!"
            FROM activity.user_activity_patterns
            WHERE user_id = 100
            "#
        )
        .fetch_one(&analyzer.pool)
        .await?;
        assert_eq!((row.sessions_count, row.total_minutes), (1, 30));
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn co_player_tracking_bidirektional() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, analyzer, voice) = setup().await?;
        *voice.groups.lock().expect("lock") =
            vec![vec![(100, "Anna".to_string()), (200, "Ben".to_string())]];
        analyzer.track_co_players().await;
        analyzer.track_co_players().await;

        let row = sqlx::query!(
            r#"
            SELECT COALESCE(sessions_together, 0)::INT AS "sessions!",
                   COALESCE(total_minutes_together, 0)::INT AS "minutes!",
                   co_player_display_name AS "name!"
            FROM activity.user_co_players
            WHERE user_id = 100
              AND co_player_id = 200
            "#
        )
        .fetch_one(&analyzer.pool)
        .await?;
        assert_eq!((row.sessions, row.minutes), (2, 20));
        assert_eq!(row.name, "Ben");
        assert_eq!(analyzer.top_co_players(200, 5).await, vec![(100, 2)]);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn top_co_players_nutzt_minuten_als_tiebreak() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, analyzer, _voice) = setup().await?;
        sqlx::query!(
            r#"
            INSERT INTO activity.user_co_players(
                user_id, co_player_id, sessions_together, total_minutes_together
            )
            VALUES(1, 10, 3, 30), (1, 20, 3, 90), (1, 30, 2, 200)
            "#
        )
        .execute(&analyzer.pool)
        .await?;

        assert_eq!(
            analyzer.top_co_players(1, 3).await,
            vec![(20, 3), (10, 3), (30, 2)]
        );
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn leave_event_speichert_display_name_und_skippt_bots(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        handle_member_event(
            pool,
            dl_discord::MemberEvent::Remove {
                guild_id: 1,
                user_id: 42,
                display_name: "Gehender User".to_string(),
                is_bot: false,
            },
        )
        .await?;
        handle_member_event(
            pool,
            dl_discord::MemberEvent::Remove {
                guild_id: 1,
                user_id: 99,
                display_name: "Bot".to_string(),
                is_bot: true,
            },
        )
        .await?;

        let name = sqlx::query!(
            r#"
            SELECT display_name AS "display_name!"
            FROM activity.member_events
            WHERE user_id = 42
              AND event_type = 'leave'
            "#
        )
        .fetch_one(pool)
        .await?
        .display_name;
        assert_eq!(name, "Gehender User");
        assert_eq!(member_event_count(pool, 99, "leave").await?, 0);
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn startup_backfill_legt_join_events_fuer_anwesende_member_an(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        sqlx::query!(
            r#"
            INSERT INTO activity.member_events(id, user_id, guild_id, event_type, display_name)
            VALUES(1, 10, 1, 'join', 'Schon da')
            "#
        )
        .execute(pool)
        .await?;
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out)
            VALUES(12, TRUE)
            "#
        )
        .execute(pool)
        .await?;

        let inserted = backfill_member_joins(
            pool,
            vec![
                BackfillMember {
                    guild_id: 1,
                    user_id: 10,
                    display_name: "Schon da".to_string(),
                    joined_at: Some("2026-01-01 10:00:00".to_string()),
                    account_created_at: Some("2025-01-01 10:00:00".to_string()),
                    is_bot: false,
                },
                BackfillMember {
                    guild_id: 1,
                    user_id: 11,
                    display_name: "Neu im Cache".to_string(),
                    joined_at: Some("2026-01-02 10:00:00".to_string()),
                    account_created_at: None,
                    is_bot: false,
                },
                BackfillMember {
                    guild_id: 1,
                    user_id: 12,
                    display_name: "Optout".to_string(),
                    joined_at: None,
                    account_created_at: None,
                    is_bot: false,
                },
                BackfillMember {
                    guild_id: 1,
                    user_id: 13,
                    display_name: "Bot".to_string(),
                    joined_at: None,
                    account_created_at: None,
                    is_bot: true,
                },
            ],
        )
        .await?;

        assert_eq!(inserted, 1);
        assert_eq!(member_event_count(pool, 10, "join").await?, 1);
        let row = sqlx::query!(
            r#"
            SELECT display_name AS "display_name!",
                   metadata->>'join_source_kind' AS "join_source_kind!"
            FROM activity.member_events
            WHERE user_id = 11
            "#
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(row.display_name, "Neu im Cache");
        assert_eq!(row.join_source_kind, "backfilled");
        Ok(())
    }

    #[cfg(feature = "testing")]
    struct SequencedBackfillPort {
        calls: std::sync::atomic::AtomicUsize,
        member: BackfillMember,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl MemberBackfillPort for SequencedBackfillPort {
        async fn current_members(&self) -> Vec<BackfillMember> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                Vec::new()
            } else {
                vec![self.member.clone()]
            }
        }
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn startup_backfill_retryt_bis_cache_member_vorhanden_sind(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool().clone();
        let port = Arc::new(SequencedBackfillPort {
            calls: std::sync::atomic::AtomicUsize::new(0),
            member: BackfillMember {
                guild_id: 1,
                user_id: 55,
                display_name: "Spaeter im Cache".to_string(),
                joined_at: Some("2026-01-03 10:00:00".to_string()),
                account_created_at: None,
                is_bot: false,
            },
        });

        let task = spawn_member_backfill_with_delays(
            pool.clone(),
            port.clone(),
            Duration::ZERO,
            Duration::from_millis(5),
        );
        for _ in 0..40 {
            if member_event_count(&pool, 55, "join").await? == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        task.abort();

        assert_eq!(member_event_count(&pool, 55, "join").await?, 1);
        assert!(
            port.calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "Backfill muss nach leerem Cache erneut pollen"
        );
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn message_activity_respektiert_optout() -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        sqlx::query!(
            r#"
            INSERT INTO core.user_privacy(user_id, opted_out)
            VALUES(77, TRUE)
            "#
        )
        .execute(pool)
        .await?;

        record_message_activity(pool, 77, 1, 9).await?;
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM activity.message_activity
            WHERE user_id = 77
            "#
        )
        .fetch_one(pool)
        .await?;
        assert_eq!(row.count, 0);
        Ok(())
    }
}
