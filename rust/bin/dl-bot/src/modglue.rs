//! Discord-Glue für dl-moderation (ModPort + aimod:*-Review-Buttons).

use std::sync::Arc;

use dl_discord::{BridgeInteraction, BridgeReply, DiscordAdapter, InteractionHandler};
use serde_json::json;
use serenity::all::{ChannelId, GuildId, Message, MessageId, UserId};
use serenity::builder::GetMessages;

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

    async fn ban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
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

    async fn post_review(
        &self,
        case: &dl_moderation::store::CaseDraft,
        case_id: &str,
    ) -> Option<u64> {
        let preview: String = case.content.chars().take(900).collect();
        let embed = json!({
            "title": format!("🛡️ Moderationsvorschlag — {}", case.category),
            "description": format!(
                "**User:** <@{}> (`{}`)\n**Kanal:** <#{}>\n**Sicherheit:** {:.0}%\n**Begründung:** {}\n\n**Nachricht:**\n{}",
                case.user_id, case.user_tag, case.channel_id,
                case.confidence * 100.0, case.reason, preview
            ),
            "color": 0xE67E22,
        });
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 3, "label": "Annehmen (Löschen + Timeout)",
              "custom_id": format!("aimod:accept:{case_id}") },
            { "type": 2, "style": 4, "label": "Ban",
              "custom_id": format!("aimod:ban:{case_id}") },
            { "type": 2, "style": 2, "label": "Ablehnen",
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

    async fn send_dm(&self, user_id: u64, text: String) {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return;
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self.adapter.send_raw_public(channel.id.get(), &body).await;
    }

    async fn fetch_context_lines(
        &self,
        guild_id: u64,
        channel_id: u64,
        before_message_id: u64,
        author_id: u64,
        message_created_at: i64,
        limit: usize,
    ) -> Vec<String> {
        let fetch_limit = limit.saturating_add(1).min(100) as u8;
        let Ok(mut messages) = ChannelId::new(channel_id)
            .messages(
                &self.adapter.http,
                GetMessages::new()
                    .before(MessageId::new(before_message_id))
                    .limit(fetch_limit),
            )
            .await
        else {
            return Vec::new();
        };
        messages.sort_by_key(|message| message.timestamp.unix_timestamp());

        let mut lines = Vec::new();
        for previous in messages {
            let mut preview = strip_mentions(&previous.content);
            if preview.is_empty() && !previous.attachments.is_empty() {
                preview = "[Anhang]".to_string();
            }
            if preview.is_empty() {
                continue;
            }

            let delta_s = message_created_at - previous.timestamp.unix_timestamp();
            let mins = (delta_s.max(0)) / 60;
            let time_tag = if mins < 60 {
                format!("[{mins}min ago]")
            } else {
                format!("[{}h ago]", mins / 60)
            };
            let prefix = if previous.author.id.get() == author_id {
                ">>>"
            } else {
                "   "
            };
            let display_name = self.display_name_for_message(guild_id, &previous);
            lines.push(format!(
                "{prefix} {time_tag} {display_name}: {}",
                truncate_chars(&preview, 150)
            ));
        }
        if lines.len() > limit {
            lines.split_off(lines.len() - limit)
        } else {
            lines
        }
    }

    async fn fetch_reply_context(
        &self,
        guild_id: u64,
        channel_id: u64,
        reply_channel_id: Option<u64>,
        reply_message_id: Option<u64>,
    ) -> Option<dl_moderation::ReplyContext> {
        let message_id = reply_message_id?;
        let channel_id = reply_channel_id.unwrap_or(channel_id);
        let message = ChannelId::new(channel_id)
            .message(&self.adapter.http, MessageId::new(message_id))
            .await
            .ok()?;
        let mut content = strip_mentions(&message.content);
        if content.is_empty() {
            content = if message.attachments.is_empty() {
                "[kein Text]".to_string()
            } else {
                "[Anhang]".to_string()
            };
        }
        Some(dl_moderation::ReplyContext {
            author: truncate_chars(&self.display_name_for_message(guild_id, &message), 60),
            content: truncate_chars(&content, 300),
        })
    }
}

