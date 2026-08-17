//! Steuert die schon eingeloggte Brave-Sitzung über CDP.
//! Kein zweites Profil, kein User-Token, kein Origin-Header
//! (Origin löst bei Brave 151 einen 403 aus).

use std::net::TcpStream as StdTcp;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const DEFAULT_PORT_FILE: &str =
    "/home/nathanael/.config/BraveSoftware/Brave-Browser/DevToolsActivePort";
const DEFAULT_BRAVE_BIN: &str = "/usr/bin/brave-browser-stable";
const DEFAULT_XAUTH: &str = "/home/nathanael/.Xauthority";
const GROWTH_PATH: &str = "/analytics/growth-activation";
const ENGAGEMENT_PATH: &str = "/analytics/engagement";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct BraveDump {
    pub stem: &'static str,
    pub json: Value,
}

pub async fn dump_insights_pages(guild_id: i64) -> Result<Vec<BraveDump>> {
    ensure_brave().await?;
    let mut ws = connect_browser().await?;
    let mut next_id = 1u64;

    let start = chrono::Utc::now() - chrono::Duration::days(119);
    let end = chrono::Utc::now();
    let start_s = start.format("%Y-%m-%d");
    let end_s = end.format("%Y-%m-%d");
    let pages = [
        (
            "growth",
            format!(
                "https://discord.com/developers/servers/{guild_id}{GROWTH_PATH}?interval=2&start={start_s}&end={end_s}"
            ),
        ),
        (
            "engagement",
            format!(
                "https://discord.com/developers/servers/{guild_id}{ENGAGEMENT_PATH}?interval=2&start={start_s}&end={end_s}"
            ),
        ),
    ];
    let first_url = pages[0].1.as_str();
    let target_id = create_page(&mut ws, &mut next_id, first_url).await?;
    let session_id = attach(&mut ws, &mut next_id, &target_id).await?;
    let _ = call(
        &mut ws,
        &mut next_id,
        "Page.enable",
        json!({}),
        Some(&session_id),
    )
    .await;

    let mut dumps = Vec::new();
    for (stem, url) in pages {
        call(
            &mut ws,
            &mut next_id,
            "Page.navigate",
            json!({ "url": url }),
            Some(&session_id),
        )
        .await?;
        wait_for_charts(&mut ws, &mut next_id, &session_id).await?;
        let dump = evaluate_dump(&mut ws, &mut next_id, &session_id).await?;
        dumps.push(BraveDump { stem, json: dump });
    }
    let _ = call(
        &mut ws,
        &mut next_id,
        "Target.closeTarget",
        json!({ "targetId": target_id }),
        None,
    )
    .await;
    close_inspect_tabs(&mut ws, &mut next_id).await;
    Ok(dumps)
}

pub async fn smoke() -> Result<String> {
    ensure_brave().await?;
    let mut ws = connect_browser().await?;
    let mut next_id = 1u64;
    let version = call(&mut ws, &mut next_id, "Browser.getVersion", json!({}), None).await?;
    let product = version
        .pointer("/result/product")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    Ok(product)
}

pub fn write_dumps(dumps: &[BraveDump], dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut paths = Vec::new();
    for dump in dumps {
        let path = dir.join(format!("{}.json", dump.stem));
        std::fs::write(&path, serde_json::to_vec_pretty(&dump.json)?)?;
        paths.push(path);
    }
    Ok(paths)
}

