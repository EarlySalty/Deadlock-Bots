use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::*;

pub type PoolResult<T> = Result<T, PoolError>;

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("Ungültige Pool-Daten: {0}")]
    Invalid(&'static str),
    #[error("Datenschutz-Opt-out verhindert die Verarbeitung")]
    OptedOut,
    #[error("Profil, Session oder Teilnahme nicht gefunden")]
    NotFound,
    #[error("Steam-Verknüpfung fehlt, ist unbestätigt oder wurde gewechselt")]
    UnverifiedSteamLink,
    #[error("Der Session-Zustand erlaubt diese Änderung nicht")]
    InvalidSessionState,
    #[error("Der API-Stand ist älter als der gespeicherte Stand")]
    StaleSnapshot,
}

#[derive(Clone)]
pub struct PoolStore {
    pool: PgPool,
}

const PUBLIC_PROJECTION: &str = "SELECT p.guild_id, p.discord_id, p.modes,
    p.group_size_min, p.group_size_max, p.play_style, p.voice_preference, p.languages, p.timezone,
    a.rank_tier, a.rank_subtier, a.games_played, a.total_play_seconds, a.observed_games,
    a.observed_play_seconds, a.window_start, a.window_end, a.fetched_at, a.heatmap
    FROM pool.profiles p LEFT JOIN pool.api_snapshots a USING (guild_id, discord_id)
    WHERE p.guild_id = $1 AND p.published
    AND NOT EXISTS (SELECT 1 FROM core.user_privacy u WHERE u.user_id = p.discord_id
                    AND (u.opted_out OR u.deleted_at IS NOT NULL))";

fn validate_scope(scope: Scope) -> PoolResult<()> {
    if scope.guild_id <= 0 || scope.discord_id <= 0 {
        return Err(PoolError::Invalid(
            "Guild-ID und Discord-ID müssen positiv sein",
        ));
    }
    Ok(())
}

async fn lock_users(tx: &mut Transaction<'_, Postgres>, users: &[i64]) -> PoolResult<()> {
    for user in users.iter().copied().collect::<BTreeSet<_>>() {
        if user <= 0 {
            return Err(PoolError::Invalid("Discord-ID muss positiv sein"));
        }
        if dl_central_db::lock_user_privacy_and_is_opted_out(tx, user).await? {
            return Err(PoolError::OptedOut);
        }
    }
    Ok(())
}

fn validate_preferences(p: &Preferences) -> PoolResult<()> {
    if p.group_size_min < 2 || p.group_size_max > 6 || p.group_size_min > p.group_size_max {
        return Err(PoolError::Invalid(
            "Gruppengröße muss zwischen 2 und 6 liegen",
        ));
    }
    if p.timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(PoolError::Invalid("Zeitzone ist unbekannt"));
    }
    if p.modes.len() > 3 || p.modes.iter().collect::<BTreeSet<_>>().len() != p.modes.len() {
        return Err(PoolError::Invalid("Spielmodi müssen eindeutig sein"));
    }
    if p.languages.is_empty()
        || p.languages.len() > 10
        || p.languages.iter().any(|s| {
            s.len() < 2 || s.len() > 16 || !s.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')
        })
    {
        return Err(PoolError::Invalid("Sprachcodes sind ungültig"));
    }
    if p.availability.len() > 168 || p.preferred_discord_ids.len() > 100 {
        return Err(PoolError::Invalid("Zu viele Zeitfenster oder Mitspieler"));
    }
    for a in &p.availability {
        if !(0..=6).contains(&a.weekday)
            || a.start_minute < 0
            || a.end_minute > 1440
            || a.start_minute >= a.end_minute
        {
            return Err(PoolError::Invalid("Zeitfenster ist ungültig"));
        }
    }
    Ok(())
}

