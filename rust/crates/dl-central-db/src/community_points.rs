//! Community-Punkte (Community-Streamer-Bruecke, Paket C).
//!
//! - Spiegel der Twitch-Tageswerte (`community_points.twitch_viewer_daily`,
//!   `twitch_streamer_daily`) samt Cursor (`sync_state`), geschrieben vom
//!   Sync-Bin `dl-community-points-sync`.
//! - Punkte-Ledger fuer Discord-Ereignisse (`community_points.ledger`):
//!   Clip-Contest, Streamer-Vorschlag, qualifizierte Beitritte ueber
//!   Streamer-Einladungen. `(source, ref)` macht jeden Import idempotent.
//! - Lesefunktionen fuer das gemeinsame Leaderboard und das
//!   Partner-Leaderboard je Zeitraum (gesamt, Season = Kalendermonat, Woche),
//!   Tage immer nach Europe/Berlin.
//!
//! Datenschutz wie in [`crate::platform_connections`]: Twitch-Punkte zaehlen
//! nur fuer verknuepfte Mitglieder, Mitglieder mit Privacy-Grabstein
//! (`core.user_privacy.opted_out` oder `deleted_at`) erscheinen nirgends.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Europe::Berlin;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::BTreeSet;

use crate::platform_connections::{is_valid_twitch_user_id, lock_twitch_identity, PLATFORM_TWITCH};
use crate::CentralDbError;

// ─── Punkteregeln (PLAN, Konstanten statt Konfiguration) ────────────────────

/// Streamer: 1 Punkt je 30 Zuschauerminuten verknuepfter Mitglieder (je Tag).
pub const STREAMER_WATCH_MINUTES_PER_POINT: i64 = 30;
/// Streamer: Punkte je erfolgreichem Raid an einen anderen Partner.
pub const STREAMER_RAID_POINTS: i64 = 25;
/// Streamer: Punkte je qualifiziertem Discord-Beitritt ueber eigene Einladungen.
pub const STREAMER_QUALIFIED_JOIN_POINTS: i32 = 50;
/// Clip-Contest: Punkte fuer Platz 1, 2, 3.
pub const CLIP_PLACE_POINTS: [i32; 3] = [100, 60, 40];
/// Clip-Contest: Punkte je abgegebener Stimme (eine Stimme je Woche).
pub const CLIP_VOTE_POINTS: i32 = 2;
/// Streamer-Vorschlag, der Partner wird (erster Vorschlagender).
pub const STREAMER_SUGGESTION_POINTS: i32 = 150;

/// Ledger-Quellen (CHECK in der Migration).
pub const SOURCE_CLIP_PLACE: &str = "clip_place";
pub const SOURCE_CLIP_VOTE: &str = "clip_vote";
pub const SOURCE_STREAMER_SUGGESTION: &str = "streamer_suggestion";
pub const SOURCE_STREAMER_QUALIFIED_JOIN: &str = "streamer_qualified_join";

/// Cursor-Namen in `community_points.sync_state`.
pub const CURSOR_VIEWERS: &str = "twitch_viewers";
pub const CURSOR_STREAMERS: &str = "twitch_streamers";
pub const CURSOR_SUGGESTION_OUTCOMES: &str = "twitch_scout_suggestion_outcomes";

fn viewer_privacy_key(twitch_user_id: &str) -> Vec<u8> {
    Sha256::digest(format!("community-points:twitch-viewer-privacy:v1:{twitch_user_id}").as_bytes())
        .to_vec()
}

/// Speichert vor der Linklöschung den minimalen Schutz gegen erneuten Import.
/// Der Aufrufer hält bereits Identitäts- und Nutzerlock in dieser Reihenfolge.
/// Der Hash ist ein personenbezogenes Sperrmerkmal, keine Anonymisierung.
pub async fn block_linked_viewer_imports(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    discord_id: i64,
) -> Result<u64, sqlx::Error> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT platform_user_id FROM core.discord_platform_connections
          WHERE discord_id = $1 AND platform = 'twitch'",
    )
    .bind(discord_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut written = 0;
    for id in ids {
        written += sqlx::query(
            "INSERT INTO community_points.twitch_viewer_privacy_blocks(subject_hash)
             VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(viewer_privacy_key(&id))
        .execute(&mut **tx)
        .await?
        .rows_affected();
    }
    Ok(written)
}

/// Aktiviert gefilterte Rohaktivität nur nach Optin und erneuter OAuthverknüpfung.
/// Identitäts- und Nutzerlock werden vom Aufrufer gehalten.
pub async fn consent_after_twitch_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    discord_id: i64,
    twitch_id: &str,
) -> Result<(), sqlx::Error> {
    // Ein bewusster Kontowechsel darf die früheren Consentdaten nicht verwaisen lassen.
    let prior: Vec<Vec<u8>> = sqlx::query_scalar(
        "DELETE FROM community_points.twitch_viewer_consents WHERE discord_id = $1
          AND subject_hash <> $2 RETURNING subject_hash",
    )
    .bind(discord_id)
    .bind(viewer_privacy_key(twitch_id))
    .fetch_all(&mut **tx)
    .await?;
    for prior_key in prior {
        sqlx::query(
            "DELETE FROM community_points.twitch_viewer_activity_state WHERE subject_hash = $1",
        )
        .bind(&prior_key)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM community_points.twitch_viewer_daily WHERE sha256(convert_to('community-points:twitch-viewer-privacy:v1:' || twitch_user_id, 'UTF8')) = $1")
            .bind(&prior_key).execute(&mut **tx).await?;
    }
    let key = viewer_privacy_key(twitch_id);
    let erased: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM community_points.twitch_viewer_privacy_blocks WHERE subject_hash = $1)",
    ).bind(&key).fetch_one(&mut **tx).await?;
    if !erased {
        return Ok(());
    }
    // Bei Neuzuordnung keine Einwilligung des früheren Kontoinhabers übernehmen.
    let same_owner: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM community_points.twitch_viewer_consents
          WHERE subject_hash = $1 AND discord_id = $2)",
    )
    .bind(&key)
    .bind(discord_id)
    .fetch_one(&mut **tx)
    .await?;
    if !same_owner {
        sqlx::query("DELETE FROM community_points.twitch_viewer_consents WHERE subject_hash = $1")
            .bind(&key)
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "DELETE FROM community_points.twitch_viewer_activity_state WHERE subject_hash = $1",
        )
        .bind(&key)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM community_points.twitch_viewer_daily WHERE twitch_user_id = $1")
            .bind(twitch_id)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO community_points.twitch_viewer_consents(subject_hash, discord_id, activity_since)
         SELECT $1, $2, clock_timestamp() FROM core.user_privacy
          WHERE user_id = $2 AND NOT opted_out AND deleted_at IS NULL AND reason = 'user_opt_in'
         ON CONFLICT(subject_hash) DO NOTHING",
    ).bind(key).bind(discord_id).execute(&mut **tx).await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ViewerConsent {
    pub twitch_user_id: String,
    pub discord_id: i64,
    pub activity_since: DateTime<Utc>,
}