impl ModGlue {
    fn display_name_for_message(&self, guild_id: u64, message: &Message) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|guild| {
                guild
                    .members
                    .get(&message.author.id)
                    .map(|member| member.display_name().to_string())
            })
            .unwrap_or_else(|| message.author.name.to_string())
    }
}

/// Review-Buttons: aimod:accept|ban|deny:{case_id} (Mod-Guard via Rechte).
/// `deny` öffnet ein Modal (Pflicht-Grund) → Submit kommt als
/// `aimod:denysubmit:{case_id}` über dieselbe Prefix-Route zurück.
pub struct ReviewHandler {
    pub moderator: Arc<dl_moderation::AiModerator>,
}

impl ReviewHandler {
    fn outcome_reply(outcome: dl_moderation::ReviewOutcome) -> BridgeReply {
        use dl_moderation::ReviewOutcome;
        match outcome {
            ReviewOutcome::NotFound => BridgeReply::ephemeral_text("Case nicht gefunden."),
            ReviewOutcome::AlreadyHandled => {
                BridgeReply::ephemeral_text("Case wurde bereits bearbeitet.")
            }
            ReviewOutcome::Done(text) => BridgeReply::ephemeral_text(text),
        }
    }
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
                let outcome = self.moderator.accept_case(case_id, interaction.user_id).await;
                Self::outcome_reply(outcome)
            }
            "ban" => {
                let outcome = self.moderator.ban_case(case_id, interaction.user_id).await;
                Self::outcome_reply(outcome)
            }
            // Button: Modal mit Pflicht-Grund öffnen (Original: DenyReasonModal,
            // required, min 4 / max 500, mehrzeilig).
            "deny" => BridgeReply {
                modal: Some(dl_discord::ModalSpec {
                    custom_id: format!("aimod:denysubmit:{case_id}"),
                    title: "Moderation ablehnen".to_string(),
                    fields: vec![dl_discord::ModalField {
                        custom_id: "reason".to_string(),
                        label: "Warum lehnst du ab?".to_string(),
                        placeholder: "Kurze Begruendung fuer die Ablehnung.".to_string(),
                        required: true,
                        min_length: 4,
                        max_length: 500,
                        paragraph: true,
                    }],
                }),
                ..BridgeReply::default()
            },
            // Modal-Submit: Grund speichern + Log posten.
            "denysubmit" => {
                let reason = interaction
                    .options
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let outcome = self
                    .moderator
                    .deny_case(case_id, interaction.user_id, &reason)
                    .await;
                Self::outcome_reply(outcome)
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

// ── SecurityGuard-Anbindung ────────────────────────────────────────────────

/// Zeitspanne `now - past` lesbar (Original: `_fmt_delta`): „Xd Yh" / „Xh Ym"
/// / „Xm", `n/a` ohne Zeitpunkt. Eingaben in Unix-Sekunden.
fn fmt_delta(now: i64, past: Option<i64>) -> String {
    let Some(past) = past else {
        return "n/a".to_string();
    };
    let total = (now - past).max(0);
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn normalize_text(value: &str) -> String {
    value
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_discord_mention(candidate: &str) -> bool {
    let Some(first) = candidate.chars().next() else {
        return false;
    };
    if first != '@' && first != '#' {
        return false;
    }
    let rest = &candidate[first.len_utf8()..];
    let rest = rest
        .strip_prefix('!')
        .or_else(|| rest.strip_prefix('&'))
        .unwrap_or(rest);
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())
}

fn strip_mentions(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('>') else {
            out.push_str(&rest[open..]);
            return normalize_text(&out);
        };
        let candidate = &after_open[..close];
        if !is_discord_mention(candidate) {
            out.push('<');
            out.push_str(candidate);
            out.push('>');
        }
        rest = &after_open[close + 1..];
    }
    out.push_str(rest);
    normalize_text(&out)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

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

    async fn send_dm_with_appeal(&self, user_id: u64, text: String, case_id: &str) -> bool {
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
        // „Einspruch"-Button → läuft über den registrierten sg:-Prefix in den
        // GuardReviewHandler (öffnet das Einspruch-Modal).
        body.insert(
            "components".into(),
            json!([{ "type": 1, "components": [
                { "type": 2, "style": 1, "label": "Einspruch",
                  "custom_id": format!("sg:appeal:{case_id}") },
            ]}]),
        );
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
            dl_moderation::guard::GuardAction::EstablishedScam => {
                ("🛡️ Scam erkannt: etablierter Account — Auto-Timeout (24h)", 0xE67E22)
            }
            dl_moderation::guard::GuardAction::Takeover => {
                ("⚠️ Account-Takeover erkannt — Quarantäne (24h-Timeout, reversibel)", 0xE74C3C)
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

        // Angereicherte Log-Felder (Original: `_log_incident`): Account-Alter,
        // Zeit seit Join, Aktivitätsfenster, Signale, Aktionen.
        let now = chrono::Utc::now().timestamp();
        let is_ban = matches!(action, dl_moderation::guard::GuardAction::Enforce);
        let action_ok_text = if case.action_ok { "yes" } else { "failed" };
        let action_text = if is_ban {
            format!("Ban: {action_ok_text}")
        } else {
            let minutes = match action {
                dl_moderation::guard::GuardAction::Propose => {
                    dl_moderation::guard::PROPOSAL_TIMEOUT_MINUTES
                }
                dl_moderation::guard::GuardAction::EstablishedScam => {
                    dl_moderation::guard::TIMEOUT_MINUTES
                }
                _ => dl_moderation::guard::TIMEOUT_MINUTES,
            };
            format!("Timeout {minutes}m: {action_ok_text}")
        };
        let reason_value: String = if case.reason.is_empty() {
            "auto-detected burst".to_string()
        } else {
            case.reason.chars().take(1000).collect()
        };
        // meta = [channel_count, message_count, attachment_count, keyword_hit].
        let mut fields = vec![
            json!({ "name": "Member", "value": format!("<@{}> ({})", case.user_id, case.user_id), "inline": false }),
            json!({ "name": "Case ID", "value": case.case_id, "inline": true }),
            json!({ "name": "Account age", "value": fmt_delta(now, Some(case.account_created_at)), "inline": true }),
            json!({ "name": "Time since join", "value": fmt_delta(now, case.joined_at), "inline": true }),
            json!({ "name": "Activity window", "value": format!(
                "{} msgs / {} channels in {}s",
                case.meta[1], case.meta[0], dl_moderation::guard::WINDOW_SECONDS
            ), "inline": false }),
            json!({ "name": "Signals", "value": format!(
                "Keywords: {} | Attachments: {}",
                case.meta[3] != 0, case.meta[2]
            ), "inline": true }),
            json!({ "name": "Actions", "value": format!(
                "{action_text}\nDeleted: {}\nDM sent: {}",
                case.deleted_count, if case.dm_sent { "yes" } else { "no" }
            ), "inline": true }),
            json!({ "name": "Reason", "value": reason_value, "inline": false }),
        ];
        if !preview.is_empty() {
            fields.push(json!({ "name": "Nachrichten", "value": preview, "inline": false }));
        }
        let embed = json!({
            "title": title,
            "color": color,
            "fields": fields,
        });
        // Ban/Timeout-aufheben immer (Mod-Aktionen); „Entbannen" wie das
        // Original (`UnbanView`) nur, wenn tatsächlich gebannt wurde.
        let mut buttons = vec![
            json!({ "type": 2, "style": 4, "label": "Ban",
                    "custom_id": format!("sg:ban:{}:{}", case.guild_id, case.user_id) }),
            json!({ "type": 2, "style": 3, "label": "Timeout aufheben",
                    "custom_id": format!("sg:untimeout:{}:{}", case.guild_id, case.user_id) }),
        ];
        if is_ban && case.action_ok {
            buttons.push(json!({ "type": 2, "style": 2, "label": "Entbannen",
                "custom_id": format!("sg:unban:{}:{}", case.guild_id, case.user_id) }));
        }
        let components = json!([{ "type": 1, "components": buttons }]);
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

/// sg:*-Mod-Buttons (Ban / Timeout aufheben / Unban) mit Rechte-Guard,
/// plus der User-seitige Einspruch-Flow (appeal / appealsubmit — ohne
/// Rechte-Guard, da vom betroffenen User in der DM ausgelöst).
pub struct GuardReviewHandler {
    pub adapter: Arc<DiscordAdapter>,
}

impl GuardReviewHandler {
    /// Einspruch-Button → Einspruch-Modal (Original: AppealView → AppealModal).
    fn open_appeal_modal(case_id: &str) -> BridgeReply {
        BridgeReply {
            modal: Some(dl_discord::ModalSpec {
                custom_id: format!("sg:appealsubmit:{case_id}"),
                title: "Einspruch".to_string(),
                fields: vec![dl_discord::ModalField {
                    custom_id: "reason".to_string(),
                    label: "Grund für den Einspruch".to_string(),
                    placeholder: "Erkläre, warum dieser Bann überprüft werden sollte.".to_string(),
                    required: true,
                    min_length: dl_moderation::guard::APPEAL_MIN_CHARS,
                    max_length: dl_moderation::guard::APPEAL_MAX_CHARS,
                    paragraph: true,
                }],
            }),
            ..BridgeReply::default()
        }
    }

    /// Einspruch-Modal abgeschickt → Embed in den Mod-Kanal + Bestätigung an
    /// den User (Original: `handle_appeal_submission`).
    async fn submit_appeal(&self, interaction: &BridgeInteraction, case_id: &str) -> BridgeReply {
        let appeal_text = interaction
            .options
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace('`', "'")
            .trim()
            .to_string();
        let safe_appeal: String = if appeal_text.is_empty() {
            "(leer)".to_string()
        } else {
            appeal_text.chars().take(1000).collect()
        };
        let embed = json!({
            "title": "Einspruch eingegangen",
            "color": 0x3498DB,
            "fields": [
                { "name": "Mitglied", "value": format!("<@{0}> ({0})", interaction.user_id), "inline": false },
                { "name": "Fall-ID", "value": case_id, "inline": true },
                { "name": "Begründung des Einspruchs", "value": safe_appeal, "inline": false },
            ],
        });
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::guard::MOD_CHANNEL_ID, &body)
            .await;
        BridgeReply::ephemeral_text("Dein Einspruch wurde an das Mod-Team weitergeleitet.")
    }
}

#[async_trait::async_trait]
impl InteractionHandler for GuardReviewHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let rest = interaction
            .custom_id
            .strip_prefix("sg:")
            .unwrap_or_default();
        // Einspruch-Flow (vom betroffenen User, kein Mod-Recht nötig):
        // custom_id `sg:appeal:{case_id}` / `sg:appealsubmit:{case_id}`.
        if let Some(case_id) = rest.strip_prefix("appeal:") {
            return Self::open_appeal_modal(case_id);
        }
        if let Some(case_id) = rest.strip_prefix("appealsubmit:") {
            return self.submit_appeal(&interaction, case_id).await;
        }

        // Alle übrigen sg:*-Aktionen sind Mod-Buttons → Rechte-Guard.
        if !interaction.author_can_manage_roles {
            return BridgeReply::ephemeral_text("Keine Berechtigung.");
        }
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
                    "Entbannen fehlgeschlagen."
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
        let Some(guild) = self.adapter.cache().guild(GuildId::new(self.guild_id)) else {
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
            .cache()
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
            .cache()
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
        let bot_id = self.adapter.cache().current_user().id.get();
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
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .and_then(|c| c.parent_id.map(|p| p.get()))
    }

    async fn user_name(&self, user_id: u64) -> String {
        self.adapter
            .cache()
            .user(UserId::new(user_id))
            .map(|u| u.name.to_string())
            .unwrap_or_else(|| format!("user-{user_id}"))
    }

    async fn post_rich(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn delete_panel(&self, channel_id: u64, message_id: u64) {
        let _ = self
            .adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("FAQ: Duplikat-Panel aufräumen"),
            )
            .await;
    }
}

// ── DM-Assistent-Anbindung ─────────────────────────────────────────────────

pub struct DmGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::dm_assistant::DmPort for DmGlue {
    async fn send_dm(&self, channel_id: u64, body: serde_json::Map<String, serde_json::Value>) {
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }
}

// ── Anonymes-Feedback-Anbindung ────────────────────────────────────────────

pub struct FeedbackGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::feedback_hub::FeedbackPort for FeedbackGlue {
    async fn send_dm_text(&self, user_id: u64, text: String) -> Result<(), String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|e| e.to_string())?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => Ok(()),
            // 50007 = Cannot send messages to this user (DMs zu) → nicht actionbar
            Err(err) if err.to_string().contains("50007") => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }

    async fn post_rich(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
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
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
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
            .cache()
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
            .cache()
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
            .cache()
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
            .cache()
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
            .cache()
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
        let Some(guild) = self.adapter.cache().guild(guild_id) else {
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

    async fn member_voice_channel_in_category(
        &self,
        guild_id: u64,
        user_id: u64,
        category_id: u64,
    ) -> Option<u64> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let channel_id = guild.voice_states.get(&UserId::new(user_id))?.channel_id?;
        let parent = guild.channels.get(&channel_id)?.parent_id?;
        (parent.get() == category_id).then(|| channel_id.get())
    }

    async fn send_dm_embed(&self, user_id: u64, embed: serde_json::Value) -> bool {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return false;
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .is_ok()
    }
}

// ── Retention-Miss-You-Anbindung ───────────────────────────────────────────

pub struct RetentionGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::retention::RetentionPort for RetentionGlue {
    async fn member_info(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Option<dl_community::retention::RetentionMember> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let member = guild.members.get(&UserId::new(user_id))?;
        Some(dl_community::retention::RetentionMember {
            display_name: member.display_name().to_string(),
            role_ids: member.roles.iter().map(|r| r.get()).collect(),
        })
    }

    async fn fetch_user_name(&self, user_id: u64) -> Option<String> {
        let user = self.adapter.http.get_user(UserId::new(user_id)).await.ok()?;
        // Discord-Präzedenz: global_name vor Username (wie resolve_user).
        Some(
            user.global_name
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| user.name.to_string()),
        )
    }

    async fn guild_label(&self, guild_id: u64) -> (String, Option<String>) {
        match self.adapter.cache().guild(GuildId::new(guild_id)) {
            Some(g) => (g.name.to_string(), g.icon_url()),
            // Python-Fallback, wenn die Gilde nicht im Cache ist.
            None => ("unserem Server".to_string(), None),
        }
    }

    async fn send_miss_you_dm(
        &self,
        user_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> dl_community::retention::MissYouDelivery {
        use dl_community::retention::MissYouDelivery;
        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) => return MissYouDelivery::Failed(err.to_string()),
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => MissYouDelivery::Sent,
            // 50007 = Cannot send messages to this user (DMs deaktiviert).
            Err(err) if err.contains("50007") => MissYouDelivery::Blocked,
            Err(err) => MissYouDelivery::Failed(err),
        }
    }
}