async fn ensure_brave() -> Result<()> {
    if explicit_ws_override().is_some() || json_version_ready() {
        return Ok(());
    }
    if browser_ws_endpoint().is_ok() {
        return Ok(());
    }
    if !start_enabled() {
        return Err(anyhow!(
            "Brave-Port-Datei fehlt und INSIGHTS_BRAVE_START ist aus"
        ));
    }
    if default_brave_running() {
        return Err(anyhow!(
            "Brave laeuft, aber DevToolsActivePort fehlt. Remote Debugging unter brave://inspect anschalten."
        ));
    }
    start_default_brave()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while std::time::Instant::now() < deadline {
        if browser_ws_endpoint().is_ok() {
            tracing::info!("Brave mit Remote-Debugging gestartet");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Err(anyhow!(
        "Brave gestartet, aber DevToolsActivePort kam nicht. DISPLAY={} pruefen.",
        detect_display()
    ))
}

fn start_enabled() -> bool {
    !matches!(
        std::env::var("INSIGHTS_BRAVE_START").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

fn start_default_brave() -> Result<()> {
    let bin = std::env::var("INSIGHTS_BRAVE_BIN").unwrap_or_else(|_| DEFAULT_BRAVE_BIN.to_string());
    let port = port_from_env_or_file().unwrap_or(9222);
    let x_display = detect_display();
    let xauth = std::env::var("XAUTHORITY").unwrap_or_else(|_| DEFAULT_XAUTH.to_string());
    tracing::info!(bin = %bin, port, x_display = %x_display, "starte Brave");
    Command::new(&bin)
        .args([
            "--password-store=basic",
            &format!("--remote-debugging-port={port}"),
            "--remote-allow-origins=*",
            "--no-first-run",
            "--hide-crash-restore-bubble",
        ])
        .env("DISPLAY", x_display)
        .env("XAUTHORITY", xauth)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Brave starten: {bin}"))?;
    Ok(())
}

fn default_brave_running() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in dir.flatten() {
        let pid = entry.file_name();
        if !pid
            .to_str()
            .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let text = String::from_utf8_lossy(&cmdline);
        let is_brave =
            text.contains("/opt/brave.com/brave/brave") || text.contains("brave-browser-stable");
        if is_brave && !text.contains("--type=") && !text.contains("--user-data-dir=") {
            return true;
        }
    }
    false
}

fn detect_display() -> String {
    if let Ok(display) = std::env::var("INSIGHTS_BRAVE_DISPLAY") {
        if !display.is_empty() {
            return display;
        }
    }
    if let Ok(display) = std::env::var("DISPLAY") {
        if !display.is_empty() {
            return display;
        }
    }
    if let Ok(dir) = std::fs::read_dir("/tmp/.X11-unix") {
        let mut sockets: Vec<String> = dir
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with('X'))
            .collect();
        sockets.sort();
        if let Some(name) = sockets.first() {
            return format!(":{}", name.trim_start_matches('X'));
        }
    }
    ":10".to_string()
}

fn port_file() -> PathBuf {
    std::env::var("INSIGHTS_BRAVE_PORT_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_PORT_FILE))
}

fn port_from_env_or_file() -> Option<u16> {
    if let Ok(raw) = std::env::var("INSIGHTS_BRAVE_PORT") {
        if let Ok(port) = raw.parse() {
            return Some(port);
        }
    }
    parse_port_file(&port_file()).ok().map(|(port, _)| port)
}

fn parse_port_file(path: &Path) -> Result<(u16, String)> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Brave-Port-Datei fehlt: {}", path.display()))?;
    parse_port_file_text(&raw)
}

fn parse_port_file_text(raw: &str) -> Result<(u16, String)> {
    let mut lines = raw.lines().map(str::trim).filter(|l| !l.is_empty());
    let port = lines
        .next()
        .ok_or_else(|| anyhow!("kein Port in DevToolsActivePort"))?
        .parse::<u16>()
        .context("Port in DevToolsActivePort ungueltig")?;
    let route = lines.next().unwrap_or("/devtools/browser").to_string();
    Ok((port, route))
}

fn explicit_ws_override() -> Option<String> {
    std::env::var("INSIGHTS_BRAVE_WS")
        .ok()
        .filter(|url| !url.is_empty())
}

fn json_version_ready() -> bool {
    port_from_env_or_file().and_then(json_version_ws).is_some()
}

fn browser_ws_endpoint() -> Result<String> {
    if let Some(url) = explicit_ws_override() {
        return Ok(url);
    }
    if let Ok(raw) = std::env::var("INSIGHTS_BRAVE_PORT") {
        let port: u16 = raw.parse().context("INSIGHTS_BRAVE_PORT ungueltig")?;
        return json_version_ws(port).ok_or_else(|| {
            anyhow!("kein webSocketDebuggerUrl unter http://127.0.0.1:{port}/json/version")
        });
    }
    let (port, route) = parse_port_file(&port_file())?;
    if let Some(url) = json_version_ws(port) {
        return Ok(url);
    }
    Ok(format!("ws://127.0.0.1:{port}{route}"))
}

fn json_version_ws(port: u16) -> Option<String> {
    use std::io::{Read, Write};
    let mut stream = StdTcp::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(400)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .ok()?;
    write!(
        stream,
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let _ = stream.flush();
    let mut raw = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => return None,
        }
        if raw.windows(4).any(|w| w == b"\r\n\r\n") && raw.contains(&b'{') {
            break;
        }
    }
    ws_url_from_version_http(&String::from_utf8_lossy(&raw))
}

fn ws_url_from_version_http(raw: &str) -> Option<String> {
    let body = raw
        .split("\r\n\r\n")
        .nth(1)
        .or_else(|| raw.split("\n\n").nth(1))?;
    let json: Value = serde_json::from_str(body.trim()).ok()?;
    json.get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn ws_request(endpoint: &str) -> Result<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut request = endpoint
        .into_client_request()
        .context("CDP-URL ungueltig")?;
    request.headers_mut().remove("Origin");
    Ok(request)
}

async fn connect_browser() -> Result<Ws> {
    let endpoint = browser_ws_endpoint()?;
    tracing::info!(%endpoint, "Brave CDP");
    if port_from_env_or_file().and_then(json_version_ws).is_none() {
        if let Err(err) = tokio::task::spawn_blocking(crate::allow::open_inspect_tab)
            .await
            .map_err(|err| anyhow!("Inspect-Tab Task: {err}"))
            .and_then(|r| r)
        {
            tracing::warn!(error = %err, "Inspect-Tab öffnen fehlgeschlagen");
        }
    }
    match handshake_with_keys(&endpoint, &["Return"]).await {
        Ok(ws) => Ok(ws),
        Err(first) => {
            tracing::info!(error = %first, "erster Allow-Versuch, jetzt Tab+Return");
            handshake_with_keys(&endpoint, &["Tab", "Return"]).await
        }
    }
}

async fn handshake_with_keys(endpoint: &str, keys: &[&str]) -> Result<Ws> {
    let request = ws_request(endpoint)?;
    let keys = keys.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let click = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let clicked = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            crate::allow::confirm_allow(&refs)
        })
        .await;
        if let Ok(Err(err)) = clicked {
            tracing::warn!(error = %err, "Allow-Klick fehlgeschlagen");
        }
    });
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(request),
    )
    .await;
    click.abort();
    match result {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(err)) => {
            Err(err).with_context(|| format!("Brave nicht erreichbar unter {endpoint}"))
        }
        Err(_) => Err(anyhow!(
            "CDP-Handshake Timeout unter {endpoint}. Allow-Klick hat nicht gegriffen oder ein anderer DevTools-Client haelt den Socket."
        )),
    }
}

