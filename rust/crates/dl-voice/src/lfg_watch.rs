use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::lfg_panel::{play_window_dm_suffix, LfgPanelPort, LfgRankRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfgWatchWindow {
    Now3h,
    Today,
    Weekend,
    Week,
}

impl LfgWatchWindow {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Now3h => "now3h",
            Self::Today => "today",
            Self::Weekend => "weekend",
            Self::Week => "week",
        }
    }

    pub(crate) fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "now3h" => Some(Self::Now3h),
            "today" => Some(Self::Today),
            "weekend" => Some(Self::Weekend),
            "week" => Some(Self::Week),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LfgWatch {
    pub id: i64,
    pub user_id: i64,
    pub guild_id: i64,
    pub mode: String,
    pub rank_min: Option<i32>,
    pub rank_max: Option<i32>,
    pub window_kind: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn insert_or_replace_watch(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
    mode: &str,
    range: LfgRankRange,
    window: LfgWatchWindow,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO activity.lfg_watches (
             user_id, guild_id, mode, rank_min, rank_max, window_kind, expires_at
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (guild_id, user_id) WHERE fired_at IS NULL
         DO UPDATE SET
             mode = EXCLUDED.mode,
             rank_min = EXCLUDED.rank_min,
             rank_max = EXCLUDED.rank_max,
             window_kind = EXCLUDED.window_kind,
             expires_at = EXCLUDED.expires_at,
             created_at = now(),
             matched_post_id = NULL",
    )
    .bind(user_id as i64)
    .bind(guild_id as i64)
    .bind(mode)
    .bind(range.min)
    .bind(range.max)
    .bind(window.as_str())
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn armed_watches_for_match(
    pool: &PgPool,
    guild_id: u64,
    mode: &str,
    exclude_user: u64,
) -> Result<Vec<LfgWatch>, sqlx::Error> {
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            i64,
            i64,
            String,
            Option<i32>,
            Option<i32>,
            String,
            DateTime<Utc>,
        ),
    >(
        "SELECT id, user_id, guild_id, mode, rank_min, rank_max, window_kind, expires_at
           FROM activity.lfg_watches
          WHERE guild_id = $1
            AND mode = $2
            AND fired_at IS NULL
            AND expires_at > now()
            AND user_id <> $3",
    )
    .bind(guild_id as i64)
    .bind(mode)
    .bind(exclude_user as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, user_id, guild_id, mode, rank_min, rank_max, window_kind, expires_at)| LfgWatch {
                id,
                user_id,
                guild_id,
                mode,
                rank_min,
                rank_max,
                window_kind,
                expires_at,
            },
        )
        .collect())
}

pub async fn claim_watch(
    pool: &PgPool,
    watch_id: i64,
    matched_post_id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE activity.lfg_watches
            SET fired_at = now(),
                matched_post_id = $2
          WHERE id = $1
            AND fired_at IS NULL",
    )
    .bind(watch_id)
    .bind(matched_post_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub fn rank_overlaps(watch_min: Option<i32>, watch_max: Option<i32>, post: LfgRankRange) -> bool {
    let watch_lo = watch_min.unwrap_or(1);
    let watch_hi = watch_max.unwrap_or(11);
    let post_lo = post.min.unwrap_or(1);
    let post_hi = post.max.unwrap_or(11);
    watch_lo <= post_hi && post_lo <= watch_hi
}

pub async fn typical_active_hours(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
) -> Result<Option<HashSet<u8>>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i32, i64)>(
        "SELECT EXTRACT(HOUR FROM started_at AT TIME ZONE 'Europe/Berlin')::int AS hour,
                count(*) AS n
           FROM activity.voice_session_log
          WHERE guild_id = $1
            AND user_id = $2
            AND started_at > now() - interval '90 days'
          GROUP BY hour",
    )
    .bind(guild_id as i64)
    .bind(user_id as i64)
    .fetch_all(pool)
    .await?;
    let total: i64 = rows.iter().map(|(_, count)| *count).sum();
    if total < 10 {
        return Ok(None);
    }

    let max = rows.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let threshold = (max / 3).max(3);
    let mut hours = HashSet::new();
    for (hour, count) in rows {
        if count >= threshold {
            let hour = hour.rem_euclid(24) as u8;
            hours.insert(hour);
            hours.insert((hour + 23) % 24);
            hours.insert((hour + 1) % 24);
        }
    }
    Ok(Some(hours))
}

