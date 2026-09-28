use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Berlin;
use dl_community::concierge::ALLGEMEIN_CHANNEL_ID;
use dl_discord::DiscordAdapter;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;

pub const DEVFEED_API_BASE: &str = "http://127.0.0.1:8789";
const POLL: Duration = Duration::from_secs(3);
const PUBLIC_LIMIT: u16 = 30;
const LOOKBACK: chrono::Duration = chrono::Duration::hours(24);
const MAX_DISCORD_FILE: usize = 8 * 1024 * 1024;
const V2_FLAG: u64 = 1 << 15;
const GOLD: u32 = 0xC8A86B;

#[derive(Debug, Deserialize)]
struct ListResponse {
    items: Vec<PublicMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicMessage {
    message_id: String,
    author_name: String,
    content: String,
    sent_at: Option<String>,
    caught_at: String,
    jump_url: String,
    #[serde(default)]
    media: Vec<PublicMedia>,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicMedia {
    attachment_id: String,
    content_type: String,
    filename: String,
    byte_size: i32,
    url: String,
}

pub fn spawn(adapter: Arc<DiscordAdapter>, pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "DevFeed-Spiegel: HTTP-Client fehlt");
                return;
            }
        };
        let mut ticks = tokio::time::interval(POLL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            if let Err(error) = poll_once(&adapter, &pool, &client).await {
                tracing::warn!(%error, "DevFeed-Spiegel: Durchlauf fehlgeschlagen");
            }
        }
    })
}

async fn poll_once(
    adapter: &DiscordAdapter,
    pool: &PgPool,
    client: &reqwest::Client,
) -> Result<(), String> {
    let url = format!("{DEVFEED_API_BASE}/public/v1/messages?limit={PUBLIC_LIMIT}");
    let page = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<ListResponse>()
        .await
        .map_err(|error| error.to_string())?;
    let cutoff = Utc::now() - LOOKBACK;
    for item in page.items.into_iter().rev() {
        if already_known(pool, &item.message_id).await? {
            continue;
        }
        let stamp = parse_stamp(item.sent_at.as_deref(), &item.caught_at);
        if stamp < cutoff {
            mark(pool, &item.message_id, 0, true).await?;
            continue;
        }
        match publish(adapter, client, &item).await {
            Ok(discord_id) => {
                mark(pool, &item.message_id, discord_id, false).await?;
                tracing::info!(
                    source = %item.message_id,
                    discord = discord_id,
                    "DevFeed-Spiegel: Nachricht in #Allgemein"
                );
            }
            Err(error) => {
                tracing::warn!(source = %item.message_id, %error, "DevFeed-Spiegel: Versand fehlgeschlagen");
            }
        }
    }
    Ok(())
}

async fn already_known(pool: &PgPool, source: &str) -> Result<bool, String> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM bot.devfeed_mirror WHERE source_message_id = $1)",
    )
    .bind(source)
    .fetch_one(pool)
    .await
    .map_err(|error| error.to_string())
}

async fn mark(pool: &PgPool, source: &str, discord_id: u64, skipped: bool) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO bot.devfeed_mirror (source_message_id, discord_message_id, skipped)
         VALUES ($1, $2, $3)
         ON CONFLICT (source_message_id) DO NOTHING",
    )
    .bind(source)
    .bind(discord_id as i64)
    .bind(skipped)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn publish(
    adapter: &DiscordAdapter,
    client: &reqwest::Client,
    item: &PublicMessage,
) -> Result<u64, String> {
    let mut files = Vec::new();
    let mut names = Vec::new();
    for media in &item.media {
        if media.byte_size <= 0 || media.byte_size as usize > MAX_DISCORD_FILE {
            continue;
        }
        let bytes = download_media(client, &media.url).await?;
        if bytes.is_empty() || bytes.len() > MAX_DISCORD_FILE {
            continue;
        }
        let filename = discord_filename(&media.attachment_id, &media.content_type, &media.filename);
        names.push(filename.clone());
        files.push((filename, bytes));
    }
    let when = parse_stamp(item.sent_at.as_deref(), &item.caught_at);
    let container = build_mirror_components(
        &item.content,
        &item.author_name,
        &item.jump_url,
        when,
        &names,
    );
    let mut payload = Map::new();
    payload.insert("flags".into(), json!(V2_FLAG));
    payload.insert("allowed_mentions".into(), json!({ "parse": [] }));
    payload.insert("components".into(), json!([container]));
    adapter
        .send_components_with_files(ALLGEMEIN_CHANNEL_ID, &payload, files)
        .await
        .map_err(|error| error.to_string())
}