async fn create_page(ws: &mut Ws, next_id: &mut u64, url: &str) -> Result<String> {
    let created = call(
        ws,
        next_id,
        "Target.createTarget",
        json!({ "url": url }),
        None,
    )
    .await?;
    created
        .pointer("/result/targetId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Target.createTarget ohne targetId"))
}

async fn close_inspect_tabs(ws: &mut Ws, next_id: &mut u64) {
    let Ok(now) = call(ws, next_id, "Target.getTargets", json!({}), None).await else {
        return;
    };
    for target in now
        .pointer("/result/targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = target.get("targetId").and_then(Value::as_str) else {
            continue;
        };
        let url = target.get("url").and_then(Value::as_str).unwrap_or("");
        if url.contains("://inspect") {
            let _ = call(
                ws,
                next_id,
                "Target.closeTarget",
                json!({ "targetId": id }),
                None,
            )
            .await;
        }
    }
}

async fn attach(ws: &mut Ws, next_id: &mut u64, target_id: &str) -> Result<String> {
    let attached = call(
        ws,
        next_id,
        "Target.attachToTarget",
        json!({ "targetId": target_id, "flatten": true }),
        None,
    )
    .await?;
    attached
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("attachToTarget ohne sessionId"))
}

async fn wait_for_charts(ws: &mut Ws, next_id: &mut u64, session_id: &str) -> Result<()> {
    for _ in 0..30 {
        let ready = evaluate(
            ws,
            next_id,
            session_id,
            "(function(){ var hc = window._Highcharts; return !!(hc && hc.charts && hc.charts.filter(Boolean).length); })()",
        )
        .await
        .unwrap_or(Value::Bool(false));
        if ready.as_bool() == Some(true) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(anyhow!(
        "Insights-Seite hat nach 30s keine Charts. Brave muss in Discord eingeloggt sein."
    ))
}

async fn evaluate_dump(ws: &mut Ws, next_id: &mut u64, session_id: &str) -> Result<Value> {
    let expr = r#"(function(){
      var iso = function(x){ return typeof x === 'number' ? new Date(x).toISOString() : String(x); };
      var hc = window._Highcharts;
      var charts = (hc && hc.charts) ? hc.charts.filter(Boolean) : [];
      var dumped = charts.map(function(c, i){
        return { i: i, series: (c.series || []).map(function(s){
          return { name: s.name, points: (s.points || []).map(function(p){
            return { x: iso(p.x), y: p.y };
          })};
        })};
      });
      var links = Array.prototype.map.call(document.querySelectorAll('a[href*="discord.gg"]'), function(a){
        var n = a.parentElement ? a.parentElement.innerText : a.innerText;
        return { href: a.href, text: n };
      });
      return { href: location.href, dumped: dumped, invites: links };
    })()"#;
    evaluate(ws, next_id, session_id, expr).await
}

