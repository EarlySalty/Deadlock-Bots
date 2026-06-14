//! Privacy/DSGVO-Kern — Opt-out-Gate und vollständige Datenlöschung.
//!
//! Port von `cogs/privacy_core.py` (reine DB-Logik; die Discord-Oberfläche
//! `/datenschutz` folgt separat). Der Löschpfad ist die **maßgebliche
//! Erasure-Vertrag**: eine atomare Transaktion über alle nutzerbezogenen
//! Tabellen (hartes DELETE, kein Anonymisieren), die Steam-seitigen Tabellen je
//! verknüpfter `steam_id`, die KV-Einträge und am Ende der bleibende Opt-out-
//! Grabstein in `user_privacy` (die Zeile selbst bleibt absichtlich erhalten).
//!
//! Jeder Zugriff ist tabellen-existenz-geschützt (wie Pythons `_table_exists`):
//! fehlt eine Tabelle, wird sie übersprungen (Zähler 0). So bleibt die Löschung
//! vollständig, auch während Python- und Rust-Bot dieselbe DB teilen.

use std::collections::{BTreeMap, HashSet};

use dl_db::{Db, DbError};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

/// Nutzerbezogene Tabellen `(Tabelle, Spalte)` — exakt `_USER_TABLES`
/// (privacy_core.py:19-55), inkl. der Tabellen, die je nach Stand evtl. (noch)
/// nicht existieren. Reihenfolge wie im Original.
const USER_TABLES: &[(&str, &str)] = &[
    ("voice_stats", "user_id"),
    ("voice_session_log", "user_id"),
    ("voice_feedback_requests", "user_id"),
    ("voice_feedback_responses", "user_id"),
    ("user_activity_patterns", "user_id"),
    ("message_activity", "user_id"),
    ("member_events", "user_id"),
    ("member_leave_surveys", "user_id"),
    ("steam_links", "user_id"),
    ("steam_cleanup_poll_state", "user_id"),
    ("steam_friendship_miss_tracker", "user_id"),
    ("steam_role_cleanup_pending", "user_id"),
    ("steam_beta_invites", "discord_id"),
    ("beta_invite_intent", "discord_id"),
    ("beta_invite_audit", "discord_id"),
    ("server_faq_logs", "user_id"),
    ("persistent_views", "user_id"),
    ("user_retention_tracking", "user_id"),
    ("user_retention_messages", "user_id"),
    ("voice_channel_anchors", "user_id"),
    ("coaching_sessions", "user_id"),
    ("steam_nudge_state", "user_id"),
    ("twitch_streamers", "discord_user_id"),
    ("twitch_link_clicks", "discord_user_id"),
    ("user_data", "user_id"),
    ("notification_log", "user_id"),
    ("notification_queue", "user_id"),
    ("dm_response_tracking", "user_id"),
    ("tempvoice_owner_prefs", "owner_id"),
    ("tempvoice_lanes", "owner_id"),
    ("tempvoice_lurkers", "user_id"),
    ("tempvoice_bans", "owner_id"),
    ("tempvoice_bans", "banned_id"),
    ("steam_quick_invites", "reserved_by"),
    ("issue_reports", "user_id"),
];

/// Steam-seitige Tabellen `(Tabelle, Spalte)`, je verknüpfter `steam_id`
/// gelöscht — exakt `_STEAM_SIDE_TABLES` (privacy_core.py:57-66).
const STEAM_SIDE_TABLES: &[(&str, &str)] = &[
    ("live_player_state", "steam_id"),
    ("deadlock_voice_watch", "steam_id"),
    ("steam_rich_presence", "steam_id"),
    ("steam_presence_watchlist", "steam_id"),
    ("steam_friend_requests", "steam_id"),
    ("steam_friendship_miss_tracker", "steam_id"),
    ("steam_beta_invites", "steam_id64"),
    ("beta_invite_audit", "steam_id64"),
];

/// Ergebnis einer Löschung: Zähler je Schlüssel (`tabelle.spalte`,
/// `tabelle:steam_id`, `kv_*`, `user_privacy_updated`) plus die entfernten
/// Steam-IDs (für die Anzeige im Discord-Layer).
#[derive(Debug, Default, Clone)]
pub struct DeleteSummary {
    pub counts: BTreeMap<String, i64>,
    pub steam_ids: Vec<String>,
}

