//! MCP-Connector — MCP-Server (Streamable HTTP) im dl-bot-Prozess.
//!
//! Claude (Desktop/Cowork) verbindet sich als MCP-Client auf
//! http://127.0.0.1:8890/mcp und arbeitet mit der Identität des laufenden Bots:
//! volle Discord-REST-Abdeckung im Rahmen der Bot-Berechtigungen plus
//! Export-Tools für Analysen (z.B. komplette Kategorien als JSON dumpen).
//!
//! Muster wie Broker/Changelog/Server-Sync: eigener axum-Router, loopback-only,
//! vom selben tokio::select! in main.rs getragen.
//!
//! Env (alle optional):
//!   MCP_CONNECTOR_HOST    default 127.0.0.1 (nicht öffentlich binden!)
//!   MCP_CONNECTOR_PORT    default 8890
//!   MCP_CONNECTOR_TOKEN   wenn gesetzt: Authorization: Bearer <t> ODER ?token=<t>
//!   MCP_DEFAULT_GUILD_ID  default: einzige Guild des Bots
//!   MCP_EXPORT_DIR        default data/mcp_exports (relativ zum WorkingDirectory)

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

const DISCORD_API: &str = "https://discord.com/api/v10";
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;
const PAGE_DELAY_MS: u64 = 450;
const MAX_INLINE_RESULT_CHARS: usize = 250_000;
const PROTOCOL_FALLBACK: &str = "2025-03-26";
const SUPPORTED_PROTOCOLS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

pub struct McpState {
    http: reqwest::Client,
    bot_token: String,
    auth_token: Option<String>,
    default_guild: Option<String>,
    export_dir: PathBuf,
}

impl McpState {
    pub fn from_env<F>(bot_token: String, lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("MCP: reqwest-Client")?;
        Ok(Self {
            http,
            bot_token,
            auth_token: lookup("MCP_CONNECTOR_TOKEN").filter(|s| !s.trim().is_empty()),
            default_guild: lookup("MCP_DEFAULT_GUILD_ID").filter(|s| !s.trim().is_empty()),
            export_dir: lookup("MCP_EXPORT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/mcp_exports")),
        })
    }

    pub fn bind_addr<F>(lookup: F) -> String
    where
        F: Fn(&str) -> Option<String>,
    {
        let host = lookup("MCP_CONNECTOR_HOST").unwrap_or_else(|| "127.0.0.1".to_string());
        let port = lookup("MCP_CONNECTOR_PORT").unwrap_or_else(|| "8890".to_string());
        format!("{host}:{port}")
    }
}

pub fn router(state: Arc<McpState>) -> Router {
    Router::new()
        .route("/mcp", post(mcp_post).get(mcp_get))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

// ───────────────────────────── HTTP-Ebene ─────────────────────────────

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn auth_ok(st: &McpState, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
    let Some(expected) = &st.auth_token else {
        return true;
    };
    if let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(tok) = v.strip_prefix("Bearer ") {
            if constant_time_eq(tok.trim(), expected) {
                return true;
            }
        }
    }
    query
        .get("token")
        .map(|t| constant_time_eq(t, expected))
        .unwrap_or(false)
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn mcp_get() -> Response {
    // Kein server-initiierter SSE-Stream nötig — Streamable HTTP erlaubt 405.
    json_response(
        StatusCode::METHOD_NOT_ALLOWED,
        json!({"error": "SSE-Stream nicht unterstützt"}),
    )
}

async fn mcp_post(
    State(st): State<Arc<McpState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !auth_ok(&st, &headers, &query) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"}));
    }
    let parsed: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return json_response(
                StatusCode::OK,
                json!({"jsonrpc": "2.0", "id": null,
                       "error": {"code": -32700, "message": format!("Parse error: {e}")}}),
            )
        }
    };

    match parsed {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if let Some(resp) = handle_rpc(&st, item).await {
                    out.push(resp);
                }
            }
            if out.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                json_response(StatusCode::OK, Value::Array(out))
            }
        }
        obj => match handle_rpc(&st, obj).await {
            Some(resp) => json_response(StatusCode::OK, resp),
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

// ───────────────────────────── JSON-RPC / MCP ─────────────────────────────

async fn handle_rpc(st: &Arc<McpState>, req: Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // Notifications (ohne id) → keine Antwort.
    let id = match id {
        Some(v) if !v.is_null() => v,
        _ => {
            tracing::debug!(method, "MCP notification");
            return None;
        }
    };

    let result: Result<Value> = match method.as_str() {
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => tools_call(st, &params).await,
        _ => Err(anyhow!("__method_not_found__")),
    };

    Some(match result {
        Ok(res) => json!({"jsonrpc": "2.0", "id": id, "result": res}),
        Err(e) if e.to_string() == "__method_not_found__" => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": format!("Method not found: {method}")}
        }),
        Err(e) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32603, "message": format!("{e:#}")}
        }),
    })
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or(PROTOCOL_FALLBACK);
    let version = if SUPPORTED_PROTOCOLS.contains(&requested) {
        requested
    } else {
        PROTOCOL_FALLBACK
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "deadlock-mcp-connector", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Discord-Zugriff über den laufenden Deadlock-Bot (REST v10, Bot-Identität). \
            Typisierte Tools für Überblick/History/Threads/Export/Senden/Member; `api_call` deckt \
            die restliche Discord-API generisch ab (Pfad relativ zu /api/v10). Große \
            Nachrichtenmengen mit export_category oder read_messages(save_to_file=true) als \
            JSON-Datei exportieren statt inline zurückgeben."
    })
}

