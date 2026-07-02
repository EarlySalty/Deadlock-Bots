//! Invite-Lounge-Watcher fuer die offene Invite-Lounge.
//!
//! Der Kanal ist absichtlich offen und ohne Panel-/Button-Flow. Dieser Watcher
//! reagiert nur auf klare Invite-Fragen ohne Steam-Freundescode und antwortet
//! als reine Text-Reply.

use std::sync::{Arc, LazyLock};

use dl_central_db::kv;
use regex::Regex;
use serde_json::{json, Map, Value};
use sqlx::PgPool;

/// Live-ID der Invite-Lounge (heute `beta-zugang`, kuenftig `deadlock-invite`;
/// per Discord-API verifiziert, ID ist rename-stabil — Welle-2a-Rename und
/// Rollback 2026-07-03 haben sie nicht veraendert. Die aeltere ID aus den
/// Python-Cogs war schlicht veraltet).
pub const INVITE_LOUNGE_CHANNEL_ID: u64 = 1_464_736_918_951_432_222;
pub const INVITE_LOUNGE_HINT_TEXT: &str = "Kleiner Tipp: pack noch deinen Steam-Freundescode dazu, sonst kann dich niemand einladen :) Du findest ihn in Steam unter Freunde → „Freund hinzufügen\". Einfach hier posten — wer Zeit hat, lädt dich ein.";

const COOLDOWN_SECONDS: i64 = 24 * 60 * 60;
const COOLDOWN_KV_NS: &str = "invite_lounge:cooldown";

static FRIEND_CODE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\b\d{6,12}\b").ok());
static STEAM_LINK_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:https?://)?(?:www\.)?(?:s\.team/p/|steamcommunity\.com)").ok()
});
static QUESTION_SIGNAL_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?iu)\b(?:wer|mag|kann|könnte|koennte|jemand|würde|wuerde|hätte|haette)\b").ok()
});

#[async_trait::async_trait]
pub trait InviteLoungeReplyPort: Send + Sync {
    async fn reply_text(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<u64, String>;
}

#[async_trait::async_trait]
impl InviteLoungeReplyPort for dl_discord::DiscordAdapter {
    async fn reply_text(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<u64, String> {
        let mut body = Map::new();
        body.insert("content".into(), Value::String(content.to_string()));
        body.insert(
            "message_reference".into(),
            json!({
                "channel_id": channel_id.to_string(),
                "message_id": message_id.to_string(),
                "fail_if_not_exists": false,
            }),
        );
        body.insert(
            "allowed_mentions".into(),
            json!({
                "parse": [],
                "replied_user": false,
            }),
        );
        self.send_raw_public(channel_id, &body).await
    }
}

pub struct InviteLoungeWatcher {
    pool: PgPool,
    port: Arc<dyn InviteLoungeReplyPort>,
}

impl InviteLoungeWatcher {
    pub fn new(pool: PgPool, port: Arc<dyn InviteLoungeReplyPort>) -> Self {
        Self { pool, port }
    }

    async fn handle_message(&self, event: dl_discord::MessageEvent) {
        if event.guild_id.is_none() || event.channel_id != INVITE_LOUNGE_CHANNEL_ID {
            return;
        }
        if !should_reply_to_content(&event.content) {
            return;
        }

        let now = chrono::Utc::now().timestamp();
        match self.cooldown_allows(event.author_id, now).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(err) => {
                tracing::warn!(
                    %err,
                    user_id = event.author_id,
                    "Invite-Lounge-Cooldown konnte nicht gelesen werden"
                );
                return;
            }
        }

        match self
            .port
            .reply_text(event.channel_id, event.message_id, INVITE_LOUNGE_HINT_TEXT)
            .await
        {
            Ok(_) => {
                if let Err(err) = self.mark_cooldown(event.author_id, now).await {
                    tracing::warn!(
                        %err,
                        user_id = event.author_id,
                        "Invite-Lounge-Cooldown konnte nicht gespeichert werden"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    channel_id = event.channel_id,
                    message_id = event.message_id,
                    "Invite-Lounge-Hinweis konnte nicht gesendet werden"
                );
            }
        }
    }

    async fn cooldown_allows(
        &self,
        user_id: u64,
        now: i64,
    ) -> Result<bool, dl_central_db::CentralDbError> {
        let Some(raw) = kv::get(&self.pool, COOLDOWN_KV_NS, &cooldown_key(user_id)).await? else {
            return Ok(true);
        };
        let Ok(last_sent_at) = raw.parse::<i64>() else {
            return Ok(true);
        };
        Ok(now.saturating_sub(last_sent_at) >= COOLDOWN_SECONDS)
    }

    async fn mark_cooldown(
        &self,
        user_id: u64,
        now: i64,
    ) -> Result<(), dl_central_db::CentralDbError> {
        kv::set(
            &self.pool,
            COOLDOWN_KV_NS,
            &cooldown_key(user_id),
            &now.to_string(),
        )
        .await
    }
}

pub fn spawn(
    pool: PgPool,
    port: Arc<dyn InviteLoungeReplyPort>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let watcher = Arc::new(InviteLoungeWatcher::new(pool, port));
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => watcher.handle_message(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub fn should_reply_to_content(content: &str) -> bool {
    is_invite_request(content) && !has_friend_code(content)
}

fn is_invite_request(content: &str) -> bool {
    let lower = content.to_lowercase();
    let folded = fold_german_umlauts(&lower);
    let mentions_invite = folded.contains("invite")
        || folded.contains("einlad")
        || folded.contains("playtest")
        || folded.contains("beta");
    mentions_invite && has_question_signal(&lower)
}

fn has_question_signal(lower_content: &str) -> bool {
    lower_content.contains('?')
        || QUESTION_SIGNAL_RE
            .as_ref()
            .is_some_and(|regex| regex.is_match(lower_content))
}

fn has_friend_code(content: &str) -> bool {
    FRIEND_CODE_RE
        .as_ref()
        .is_some_and(|regex| regex.is_match(content))
        || STEAM_LINK_RE
            .as_ref()
            .is_some_and(|regex| regex.is_match(content))
}

fn fold_german_umlauts(value: &str) -> String {
    value
        .replace('ä', "a")
        .replace('ö', "o")
        .replace('ü', "u")
        .replace('ß', "ss")
}

fn cooldown_key(user_id: u64) -> String {
    format!("user:{user_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nette_frage_ohne_code_triggert() {
        assert!(should_reply_to_content(
            "Hey, kann mich jemand bitte zu Deadlock inviten?"
        ));
    }

    #[test]
    fn frage_mit_neunstelligem_code_triggert_nicht() {
        assert!(!should_reply_to_content(
            "Mag mich jemand zum Playtest einladen? Mein Code ist 123456789"
        ));
    }

    #[test]
    fn antwort_ohne_fragesignal_triggert_nicht() {
        assert!(!should_reply_to_content("ich lad dich ein"));
    }

    #[test]
    fn steam_link_zaehlt_als_code() {
        assert!(!should_reply_to_content(
            "Kann mich jemand inviten https://steamcommunity.com/id/example"
        ));
        assert!(!should_reply_to_content(
            "Wer hat einen Invite fuer mich? s.team/p/abc-def"
        ));
    }
}