impl DeleteSummary {
    /// Summe der Zähler aller angegebenen Schlüssel (für die Zusammenfassung).
    pub fn sum(&self, keys: &[&str]) -> i64 {
        keys.iter().filter_map(|k| self.counts.get(*k)).sum()
    }
}

fn existing_tables(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for r in rows {
        set.insert(r?);
    }
    Ok(set)
}

fn coerce_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Opt-out-Gate (Port `is_opted_out`, privacy_core.py:213-223). Fail-open: bei
/// Fehler ODER fehlender Zeile → `false` (Nutzer gilt als NICHT opted-out).
/// Wertbasiert (`opted_out != 0`), nicht zeilenbasiert.
pub async fn is_opted_out(db: &Db, user_id: i64) -> bool {
    db.read(move |conn| {
        let v: Option<i64> = conn
            .query_row(
                "SELECT opted_out FROM user_privacy WHERE user_id=?1",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.map(|x| x != 0).unwrap_or(false))
    })
    .await
    .unwrap_or(false)
}

/// Opt-in (Port `set_opt_in`, privacy_core.py:226-241). Hebt nur das Opt-out
/// auf; stellt KEINE gelöschten Daten wieder her.
pub async fn set_opt_in(db: &Db, user_id: i64, now: i64) -> Result<(), DbError> {
    db.write(move |conn| {
        conn.execute(
            "INSERT INTO user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
             VALUES (?1, 0, NULL, 'user_opt_in', ?2)
             ON CONFLICT(user_id) DO UPDATE SET
               opted_out = 0, deleted_at = NULL,
               reason = excluded.reason, updated_at = excluded.updated_at",
            params![user_id, now],
        )?;
        Ok(())
    })
    .await
}

/// Vollständige Löschung (Port `delete_user_data`, privacy_core.py:485-524).
/// Eine atomare Transaktion; bei Fehler Rollback (alles-oder-nichts).
/// Idempotent. `now` = Unix-Sekunden, `reason` z. B. `"slash_datenschutz"`.
pub async fn delete_user_data(
    db: &Db,
    user_id: i64,
    reason: String,
    now: i64,
) -> Result<DeleteSummary, DbError> {
    db.write(move |conn| {
        let tables = existing_tables(conn)?;
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();

        // Schritt 0: Steam-IDs VOR dem Löschen von steam_links erfassen.
        let steam_ids: Vec<String> = if tables.contains("steam_links") {
            let mut s = conn.prepare("SELECT steam_id FROM steam_links WHERE user_id=?1")?;
            let rows = s.query_map(params![user_id], |r| r.get::<_, Option<String>>(0))?;
            rows.filter_map(|r| r.ok().flatten())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };

        let tx = conn.transaction()?;

        // Schritt 1: nutzerbezogene Tabellen (hartes DELETE).
        for &(table, col) in USER_TABLES {
            if !tables.contains(table) {
                continue;
            }
            let n = tx.execute(
                &format!("DELETE FROM {table} WHERE {col}=?1"),
                params![user_id],
            )?;
            counts.insert(format!("{table}.{col}"), n as i64);
        }

        // Schritt 2: user_co_players — beide Seiten.
        if tables.contains("user_co_players") {
            let n = tx.execute(
                "DELETE FROM user_co_players WHERE user_id=?1 OR co_player_id=?1",
                params![user_id],
            )?;
            counts.insert("user_co_players".to_string(), n as i64);
        }

        // Schritt 3: Steam-seitige Tabellen je steam_id.
        for sid in &steam_ids {
            for &(table, col) in STEAM_SIDE_TABLES {
                if !tables.contains(table) {
                    continue;
                }
                let n = tx.execute(&format!("DELETE FROM {table} WHERE {col}=?1"), params![sid])?;
                counts.insert(format!("{table}:{sid}"), n as i64);
            }
        }

        // Schritt 4: KV-Einträge (ai_onboarding-Sessions/Views + voice_nudge).
        if tables.contains("kv_store") {
            let uid_key = user_id.to_string();
            let n = tx.execute(
                "DELETE FROM kv_store WHERE ns='ai_onboarding:sessions' AND k=?1",
                params![uid_key],
            )?;
            counts.insert("kv_ai_onboarding_sessions".to_string(), n as i64);

            // persistent_views: v ist JSON; löschen, wenn payload.user_id == uid.
            let pending: Vec<String> = {
                let mut s = tx.prepare(
                    "SELECT k, v FROM kv_store WHERE ns='ai_onboarding:persistent_views'",
                )?;
                let rows: Vec<(String, Option<String>)> = s
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                rows.into_iter()
                    .filter(|(_, v)| {
                        v.as_deref()
                            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                            .and_then(|p| p.get("user_id").and_then(coerce_i64))
                            == Some(user_id)
                    })
                    .map(|(k, _)| k)
                    .collect()
            };
            let mut views = 0i64;
            for k in pending {
                views += tx.execute(
                    "DELETE FROM kv_store WHERE ns='ai_onboarding:persistent_views' AND k=?1",
                    params![k],
                )? as i64;
            }
            counts.insert("kv_ai_onboarding_views".to_string(), views);

            let mut nudge = 0i64;
            for ns in ["voice_nudge_first_seen", "voice_nudge_done"] {
                nudge += tx.execute(
                    "DELETE FROM kv_store WHERE ns=?1 AND k=?2",
                    params![ns, uid_key],
                )? as i64;
            }
            counts.insert("kv_voice_nudge".to_string(), nudge);
        }

        // Schritt 5: Opt-out-Grabstein (Zeile bleibt erhalten).
        if tables.contains("user_privacy") {
            tx.execute(
                "INSERT INTO user_privacy(user_id, opted_out, deleted_at, reason, updated_at)
                 VALUES (?1, 1, ?2, ?3, ?2)
                 ON CONFLICT(user_id) DO UPDATE SET
                   opted_out = 1, deleted_at = excluded.deleted_at,
                   reason = excluded.reason, updated_at = excluded.updated_at",
                params![user_id, now, reason],
            )?;
            counts.insert("user_privacy_updated".to_string(), 1);
        }

        tx.commit()?;
        Ok(DeleteSummary { counts, steam_ids })
    })
    .await
}