fn tool_error(msg: String) -> Value {
    json!({ "content": [{"type": "text", "text": msg}], "isError": true })
}

async fn tools_call(st: &Arc<McpState>, params: &Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow!("tools/call ohne name"))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let outcome = match name {
        "server_overview" => tool_server_overview(st, &args).await,
        "list_channels" => tool_list_channels(st, &args).await,
        "read_messages" => tool_read_messages(st, &args).await,
        "list_threads" => tool_list_threads(st, &args).await,
        "export_category" => tool_export_category(st, &args).await,
        "send_message" => tool_send_message(st, &args).await,
        "search_members" => tool_search_members(st, &args).await,
        "api_call" => tool_api_call(st, &args).await,
        other => Err(anyhow!("Unbekanntes Tool: {other}")),
    };

    Ok(match outcome {
        Ok(v) => {
            let text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
            let text = if text.len() > MAX_INLINE_RESULT_CHARS {
                match spill_to_file(st, name, &v) {
                    Ok(path) => format!(
                        "Ergebnis zu groß für Inline-Ausgabe ({} Zeichen) — als Datei gespeichert: {}",
                        text.len(),
                        path.display()
                    ),
                    Err(e) => format!("Ergebnis zu groß und Datei-Spill fehlgeschlagen: {e:#}"),
                }
            } else {
                text
            };
            json!({ "content": [{"type": "text", "text": text}], "isError": false })
        }
        Err(e) => tool_error(format!("{e:#}")),
    })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "server_overview",
            "description": "Überblick über den Server: Guild-Infos, Rollen, Kategorien mit Channels, Member-Zahl.",
            "inputSchema": { "type": "object", "properties": {
                "guild_id": {"type": "string", "description": "Optional; sonst Default-Guild"}
            }}
        },
        {
            "name": "list_channels",
            "description": "Alle Channels der Guild, gruppiert nach Kategorie (inkl. IDs, Typen, Topics).",
            "inputSchema": { "type": "object", "properties": {
                "guild_id": {"type": "string"}
            }}
        },
        {
            "name": "read_messages",
            "description": "Nachrichtenverlauf eines Channels/Threads, vollständig paginiert (chronologisch). \
                Standard: neueste 200. full=true holt ALLES. Zeitfenster via after/before (ISO-Datum \
                oder Message-ID). save_to_file=true schreibt JSON in den Export-Ordner (empfohlen ab \
                ~1000 Nachrichten) und liefert nur Pfad+Statistik.",
            "inputSchema": { "type": "object", "required": ["channel_id"], "properties": {
                "channel_id": {"type": "string"},
                "max": {"type": "integer", "description": "Obergrenze, default 200 (bei full: unbegrenzt)"},
                "full": {"type": "boolean", "description": "Kompletten Verlauf holen"},
                "after": {"type": "string", "description": "ISO-Datum/Zeit oder Snowflake"},
                "before": {"type": "string", "description": "ISO-Datum/Zeit oder Snowflake"},
                "include_threads": {"type": "boolean", "description": "Threads des Channels mitlesen"},
                "save_to_file": {"type": "boolean"},
                "guild_id": {"type": "string", "description": "Nur nötig für include_threads"}
            }}
        },
        {
            "name": "list_threads",
            "description": "Aktive + archivierte Threads eines Channels (auch Forum-Channels).",
            "inputSchema": { "type": "object", "required": ["channel_id"], "properties": {
                "channel_id": {"type": "string"},
                "guild_id": {"type": "string"}
            }}
        },
        {
            "name": "export_category",
            "description": "Exportiert ALLE Nachrichten aller Text-/Announcement-/Forum-Channels einer \
                Kategorie (inkl. Threads, auch archivierte) als JSON-Dateien in den Export-Ordner. \
                Liefert Zusammenfassung mit Pfaden und Zählern. Für große Analysen der richtige Weg.",
            "inputSchema": { "type": "object", "required": ["category"], "properties": {
                "category": {"type": "string", "description": "Kategorie-Name (case-insensitiv) oder ID"},
                "guild_id": {"type": "string"},
                "include_threads": {"type": "boolean", "description": "default true"},
                "after": {"type": "string", "description": "Nur Nachrichten nach diesem ISO-Datum"}
            }}
        },
        {
            "name": "send_message",
            "description": "Nachricht als Bot in einen Channel senden (optional als Reply).",
            "inputSchema": { "type": "object", "required": ["channel_id", "content"], "properties": {
                "channel_id": {"type": "string"},
                "content": {"type": "string"},
                "reply_to": {"type": "string", "description": "Message-ID für Reply"}
            }}
        },
        {
            "name": "search_members",
            "description": "Mitglieder per Namens-Prefix suchen (Username/Nickname).",
            "inputSchema": { "type": "object", "required": ["query"], "properties": {
                "query": {"type": "string"},
                "guild_id": {"type": "string"},
                "limit": {"type": "integer", "description": "default 10, max 1000"}
            }}
        },
        {
            "name": "api_call",
            "description": "Generischer Discord-REST-Call (v10) mit Bot-Token — voller API-Zugriff im Rahmen \
                der Bot-Berechtigungen. path relativ, z.B. '/guilds/{id}/roles'. Destruktive Aufrufe \
                (DELETE, Bans, Prune, Bulk-Delete) verlangen confirm=true. Optional reason für Audit-Log.",
            "inputSchema": { "type": "object", "required": ["method", "path"], "properties": {
                "method": {"type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"]},
                "path": {"type": "string"},
                "query": {"type": "object", "additionalProperties": {"type": "string"}},
                "body": {"type": "object"},
                "reason": {"type": "string", "description": "X-Audit-Log-Reason"},
                "confirm": {"type": "boolean", "description": "Pflicht bei destruktiven Aufrufen"}
            }}
        }
    ])
}

