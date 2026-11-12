use std::collections::BTreeSet;
use std::time::Duration;

use dl_community::faq::{FaqChat, FaqPort, ReplayMessage};
use dl_community::ticket_triage::{MemberLookup, ShadowSource, TicketImage};
use serde_json::Value;

struct PrintPort;

#[async_trait::async_trait]
impl FaqPort for PrintPort {
    async fn create_faq_channel(&self, _: u64, _: u64, _: &str) -> Result<u64, String> {
        Err("replay".into())
    }
    async fn send_message(&self, _: u64, _: &str, _: Option<Value>) -> Result<u64, String> {
        Err("replay".into())
    }
    async fn channel_category(&self, _: u64, _: u64) -> Option<u64> {
        None
    }
    async fn private_faq_channel_owned_by_user(&self, _: u64, _: u64, _: u64) -> bool {
        false
    }
    async fn user_name(&self, _: u64) -> String {
        String::new()
    }
    async fn delete_channel(&self, _: u64) -> Result<(), String> {
        Err("replay".into())
    }
    async fn delete_message(&self, _: u64, _: u64) -> Result<(), String> {
        Err("replay".into())
    }
    async fn post_rich(&self, _: u64, _: serde_json::Map<String, Value>) -> Result<u64, String> {
        Err("replay".into())
    }
    async fn edit_rich(
        &self,
        _: u64,
        _: u64,
        _: serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        Err("replay".into())
    }
    async fn delete_panel(&self, _: u64, _: u64) {}
    async fn member_by_id(&self, _: u64, _: u64) -> MemberLookup {
        MemberLookup::Unknown
    }
    async fn member_by_name(&self, _: u64, _: &str) -> MemberLookup {
        MemberLookup::Unknown
    }
}

fn number(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .or_else(|| value[key].as_str().and_then(|text| text.parse().ok()))
        .ok_or_else(|| format!("{key} fehlt"))
}

fn replay_messages(input: &Value) -> Result<Vec<ReplayMessage>, String> {
    let messages = input["messages"].as_array().ok_or("messages fehlt")?;
    messages
        .iter()
        .map(|message| {
            let images = message["images"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|image| {
                    Ok(TicketImage {
                        url: image["url"].as_str().ok_or("url fehlt")?.to_string(),
                        size: number(image, "size")?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(ReplayMessage {
                content: message["content"].as_str().unwrap_or_default().to_string(),
                images,
            })
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Pfad zur Replay-Datei fehlt")?;
    let input: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let endpoint = std::env::var("DISCORD_BRAIN_PROBE_ENDPOINT")?;
    let token = std::env::var("DISCORD_BRAIN_CLIENT_TOKEN")?;
    let timeout_ms: u64 = std::env::var("DISCORD_BRAIN_PROBE_TIMEOUT_MS")?.parse()?;
    let brain = dl_brain::brain_api::BrainApiAnswerer::new(
        &endpoint,
        &token,
        Duration::from_millis(timeout_ms),
        "dl-bot-scam-replay".into(),
        BTreeSet::from(["bot.public".into()]),
    )?;
    let pool =
        sqlx::postgres::PgPoolOptions::new().connect_lazy("postgres://replay@localhost/replay")?;
    let faq = FaqChat::brain_only(
        pool,
        std::sync::Arc::new(PrintPort),
        std::sync::Arc::new(brain),
    );
    let guild_id = number(&input, "guild_id")?;
    let source = ShadowSource::PublicReport {
        guild_id,
        channel_id: number(&input, "channel_id")?,
        message_id: number(&input, "message_id")?,
    };
    let entry = faq
        .replay_public_report(
            source,
            guild_id,
            number(&input, "author_id")?,
            &replay_messages(&input)?,
        )
        .await
        .ok_or("Replay ohne Inhalt")?;
    println!("{entry}");
    Ok(())
}
