//! Discord-Glue für dl-moderation (ModPort + aimod:*-Review-Buttons).

use std::sync::Arc;

use dl_discord::{BridgeInteraction, BridgeReply, DiscordAdapter, InteractionHandler};
use serde_json::json;
use serenity::all::{ChannelId, GuildId, MessageId, UserId};

pub struct ModGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_moderation::ModPort for ModGlue {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn timeout_member(&self, guild_id: u64, user_id: u64, minutes: i64) -> bool {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": until.to_rfc3339() }),
                Some("AI-Moderation: Timeout"),
            )
            .await
            .is_ok()
    }

    async fn post_review(
        &self,
        case: &dl_moderation::store::CaseDraft,
        case_id: &str,
    ) -> Option<u64> {
        let preview: String = case.content.chars().take(900).collect();
        let embed = json!({
            "title": format!("🛡️ Moderationsvorschlag — {}", case.category),
            "description": format!(
                "**User:** <@{}> (`{}`)\n**Kanal:** <#{}>\n**Confidence:** {:.0}%\n**Begründung:** {}\n\n**Nachricht:**\n{}",
                case.user_id, case.user_tag, case.channel_id,
                case.confidence * 100.0, case.reason, preview
            ),
            "color": 0xE67E22,
        });
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 3, "label": "Accept (Delete + Timeout)",
              "custom_id": format!("aimod:accept:{case_id}") },
            { "type": 2, "style": 4, "label": "Ban",
              "custom_id": format!("aimod:ban:{case_id}") },
            { "type": 2, "style": 2, "label": "Deny",
              "custom_id": format!("aimod:deny:{case_id}") },
        ]}]);
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        self.adapter
            .send_raw_public(dl_moderation::MOD_REVIEW_CHANNEL_ID, &body)
            .await
            .ok()
    }

    async fn post_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &body)
            .await;
    }
}

/// Review-Buttons: aimod:accept|ban|deny:{case_id} (Mod-Guard via Rechte).
pub struct ReviewHandler {
    pub moderator: Arc<dl_moderation::AiModerator>,
}