// ───────────────────────────── Discord-REST-Kern ─────────────────────────────

async fn discord_call(
    st: &McpState,
    method: &str,
    path: &str,
    query: &[(String, String)],
    body: Option<&Value>,
    audit_reason: Option<&str>,
) -> Result<Value> {
    let url = format!("{DISCORD_API}{path}");
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let m = reqwest::Method::from_bytes(method.as_bytes()).context("HTTP-Methode")?;
        let mut req = st
            .http
            .request(m, &url)
            .header("Authorization", format!("Bot {}", st.bot_token))
            .header(
                "User-Agent",
                "DiscordBot (Deadlock-Bots mcp-connector, 1.0)",
            );
        if !query.is_empty() {
            req = req.query(query);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        if let Some(reason) = audit_reason {
            req = req.header("X-Audit-Log-Reason", reason);
        }
        let resp = req.send().await.context("Discord-Request fehlgeschlagen")?;
        let status = resp.status();

        if status.as_u16() == 429 {
            let hdr_retry = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<f64>().ok());
            let body_json: Value = resp.json().await.unwrap_or(Value::Null);
            let retry = body_json
                .get("retry_after")
                .and_then(|v| v.as_f64())
                .or(hdr_retry)
                .unwrap_or(1.0);
            if attempt > 8 {
                bail!("Rate-Limit: zu viele Versuche ({url})");
            }
            tracing::warn!(path, retry, attempt, "MCP: Discord 429, warte");
            tokio::time::sleep(Duration::from_millis((retry * 1000.0) as u64 + 100)).await;
            continue;
        }
        if status.is_server_error() && attempt <= 3 {
            tracing::warn!(%status, path, attempt, "MCP: Discord 5xx, Retry");
            tokio::time::sleep(Duration::from_millis(800 * u64::from(attempt))).await;
            continue;
        }
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "Discord {status}: {} ({method} {path})",
                truncate_str(&text, 600)
            );
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(&text).context("Discord-Antwort kein JSON");
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