fn sql_value_to_json(v: rusqlite::types::Value) -> Value {
    use rusqlite::types::Value as S;
    match v {
        S::Null => Value::Null,
        S::Integer(i) => Value::from(i),
        S::Real(f) => Value::from(f),
        S::Text(t) => Value::from(t),
        S::Blob(b) => {
            use base64::Engine;
            Value::from(base64::engine::general_purpose::STANDARD.encode(b))
        }
    }
}

/// `SELECT *` → Vec von Zeilen-Objekten (Spaltenname → JSON-Wert).
fn select_rows<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    p: P,
) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let cols: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let mut rows = stmt.query(p)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut obj = serde_json::Map::new();
        for (i, name) in cols.iter().enumerate() {
            obj.insert(
                name.clone(),
                sql_value_to_json(row.get::<_, rusqlite::types::Value>(i)?),
            );
        }
        out.push(Value::Object(obj));
    }
    Ok(out)
}

/// KV-Wert (parst JSON, sonst roher String) oder `None`.
fn kv_value(conn: &Connection, ns: &str, k: &str) -> rusqlite::Result<Option<Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT v FROM kv_store WHERE ns=?1 AND k=?2",
            params![ns, k],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.map(|s| serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s))))
}

/// Feldwert als i64 mit Python-Semantik `int(x or uid)`: null/0/fehlend → uid.
fn field_or(row: &serde_json::Map<String, Value>, key: &str, uid: i64) -> i64 {
    row.get(key)
        .and_then(coerce_i64)
        .filter(|v| *v != 0)
        .unwrap_or(uid)
}

fn redact_co_players(rows: Vec<Value>, uid: i64) -> Vec<Value> {
    let mut out = Vec::new();
    for row in rows {
        let Value::Object(mut obj) = row else {
            continue;
        };
        let uval = field_or(&obj, "user_id", uid);
        let coval = field_or(&obj, "co_player_id", uid);
        if uval != uid && coval != uid {
            continue; // nur Zeilen mit Nutzer-Beteiligung
        }
        if obj.contains_key("co_player_id") {
            obj.insert("co_player_id".into(), Value::from("redacted"));
        }
        if uval != uid {
            obj.insert("user_id".into(), Value::from(uid));
        }
        if obj.contains_key("user_display_name") && uval != uid {
            obj.insert("user_display_name".into(), Value::from("redacted"));
        }
        if obj.contains_key("co_player_display_name") && coval != uid {
            obj.insert("co_player_display_name".into(), Value::from("redacted"));
        }
        out.push(Value::Object(obj));
    }
    out
}