#[async_trait::async_trait]
impl InteractionHandler for ReviewHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !interaction.author_can_manage_roles {
            return BridgeReply::ephemeral_text("Keine Berechtigung.");
        }
        let rest = interaction
            .custom_id
            .strip_prefix("aimod:")
            .unwrap_or_default();
        let Some((action, case_id)) = rest.split_once(':') else {
            return BridgeReply::ephemeral_text("Unbekannte Aktion.");
        };
        match action {
            "accept" => {
                self.moderator
                    .store
                    .resolve_case(case_id, "accepted", interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text(
                    "Akzeptiert — Nachricht löschen/Timeout bitte im Case prüfen.",
                )
            }
            "ban" => {
                self.moderator
                    .store
                    .resolve_case(case_id, "banned", interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text("Als Ban markiert — Ban bitte manuell ausführen.")
            }
            "deny" => {
                self.moderator
                    .store
                    .resolve_case(case_id, "denied", interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text("Vorschlag abgelehnt.")
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

// ── SecurityGuard-Anbindung ────────────────────────────────────────────────

pub struct GuardGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_moderation::guard::GuardPort for GuardGlue {
    async fn ban(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .ban_user(
                GuildId::new(guild_id),
                UserId::new(user_id),
                1, // 1 Tag Nachrichten löschen (wie delete_message_days=1)
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn timeout(&self, guild_id: u64, user_id: u64, minutes: i64, reason: &str) -> bool {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": until.to_rfc3339() }),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) -> bool {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("SecurityGuard: Beweissicherung/Aufräumen"),
            )
            .await
            .is_ok()
    }

    async fn send_dm(&self, user_id: u64, text: String) -> bool {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return false;
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .is_ok()
    }

    async fn post_mod_alert(
        &self,
        case: &dl_moderation::guard::Incident,
        action: &dl_moderation::guard::GuardAction,
    ) {
        let (title, color) = match action {
            dl_moderation::guard::GuardAction::Enforce => ("🛡️ Scam-Vollzug (Ban)", 0xED4245),
            dl_moderation::guard::GuardAction::Propose => {
                ("🛡️ Scam-Verdacht (Holding-Timeout 60 min)", 0xFFA500)
            }
        };
        let preview: String = case
            .messages
            .iter()
            .filter(|m| !m.content.is_empty())
            .map(|m| {
                format!(
                    "<#{}>: {}",
                    m.channel_id,
                    m.content.chars().take(150).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .take(900)
            .collect();
        let embed = json!({
            "title": title,
            "description": format!(
                "**User:** <@{}> (`{}`)\n**Case:** `{}`\n**Grund:** {}\n**Kanäle/Nachrichten/Anhänge/Keyword:** {}/{}/{}/{}\n\n{}",
                case.user_id, case.user_tag, case.case_id, case.reason,
                case.meta[0], case.meta[1], case.meta[2], case.meta[3], preview
            ),
            "color": color,
        });
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 4, "label": "Ban",
              "custom_id": format!("sg:ban:{}:{}", case.guild_id, case.user_id) },
            { "type": 2, "style": 3, "label": "Timeout aufheben",
              "custom_id": format!("sg:untimeout:{}:{}", case.guild_id, case.user_id) },
            { "type": 2, "style": 2, "label": "Unban",
              "custom_id": format!("sg:unban:{}:{}", case.guild_id, case.user_id) },
        ]}]);
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::guard::MOD_CHANNEL_ID, &body)
            .await;
    }

    async fn post_public_notice(&self, channel_id: u64, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }
}

/// sg:*-Mod-Buttons (Ban / Timeout aufheben / Unban) mit Rechte-Guard.
pub struct GuardReviewHandler {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl InteractionHandler for GuardReviewHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !interaction.author_can_manage_roles {
            return BridgeReply::ephemeral_text("Keine Berechtigung.");
        }
        let rest = interaction
            .custom_id
            .strip_prefix("sg:")
            .unwrap_or_default();
        let parts: Vec<&str> = rest.split(':').collect();
        let (Some(action), Some(guild_id), Some(user_id)) = (
            parts.first().copied(),
            parts.get(1).and_then(|v| v.parse::<u64>().ok()),
            parts.get(2).and_then(|v| v.parse::<u64>().ok()),
        ) else {
            return BridgeReply::ephemeral_text("Unbekannte Aktion.");
        };
        match action {
            "ban" => {
                let ok = self
                    .adapter
                    .http
                    .ban_user(
                        GuildId::new(guild_id),
                        UserId::new(user_id),
                        1,
                        Some("SecurityGuard: Mod-Bestätigung"),
                    )
                    .await
                    .is_ok();
                BridgeReply::ephemeral_text(if ok {
                    "Gebannt."
                } else {
                    "Ban fehlgeschlagen."
                })
            }
            "untimeout" => {
                let ok = self
                    .adapter
                    .http
                    .edit_member(
                        GuildId::new(guild_id),
                        UserId::new(user_id),
                        &json!({ "communication_disabled_until": null }),
                        Some("SecurityGuard: Timeout aufgehoben"),
                    )
                    .await
                    .is_ok();
                BridgeReply::ephemeral_text(if ok {
                    "Timeout aufgehoben."
                } else {
                    "Aufheben fehlgeschlagen."
                })
            }
            "unban" => {
                let ok = self
                    .adapter
                    .http
                    .remove_ban(
                        GuildId::new(guild_id),
                        UserId::new(user_id),
                        Some("SecurityGuard: Unban durch Mod"),
                    )
                    .await
                    .is_ok();
                BridgeReply::ephemeral_text(if ok {
                    "Entbannt."
                } else {
                    "Unban fehlgeschlagen."
                })
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

// ── Coaching-Plattform-Anbindung ───────────────────────────────────────────

pub struct CoachingGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub guild_id: u64,
}

#[async_trait::async_trait]
impl dl_community::coaching::CoachingPort for CoachingGlue {
    async fn coach_members(&self, role_id: u64) -> Vec<(u64, String, String, String)> {
        let Some(guild) = self.adapter.cache.guild(GuildId::new(self.guild_id)) else {
            return Vec::new();
        };
        let role = serenity::all::RoleId::new(role_id);
        guild
            .members
            .values()
            .filter(|member| member.roles.contains(&role))
            .map(|member| {
                let avatar = member
                    .user
                    .avatar_url()
                    .unwrap_or_else(|| member.user.default_avatar_url());
                let avatar = if avatar.contains('?') {
                    format!("{avatar}&size=256")
                } else {
                    format!("{avatar}?size=256")
                };
                (
                    member.user.id.get(),
                    member.user.name.to_string(),
                    member.display_name().to_string(),
                    avatar,
                )
            })
            .collect()
    }

    async fn send_dm(&self, user_id: u64, text: String) -> Result<bool, String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|e| e.to_string())?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => Ok(true),
            // 50007 = Cannot send messages to this user (DMs zu) → ackbar
            Err(err) if err.to_string().contains("50007") => Ok(false),
            Err(err) => Err(err.to_string()),
        }
    }
}

// ── Website-Invite-Anbindung ───────────────────────────────────────────────

pub struct InviteGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::invites::InvitePort for InviteGlue {
    async fn guild_invite_codes(&self, guild_id: u64) -> Vec<String> {
        self.adapter
            .http
            .get_guild_invites(GuildId::new(guild_id))
            .await
            .map(|invites| invites.into_iter().map(|i| i.code).collect())
            .unwrap_or_default()
    }

    async fn create_permanent_invite(
        &self,
        channel_id: u64,
        reason: &str,
    ) -> Result<String, String> {
        self.adapter
            .http
            .create_invite(
                ChannelId::new(channel_id),
                &json!({ "max_age": 0, "max_uses": 0, "unique": true }),
                Some(reason),
            )
            .await
            .map(|invite| invite.code)
            .map_err(|e| e.to_string())
    }
}

// ── Leave-Survey-Anbindung ─────────────────────────────────────────────────

pub struct SurveyGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::leave_survey::SurveyPort for SurveyGlue {
    async fn send_survey_dm(
        &self,
        user_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> String {
        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) if err.to_string().contains("50007") => return "blocked".to_string(),
            Err(_) => return "failed".to_string(),
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => "sent".to_string(),
            Err(err) if err.to_string().contains("50007") => "blocked".to_string(),
            Err(_) => "failed".to_string(),
        }
    }

    async fn post_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_community::leave_survey::LOGS_CHANNEL_ID, &body)
            .await;
    }

    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))?
            .members
            .get(&UserId::new(user_id))
            .map(|m| m.display_name().to_string())
    }
}