async fn resolve_guild(st: &McpState, args: &Value) -> Result<String> {
    if let Some(g) = args.get("guild_id").and_then(|v| v.as_str()) {
        if !g.trim().is_empty() {
            return Ok(g.to_string());
        }
    }
    if let Some(g) = &st.default_guild {
        return Ok(g.clone());
    }
    let guilds = discord_call(st, "GET", "/users/@me/guilds", &[], None, None).await?;
    let arr = guilds.as_array().cloned().unwrap_or_default();
    match arr.len() {
        1 => Ok(arr[0]["id"].as_str().unwrap_or_default().to_string()),
        0 => bail!("Bot ist in keiner Guild"),
        n => bail!(
            "Bot ist in {n} Guilds — guild_id angeben oder MCP_DEFAULT_GUILD_ID setzen. Guilds: {}",
            arr.iter()
                .map(|g| format!(
                    "{} ({})",
                    g["name"].as_str().unwrap_or("?"),
                    g["id"].as_str().unwrap_or("?")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// ISO-Datum/Zeit oder rohe Snowflake → Snowflake-ID (u64).
fn to_snowflake(input: &str) -> Result<u64> {
    let s = input.trim();
    if s.chars().all(|c| c.is_ascii_digit()) && s.len() >= 15 {
        return s.parse::<u64>().context("Snowflake ungültig");
    }
    let normalized = if s.len() == 10 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    let dt: DateTime<Utc> = normalized
        .parse::<DateTime<Utc>>()
        .or_else(|_| DateTime::parse_from_rfc3339(&normalized).map(|d| d.with_timezone(&Utc)))
        .with_context(|| format!("Zeitangabe nicht parsebar: {input}"))?;
    let ms = dt.timestamp_millis() - DISCORD_EPOCH_MS;
    if ms < 0 {
        bail!("Zeitpunkt liegt vor der Discord-Epoche");
    }
    Ok((ms as u64) << 22)
}

fn snowflake_to_iso(id: &str) -> Option<String> {
    let n: u64 = id.parse().ok()?;
    let ms = (n >> 22) as i64 + DISCORD_EPOCH_MS;
    DateTime::<Utc>::from_timestamp_millis(ms).map(|d| d.to_rfc3339())
}

fn simplify_message(m: &Value) -> Value {
    let reactions: Vec<Value> = m
        .get("reactions")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    json!({
                        "emoji": r["emoji"]["name"].as_str().unwrap_or("?"),
                        "count": r["count"].as_u64().unwrap_or(0)
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let attachments: Vec<Value> = m
        .get("attachments")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .map(|a| json!({"filename": a["filename"], "content_type": a["content_type"]}))
                .collect()
        })
        .unwrap_or_default();
    let mut out = json!({
        "id": m["id"],
        "timestamp": m["timestamp"],
        "author": {
            "id": m["author"]["id"],
            "username": m["author"]["username"],
            "global_name": m["author"]["global_name"],
            "bot": m["author"]["bot"].as_bool().unwrap_or(false)
        },
        "content": m["content"],
        "type": m["type"]
    });
    if let Some(obj) = out.as_object_mut() {
        if !reactions.is_empty() {
            obj.insert("reactions".into(), Value::Array(reactions));
        }
        if !attachments.is_empty() {
            obj.insert("attachments".into(), Value::Array(attachments));
        }
        if let Some(r) = m.get("message_reference").and_then(|r| r.get("message_id")) {
            obj.insert("reply_to".into(), r.clone());
        }
        if let Some(t) = m.get("thread").and_then(|t| t.get("id")) {
            obj.insert("started_thread".into(), t.clone());
        }
        if m.get("edited_timestamp")
            .map(|e| !e.is_null())
            .unwrap_or(false)
        {
            obj.insert("edited".into(), json!(true));
        }
        if let Some(embeds) = m.get("embeds").and_then(|e| e.as_array()) {
            if !embeds.is_empty() {
                obj.insert("embed_count".into(), json!(embeds.len()));
            }
        }
    }
    out
}

/// Kompletten (oder begrenzten) Verlauf holen. Rückgabe chronologisch (älteste zuerst).
async fn fetch_messages(
    st: &McpState,
    channel_id: &str,
    after: Option<u64>,
    before: Option<u64>,
    max: usize, // 0 = unbegrenzt
) -> Result<Vec<Value>> {
    let mut collected: Vec<Value> = Vec::new();
    let mut cursor: Option<u64> = before;
    loop {
        let mut query = vec![("limit".to_string(), "100".to_string())];
        if let Some(c) = cursor {
            query.push(("before".to_string(), c.to_string()));
        }
        let batch = discord_call(
            st,
            "GET",
            &format!("/channels/{channel_id}/messages"),
            &query,
            None,
            None,
        )
        .await?;
        let arr = batch.as_array().cloned().unwrap_or_default();
        if arr.is_empty() {
            break;
        }
        let mut oldest: Option<u64> = None;
        let mut hit_lower_bound = false;
        for m in &arr {
            let id: u64 = m["id"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
            oldest = Some(oldest.map_or(id, |o: u64| o.min(id)));
            if let Some(a) = after {
                if id <= a {
                    hit_lower_bound = true;
                    continue;
                }
            }
            collected.push(m.clone());
        }
        if max > 0 && collected.len() >= max {
            collected.truncate(max);
            break;
        }
        if hit_lower_bound || arr.len() < 100 {
            break;
        }
        cursor = oldest;
        tokio::time::sleep(Duration::from_millis(PAGE_DELAY_MS)).await;
    }
    // Discord liefert neueste zuerst → chronologisch drehen.
    collected.reverse();
    Ok(collected)
}

/// Aktive + archivierte Threads eines Channels.
async fn fetch_threads(st: &McpState, guild_id: &str, channel_id: &str) -> Result<Vec<Value>> {
    let mut threads: Vec<Value> = Vec::new();

    // Aktive Threads (guild-weit, nach parent gefiltert)
    if let Ok(active) = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}/threads/active"),
        &[],
        None,
        None,
    )
    .await
    {
        if let Some(arr) = active.get("threads").and_then(|t| t.as_array()) {
            threads.extend(
                arr.iter()
                    .filter(|t| t["parent_id"].as_str() == Some(channel_id))
                    .cloned(),
            );
        }
    }

    // Archivierte Threads (öffentlich; privat falls berechtigt), paginiert
    for kind in ["public", "private"] {
        let mut before: Option<String> = None;
        loop {
            let mut query = vec![("limit".to_string(), "100".to_string())];
            if let Some(b) = &before {
                query.push(("before".to_string(), b.clone()));
            }
            let path = format!("/channels/{channel_id}/threads/archived/{kind}");
            let page = match discord_call(st, "GET", &path, &query, None, None).await {
                Ok(p) => p,
                Err(e) => {
                    // private ggf. ohne Berechtigung — still überspringen
                    tracing::debug!(kind, channel_id, error = %e, "MCP: archived threads übersprungen");
                    break;
                }
            };
            let arr = page
                .get("threads")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default();
            if arr.is_empty() {
                break;
            }
            before = arr
                .last()
                .and_then(|t| t["thread_metadata"]["archive_timestamp"].as_str())
                .map(|s| s.to_string());
            threads.extend(arr);
            if !page
                .get("has_more")
                .and_then(|h| h.as_bool())
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(PAGE_DELAY_MS)).await;
        }
    }
    Ok(threads)
}

fn sanitize_filename(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "channel".to_string()
    } else {
        trimmed.to_lowercase()
    }
}

fn spill_to_file(st: &McpState, prefix: &str, v: &Value) -> Result<PathBuf> {
    std::fs::create_dir_all(&st.export_dir).context("Export-Dir anlegen")?;
    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    let path = st
        .export_dir
        .join(format!("{stamp}_{}.json", sanitize_filename(prefix)));
    let file = std::fs::File::create(&path).context("Datei anlegen")?;
    serde_json::to_writer_pretty(file, v).context("JSON schreiben")?;
    Ok(path)
}

// ───────────────────────────── Tools ─────────────────────────────

async fn tool_server_overview(st: &McpState, args: &Value) -> Result<Value> {
    let guild_id = resolve_guild(st, args).await?;
    let guild = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}"),
        &[("with_counts".into(), "true".into())],
        None,
        None,
    )
    .await?;
    let channels = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}/channels"),
        &[],
        None,
        None,
    )
    .await?;
    let roles = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}/roles"),
        &[],
        None,
        None,
    )
    .await?;
    Ok(json!({
        "guild": {
            "id": guild["id"], "name": guild["name"],
            "member_count": guild["approximate_member_count"],
            "online": guild["approximate_presence_count"]
        },
        "channels": group_channels(&channels),
        "roles": roles.as_array().map(|arr| arr.iter().map(|r| json!({
            "id": r["id"], "name": r["name"], "position": r["position"]
        })).collect::<Vec<_>>()).unwrap_or_default()
    }))
}

