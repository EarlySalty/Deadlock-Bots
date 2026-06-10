//! serenity-Adapter: implementiert die Ports von dl-broker und dl-changelog.
//!
//! REST-Aktionen laufen über `serenity::http::Http` und brauchen KEIN
//! Gateway. Cache-abhängige Lesepfade (Voice-Member, Rollen-Mitglieder,
//! Member-Resolution) nutzen den Gateway-Cache, sobald er verbunden ist —
//! vorher liefern sie einen sauberen Fehler statt stiller Falschdaten.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dl_broker::port::{
    DiscordPort, GuildRoles, InviteInfo, MemberInfo, PortError, RichMessage, RoleInfo, RoleMembers,
    ViewSpec,
};
use dl_changelog::{ChangelogDiscord, ChangelogError};
use serde_json::{json, Map, Value};
use serenity::all::{Cache, ChannelId, ChannelType, GuildId, Http, MessageId, RoleId, UserId};

pub struct DiscordAdapter {
    pub http: Arc<Http>,
    /// Gateway-Cache — erst nach Gateway-Start befüllt.
    pub cache: Arc<Cache>,
    /// Vom Gateway-Handler gesetzt, sobald READY empfangen wurde.
    pub gateway_ready: Arc<AtomicBool>,
}

impl DiscordAdapter {
    pub fn new(token: &str) -> Arc<Self> {
        Arc::new(Self {
            http: Arc::new(Http::new(token)),
            cache: Arc::new(Cache::new()),
            gateway_ready: Arc::new(AtomicBool::new(false)),
        })
    }

    fn cache_ready(&self) -> bool {
        self.gateway_ready.load(Ordering::Relaxed)
    }

    /// Erste Guild (Diagnose-Default wie Python `bot.guilds[0]`).
    fn first_guild(&self) -> Option<GuildId> {
        self.cache.guilds().first().copied()
    }

    /// view_spec → Discord-Komponenten (Action-Row mit einem Button).
    ///
    /// twitch_live_tracking baut die custom_id EXAKT wie
    /// TwitchLiveTrackingView.build_custom_id — der Klick wird vom
    /// Gateway-Besitzer verarbeitet (Kopplung siehe rust/docs/03-phase2.md).
    fn view_components(spec: &ViewSpec) -> Value {
        match spec {
            ViewSpec::LinkButton { label, url } => json!([{
                "type": 1,
                "components": [{
                    "type": 2,
                    "style": 5,
                    "label": label,
                    "url": url,
                }],
            }]),
            ViewSpec::TwitchLiveTracking {
                streamer_login,
                tracking_token,
                button_label,
                ..
            } => {
                let login_part: String = streamer_login
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .take(24)
                    .collect();
                let login_part = if login_part.is_empty() {
                    "stream".to_string()
                } else {
                    login_part
                };
                let token_part: String = tracking_token.chars().take(32).collect();
                let token_part = if token_part.is_empty() {
                    "track".to_string()
                } else {
                    token_part
                };
                let label = if button_label.trim().is_empty() {
                    "Auf Twitch ansehen"
                } else {
                    button_label.trim()
                };
                json!([{
                    "type": 1,
                    "components": [{
                        "type": 2,
                        "style": 1,
                        "label": label.chars().take(80).collect::<String>(),
                        "custom_id": format!("twitch-live:{login_part}:{token_part}"),
                    }],
                }])
            }
        }
    }

    fn allowed_mentions(user_ids: &[u64], role_ids: &[u64]) -> Value {
        // Wie _build_allowed_mentions: everyone aus, nur explizite IDs.
        json!({
            "parse": [],
            "users": user_ids.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
            "roles": role_ids.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
            "replied_user": false,
        })
    }

    fn rich_body(message: &RichMessage) -> Map<String, Value> {
        let mut body = Map::new();
        body.insert("content".into(), json!(message.content));
        body.insert("embeds".into(), json!([message.embed]));
        body.insert(
            "allowed_mentions".into(),
            Self::allowed_mentions(&message.allowed_user_ids, &message.allowed_role_ids),
        );
        if let Some(spec) = &message.view_spec {
            body.insert("components".into(), Self::view_components(spec));
        }
        body
    }