async fn evaluate(
    ws: &mut Ws,
    next_id: &mut u64,
    session_id: &str,
    expression: &str,
) -> Result<Value> {
    let result = call(
        ws,
        next_id,
        "Runtime.evaluate",
        json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": false
        }),
        Some(session_id),
    )
    .await?;
    if let Some(err) = result.pointer("/result/exceptionDetails") {
        return Err(anyhow!("Runtime.evaluate: {err}"));
    }
    Ok(result
        .pointer("/result/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

async fn call(
    ws: &mut Ws,
    next_id: &mut u64,
    method: &str,
    params: Value,
    session_id: Option<&str>,
) -> Result<Value> {
    let id = *next_id;
    *next_id += 1;
    let mut msg = json!({ "id": id, "method": method, "params": params });
    if let Some(session) = session_id {
        msg["sessionId"] = Value::String(session.to_string());
    }
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .context("CDP senden")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let leftover = deadline.saturating_duration_since(tokio::time::Instant::now());
        if leftover.is_zero() {
            return Err(anyhow!("CDP Timeout auf {method}"));
        }
        let next = tokio::time::timeout(leftover, ws.next())
            .await
            .map_err(|_| anyhow!("CDP Timeout auf {method}"))?
            .ok_or_else(|| anyhow!("Brave hat die CDP-Verbindung geschlossen"))?
            .context("CDP lesen")?;
        let text = match next {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
            Message::Ping(p) => {
                let _ = ws.send(Message::Pong(p)).await;
                continue;
            }
            Message::Close(_) => return Err(anyhow!("Brave hat CDP geschlossen")),
            _ => continue,
        };
        tracing::debug!(method, incoming = %text.chars().take(240).collect::<String>(), "CDP");
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let reply_id = parsed.get("id").and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        });
        if reply_id == Some(id) {
            return Ok(parsed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_port_file_text, ws_request, ws_url_from_version_http};

    #[test]
    fn ws_url_aus_port_datei() {
        let (port, route) = parse_port_file_text("9222\n/devtools/browser/abc\n").expect("parse");
        assert_eq!(
            format!("ws://127.0.0.1:{port}{route}"),
            "ws://127.0.0.1:9222/devtools/browser/abc"
        );
    }

    #[test]
    fn handshake_setzt_keinen_origin() {
        let request = ws_request("ws://127.0.0.1:9333/devtools/browser/abc").expect("req");
        assert!(request.headers().get("Origin").is_none());
    }

    #[test]
    fn json_version_body_liefert_ws_url() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"webSocketDebuggerUrl\":\"ws://127.0.0.1:9333/devtools/browser/abc\"}\n";
        assert_eq!(
            ws_url_from_version_http(raw).expect("url"),
            "ws://127.0.0.1:9333/devtools/browser/abc"
        );
    }
}