fn group_channels(channels: &Value) -> Value {
    let arr = channels.as_array().cloned().unwrap_or_default();
    let mut categories: Vec<Value> = Vec::new();
    let mut by_parent: HashMap<Option<String>, Vec<Value>> = HashMap::new();
    for c in &arr {
        let t = c["type"].as_u64().unwrap_or(0);
        let entry = json!({
            "id": c["id"], "name": c["name"], "type": t,
            "topic": c["topic"], "position": c["position"]
        });
        if t == 4 {
            categories.push(c.clone());
        } else {
            by_parent
                .entry(c["parent_id"].as_str().map(|s| s.to_string()))
                .or_default()
                .push(entry);
        }
    }
    categories.sort_by_key(|c| c["position"].as_i64().unwrap_or(0));
    let mut out: Vec<Value> = Vec::new();
    for cat in &categories {
        let id = cat["id"].as_str().unwrap_or_default().to_string();
        out.push(json!({
            "category": {"id": cat["id"], "name": cat["name"]},
            "channels": by_parent.remove(&Some(id)).unwrap_or_default()
        }));
    }
    if let Some(orphans) = by_parent.remove(&None) {
        out.push(json!({"category": null, "channels": orphans}));
    }
    Value::Array(out)
}

async fn tool_list_channels(st: &McpState, args: &Value) -> Result<Value> {
    let guild_id = resolve_guild(st, args).await?;
    let channels = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}/channels"),
        &[],
        None,
        None,
    )
    .await?;
    Ok(group_channels(&channels))
}