    async fn open_dm(&self, user_id: u64) -> Result<ChannelId, PortError> {
        // Existenz des Users prüfen (Python: _resolve_user → 404)
        if self.http.get_user(UserId::new(user_id)).await.is_err() {
            return Err(PortError::UserNotFound);
        }
        let channel = self
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|_| PortError::DmOpenFailed)?;
        Ok(channel.id)
    }

    async fn send_raw(
        &self,
        channel_id: u64,
        body: &Map<String, Value>,
    ) -> Result<u64, serenity::Error> {
        let message = self
            .http
            .send_message(ChannelId::new(channel_id), Vec::new(), body)
            .await?;
        Ok(message.id.get())
    }

    fn is_unknown_channel(err: &serenity::Error) -> bool {
        // Discord-Fehlercode 10003 = Unknown Channel; 404 generell als
        // "nicht gefunden" werten (wie Pythons get/fetch-Fallbacks).
        if let serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp)) = err {
            return resp.status_code.as_u16() == 404;
        }
        false
    }
}

#[async_trait::async_trait]
impl DiscordPort for DiscordAdapter {
    async fn is_ready(&self) -> bool {
        self.cache_ready()
    }

    async fn send_channel_message(&self, channel_id: u64, content: &str) -> Result<u64, PortError> {
        let mut body = Map::new();
        body.insert("content".into(), json!(content));
        self.send_raw(channel_id, &body).await.map_err(|err| {
            if Self::is_unknown_channel(&err) {
                PortError::ChannelNotFound
            } else {
                PortError::Discord(err.to_string())
            }
        })
    }

    async fn send_dm(&self, user_id: u64, content: &str) -> Result<(Option<u64>, u64), PortError> {
        let dm_channel = self.open_dm(user_id).await?;
        let mut body = Map::new();
        body.insert("content".into(), json!(content));
        let message_id = self
            .send_raw(dm_channel.get(), &body)
            .await
            .map_err(|err| PortError::Discord(err.to_string()))?;
        Ok((Some(dm_channel.get()), message_id))
    }

    async fn create_text_channel(
        &self,
        category_id: u64,
        name: &str,
        topic: Option<&str>,
    ) -> Result<u64, PortError> {
        let category = self
            .http
            .get_channel(ChannelId::new(category_id))
            .await
            .map_err(|_| PortError::CategoryNotFound)?;
        let guild_channel = category.guild().ok_or(PortError::CategoryNotFound)?;
        if guild_channel.kind != ChannelType::Category {
            return Err(PortError::CategoryNotFound);
        }
        let guild_id = guild_channel.guild_id;

        let mut body = Map::new();
        body.insert("name".into(), json!(name));
        body.insert("type".into(), json!(0));
        body.insert("parent_id".into(), json!(category_id.to_string()));
        if let Some(topic) = topic {
            body.insert("topic".into(), json!(topic));
        }
        let created = self
            .http
            .create_channel(guild_id, &body, Some("master-broker:create-channel"))
            .await
            .map_err(|err| PortError::Discord(err.to_string()))?;
        Ok(created.id.get())
    }

    async fn delete_channel(&self, channel_id: u64) -> Result<(), PortError> {
        self.http
            .delete_channel(
                ChannelId::new(channel_id),
                Some("master-broker:delete-channel"),
            )
            .await
            .map(|_| ())
            .map_err(|err| {
                if Self::is_unknown_channel(&err) {
                    PortError::ChannelNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })
    }

    async fn send_rich_message(&self, message: &RichMessage) -> Result<u64, PortError> {
        let body = Self::rich_body(message);
        self.send_raw(message.channel_id, &body)
            .await
            .map_err(|err| {
                if Self::is_unknown_channel(&err) {
                    PortError::ChannelNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })
    }

    async fn edit_rich_message(
        &self,
        message_id: u64,
        message: &RichMessage,
    ) -> Result<(), PortError> {
        let body = Self::rich_body(message);
        self.http
            .edit_message(
                ChannelId::new(message.channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|err| {
                if Self::is_unknown_channel(&err) {
                    PortError::MessageNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })
    }

    async fn add_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), PortError> {
        self.http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some(reason),
            )
            .await
            .map_err(|err| PortError::Discord(err.to_string()))
    }

    async fn remove_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), PortError> {
        self.http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some(reason),
            )
            .await
            .map_err(|err| PortError::Discord(err.to_string()))
    }

