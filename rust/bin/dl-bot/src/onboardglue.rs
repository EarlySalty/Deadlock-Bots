//! Onboarding-Buttons — Cutover-kritischer Kern aus `cogs/welcome_dm/` +
//! `cogs/onboarding.py`.
//!
//! Die Onboarding-Nachrichten mit ihren persistenten Buttons existieren
//! bereits im Regelkanal; nach dem Cutover müssen ihre custom_ids weiter
//! funktionieren. Portiert: Regelbestätigung (`wdm:q4:confirm` →
//! Onboarding-Rolle — DAS Zugangs-Gate für neue Mitglieder), Steam-Login
//! (`steam:openid`) und die DM-Assistent-Hinweise (`dma:fallback:*`,
//! Texte wortgleich). Die Schritt-Navigation des Kanal-Flows
//! (`wdm:q0/qS/...`) antwortet ehrlich mit einem Hinweis — der volle
//! Flow folgt mit dem Onboarding-Rest (Phase 7).

use std::sync::Arc;
use std::time::Duration;

use dl_community::ai_onboarding::{AiOnboardingPort, MemberRole};
use dl_community::onboarding::OnboardingThreadError;
use dl_discord::{BridgeInteraction, BridgeReply, DiscordAdapter, InteractionHandler};
use serde_json::{json, Map, Value};
use serenity::all::{ChannelId, GuildId, RoleId, UserId};

pub const ONBOARD_COMPLETE_ROLE_ID: u64 = 1304216250649415771;
pub const MAIN_GUILD_ID: u64 = 1289721245281292288;

pub struct OnboardingHandler {
    pub adapter: Arc<DiscordAdapter>,
    pub steam: Arc<dl_bridges::steam::SteamBotClient>,
}

pub struct AiOnboardingGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl AiOnboardingPort for AiOnboardingGlue {
    async fn post_message(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn add_member_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), String> {
        self.adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some(reason),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn member_roles(&self, guild_id: u64, user_id: u64) -> Vec<MemberRole> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        let Some(member) = guild.members.get(&UserId::new(user_id)) else {
            return Vec::new();
        };
        member
            .roles
            .iter()
            .filter_map(|role_id| {
                guild.roles.get(role_id).map(|role| MemberRole {
                    id: role_id.get(),
                    name: role.name.clone(),
                })
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl InteractionHandler for OnboardingHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        match interaction.custom_id.as_str() {
            // Regelbestätigung → Onboarding-Rolle (Zugangs-Gate)
            "wdm:q4:confirm" => {
                let guild_id = if interaction.guild_id > 0 {
                    interaction.guild_id
                } else {
                    MAIN_GUILD_ID
                };
                match self
                    .adapter
                    .http
                    .add_member_role(
                        GuildId::new(guild_id),
                        UserId::new(interaction.user_id),
                        RoleId::new(ONBOARD_COMPLETE_ROLE_ID),
                        Some("Welcome DM: Regeln bestätigt"),
                    )
                    .await
                {
                    Ok(()) => BridgeReply::ephemeral_text("✅ Danke! Willkommen an Bord!"),
                    Err(err) => {
                        tracing::warn!(%err, user_id = interaction.user_id, "Onboarding: Rollen-Vergabe fehlgeschlagen");
                        BridgeReply::ephemeral_text(
                            "Regeln bestätigt — die Rolle konnte gerade nicht vergeben werden, das Team schaut drauf.",
                        )
                    }
                }
            }

            // Frischer Steam-Login-Link (Einmal-URL vom Steam-Bot)
            "steam:openid" => match self.steam.fetch_steam_link_url(interaction.user_id).await {
                Some(url) => BridgeReply {
                    content: Some(
                        "Melde dich kurz bei Steam an (OpenID — kein Passwort nötig):".to_string(),
                    ),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 2, "style": 5, "label": "Mit Steam anmelden",
                        "emoji": {"name": "🎮"}, "url": url,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                },
                None => BridgeReply::ephemeral_text(
                    "Der Link-Dienst ist gerade nicht verfügbar. Nutze vorerst **/account_verknüpfen**.",
                ),
            },

            // DM-Assistent-Hinweise (Texte wortgleich zum Original)
            "dma:fallback:steam" => BridgeReply::ephemeral_text(
                "Verknüpfe deinen Steam-Account mit **/steamlink** im Server.\nDas ist wichtig für Rang, Live-Status in den Voice Lanes und Spielersuche.",
            ),
            "dma:fallback:faq" => BridgeReply::ephemeral_text(
                "Nutze **/faq** oder **/serverfaq <frage>** im Server für häufig gestellte Fragen.\nDu kannst mir hier auch direkt deine Frage stellen!",
            ),
            "dma:fallback:streamer" => BridgeReply::ephemeral_text(
                "Nutze **/streamer** im Server, um den Streamer-Partner-Prozess zu starten!\nAls Partner bekommst du: Auto-Raid, Chat Guard, Analytics und mehr.",
            ),
            "dma:fallback:beta" => BridgeReply::ephemeral_text(
                "Für einen Deadlock Beta-Invite nutze **/betainvite** im Server.\nAlternativ schau im <#1428745737323155679> Channel vorbei.",
            ),

            // Schritt-Navigation: Flow-Steuerung folgt mit dem Onboarding-Rest
            _ => BridgeReply::ephemeral_text(
                "Dieser Onboarding-Schritt wird gerade umgebaut — die Regeln kannst du oben bestätigen, alles andere findest du in den Server-Kanälen.",
            ),
        }
    }
}

pub fn register(
    router: &mut dl_discord::InteractionRouter,
    adapter: Arc<DiscordAdapter>,
    steam: Arc<dl_bridges::steam::SteamBotClient>,
) {
    let handler = Arc::new(OnboardingHandler { adapter, steam });
    for custom_id in [
        "wdm:q4:confirm",
        "wdm:q0:intro_next",
        "wdm:q1:masterbot",
        "wdm:q2:servertour",
        "wdm:qS:next",
        "wdm:qS:status",
        "steam:openid",
        "steam:next",
        "dma:fallback:steam",
        "dma:fallback:faq",
        "dma:fallback:streamer",
        "dma:fallback:beta",
    ] {
        router.on_custom_id(custom_id, handler.clone());
    }
}

// ── Onboarding-Wizard-Anbindung ────────────────────────────────────────────

pub struct WizardGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub tags: Arc<dl_community::tags::TagService>,
    pub steam: Arc<dl_bridges::steam::SteamBotClient>,
    pub db: dl_db::Db,
}

const DISCORD_TRANSIENT_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(750), Duration::from_millis(1500)];