async fn tool_read_messages(st: &McpState, args: &Value) -> Result<Value> {
    let channel_id = args["channel_id"]
        .as_str()
        .ok_or_else(|| anyhow!("channel_id fehlt"))?;
    let full = args["full"].as_bool().unwrap_or(false);
    let max = args["max"]
        .as_u64()
        .map(|m| m as usize)
        .unwrap_or(if full { 0 } else { 200 });
    let after = match args["after"].as_str() {
        Some(s) => Some(to_snowflake(s)?),
        None => None,
    };
    let before = match args["before"].as_str() {
        Some(s) => Some(to_snowflake(s)?),
        None => None,
    };

    let msgs = fetch_messages(st, channel_id, after, before, max).await?;
    let mut result = json!({
        "channel_id": channel_id,
        "count": msgs.len(),
        "oldest": msgs.first().and_then(|m| m["timestamp"].as_str()),
        "newest": msgs.last().and_then(|m| m["timestamp"].as_str()),
        "messages": msgs.iter().map(simplify_message).collect::<Vec<_>>()
    });

    if args["include_threads"].as_bool().unwrap_or(false) {
        let guild_id = resolve_guild(st, args).await?;
        let mut thread_dumps: Vec<Value> = Vec::new();
        for t in fetch_threads(st, &guild_id, channel_id).await? {
            let tid = t["id"].as_str().unwrap_or_default().to_string();
            let tmsgs = fetch_messages(st, &tid, after, None, 0).await?;
            thread_dumps.push(json!({
                "id": t["id"], "name": t["name"],
                "archived": t["thread_metadata"]["archived"],
                "message_count": tmsgs.len(),
                "messages": tmsgs.iter().map(simplify_message).collect::<Vec<_>>()
            }));
        }
        if let Some(obj) = result.as_object_mut() {
            obj.insert("threads".into(), Value::Array(thread_dumps));
        }
    }

    if args["save_to_file"].as_bool().unwrap_or(false) {
        let path = spill_to_file(st, &format!("messages_{channel_id}"), &result)?;
        return Ok(json!({
            "saved_to": path.display().to_string(),
            "count": result["count"],
            "oldest": result["oldest"],
            "newest": result["newest"]
        }));
    }
    Ok(result)
}

