//! `/api/server-stats` — aggregierte Server-Statistik + Beitritts-Quellen.
//!
//! Port von `_handle_server_stats` + `_build_member_source_analytics`. Die
//! Quellen-Klassifikation nutzt die korrigierte [`dl_activity::join_source`]
//! (mit Twitch-Override-Fix) — sobald die `twitch_streamer_invites`-Tabelle
//! befüllt ist (Daten-Brücke vom Twitch-Bot), zählen Streamer-Invite-Joins
//! korrekt als `twitch`. Der Metadaten-Backfill des Originals entfällt hier;
//! die rückwirkende Neu-Klassifikation ist ein eigener Lauf.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use dl_activity::join_source::{self, website_subpage_label};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};

use crate::web::{err_text, ok_json, DashboardApp};

const WEBSITE_SLUGS: [&str; 6] = [
    "landing",
    "streamer",
    "mitspieler",
    "coaching",
    "helden",
    "guides",
];

fn parse_guild_filter(params: &HashMap<String, String>) -> Result<Option<i64>, Response> {
    match params.get("guild_id").map(|s| s.trim()) {
        None | Some("") => Ok(None),
        Some(raw) => raw
            .parse::<i64>()
            .map(|v| (v != 0).then_some(v))
            .map_err(|_| err_text(400, "guild_id must be an integer")),
    }
}

struct JoinRow {
    user_id: i64,
    timestamp: Option<String>,
    display_name: Option<String>,
    metadata: Value,
}

struct SourceData {
    joins: Vec<JoinRow>,
    twitch_lookup: HashMap<String, String>,
    website_lookup: HashMap<String, String>,
    twitch_assigned_links: Vec<Value>,
}

fn coerce_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn meta_str(meta: &Value, key: &str) -> String {
    meta.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

pub async fn server_stats(
    State(app): State<DashboardApp>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(resp) = app.guard_read(&headers) {
        return resp;
    }
    let guild = match parse_guild_filter(&params) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    // ── Top-Level-Aggregate ────────────────────────────────────────────────
    let aggregates = app
        .db()
        .read(move |conn| {
            let mut member_events = Map::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT event_type, COUNT(*) FROM member_events
                     WHERE (? IS NULL OR guild_id = ?) GROUP BY event_type",
                )?;
                let mut rows = stmt.query(params![guild, guild])?;
                while let Some(r) = rows.next()? {
                    let t: Option<String> = r.get(0)?;
                    let c: i64 = r.get(1)?;
                    member_events.insert(t.unwrap_or_default(), json!(c));
                }
            }
            let total_messages: i64 = conn
                .query_row(
                    "SELECT SUM(message_count) FROM message_activity WHERE (? IS NULL OR guild_id = ?)",
                    params![guild, guild],
                    |r| r.get::<_, Option<i64>>(0),
                )?
                .unwrap_or(0);
            let total_seconds: i64 = conn
                .query_row(
                    "SELECT SUM(duration_seconds) FROM voice_session_log WHERE (? IS NULL OR guild_id = ?)",
                    params![guild, guild],
                    |r| r.get::<_, Option<i64>>(0),
                )?
                .unwrap_or(0);
            let active_users_7d: i64 = conn.query_row(
                "SELECT COUNT(DISTINCT user_id) FROM message_activity
                 WHERE (? IS NULL OR guild_id = ?) AND last_message_at >= datetime('now', '-7 days')",
                params![guild, guild],
                |r| r.get(0),
            )?;
            let (joins, leaves): (i64, i64) = conn.query_row(
                "SELECT SUM(CASE WHEN event_type='join' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN event_type='leave' THEN 1 ELSE 0 END)
                 FROM member_events
                 WHERE (? IS NULL OR guild_id = ?) AND timestamp >= datetime('now', '-30 days')",
                params![guild, guild],
                |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
            )?;
            Ok(json!({
                "member_events": member_events,
                "total_messages": total_messages,
                "total_voice_hours": total_seconds / 3600,
                "active_users_7d": active_users_7d,
                "growth_30d": { "joins": joins, "leaves": leaves, "net": joins - leaves },
            }))
        })
        .await;
    let mut payload = match aggregates {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(%err, "server_stats Aggregate fehlgeschlagen");
            return err_text(500, "Server stats unavailable");
        }
    };

    // ── Beitritts-Quellen ──────────────────────────────────────────────────
    let source_data = app
        .db()
        .read(move |conn| load_source_data(conn, guild))
        .await;
    let mut source = match source_data {
        Ok(d) => build_member_sources(d),
        Err(err) => {
            tracing::error!(%err, "server_stats Quellen fehlgeschlagen");
            return err_text(500, "Server stats unavailable");
        }
    };
    // Vanity-Links über die guild-stats-Broker-Bridge (statt Bot-Cache).
    let public_links = fetch_public_links(&app, guild).await;
    if let Value::Object(ref mut m) = source {
        m.insert("public_links".into(), json!(public_links));
    }
    payload["member_sources_30d"] = source;
    ok_json(payload)
}