/// Redigiert ein Fremd-ID-Feld zu `"redacted"`, wenn es nicht dem Nutzer gehört.
fn redact_other_id(rows: &mut [Value], uid: i64, redact_field: &str) {
    for row in rows.iter_mut() {
        if let Value::Object(obj) = row {
            if obj.contains_key(redact_field) && field_or(obj, redact_field, 0) != uid {
                obj.insert(redact_field.into(), Value::from("redacted"));
            }
        }
    }
}

/// Vollständige Datenauskunft (Port `export_user_data`, privacy_core.py:403-482).
/// Read-only; inkl. der drei Redaktionen (Fremd-IDs werden nie geleakt).
pub async fn export_user_data(db: &Db, user_id: i64, now: i64) -> Result<Value, DbError> {
    db.read(move |conn| {
        let tables = existing_tables(conn)?;
        let mut tbl = serde_json::Map::new();

        for &(table, col) in USER_TABLES {
            if !tables.contains(table) {
                continue;
            }
            let rows = select_rows(
                conn,
                &format!("SELECT * FROM {table} WHERE {col}=?1"),
                params![user_id],
            )?;
            tbl.insert(format!("{table}.{col}"), Value::Array(rows));
        }

        if tables.contains("user_co_players") {
            let rows = select_rows(
                conn,
                "SELECT * FROM user_co_players WHERE user_id=?1 OR co_player_id=?1",
                params![user_id],
            )?;
            tbl.insert(
                "user_co_players".into(),
                Value::Array(redact_co_players(rows, user_id)),
            );
        }

        let steam_ids: Vec<String> = if tables.contains("steam_links") {
            let mut s = conn.prepare("SELECT steam_id FROM steam_links WHERE user_id=?1")?;
            let rows = s.query_map(params![user_id], |r| r.get::<_, Option<String>>(0))?;
            rows.filter_map(|r| r.ok().flatten())
                .map(|s| s.trim().to_string())
                .collect()
        } else {
            Vec::new()
        };
        for sid in &steam_ids {
            for &(table, col) in STEAM_SIDE_TABLES {
                if !tables.contains(table) {
                    continue;
                }
                let rows = select_rows(
                    conn,
                    &format!("SELECT * FROM {table} WHERE {col}=?1"),
                    params![sid],
                )?;
                tbl.insert(format!("{table}:{sid}"), Value::Array(rows));
            }
        }

        // Redaktion 1: co_player_ids in Voice-Logs (enthalten fremde IDs) → null.
        if let Some(Value::Array(rows)) = tbl.get_mut("voice_session_log.user_id") {
            for row in rows.iter_mut() {
                if let Value::Object(obj) = row {
                    if obj.contains_key("co_player_ids") {
                        obj.insert("co_player_ids".into(), Value::Null);
                    }
                }
            }
        }
        // Redaktion 2/3: tempvoice_bans Fremd-IDs.
        if let Some(Value::Array(rows)) = tbl.get_mut("tempvoice_bans.owner_id") {
            redact_other_id(rows, user_id, "banned_id");
        }
        if let Some(Value::Array(rows)) = tbl.get_mut("tempvoice_bans.banned_id") {
            redact_other_id(rows, user_id, "owner_id");
        }

        // KV-Sektion (nur wenn kv_store existiert).
        let mut kv = serde_json::Map::new();
        if tables.contains("kv_store") {
            let uid_key = user_id.to_string();
            kv.insert(
                "ai_onboarding_sessions".into(),
                kv_value(conn, "ai_onboarding:sessions", &uid_key)?.unwrap_or(Value::Null),
            );
            let mut views = Vec::new();
            {
                let mut s = conn
                    .prepare("SELECT v FROM kv_store WHERE ns='ai_onboarding:persistent_views'")?;
                let raws: Vec<Option<String>> = s
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for raw in raws.into_iter().flatten() {
                    if let Ok(payload) = serde_json::from_str::<Value>(&raw) {
                        if payload.get("user_id").and_then(coerce_i64).unwrap_or(0) == user_id {
                            views.push(payload);
                        }
                    }
                }
            }
            kv.insert("ai_onboarding_views".into(), Value::Array(views));
            kv.insert(
                "voice_nudge".into(),
                serde_json::json!({
                    "first_seen": kv_value(conn, "voice_nudge_first_seen", &uid_key)?,
                    "done": kv_value(conn, "voice_nudge_done", &uid_key)?,
                }),
            );
        }

        let user_privacy = if tables.contains("user_privacy") {
            Value::Array(select_rows(
                conn,
                "SELECT * FROM user_privacy WHERE user_id=?1",
                params![user_id],
            )?)
        } else {
            Value::Null
        };

        Ok(serde_json::json!({
            "user_id": user_id,
            "generated_at": now,
            "tables": Value::Object(tbl),
            "kv": Value::Object(kv),
            "steam_ids": steam_ids,
            "user_privacy": user_privacy,
        }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        // Minimal-Schema für die getesteten Pfade.
        db.write(|conn| {
            conn.execute_batch(
                "CREATE TABLE user_privacy(user_id INTEGER PRIMARY KEY, opted_out INTEGER NOT NULL DEFAULT 0, deleted_at INTEGER, reason TEXT, updated_at INTEGER);
                 CREATE TABLE voice_stats(user_id INTEGER, x INTEGER);
                 CREATE TABLE message_activity(user_id INTEGER, message_count INTEGER);
                 CREATE TABLE steam_links(user_id INTEGER, steam_id TEXT);
                 CREATE TABLE live_player_state(steam_id TEXT, foo TEXT);
                 CREATE TABLE user_co_players(user_id INTEGER, co_player_id INTEGER);
                 CREATE TABLE tempvoice_bans(owner_id INTEGER, banned_id INTEGER);
                 CREATE TABLE kv_store(ns TEXT, k TEXT, v TEXT, PRIMARY KEY(ns,k));",
            )?;
            Ok(())
        })
        .await
        .expect("schema");
        (dir, db)
    }

    #[tokio::test]
    async fn is_opted_out_fail_open_und_wertbasiert() {
        let (_d, db) = mk_db().await;
        // keine Zeile → false
        assert!(!is_opted_out(&db, 7).await);
        // opted_out=0 → false (wertbasiert, NICHT zeilenbasiert)
        db.write(|c| {
            c.execute(
                "INSERT INTO user_privacy(user_id,opted_out) VALUES(7,0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(!is_opted_out(&db, 7).await);
        // opted_out=1 → true
        set_opt_out_for_test(&db, 7).await;
        assert!(is_opted_out(&db, 7).await);
    }

    async fn set_opt_out_for_test(db: &Db, uid: i64) {
        db.write(move |c| {
            c.execute(
                "UPDATE user_privacy SET opted_out=1 WHERE user_id=?1",
                params![uid],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn set_opt_in_hebt_opt_out_auf() {
        let (_d, db) = mk_db().await;
        delete_user_data(&db, 5, "test".into(), 1000).await.unwrap();
        assert!(is_opted_out(&db, 5).await);
        set_opt_in(&db, 5, 2000).await.unwrap();
        assert!(!is_opted_out(&db, 5).await);
    }

    #[tokio::test]
    async fn delete_loescht_alle_seiten_und_setzt_grabstein() {
        let (_d, db) = mk_db().await;
        db.write(|c| {
            c.execute_batch(
                "INSERT INTO voice_stats(user_id,x) VALUES(42,1),(42,2),(99,1);
                 INSERT INTO message_activity(user_id,message_count) VALUES(42,10);
                 INSERT INTO steam_links(user_id,steam_id) VALUES(42,'STEAM_42');
                 INSERT INTO live_player_state(steam_id,foo) VALUES('STEAM_42','a');
                 INSERT INTO user_co_players(user_id,co_player_id) VALUES(42,99),(99,42),(1,2);
                 INSERT INTO tempvoice_bans(owner_id,banned_id) VALUES(42,7),(7,42);
                 INSERT INTO kv_store(ns,k,v) VALUES
                   ('ai_onboarding:sessions','42','{}'),
                   ('ai_onboarding:persistent_views','viewA','{\"user_id\":42}'),
                   ('ai_onboarding:persistent_views','viewB','{\"user_id\":99}'),
                   ('voice_nudge_done','42','1');",
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let s = delete_user_data(&db, 42, "slash_datenschutz".into(), 1234)
            .await
            .unwrap();

        // Steam-IDs erfasst, Steam-Seite gelöscht.
        assert_eq!(s.steam_ids, vec!["STEAM_42".to_string()]);
        assert_eq!(s.counts.get("live_player_state:STEAM_42").copied(), Some(1));
        // user_co_players beide Seiten weg (42<->99), Fremdzeile (1,2) bleibt.
        assert_eq!(s.counts.get("user_co_players").copied(), Some(2));
        // tempvoice_bans über owner_id UND banned_id.
        assert_eq!(s.counts.get("tempvoice_bans.owner_id").copied(), Some(1));
        assert_eq!(s.counts.get("tempvoice_bans.banned_id").copied(), Some(1));
        // KV: Session + nur der eigene View + voice_nudge.
        assert_eq!(s.counts.get("kv_ai_onboarding_sessions").copied(), Some(1));
        assert_eq!(s.counts.get("kv_ai_onboarding_views").copied(), Some(1));
        assert_eq!(s.counts.get("kv_voice_nudge").copied(), Some(1));
        assert_eq!(s.counts.get("user_privacy_updated").copied(), Some(1));

        // Verifikation in der DB: eigene Daten weg, fremde bleiben.
        let (vs_self, vs_other, cop_foreign, view_other, tomb): (i64, i64, i64, i64, i64) = db
            .read(|c| {
                Ok((
                    c.query_row(
                        "SELECT COUNT(*) FROM voice_stats WHERE user_id=42",
                        [],
                        |r| r.get(0),
                    )?,
                    c.query_row(
                        "SELECT COUNT(*) FROM voice_stats WHERE user_id=99",
                        [],
                        |r| r.get(0),
                    )?,
                    c.query_row(
                        "SELECT COUNT(*) FROM user_co_players WHERE user_id=1",
                        [],
                        |r| r.get(0),
                    )?,
                    c.query_row("SELECT COUNT(*) FROM kv_store WHERE k='viewB'", [], |r| {
                        r.get(0)
                    })?,
                    c.query_row(
                        "SELECT opted_out FROM user_privacy WHERE user_id=42",
                        [],
                        |r| r.get(0),
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(vs_self, 0, "eigene voice_stats gelöscht");
        assert_eq!(vs_other, 1, "fremde voice_stats unberührt");
        assert_eq!(cop_foreign, 1, "fremde co_player-Zeile unberührt");
        assert_eq!(view_other, 1, "fremder onboarding-view unberührt");
        assert_eq!(tomb, 1, "opt-out-Grabstein gesetzt");
    }

    #[tokio::test]
    async fn delete_ist_idempotent() {
        let (_d, db) = mk_db().await;
        db.write(|c| {
            c.execute("INSERT INTO voice_stats(user_id,x) VALUES(8,1)", [])?;
            Ok(())
        })
        .await
        .unwrap();
        let first = delete_user_data(&db, 8, "r".into(), 1).await.unwrap();
        assert_eq!(first.counts.get("voice_stats.user_id").copied(), Some(1));
        let second = delete_user_data(&db, 8, "r".into(), 2).await.unwrap();
        assert_eq!(second.counts.get("voice_stats.user_id").copied(), Some(0));
    }

    #[tokio::test]
    async fn export_redigiert_fremde_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("e.sqlite3")).expect("db");
        db.write(|c| {
            c.execute_batch(
                "CREATE TABLE voice_session_log(user_id INTEGER, co_player_ids TEXT);
                 CREATE TABLE user_co_players(user_id INTEGER, co_player_id INTEGER, user_display_name TEXT, co_player_display_name TEXT);
                 CREATE TABLE tempvoice_bans(owner_id INTEGER, banned_id INTEGER);
                 INSERT INTO voice_session_log(user_id,co_player_ids) VALUES(42,'[99,100]');
                 INSERT INTO user_co_players VALUES(42,99,'me','them'),(77,42,'them','me');
                 INSERT INTO tempvoice_bans VALUES(42,99),(88,42);",
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let snap = export_user_data(&db, 42, 5000).await.unwrap();

        // Redaktion 1: co_player_ids im Voice-Log → null.
        assert!(snap["tables"]["voice_session_log.user_id"][0]["co_player_ids"].is_null());

        // co_players: co_player_id redigiert, user_id immer auf Anfragenden gesetzt.
        let cps = snap["tables"]["user_co_players"].as_array().unwrap();
        assert_eq!(cps.len(), 2);
        for r in cps {
            assert_eq!(r["co_player_id"], serde_json::json!("redacted"));
            assert_eq!(r["user_id"], serde_json::json!(42));
        }

        // Redaktion 2/3: tempvoice_bans Fremd-IDs.
        assert_eq!(
            snap["tables"]["tempvoice_bans.owner_id"][0]["banned_id"],
            serde_json::json!("redacted")
        );
        assert_eq!(
            snap["tables"]["tempvoice_bans.banned_id"][0]["owner_id"],
            serde_json::json!("redacted")
        );
        assert_eq!(snap["user_id"], serde_json::json!(42));
    }
}
