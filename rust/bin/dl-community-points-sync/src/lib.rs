//! Sync der Community-Punkte aus dem Twitch-Bot (interne API, Paket B) in die
//! zentrale Deadlock-DB (Community-Streamer-Bruecke, Paket C).
//!
//! - `GET /internal/twitch/v1/community-points/viewers` und `/streamers`
//!   seitenweise ab dem gespeicherten Cursor (`updated_since`, exklusiv). Der
//!   Cursor `next_updated_since` wird exakt so gespeichert, wie er kommt, und
//!   zusammen mit der Seite in einer Transaktion geschrieben.
//! - Danach Ledger-Importe aus der zentralen DB: qualifizierte Beitritte ueber
//!   Streamer-Einladungen und, sobald die Tabellen von Paket D existieren,
//!   Clip-Contest-Plaetze und -Stimmen.
//!
//! URL aus `runtime.bridges.twitch_api_url`, nur numerisches Loopback, keine
//! Redirects, Token `TWITCH_INTERNAL_API_TOKEN` aus dem Infisical-Bootstrap
//! (wie `dl-twitch-invite-sync`).

use chrono::{DateTime, NaiveDate, Utc};
use dl_central_db::community_points::{
    apply_streamer_page, apply_viewer_page, load_cursor, StreamerDailyRow, ViewerDailyRow,
    CURSOR_STREAMERS, CURSOR_VIEWERS,
};
use dl_central_db::platform_connections::is_valid_twitch_user_id;
use serde::Deserialize;
use sqlx::PgPool;

/// Standard-URL des internen Twitch-API-Servers.
pub const DEFAULT_TWITCH_API_URL: &str = "http://127.0.0.1:8776";
/// Seitengroesse je Abruf (Quelle erlaubt 1..5000).
pub const PAGE_LIMIT: u32 = 1000;
/// Obergrenze Seiten je Quelle und Lauf (Schutz gegen Endlosschleifen).
pub const MAX_PAGES_PER_RUN: usize = 500;

pub const VIEWERS_PATH: &str = "/internal/twitch/v1/community-points/viewers";
pub const STREAMERS_PATH: &str = "/internal/twitch/v1/community-points/streamers";

/// Prueft die Basis-URL: http(s), numerische Loopback-Adresse, ohne
/// Zugangsdaten, Pfad, Query oder Fragment.
pub fn validate_base_url(base: &str) -> anyhow::Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(base)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        })
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        anyhow::bail!(
            "Community-Punkte-Sync benötigt eine numerische Loopback-Adresse ohne Zugangsdaten oder Pfad"
        );
    }
    Ok(parsed)
}

/// HTTP-Client wie beim Invite-Sync: kein Proxy, keine Redirects.
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

// ─── Wire-Format (PLAN, Paket B) ────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WirePage<T> {
    pub rows: Vec<T>,
    pub next_updated_since: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ViewerWire {
    pub twitch_user_id: String,
    pub channel_twitch_user_id: String,
    pub day: String,
    pub watch_minutes: i64,
    pub chat_messages: i64,
    pub points_watch: i64,
    pub points_chat: i64,
    pub points_discovery: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamerWire {
    pub streamer_twitch_user_id: String,
    pub streamer_login: String,
    #[serde(default)]
    pub discord_user_id: Option<String>,
    pub day: String,
    pub viewer_minutes: i64,
    pub unique_viewers: i64,
    pub raids_to_partners: i64,
    pub updated_at: String,
}

fn count(value: i64) -> Option<i32> {
    i32::try_from(value).ok().filter(|v| *v >= 0)
}

fn day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()
}

fn timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

fn twitch_id(value: &str) -> Option<String> {
    let value = value.trim();
    is_valid_twitch_user_id(value).then(|| value.to_string())
}

/// Prueft eine Zuschauerzeile. Ungueltige Zeilen werden uebersprungen (der
/// Cursor laeuft trotzdem weiter, sonst haengt der Sync an einer Zeile fest).
pub fn viewer_row(wire: &ViewerWire) -> Option<ViewerDailyRow> {
    Some(ViewerDailyRow {
        twitch_user_id: twitch_id(&wire.twitch_user_id)?,
        channel_twitch_user_id: twitch_id(&wire.channel_twitch_user_id)?,
        day: day(&wire.day)?,
        watch_minutes: count(wire.watch_minutes)?,
        chat_messages: count(wire.chat_messages)?,
        points_watch: count(wire.points_watch)?,
        points_chat: count(wire.points_chat)?,
        points_discovery: count(wire.points_discovery)?,
        source_updated_at: timestamp(&wire.updated_at)?,
    })
}