    async fn move_voice(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: Option<u64>,
    ) -> Result<(), PortError> {
        let body = json!({ "channel_id": channel_id.map(|v| v.to_string()) });
        self.http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &body,
                Some("master-broker:move-voice"),
            )
            .await
            .map(|_| ())
            .map_err(|err| PortError::Discord(err.to_string()))
    }

    async fn voice_members(&self, channel_id: u64) -> Result<Vec<MemberInfo>, PortError> {
        if !self.cache_ready() {
            return Err(PortError::Discord(
                "gateway not connected (voice members need the cache)".to_string(),
            ));
        }
        for guild_id in self.cache.guilds() {
            let Some(guild) = self.cache.guild(guild_id) else {
                continue;
            };
            if !guild.channels.contains_key(&ChannelId::new(channel_id)) {
                continue;
            }
            let members = guild
                .voice_states
                .iter()
                .filter(|(_, vs)| vs.channel_id == Some(ChannelId::new(channel_id)))
                .filter_map(|(user_id, _)| {
                    guild.members.get(user_id).map(|m| MemberInfo {
                        user_id: user_id.get(),
                        display_name: m.display_name().to_string(),
                    })
                })
                .collect();
            return Ok(members);
        }
        Err(PortError::ChannelNotFound)
    }

    async fn create_invite(&self, channel_id: u64, reason: &str) -> Result<InviteInfo, PortError> {
        let invite = self
            .http
            .create_invite(
                ChannelId::new(channel_id),
                &json!({ "max_age": 0, "max_uses": 0, "unique": true }),
                Some(reason),
            )
            .await
            .map_err(|err| {
                if Self::is_unknown_channel(&err) {
                    PortError::ChannelNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })?;
        let guild_id = invite.guild.as_ref().map(|g| g.id.get()).unwrap_or(0);
        Ok(InviteInfo {
            invite_url: format!("https://discord.gg/{}", invite.code),
            code: invite.code,
            guild_id,
        })
    }

    async fn list_roles(&self, guild_id: Option<u64>) -> Result<GuildRoles, PortError> {
        let guild_id = match guild_id {
            Some(id) => GuildId::new(id),
            None => self.first_guild().ok_or(PortError::GuildNotFound)?,
        };
        let Some(guild) = self.cache.guild(guild_id) else {
            return Err(PortError::GuildNotFound);
        };
        // member_count pro Rolle aus dem Member-Cache (chunked vorausgesetzt)
        let mut counts: std::collections::HashMap<RoleId, usize> = std::collections::HashMap::new();
        for member in guild.members.values() {
            for role in &member.roles {
                *counts.entry(*role).or_default() += 1;
            }
        }
        let roles = guild
            .roles
            .values()
            .map(|role| RoleInfo {
                id: role.id.get(),
                name: role.name.clone(),
                position: i64::from(role.position),
                member_count: counts.get(&role.id).copied().unwrap_or(0),
            })
            .collect();
        Ok(GuildRoles {
            guild_id: guild_id.get(),
            chunked: true,
            roles,
        })
    }

    async fn role_members(
        &self,
        guild_id: Option<u64>,
        role_id: u64,
    ) -> Result<RoleMembers, PortError> {
        let guild_id = match guild_id {
            Some(id) => GuildId::new(id),
            None => self.first_guild().ok_or(PortError::GuildNotFound)?,
        };
        let Some(guild) = self.cache.guild(guild_id) else {
            return Err(PortError::GuildNotFound);
        };
        let role = guild
            .roles
            .get(&RoleId::new(role_id))
            .ok_or(PortError::RoleNotFound)?;
        let members = guild
            .members
            .values()
            .filter(|m| m.roles.contains(&RoleId::new(role_id)))
            .map(|m| MemberInfo {
                user_id: m.user.id.get(),
                display_name: m.display_name().to_string(),
            })
            .collect();
        Ok(RoleMembers {
            role_id,
            name: role.name.clone(),
            members,
        })
    }
}