pub async fn load_viewer_consents(pool: &PgPool) -> Result<Vec<ViewerConsent>, CentralDbError> {
    Ok(sqlx::query_as(
        "SELECT c.platform_user_id AS twitch_user_id, s.discord_id, s.activity_since
           FROM community_points.twitch_viewer_consents s
           JOIN core.discord_platform_connections c ON c.discord_id = s.discord_id
            AND c.platform = 'twitch'
            AND s.subject_hash = sha256(convert_to('community-points:twitch-viewer-privacy:v1:' || c.platform_user_id, 'UTF8'))
          WHERE NOT EXISTS(SELECT 1 FROM core.user_privacy p WHERE p.user_id = s.discord_id
                            AND (p.opted_out OR p.deleted_at IS NOT NULL))
          ORDER BY c.platform_user_id",
    ).fetch_all(pool).await?)
}

/// Authentifizierter Rohdatenstand, ausdrücklich an Identität und Einwilligung gebunden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerActivityDay {
    pub twitch_user_id: String,
    pub day: NaiveDate,
    pub activity_since: DateTime<Utc>,
    pub computed_at: DateTime<Utc>,
    pub rows: Vec<ViewerDailyRow>,
}

/// Punkte fuer einen Clip-Contest-Platz (1..=3), sonst `None`.
pub fn clip_place_points(place: i64) -> Option<i32> {
    usize::try_from(place)
        .ok()
        .and_then(|p| p.checked_sub(1))
        .and_then(|idx| CLIP_PLACE_POINTS.get(idx).copied())
}

/// Streamer-Watchtime-Punkte eines Tages aus den Zuschauerminuten
/// verknuepfter Mitglieder (abgerundet).
pub fn streamer_watch_points(minutes_of_day: i64) -> i64 {
    minutes_of_day.max(0) / STREAMER_WATCH_MINUTES_PER_POINT
}

/// Streamer-Raid-Punkte.
pub fn streamer_raid_points(raids: i64) -> i64 {
    raids.max(0) * STREAMER_RAID_POINTS
}

// ─── Zeitraeume ─────────────────────────────────────────────────────────────

/// Zeitraum eines Leaderboards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    /// Alles seit Beginn.
    Gesamt,
    /// Aktueller Kalendermonat (Season), Europe/Berlin.
    Season,
    /// Aktuelle Woche ab Montag, Europe/Berlin.
    Woche,
}

/// Grenzen eines Zeitraums: Berliner Tage `[start_day, end_day)` und dieselben
/// Grenzen als UTC-Zeitpunkte. `None` fuer [`Period::Gesamt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodBounds {
    pub start_day: NaiveDate,
    pub end_day: NaiveDate,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
}

/// Berliner Kalendertag eines Zeitpunkts.
pub fn berlin_day(ts: DateTime<Utc>) -> NaiveDate {
    ts.with_timezone(&Berlin).date_naive()
}

/// Berliner Mitternacht eines Tages in UTC (Mitternacht ist in Berlin nie
/// von der Zeitumstellung betroffen).
pub fn berlin_midnight_utc(day: NaiveDate) -> DateTime<Utc> {
    let naive = day.and_hms_opt(0, 0, 0).unwrap_or_default();
    Berlin
        .from_local_datetime(&naive)
        .earliest()
        .map(|ts| ts.with_timezone(&Utc))
        .unwrap_or_else(|| naive.and_utc())
}

impl Period {
    /// Grenzen relativ zum Berliner Tag `today`.
    pub fn bounds(self, today: NaiveDate) -> Option<PeriodBounds> {
        let (start_day, end_day) = match self {
            Period::Gesamt => return None,
            Period::Season => {
                let start = today.with_day(1)?;
                let end = if start.month() == 12 {
                    NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)?
                } else {
                    NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)?
                };
                (start, end)
            }
            Period::Woche => {
                let offset = i64::from(today.weekday().num_days_from_monday());
                let start = today - Duration::days(offset);
                (start, start + Duration::days(7))
            }
        };
        Some(PeriodBounds {
            start_day,
            end_day,
            start_at: berlin_midnight_utc(start_day),
            end_at: berlin_midnight_utc(end_day),
        })
    }
}

// ─── Sync: Zeilen, Upsert, Cursor ───────────────────────────────────────────

/// Gepruefte Zuschauer-Tageszeile aus der internen Twitch-API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerDailyRow {
    pub twitch_user_id: String,
    pub channel_twitch_user_id: String,
    pub day: NaiveDate,
    pub watch_minutes: i32,
    pub chat_messages: i32,
    pub points_watch: i32,
    pub points_chat: i32,
    pub points_discovery: i32,
    pub source_updated_at: DateTime<Utc>,
}

/// Gepruefte Streamer-Tageszeile aus der internen Twitch-API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamerDailyRow {
    pub streamer_twitch_user_id: String,
    pub day: NaiveDate,
    pub streamer_login: String,
    pub discord_user_id: Option<i64>,
    pub viewer_minutes: i32,
    pub unique_viewers: i32,
    pub raids_to_partners: i32,
    pub source_updated_at: DateTime<Utc>,
}

/// Gespeicherter Cursor einer Quelle (`None`: noch nie gelesen).
pub async fn load_cursor(pool: &PgPool, name: &str) -> Result<Option<String>, CentralDbError> {
    let cursor: Option<Option<String>> =
        sqlx::query_scalar("SELECT cursor FROM community_points.sync_state WHERE name = $1")
            .bind(name)
            .fetch_optional(pool)
            .await?;
    Ok(cursor.flatten())
}