fn is_transient_discord_error(err: &serenity::Error) -> bool {
    matches!(
        err,
        serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp))
            if (500..=599).contains(&resp.status_code.as_u16())
    )
}

fn thread_error_from_serenity(err: serenity::Error) -> OnboardingThreadError {
    if is_transient_discord_error(&err) {
        OnboardingThreadError::Transient(err.to_string())
    } else {
        OnboardingThreadError::Permanent(err.to_string())
    }
}

async fn retry_discord_http<T, Op, Fut>(mut operation: Op) -> Result<T, serenity::Error>
where
    Op: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, serenity::Error>>,
{
    let mut attempt = 0usize;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err)
                if is_transient_discord_error(&err)
                    && attempt < DISCORD_TRANSIENT_RETRY_DELAYS.len() =>
            {
                let delay = DISCORD_TRANSIENT_RETRY_DELAYS[attempt];
                attempt += 1;
                tokio::time::sleep(delay).await;
            }
            Err(err) => return Err(err),
        }
    }
}

fn thread_body(name: &str, channel_type: u8, invitable: Option<bool>) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("name".into(), json!(name));
    body.insert("type".into(), json!(channel_type));
    body.insert("auto_archive_duration".into(), json!(60));
    if let Some(invitable) = invitable {
        body.insert("invitable".into(), json!(invitable));
    }
    body
}

impl WizardGlue {
    async fn create_thread_with_retry(
        &self,
        body: &Map<String, Value>,
        reason: &'static str,
    ) -> Result<serenity::all::GuildChannel, serenity::Error> {
        retry_discord_http(|| {
            let http = self.adapter.http.clone();
            let body = body.clone();
            async move {
                http.create_thread(
                    ChannelId::new(dl_community::onboarding::RULES_CHANNEL_ID),
                    &body,
                    Some(reason),
                )
                .await
            }
        })
        .await
    }

    async fn add_thread_member_with_retry(
        &self,
        thread_id: ChannelId,
        user_id: UserId,
    ) -> Result<(), serenity::Error> {
        retry_discord_http(|| {
            let http = self.adapter.http.clone();
            async move { http.add_thread_channel_member(thread_id, user_id).await }
        })
        .await
    }

    async fn delete_thread_quietly(&self, thread_id: ChannelId) {
        if let Err(err) = retry_discord_http(|| {
            let http = self.adapter.http.clone();
            async move {
                http.delete_channel(thread_id, Some("Onboarding-Thread-Fallback"))
                    .await
            }
        })
        .await
        {
            tracing::debug!(%err, thread_id = thread_id.get(), "Onboarding: fehlgeschlagenen privaten Thread nicht geloescht");
        }
    }
}