pub fn timing_actionable(
    now_hour_berlin: u8,
    weekday_is_weekend: bool,
    in_voice_now: bool,
    typical: &Option<HashSet<u8>>,
    window: LfgWatchWindow,
    post_window: Option<&str>,
) -> bool {
    if in_voice_now {
        return true;
    }
    if matches!(post_window, Some("jetzt")) {
        return true;
    }
    let window_says_now = match window {
        LfgWatchWindow::Now3h | LfgWatchWindow::Today => true,
        LfgWatchWindow::Weekend => weekday_is_weekend,
        LfgWatchWindow::Week => false,
    };
    if window_says_now {
        return true;
    }
    match typical {
        Some(hours) => hours.contains(&now_hour_berlin),
        None => true,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_match_for_new_post(
    pool: &PgPool,
    port: &dyn LfgPanelPort,
    guild_id: u64,
    post_id: i64,
    thread_id: u64,
    owner_id: u64,
    mode: &str,
    mode_display: &str,
    range: LfgRankRange,
    post_window: Option<String>,
    rank_label: String,
) {
    let watches = match armed_watches_for_match(pool, guild_id, mode, owner_id).await {
        Ok(watches) => watches,
        Err(err) => {
            tracing::error!(%err, "LFG-Matcher: armed_watches fehlgeschlagen");
            return;
        }
    };
    let (now_hour, is_weekend): (i32, bool) = match sqlx::query_as(
        "SELECT EXTRACT(HOUR FROM now() AT TIME ZONE 'Europe/Berlin')::int,
                EXTRACT(ISODOW FROM now() AT TIME ZONE 'Europe/Berlin') >= 6",
    )
    .fetch_one(pool)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(%err, "LFG-Matcher: Zeit");
            return;
        }
    };
    let now_hour = now_hour.rem_euclid(24) as u8;
    let guild_id_i64 = match i64::try_from(guild_id) {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(%err, guild_id, "LFG-Matcher: guild_id passt nicht in BIGINT");
            return;
        }
    };

    for watch in watches {
        if watch.guild_id != guild_id_i64 || watch.expires_at <= Utc::now() {
            continue;
        }
        if watch.mode != mode {
            continue;
        }
        if !rank_overlaps(watch.rank_min, watch.rank_max, range) {
            continue;
        }
        let Some(window) = LfgWatchWindow::from_str(&watch.window_kind) else {
            tracing::warn!(
                watch_id = watch.id,
                window_kind = %watch.window_kind,
                "LFG-Matcher: unbekanntes Watch-Fenster ignoriert"
            );
            continue;
        };
        let watcher_id = match u64::try_from(watch.user_id) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(%err, user_id = watch.user_id, "LFG-Matcher: user_id passt nicht in u64");
                continue;
            }
        };
        let in_voice = port
            .member_voice_channel(guild_id, watcher_id)
            .await
            .is_some();
        let typical = match typical_active_hours(pool, guild_id, watcher_id).await {
            Ok(hours) => hours,
            Err(err) => {
                tracing::warn!(%err, user_id = watch.user_id, "LFG-Matcher: typische Stunden konnten nicht geladen werden");
                None
            }
        };
        if !timing_actionable(
            now_hour,
            is_weekend,
            in_voice,
            &typical,
            window,
            post_window.as_deref(),
        ) {
            continue;
        }

        match claim_watch(pool, watch.id, post_id).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(err) => {
                tracing::error!(%err, watch_id = watch.id, "LFG-Matcher: claim fehlgeschlagen");
                continue;
            }
        }

        let window_suffix = post_window
            .as_deref()
            .and_then(play_window_dm_suffix)
            .unwrap_or_default();
        let content = format!(
            "🔔 **Passendes Gesuch!**\n**{mode_display}** · {rank_label}{window_suffix} — von <@{owner_id}>.\nSchau vorbei: https://discord.com/channels/{guild_id}/{thread_id}\n\n_Einmalige Benachrichtigung. Für neue klick wieder auf „🔔 Benachrichtige mich\" im LFG-Panel._"
        );
        if let Err(err) = port.send_dm(watcher_id, content).await {
            tracing::warn!(%err, user_id = watch.user_id, "LFG-Matcher: DM fehlgeschlagen");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rank_overlap_matrix() {
        let egal = LfgRankRange {
            min: None,
            max: None,
        };
        assert!(rank_overlaps(None, None, egal));
        assert!(rank_overlaps(Some(3), Some(5), egal));
        assert!(rank_overlaps(
            None,
            None,
            LfgRankRange {
                min: Some(7),
                max: Some(9),
            }
        ));
        assert!(rank_overlaps(
            Some(3),
            Some(6),
            LfgRankRange {
                min: Some(5),
                max: Some(8),
            }
        ));
        assert!(!rank_overlaps(
            Some(1),
            Some(3),
            LfgRankRange {
                min: Some(7),
                max: Some(9),
            }
        ));
    }

    #[test]
    fn timing_or_logic() {
        let none: Option<HashSet<u8>> = None;
        assert!(timing_actionable(
            3,
            false,
            true,
            &none,
            LfgWatchWindow::Week,
            None
        ));
        assert!(timing_actionable(
            3,
            false,
            false,
            &none,
            LfgWatchWindow::Week,
            None
        ));
        assert!(timing_actionable(
            14,
            false,
            false,
            &none,
            LfgWatchWindow::Now3h,
            None
        ));
        assert!(timing_actionable(
            14,
            false,
            false,
            &none,
            LfgWatchWindow::Weekend,
            None
        ));
        assert!(timing_actionable(
            14,
            true,
            false,
            &none,
            LfgWatchWindow::Weekend,
            None
        ));
        assert!(timing_actionable(
            14,
            false,
            false,
            &none,
            LfgWatchWindow::Week,
            Some("jetzt")
        ));
        let typ: Option<HashSet<u8>> = Some([14u8].into_iter().collect());
        assert!(timing_actionable(
            14,
            false,
            false,
            &typ,
            LfgWatchWindow::Week,
            None
        ));
    }

    #[tokio::test]
    async fn typical_hours_from_seeded_sessions() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let date = (Utc::now() - chrono::Duration::days(5)).date_naive();
        let started = DateTime::<Utc>::from_naive_utc_and_offset(
            date.and_hms_opt(19, 0, 0).expect("valid test time"),
            Utc,
        );
        let ended = started + chrono::Duration::minutes(30);
        for idx in 0..12 {
            sqlx::query(
                "INSERT INTO activity.voice_session_log (
                     id, user_id, guild_id, channel_id, started_at, ended_at,
                     duration_seconds, points
                 )
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(600_000_i64 + idx)
            .bind(99_i64)
            .bind(1_i64)
            .bind(1_i64)
            .bind(started)
            .bind(ended)
            .bind(1_800_i64)
            .bind(0_i32)
            .execute(&pool)
            .await
            .expect("seed");
        }
        let hours = typical_active_hours(&pool, 1, 99)
            .await
            .expect("hours")
            .expect("not cold");
        assert!(hours.contains(&21), "21 Uhr Berlin muss typisch sein");
    }

    #[tokio::test]
    async fn upsert_replaces_armed_watch() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let exp = Utc::now() + chrono::Duration::hours(3);
        insert_or_replace_watch(
            &pool,
            1,
            42,
            "casual",
            LfgRankRange {
                min: None,
                max: None,
            },
            LfgWatchWindow::Now3h,
            exp,
        )
        .await
        .expect("first");
        let second = insert_or_replace_watch(
            &pool,
            1,
            42,
            "ranked",
            LfgRankRange {
                min: Some(3),
                max: Some(5),
            },
            LfgWatchWindow::Today,
            exp,
        )
        .await;
        assert!(
            second.is_ok(),
            "zweiter Upsert darf keinen Unique-Index-Fehler werfen: {second:?}"
        );
        let armed = armed_watches_for_match(&pool, 1, "ranked", 0)
            .await
            .expect("armed");
        assert_eq!(armed.len(), 1);
        assert_eq!(armed[0].mode, "ranked");
        let casual = armed_watches_for_match(&pool, 1, "casual", 0)
            .await
            .expect("casual");
        assert!(casual.is_empty(), "alter casual-Watch muss ersetzt sein");
    }

    #[tokio::test]
    async fn claim_watch_claimt_nur_einmal() {
        let db = dl_central_db::testing::test_pool().await.expect("pool");
        let pool = db.pool().clone();
        let exp = Utc::now() + chrono::Duration::hours(3);
        insert_or_replace_watch(
            &pool,
            1,
            43,
            "casual",
            LfgRankRange {
                min: None,
                max: None,
            },
            LfgWatchWindow::Now3h,
            exp,
        )
        .await
        .expect("watch");
        let armed = armed_watches_for_match(&pool, 1, "casual", 0)
            .await
            .expect("armed");
        assert_eq!(armed.len(), 1);

        assert!(claim_watch(&pool, armed[0].id, 9001)
            .await
            .expect("first claim"));
        assert!(!claim_watch(&pool, armed[0].id, 9002)
            .await
            .expect("second claim"));
    }
}
