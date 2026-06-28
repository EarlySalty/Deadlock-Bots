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

/// Balance-Score eines Anmelders (Port von `turnier.py::_rank_score`).
///
/// `tier` ist hier die gespeicherte `rank_value`-Spalte (1..=11), nicht der
/// Steam-Tier — genau wie der Original-Aufruf in `_run_auto_balance`
/// (`_rank_score(s.get("rank_value"), s.get("rank_subvalue"))`). `tier == 0` → 3
/// (greift praktisch nie, weil `rank_value` mindestens 1 ist, aber 1:1
/// übernommen).
///
/// Sub-Klemmung exakt wie Python `max(1, min(6, int(subrank or 3)))`: ein
/// fehlender/0-Subrank (häufig bei Solo-Anmeldern) zählt als **3**, nicht 1 —
/// sonst weicht die Score-Sortierung vom Original ab.
pub fn rank_score(tier: i64, subrank: i64) -> i64 {
    let sub = if subrank == 0 { 3 } else { subrank }.clamp(1, 6);
    if tier == 0 {
        return 3;
    }
    tier * 6 + sub
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
                let existing: Option<(i64, String, i64)> = conn
                    .query_row(
                        "SELECT t.id, t.name, COALESCE(COUNT(s.user_id), 0)
                           FROM customgames_tournament_teams t
                           LEFT JOIN customgames_tournament_signups s
                             ON s.guild_id = t.guild_id AND s.team_id = t.id
                          WHERE t.guild_id = ?1 AND t.name_key = ?2
                          GROUP BY t.id, t.name",
                        rusqlite::params![guild_id, key_clone],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                if let Some((id, name, member_count)) = existing {
                    return Ok(Team {
                        id,
                        name,
                        created: false,
                        member_count,
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
                    "SELECT t.id, t.name, t.created_by, COALESCE(COUNT(s.user_id), 0)
                       FROM customgames_tournament_teams t
                       LEFT JOIN customgames_tournament_signups s
                         ON s.guild_id = t.guild_id AND s.team_id = t.id
                      WHERE t.guild_id = ?1 AND t.id = ?2
                      GROUP BY t.id, t.name, t.created_by",
                    rusqlite::params![guild_id, team_id],
                    |row| {
                        Ok(TeamRow {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            created_by: row.get(2)?,
                            member_count: row.get(3)?,
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

    /// Auto-Balance (Port von `turnier.py::_run_auto_balance`, 1014-1141).
    ///
    /// Läuft nur bei aktiver Periode. Füllt zuerst nicht-volle Teams nach
    /// Rang-Score auf (volle Teams werden NIE angefasst), dann verteilt es die
    /// danach noch unzugewiesenen Solo-Anmelder per Snake-Draft auf neu
    /// erzeugte „Team A/B/…"-Teams. Schreibt autonom in die DB.
    ///
    /// Gibt `(neue_teams, zugewiesene_spieler)` zurück (wie die Log-Zeile am
    /// Python-Ende). `(0, 0)`, wenn nichts zu tun war.
    pub async fn auto_balance(&self, guild_id: u64) -> (usize, usize) {
        // Guard: nur innerhalb einer aktiven Periode (Python: get_active_period).
        let Some((_, _, period_team_size)) = self.active_period(guild_id).await else {
            return (0, 0);
        };
        let team_size = if period_team_size > 0 {
            period_team_size
        } else {
            TEAM_MAX_SIZE
        };

        let teams = self.list_teams(guild_id).await;
        let signups = self.list_signups(guild_id).await;
        if signups.is_empty() {
            return (0, 0);
        }

        // Volle Teams identifizieren (nie anfassen) + Mitgliederzählung.
        let mut full_team_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut team_member_counts: std::collections::HashMap<i64, i64> =
            std::collections::HashMap::new();
        for t in &teams {
            team_member_counts.insert(t.id, t.member_count);
            if t.member_count >= team_size {
                full_team_ids.insert(t.id);
            }
        }

        // Pool: unzugewiesen ODER in einem nicht-vollen Team.
        let mut pool: Vec<Signup> = signups
            .into_iter()
            .filter(|s| match s.team_id {
                None => true,
                Some(tid) => !full_team_ids.contains(&tid),
            })
            .collect();
        if pool.is_empty() {
            return (0, 0);
        }

        // Nach Rang-Score absteigend sortieren (stabil, wie Pythons list.sort).
        pool.sort_by_key(|s| std::cmp::Reverse(rank_score(s.rank_value, s.rank_subvalue)));

        // 1) Nicht-volle Teams zuerst auffüllen.
        let non_full_teams: Vec<&TeamRow> = teams
            .iter()
            .filter(|t| !full_team_ids.contains(&t.id))
            .collect();
        for team in non_full_teams {
            let tid = team.id;
            let current = team_member_counts.get(&tid).copied().unwrap_or(0);
            let slots_available = team_size - current;
            if slots_available <= 0 {
                continue;
            }
            // Spieler aus dem Pool, die nicht bereits in DIESEM Team sind.
            let to_add: Vec<u64> = pool
                .iter()
                .filter(|s| s.team_id != Some(tid))
                .take(slots_available as usize)
                .map(|s| s.user_id)
                .collect();
            for uid in to_add {
                match self.assign_signup_team(guild_id, uid, Some(tid)).await {
                    Ok(_) => {
                        pool.retain(|s| s.user_id != uid);
                        *team_member_counts.entry(tid).or_insert(0) += 1;
                    }
                    Err(err) => {
                        tracing::warn!(%err, user = uid, team = tid, "assign_signup_team fehlgeschlagen");
                    }
                }
            }
        }

        // Pool neu filtern: jetzt nur noch wirklich unzugewiesene Spieler.
        pool.retain(|s| s.team_id.is_none());
        if (pool.len() as i64) < team_size {
            return (0, 0);
        }

        // 2) Snake-Draft in neue Teams.
        // Nächste freien Auto-Namen ("Team A", "Team B", …) finden.
        let existing_names: std::collections::HashSet<String> =
            teams.iter().map(|t| t.name.to_lowercase()).collect();
        let num_new_teams = pool.len() / team_size as usize;
        let mut auto_names: Vec<String> = Vec::new();
        let mut letter_idx: u32 = 0;
        while auto_names.len() < num_new_teams {
            let name = format!("Team {}", char::from(b'A' + (letter_idx as u8)));
            if !existing_names.contains(&name.to_lowercase()) {
                auto_names.push(name);
            }
            letter_idx += 1;
            if letter_idx > 25 {
                // Fallback auf nummerierte Namen.
                let n = letter_idx - 25;
                let name = format!("Team {n}");
                if !existing_names.contains(&name.to_lowercase()) {
                    auto_names.push(name);
                }
                letter_idx += 1;
            }
        }

        let mut new_team_ids: Vec<i64> = Vec::new();
        for name in &auto_names {
            match self.get_or_create_team(guild_id, name, None).await {
                Ok(team) => new_team_ids.push(team.id),
                Err(err) => tracing::warn!(%err, name = %name, "get_or_create_team fehlgeschlagen"),
            }
        }
        if new_team_ids.is_empty() {
            return (0, 0);
        }

        // Snake-Draft-Zuweisung: Indizes 0,S-1,S,2S-1,2S,…
        let n_teams = new_team_ids.len();
        let assignable = (n_teams * team_size as usize).min(pool.len());
        for (i, player) in pool.iter().take(n_teams * team_size as usize).enumerate() {
            let cycle = i / n_teams;
            let pos_in_cycle = i % n_teams;
            let team_idx = if cycle % 2 == 0 {
                pos_in_cycle
            } else {
                n_teams - 1 - pos_in_cycle
            };
            if team_idx >= new_team_ids.len() {
                continue;
            }
            let tid = new_team_ids[team_idx];
            if let Err(err) = self
                .assign_signup_team(guild_id, player.user_id, Some(tid))
                .await
            {
                tracing::warn!(%err, user = player.user_id, team = tid, "Snake-draft assign fehlgeschlagen");
            }
        }

        tracing::info!(
            guild = guild_id,
            neue_teams = new_team_ids.len(),
            spieler = assignable,
            "Auto-Balance abgeschlossen"
        );
        (new_team_ids.len(), assignable)
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
                            is_active, team_size, created_by, created_at
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
                            "team_size": row.get::<_, i64>(6)?,
                            "created_by": row.get::<_, Option<i64>>(7)?,
                            "created_at": row.get::<_, Option<String>>(8)?,
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

    /// Zusammenfassung der Anmeldungen/Teams (wie `summary_async`).
    pub async fn summary(&self, guild_id: u64) -> Result<serde_json::Value, DbError> {
        self.db
            .read(move |conn| {
                let (signups_total, solo, team, unassigned): (i64, i64, i64, i64) = conn
                    .query_row(
                        "SELECT COUNT(*),
                                COALESCE(SUM(CASE WHEN registration_mode = 'solo' THEN 1 ELSE 0 END), 0),
                                COALESCE(SUM(CASE WHEN registration_mode = 'team' THEN 1 ELSE 0 END), 0),
                                COALESCE(SUM(CASE WHEN registration_mode = 'solo' AND team_id IS NULL THEN 1 ELSE 0 END), 0)
                         FROM customgames_tournament_signups WHERE guild_id = ?1",
                        [guild_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )?;
                let teams_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM customgames_tournament_teams WHERE guild_id = ?1",
                    [guild_id],
                    |r| r.get(0),
                )?;
                Ok(serde_json::json!({
                    "signups_total": signups_total,
                    "solo_count": solo,
                    "team_count": team,
                    "unassigned_solo": unassigned,
                    "teams_count": teams_count,
                }))
            })
            .await
    }

    /// Alle Perioden einer Gilde, neueste zuerst (wie `list_periods_async`).
    pub async fn list_periods(&self, guild_id: u64) -> Result<Vec<serde_json::Value>, DbError> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, guild_id, name, registration_start, registration_end,
                            is_active, created_at
                     FROM tournament_periods WHERE guild_id = ?1 ORDER BY id DESC",
                )?;
                let rows = stmt
                    .query_map([guild_id], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, i64>(0)?,
                            "guild_id": row.get::<_, i64>(1)?,
                            "name": row.get::<_, String>(2)?,
                            "registration_start": row.get::<_, String>(3)?,
                            "registration_end": row.get::<_, String>(4)?,
                            "is_active": row.get::<_, i64>(5)?,
                            "created_at": row.get::<_, Option<String>>(6)?,
                        }))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Deaktiviert eine Periode per ID (wie `close_period_async`). `false`, wenn
    /// sie nicht existiert.
    pub async fn close_period(&self, guild_id: u64, period_id: i64) -> Result<bool, DbError> {
        self.db
            .write(move |conn| {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM tournament_periods WHERE guild_id = ?1 AND id = ?2",
                        rusqlite::params![guild_id, period_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !exists {
                    return Ok(false);
                }
                conn.execute(
                    "UPDATE tournament_periods SET is_active = 0 WHERE guild_id = ?1 AND id = ?2",
                    rusqlite::params![guild_id, period_id],
                )?;
                Ok(true)
            })
            .await
    }

    /// Löscht ein Team und hängt seine Anmeldungen ab (wie `delete_team_async`).
    /// `false`, wenn das Team nicht existiert.
    pub async fn delete_team(&self, guild_id: u64, team_id: i64) -> Result<bool, DbError> {
        self.db
            .write(move |conn| {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM customgames_tournament_teams WHERE guild_id = ?1 AND id = ?2",
                        rusqlite::params![guild_id, team_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if !exists {
                    return Ok(false);
                }
                // Anmeldungen zuerst abhängen (SQLite-FK nicht garantiert).
                conn.execute(
                    "UPDATE customgames_tournament_signups SET team_id = NULL
                     WHERE guild_id = ?1 AND team_id = ?2",
                    rusqlite::params![guild_id, team_id],
                )?;
                conn.execute(
                    "DELETE FROM customgames_tournament_teams WHERE guild_id = ?1 AND id = ?2",
                    rusqlite::params![guild_id, team_id],
                )?;
                Ok(true)
            })
            .await
    }

    /// Löscht alle Anmeldungen einer Gilde, liefert die Anzahl (wie
    /// `clear_all_signups_async`).
    pub async fn clear_all_signups(&self, guild_id: u64) -> Result<i64, DbError> {
        self.db
            .write(move |conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM customgames_tournament_signups WHERE guild_id = ?1",
                    [guild_id],
                    |r| r.get(0),
                )?;
                conn.execute(
                    "DELETE FROM customgames_tournament_signups WHERE guild_id = ?1",
                    [guild_id],
                )?;
                Ok(count)
            })
            .await
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
    async fn team_member_count_in_get_team_und_get_or_create_team() {
        let (_dir, store) = store().await;
        let team = store
            .get_or_create_team(1, "Alpha", Some(100))
            .await
            .expect("team");
        assert!(team.created);
        assert_eq!(team.member_count, 0);

        store
            .upsert_signup(1, 10, "team", "phantom", 3, Some(team.id), false, None)
            .await
            .expect("signup 10");
        store
            .upsert_signup(1, 11, "team", "seeker", 2, Some(team.id), false, None)
            .await
            .expect("signup 11");

        let row = store.get_team(1, team.id).await.expect("team row");
        assert_eq!(row.member_count, 2);
        let again = store
            .get_or_create_team(1, "alpha", None)
            .await
            .expect("team again");
        assert!(!again.created);
        assert_eq!(again.member_count, 2);

        store
            .upsert_signup(1, 11, "solo", "seeker", 2, None, false, None)
            .await
            .expect("solo update");

        let row = store.get_team(1, team.id).await.expect("team row");
        assert_eq!(row.member_count, 1);
        let again = store
            .get_or_create_team(1, "Alpha", None)
            .await
            .expect("team again");
        assert_eq!(again.member_count, 1);
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

    #[test]
    fn rank_score_wie_python() {
        // sub 0/fehlend → 3 (Python: int(subrank or 3)), also 1*6+3 = 9.
        assert_eq!(rank_score(1, 0), 9);
        // rank_value 11, sub 6 → 11*6+6 = 72 (Eternus 6).
        assert_eq!(rank_score(11, 6), 72);
        // tier 0 → 3 (Obscurus-Sonderfall).
        assert_eq!(rank_score(0, 4), 3);
        // sub wird auf 1..=6 geklemmt (oberer Rand).
        assert_eq!(rank_score(5, 9), 5 * 6 + 6);
        // sub 1 bleibt 1 (untere Grenze, kein „or 3"-Default, da != 0).
        assert_eq!(rank_score(5, 1), 5 * 6 + 1);
    }

    #[tokio::test]
    async fn auto_balance_ohne_periode_macht_nichts() {
        let (_dir, store) = store().await;
        // Anmeldung ohne aktive Periode → kein Auto-Balance.
        store
            .upsert_signup(1, 100, "solo", "phantom", 0, None, false, None)
            .await
            .expect("signup");
        assert_eq!(store.auto_balance(1).await, (0, 0));
        // Spieler bleibt teamlos.
        assert!(store
            .get_signup(1, 100)
            .await
            .expect("sg")
            .team_id
            .is_none());
    }

    #[tokio::test]
    async fn auto_balance_snake_draft_in_neue_teams() {
        let (_dir, store) = store().await;
        store
            .create_period(1, "Cup", "2026-06-01", "2026-12-31", 2, None)
            .await
            .expect("period");
        // 4 Solo-Anmelder, absteigende Rang-Scores (eternus > phantom > seeker > initiate).
        for (uid, rank) in [
            (1u64, "eternus"),
            (2, "phantom"),
            (3, "seeker"),
            (4, "initiate"),
        ] {
            store
                .upsert_signup(1, uid, "solo", rank, 0, None, false, None)
                .await
                .expect("signup");
        }
        let (new_teams, assigned) = store.auto_balance(1).await;
        assert_eq!(new_teams, 2);
        assert_eq!(assigned, 4);
        // Snake-Draft team_size=2: A=[idx0,idx3], B=[idx1,idx2]
        // (pool nach Score absteigend: 1,2,3,4).
        let teams = store.list_teams(1).await;
        assert_eq!(teams.len(), 2);
        // Jedes Team voll (2 Spieler).
        for t in &teams {
            assert_eq!(t.member_count, 2, "team {} count", t.name);
        }
        // Spieler 1 und 4 im selben Team, 2 und 3 im selben.
        let t1 = store.get_signup(1, 1).await.expect("s1").team_id;
        let t4 = store.get_signup(1, 4).await.expect("s4").team_id;
        let t2 = store.get_signup(1, 2).await.expect("s2").team_id;
        let t3 = store.get_signup(1, 3).await.expect("s3").team_id;
        assert_eq!(t1, t4);
        assert_eq!(t2, t3);
        assert_ne!(t1, t2);
    }

    #[tokio::test]
    async fn auto_balance_laesst_volle_teams_unberuehrt() {
        let (_dir, store) = store().await;
        store
            .create_period(1, "Cup", "2026-06-01", "2026-12-31", 2, None)
            .await
            .expect("period");
        // Volles Team (team_size=2): 2 Mitglieder.
        let full = store.get_or_create_team(1, "Full", None).await.expect("t");
        store
            .upsert_signup(1, 10, "team", "phantom", 0, Some(full.id), false, None)
            .await
            .expect("s10");
        store
            .upsert_signup(1, 11, "team", "phantom", 0, Some(full.id), false, None)
            .await
            .expect("s11");
        // 2 freie Solo-Anmelder.
        store
            .upsert_signup(1, 20, "solo", "seeker", 0, None, false, None)
            .await
            .expect("s20");
        store
            .upsert_signup(1, 21, "solo", "seeker", 0, None, false, None)
            .await
            .expect("s21");
        let (new_teams, assigned) = store.auto_balance(1).await;
        // Volles Team unangetastet, 1 neues Team für die 2 Solo-Spieler.
        assert_eq!(new_teams, 1);
        assert_eq!(assigned, 2);
        // Die vollen Team-Mitglieder bleiben im selben Team.
        assert_eq!(
            store.get_signup(1, 10).await.expect("s10").team_id,
            Some(full.id)
        );
        assert_eq!(
            store.get_signup(1, 11).await.expect("s11").team_id,
            Some(full.id)
        );
        // Die Solo-Spieler haben jetzt ein (neues, anderes) Team.
        let t20 = store.get_signup(1, 20).await.expect("s20").team_id;
        assert!(t20.is_some());
        assert_ne!(t20, Some(full.id));
    }

    #[tokio::test]
    async fn admin_methoden() {
        let (_dir, store) = store().await;
        // Perioden: anlegen, listen, schliessen.
        let p1 = store
            .create_period(1, "Cup #1", "2026-06-01", "2026-06-15", 6, None)
            .await
            .expect("p1");
        store
            .create_period(1, "Cup #2", "2026-07-01", "2026-07-15", 4, None)
            .await
            .expect("p2");
        let periods = store.list_periods(1).await.expect("list");
        assert_eq!(periods.len(), 2);
        // neueste zuerst (Cup #2), nur sie ist aktiv
        assert_eq!(periods[0]["name"], "Cup #2");
        assert_eq!(periods[0]["is_active"], 1);
        assert_eq!(periods[1]["is_active"], 0);
        // schliessen: existierend → true, dann inaktiv; unbekannt → false
        assert!(store.close_period(1, p1).await.expect("close"));
        assert!(!store.close_period(1, 9999).await.expect("close"));

        // Team anlegen + löschen (Anmeldung wird abgehängt).
        let team = store
            .get_or_create_team(1, "Alpha", None)
            .await
            .expect("team");
        store
            .upsert_signup(1, 100, "team", "phantom", 0, Some(team.id), false, None)
            .await
            .expect("signup");
        assert!(store.delete_team(1, team.id).await.expect("delete"));
        assert!(!store.delete_team(1, team.id).await.expect("delete")); // schon weg
                                                                        // Signup existiert noch, aber ohne Team.
        let sg = store.get_signup(1, 100).await.expect("signup da");
        assert!(sg.team_id.is_none());

        // summary + clear.
        let summary = store.summary(1).await.expect("summary");
        assert_eq!(summary["signups_total"], 1);
        assert_eq!(summary["teams_count"], 0);
        let cleared = store.clear_all_signups(1).await.expect("clear");
        assert_eq!(cleared, 1);
        assert_eq!(store.summary(1).await.expect("s")["signups_total"], 0);
    }
}
