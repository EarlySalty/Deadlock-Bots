use crate::model::{Participant, SeedPlayer};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::num::TryFromIntError;

#[derive(Debug, thiserror::Error)]
pub enum SquadErr {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("Seed-Daten konnten nicht serialisiert werden: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Seed-Spieler fehlt: {0}")]
    MissingSeedParticipant(String),
    #[error("Seed-Team fehlt: {0}")]
    MissingSeedTeam(String),
    #[error("Team fehlt: id {0}")]
    MissingTeamId(i64),
    #[error("Participant fehlt: id {0}")]
    MissingParticipantId(i64),
    #[error("Participant-Name ist nach Trim leer: {0:?}")]
    InvalidParticipantName(String),
    #[error("Participant-display_name bereits vergeben: {display_name} (id {existing_id})")]
    ParticipantDisplayNameTaken {
        display_name: String,
        existing_id: i64,
    },
    #[error("Scrim-ID {label}={value} passt nicht in PostgreSQL int4")]
    IdOutOfRange {
        label: &'static str,
        value: i64,
        source: TryFromIntError,
    },
    #[error("scheduled_at ist kein RFC3339-Zeitstempel: {value:?}")]
    InvalidScheduledAt {
        value: String,
        source: chrono::ParseError,
    },
    #[error("Seed-Spielername kollidiert nach Trim: {trimmed} ({first:?} vs {second:?})")]
    DuplicateSeedParticipantName {
        trimmed: String,
        first: String,
        second: String,
    },
}

const STATUS_NEW: &str = "new";
const STATUS_ASSIGNED: &str = "assigned";
const STATUS_BENCH: &str = "bench";
const SOURCE_DISCORD_REACTION: &str = "discord_reaction";
const SOURCE_SEED: &str = "seed";
const RANK_SOURCE_SELF: &str = "self";
const MATCH_STATUS_PLANNED: &str = "planned";

const PARTICIPANTS_LOCK: i64 = 42_060_004_001;
const TEAMS_LOCK: i64 = 42_060_004_002;
const MATCHES_LOCK: i64 = 42_060_004_003;

pub async fn upsert_participant_by_discord(
    pool: &PgPool,
    discord_id: i64,
    display_name: &str,
) -> Result<i64, SquadErr> {
    let display_name = normalize_participant_name(display_name)?;
    let now = now_utc();
    let mut tx = pool.begin().await?;
    lock_scrim_key(&mut tx, PARTICIPANTS_LOCK).await?;

    if let Some(row) = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.participants
         WHERE discord_id = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
        discord_id
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        let id = row.id;
        ensure_participant_display_name_available(&mut tx, &display_name, id).await?;
        sqlx::query!(
            r#"
            UPDATE scrim.participants
               SET display_name = $2,
                   updated_at = $3
             WHERE id = $1
            "#,
            id,
            display_name,
            now
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(i64::from(id));
    }

    if let Some(row) = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.participants
         WHERE display_name = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
        display_name
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        let id = row.id;
        sqlx::query!(
            r#"
            UPDATE scrim.participants
               SET discord_id = COALESCE(discord_id, $2),
                   updated_at = $3
             WHERE id = $1
            "#,
            id,
            discord_id,
            now
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(i64::from(id));
    }

    let id = next_participant_id(&mut tx).await?;
    sqlx::query!(
        r#"
        INSERT INTO scrim.participants(
            id, discord_id, display_name, rank_source, rank_verified, status,
            source, created_at, updated_at
        )
        VALUES($1, $2, $3, $4, FALSE, $5, $6, $7, $7)
        "#,
        id,
        discord_id,
        display_name,
        RANK_SOURCE_SELF,
        STATUS_NEW,
        SOURCE_DISCORD_REACTION,
        now
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(i64::from(id))
}

async fn ensure_participant_display_name_available(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    display_name: &str,
    current_id: i32,
) -> Result<(), SquadErr> {
    if let Some(row) = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.participants
         WHERE display_name = $1
           AND id <> $2
         ORDER BY id ASC
         LIMIT 1
        "#,
        display_name,
        current_id
    )
    .fetch_optional(&mut **tx)
    .await?
    {
        return Err(SquadErr::ParticipantDisplayNameTaken {
            display_name: display_name.to_string(),
            existing_id: i64::from(row.id),
        });
    }
    Ok(())
}