impl PoolStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn save_preferences(
        &self,
        scope: Scope,
        preferences: &Preferences,
        completed: bool,
    ) -> PoolResult<Profile> {
        validate_scope(scope)?;
        validate_preferences(preferences)?;
        if completed && preferences.modes.is_empty() {
            return Err(PoolError::Invalid(
                "Ein fertiges Profil braucht einen Spielmodus",
            ));
        }
        if preferences
            .preferred_discord_ids
            .contains(&scope.discord_id)
        {
            return Err(PoolError::Invalid(
                "Eigene ID ist kein bevorzugter Mitspieler",
            ));
        }
        let mut users = preferences.preferred_discord_ids.clone();
        users.push(scope.discord_id);
        let mut tx = self.pool.begin().await?;
        lock_users(&mut tx, &users).await?;
        for target in &preferences.preferred_discord_ids {
            ensure_published_profile(
                &mut tx,
                Scope {
                    discord_id: *target,
                    ..scope
                },
            )
            .await?;
        }
        sqlx::query("INSERT INTO core.users (discord_id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(scope.discord_id)
            .execute(&mut *tx)
            .await?;
        let profile = sqlx::query_as::<_, Profile>(
            "INSERT INTO pool.profiles (guild_id, discord_id, modes, group_size_min, group_size_max,
             play_style, voice_preference, languages, timezone, published, interview_completed_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,CASE WHEN $10 THEN now() END)
             ON CONFLICT (guild_id, discord_id) DO UPDATE SET modes = EXCLUDED.modes,
             group_size_min = EXCLUDED.group_size_min, group_size_max = EXCLUDED.group_size_max,
             play_style = EXCLUDED.play_style, voice_preference = EXCLUDED.voice_preference,
             languages = EXCLUDED.languages, timezone = EXCLUDED.timezone,
             published = EXCLUDED.published, interview_completed_at = EXCLUDED.interview_completed_at,
             updated_at = now() RETURNING *"
        ).bind(scope.guild_id).bind(scope.discord_id).bind(&preferences.modes)
            .bind(preferences.group_size_min).bind(preferences.group_size_max)
            .bind(preferences.play_style).bind(preferences.voice_preference).bind(&preferences.languages)
            .bind(&preferences.timezone).bind(completed).fetch_one(&mut *tx).await?;
        sqlx::query("DELETE FROM pool.availability WHERE guild_id = $1 AND discord_id = $2")
            .bind(scope.guild_id)
            .bind(scope.discord_id)
            .execute(&mut *tx)
            .await?;
        for a in &preferences.availability {
            sqlx::query(
                "INSERT INTO pool.availability VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
            )
            .bind(scope.guild_id)
            .bind(scope.discord_id)
            .bind(a.weekday)
            .bind(a.start_minute)
            .bind(a.end_minute)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("DELETE FROM pool.preferred_players WHERE guild_id = $1 AND discord_id = $2")
            .bind(scope.guild_id)
            .bind(scope.discord_id)
            .execute(&mut *tx)
            .await?;
        for target in &preferences.preferred_discord_ids {
            sqlx::query(
                "INSERT INTO pool.preferred_players VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
            )
            .bind(scope.guild_id)
            .bind(scope.discord_id)
            .bind(target)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(profile)
    }

    pub async fn own_profile(&self, scope: Scope) -> PoolResult<Option<Profile>> {
        validate_scope(scope)?;
        Ok(
            sqlx::query_as("SELECT * FROM pool.profiles WHERE guild_id = $1 AND discord_id = $2")
                .bind(scope.guild_id)
                .bind(scope.discord_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn availability(&self, scope: Scope) -> PoolResult<Vec<Availability>> {
        validate_scope(scope)?;
        Ok(sqlx::query_as(
            "SELECT weekday, start_minute, end_minute FROM pool.availability
             WHERE guild_id = $1 AND discord_id = $2 ORDER BY weekday, start_minute, end_minute",
        )
        .bind(scope.guild_id)
        .bind(scope.discord_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn preferred_players(&self, scope: Scope) -> PoolResult<Vec<i64>> {
        validate_scope(scope)?;
        Ok(sqlx::query_scalar(
            "SELECT target_discord_id FROM pool.preferred_players
             WHERE guild_id = $1 AND discord_id = $2 ORDER BY target_discord_id",
        )
        .bind(scope.guild_id)
        .bind(scope.discord_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn set_dm_opt_in(&self, scope: Scope, enabled: bool) -> PoolResult<()> {
        validate_scope(scope)?;
        let mut tx = self.pool.begin().await?;
        lock_users(&mut tx, &[scope.discord_id]).await?;
        let rows = sqlx::query(
            "UPDATE pool.profiles SET dm_opt_in = $3, updated_at = now()
             WHERE guild_id = $1 AND discord_id = $2",
        )
        .bind(scope.guild_id)
        .bind(scope.discord_id)
        .bind(enabled)
        .execute(&mut *tx)
        .await?;
        if rows.rows_affected() != 1 {
            return Err(PoolError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_profiles(
        &self,
        guild_id: i64,
        filter: &PoolFilter,
    ) -> PoolResult<Vec<PublicProfile>> {
        if guild_id <= 0
            || filter.offset < 0
            || filter.limit < 0
            || filter.weekday.is_some_and(|d| !(0..=6).contains(&d))
            || filter.minute.is_some_and(|m| !(0..1440).contains(&m))
            || filter.min_rank.is_some_and(|r| r < 0)
            || filter.max_rank.is_some_and(|r| r < 0)
            || matches!((filter.min_rank, filter.max_rank), (Some(a), Some(b)) if a > b)
            || filter.weekday.is_some() != filter.minute.is_some()
            || filter.weekday.is_some() != filter.timezone.is_some()
            || filter
                .timezone
                .as_ref()
                .is_some_and(|tz| tz.parse::<chrono_tz::Tz>().is_err())
        {
            return Err(PoolError::Invalid("Pool-Filter ist ungültig"));
        }
        let sql = format!(
            "{PUBLIC_PROJECTION}
             AND ($2::text IS NULL OR $2 = ANY(p.modes))
             AND ($3::integer IS NULL OR a.rank_tier >= $3)
             AND ($4::integer IS NULL OR a.rank_tier <= $4)
             AND ($5::smallint IS NULL OR (p.timezone = $7 AND EXISTS (
                 SELECT 1 FROM pool.availability v WHERE v.guild_id = p.guild_id
                 AND v.discord_id = p.discord_id AND v.weekday = $5
                 AND v.start_minute <= $6 AND v.end_minute > $6)))
             ORDER BY p.discord_id LIMIT $8 OFFSET $9"
        );
        Ok(sqlx::query_as(&sql)
            .bind(guild_id)
            .bind(filter.mode)
            .bind(filter.min_rank)
            .bind(filter.max_rank)
            .bind(filter.weekday)
            .bind(filter.minute)
            .bind(&filter.timezone)
            .bind(if filter.limit == 0 {
                50
            } else {
                filter.limit.min(100)
            })
            .bind(filter.offset)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn public_profile(&self, scope: Scope) -> PoolResult<Option<PublicProfile>> {
        validate_scope(scope)?;
        Ok(
            sqlx::query_as(&format!("{PUBLIC_PROJECTION} AND p.discord_id = $2"))
                .bind(scope.guild_id)
                .bind(scope.discord_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn link_verified_steam(&self, scope: Scope, steam_id: &str) -> PoolResult<()> {
        validate_scope(scope)?;
        let id = steam_id
            .parse::<i64>()
            .map_err(|_| PoolError::Invalid("SteamID64 ist ungültig"))?;
        if !(76561197960265728..=76561202255233023).contains(&id) || id.to_string() != steam_id {
            return Err(PoolError::Invalid("SteamID64 ist ungültig"));
        }
        let mut tx = self.pool.begin().await?;
        lock_users(&mut tx, &[scope.discord_id]).await?;
        ensure_profile(&mut tx, scope).await?;
        sqlx::query(
            "INSERT INTO core.steam_links
             (discord_id, steam_id, steam_id64, verified, linked_at, updated_at, primary_account)
             VALUES ($1,$2,$3,true,now(),now(),NOT EXISTS (
                 SELECT 1 FROM core.steam_links WHERE discord_id = $1 AND primary_account))
             ON CONFLICT (discord_id, steam_id) DO UPDATE SET verified = true,
             steam_id64 = EXCLUDED.steam_id64, updated_at = now()",
        )
        .bind(scope.discord_id)
        .bind(steam_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn steam_links_for_ingest(&self, guild_id: i64) -> PoolResult<Vec<SteamLink>> {
        if guild_id <= 0 {
            return Err(PoolError::Invalid("Guild-ID muss positiv sein"));
        }
        Ok(sqlx::query_as(
            "SELECT DISTINCT ON (p.discord_id) p.discord_id, s.steam_id, s.primary_account
             FROM pool.profiles p JOIN core.steam_links s USING (discord_id)
             WHERE p.guild_id = $1 AND p.published AND s.verified AND s.steam_id64 IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM core.user_privacy u WHERE u.user_id = p.discord_id
                 AND (u.opted_out OR u.deleted_at IS NOT NULL))
             ORDER BY p.discord_id, s.primary_account DESC, s.steam_id",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn store_api_snapshot(
        &self,
        scope: Scope,
        snapshot: &ApiSnapshot,
        matches: &[Match],
    ) -> PoolResult<()> {
        validate_scope(scope)?;
        if snapshot.heatmap.len() != 168
            || snapshot.heatmap.iter().any(|n| *n < 0)
            || snapshot.observed_games < 0
            || snapshot.observed_play_seconds < 0
            || snapshot.window_start > snapshot.window_end
            || snapshot.window_end > snapshot.fetched_at
            || snapshot.rank_tier.is_some_and(|n| n < 0)
            || snapshot.rank_subtier.is_some_and(|n| n < 0)
            || snapshot.games_played.is_some_and(|n| n < 0)
            || snapshot.total_play_seconds.is_some_and(|n| n < 0)
            || matches.len() > 10000
            || matches.iter().any(|m| {
                m.match_id <= 0
                    || m.duration_seconds.is_some_and(|n| n < 0)
                    || m.started_at < snapshot.window_start
                    || m.started_at > snapshot.window_end
            })
        {
            return Err(PoolError::Invalid(
                "API-Snapshot oder Matchhistorie ist ungültig",
            ));
        }
        let mut tx = self.pool.begin().await?;
        lock_users(&mut tx, &[scope.discord_id]).await?;
        ensure_profile(&mut tx, scope).await?;
        let link = selected_steam_id(&mut tx, scope.discord_id).await?;
        if link.as_deref() != Some(&snapshot.steam_id) {
            return Err(PoolError::UnverifiedSteamLink);
        }
        let previous: Option<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT steam_id, fetched_at FROM pool.api_snapshots WHERE guild_id = $1 AND discord_id = $2")
            .bind(scope.guild_id).bind(scope.discord_id).fetch_optional(&mut *tx).await?;
        if previous
            .as_ref()
            .is_some_and(|(_, fetched)| *fetched > snapshot.fetched_at)
        {
            return Err(PoolError::StaleSnapshot);
        }
        if previous
            .as_ref()
            .is_some_and(|(id, _)| id != &snapshot.steam_id)
        {
            sqlx::query("DELETE FROM pool.co_players WHERE guild_id = $1 AND (discord_id = $2 OR target_discord_id = $2)")
                .bind(scope.guild_id).bind(scope.discord_id).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO pool.api_snapshots (guild_id,discord_id,steam_id,rank_tier,rank_subtier,
             games_played,total_play_seconds,observed_games,observed_play_seconds,window_start,window_end,fetched_at,heatmap)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
             ON CONFLICT (guild_id,discord_id) DO UPDATE SET steam_id=EXCLUDED.steam_id,
             rank_tier=EXCLUDED.rank_tier,rank_subtier=EXCLUDED.rank_subtier,games_played=EXCLUDED.games_played,
             total_play_seconds=EXCLUDED.total_play_seconds,observed_games=EXCLUDED.observed_games,
             observed_play_seconds=EXCLUDED.observed_play_seconds,window_start=EXCLUDED.window_start,
             window_end=EXCLUDED.window_end,fetched_at=EXCLUDED.fetched_at,heatmap=EXCLUDED.heatmap")
            .bind(scope.guild_id).bind(scope.discord_id).bind(&snapshot.steam_id).bind(snapshot.rank_tier)
            .bind(snapshot.rank_subtier).bind(snapshot.games_played).bind(snapshot.total_play_seconds)
            .bind(snapshot.observed_games).bind(snapshot.observed_play_seconds).bind(snapshot.window_start)
            .bind(snapshot.window_end).bind(snapshot.fetched_at).bind(&snapshot.heatmap).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM pool.matches WHERE guild_id = $1 AND discord_id = $2")
            .bind(scope.guild_id)
            .bind(scope.discord_id)
            .execute(&mut *tx)
            .await?;
        for m in matches {
            sqlx::query("INSERT INTO pool.matches (guild_id,discord_id,steam_id,match_id,started_at,duration_seconds,mode)
                 VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
                .bind(scope.guild_id).bind(scope.discord_id).bind(&snapshot.steam_id).bind(m.match_id)
                .bind(m.started_at).bind(m.duration_seconds).bind(m.mode).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn store_co_player(&self, guild_id: i64, pair: &CoPlayer) -> PoolResult<()> {
        validate_scope(Scope {
            guild_id,
            discord_id: pair.discord_id,
        })?;
        if pair.target_discord_id <= pair.discord_id
            || pair.games_together <= 0
            || pair.window_start > pair.last_played_at
            || pair.last_played_at > pair.window_end
            || pair.window_end > pair.fetched_at
        {
            return Err(PoolError::Invalid("Mitspieler-Paar ist ungültig"));
        }
        let mut tx = self.pool.begin().await?;
        lock_users(&mut tx, &[pair.discord_id, pair.target_discord_id]).await?;
        for user in [pair.discord_id, pair.target_discord_id] {
            ensure_published_profile(
                &mut tx,
                Scope {
                    guild_id,
                    discord_id: user,
                },
            )
            .await?;
            if selected_steam_id(&mut tx, user).await?.is_none() {
                return Err(PoolError::UnverifiedSteamLink);
            }
        }
        let rows = sqlx::query("INSERT INTO pool.co_players
             (guild_id,discord_id,target_discord_id,games_together,last_played_at,window_start,window_end,fetched_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (guild_id,discord_id,target_discord_id)
             DO UPDATE SET games_together=EXCLUDED.games_together,last_played_at=EXCLUDED.last_played_at,
             window_start=EXCLUDED.window_start,window_end=EXCLUDED.window_end,fetched_at=EXCLUDED.fetched_at
             WHERE pool.co_players.fetched_at <= EXCLUDED.fetched_at")
            .bind(guild_id).bind(pair.discord_id).bind(pair.target_discord_id).bind(pair.games_together)
            .bind(pair.last_played_at).bind(pair.window_start).bind(pair.window_end).bind(pair.fetched_at)
            .execute(&mut *tx).await?;
        if rows.rows_affected() != 1 {
            return Err(PoolError::StaleSnapshot);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn recently_played_with(
        &self,
        scope: Scope,
        limit: i64,
    ) -> PoolResult<Vec<CoPlayer>> {
        validate_scope(scope)?;
        Ok(sqlx::query_as("SELECT c.discord_id,c.target_discord_id,c.games_together,c.last_played_at,
             c.window_start,c.window_end,c.fetched_at FROM pool.co_players c
             JOIN pool.profiles a ON a.guild_id=c.guild_id AND a.discord_id=c.discord_id
             JOIN pool.profiles b ON b.guild_id=c.guild_id AND b.discord_id=c.target_discord_id
             WHERE c.guild_id=$1 AND (c.discord_id=$2 OR c.target_discord_id=$2) AND a.published AND b.published
             AND NOT EXISTS (SELECT 1 FROM core.user_privacy u WHERE u.user_id IN (c.discord_id,c.target_discord_id)
                 AND (u.opted_out OR u.deleted_at IS NOT NULL))
             ORDER BY c.last_played_at DESC,c.discord_id,c.target_discord_id LIMIT $3")
            .bind(scope.guild_id).bind(scope.discord_id).bind(limit.clamp(1,100)).fetch_all(&self.pool).await?)
    }

    pub async fn create_session(
        &self,
        initiator: Scope,
        targets: &[i64],
        mode: Mode,
    ) -> PoolResult<Session> {
        validate_scope(initiator)?;
        let mut users = targets.to_vec();
        users.push(initiator.discord_id);
        if !(2..=6).contains(&users.len())
            || users.iter().collect::<BTreeSet<_>>().len() != users.len()
        {
            return Err(PoolError::Invalid(
                "Session braucht 2 bis 6 verschiedene Teilnehmer",
            ));
        }
        let mut tx = self.pool.begin().await?;
        lock_users(&mut tx, &users).await?;
        for user in &users {
            ensure_published_profile(
                &mut tx,
                Scope {
                    discord_id: *user,
                    ..initiator
                },
            )
            .await?;
        }
        let session = sqlx::query_as::<_, Session>(
            "INSERT INTO pool.sessions (guild_id,initiator_discord_id,mode)
             VALUES ($1,$2,$3) RETURNING *",
        )
        .bind(initiator.guild_id)
        .bind(initiator.discord_id)
        .bind(mode)
        .fetch_one(&mut *tx)
        .await?;
        for user in users {
            sqlx::query("INSERT INTO pool.session_participants (guild_id,session_id,discord_id) VALUES ($1,$2,$3)")
                .bind(initiator.guild_id).bind(session.session_id).bind(user).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(session)
    }

    pub async fn participant_session(
        &self,
        scope: Scope,
        session_id: i64,
    ) -> PoolResult<Option<Session>> {
        validate_scope(scope)?;
        Ok(sqlx::query_as("SELECT s.* FROM pool.sessions s JOIN pool.session_participants p
             USING (guild_id,session_id) WHERE s.guild_id=$1 AND s.session_id=$2 AND p.discord_id=$3")
            .bind(scope.guild_id).bind(session_id).bind(scope.discord_id).fetch_optional(&self.pool).await?)
    }

    pub async fn own_sessions(
        &self,
        scope: Scope,
        limit: i64,
        offset: i64,
    ) -> PoolResult<Vec<OwnSession>> {
        validate_scope(scope)?;
        if limit < 0 || offset < 0 {
            return Err(PoolError::Invalid(
                "Seitengröße und Offset dürfen nicht negativ sein",
            ));
        }
        #[derive(sqlx::FromRow)]
        struct OwnSessionRow {
            #[sqlx(flatten)]
            session: Session,
            joined_at: DateTime<Utc>,
            has_feedback: bool,
            #[sqlx(flatten)]
            feedback: Feedback,
        }
        let rows: Vec<OwnSessionRow> = sqlx::query_as(
            "SELECT s.*,p.joined_at,f.discord_id IS NOT NULL AS has_feedback,
             COALESCE(f.play_again,false) AS play_again,COALESCE(f.friendly,false) AS friendly,
             COALESCE(f.good_communication,false) AS good_communication,
             COALESCE(f.balanced_match,false) AS balanced_match
             FROM pool.sessions s JOIN pool.session_participants p USING(guild_id,session_id)
             LEFT JOIN pool.feedback f ON f.guild_id=p.guild_id AND f.session_id=p.session_id AND f.discord_id=p.discord_id
             WHERE s.guild_id=$1 AND p.discord_id=$2 AND s.status='ended' AND p.joined_at IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM core.user_privacy u WHERE u.user_id=$2 AND (u.opted_out OR u.deleted_at IS NOT NULL))
             ORDER BY s.ended_at DESC,s.session_id DESC LIMIT $3 OFFSET $4",
        )
        .bind(scope.guild_id).bind(scope.discord_id)
        .bind(if limit == 0 { 50 } else { limit.min(100) }).bind(offset)
        .fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| OwnSession {
                session: row.session,
                joined_at: row.joined_at,
                feedback: row.has_feedback.then_some(row.feedback),
            })
            .collect())
    }

    pub async fn participants(
        &self,
        scope: Scope,
        session_id: i64,
    ) -> PoolResult<Vec<Participant>> {
        validate_scope(scope)?;
        Ok(sqlx::query_as("SELECT p.discord_id,p.joined_at FROM pool.session_participants p
             WHERE p.guild_id=$1 AND p.session_id=$2 AND EXISTS (
                 SELECT 1 FROM pool.session_participants a WHERE a.guild_id=$1 AND a.session_id=$2 AND a.discord_id=$3)
             ORDER BY p.discord_id")
            .bind(scope.guild_id).bind(session_id).bind(scope.discord_id).fetch_all(&self.pool).await?)
    }

    pub async fn due_sessions(
        &self,
        guild_id: i64,
        now: DateTime<Utc>,
    ) -> PoolResult<Vec<Session>> {
        if guild_id <= 0 {
            return Err(PoolError::Invalid("Guild-ID muss positiv sein"));
        }
        Ok(sqlx::query_as("SELECT * FROM pool.sessions WHERE guild_id=$1
             AND status IN ('pending','open','active') AND expires_at <= $2 ORDER BY expires_at,session_id")
            .bind(guild_id).bind(now).fetch_all(&self.pool).await?)
    }

    pub async fn attach_channel(
        &self,
        guild_id: i64,
        session_id: i64,
        channel_id: i64,
    ) -> PoolResult<Session> {
        if guild_id <= 0 || channel_id <= 0 {
            return Err(PoolError::Invalid(
                "Guild-ID und Kanal-ID müssen positiv sein",
            ));
        }
        let mut tx = self.pool.begin().await?;
        lock_session_users(&mut tx, guild_id, session_id).await?;
        let session = sqlx::query_as("UPDATE pool.sessions SET channel_id=$3,status='open'
             WHERE guild_id=$1 AND session_id=$2 AND status='pending' AND expires_at > now() RETURNING *")
            .bind(guild_id).bind(session_id).bind(channel_id).fetch_optional(&mut *tx).await?
            .ok_or(PoolError::InvalidSessionState)?;
        tx.commit().await?;
        Ok(session)
    }

    pub async fn record_join(
        &self,
        scope: Scope,
        session_id: i64,
        at: DateTime<Utc>,
    ) -> PoolResult<()> {
        validate_scope(scope)?;
        let mut tx = self.pool.begin().await?;
        lock_session_users(&mut tx, scope.guild_id, session_id).await?;
        let session: Session = sqlx::query_as(
            "SELECT * FROM pool.sessions
             WHERE guild_id=$1 AND session_id=$2 FOR UPDATE",
        )
        .bind(scope.guild_id)
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?;
        if !matches!(session.status, SessionStatus::Open | SessionStatus::Active)
            || at < session.created_at
            || at >= session.expires_at
        {
            return Err(PoolError::InvalidSessionState);
        }
        let rows = sqlx::query(
            "UPDATE pool.session_participants SET joined_at=COALESCE(joined_at,$4)
             WHERE guild_id=$1 AND session_id=$2 AND discord_id=$3",
        )
        .bind(scope.guild_id)
        .bind(session_id)
        .bind(scope.discord_id)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        if rows.rows_affected() != 1 {
            return Err(PoolError::NotFound);
        }
        sqlx::query(
            "UPDATE pool.sessions SET status='active',started_at=COALESCE(started_at,$3)
             WHERE guild_id=$1 AND session_id=$2",
        )
        .bind(scope.guild_id)
        .bind(session_id)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn finish_session(
        &self,
        guild_id: i64,
        session_id: i64,
        status: SessionStatus,
        at: DateTime<Utc>,
    ) -> PoolResult<Session> {
        if guild_id <= 0
            || !matches!(
                status,
                SessionStatus::Ended | SessionStatus::Expired | SessionStatus::Failed
            )
        {
            return Err(PoolError::InvalidSessionState);
        }
        let mut tx = self.pool.begin().await?;
        lock_session_users(&mut tx, guild_id, session_id).await?;
        let session = sqlx::query_as(
            "UPDATE pool.sessions SET status=$3,ended_at=$4
             WHERE guild_id=$1 AND session_id=$2 AND status IN ('pending','open','active')
             AND $4 >= COALESCE(started_at,created_at)
             AND (($3='failed') OR ($3='ended' AND status='active') OR
                 ($3='expired' AND status IN ('pending','open') AND expires_at <= $4))
             RETURNING *",
        )
        .bind(guild_id)
        .bind(session_id)
        .bind(status)
        .bind(at)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PoolError::InvalidSessionState)?;
        tx.commit().await?;
        Ok(session)
    }

    pub async fn save_feedback(
        &self,
        scope: Scope,
        session_id: i64,
        feedback: Feedback,
    ) -> PoolResult<()> {
        validate_scope(scope)?;
        let mut tx = self.pool.begin().await?;
        lock_session_users(&mut tx, scope.guild_id, session_id).await?;
        let eligible: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pool.sessions s
             JOIN pool.session_participants p USING(guild_id,session_id)
             WHERE s.guild_id=$1 AND s.session_id=$2 AND p.discord_id=$3 AND s.status='ended' AND p.joined_at IS NOT NULL)")
            .bind(scope.guild_id).bind(session_id).bind(scope.discord_id).fetch_one(&mut *tx).await?;
        if !eligible {
            return Err(PoolError::NotFound);
        }
        sqlx::query("INSERT INTO pool.feedback (guild_id,session_id,discord_id,play_again,friendly,good_communication,balanced_match)
             VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (guild_id,session_id,discord_id)
             DO UPDATE SET play_again=EXCLUDED.play_again,friendly=EXCLUDED.friendly,
             good_communication=EXCLUDED.good_communication,balanced_match=EXCLUDED.balanced_match,updated_at=now()")
            .bind(scope.guild_id).bind(session_id).bind(scope.discord_id).bind(feedback.play_again)
            .bind(feedback.friendly).bind(feedback.good_communication).bind(feedback.balanced_match)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn own_feedback(
        &self,
        scope: Scope,
        session_id: i64,
    ) -> PoolResult<Option<Feedback>> {
        validate_scope(scope)?;
        Ok(sqlx::query_as(
            "SELECT play_again,friendly,good_communication,balanced_match FROM pool.feedback
             WHERE guild_id=$1 AND session_id=$2 AND discord_id=$3",
        )
        .bind(scope.guild_id)
        .bind(session_id)
        .bind(scope.discord_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_profile(&self, scope: Scope) -> PoolResult<u64> {
        validate_scope(scope)?;
        let mut tx = self.pool.begin().await?;
        dl_central_db::lock_user_privacy(&mut tx, scope.discord_id).await?;
        let rows = sqlx::query("DELETE FROM pool.profiles WHERE guild_id=$1 AND discord_id=$2")
            .bind(scope.guild_id)
            .bind(scope.discord_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(rows.rows_affected())
    }
}

async fn selected_steam_id(
    tx: &mut Transaction<'_, Postgres>,
    discord_id: i64,
) -> PoolResult<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT steam_id FROM core.steam_links
         WHERE discord_id=$1 AND verified AND steam_id64 IS NOT NULL
         ORDER BY primary_account DESC,steam_id LIMIT 1 FOR SHARE",
    )
    .bind(discord_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn ensure_profile(tx: &mut Transaction<'_, Postgres>, scope: Scope) -> PoolResult<()> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pool.profiles WHERE guild_id=$1 AND discord_id=$2)",
    )
    .bind(scope.guild_id)
    .bind(scope.discord_id)
    .fetch_one(&mut **tx)
    .await?;
    if !exists {
        return Err(PoolError::NotFound);
    }
    Ok(())
}

async fn ensure_published_profile(
    tx: &mut Transaction<'_, Postgres>,
    scope: Scope,
) -> PoolResult<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pool.profiles WHERE guild_id=$1 AND discord_id=$2 AND published)")
        .bind(scope.guild_id).bind(scope.discord_id).fetch_one(&mut **tx).await?;
    if !exists {
        return Err(PoolError::NotFound);
    }
    Ok(())
}

async fn lock_session_users(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: i64,
    session_id: i64,
) -> PoolResult<()> {
    let users: Vec<i64> = sqlx::query_scalar(
        "SELECT discord_id FROM pool.session_participants
         WHERE guild_id=$1 AND session_id=$2 ORDER BY discord_id",
    )
    .bind(guild_id)
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    if users.is_empty() {
        return Err(PoolError::NotFound);
    }
    lock_users(tx, &users).await?;
    Ok(())
}

pub async fn delete_all_for_user_tx(
    tx: &mut Transaction<'_, Postgres>,
    discord_id: i64,
) -> Result<u64, sqlx::Error> {
    dl_central_db::lock_user_privacy(tx, discord_id).await?;
    Ok(sqlx::query("DELETE FROM pool.profiles WHERE discord_id=$1")
        .bind(discord_id)
        .execute(&mut **tx)
        .await?
        .rows_affected())
}
