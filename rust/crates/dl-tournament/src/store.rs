//! Turnier-Store — Port von `cogs/customgames/tournament_store.py`
//! (Tabellen, Validierung und Status-Strings unverändert).

use dl_db::{Db, DbError};
use rusqlite::OptionalExtension;

pub const TEAM_NAME_MIN: usize = 2;
pub const TEAM_NAME_MAX: usize = 32;
pub const TEAM_MAX_SIZE: i64 = 6;

pub const RANK_KEYS: [&str; 11] = [
    "initiate",
    "seeker",
    "alchemist",
    "arcanist",
    "ritualist",
    "emissary",
    "archon",
    "oracle",
    "phantom",
    "ascendant",
    "eternus",
];

pub fn normalize_rank(raw: &str) -> &'static str {
    let normalized = raw.trim().to_lowercase();
    RANK_KEYS
        .iter()
        .find(|key| **key == normalized)
        .copied()
        .unwrap_or(RANK_KEYS[0])
}

/// Anzeigename eines Rang-Keys (wie tstore.rank_label: capitalize).
pub fn rank_label(rank_key: &str) -> &'static str {
    match normalize_rank(rank_key) {
        "initiate" => "Initiate",
        "seeker" => "Seeker",
        "alchemist" => "Alchemist",
        "arcanist" => "Arcanist",
        "ritualist" => "Ritualist",
        "emissary" => "Emissary",
        "archon" => "Archon",
        "oracle" => "Oracle",
        "phantom" => "Phantom",
        "ascendant" => "Ascendant",
        "eternus" => "Eternus",
        _ => "Obscurus",
    }
}

pub fn rank_value(rank_key: &str) -> i64 {
    let key = normalize_rank(rank_key);
    RANK_KEYS.iter().position(|k| *k == key).unwrap_or(0) as i64 + 1
}

pub fn normalize_mode(raw: &str) -> Result<&'static str, String> {
    match raw.trim().to_lowercase().as_str() {
        "solo" => Ok("solo"),
        "team" => Ok("team"),
        _ => Err("registration_mode must be 'solo' or 'team'".to_string()),
    }
}

/// Whitespace kollabieren + Längen-Validierung (wie clean_team_name).
pub fn clean_team_name(raw: &str) -> Result<String, String> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.chars().count() < TEAM_NAME_MIN {
        return Err(format!("Team name must be at least {TEAM_NAME_MIN} chars"));
    }
    if name.chars().count() > TEAM_NAME_MAX {
        return Err(format!("Team name must be at most {TEAM_NAME_MAX} chars"));
    }
    Ok(name)
}

