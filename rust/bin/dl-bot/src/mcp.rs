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
//! Betriebswerte kommen aus der zentralen TOML, der bestehende
//! TWITCH_INTERNAL_API_TOKEN aus Infisical. Der Listener bindet nur Loopback.

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use dl_core::runtime_config::StartOptions;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

mod public;

const DISCORD_API: &str = "https://discord.com/api/v10";
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;
const PAGE_DELAY_MS: u64 = 450;
const MAX_INLINE_RESULT_CHARS: usize = 250_000;
const PROTOCOL_FALLBACK: &str = "2025-03-26";
const SUPPORTED_PROTOCOLS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

pub struct McpState {
    http: reqwest::Client,
    bot_token: String,
    discord_api: String,
    auth_token: String,
    public_token: Option<String>,
    public_policy: StartOptions,
    public_tempvoice: Option<Arc<dl_voice::tempvoice::TempVoiceEngine>>,
    default_guild: Option<String>,
    export_dir: PathBuf,
    public_source: Option<(Arc<dl_discord::DiscordAdapter>, sqlx::PgPool, u64)>,
    public_cache: tokio::sync::Mutex<Option<public::CachedFacts>>,
}

impl McpState {
    pub fn from_config(
        bot_token: String,
        auth_token: Option<String>,
        config: &StartOptions,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("MCP: reqwest-Client")?;
        let auth_token = auth_token
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .context("MCP: interner Auth-Token aus Infisical fehlt")?;
        Ok(Self {
            http,
            bot_token,
            discord_api: DISCORD_API.into(),
            auth_token,
            public_token: None,
            public_policy: config.clone(),
            public_tempvoice: None,
            default_guild: config.mcp_guild_id.map(|id| id.to_string()),
            export_dir: config
                .mcp_export_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from("data/mcp_exports")),
            public_source: None,
            public_cache: tokio::sync::Mutex::new(None),
        })
    }

    pub fn with_public_source(
        mut self,
        adapter: Arc<dl_discord::DiscordAdapter>,
        pool: sqlx::PgPool,
        guild_id: u64,
    ) -> Self {
        self.public_source = Some((adapter, pool, guild_id));
        self
    }

    pub fn bind_addr(config: &StartOptions) -> String {
        format!("127.0.0.1:{}", config.mcp_port.unwrap_or(8890))
    }

    pub fn with_public_access(
        mut self,
        token: Option<String>,
        engine: Arc<dl_voice::tempvoice::TempVoiceEngine>,
    ) -> Result<Self> {
        if let Some(token) = token {
            if token.trim().is_empty() || constant_time_eq(&token, &self.auth_token) {
                bail!("Öffentlicher MCP-Zugang benötigt einen eigenen Schlüssel");
            }
            self.public_token = Some(token);
        }
        self.public_tempvoice = Some(engine);
        Ok(self)
    }
}

pub fn router(state: Arc<McpState>) -> Router {
    Router::new()
        .route("/mcp", post(mcp_post).get(mcp_get))
        .route("/mcp/public", post(public_post))
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

fn auth_ok(st: &McpState, headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, token)| constant_time_eq(token.trim(), &st.auth_token))
        .unwrap_or(false)
}

fn public_auth(st: &McpState, headers: &HeaderMap) -> bool {
    st.public_token.as_ref().is_some_and(|expected| {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .is_some_and(|(scheme, token)| {
                scheme.eq_ignore_ascii_case("Bearer") && constant_time_eq(token.trim(), expected)
            })
    })
}

async fn public_post(
    State(st): State<Arc<McpState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !public_auth(&st, &headers) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error":"unauthorized"}));
    }
    let Ok(req) = serde_json::from_str::<Value>(&body) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"invalid_request"}));
    };
    let user = headers
        .get("x-discord-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|id| *id != 0);
    let result = match req["method"].as_str() {
        Some("tools/list") => {
            json!({"tools":[{"name":"public_server_facts"},{"name":"read_messages"},{"name":"send_message"}]})
        }
        Some("tools/call") if req["params"]["name"] == "public_server_facts" => {
            let Ok(access) = public::access(&st, user).await else {
                return json_response(StatusCode::FORBIDDEN, json!({"error":"forbidden"}));
            };
            match public::facts_for(&st, &req["params"]["arguments"], &access).await {
                Ok(facts) => {
                    json!({"content":[{"type":"text","text":serde_json::to_string(&facts).expect("Fakten sind serialisierbar")}],"isError":false})
                }
                Err(_) => {
                    return json_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        json!({"error":"public_facts_unavailable"}),
                    )
                }
            }
        }
        Some("tools/call")
            if matches!(
                req["params"]["name"].as_str(),
                Some("read_messages" | "send_message")
            ) =>
        {
            let request_id = headers
                .get("x-discord-request-id")
                .and_then(|v| v.to_str().ok())
                .filter(|id| !id.is_empty() && id.len() <= 128);
            if request_id.is_none() {
                return json_response(StatusCode::FORBIDDEN, json!({"error":"forbidden"}));
            }
            let Ok(access) = public::access(&st, user).await else {
                return json_response(StatusCode::FORBIDDEN, json!({"error":"forbidden"}));
            };
            match public::request_tool(
                &st,
                req["params"]["name"].as_str().unwrap_or_default(),
                &req["params"]["arguments"],
                &access,
            )
            .await
            {
                Ok(value) => {
                    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                }
                Err(_) => {
                    return json_response(StatusCode::FORBIDDEN, json!({"error":"forbidden"}))
                }
            }
        }
        _ => return json_response(StatusCode::FORBIDDEN, json!({"error":"forbidden"})),
    };
    json_response(
        StatusCode::OK,
        json!({"jsonrpc":"2.0","id":req["id"],"result":result}),
    )
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let left = Sha256::digest(a.as_bytes());
    let right = Sha256::digest(b.as_bytes());
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