// ── Team-Balancer-Anbindung (!balance) ─────────────────────────────────────

pub struct BalanceGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub db: dl_db::Db,
}

#[async_trait::async_trait]
impl dl_tournament::balance_cmd::BalancePort for BalanceGlue {
    async fn caller_voice_members(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Vec<dl_tournament::balance_cmd::VoiceMember> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        let Some(channel_id) = guild
            .voice_states
            .get(&UserId::new(user_id))
            .and_then(|vs| vs.channel_id)
        else {
            return Vec::new();
        };
        guild
            .voice_states
            .iter()
            .filter(|(_, vs)| vs.channel_id == Some(channel_id))
            .filter_map(|(uid, _)| {
                let member = guild.members.get(uid)?;
                if member.user.bot {
                    return None;
                }
                Some(dl_tournament::balance_cmd::VoiceMember {
                    user_id: uid.get(),
                    display_name: member.display_name().to_string(),
                    role_ids: member.roles.iter().map(|r| r.get()).collect(),
                })
            })
            .collect()
    }

    async fn resolve_member(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Option<dl_tournament::balance_cmd::VoiceMember> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let member = guild.members.get(&UserId::new(user_id))?;
        Some(dl_tournament::balance_cmd::VoiceMember {
            user_id,
            display_name: member.display_name().to_string(),
            role_ids: member.roles.iter().map(|r| r.get()).collect(),
        })
    }