async fn store_cursor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    name: &str,
    cursor: Option<&str>,
) -> Result<(), CentralDbError> {
    // Ein `null`-Cursor (leere Quelle) loescht nie einen vorhandenen Stand.
    sqlx::query(
        "INSERT INTO community_points.sync_state (name, cursor, updated_at)
         VALUES ($1, $2, now())
         ON CONFLICT (name) DO UPDATE SET
             cursor = COALESCE(EXCLUDED.cursor, community_points.sync_state.cursor),
             updated_at = now()",
    )
    .bind(name)
    .bind(cursor)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Schreibt eine Seite Zuschauerzeilen und den Folge-Cursor in einer
/// Transaktion. Aeltere Quellstaende ueberschreiben nie juengere.
pub async fn apply_viewer_page(
    pool: &PgPool,
    rows: &[ViewerDailyRow],
    next_cursor: Option<&str>,
) -> Result<u64, CentralDbError> {
    apply_viewer_page_with_activity(pool, rows, &[], next_cursor).await
}

pub async fn apply_viewer_page_with_activity(
    pool: &PgPool,
    rows: &[ViewerDailyRow],
    activities: &[ViewerActivityDay],
    next_cursor: Option<&str>,
) -> Result<u64, CentralDbError> {
    for activity in activities {
        let mut channels = BTreeSet::new();
        if !is_valid_twitch_user_id(&activity.twitch_user_id)
            || activity.day < berlin_day(activity.activity_since)
            || activity.computed_at < activity.activity_since
            || activity.rows.iter().any(|r| {
                r.twitch_user_id != activity.twitch_user_id
                    || r.day != activity.day
                    || !is_valid_twitch_user_id(&r.channel_twitch_user_id)
                    || !channels.insert(&r.channel_twitch_user_id)
                    || [
                        r.watch_minutes,
                        r.chat_messages,
                        r.points_watch,
                        r.points_chat,
                        r.points_discovery,
                    ]
                    .iter()
                    .any(|n| *n < 0)
            })
        {
            return Err(CentralDbError::InvalidInput(
                "Rohaktivität passt nicht zur Einwilligung".into(),
            ));
        }
    }
    let mut tx = pool.begin().await?;
    lock_twitch_identity(&mut tx).await?;
    // Die Zuordnung bleibt bis zum Commit stabil. Alle Nutzerlocks werden
    // sortiert nach dem globalen Identitätslock erworben.
    let twitch_ids: Vec<&str> = rows
        .iter()
        .map(|row| row.twitch_user_id.as_str())
        .chain(activities.iter().map(|a| a.twitch_user_id.as_str()))
        .collect();
    let linked: Vec<(String, i64)> = sqlx::query_as(
        "SELECT platform_user_id, discord_id FROM core.discord_platform_connections
          WHERE platform = 'twitch' AND platform_user_id = ANY($1)",
    )
    .bind(&twitch_ids)
    .fetch_all(&mut *tx)
    .await?;
    let ids: BTreeSet<i64> = linked.iter().map(|(_, id)| *id).collect();
    let mut blocked_users = BTreeSet::new();
    for id in ids {
        if crate::lock_user_privacy_and_is_opted_out(&mut tx, id).await? {
            blocked_users.insert(id);
        }
    }
    let blocked_twitch_ids: BTreeSet<&str> = linked
        .iter()
        .filter(|(_, id)| blocked_users.contains(id))
        .map(|(twitch_id, _)| twitch_id.as_str())
        .collect();
    let mut written = 0;
    for row in rows {
        if blocked_twitch_ids.contains(row.twitch_user_id.as_str()) {
            continue;
        }
        let erased: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_points.twitch_viewer_privacy_blocks
                            WHERE subject_hash = $1)",
        )
        .bind(viewer_privacy_key(&row.twitch_user_id))
        .fetch_one(&mut *tx)
        .await?;
        if erased {
            continue;
        }
        written += write_viewer_row(&mut tx, row).await?;
    }
    for activity in activities {
        if blocked_twitch_ids.contains(activity.twitch_user_id.as_str()) {
            continue;
        }
        let key = viewer_privacy_key(&activity.twitch_user_id);
        let current: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_points.twitch_viewer_consents s
               JOIN core.discord_platform_connections c ON c.discord_id = s.discord_id
                AND c.platform = 'twitch' AND c.platform_user_id = $2
              WHERE s.subject_hash = $1 AND s.activity_since = $3)",
        )
        .bind(&key)
        .bind(&activity.twitch_user_id)
        .bind(activity.activity_since)
        .fetch_one(&mut *tx)
        .await?;
        if !current {
            continue;
        }
        let accepted = sqlx::query(
            "INSERT INTO community_points.twitch_viewer_activity_state(subject_hash, day, activity_since, computed_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(subject_hash, day) DO UPDATE SET activity_since = EXCLUDED.activity_since,
                computed_at = EXCLUDED.computed_at
             WHERE community_points.twitch_viewer_activity_state.activity_since = EXCLUDED.activity_since
               AND community_points.twitch_viewer_activity_state.computed_at < EXCLUDED.computed_at",
        ).bind(&key).bind(activity.day).bind(activity.activity_since).bind(activity.computed_at)
            .execute(&mut *tx).await?.rows_affected();
        if accepted == 0 {
            continue;
        }
        // Auch ein bestätigter leerer Tag ersetzt den bisherigen Stand.
        sqlx::query("DELETE FROM community_points.twitch_viewer_daily WHERE twitch_user_id = $1 AND day = $2")
            .bind(&activity.twitch_user_id).bind(activity.day).execute(&mut *tx).await?;
        for row in &activity.rows {
            let mut row = row.clone();
            row.source_updated_at = activity.computed_at;
            written += write_viewer_row(&mut tx, &row).await?;
        }
    }
    store_cursor(&mut tx, CURSOR_VIEWERS, next_cursor).await?;
    tx.commit().await?;
    Ok(written)
}

async fn write_viewer_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &ViewerDailyRow,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "INSERT INTO community_points.twitch_viewer_daily
                 (twitch_user_id, channel_twitch_user_id, day, watch_minutes, chat_messages,
                  points_watch, points_chat, points_discovery, source_updated_at, synced_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
             ON CONFLICT (twitch_user_id, channel_twitch_user_id, day) DO UPDATE SET
                 watch_minutes = EXCLUDED.watch_minutes,
                 chat_messages = EXCLUDED.chat_messages,
                 points_watch = EXCLUDED.points_watch,
                 points_chat = EXCLUDED.points_chat,
                 points_discovery = EXCLUDED.points_discovery,
                 source_updated_at = EXCLUDED.source_updated_at,
                 synced_at = now()
             WHERE community_points.twitch_viewer_daily.source_updated_at
                   <= EXCLUDED.source_updated_at",
    )
    .bind(&row.twitch_user_id)
    .bind(&row.channel_twitch_user_id)
    .bind(row.day)
    .bind(row.watch_minutes)
    .bind(row.chat_messages)
    .bind(row.points_watch)
    .bind(row.points_chat)
    .bind(row.points_discovery)
    .bind(row.source_updated_at)
    .execute(&mut **tx)
    .await?
    .rows_affected())
}