async fn tool_list_threads(st: &McpState, args: &Value) -> Result<Value> {
    let channel_id = args["channel_id"]
        .as_str()
        .ok_or_else(|| anyhow!("channel_id fehlt"))?;
    let guild_id = resolve_guild(st, args).await?;
    let threads = fetch_threads(st, &guild_id, channel_id).await?;
    Ok(json!(threads
        .iter()
        .map(|t| json!({
            "id": t["id"], "name": t["name"],
            "archived": t["thread_metadata"]["archived"],
            "message_count": t["message_count"],
            "created": t["id"].as_str().and_then(snowflake_to_iso)
        }))
        .collect::<Vec<_>>()))
}

async fn tool_export_category(st: &McpState, args: &Value) -> Result<Value> {
    let category_arg = args["category"]
        .as_str()
        .ok_or_else(|| anyhow!("category fehlt"))?;
    let guild_id = resolve_guild(st, args).await?;
    let include_threads = args["include_threads"].as_bool().unwrap_or(true);
    let after = match args["after"].as_str() {
        Some(s) => Some(to_snowflake(s)?),
        None => None,
    };

    let channels = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}/channels"),
        &[],
        None,
        None,
    )
    .await?;
    let arr = channels.as_array().cloned().unwrap_or_default();

    let category = arr
        .iter()
        .find(|c| {
            c["type"].as_u64() == Some(4)
                && (c["id"].as_str() == Some(category_arg)
                    || c["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(category_arg)))
        })
        .or_else(|| {
            let needle = category_arg.to_lowercase();
            arr.iter().find(|c| {
                c["type"].as_u64() == Some(4)
                    && c["name"]
                        .as_str()
                        .is_some_and(|n| n.to_lowercase().contains(&needle))
            })
        })
        .ok_or_else(|| {
            let cats: Vec<String> = arr
                .iter()
                .filter(|c| c["type"].as_u64() == Some(4))
                .filter_map(|c| c["name"].as_str().map(|s| s.to_string()))
                .collect();
            anyhow!(
                "Kategorie '{category_arg}' nicht gefunden. Vorhandene Kategorien: {}",
                cats.join(", ")
            )
        })?;

    let cat_id = category["id"].as_str().unwrap_or_default().to_string();
    let cat_name = category["name"].as_str().unwrap_or("kategorie").to_string();

    // Nachrichtenfähige Kindkanäle: Text(0), Announcement(5), Forum(15), Media(16)
    let children: Vec<&Value> = arr
        .iter()
        .filter(|c| {
            c["parent_id"].as_str() == Some(cat_id.as_str())
                && matches!(c["type"].as_u64(), Some(0) | Some(5) | Some(15) | Some(16))
        })
        .collect();

    if children.is_empty() {
        bail!("Kategorie '{cat_name}' hat keine nachrichtenfähigen Channels");
    }

    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    let export_root = st
        .export_dir
        .join(format!("{stamp}_{}", sanitize_filename(&cat_name)));
    std::fs::create_dir_all(&export_root).context("Export-Verzeichnis anlegen")?;

    let mut summary_channels: Vec<Value> = Vec::new();
    let mut total_messages: usize = 0;

    for ch in &children {
        let ch_id = ch["id"].as_str().unwrap_or_default().to_string();
        let ch_name = ch["name"].as_str().unwrap_or("channel").to_string();
        let ch_type = ch["type"].as_u64().unwrap_or(0);
        tracing::info!(channel = %ch_name, id = %ch_id, "MCP-Export läuft");

        // Forum/Media-Channels haben Nachrichten nur in Threads.
        let messages: Vec<Value> = if matches!(ch_type, 15 | 16) {
            Vec::new()
        } else {
            fetch_messages(st, &ch_id, after, None, 0).await?
        };

        let mut thread_dumps: Vec<Value> = Vec::new();
        if include_threads {
            for t in fetch_threads(st, &guild_id, &ch_id).await? {
                let tid = t["id"].as_str().unwrap_or_default().to_string();
                let tmsgs = fetch_messages(st, &tid, after, None, 0).await?;
                total_messages += tmsgs.len();
                thread_dumps.push(json!({
                    "id": t["id"], "name": t["name"],
                    "archived": t["thread_metadata"]["archived"],
                    "message_count": tmsgs.len(),
                    "messages": tmsgs.iter().map(simplify_message).collect::<Vec<_>>()
                }));
            }
        }

        total_messages += messages.len();
        let dump = json!({
            "channel": {"id": ch["id"], "name": ch["name"], "type": ch_type, "topic": ch["topic"]},
            "message_count": messages.len(),
            "oldest": messages.first().and_then(|m| m["timestamp"].as_str()),
            "newest": messages.last().and_then(|m| m["timestamp"].as_str()),
            "messages": messages.iter().map(simplify_message).collect::<Vec<_>>(),
            "threads": thread_dumps
        });
        let path = export_root.join(format!("{}.json", sanitize_filename(&ch_name)));
        let file = std::fs::File::create(&path).context("Channel-Dump schreiben")?;
        serde_json::to_writer_pretty(file, &dump).context("JSON schreiben")?;

        summary_channels.push(json!({
            "name": ch["name"], "id": ch["id"],
            "messages": dump["message_count"],
            "threads": dump["threads"].as_array().map(|a| a.len()).unwrap_or(0),
            "file": path.display().to_string()
        }));
    }

    let summary = json!({
        "category": {"id": cat_id, "name": cat_name},
        "guild_id": guild_id,
        "exported_at": Utc::now().to_rfc3339(),
        "total_messages": total_messages,
        "export_dir": export_root.display().to_string(),
        "channels": summary_channels
    });
    let file = std::fs::File::create(export_root.join("summary.json")).context("summary.json")?;
    serde_json::to_writer_pretty(file, &summary).context("JSON schreiben")?;
    Ok(summary)
}

async fn tool_send_message(st: &McpState, args: &Value) -> Result<Value> {
    let channel_id = args["channel_id"]
        .as_str()
        .ok_or_else(|| anyhow!("channel_id fehlt"))?;
    let content = args["content"]
        .as_str()
        .ok_or_else(|| anyhow!("content fehlt"))?;
    let mut body = json!({ "content": content });
    if let Some(reply) = args["reply_to"].as_str() {
        body["message_reference"] = json!({ "message_id": reply, "fail_if_not_exists": false });
    }
    let msg = discord_call(
        st,
        "POST",
        &format!("/channels/{channel_id}/messages"),
        &[],
        Some(&body),
        None,
    )
    .await?;
    Ok(json!({"sent": true, "message_id": msg["id"], "channel_id": channel_id}))
}

async fn tool_search_members(st: &McpState, args: &Value) -> Result<Value> {
    let q = args["query"]
        .as_str()
        .ok_or_else(|| anyhow!("query fehlt"))?;
    let guild_id = resolve_guild(st, args).await?;
    let limit = args["limit"].as_u64().unwrap_or(10).min(1000);
    let res = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild_id}/members/search"),
        &[
            ("query".into(), q.into()),
            ("limit".into(), limit.to_string()),
        ],
        None,
        None,
    )
    .await?;
    Ok(json!(res
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|m| json!({
            "user_id": m["user"]["id"],
            "username": m["user"]["username"],
            "global_name": m["user"]["global_name"],
            "nick": m["nick"],
            "roles": m["roles"],
            "joined_at": m["joined_at"]
        }))
        .collect::<Vec<_>>()))
}

