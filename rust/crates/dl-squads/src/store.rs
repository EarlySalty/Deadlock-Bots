use crate::model::{Participant, SeedPlayer};
use chrono::Utc;
use dl_db::Db;
use rusqlite::{params, OptionalExtension, Row};

#[derive(Debug, thiserror::Error)]
pub enum SquadErr {
    #[error(transparent)]
    Db(#[from] dl_db::DbError),
    #[error("Seed-Daten konnten nicht serialisiert werden: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Seed-Spieler fehlt: {0}")]
    MissingSeedParticipant(String),
    #[error("Seed-Team fehlt: {0}")]
    MissingSeedTeam(String),
    #[error("Participant-Name ist nach Trim leer: {0:?}")]
    InvalidParticipantName(String),
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

pub async fn upsert_participant_by_discord(
    db: &Db,
    discord_id: i64,
    display_name: &str,
) -> Result<i64, SquadErr> {
    let display_name = normalize_participant_name(display_name)?;
    let now = now_iso();
    let participant_id = db
        .write(move |conn| {
            conn.execute(
                "INSERT INTO scrim_participant(
                    discord_id, display_name, rank_source, rank_verified, status,
                    source, created_at, updated_at
                 ) VALUES(?1, ?2, ?3, 0, ?4, ?5, ?6, ?6)
                 ON CONFLICT(discord_id) WHERE discord_id IS NOT NULL DO UPDATE SET
                    display_name = excluded.display_name,
                    updated_at = excluded.updated_at
                 ON CONFLICT(display_name) DO UPDATE SET
                    discord_id = COALESCE(scrim_participant.discord_id, excluded.discord_id),
                    updated_at = excluded.updated_at",
                params![
                    discord_id,
                    display_name,
                    RANK_SOURCE_SELF,
                    STATUS_NEW,
                    SOURCE_DISCORD_REACTION,
                    now,
                ],
            )?;
            conn.query_row(
                "SELECT id
                   FROM scrim_participant
                  WHERE discord_id = ?1 OR display_name = ?2
                  ORDER BY CASE WHEN discord_id = ?1 THEN 0 ELSE 1 END, id ASC
                  LIMIT 1",
                params![discord_id, display_name],
                |row| row.get(0),
            )
        })
        .await?;
    Ok(participant_id)
}

pub async fn upsert_participant_by_name(db: &Db, player: &SeedPlayer) -> Result<i64, SquadErr> {
    let name = normalize_participant_name(&player.name)?;
    let rank = player.rank.clone();
    let roles = player.roles.clone();
    let availability = availability_json(player)?;
    let now = now_iso();
    let participant_id = db
        .write(move |conn| {
            conn.execute(
                "INSERT INTO scrim_participant(
                    display_name, rank, rank_source, rank_verified, roles, availability,
                    status, source, created_at, updated_at
                 ) VALUES(?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?8)
                 ON CONFLICT(display_name) DO NOTHING",
                params![
                    name,
                    rank,
                    RANK_SOURCE_SELF,
                    roles,
                    availability,
                    STATUS_NEW,
                    SOURCE_SEED,
                    now,
                ],
            )?;
            conn.query_row(
                "SELECT id FROM scrim_participant WHERE display_name = ?1",
                params![name],
                |row| row.get(0),
            )
        })
        .await?;
    Ok(participant_id)
}