async fn download_media(client: &reqwest::Client, path: &str) -> Result<Vec<u8>, String> {
    let url = format!("{DEVFEED_API_BASE}{path}");
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    Ok(response
        .bytes()
        .await
        .map_err(|error| error.to_string())?
        .to_vec())
}

fn parse_stamp(sent_at: Option<&str>, caught_at: &str) -> DateTime<Utc> {
    sent_at
        .and_then(parse_rfc3339)
        .or_else(|| parse_rfc3339(caught_at))
        .unwrap_or_else(Utc::now)
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn discord_filename(attachment_id: &str, content_type: &str, original: &str) -> String {
    let ext = match content_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        _ => original.rsplit('.').next().unwrap_or("bin"),
    };
    format!("{attachment_id}.{ext}")
}

fn mirror_body(item: &PublicMessage) -> Value {
    let content = sanitize_mentions(item.content.trim());
    let text = if content.is_empty() {
        "(kein Text)".to_string()
    } else {
        content
    };
    let when = parse_stamp(item.sent_at.as_deref(), &item.caught_at);
    let local = when.with_timezone(&Berlin).format("%d.%m.%Y %H:%M");
    let author = sanitize_mentions(&item.author_name);
    json!({
        "type": 17,
        "accent_color": GOLD,
        "spoiler": false,
        "components": [
            {
                "type": 10,
                "content": format!("**{author}** (Spiegelung)\n{text}")
            },
            {
                "type": 10,
                "content": format!("{local} · [Originalnachricht]({})", item.jump_url)
            }
        ]
    })
}

fn sanitize_mentions(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (starts_at(&chars, i, "everyone") || starts_at(&chars, i, "here")) {
            out.push('@');
            out.push('\u{200b}');
            i += 1;
            continue;
        }
        if chars[i] == '<' && i + 1 < chars.len() && chars[i + 1] == '@' {
            let mut j = i + 2;
            if j < chars.len() && (chars[j] == '!' || chars[j] == '&') {
                j += 1;
            }
            let start = j;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && j < chars.len() && chars[j] == '>' {
                out.push_str("`@…`");
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn starts_at(chars: &[char], index: usize, word: &str) -> bool {
    let rest: String = chars[index + 1..].iter().collect();
    rest.to_ascii_lowercase().starts_with(word)
}

pub fn build_mirror_components(
    item_content: &str,
    author: &str,
    jump: &str,
    when: DateTime<Utc>,
    images: &[String],
) -> Value {
    let fake = PublicMessage {
        message_id: "1".into(),
        author_name: author.into(),
        content: item_content.into(),
        sent_at: Some(when.to_rfc3339()),
        caught_at: when.to_rfc3339(),
        jump_url: jump.into(),
        media: Vec::new(),
    };
    let mut container = mirror_body(&fake);
    if !images.is_empty() {
        let items: Vec<Value> = images
            .iter()
            .map(|name| json!({ "media": { "url": format!("attachment://{name}") } }))
            .collect();
        if let Some(components) = container
            .get_mut("components")
            .and_then(Value::as_array_mut)
        {
            components.push(json!({ "type": 12, "items": items }));
        }
    }
    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_disarmed() {
        let text = sanitize_mentions("hi @everyone and <@123> plus <@&99> and @here");
        assert!(!text.contains("@everyone"));
        assert!(!text.contains("@here"));
        assert!(!text.contains("<@123>"));
        assert!(!text.contains("<@&99>"));
        assert!(text.contains("@\u{200b}everyone"));
        assert!(text.contains("`@…`"));
    }

    #[test]
    fn empty_content_gets_placeholder() {
        let when = DateTime::parse_from_rfc3339("2026-09-28T19:37:49Z")
            .expect("fixture timestamp")
            .with_timezone(&Utc);
        let card = build_mirror_components(
            "",
            "Yoshi",
            "https://discord.com/channels/1/2/3",
            when,
            &["a.jpg".into()],
        );
        let blob = card.to_string();
        assert!(blob.contains("Spiegelung"));
        assert!(blob.contains("kein Text"));
        assert!(blob.contains("attachment://a.jpg"));
        assert!(blob.contains("Originalnachricht"));
        assert!(!blob.contains("@everyone"));
    }

    #[test]
    fn filenames_stay_on_attachment_id() {
        assert_eq!(
            discord_filename("1554", "image/jpeg", "IMG.jpg"),
            "1554.jpg"
        );
    }
}