/// Wie [`apply_viewer_page`] fuer Streamer-Tageszeilen.
pub async fn apply_streamer_page(
    pool: &PgPool,
    rows: &[StreamerDailyRow],
    next_cursor: Option<&str>,
) -> Result<u64, CentralDbError> {
    let mut tx = pool.begin().await?;
    // Einheitliche Reihenfolge verhindert wechselseitiges Warten zweier Seiten.
    let ids: BTreeSet<i64> = rows.iter().filter_map(|row| row.discord_user_id).collect();
    let mut blocked = BTreeSet::new();
    for id in ids {
        if crate::lock_user_privacy_and_is_opted_out(&mut tx, id).await? {
            blocked.insert(id);
        }
    }
    let mut written = 0;
    for row in rows {
        if row.discord_user_id.is_some_and(|id| blocked.contains(&id)) {
            continue;
        }
        written += sqlx::query(
            "INSERT INTO community_points.twitch_streamer_daily
                 (streamer_twitch_user_id, day, streamer_login, discord_user_id, viewer_minutes,
                  unique_viewers, raids_to_partners, source_updated_at, synced_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())
             ON CONFLICT (streamer_twitch_user_id, day) DO UPDATE SET
                 streamer_login = EXCLUDED.streamer_login,
                 discord_user_id = EXCLUDED.discord_user_id,
                 viewer_minutes = EXCLUDED.viewer_minutes,
                 unique_viewers = EXCLUDED.unique_viewers,
                 raids_to_partners = EXCLUDED.raids_to_partners,
                 source_updated_at = EXCLUDED.source_updated_at,
                 synced_at = now()
             WHERE community_points.twitch_streamer_daily.source_updated_at
                   <= EXCLUDED.source_updated_at",
        )
        .bind(&row.streamer_twitch_user_id)
        .bind(row.day)
        .bind(&row.streamer_login)
        .bind(row.discord_user_id)
        .bind(row.viewer_minutes)
        .bind(row.unique_viewers)
        .bind(row.raids_to_partners)
        .bind(row.source_updated_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    store_cursor(&mut tx, CURSOR_STREAMERS, next_cursor).await?;
    tx.commit().await?;
    Ok(written)
}

// ─── Ledger ─────────────────────────────────────────────────────────────────

/// Empfaenger eines Ledger-Eintrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerRecipient {
    /// Community-Mitglied (gemeinsames Leaderboard).
    Member(i64),
    /// Partner-Streamer (Streamer-Leaderboard).
    Streamer(String),
}

/// Ein Ereignis fuer das Ledger. `source` ist eine der `SOURCE_*`-Konstanten,
/// `reference` ist je Quelle eindeutig (z. B. `streamer_suggestion:<id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEvent {
    pub recipient: LedgerRecipient,
    pub source: &'static str,
    pub reference: String,
    pub points: i32,
    pub occurred_at: DateTime<Utc>,
}

/// Bucht ein Ereignis genau einmal. `Ok(true)` bei neuer Zeile, `Ok(false)`
/// wenn `(source, ref)` schon gebucht war oder das Mitglied der Speicherung
/// widersprochen hat. Gedacht fuer Paket F (Streamer-Vorschlag).
pub async fn record_ledger_event(
    pool: &PgPool,
    event: &LedgerEvent,
) -> Result<bool, CentralDbError> {
    let mut tx = pool.begin().await?;
    let inserted = insert_ledger_event(&mut tx, event).await?;
    tx.commit().await?;
    Ok(inserted)
}

async fn insert_ledger_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &LedgerEvent,
) -> Result<bool, CentralDbError> {
    let (discord_id, streamer) = match &event.recipient {
        LedgerRecipient::Member(id) if *id > 0 => (Some(*id), None),
        LedgerRecipient::Streamer(id) if is_valid_twitch_user_id(id) => (None, Some(id.clone())),
        _ => {
            return Err(CentralDbError::InvalidInput(
                "ungueltiger Ledger-Empfaenger".to_string(),
            ))
        }
    };
    if let Some(id) = discord_id {
        if crate::lock_user_privacy_and_is_opted_out(tx, id).await? {
            return Ok(false);
        }
    }
    if event.source == SOURCE_STREAMER_SUGGESTION {
        let channel = event
            .reference
            .strip_prefix("streamer_suggestion:")
            .filter(|id| is_valid_twitch_user_id(id))
            .ok_or_else(|| CentralDbError::InvalidInput("Ungültiger Vorschlagskanal".into()))?;
        if discord_id.is_none() || event.points != STREAMER_SUGGESTION_POINTS {
            return Err(CentralDbError::InvalidInput(
                "Ungültiger Vorschlagscredit".into(),
            ));
        }
        // Der personenbezogene Ledger darf gelöscht werden, dieser Kanalnachweis bleibt.
        let first = sqlx::query(
            "INSERT INTO community_points.streamer_suggestion_credits(channel_id)
             VALUES ($1) ON CONFLICT(channel_id) DO NOTHING",
        )
        .bind(channel)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if first == 0 {
            return Ok(false);
        }
    }
    let inserted = sqlx::query(
        "INSERT INTO community_points.ledger
             (discord_id, streamer_twitch_user_id, source, ref, points, occurred_at)
         SELECT $1, $2, $3, $4, $5, $6
          WHERE $1::bigint IS NULL OR NOT EXISTS (
                SELECT 1 FROM core.user_privacy p
                 WHERE p.user_id = $1::bigint
                   AND (p.opted_out = TRUE OR p.deleted_at IS NOT NULL))
         ON CONFLICT (source, ref) DO NOTHING",
    )
    .bind(discord_id)
    .bind(streamer)
    .bind(event.source)
    .bind(&event.reference)
    .bind(event.points)
    .bind(event.occurred_at)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(inserted == 1)
}