#[async_trait::async_trait]
impl dl_community::onboarding::OnboardingPort for WizardGlue {
    async fn create_onboarding_thread(
        &self,
        _guild_id: u64,
        user_id: u64,
        name: &str,
    ) -> Result<u64, OnboardingThreadError> {
        // privater Thread (type 12), 60-min-Auto-Archiv, invitable — wie Original
        let private_body = thread_body(name, 12, Some(true));
        let mut private_thread_id = None;
        match self
            .create_thread_with_retry(&private_body, "Onboarding-Thread")
            .await
        {
            Ok(thread) => {
                private_thread_id = Some(thread.id);
                match self
                    .add_thread_member_with_retry(thread.id, UserId::new(user_id))
                    .await
                {
                    Ok(()) => return Ok(thread.id.get()),
                    Err(err) => {
                        tracing::warn!(%err, user_id, thread_id = thread.id.get(), "Onboarding: User konnte privatem Thread nicht hinzugefuegt werden; nutze Public-Fallback");
                    }
                }
            }
            Err(err) => {
                tracing::debug!(%err, user_id, "Onboarding: privater Thread nicht nutzbar; nutze Public-Fallback");
            }
        }

        if let Some(thread_id) = private_thread_id {
            self.delete_thread_quietly(thread_id).await;
        }

        let public_body = thread_body(name, 11, None);
        self.create_thread_with_retry(&public_body, "Onboarding-Thread-Fallback")
            .await
            .map(|thread| thread.id.get())
            .map_err(thread_error_from_serenity)
    }

    async fn send_step(
        &self,
        channel_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), serde_json::json!([embed]));
        body.insert("components".into(), components);
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn member_is_bot(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| g.members.get(&UserId::new(user_id)).map(|m| m.user.bot))
            .unwrap_or(false)
    }

    async fn has_verified_steam_link(&self, user_id: u64) -> bool {
        let Ok(user_id_sql) = i64::try_from(user_id) else {
            return false;
        };
        self.db
            .read(move |conn| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    "SELECT 1 FROM steam_links
                     WHERE user_id=?1 AND verified=1 AND is_steam_friend=1
                     LIMIT 1",
                    rusqlite::params![user_id_sql],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map(|row| row.is_some())
            })
            .await
            .unwrap_or(false)
    }

    async fn claim_screening_auto_start(&self, user_id: u64) -> bool {
        let key = user_id.to_string();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO kv_store(ns, k, v) VALUES(?1, ?2, 'claimed')",
                    rusqlite::params![dl_community::onboarding::AUTO_ONBOARDING_CLAIM_NS, key],
                )
                .map(|affected| affected > 0)
            })
            .await
            .unwrap_or(false)
    }

    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.display_name().to_string())
            })
            .unwrap_or_else(|| format!("User {user_id}"))
    }

    async fn steam_link_url(&self, user_id: u64) -> Option<String> {
        self.steam.fetch_steam_link_url(user_id).await
    }

    async fn set_user_tag(&self, user_id: u64, key: &str, value: &str) {
        if let Err(err) = self.tags.set_user_tag(user_id, key, value).await {
            tracing::warn!(%err, user_id, key, "Onboarding-Tag konnte nicht gesetzt werden");
        }
    }

    async fn register_pending_verify(&self, user_id: u64, channel_id: u64) {
        let _ = self
            .db
            .write(move |conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS onboarding_pending_verify(
                       user_id INTEGER PRIMARY KEY, channel_id INTEGER NOT NULL,
                       updated_at DATETIME DEFAULT CURRENT_TIMESTAMP);",
                )?;
                conn.execute(
                    "INSERT INTO onboarding_pending_verify(user_id, channel_id) VALUES(?1, ?2)
                     ON CONFLICT(user_id) DO UPDATE SET channel_id=excluded.channel_id,
                       updated_at=CURRENT_TIMESTAMP",
                    rusqlite::params![user_id, channel_id],
                )?;
                Ok(())
            })
            .await;
    }

    async fn pop_pending_verify(&self, user_id: u64) -> Option<u64> {
        self.db
            .write(move |conn| {
                use rusqlite::OptionalExtension;
                let channel: Option<i64> = conn
                    .query_row(
                        "SELECT channel_id FROM onboarding_pending_verify WHERE user_id=?1",
                        rusqlite::params![user_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if channel.is_some() {
                    conn.execute(
                        "DELETE FROM onboarding_pending_verify WHERE user_id=?1",
                        rusqlite::params![user_id],
                    )?;
                }
                Ok(channel.map(|c| c as u64))
            })
            .await
            .ok()
            .flatten()
    }

    async fn send_text(&self, channel_id: u64, content: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), serde_json::Value::from(content));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }
}