async fn mcp_get(State(st): State<Arc<McpState>>, headers: HeaderMap) -> Response {
    if public_auth(&st, &headers) {
        return json_response(StatusCode::FORBIDDEN, json!({"error":"forbidden"}));
    }
    if !auth_ok(&st, &headers) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"}));
    }
    // Kein server-initiierter SSE-Stream nötig — Streamable HTTP erlaubt 405.
    json_response(
        StatusCode::METHOD_NOT_ALLOWED,
        json!({"error": "SSE-Stream nicht unterstützt"}),
    )
}

async fn mcp_post(State(st): State<Arc<McpState>>, headers: HeaderMap, body: String) -> Response {
    if public_auth(&st, &headers) {
        return json_response(StatusCode::FORBIDDEN, json!({"error":"forbidden"}));
    }
    if !auth_ok(&st, &headers) {
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
        "public_server_facts" => public::facts(st, &args)
            .await
            .and_then(|facts| serde_json::to_value(facts).map_err(Into::into)),
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
            "name": "public_server_facts",
            "description": "Aktuelle öffentliche Kanalstruktur, Topics, Voice-Anzahlen ohne Namen und registrierte Infotexte unseres Bots. Sichtbarkeit wird für everyone geprüft; Cache 60 Sekunden.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
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
    let url = format!("{}{path}", st.discord_api);
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn brain_zugang_prueft_rechte_an_der_http_grenze() {
        use serenity::all::{Cache, Guild, GuildId, Role, RoleId, Permissions};
        let adapter = dl_discord::DiscordAdapter::new("synthetic-bot");
        let cache = Arc::new(Cache::new());
        let mut guild = Guild::default();
        guild.id = GuildId::new(1);
        for id in [1, 3, 4] {
            let mut role = Role::default();
            role.id = RoleId::new(id);
            role.permissions = Permissions::READ_MESSAGE_HISTORY;
            guild.roles.insert(role.id, role);
        }
        for (id, overwrites) in [
            (10, json!([{"id":"3","type":0,"deny":"0","allow":"1024"}])),
            (11, json!([{"id":"4","type":0,"deny":"0","allow":"1024"}])),
            (12, json!([{"id":"42","type":1,"deny":"0","allow":"1024"}])),
        ] {
            let channel: serenity::all::GuildChannel = serde_json::from_value(json!({"id":id.to_string(),"guild_id":"1","name":"Kanal","type":0,"position":0,"permission_overwrites":overwrites})).expect("Testkanal");
            guild.channels.insert(channel.id, channel);
        }
        let mut event: serenity::all::GuildCreateEvent = serde_json::from_value(serde_json::to_value(guild).expect("Testguild")).expect("Testereignis");
        cache.update(&mut event);
        adapter.link_cache(cache);
        let mock = Router::new().fallback(|request: axum::extract::Request| async move {
            let path = request.uri().path();
            if path.ends_with("/members/42") { axum::Json(json!({"roles":["3","4"]})).into_response() }
            else if path.ends_with("/members/43") { axum::Json(json!({"roles":["3"]})).into_response() }
            else if path.contains("/members/") { StatusCode::NOT_FOUND.into_response() }
            else { axum::Json(json!([{"id":"100","content":"Nur für diese Antwort"}])).into_response() }
        });
        let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("Testlistener");
        let mock_addr = mock_listener.local_addr().expect("Testadresse");
        let mock_task = tokio::spawn(async move { axum::serve(mock_listener, mock).await.expect("Testserver"); });
        let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy("postgresql:///synthetic").expect("Testpool");
        let mut state = state(Some("general-only")).expect("Teststate").with_public_source(adapter, pool, 1);
        state.public_token = Some("brain-only".into());
        state.public_policy.mcp_verified_role_id = Some(3);
        state.discord_api = format!("http://{mock_addr}");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("Testlistener");
        let addr = listener.local_addr().expect("Testadresse");
        let task = tokio::spawn(async move { axum::serve(listener, router(Arc::new(state))).await.expect("Testserver"); });
        let client = reqwest::Client::new();
        for route in ["/mcp", "/mcp/public"] {
            for tool in ["api_call", "export_category", "search_members", "delete_message"] {
                let response = client.post(format!("http://{addr}{route}")).bearer_auth("brain-only").json(&json!({"id":1,"method":"tools/call","params":{"name":tool,"arguments":{}}})).send().await.expect("HTTP-Antwort");
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{route} {tool}");
            }
        }
        for (user, channel, tool, status) in [
            ("43",11,"read_messages",403), ("42",11,"read_messages",200),
            ("43",12,"read_messages",403), ("42",12,"read_messages",200),
            ("999",10,"read_messages",200), ("999",11,"read_messages",403),
            ("999",10,"send_message",403), ("43",10,"send_message",403),
        ] {
            let arguments = if tool == "send_message" { json!({"channel_id":channel,"content":"Test"}) } else { json!({"channel_id":channel}) };
            let response = client.post(format!("http://{addr}/mcp/public")).bearer_auth("brain-only").header("x-discord-user-id",user).header("x-discord-request-id","synthetic-request").json(&json!({"id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}})).send().await.expect("HTTP-Antwort");
            assert_eq!(response.status().as_u16(), status, "{user} {channel} {tool}");
        }
        let response = client.post(format!("http://{addr}/mcp")).bearer_auth("brain-only").json(&json!({"id":1,"method":"tools/call","params":{"name":"read_messages","arguments":{"channel_id":10}}})).send().await.expect("HTTP-Antwort");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        task.abort();
        mock_task.abort();
    }

    fn state(token: Option<&str>) -> Result<McpState> {
        McpState::from_config(
            "bot-token".into(),
            token.map(str::to_owned),
            &StartOptions::default(),
        )
    }

    #[test]
    fn fehlender_oder_leerer_token_sperrt_den_mcp_start() {
        assert!(state(None).is_err());
        assert!(state(Some("  ")).is_err());
    }

    #[test]
    fn typed_toml_options_steuern_nur_port_guild_und_exportpfad() {
        let config = StartOptions {
            mcp_host: Some("0.0.0.0".parse().expect("gültige Test-IP")),
            mcp_port: Some(8891),
            mcp_guild_id: Some(12345),
            mcp_export_dir: Some(PathBuf::from("data/test-exports")),
            ..Default::default()
        };
        let state = McpState::from_config("bot-token".into(), Some(" expected ".into()), &config)
            .expect("gültiger Test-State");
        assert_eq!(state.auth_token, "expected");
        assert_eq!(state.default_guild.as_deref(), Some("12345"));
        assert_eq!(state.export_dir, PathBuf::from("data/test-exports"));
        assert_eq!(McpState::bind_addr(&config), "127.0.0.1:8891");
    }

    #[test]
    fn nur_bearer_header_mit_richtigem_token_wird_akzeptiert() {
        let state = state(Some("expected")).expect("gültiger Test-State");
        let mut headers = HeaderMap::new();
        assert!(!auth_ok(&state, &headers));
        for value in [
            "Basic expected",
            "Bearer wrong",
            "Bearer much-longer-than-expected",
        ] {
            headers.insert(
                header::AUTHORIZATION,
                value.parse().expect("gültiger Header"),
            );
            assert!(!auth_ok(&state, &headers));
        }
        headers.insert(
            header::AUTHORIZATION,
            "Bearer expected".parse().expect("gültiger Header"),
        );
        assert!(auth_ok(&state, &headers));
    }

    #[tokio::test]
    async fn query_token_wird_verworfen_und_auth_laeuft_vor_json_parse() {
        let app = router(Arc::new(
            state(Some("expected")).expect("gültiger Test-State"),
        ));
        let response = app
            .clone()
            .oneshot(
                Request::get("/mcp?token=expected")
                    .body(Body::empty())
                    .expect("gültige Test-Request"),
            )
            .await
            .expect("gültige Test-Response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .clone()
            .oneshot(
                Request::post("/mcp?token=expected")
                    .body(Body::from("kein json"))
                    .expect("gültige Test-Request"),
            )
            .await
            .expect("gültige Test-Response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer expected")
                    .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
                    .expect("gültige Test-Request"),
            )
            .await
            .expect("gültige Test-Response");
        assert_eq!(response.status(), StatusCode::OK);
    }
}