// ── Clip-Einsendungen-Anbindung ────────────────────────────────────────────

pub struct ClipGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::clips::ClipPort for ClipGlue {
    async fn upsert_interface(
        &self,
        channel_id: u64,
        existing_message_id: Option<u64>,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        if let Some(message_id) = existing_message_id {
            let edited = self
                .adapter
                .http
                .edit_message(
                    ChannelId::new(channel_id),
                    serenity::all::MessageId::new(message_id),
                    &body,
                    Vec::new(),
                )
                .await;
            match edited {
                Ok(message) => return Ok(message.id.get()),
                Err(err) if err.to_string().contains("10008") => {} // Nachricht weg → neu posten
                Err(err) => return Err(err.to_string()),
            }
        }
        self.adapter
            .send_raw_public(channel_id, &body)
            .await
            .map_err(|e| e.to_string())
    }

    async fn send_dump(
        &self,
        user_id: u64,
        fallback_channel_id: u64,
        caption: String,
        filename: String,
        content: String,
    ) {
        let attachment = serenity::all::CreateAttachment::bytes(content.into_bytes(), filename);
        let dm = async {
            let channel = self
                .adapter
                .http
                .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
                .await
                .map_err(|e| e.to_string())?;
            channel
                .send_files(
                    &self.adapter.http,
                    vec![attachment.clone()],
                    serenity::all::CreateMessage::new().content(caption.clone()),
                )
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(err) = dm {
            tracing::warn!(%err, "Clip-Dump-DM fehlgeschlagen — Fallback in den Submit-Kanal");
            let _ = ChannelId::new(fallback_channel_id)
                .send_files(
                    &self.adapter.http,
                    vec![attachment],
                    serenity::all::CreateMessage::new().content("📦 **Wochen-Dump (Clips)**"),
                )
                .await;
        }
    }

    async fn guild_name(&self, guild_id: u64) -> String {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))
            .map(|g| g.name.to_string())
            .unwrap_or_else(|| guild_id.to_string())
    }
}

// ── FAQ-Chat-Anbindung ─────────────────────────────────────────────────────

pub struct FaqGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::faq::FaqPort for FaqGlue {
    async fn create_faq_channel(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_name: &str,
    ) -> Result<u64, String> {
        let bot_id = self.adapter.cache.current_user().id.get();
        // VIEW=1024, SEND=2048, HISTORY=65536, MANAGE_CHANNELS=16
        let body = json!({
            "name": channel_name,
            "type": 0,
            "parent_id": dl_community::faq::FAQ_CATEGORY_ID.to_string(),
            "permission_overwrites": [
                { "id": guild_id.to_string(), "type": 0, "deny": "1024" },
                { "id": user_id.to_string(), "type": 1, "allow": "68608" },
                { "id": bot_id.to_string(), "type": 1, "allow": "68624" },
            ],
        });
        self.adapter
            .http
            .create_channel(
                GuildId::new(guild_id),
                body.as_object().expect("json object"),
                Some("FAQ Chat"),
            )
            .await
            .map(|c| c.id.get())
            .map_err(|e| e.to_string())
    }