/// Individueller Vorschlagscredit mit unveränderter Herkunft und Privacyepoche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionOutcomeEvent {
    pub event: LedgerEvent,
    pub submitted_at: Option<DateTime<Utc>>,
    pub privacy_epoch: Option<i64>,
}

/// Bucht eine Seite Streamer-Vorschlags-Ergebnisse (Twitch-Bot,
/// `scout/community-suggestions/outcomes`) samt Folge-Cursor in einer
/// Transaktion. Je vorgeschlagenem Kanal gibt es die Punkte genau einmal
/// (`ref` = `streamer_suggestion:<twitch_user_id>`), auch wenn der Kanal
/// pausiert und wieder Partner wird. Liefert die Zahl neuer Buchungen.
pub async fn apply_suggestion_outcome_page(
    pool: &PgPool,
    outcomes: &[SuggestionOutcomeEvent],
    next_cursor: Option<&str>,
) -> Result<u64, CentralDbError> {
    let mut tx = pool.begin().await?;
    let ids: BTreeSet<i64> = outcomes
        .iter()
        .filter_map(|outcome| match outcome.event.recipient {
            LedgerRecipient::Member(id) => Some(id),
            LedgerRecipient::Streamer(_) => None,
        })
        .collect();
    for id in ids {
        crate::lock_user_privacy(&mut tx, id).await?;
    }
    // Einheitliche Kanalreihenfolge verhindert gegenläufige Mehrzeilenbuchungen.
    let mut ordered: Vec<_> = outcomes.iter().collect();
    ordered.sort_by(|a, b| a.event.reference.cmp(&b.event.reference));
    let mut written = 0;
    for outcome in ordered {
        let LedgerRecipient::Member(id) = outcome.event.recipient else {
            return Err(CentralDbError::InvalidInput(
                "Vorschlag ohne Mitglied".into(),
            ));
        };
        let privacy: Option<(i64, String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT epoch, action, activity_since FROM community.scout_privacy_epochs
              WHERE subject_hash = sha256(convert_to('scout-community:discord-privacy:v1:' || $1::bigint::text, 'UTF8'))",
        ).bind(id).fetch_optional(&mut *tx).await?;
        let allowed = match privacy {
            None => outcome.privacy_epoch.unwrap_or(0) == 0,
            Some((epoch, action, Some(since))) => {
                action == "consent"
                    && outcome.privacy_epoch == Some(epoch)
                    && outcome.submitted_at.is_some_and(|stamp| stamp >= since)
            }
            Some(_) => false,
        };
        if !allowed {
            continue;
        }
        if let Some(stamp) = outcome.submitted_at {
            let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&mut *tx)
                .await?;
            if stamp > now {
                continue;
            }
        }
        if insert_ledger_event(&mut tx, &outcome.event).await? {
            written += 1;
        }
    }
    store_cursor(&mut tx, CURSOR_SUGGESTION_OUTCOMES, next_cursor).await?;
    tx.commit().await?;
    Ok(written)
}

/// Bucht qualifizierte Beitritte ueber Streamer-Einladungen
/// (`bot.twitch_invite_joins`, Status `qualified`) als 50 Punkte fuer den
/// Kanal des Streamers (`streamer_twitch_user_id`). Persoenliche Links von
/// Zuschauern zaehlen fuer den Kanal, in dem sie entstanden sind. Ref
/// `qualified_invite:<join_id>` wie in `docs/qualified-invites.md`.
pub async fn import_qualified_join_ledger(pool: &PgPool) -> Result<u64, CentralDbError> {
    let mut tx = pool.begin().await?;
    lock_twitch_identity(&mut tx).await?;
    let inserted = sqlx::query(
        "INSERT INTO community_points.ledger
             (streamer_twitch_user_id, source, ref, points, occurred_at, meta)
         SELECT j.streamer_twitch_user_id, $1, 'qualified_invite:' || j.join_id, $2,
                j.qualified_at, jsonb_build_object('join_id', j.join_id::text)
           FROM bot.twitch_invite_joins j
          WHERE j.status = 'qualified'
            AND j.qualified_at IS NOT NULL
            AND j.streamer_twitch_user_id ~ '^[1-9][0-9]{0,19}$'
         ON CONFLICT (source, ref) DO NOTHING",
    )
    .bind(SOURCE_STREAMER_QUALIFIED_JOIN)
    .bind(STREAMER_QUALIFIED_JOIN_POINTS)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(inserted)
}

/// Ergebnis des Clip-Contest-Imports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClipImport {
    /// Die Clip-Contest-Tabellen (Paket D) fehlen noch: nichts importiert.
    pub tables_missing: bool,
    pub places: u64,
    pub votes: u64,
}