#[async_trait::async_trait]
impl ChangelogDiscord for DiscordAdapter {
    async fn send(
        &self,
        channel_id: u64,
        content: Option<&str>,
        embeds: &[Value],
        mention_roles: bool,
    ) -> Result<u64, ChangelogError> {
        let mut body = Map::new();
        if let Some(content) = content {
            body.insert("content".into(), json!(content));
        }
        body.insert("embeds".into(), json!(embeds));
        if mention_roles {
            body.insert(
                "allowed_mentions".into(),
                json!({ "parse": ["roles"], "replied_user": false }),
            );
        }
        self.send_raw(channel_id, &body).await.map_err(|err| {
            if Self::is_unknown_channel(&err) {
                ChangelogError::ChannelNotFound(channel_id)
            } else {
                ChangelogError::Discord(err.to_string())
            }
        })
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) -> Result<(), ChangelogError> {
        self.http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("changelog: replace previous post"),
            )
            .await
            .map_err(|err| ChangelogError::Discord(err.to_string()))
    }

    async fn send_file(
        &self,
        channel_id: u64,
        content: &str,
        path: &Path,
    ) -> Result<(), ChangelogError> {
        let attachment = serenity::builder::CreateAttachment::path(path)
            .await
            .map_err(|err| ChangelogError::Discord(err.to_string()))?;
        let mut body = Map::new();
        body.insert("content".into(), json!(content));
        self.http
            .send_message(ChannelId::new(channel_id), vec![attachment], &body)
            .await
            .map(|_| ())
            .map_err(|err| ChangelogError::Discord(err.to_string()))
    }

    async fn fetch_message(
        &self,
        channel_id: u64,
        message_id: u64,
    ) -> Result<Value, ChangelogError> {
        let message = self
            .http
            .get_message(ChannelId::new(channel_id), MessageId::new(message_id))
            .await
            .map_err(|err| {
                if Self::is_unknown_channel(&err) {
                    ChangelogError::ChannelNotFound(channel_id)
                } else {
                    ChangelogError::Discord(err.to_string())
                }
            })?;
        Ok(serialize_message_py(&message))
    }

    async fn fetch_history(
        &self,
        channel_id: u64,
        limit: u8,
        before_id: Option<u64>,
    ) -> Result<Vec<Value>, ChangelogError> {
        let pagination = before_id
            .map(|before| serenity::http::MessagePagination::Before(MessageId::new(before)));
        let messages = self
            .http
            .get_messages(ChannelId::new(channel_id), pagination, Some(limit))
            .await
            .map_err(|err| {
                if Self::is_unknown_channel(&err) {
                    ChangelogError::ChannelNotFound(channel_id)
                } else {
                    ChangelogError::Discord(err.to_string())
                }
            })?;
        Ok(messages.iter().map(serialize_message_py).collect())
    }
}

/// Serialisierung im Format von ChangelogPublisher._serialize_message.
fn serialize_message_py(message: &serenity::all::Message) -> Value {
    let author_name = match message.author.discriminator {
        Some(d) => format!("{}#{:04}", message.author.name, d),
        None => message.author.name.to_string(),
    };
    json!({
        "id": message.id.get(),
        "created_at": message.id.created_at().to_rfc3339(),
        "author": {
            "id": message.author.id.get(),
            "name": author_name,
            "bot": message.author.bot,
        },
        "content": message.content,
        "embeds": message.embeds,
        "attachments": message.attachments.iter().map(|a| json!({
            "filename": a.filename,
            "url": a.url,
        })).collect::<Vec<_>>(),
    })
}