fn is_destructive(method: &str, path: &str) -> bool {
    if method.eq_ignore_ascii_case("DELETE") {
        return true;
    }
    let p = path.to_lowercase();
    p.contains("/bans/")
        || p.ends_with("/bans")
        || p.contains("/prune")
        || p.contains("bulk-delete")
}

async fn tool_api_call(st: &McpState, args: &Value) -> Result<Value> {
    let method = args["method"]
        .as_str()
        .ok_or_else(|| anyhow!("method fehlt"))?
        .to_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        bail!("Methode nicht erlaubt: {method}");
    }
    let path_raw = args["path"].as_str().ok_or_else(|| anyhow!("path fehlt"))?;
    let path = if path_raw.starts_with('/') {
        path_raw.to_string()
    } else {
        format!("/{path_raw}")
    };

    if path.contains("..") || path.starts_with("//") {
        bail!("Pfad unzulässig");
    }
    if is_destructive(&method, &path) && !args["confirm"].as_bool().unwrap_or(false) {
        bail!("Destruktiver Aufruf ({method} {path}) — erneut mit confirm=true aufrufen.");
    }

    let query: Vec<(String, String)> = args["query"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| v.to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let body = args.get("body").filter(|b| !b.is_null());
    let reason = args["reason"].as_str();

    let res = discord_call(st, &method, &path, &query, body, reason).await?;
    Ok(if res.is_null() {
        json!({"ok": true, "note": "204 No Content"})
    } else {
        res
    })
}