pub async fn create_team(
    db: &Db,
    name: &str,
    coach: Option<&str>,
    role_id: Option<i64>,
    channel_id: Option<i64>,
) -> Result<i64, SquadErr> {
    let name = name.to_string();
    let coach = coach.map(str::to_string);
    let now = now_iso();
    let team_id = db
        .write(move |conn| {
            conn.execute(
                "INSERT INTO scrim_team(name, coach, discord_role_id, discord_channel_id, created_at)
                 VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(name) DO UPDATE SET
                    coach = excluded.coach,
                    discord_role_id = excluded.discord_role_id,
                    discord_channel_id = excluded.discord_channel_id",
                params![name, coach, role_id, channel_id, now],
            )?;
            conn.query_row(
                "SELECT id FROM scrim_team WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
        })
        .await?;
    Ok(team_id)
}

pub async fn add_team_member(
    db: &Db,
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
    let now = now_iso();
    db.write(move |conn| {
        conn.execute(
            "INSERT INTO scrim_team_member(
                team_id, participant_id, role, is_captain, is_bench
             ) VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(team_id, participant_id) DO UPDATE SET
                role = excluded.role,
                is_captain = excluded.is_captain,
                is_bench = excluded.is_bench",
            params![
                team_id,
                participant_id,
                role,
                is_captain as i64,
                is_bench as i64
            ],
        )?;
        conn.execute(
            "UPDATE scrim_participant
                SET status = ?1, updated_at = ?2
              WHERE id = ?3",
            params![status, now, participant_id],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

pub async fn list_pool(db: &Db, status: Option<&str>) -> Result<Vec<Participant>, SquadErr> {
    let status = status.map(str::to_string);
    let participants = db
        .read(move |conn| {
            if let Some(status) = status {
                let mut stmt = conn.prepare(
                    "SELECT id, discord_id, display_name, rank, rank_source, rank_verified,
                            roles, availability, status, source, created_at, updated_at
                       FROM scrim_participant
                      WHERE status = ?1
                      ORDER BY id ASC",
                )?;
                let rows = stmt.query_map(params![status], row_to_participant)?;
                rows.collect::<Result<Vec<_>, _>>()
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, discord_id, display_name, rank, rank_source, rank_verified,
                            roles, availability, status, source, created_at, updated_at
                       FROM scrim_participant
                      ORDER BY id ASC",
                )?;
                let rows = stmt.query_map([], row_to_participant)?;
                rows.collect::<Result<Vec<_>, _>>()
            }
        })
        .await?;
    Ok(participants)
}

pub(crate) async fn create_match(
    db: &Db,
    team_a_id: i64,
    team_b_id: i64,
    when_text: Option<&str>,
    scheduled_at: Option<&str>,
) -> Result<i64, SquadErr> {
    let when_text = when_text.map(str::to_string);
    let scheduled_at = scheduled_at.map(str::to_string);
    let now = now_iso();
    let match_id = db
        .write(move |conn| {
            let existing = conn
                .query_row(
                    "SELECT id
                       FROM scrim_match
                      WHERE team_a_id = ?1
                        AND team_b_id = ?2
                        AND COALESCE(when_text, '') = COALESCE(?3, '')
                      LIMIT 1",
                    params![team_a_id, team_b_id, when_text],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok(id);
            }
            conn.execute(
                "INSERT INTO scrim_match(
                    team_a_id, team_b_id, when_text, scheduled_at, status, created_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    team_a_id,
                    team_b_id,
                    when_text,
                    scheduled_at,
                    MATCH_STATUS_PLANNED,
                    now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .await?;
    Ok(match_id)
}

pub(crate) async fn set_participant_status(
    db: &Db,
    participant_id: i64,
    status: &str,
) -> Result<(), SquadErr> {
    let status = status.to_string();
    let now = now_iso();
    db.write(move |conn| {
        conn.execute(
            "UPDATE scrim_participant SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now, participant_id],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

fn row_to_participant(row: &Row<'_>) -> rusqlite::Result<Participant> {
    Ok(Participant {
        id: row.get(0)?,
        discord_id: row.get(1)?,
        display_name: row.get(2)?,
        rank: row.get(3)?,
        rank_source: row.get(4)?,
        rank_verified: row.get::<_, i64>(5)? != 0,
        roles: row.get(6)?,
        availability: row.get(7)?,
        status: row.get(8)?,
        source: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn normalize_participant_name(name: &str) -> Result<String, SquadErr> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err(SquadErr::InvalidParticipantName(name.to_string()))
    } else {
        Ok(trimmed.to_string())
    }
}

fn availability_json(player: &SeedPlayer) -> Result<Option<String>, serde_json::Error> {
    if player.availability.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string(&player.availability).map(Some)
    }
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}