/// Bucht Clip-Contest-Punkte aus den Tabellen von Paket D:
/// Plaetze 1..3 aus `clips.clip_contest_results` (Discord-Einsender ueber
/// `user_id`, Twitch-Einsender ueber `streamer_twitch_user_id`) und 2 Punkte
/// je Stimme aus `clips.clip_votes` abgeschlossener Votings
/// (`clips.clip_votings.status = 'closed'`, eine Stimme je Fenster).
///
/// Solange die Tabellen fehlen, passiert nichts (`tables_missing`).
pub async fn import_clip_contest_ledger(pool: &PgPool) -> Result<ClipImport, CentralDbError> {
    let present: bool = sqlx::query_scalar(
        "SELECT to_regclass('clips.clip_contest_results') IS NOT NULL
            AND to_regclass('clips.clip_votes') IS NOT NULL
            AND to_regclass('clips.clip_votings') IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    if !present {
        return Ok(ClipImport {
            tables_missing: true,
            ..ClipImport::default()
        });
    }
    let mut tx = pool.begin().await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT r.user_id FROM clips.clip_contest_results r
          WHERE r.source='discord' AND r.user_id > 0 AND r.place BETWEEN 1 AND 3
            AND NOT EXISTS (SELECT 1 FROM community_points.ledger l
                WHERE l.source=$1 AND l.ref='clip_place:' || r.window_id || ':' || r.place)
         UNION SELECT v.voter_user_id FROM clips.clip_votes v
          JOIN clips.clip_votings vt ON vt.window_id=v.window_id
          WHERE vt.status='closed' AND v.voter_user_id > 0
            AND NOT EXISTS (SELECT 1 FROM community_points.ledger l
                WHERE l.source=$2 AND l.ref='clip_vote:' || v.window_id || ':' || v.voter_user_id)
         ORDER BY 1",
    )
    .bind(SOURCE_CLIP_PLACE)
    .bind(SOURCE_CLIP_VOTE)
    .fetch_all(&mut *tx)
    .await?;
    // Später hinzugekommene Nutzer folgen im nächsten Import mit eigener Sperre.
    for id in &ids {
        crate::lock_user_privacy(&mut tx, *id).await?;
    }
    let places = sqlx::query(
        "INSERT INTO community_points.ledger
             (discord_id, streamer_twitch_user_id, source, ref, points, occurred_at, meta)
         SELECT CASE WHEN r.source = 'discord' THEN r.user_id END,
                CASE WHEN r.source = 'twitch' THEN r.streamer_twitch_user_id END,
                $1, 'clip_place:' || r.window_id || ':' || r.place,
                ($2::int[])[r.place], r.decided_at,
                jsonb_build_object('window_id', r.window_id, 'place', r.place,
                                   'submission_id', r.submission_id)
           FROM clips.clip_contest_results r
          WHERE r.place BETWEEN 1 AND 3
            AND ((r.source = 'discord' AND r.user_id > 0
                  AND r.user_id = ANY($3::bigint[])
                  AND NOT EXISTS (
                      SELECT 1 FROM core.user_privacy p
                       WHERE p.user_id = r.user_id
                         AND (p.opted_out = TRUE OR p.deleted_at IS NOT NULL)))
              OR (r.source = 'twitch'
                  AND r.streamer_twitch_user_id ~ '^[1-9][0-9]{0,19}$'))
         ON CONFLICT (source, ref) DO NOTHING",
    )
    .bind(SOURCE_CLIP_PLACE)
    .bind(CLIP_PLACE_POINTS.to_vec())
    .bind(&ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let votes = sqlx::query(
        "INSERT INTO community_points.ledger
             (discord_id, source, ref, points, occurred_at, meta)
         SELECT v.voter_user_id, $1, 'clip_vote:' || v.window_id || ':' || v.voter_user_id, $2,
                COALESCE(vt.closed_at, vt.voting_end_at, v.updated_at),
                jsonb_build_object('window_id', v.window_id)
           FROM clips.clip_votes v
           JOIN clips.clip_votings vt ON vt.window_id = v.window_id
          WHERE vt.status = 'closed'
            AND v.voter_user_id > 0
            AND v.voter_user_id = ANY($3::bigint[])
            AND NOT EXISTS (
                SELECT 1 FROM core.user_privacy p
                 WHERE p.user_id = v.voter_user_id
                   AND (p.opted_out = TRUE OR p.deleted_at IS NOT NULL))
         ON CONFLICT (source, ref) DO NOTHING",
    )
    .bind(SOURCE_CLIP_VOTE)
    .bind(CLIP_VOTE_POINTS)
    .bind(&ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(ClipImport {
        tables_missing: false,
        places,
        votes,
    })
}

// ─── Lesen: gemeinsames Leaderboard ─────────────────────────────────────────

/// Punkte eines Mitglieds in einem Zeitraum, aufgeschluesselt.
#[derive(Debug, Clone, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct MemberPoints {
    pub discord_id: i64,
    pub voice: i64,
    pub twitch_watch: i64,
    pub twitch_chat: i64,
    pub twitch_discovery: i64,
    pub clips: i64,
    pub suggestions: i64,
}

impl MemberPoints {
    pub fn twitch(&self) -> i64 {
        self.twitch_watch + self.twitch_chat + self.twitch_discovery
    }

    pub fn total(&self) -> i64 {
        self.voice + self.twitch() + self.clips + self.suggestions
    }
}

/// Sortiert absteigend nach Gesamtpunkten, bei Gleichstand nach Discord-ID.
pub fn sort_member_board(rows: &mut [MemberPoints]) {
    rows.sort_by(|a, b| {
        b.total()
            .cmp(&a.total())
            .then_with(|| a.discord_id.cmp(&b.discord_id))
    });
}

/// Platz eines Mitglieds: 1 + Anzahl Mitglieder mit mehr Punkten (gleiche
/// Punkte teilen sich den Platz). `None`, wenn das Mitglied keine Punkte hat.
pub fn member_rank(board: &[MemberPoints], discord_id: i64) -> Option<(usize, i64)> {
    let own = board.iter().find(|row| row.discord_id == discord_id)?;
    let total = own.total();
    let ahead = board.iter().filter(|row| row.total() > total).count();
    Some((ahead + 1, total))
}

const NOT_OPTED_OUT_ID: &str = "NOT EXISTS (
            SELECT 1 FROM core.user_privacy p
             WHERE p.user_id = parts.discord_id
               AND (p.opted_out = TRUE OR p.deleted_at IS NOT NULL)
        )";