fn load_source_data(
    conn: &rusqlite::Connection,
    guild: Option<i64>,
) -> rusqlite::Result<SourceData> {
    let mut stmt = conn.prepare(
        "SELECT user_id, timestamp, display_name, metadata FROM member_events
         WHERE event_type='join' AND (? IS NULL OR guild_id = ?) ORDER BY timestamp DESC",
    )?;
    let joins: Vec<JoinRow> = stmt
        .query_map(params![guild, guild], |r| {
            let meta_raw: Option<String> = r.get(3)?;
            let metadata = meta_raw
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            Ok(JoinRow {
                user_id: r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                timestamp: r.get(1)?,
                display_name: r.get(2)?,
                metadata,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // website_invites-KV → code → slug.
    let mut website_lookup = HashMap::new();
    {
        let mut s = conn.prepare("SELECT k, v FROM kv_store WHERE ns = 'website_invites'")?;
        let mut rows = s.query([])?;
        while let Some(r) = rows.next()? {
            let k: String = r.get(0)?;
            let v: Option<String> = r.get(1)?;
            let slug = if k == "main" {
                "landing".to_string()
            } else {
                k.trim().to_lowercase()
            };
            if !WEBSITE_SLUGS.contains(&slug.as_str()) {
                continue;
            }
            if let Some(code) = v
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|p| {
                    p.get("code")
                        .and_then(Value::as_str)
                        .map(|c| c.trim().to_string())
                })
                .filter(|c| !c.is_empty())
            {
                website_lookup.entry(code.to_lowercase()).or_insert(slug);
            }
        }
    }

    // twitch_streamer_invites (existiert evtl. nicht — dann leer).
    let mut twitch_lookup = HashMap::new();
    let mut twitch_assigned_links = Vec::new();
    let table_exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='twitch_streamer_invites'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if table_exists {
        let mut s = conn.prepare(
            "SELECT streamer_login, invite_code, invite_url, created_at, last_sent_at
             FROM twitch_streamer_invites WHERE (? IS NULL OR guild_id = ?) ORDER BY streamer_login",
        )?;
        let mut rows = s.query(params![guild, guild])?;
        while let Some(r) = rows.next()? {
            let login = r
                .get::<_, Option<String>>(0)?
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            let code = r
                .get::<_, Option<String>>(1)?
                .unwrap_or_default()
                .trim()
                .to_string();
            let url = r
                .get::<_, Option<String>>(2)?
                .unwrap_or_default()
                .trim()
                .to_string();
            let created: Option<String> = r.get(3)?;
            let last: Option<String> = r.get(4)?;
            if !code.is_empty() && !login.is_empty() {
                twitch_lookup
                    .entry(code.to_lowercase())
                    .or_insert(login.clone());
            }
            if !login.is_empty() || !code.is_empty() || !url.is_empty() {
                let invite_url = if !url.is_empty() {
                    Some(url)
                } else if !code.is_empty() {
                    Some(format!("https://discord.gg/{code}"))
                } else {
                    None
                };
                twitch_assigned_links.push(json!({
                    "streamer_login": (!login.is_empty()).then_some(login),
                    "invite_code": (!code.is_empty()).then_some(code),
                    "invite_url": invite_url,
                    "created_at": created,
                    "last_sent_at": last,
                }));
            }
        }
    }

    Ok(SourceData {
        joins,
        twitch_lookup,
        website_lookup,
        twitch_assigned_links,
    })
}

/// Geordnete Gruppen-Sammlung (Einfügereihenfolge bleibt erhalten).
#[derive(Default)]
struct Groups {
    keys: Vec<String>,
    map: HashMap<String, Value>,
}
impl Groups {
    fn entry_or<'a>(&'a mut self, key: &str, init: impl FnOnce() -> Value) -> &'a mut Value {
        if !self.map.contains_key(key) {
            self.keys.push(key.to_string());
            self.map.insert(key.to_string(), init());
        }
        self.map.get_mut(key).expect("eben eingefügt")
    }
    fn bump(entry: &mut Value) {
        let c = entry.get("count").and_then(Value::as_i64).unwrap_or(0);
        entry["count"] = json!(c + 1);
    }
    fn into_values(self) -> Vec<Value> {
        self.keys
            .into_iter()
            .filter_map(|k| self.map.get(&k).cloned())
            .collect()
    }
}

fn build_member_sources(data: SourceData) -> Value {
    let mut bucket_counts: HashMap<String, i64> = join_source::BUCKETS
        .iter()
        .map(|b| (b.to_string(), 0))
        .collect();
    let mut public_groups = Groups::default();
    public_groups.entry_or(
        "server_discovery",
        || json!({ "kind": "server_discovery", "label": "Server entdecken", "count": 0 }),
    );
    public_groups.entry_or(
        "other",
        || json!({ "kind": "other", "label": "Public (Sonstige)", "count": 0 }),
    );
    let mut website_groups = Groups::default();
    let mut twitch_groups = Groups::default();
    let mut personal_groups = Groups::default();
    let mut bot_invite_groups = Groups::default();
    let mut recent = Vec::new();

    let tracked_joins = data.joins.len();

    for row in &data.joins {
        let c = join_source::classify(&row.metadata, &data.twitch_lookup, &data.website_lookup);
        *bucket_counts.entry(c.bucket.clone()).or_insert(0) += 1;
        let code = c.invite_code.clone();
        let url = c.invite_url.clone();

        match c.bucket.as_str() {
            "public" => {
                if matches!(
                    c.kind.as_str(),
                    "server_discovery" | "discovery" | "public_discovery"
                ) {
                    Groups::bump(public_groups.entry_or("server_discovery", || json!({})));
                } else if matches!(c.kind.as_str(), "vanity" | "vanity_url" | "public_vanity") {
                    let key = code
                        .as_deref()
                        .map(|c| format!("vanity:{}", c.to_lowercase()))
                        .unwrap_or_else(|| "vanity:unknown".to_string());
                    let (code2, url2) = (code.clone(), url.clone());
                    let e = public_groups.entry_or(&key, || {
                        json!({
                            "kind": "vanity",
                            "label": code2.as_deref().map(|c| format!("discord.gg/{c}")).unwrap_or_else(|| "Vanity-Link".to_string()),
                            "invite_code": code2,
                            "invite_url": url2,
                            "count": 0,
                        })
                    });
                    Groups::bump(e);
                } else {
                    Groups::bump(public_groups.entry_or("other", || json!({})));
                }
            }
            "website" => {
                let slug = code
                    .as_deref()
                    .and_then(|c| data.website_lookup.get(&c.to_lowercase()))
                    .cloned()
                    .unwrap_or_else(|| "landing".to_string());
                let label = website_subpage_label(&slug).to_string();
                let (code2, url2, slug2, label2) =
                    (code.clone(), url.clone(), slug.clone(), label.clone());
                let e = website_groups.entry_or(&slug, || {
                    json!({
                        "subpage_slug": slug2, "subpage_label": label2.clone(), "label": label2,
                        "invite_code": code2, "invite_url": url2, "count": 0,
                    })
                });
                Groups::bump(e);
                fill_invite(e, &code, &url);
            }
            "twitch" => {
                let key = c
                    .twitch_login
                    .clone()
                    .or_else(|| code.as_ref().map(|c| c.to_lowercase()))
                    .unwrap_or_else(|| "unknown".to_string());
                let label = c
                    .twitch_login
                    .clone()
                    .or_else(|| code.as_ref().map(|c| format!("Invite {c}")))
                    .unwrap_or_else(|| "Unbekannt".to_string());
                let (login2, code2, url2) = (c.twitch_login.clone(), code.clone(), url.clone());
                let e = twitch_groups.entry_or(&key, || {
                    json!({
                        "streamer_login": login2, "label": label,
                        "invite_code": code2, "invite_url": url2, "count": 0,
                    })
                });
                Groups::bump(e);
                fill_invite(e, &code, &url);
            }
            "personal" => {
                let inviter_id = row.metadata.get("inviter_id").and_then(coerce_i64);
                let inviter_name = meta_str(&row.metadata, "inviter_name");
                let known = code
                    .as_deref()
                    .filter(|c| c.eq_ignore_ascii_case("xmnqmbuz7z"))
                    .map(|_| "In-Game (Build Publisher)".to_string());
                let (key, label) = if let Some(k) = known {
                    (
                        format!("known:{}", code.as_deref().unwrap_or("").to_lowercase()),
                        k,
                    )
                } else if let Some(id) = inviter_id {
                    (
                        format!("id:{id}"),
                        if inviter_name.is_empty() {
                            format!("User {id}")
                        } else {
                            inviter_name.clone()
                        },
                    )
                } else if !inviter_name.is_empty() {
                    (
                        format!("name:{}", inviter_name.to_lowercase()),
                        inviter_name.clone(),
                    )
                } else if let Some(cd) = &code {
                    (
                        format!("code:{}", cd.to_lowercase()),
                        format!("Invite {cd}"),
                    )
                } else {
                    ("other".to_string(), "Sonstige Invite-Links".to_string())
                };
                let (code2, url2) = (code.clone(), url.clone());
                let e = personal_groups.entry_or(&key, || {
                    json!({ "label": label, "inviter_id": inviter_id, "invite_code": code2, "invite_url": url2, "count": 0 })
                });
                Groups::bump(e);
                fill_invite(e, &code, &url);
            }
            "bot_invite" => {
                let inviter_id = row.metadata.get("inviter_id").and_then(coerce_i64);
                let inviter_name = meta_str(&row.metadata, "inviter_name");
                let (key, label) = if let Some(id) = inviter_id {
                    (
                        format!("id:{id}"),
                        if inviter_name.is_empty() {
                            format!("Bot {id}")
                        } else {
                            inviter_name.clone()
                        },
                    )
                } else if !inviter_name.is_empty() {
                    (
                        format!("name:{}", inviter_name.to_lowercase()),
                        inviter_name.clone(),
                    )
                } else {
                    ("bot_other".to_string(), "Bot Invite".to_string())
                };
                let (code2, url2) = (code.clone(), url.clone());
                let e = bot_invite_groups.entry_or(&key, || {
                    json!({ "label": label, "inviter_id": inviter_id, "invite_code": code2, "invite_url": url2, "count": 0 })
                });
                Groups::bump(e);
            }
            _ => {}
        }

        if recent.len() < 20 {
            recent.push(json!({
                "user_id": row.user_id,
                "display_name": row.display_name.clone().filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("User {}", row.user_id)),
                "timestamp": row.timestamp,
                "bucket": c.bucket,
                "label": c.label,
                "invite_code": code,
                "invite_url": url,
                "twitch_streamer_login": c.twitch_login,
            }));
        }
    }

    let count = |b: &str| *bucket_counts.get(b).unwrap_or(&0);
    let known_joins = count("public")
        + count("website")
        + count("twitch")
        + count("personal")
        + count("bot_invite");

    let public_breakdown: Vec<Value> = public_groups
        .into_values()
        .into_iter()
        .filter(|e| e.get("count").and_then(Value::as_i64).unwrap_or(0) > 0)
        .collect();
    let website_breakdown = sort_by_count(website_groups.into_values(), "subpage_label");
    let twitch_breakdown = sort_by_count(twitch_groups.into_values(), "label");
    let personal_breakdown = sort_by_count(personal_groups.into_values(), "label");
    let bot_invite_breakdown = sort_by_count(bot_invite_groups.into_values(), "label");

    let mut assigned = data.twitch_assigned_links;
    assigned.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v.get("streamer_login")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase(),
                v.get("invite_code")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase(),
            )
        };
        key(a).cmp(&key(b))
    });

    json!({
        "window_days": Value::Null,
        "tracked_joins": tracked_joins,
        "known_joins": known_joins,
        "unknown_joins": count("unknown"),
        "bucket_counts": {
            "public": count("public"), "website": count("website"), "twitch": count("twitch"),
            "personal": count("personal"), "bot_invite": count("bot_invite"), "unknown": count("unknown"),
        },
        "bucket_labels": {
            "public": "Public", "website": "Website", "twitch": "Twitch",
            "personal": "Persönlich", "bot_invite": "Bot Invites", "unknown": "Unbekannt",
        },
        "public_breakdown": public_breakdown,
        "website_breakdown": website_breakdown,
        "twitch_breakdown": twitch_breakdown,
        "twitch_assigned_links": assigned,
        "personal_breakdown": personal_breakdown,
        "bot_invite_breakdown": bot_invite_breakdown,
        "recent": recent,
    })
}