    async fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        components: Option<serde_json::Value>,
    ) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        if let Some(components) = components {
            body.insert("components".into(), components);
        }
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .and_then(|c| c.parent_id.map(|p| p.get()))
    }

    async fn user_name(&self, user_id: u64) -> String {
        self.adapter
            .cache
            .user(UserId::new(user_id))
            .map(|u| u.name.to_string())
            .unwrap_or_else(|| format!("user-{user_id}"))
    }
}

// ── LFG-Lobby-Finder-Anbindung ─────────────────────────────────────────────

pub struct LfgGlue {
    pub adapter: Arc<DiscordAdapter>,
}

const LFG_CATEGORIES: [(u64, &str); 4] = [
    (1289721245281292290, "Casual"),
    (1412804540994162789, "Ranked"),
    (1357422957017698478, "Street Brawl"),
    (1465839366634209361, "New Player"),
];
const LFG_STAGINGS: [u64; 3] = [
    1501089974093873232,
    1412804671432818890,
    1357422958544420944,
];
const JUICE_KAMMER_ID: u64 = 1493690350580138114;

fn rank_from_role_names(names: &[String]) -> (String, i64, Option<i64>) {
    let mut best: (String, i64, Option<i64>) = (String::new(), 0, None);
    for name in names {
        let lower = name.trim().to_lowercase();
        let mut parts = lower.split_whitespace();
        let Some(first) = parts.next() else { continue };
        let Some((rank, value)) = dl_activity::lfg::RANK_NAMES
            .iter()
            .find(|(rank, _)| *rank == first)
        else {
            continue;
        };
        let sub: Option<i64> = parts
            .next()
            .and_then(|raw| raw.parse().ok())
            .filter(|v| (1..=6).contains(v));
        if *value > best.1 || (*value == best.1 && sub.is_some()) {
            let mut display = rank.to_string();
            if let Some(head) = display.get_mut(0..1) {
                head.make_ascii_uppercase();
            }
            best = (display, *value, sub);
        }
    }
    best
}

#[async_trait::async_trait]
impl dl_activity::lfg::LfgPort for LfgGlue {
    async fn scan_lanes(
        &self,
        guild_id: u64,
        co_player_ids: &[u64],
    ) -> Vec<dl_activity::lfg::LaneInfo> {
        use dl_activity::lfg::{LaneInfo, LaneLabel};
        let Some(guild) = self.adapter.cache.guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        let co_set: std::collections::HashSet<u64> = co_player_ids.iter().copied().collect();
        let mut lanes = Vec::new();
        for channel in guild.channels.values() {
            if channel.kind != serenity::all::ChannelType::Voice {
                continue;
            }
            let Some((category_id, label)) = channel.parent_id.and_then(|parent| {
                LFG_CATEGORIES
                    .iter()
                    .find(|(id, _)| *id == parent.get())
                    .copied()
            }) else {
                continue;
            };
            let label = match label {
                "Ranked" => LaneLabel::Ranked,
                "Street Brawl" => LaneLabel::StreetBrawl,
                "New Player" => LaneLabel::NewPlayer,
                _ => LaneLabel::Casual,
            };
            let member_ids: Vec<u64> = guild
                .voice_states
                .iter()
                .filter(|(_, vs)| vs.channel_id == Some(channel.id))
                .map(|(user_id, _)| user_id.get())
                .collect();
            let mut ranks: Vec<i64> = Vec::new();
            let mut co_names: Vec<String> = Vec::new();
            for user_id in &member_ids {
                if let Some(member) = guild.members.get(&UserId::new(*user_id)) {
                    if member.user.bot {
                        continue;
                    }
                    let names: Vec<String> = member
                        .roles
                        .iter()
                        .filter_map(|rid| guild.roles.get(rid).map(|r| r.name.to_string()))
                        .collect();
                    let (_, value, _) = rank_from_role_names(&names);
                    if value > 0 {
                        ranks.push(value);
                    }
                    if co_set.contains(user_id) {
                        co_names.push(member.display_name().to_string());
                    }
                }
            }
            let member_count = member_ids.len();
            let mut avg = if ranks.is_empty() {
                0.0
            } else {
                ranks.iter().sum::<i64>() as f64 / ranks.len() as f64
            };
            let avg_label = if channel.id.get() == JUICE_KAMMER_ID {
                avg = 11.0;
                "Eternus".to_string()
            } else if avg == 0.0 {
                "Leer".to_string()
            } else {
                let tier = (avg.round() as i64).clamp(1, 11);
                let mut name = dl_activity::lfg::RANK_NAMES
                    .iter()
                    .find(|(_, value)| *value == tier)
                    .map(|(rank, _)| rank.to_string())
                    .unwrap_or_else(|| "Unbekannt".to_string());
                if let Some(head) = name.get_mut(0..1) {
                    head.make_ascii_uppercase();
                }
                name
            };
            let mut limit = match channel.user_limit {
                Some(0) | None => 99,
                Some(value) => value as usize,
            };
            if label == LaneLabel::NewPlayer {
                limit = limit.min(6);
            }
            lanes.push(LaneInfo {
                channel_id: channel.id.get(),
                label,
                member_count,
                user_limit: limit,
                avg_rank_value: avg,
                co_players_present: co_names.len(),
                name: channel.name.to_string(),
                avg_rank_label: avg_label,
                category_id,
                position: channel.position as i64,
                is_staging: LFG_STAGINGS.contains(&channel.id.get()),
                co_player_names: co_names,
            });
        }
        lanes.sort_by_key(|lane| (lane.category_id, lane.position, lane.channel_id));
        lanes
    }