/// Gemeinsames Leaderboard eines Zeitraums: Voice (`voice.voice_stats` fuer
/// gesamt, `activity.voice_session_log` je Zeitraum), Twitch-Punkte nur ueber
/// `core.discord_platform_connections` und Ledger-Punkte. Sortiert, ohne
/// Mitglieder mit Privacy-Grabstein und ohne Mitglieder ohne Punkte.
pub async fn community_board(
    pool: &PgPool,
    period: Period,
    today: NaiveDate,
) -> Result<Vec<MemberPoints>, CentralDbError> {
    let bounds = period.bounds(today);
    let sql = format!(
        "WITH parts AS (
             SELECT s.user_id AS discord_id, s.total_points::bigint AS voice,
                    0::bigint AS twitch_watch, 0::bigint AS twitch_chat,
                    0::bigint AS twitch_discovery, 0::bigint AS clips, 0::bigint AS suggestions
               FROM voice.voice_stats s
              WHERE $1::date IS NULL
             UNION ALL
             SELECT l.user_id, COALESCE(sum(l.points), 0)::bigint, 0, 0, 0, 0, 0
               FROM activity.voice_session_log l
              WHERE $1::date IS NOT NULL
                AND l.ended_at >= $3::timestamptz AND l.ended_at < $4::timestamptz
              GROUP BY l.user_id
             UNION ALL
             SELECT c.discord_id, 0,
                    COALESCE(sum(v.points_watch), 0)::bigint,
                    COALESCE(sum(v.points_chat), 0)::bigint,
                    COALESCE(sum(v.points_discovery), 0)::bigint, 0, 0
               FROM community_points.twitch_viewer_daily v
               JOIN core.discord_platform_connections c
                 ON c.platform = $5 AND c.platform_user_id = v.twitch_user_id
              WHERE $1::date IS NULL OR (v.day >= $1::date AND v.day < $2::date)
              GROUP BY c.discord_id
             UNION ALL
             SELECT l.discord_id, 0, 0, 0, 0,
                    COALESCE(sum(l.points) FILTER (WHERE l.source IN ($6, $7)), 0)::bigint,
                    COALESCE(sum(l.points) FILTER (WHERE l.source = $8), 0)::bigint
               FROM community_points.ledger l
              WHERE l.discord_id IS NOT NULL
                AND ($3::timestamptz IS NULL
                     OR (l.occurred_at >= $3::timestamptz AND l.occurred_at < $4::timestamptz))
              GROUP BY l.discord_id
         )
         SELECT parts.discord_id,
                sum(parts.voice)::bigint AS voice,
                sum(parts.twitch_watch)::bigint AS twitch_watch,
                sum(parts.twitch_chat)::bigint AS twitch_chat,
                sum(parts.twitch_discovery)::bigint AS twitch_discovery,
                sum(parts.clips)::bigint AS clips,
                sum(parts.suggestions)::bigint AS suggestions
           FROM parts
          WHERE parts.discord_id > 0
            AND {NOT_OPTED_OUT_ID}
          GROUP BY parts.discord_id
         HAVING sum(parts.voice + parts.twitch_watch + parts.twitch_chat
                    + parts.twitch_discovery + parts.clips + parts.suggestions) > 0"
    );
    let mut rows = sqlx::query_as::<_, MemberPoints>(&sql)
        .bind(bounds.map(|b| b.start_day))
        .bind(bounds.map(|b| b.end_day))
        .bind(bounds.map(|b| b.start_at))
        .bind(bounds.map(|b| b.end_at))
        .bind(PLATFORM_TWITCH)
        .bind(SOURCE_CLIP_PLACE)
        .bind(SOURCE_CLIP_VOTE)
        .bind(SOURCE_STREAMER_SUGGESTION)
        .fetch_all(pool)
        .await?;
    sort_member_board(&mut rows);
    Ok(rows)
}

// ─── Lesen: Partner-Leaderboard ─────────────────────────────────────────────

/// Punkte eines Partner-Streamers in einem Zeitraum.
#[derive(Debug, Clone, Default, PartialEq, Eq, sqlx::FromRow)]
pub struct StreamerPoints {
    pub streamer_twitch_user_id: String,
    pub streamer_login: String,
    pub discord_user_id: Option<i64>,
    /// Zuschauerminuten verknuepfter Community-Mitglieder im eigenen Kanal.
    pub community_minutes: i64,
    /// Daraus: 1 Punkt je 30 Minuten, je Tag abgerundet.
    pub watch_points: i64,
    pub raids: i64,
    pub qualified_joins: i64,
    pub join_points: i64,
    pub clip_points: i64,
}

impl StreamerPoints {
    pub fn raid_points(&self) -> i64 {
        streamer_raid_points(self.raids)
    }

    pub fn total(&self) -> i64 {
        self.watch_points + self.raid_points() + self.join_points + self.clip_points
    }
}

/// Sortiert absteigend nach Punkten, bei Gleichstand nach Login.
pub fn sort_streamer_board(rows: &mut [StreamerPoints]) {
    rows.sort_by(|a, b| {
        b.total()
            .cmp(&a.total())
            .then_with(|| a.streamer_login.cmp(&b.streamer_login))
            .then_with(|| a.streamer_twitch_user_id.cmp(&b.streamer_twitch_user_id))
    });
}