pub fn team_name_key(name: &str) -> Result<String, String> {
    Ok(clean_team_name(name)?.to_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamRow {
    pub id: i64,
    pub name: String,
    pub created_by: Option<u64>,
    pub member_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub id: i64,
    pub name: String,
    pub created: bool,
    pub member_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signup {
    pub user_id: u64,
    pub registration_mode: String,
    pub rank: String,
    pub rank_value: i64,
    pub rank_subvalue: i64,
    pub display_name: Option<String>,
    pub team_id: Option<i64>,
    pub team_name: Option<String>,
    pub assigned_by_admin: bool,
    /// inserted | updated | unchanged (nur bei upsert gesetzt)
    pub status: String,
}

pub struct TournamentStore {
    pub db: Db,
}

impl TournamentStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS customgames_tournament_teams(
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       guild_id INTEGER NOT NULL,
                       name TEXT NOT NULL,
                       name_key TEXT NOT NULL,
                       created_by INTEGER,
                       created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                       UNIQUE(guild_id, name_key)
                     );
                     CREATE TABLE IF NOT EXISTS customgames_tournament_signups(
                       guild_id INTEGER NOT NULL,
                       user_id INTEGER NOT NULL,
                       registration_mode TEXT NOT NULL CHECK (registration_mode IN ('solo', 'team')),
                       rank TEXT NOT NULL,
                       rank_value INTEGER NOT NULL,
                       rank_subvalue INTEGER NOT NULL DEFAULT 0,
                       team_id INTEGER,
                       assigned_by_admin INTEGER NOT NULL DEFAULT 0,
                       created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                       updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                       display_name TEXT,
                       PRIMARY KEY(guild_id, user_id),
                       FOREIGN KEY(team_id) REFERENCES customgames_tournament_teams(id) ON DELETE SET NULL
                     );
                     CREATE TABLE IF NOT EXISTS tournament_periods(
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       guild_id INTEGER NOT NULL,
                       name TEXT NOT NULL,
                       registration_start DATETIME NOT NULL,
                       registration_end DATETIME NOT NULL,
                       is_active INTEGER NOT NULL DEFAULT 1,
                       team_size INTEGER NOT NULL DEFAULT 6,
                       created_by INTEGER,
                       created_at DATETIME DEFAULT CURRENT_TIMESTAMP
                     );
                     CREATE TABLE IF NOT EXISTS turnier_auth_tokens(
                       token TEXT PRIMARY KEY,
                       user_id INTEGER NOT NULL,
                       display_name TEXT NOT NULL,
                       expires_at REAL NOT NULL
                     );",
                )
            })
            .await
    }

    /// Idempotent: existierendes Team wird zurückgegeben (created=false).
    pub async fn get_or_create_team(
        &self,
        guild_id: u64,
        team_name: &str,
        created_by: Option<u64>,
    ) -> Result<Team, String> {
        let name = clean_team_name(team_name)?;
        let key = name.to_lowercase();
        let (name_clone, key_clone) = (name.clone(), key.clone());
        self.db
            .write(move |conn| {
                let existing: Option<(i64, String)> = conn
                    .query_row(
                        "SELECT id, name FROM customgames_tournament_teams
                          WHERE guild_id = ?1 AND name_key = ?2",
                        rusqlite::params![guild_id, key_clone],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                if let Some((id, name)) = existing {
                    return Ok(Team {
                        id,
                        name,
                        created: false,
                        member_count: 0,
                    });
                }
                conn.execute(
                    "INSERT INTO customgames_tournament_teams(guild_id, name, name_key, created_by)
                     VALUES(?1, ?2, ?3, ?4)",
                    rusqlite::params![guild_id, name_clone, key_clone, created_by],
                )?;
                Ok(Team {
                    id: conn.last_insert_rowid(),
                    name: name_clone,
                    created: true,
                    member_count: 0,
                })
            })
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn team_exists(&self, guild_id: u64, team_id: i64) -> bool {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT 1 FROM customgames_tournament_teams WHERE guild_id = ?1 AND id = ?2",
                    rusqlite::params![guild_id, team_id],
                    |_| Ok(()),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// Anmeldung anlegen/aktualisieren — Status inserted/updated/unchanged.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub async fn upsert_signup(
        &self,
        guild_id: u64,
        user_id: u64,
        registration_mode: &str,
        rank: &str,
        rank_subvalue: i64,
        team_id: Option<i64>,
        assigned_by_admin: bool,
        display_name: Option<String>,
    ) -> Result<Signup, String> {
        let mode = normalize_mode(registration_mode)?;
        let rank_key = normalize_rank(rank);
        let rank_num = rank_value(rank_key);
        let rank_sub = rank_subvalue.clamp(0, 6);
        if mode == "team" && team_id.is_none() {
            return Err("team_id is required for team registrations".to_string());
        }
        if let Some(team_id) = team_id {
            if !self.team_exists(guild_id, team_id).await {
                return Err("team_id does not exist in this guild".to_string());
            }
        }
        let assigned_flag = i64::from(assigned_by_admin);
        let dname = display_name
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());

        let status = self
            .db
            .write(move |conn| {
                let existing: Option<(String, String, i64, i64, Option<i64>, i64, Option<String>)> =
                    conn.query_row(
                        "SELECT registration_mode, rank, rank_value, rank_subvalue, team_id,
                                assigned_by_admin, display_name
                           FROM customgames_tournament_signups
                          WHERE guild_id = ?1 AND user_id = ?2",
                        rusqlite::params![guild_id, user_id],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                                row.get(6)?,
                            ))
                        },
                    )
                    .optional()?;
                match existing {
                    Some((p_mode, p_rank, p_val, p_sub, p_team, p_assigned, p_dname)) => {
                        let unchanged = p_mode == mode
                            && p_rank == rank_key
                            && p_val == rank_num
                            && p_sub == rank_sub
                            && p_team == team_id
                            && p_assigned == assigned_flag
                            && (dname.is_none() || p_dname == dname);
                        if unchanged {
                            return Ok("unchanged");
                        }
                        let effective_dname = dname.clone().or(p_dname);
                        conn.execute(
                            "UPDATE customgames_tournament_signups
                                SET registration_mode = ?1, rank = ?2, rank_value = ?3,
                                    rank_subvalue = ?4, team_id = ?5, assigned_by_admin = ?6,
                                    display_name = ?7, updated_at = CURRENT_TIMESTAMP
                              WHERE guild_id = ?8 AND user_id = ?9",
                            rusqlite::params![
                                mode,
                                rank_key,
                                rank_num,
                                rank_sub,
                                team_id,
                                assigned_flag,
                                effective_dname,
                                guild_id,
                                user_id
                            ],
                        )?;
                        Ok("updated")
                    }
                    None => {
                        conn.execute(
                            "INSERT INTO customgames_tournament_signups(
                               guild_id, user_id, registration_mode, rank, rank_value,
                               rank_subvalue, team_id, assigned_by_admin, display_name
                             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                            rusqlite::params![
                                guild_id,
                                user_id,
                                mode,
                                rank_key,
                                rank_num,
                                rank_sub,
                                team_id,
                                assigned_flag,
                                dname
                            ],
                        )?;
                        Ok("inserted")
                    }
                }
            })
            .await
            .map_err(|e| e.to_string())?;

        let mut signup = self
            .get_signup(guild_id, user_id)
            .await
            .ok_or_else(|| "signup missing after upsert".to_string())?;
        signup.status = status.to_string();
        Ok(signup)
    }

    pub async fn get_signup(&self, guild_id: u64, user_id: u64) -> Option<Signup> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT s.user_id, s.registration_mode, s.rank, s.rank_value,
                            s.rank_subvalue, s.display_name, s.team_id, s.assigned_by_admin,
                            t.name
                       FROM customgames_tournament_signups s
                       LEFT JOIN customgames_tournament_teams t
                         ON t.guild_id = s.guild_id AND t.id = s.team_id
                      WHERE s.guild_id = ?1 AND s.user_id = ?2",
                    rusqlite::params![guild_id, user_id],
                    |row| {
                        Ok(Signup {
                            user_id: row.get(0)?,
                            registration_mode: row.get(1)?,
                            rank: row.get(2)?,
                            rank_value: row.get(3)?,
                            rank_subvalue: row.get(4)?,
                            display_name: row.get(5)?,
                            team_id: row.get(6)?,
                            assigned_by_admin: row.get::<_, i64>(7)? != 0,
                            team_name: row.get(8)?,
                            status: String::new(),
                        })
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    pub async fn remove_signup(&self, guild_id: u64, user_id: u64) -> bool {
        self.db
            .write(move |conn| {
                Ok(conn.execute(
                    "DELETE FROM customgames_tournament_signups WHERE guild_id = ?1 AND user_id = ?2",
                    rusqlite::params![guild_id, user_id],
                )? > 0)
            })
            .await
            .unwrap_or(false)
    }

    /// Teams mit Mitgliederzahl, alphabetisch (wie list_teams_async).
    pub async fn list_teams(&self, guild_id: u64) -> Vec<TeamRow> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT t.id, t.name, t.created_by, COALESCE(COUNT(s.user_id), 0)
                       FROM customgames_tournament_teams t
                       LEFT JOIN customgames_tournament_signups s
                         ON s.guild_id = t.guild_id AND s.team_id = t.id
                      WHERE t.guild_id = ?1
                      GROUP BY t.id, t.name, t.created_by
                      ORDER BY lower(t.name) ASC",
                )?;
                let rows = stmt.query_map([guild_id], |row| {
                    Ok(TeamRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_by: row.get(2)?,
                        member_count: row.get(3)?,
                    })
                })?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }

    /// Alle Anmeldungen (Rang absteigend, frisch zuerst — wie list_signups_async).
    pub async fn list_signups(&self, guild_id: u64) -> Vec<Signup> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT s.user_id, s.registration_mode, s.rank, s.rank_value,
                            s.rank_subvalue, s.display_name, s.team_id, s.assigned_by_admin, t.name
                       FROM customgames_tournament_signups s
                       LEFT JOIN customgames_tournament_teams t
                         ON t.guild_id = s.guild_id AND t.id = s.team_id
                      WHERE s.guild_id = ?1
                      ORDER BY s.rank_value DESC, s.updated_at DESC",
                )?;
                let rows = stmt.query_map([guild_id], |row| {
                    Ok(Signup {
                        user_id: row.get(0)?,
                        registration_mode: row.get(1)?,
                        rank: row.get(2)?,
                        rank_value: row.get(3)?,
                        rank_subvalue: row.get(4)?,
                        display_name: row.get(5)?,
                        team_id: row.get(6)?,
                        assigned_by_admin: row.get::<_, i64>(7)? != 0,
                        team_name: row.get(8)?,
                        status: String::new(),
                    })
                })?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }

    pub async fn get_team(&self, guild_id: u64, team_id: i64) -> Option<TeamRow> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT id, name, created_by FROM customgames_tournament_teams
                      WHERE guild_id = ?1 AND id = ?2",
                    rusqlite::params![guild_id, team_id],
                    |row| {
                        Ok(TeamRow {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            created_by: row.get(2)?,
                            member_count: 0,
                        })
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    /// Umbenennen (UNIQUE-Kollision → deutscher Fehlertext wie Original).
    pub async fn rename_team(
        &self,
        guild_id: u64,
        team_id: i64,
        new_name: &str,
    ) -> Result<bool, String> {
        let name = clean_team_name(new_name)?;
        let key = name.to_lowercase();
        if self.get_team(guild_id, team_id).await.is_none() {
            return Ok(false);
        }
        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE customgames_tournament_teams SET name = ?1, name_key = ?2
                      WHERE guild_id = ?3 AND id = ?4",
                    rusqlite::params![name, key, guild_id, team_id],
                )
                .map(|_| true)
            })
            .await
            .map_err(|e| {
                if e.to_string().contains("UNIQUE") {
                    "Ein Team mit diesem Namen existiert bereits".to_string()
                } else {
                    e.to_string()
                }
            })
    }

    /// Team-Zuordnung setzen/lösen (assigned_by_admin folgt team_id wie Original).
    pub async fn assign_signup_team(
        &self,
        guild_id: u64,
        user_id: u64,
        team_id: Option<i64>,
    ) -> Result<bool, String> {
        if let Some(team_id) = team_id {
            if !self.team_exists(guild_id, team_id).await {
                return Err("team_id does not exist in this guild".to_string());
            }
        }
        let assigned = i64::from(team_id.is_some());
        self.db
            .write(move |conn| {
                Ok(conn.execute(
                    "UPDATE customgames_tournament_signups
                        SET team_id = ?1, assigned_by_admin = ?2, updated_at = CURRENT_TIMESTAMP
                      WHERE guild_id = ?3 AND user_id = ?4",
                    rusqlite::params![team_id, assigned, guild_id, user_id],
                )? > 0)
            })
            .await
            .map_err(|e| e.to_string())
    }

    /// Verifizierter Steam-Rang für die Web-Anmeldung.
    pub async fn verified_steam_rank(&self, user_id: u64) -> Option<(String, i64)> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT deadlock_rank_name, deadlock_subrank FROM steam_links
                      WHERE user_id = ?1 AND verified = 1
                      ORDER BY primary_account DESC, deadlock_rank_updated_at DESC LIMIT 1",
                    [user_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                        ))
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .map(|(name, sub)| {
                (
                    name.unwrap_or_else(|| "initiate".to_string()),
                    sub.unwrap_or(0),
                )
            })
    }

    /// Neue Periode aktiviert sich und deaktiviert die bisherige.
    pub async fn create_period(
        &self,
        guild_id: u64,
        name: &str,
        registration_start: &str,
        registration_end: &str,
        team_size: i64,
        created_by: Option<u64>,
    ) -> Result<i64, String> {
        let (name, start, end) = (
            name.to_string(),
            registration_start.to_string(),
            registration_end.to_string(),
        );
        let tsize = team_size.clamp(2, 20);
        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE tournament_periods SET is_active = 0 WHERE guild_id = ?1 AND is_active = 1",
                    [guild_id],
                )?;
                conn.execute(
                    "INSERT INTO tournament_periods(guild_id, name, registration_start, registration_end, is_active, team_size, created_by)
                     VALUES(?1, ?2, ?3, ?4, 1, ?5, ?6)",
                    rusqlite::params![guild_id, name, start, end, tsize, created_by],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn active_period(&self, guild_id: u64) -> Option<(i64, String, i64)> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT id, name, team_size FROM tournament_periods
                      WHERE guild_id = ?1 AND is_active = 1 ORDER BY id DESC LIMIT 1",
                    [guild_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    /// Aktive Periode als volles JSON-Objekt (Spalten wie das Original-Dict).
    pub async fn active_period_json(&self, guild_id: u64) -> Option<serde_json::Value> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT id, guild_id, name, registration_start, registration_end,
                            is_active, created_by, created_at
                       FROM tournament_periods
                      WHERE guild_id = ?1 AND is_active = 1 ORDER BY id DESC LIMIT 1",
                    [guild_id],
                    |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, i64>(0)?,
                            "guild_id": row.get::<_, i64>(1)?,
                            "name": row.get::<_, String>(2)?,
                            "registration_start": row.get::<_, String>(3)?,
                            "registration_end": row.get::<_, String>(4)?,
                            "is_active": row.get::<_, i64>(5)?,
                            "created_by": row.get::<_, Option<i64>>(6)?,
                            "created_at": row.get::<_, Option<String>>(7)?,
                        }))
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    /// (registration_start, registration_end) einer Periode.
    pub async fn period_window(&self, guild_id: u64, period_id: i64) -> Option<(String, String)> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT registration_start, registration_end FROM tournament_periods
                      WHERE guild_id = ?1 AND id = ?2",
                    rusqlite::params![guild_id, period_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    /// Einmal-Token für die Turnier-Website (TTL Sekunden).
    pub async fn create_auth_token(
        &self,
        user_id: u64,
        display_name: &str,
        ttl_seconds: f64,
    ) -> String {
        let token = hex::encode(rand::random::<[u8; 16]>());
        let display_name = display_name.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let expires_at = now + ttl_seconds;
        let token_clone = token.clone();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM turnier_auth_tokens WHERE expires_at < ?1",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO turnier_auth_tokens(token, user_id, display_name, expires_at)
                     VALUES(?1, ?2, ?3, ?4)",
                    rusqlite::params![token_clone, user_id, display_name, expires_at],
                )
                .map(|_| ())
            })
            .await;
        token
    }

    /// Lesen + löschen (One-Time); None wenn fehlend oder abgelaufen.
    pub async fn consume_auth_token(&self, token: &str) -> Option<(u64, String)> {
        let token = token.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.db
            .write(move |conn| {
                let row: Option<(u64, String, f64)> = conn
                    .query_row(
                        "SELECT user_id, display_name, expires_at FROM turnier_auth_tokens WHERE token = ?1",
                        [&token],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                conn.execute("DELETE FROM turnier_auth_tokens WHERE token = ?1", [&token])?;
                Ok(row.filter(|(_, _, expires)| *expires >= now)
                    .map(|(user_id, name, _)| (user_id, name)))
            })
            .await
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (tempfile::TempDir, TournamentStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let store = TournamentStore::new(db);
        store.ensure_schema().await.expect("schema");
        (dir, store)
    }

    #[test]
    fn validierung_wie_python() {
        assert_eq!(normalize_rank("Phantom"), "phantom");
        assert_eq!(normalize_rank("quatsch"), "initiate");
        assert_eq!(rank_value("eternus"), 11);
        assert_eq!(rank_value("initiate"), 1);
        assert!(normalize_mode("Solo").is_ok());
        assert!(normalize_mode("flex").is_err());
        assert_eq!(clean_team_name("  Team   Eins  ").expect("ok"), "Team Eins");
        assert!(clean_team_name("x").is_err());
        assert_eq!(team_name_key("Team EINS").expect("ok"), "team eins");
    }

    #[tokio::test]
    async fn team_und_signup_lifecycle() {
        let (_dir, store) = store().await;
        let team = store
            .get_or_create_team(1, "Die  Bestien", Some(100))
            .await
            .expect("team");
        assert!(team.created);
        assert_eq!(team.name, "Die Bestien");
        // idempotent (case-insensitiv)
        let again = store
            .get_or_create_team(1, "die bestien", None)
            .await
            .expect("team");
        assert!(!again.created);
        assert_eq!(again.id, team.id);

        // Solo-Signup → inserted, dann unchanged, dann updated
        let signup = store
            .upsert_signup(
                1,
                100,
                "solo",
                "phantom",
                3,
                None,
                false,
                Some("Anna".into()),
            )
            .await
            .expect("signup");
        assert_eq!(signup.status, "inserted");
        assert_eq!(signup.rank_value, 9);
        let signup = store
            .upsert_signup(1, 100, "solo", "phantom", 3, None, false, None)
            .await
            .expect("signup");
        assert_eq!(signup.status, "unchanged");
        assert_eq!(signup.display_name.as_deref(), Some("Anna"));
        let signup = store
            .upsert_signup(1, 100, "team", "phantom", 3, Some(team.id), false, None)
            .await
            .expect("signup");
        assert_eq!(signup.status, "updated");
        assert_eq!(signup.team_name.as_deref(), Some("Die Bestien"));

        // team-Modus ohne team_id → Fehler; unbekanntes Team → Fehler
        assert!(store
            .upsert_signup(1, 200, "team", "seeker", 0, None, false, None)
            .await
            .is_err());
        assert!(store
            .upsert_signup(1, 200, "team", "seeker", 0, Some(999), false, None)
            .await
            .is_err());

        assert!(store.remove_signup(1, 100).await);
        assert!(!store.remove_signup(1, 100).await);
    }

    #[tokio::test]
    async fn perioden_wechseln_aktiv() {
        let (_dir, store) = store().await;
        let first = store
            .create_period(1, "Cup #1", "2026-06-01", "2026-06-15", 6, Some(100))
            .await
            .expect("p1");
        assert_eq!(
            store.active_period(1).await.map(|(id, _, _)| id),
            Some(first)
        );
        let second = store
            .create_period(1, "Cup #2", "2026-07-01", "2026-07-15", 4, None)
            .await
            .expect("p2");
        let active = store.active_period(1).await.expect("aktiv");
        assert_eq!(active.0, second);
        assert_eq!(active.2, 4);
    }

    #[tokio::test]
    async fn auth_token_einmalig() {
        let (_dir, store) = store().await;
        let token = store.create_auth_token(100, "Anna", 60.0).await;
        assert_eq!(
            store.consume_auth_token(&token).await,
            Some((100, "Anna".to_string()))
        );
        // zweiter Verbrauch schlägt fehl (One-Time)
        assert_eq!(store.consume_auth_token(&token).await, None);
        // abgelaufener Token
        let expired = store.create_auth_token(200, "Ben", -1.0).await;
        assert_eq!(store.consume_auth_token(&expired).await, None);
    }
}