    async fn db_rank(&self, user_id: u64) -> Option<String> {
        use rusqlite::OptionalExtension;
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT rank FROM user_ranks WHERE user_id = ?1",
                    [user_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .flatten()
    }

    async fn caller_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        Some(guild.voice_states.get(&UserId::new(user_id))?.channel_id?.get())
    }

    async fn create_match_channel(
        &self,
        guild_id: u64,
        name: &str,
        category_id: u64,
    ) -> Option<u64> {
        let mut body = serde_json::Map::new();
        body.insert("name".into(), json!(name));
        body.insert("type".into(), json!(2)); // 2 = Voice-Channel
        body.insert("parent_id".into(), json!(category_id.to_string()));
        self.adapter
            .http
            .create_channel(GuildId::new(guild_id), &body, Some("Team-Balancer: Match-Channel"))
            .await
            .ok()
            .map(|c| c.id.get())
    }

    async fn move_member(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: u64,
    ) -> dl_tournament::balance_cmd::MoveOutcome {
        use dl_tournament::balance_cmd::MoveOutcome;
        // Nur verschieben, wenn das Mitglied gerade in einem Voice-Channel ist —
        // sonst liefert Discord 40032 („Target user is not connected to voice“).
        let in_voice = self
            .adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| g.voice_states.get(&UserId::new(user_id)).and_then(|vs| vs.channel_id))
            .is_some();
        if !in_voice {
            return MoveOutcome::NotInVoice;
        }
        match self
            .adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "channel_id": channel_id.to_string() }),
                Some("Team-Balancer: Move"),
            )
            .await
        {
            Ok(_) => MoveOutcome::Moved,
            Err(err) => MoveOutcome::Failed(err.to_string()),
        }
    }

    async fn channel_member_count(&self, guild_id: u64, channel_id: u64) -> Option<(String, usize)> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let channel = ChannelId::new(channel_id);
        let name = guild.channels.get(&channel)?.name.to_string();
        // Nicht-Bot-Mitglieder im Voice-Channel zählen.
        let count = guild
            .voice_states
            .iter()
            .filter(|(_, vs)| vs.channel_id == Some(channel))
            .filter(|(uid, _)| guild.members.get(uid).map(|m| !m.user.bot).unwrap_or(false))
            .count();
        Some((name, count))
    }

    async fn delete_channel(&self, channel_id: u64) -> bool {
        self.adapter
            .http
            .delete_channel(ChannelId::new(channel_id), Some("Team-Balancer: Match beendet"))
            .await
            .is_ok()
    }

    async fn can_manage_channels(&self, guild_id: u64, user_id: u64) -> bool {
        // Python: @commands.has_permissions(manage_channels=True) (+ Admin/Owner).
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
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
                        .map(|r| r.permissions.manage_channels() || r.permissions.administrator())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }
}