pub async fn upsert_participant_by_name(
    pool: &PgPool,
    player: &SeedPlayer,
) -> Result<i64, SquadErr> {
    let name = normalize_participant_name(&player.name)?;
    let rank = player.rank.clone();
    let roles = player.roles.clone();
    let availability = availability_json(player)?;
    let now = now_utc();
    let mut tx = pool.begin().await?;
    lock_scrim_key(&mut tx, PARTICIPANTS_LOCK).await?;

    if let Some(row) = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.participants
         WHERE display_name = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
        name
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(i64::from(row.id));
    }

    let id = next_participant_id(&mut tx).await?;
    sqlx::query!(
        r#"
        INSERT INTO scrim.participants(
            id, display_name, rank, rank_source, rank_verified, roles, availability,
            status, source, created_at, updated_at
        )
        VALUES($1, $2, $3, $4, FALSE, $5, $6, $7, $8, $9, $9)
        "#,
        id,
        name,
        rank,
        RANK_SOURCE_SELF,
        roles,
        availability,
        STATUS_NEW,
        SOURCE_SEED,
        now
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(i64::from(id))
}

pub async fn create_team(
    pool: &PgPool,
    name: &str,
    coach: Option<&str>,
    role_id: Option<i64>,
    channel_id: Option<i64>,
) -> Result<i64, SquadErr> {
    let name = name.to_string();
    let coach = coach.map(str::to_string);
    let now = now_utc();
    let mut tx = pool.begin().await?;
    lock_scrim_key(&mut tx, TEAMS_LOCK).await?;

    if let Some(row) = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.teams
         WHERE name = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
        name
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        let id = row.id;
        sqlx::query!(
            r#"
            UPDATE scrim.teams
               SET coach = $2,
                   discord_role_id = $3,
                   discord_channel_id = $4
             WHERE id = $1
            "#,
            id,
            coach,
            role_id,
            channel_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(i64::from(id));
    }

    let id = next_team_id(&mut tx).await?;
    sqlx::query!(
        r#"
        INSERT INTO scrim.teams(
            id, name, coach, discord_role_id, discord_channel_id, created_at
        )
        VALUES($1, $2, $3, $4, $5, $6)
        "#,
        id,
        name,
        coach,
        role_id,
        channel_id,
        now
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(i64::from(id))
}

pub async fn add_team_member(
    pool: &PgPool,
    team_id: i64,
    participant_id: i64,
    role: Option<&str>,
    is_captain: bool,
    is_bench: bool,
) -> Result<(), SquadErr> {
    let role = role.map(str::to_string);
    let status = if is_bench {
        STATUS_BENCH
    } else {
        STATUS_ASSIGNED
    }
    .to_string();
    let team_id_i32 = to_i32_id("team_id", team_id)?;
    let participant_id_i32 = to_i32_id("participant_id", participant_id)?;
    let now = now_utc();
    let mut tx = pool.begin().await?;

    ensure_team_exists(&mut tx, team_id_i32, team_id).await?;
    ensure_participant_exists(&mut tx, participant_id_i32, participant_id).await?;

    sqlx::query!(
        r#"
        INSERT INTO scrim.team_members(
            team_id, participant_id, role, is_captain, is_bench
        )
        VALUES($1, $2, $3, $4, $5)
        ON CONFLICT(team_id, participant_id) DO UPDATE SET
            role = excluded.role,
            is_captain = excluded.is_captain,
            is_bench = excluded.is_bench
        "#,
        team_id_i32,
        participant_id_i32,
        role,
        is_captain,
        is_bench
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        r#"
        UPDATE scrim.participants
           SET status = $1,
               updated_at = $2
         WHERE id = $3
        "#,
        status,
        now,
        participant_id_i32
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn list_pool(pool: &PgPool, status: Option<&str>) -> Result<Vec<Participant>, SquadErr> {
    let participants = if let Some(status) = status {
        sqlx::query_as!(
            Participant,
            r#"
            SELECT id::bigint AS "id!",
                   discord_id,
                   display_name,
                   rank,
                   rank_source,
                   rank_verified,
                   roles,
                   availability,
                   status,
                   source,
                   created_at,
                   updated_at
              FROM scrim.participants
             WHERE status = $1
             ORDER BY id ASC
            "#,
            status
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as!(
            Participant,
            r#"
            SELECT id::bigint AS "id!",
                   discord_id,
                   display_name,
                   rank,
                   rank_source,
                   rank_verified,
                   roles,
                   availability,
                   status,
                   source,
                   created_at,
                   updated_at
              FROM scrim.participants
             ORDER BY id ASC
            "#
        )
        .fetch_all(pool)
        .await?
    };
    Ok(participants)
}

pub(crate) async fn create_match(
    pool: &PgPool,
    team_a_id: i64,
    team_b_id: i64,
    when_text: Option<&str>,
    scheduled_at: Option<&str>,
) -> Result<i64, SquadErr> {
    let team_a_id_i32 = to_i32_id("team_a_id", team_a_id)?;
    let team_b_id_i32 = to_i32_id("team_b_id", team_b_id)?;
    let when_text = when_text.map(str::to_string);
    let scheduled_at = parse_scheduled_at(scheduled_at)?;
    let now = now_utc();
    let mut tx = pool.begin().await?;
    lock_scrim_key(&mut tx, MATCHES_LOCK).await?;

    ensure_team_exists(&mut tx, team_a_id_i32, team_a_id).await?;
    ensure_team_exists(&mut tx, team_b_id_i32, team_b_id).await?;

    if let Some(row) = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.matches
         WHERE team_a_id = $1
           AND team_b_id = $2
           AND COALESCE(when_text, '') = COALESCE($3::text, '')
         ORDER BY id ASC
         LIMIT 1
        "#,
        team_a_id_i32,
        team_b_id_i32,
        when_text
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(i64::from(row.id));
    }

    let id = next_match_id(&mut tx).await?;
    sqlx::query!(
        r#"
        INSERT INTO scrim.matches(
            id, team_a_id, team_b_id, when_text, scheduled_at, status, created_at
        )
        VALUES($1, $2, $3, $4, $5, $6, $7)
        "#,
        id,
        team_a_id_i32,
        team_b_id_i32,
        when_text,
        scheduled_at,
        MATCH_STATUS_PLANNED,
        now
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(i64::from(id))
}

pub(crate) async fn set_participant_status(
    pool: &PgPool,
    participant_id: i64,
    status: &str,
) -> Result<(), SquadErr> {
    let participant_id = to_i32_id("participant_id", participant_id)?;
    let now = now_utc();
    sqlx::query!(
        r#"
        UPDATE scrim.participants
           SET status = $1,
               updated_at = $2
         WHERE id = $3
        "#,
        status,
        now,
        participant_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn lock_scrim_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        SELECT 1 AS "locked!"
          FROM pg_advisory_xact_lock($1)
        "#,
        key
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

async fn next_participant_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<i32, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int4 AS "next_id!"
          FROM scrim.participants
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

async fn next_team_id(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<i32, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int4 AS "next_id!"
          FROM scrim.teams
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

async fn next_match_id(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<i32, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT (COALESCE(MAX(id), 0) + 1)::int4 AS "next_id!"
          FROM scrim.matches
        "#
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.next_id)
}

async fn ensure_team_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    team_id_i32: i32,
    team_id: i64,
) -> Result<(), SquadErr> {
    let row = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.teams
         WHERE id = $1
        "#,
        team_id_i32
    )
    .fetch_optional(&mut **tx)
    .await?;
    if row.is_none() {
        return Err(SquadErr::MissingTeamId(team_id));
    }
    Ok(())
}

async fn ensure_participant_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    participant_id_i32: i32,
    participant_id: i64,
) -> Result<(), SquadErr> {
    let row = sqlx::query!(
        r#"
        SELECT id
          FROM scrim.participants
         WHERE id = $1
        "#,
        participant_id_i32
    )
    .fetch_optional(&mut **tx)
    .await?;
    if row.is_none() {
        return Err(SquadErr::MissingParticipantId(participant_id));
    }
    Ok(())
}

fn normalize_participant_name(name: &str) -> Result<String, SquadErr> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err(SquadErr::InvalidParticipantName(name.to_string()))
    } else {
        Ok(trimmed.to_string())
    }
}

fn parse_scheduled_at(value: Option<&str>) -> Result<Option<DateTime<Utc>>, SquadErr> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|source| SquadErr::InvalidScheduledAt {
                    value: value.to_string(),
                    source,
                })
        })
        .transpose()
}

fn to_i32_id(label: &'static str, value: i64) -> Result<i32, SquadErr> {
    i32::try_from(value).map_err(|source| SquadErr::IdOutOfRange {
        label,
        value,
        source,
    })
}

fn availability_json(player: &SeedPlayer) -> Result<Option<String>, serde_json::Error> {
    if player.availability.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string(&player.availability).map(Some)
    }
}

fn now_utc() -> DateTime<Utc> {
    Utc::now()
}