fn fill_invite(entry: &mut Value, code: &Option<String>, url: &Option<String>) {
    if let Some(code) = code {
        if entry.get("invite_code").map(Value::is_null).unwrap_or(true) {
            entry["invite_code"] = json!(code);
        }
    }
    if let Some(url) = url {
        if entry.get("invite_url").map(Value::is_null).unwrap_or(true) {
            entry["invite_url"] = json!(url);
        }
    }
}

fn sort_by_count(mut entries: Vec<Value>, label_key: &str) -> Vec<Value> {
    entries.sort_by(|a, b| {
        let ca = a.get("count").and_then(Value::as_i64).unwrap_or(0);
        let cb = b.get("count").and_then(Value::as_i64).unwrap_or(0);
        cb.cmp(&ca).then_with(|| {
            let la = a
                .get(label_key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let lb = b
                .get(label_key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            la.cmp(&lb)
        })
    });
    entries
}

/// Vanity-Link der Gilde über die guild-stats-Broker-Bridge (statt Bot-Cache).
async fn fetch_public_links(app: &DashboardApp, guild: Option<i64>) -> Vec<Value> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut url = format!(
        "{}/internal/master/v1/discord/guild-stats",
        app.broker_base().trim_end_matches('/')
    );
    if let Some(g) = guild {
        url.push_str(&format!("?guild_id={g}"));
    }
    let Ok(resp) = client.get(&url).send().await else {
        return Vec::new();
    };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let Ok(data) = resp.json::<Value>().await else {
        return Vec::new();
    };
    let code = data
        .get("vanity_url_code")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty());
    match code {
        Some(code) => vec![json!({
            "guild_id": data.get("guild_id").and_then(Value::as_str),
            "guild_name": data.get("name"),
            "type": "vanity",
            "label": "Vanity-Link",
            "code": code,
            "url": format!("https://discord.gg/{code}"),
        })],
        None => Vec::new(),
    }
}
