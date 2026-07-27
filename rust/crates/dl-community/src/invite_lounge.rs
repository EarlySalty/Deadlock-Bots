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
pub const INVITE_LOUNGE_CHANNEL_ID: u64 = 1_426_220_702_054_355_077;
pub const INVITE_LOUNGE_HINT_TEXT: &str = "Kleiner Tipp: pack noch deinen Steam-Freundescode dazu, sonst kann dich niemand einladen :) Du findest ihn in Steam unter Freunde → „Freund hinzufügen\". Einfach hier posten — wer Zeit hat, lädt dich ein.";

const COOLDOWN_SECONDS: i64 = 24 * 60 * 60;
const COOLDOWN_KV_NS: &str = "invite_lounge:cooldown";
/// Gleiches 7-Tage-Fenster wie dl-moderation NEW_MEMBER_MAX_JOIN_HOURS=168;
/// lokal, weil dl-community nicht von dl-moderation abhaengt.
pub const NEWCOMER_MAX_JOIN_SECONDS: i64 = 7 * 24 * 60 * 60;

static FRIEND_CODE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\b\d{6,12}\b").ok());
static STEAM_LINK_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:https?://)?(?:www\.)?(?:s\.team/p/|steamcommunity\.com)").ok()
});
/// Laeuft auf umlaut-gefoldetem Kleintext. `ein\w{0,3}lad` deckt den trennbaren
/// Verbstamm mit eingeschobenem `zu` ab (einladen/einzuladen/Einladung), die
/// zweite Haelfte die echte Trennung ("laedt mich ein", "lade uns bitte ein").
static INVITE_TERM_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"invite|playtest|beta|\bein\w{0,3}lad|\blad(?:e|et|t)?\s+(?:\w+\s+){0,3}\bein\b")
        .ok()
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

        let now = chrono::Utc::now().timestamp();
        if !should_reply(&event.content, event.author_joined_at, now) {
            return;
        }

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

pub fn should_reply(content: &str, author_joined_at: Option<i64>, now: i64) -> bool {
    is_newcomer(author_joined_at, now) && should_reply_to_content(content)
}

fn is_newcomer(joined_at: Option<i64>, now: i64) -> bool {
    joined_at.is_some_and(|joined| now.saturating_sub(joined) < NEWCOMER_MAX_JOIN_SECONDS)
}

fn is_invite_request(content: &str) -> bool {
    let lower = content.to_lowercase();
    let folded = fold_german_umlauts(&lower);
    let mentions_invite = INVITE_TERM_RE
        .as_ref()
        .is_some_and(|regex| regex.is_match(&folded));
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
    fn trennbares_verb_einzuladen_triggert() {
        assert!(should_reply_to_content(
            "Hallöchen zusammen, bin neu hier und suche zum einen jemand der so freundlich wäre mich in Deadlock einzuladen und zum anderen bissl Anschluss zum Game"
        ));
    }

    #[test]
    fn getrenntes_laedt_mich_ein_triggert() {
        assert!(should_reply_to_content("Wer lädt mich ein?"));
        assert!(should_reply_to_content("Kann mich jemand mal einladen"));
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

    #[test]
    fn veteran_mit_invite_frage_triggert_nicht() {
        let now = 1_700_000_000;
        let content = "Hast du schon eine Einladung bekommen? Ich kann dich gerne im Laufe des Tages einladen";

        assert!(!should_reply(content, Some(now - 400 * 24 * 60 * 60), now));
    }

    #[test]
    fn neuling_mit_invite_frage_triggert() {
        let now = 1_700_000_000;
        let content = "Hast du schon eine Einladung bekommen? Ich kann dich gerne im Laufe des Tages einladen";

        assert!(should_reply(content, Some(now - 2 * 24 * 60 * 60), now));
    }

    #[test]
    fn unbekanntes_beitrittsdatum_triggert_nicht() {
        let now = 1_700_000_000;

        assert!(!should_reply(
            "Hey, kann mich jemand bitte zu Deadlock inviten?",
            None,
            now,
        ));
    }

    #[test]
    fn grenzfall_genau_sieben_tage_triggert_nicht() {
        let now = 1_700_000_000;

        assert!(!should_reply(
            "Hey, kann mich jemand bitte zu Deadlock inviten?",
            Some(now - NEWCOMER_MAX_JOIN_SECONDS),
            now,
        ));
    }

    #[test]
    fn neuling_mit_freundescode_triggert_nicht() {
        let now = 1_700_000_000;

        assert!(!should_reply(
            "Kann mich jemand inviten? Mein Code ist 123456789",
            Some(now - 60),
            now,
        ));
    }
}