/// Partner-Leaderboard: alle Streamer mit Tageszeilen aus dem Twitch-Bot
/// (Login und Discord-ID aus der juengsten Zeile). Watchtime nur von
/// verknuepften Zuschauern ohne Privacy-Grabstein, je Tag abgerundet. Streamer
/// mit Privacy-Grabstein und Streamer ohne Punkte fehlen.
pub async fn streamer_board(
    pool: &PgPool,
    period: Period,
    today: NaiveDate,
) -> Result<Vec<StreamerPoints>, CentralDbError> {
    let bounds = period.bounds(today);
    let mut rows = sqlx::query_as::<_, StreamerPoints>(
        "WITH partners AS (
             SELECT DISTINCT ON (d.streamer_twitch_user_id)
                    d.streamer_twitch_user_id, d.streamer_login, d.discord_user_id
               FROM community_points.twitch_streamer_daily d
              ORDER BY d.streamer_twitch_user_id, d.day DESC, d.source_updated_at DESC
         ),
         watch_days AS (
             SELECT v.channel_twitch_user_id AS streamer_twitch_user_id, v.day,
                    sum(v.watch_minutes)::bigint AS minutes
               FROM community_points.twitch_viewer_daily v
               JOIN core.discord_platform_connections c
                 ON c.platform = $5 AND c.platform_user_id = v.twitch_user_id
              WHERE v.twitch_user_id <> v.channel_twitch_user_id
                AND ($1::date IS NULL OR (v.day >= $1::date AND v.day < $2::date))
                AND NOT EXISTS (
                    SELECT 1 FROM core.user_privacy p
                     WHERE p.user_id = c.discord_id
                       AND (p.opted_out = TRUE OR p.deleted_at IS NOT NULL))
              GROUP BY v.channel_twitch_user_id, v.day
         ),
         watch AS (
             SELECT streamer_twitch_user_id,
                    sum(minutes)::bigint AS minutes,
                    sum(minutes / $6)::bigint AS points
               FROM watch_days
              GROUP BY streamer_twitch_user_id
         ),
         raids AS (
             SELECT d.streamer_twitch_user_id, sum(d.raids_to_partners)::bigint AS raids
               FROM community_points.twitch_streamer_daily d
              WHERE $1::date IS NULL OR (d.day >= $1::date AND d.day < $2::date)
              GROUP BY d.streamer_twitch_user_id
         ),
         led AS (
             SELECT l.streamer_twitch_user_id,
                    count(*) FILTER (WHERE l.source = $7)::bigint AS joins,
                    COALESCE(sum(l.points) FILTER (WHERE l.source = $7), 0)::bigint AS join_points,
                    COALESCE(sum(l.points) FILTER (WHERE l.source = $8), 0)::bigint AS clip_points
               FROM community_points.ledger l
              WHERE l.streamer_twitch_user_id IS NOT NULL
                AND ($3::timestamptz IS NULL
                     OR (l.occurred_at >= $3::timestamptz AND l.occurred_at < $4::timestamptz))
              GROUP BY l.streamer_twitch_user_id
         )
         SELECT p.streamer_twitch_user_id, p.streamer_login, p.discord_user_id,
                COALESCE(w.minutes, 0)::bigint AS community_minutes,
                COALESCE(w.points, 0)::bigint AS watch_points,
                COALESCE(r.raids, 0)::bigint AS raids,
                COALESCE(l.joins, 0)::bigint AS qualified_joins,
                COALESCE(l.join_points, 0)::bigint AS join_points,
                COALESCE(l.clip_points, 0)::bigint AS clip_points
           FROM partners p
           LEFT JOIN watch w ON w.streamer_twitch_user_id = p.streamer_twitch_user_id
           LEFT JOIN raids r ON r.streamer_twitch_user_id = p.streamer_twitch_user_id
           LEFT JOIN led l ON l.streamer_twitch_user_id = p.streamer_twitch_user_id
          WHERE p.discord_user_id IS NULL OR NOT EXISTS (
                SELECT 1 FROM core.user_privacy pr
                 WHERE pr.user_id = p.discord_user_id
                   AND (pr.opted_out = TRUE OR pr.deleted_at IS NOT NULL))",
    )
    .bind(bounds.map(|b| b.start_day))
    .bind(bounds.map(|b| b.end_day))
    .bind(bounds.map(|b| b.start_at))
    .bind(bounds.map(|b| b.end_at))
    .bind(PLATFORM_TWITCH)
    .bind(STREAMER_WATCH_MINUTES_PER_POINT)
    .bind(SOURCE_STREAMER_QUALIFIED_JOIN)
    .bind(SOURCE_CLIP_PLACE)
    .fetch_all(pool)
    .await?;
    rows.retain(|row| row.total() > 0);
    sort_streamer_board(&mut rows);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn punkteformeln_nach_plan() {
        assert_eq!(clip_place_points(1), Some(100));
        assert_eq!(clip_place_points(2), Some(60));
        assert_eq!(clip_place_points(3), Some(40));
        assert_eq!(clip_place_points(0), None);
        assert_eq!(clip_place_points(4), None);
        assert_eq!(clip_place_points(-1), None);
        assert_eq!(streamer_watch_points(29), 0);
        assert_eq!(streamer_watch_points(30), 1);
        assert_eq!(streamer_watch_points(4200), 140);
        assert_eq!(streamer_watch_points(-5), 0);
        assert_eq!(streamer_raid_points(2), 50);
        assert_eq!(streamer_raid_points(-1), 0);
        assert_eq!(CLIP_VOTE_POINTS, 2);
        assert_eq!(STREAMER_SUGGESTION_POINTS, 150);
        assert_eq!(STREAMER_QUALIFIED_JOIN_POINTS, 50);
    }

    #[test]
    fn zeitraeume_nach_berliner_kalender() {
        assert_eq!(Period::Gesamt.bounds(d(2026, 10, 1)), None);
        let season = Period::Season.bounds(d(2026, 10, 15)).unwrap();
        assert_eq!(
            (season.start_day, season.end_day),
            (d(2026, 10, 1), d(2026, 11, 1))
        );
        // Oktober startet in Sommerzeit (UTC+2), November in Winterzeit (UTC+1).
        assert_eq!(season.start_at.to_rfc3339(), "2026-09-30T22:00:00+00:00");
        assert_eq!(season.end_at.to_rfc3339(), "2026-10-31T23:00:00+00:00");
        let dez = Period::Season.bounds(d(2026, 12, 31)).unwrap();
        assert_eq!(
            (dez.start_day, dez.end_day),
            (d(2026, 12, 1), d(2027, 1, 1))
        );
        // 2026-10-01 ist ein Donnerstag.
        let woche = Period::Woche.bounds(d(2026, 10, 1)).unwrap();
        assert_eq!(
            (woche.start_day, woche.end_day),
            (d(2026, 9, 28), d(2026, 10, 5))
        );
        let montag = Period::Woche.bounds(d(2026, 9, 28)).unwrap();
        assert_eq!(montag.start_day, d(2026, 9, 28));
        let sonntag = Period::Woche.bounds(d(2026, 10, 4)).unwrap();
        assert_eq!(sonntag.start_day, d(2026, 9, 28));
    }

    #[test]
    fn berliner_tag_kippt_um_mitternacht_ortszeit() {
        let late = Utc.with_ymd_and_hms(2026, 9, 30, 22, 30, 0).unwrap();
        assert_eq!(berlin_day(late), d(2026, 10, 1));
        let winter = Utc.with_ymd_and_hms(2026, 12, 31, 22, 59, 0).unwrap();
        assert_eq!(berlin_day(winter), d(2026, 12, 31));
    }

    fn member(id: i64, voice: i64, watch: i64) -> MemberPoints {
        MemberPoints {
            discord_id: id,
            voice,
            twitch_watch: watch,
            ..MemberPoints::default()
        }
    }

    #[test]
    fn rangliste_teilt_plaetze_bei_gleichstand() {
        let mut board = vec![
            member(3, 10, 0),
            member(1, 5, 5),
            member(2, 50, 0),
            member(4, 1, 0),
        ];
        sort_member_board(&mut board);
        let ids: Vec<i64> = board.iter().map(|r| r.discord_id).collect();
        assert_eq!(ids, vec![2, 1, 3, 4]);
        assert_eq!(member_rank(&board, 2), Some((1, 50)));
        assert_eq!(member_rank(&board, 1), Some((2, 10)));
        assert_eq!(member_rank(&board, 3), Some((2, 10)));
        assert_eq!(member_rank(&board, 4), Some((4, 1)));
        assert_eq!(member_rank(&board, 99), None);
    }

    #[test]
    fn streamer_summe_nach_plan() {
        let s = StreamerPoints {
            streamer_twitch_user_id: "456".into(),
            streamer_login: "name".into(),
            watch_points: 7,
            raids: 2,
            qualified_joins: 1,
            join_points: 50,
            clip_points: 100,
            ..StreamerPoints::default()
        };
        assert_eq!(s.raid_points(), 50);
        assert_eq!(s.total(), 7 + 50 + 50 + 100);
    }
}