/// Prueft eine Streamerzeile. Eine ungueltige Discord-ID wird zu `None`.
pub fn streamer_row(wire: &StreamerWire) -> Option<StreamerDailyRow> {
    let login = wire.streamer_login.trim();
    if login.is_empty() {
        return None;
    }
    Some(StreamerDailyRow {
        streamer_twitch_user_id: twitch_id(&wire.streamer_twitch_user_id)?,
        day: day(&wire.day)?,
        streamer_login: login.to_string(),
        discord_user_id: wire
            .discord_user_id
            .as_deref()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            .filter(|id| *id > 0),
        viewer_minutes: count(wire.viewer_minutes)?,
        unique_viewers: count(wire.unique_viewers)?,
        raids_to_partners: count(wire.raids_to_partners)?,
        source_updated_at: timestamp(&wire.updated_at)?,
    })
}

// ─── Seitenlogik ────────────────────────────────────────────────────────────

/// Ziel einer Quelle: Cursor lesen, Seite samt Folge-Cursor atomar schreiben.
pub trait PageSink<R> {
    fn cursor(&self) -> impl std::future::Future<Output = anyhow::Result<Option<String>>>;
    fn apply(
        &self,
        rows: &[R],
        next_cursor: Option<&str>,
    ) -> impl std::future::Future<Output = anyhow::Result<u64>>;
}

/// Zentrale DB als Ziel fuer Zuschauerzeilen.
pub struct ViewerDbSink<'a>(pub &'a PgPool);
/// Zentrale DB als Ziel fuer Streamerzeilen.
pub struct StreamerDbSink<'a>(pub &'a PgPool);

impl PageSink<ViewerDailyRow> for ViewerDbSink<'_> {
    async fn cursor(&self) -> anyhow::Result<Option<String>> {
        Ok(load_cursor(self.0, CURSOR_VIEWERS).await?)
    }
    async fn apply(&self, rows: &[ViewerDailyRow], next: Option<&str>) -> anyhow::Result<u64> {
        Ok(apply_viewer_page(self.0, rows, next).await?)
    }
}

impl PageSink<StreamerDailyRow> for StreamerDbSink<'_> {
    async fn cursor(&self) -> anyhow::Result<Option<String>> {
        Ok(load_cursor(self.0, CURSOR_STREAMERS).await?)
    }
    async fn apply(&self, rows: &[StreamerDailyRow], next: Option<&str>) -> anyhow::Result<u64> {
        Ok(apply_streamer_page(self.0, rows, next).await?)
    }
}

/// Ergebnis einer Quelle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceSummary {
    pub pages: usize,
    pub fetched: usize,
    pub skipped: usize,
    pub written: u64,
    pub cursor: Option<String>,
}

async fn fetch_page<W: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &reqwest::Url,
    token: &str,
    cursor: Option<&str>,
) -> anyhow::Result<WirePage<W>> {
    let limit = PAGE_LIMIT.to_string();
    let mut query: Vec<(&str, &str)> = vec![("limit", limit.as_str())];
    if let Some(cursor) = cursor {
        query.push(("updated_since", cursor));
    }
    let page = client
        .get(url.clone())
        .query(&query)
        .header("X-Internal-Token", token)
        .send()
        .await?
        .error_for_status()?
        .json::<WirePage<W>>()
        .await?;
    Ok(page)
}

/// Liest eine Quelle seitenweise ab dem gespeicherten Cursor, bis `has_more`
/// falsch ist. Jede Seite wird mit ihrem Folge-Cursor atomar gespeichert;
/// bricht der Lauf ab, setzt der naechste an der letzten gespeicherten Seite an.
pub async fn sync_source<W, R, S>(
    client: &reqwest::Client,
    url: &reqwest::Url,
    token: &str,
    sink: &S,
    validate: impl Fn(&W) -> Option<R>,
) -> anyhow::Result<SourceSummary>
where
    W: serde::de::DeserializeOwned,
    S: PageSink<R>,
{
    let mut summary = SourceSummary {
        cursor: sink.cursor().await?,
        ..SourceSummary::default()
    };
    loop {
        if summary.pages >= MAX_PAGES_PER_RUN {
            anyhow::bail!("mehr als {MAX_PAGES_PER_RUN} Seiten in einem Lauf, Abbruch");
        }
        let page: WirePage<W> = fetch_page(client, url, token, summary.cursor.as_deref()).await?;
        summary.pages += 1;
        summary.fetched += page.rows.len();
        let rows: Vec<R> = page.rows.iter().filter_map(&validate).collect();
        summary.skipped += page.rows.len() - rows.len();
        let next = page.next_updated_since.clone();
        if page.has_more && (next.is_none() || next == summary.cursor) {
            anyhow::bail!("Quelle meldet weitere Seiten ohne neuen Cursor, Abbruch");
        }
        summary.written += sink.apply(&rows, next.as_deref()).await?;
        if next.is_some() {
            summary.cursor = next;
        }
        if !page.has_more {
            return Ok(summary);
        }
    }
}

/// Endpunkt-URL aus Basis und Pfad.
pub fn endpoint(base: &reqwest::Url, path: &str) -> anyhow::Result<reqwest::Url> {
    Ok(base.join(path)?)
}

#[cfg(test)]
mod tests;