    async fn member_rank(&self, guild_id: u64, user_id: u64) -> (String, i64, Option<i64>) {
        let names: Vec<String> = self
            .adapter
            .cache
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members.get(&UserId::new(user_id)).map(|m| {
                    m.roles
                        .iter()
                        .filter_map(|rid| g.roles.get(rid).map(|r| r.name.to_string()))
                        .collect()
                })
            })
            .unwrap_or_default();
        rank_from_role_names(&names)
    }

    async fn member_in_voice(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.voice_states
                    .get(&UserId::new(user_id))
                    .map(|vs| vs.channel_id.is_some())
            })
            .unwrap_or(false)
    }

    async fn post_embed(&self, channel_id: u64, embed: serde_json::Value) {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }
}

// ── Coaching-Anfragen-Anbindung ────────────────────────────────────────────

pub struct CoachingReqGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::coaching_requests::CoachingPort for CoachingReqGlue {
    async fn coach_member_ids(&self, guild_id: u64) -> Vec<u64> {
        use dl_community::coaching_requests::{COACH_ROLE_ID, OWNER_EXCLUDE_ID};
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.members
                    .values()
                    .filter(|m| {
                        !m.user.bot
                            && m.user.id.get() != OWNER_EXCLUDE_ID
                            && m.roles.iter().any(|r| r.get() == COACH_ROLE_ID)
                    })
                    .map(|m| m.user.id.get())
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String {
        self.adapter
            .cache
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.display_name().to_string())
            })
            .unwrap_or_else(|| format!("User {user_id}"))
    }

    async fn member_is_admin(&self, guild_id: u64, user_id: u64) -> bool {
        let guild_id = GuildId::new(guild_id);
        let Some(guild) = self.adapter.cache.guild(guild_id) else {
            return false;
        };
        if guild.owner_id.get() == user_id {
            return true;
        }
        guild
            .members
            .get(&UserId::new(user_id))
            .map(|m| {
                m.roles.iter().any(|rid| {
                    guild
                        .roles
                        .get(rid)
                        .map(|r| r.permissions.administrator())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    async fn send_request_message(
        &self,
        channel_id: u64,
        content: &str,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        self.adapter
            .send_raw_public(channel_id, &body)
            .await
            .map_err(|e| e.to_string())
    }

    async fn edit_request_message(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        let _ = self
            .adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                serenity::all::MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await;
    }

    async fn send_channel_text(&self, channel_id: u64, content: &str) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn send_dm(&self, user_id: u64, content: &str) -> bool {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return false;
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .is_ok()
    }

    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
        let _ = self
            .adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                serenity::all::RoleId::new(role_id),
                Some(reason),
            )
            .await;
    }

    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
        let _ = self
            .adapter
            .http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                serenity::all::RoleId::new(role_id),
                Some(reason),
            )
            .await;
    }
}
