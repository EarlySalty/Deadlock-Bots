//! serenity-Adapter: implementiert die Ports von dl-broker und dl-changelog.
//!
//! REST-Aktionen laufen über `serenity::http::Http` und brauchen KEIN
//! Gateway. Cache-abhängige Lesepfade (Voice-Member, Rollen-Mitglieder,
//! Member-Resolution) nutzen den Gateway-Cache, sobald er verbunden ist —
//! vorher liefern sie einen sauberen Fehler statt stiller Falschdaten.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use dl_broker::port::{
    DiscordPort, GuildMemberInfo, GuildRoles, GuildStats, InviteInfo, MemberAccess, MemberInfo,
    MemberPresence, MessageReaction, PortError, ResolvedUser, RichMessage, RoleInfo, RoleMembers,
    ViewSpec,
};
use dl_changelog::{ChangelogDiscord, ChangelogError};
use serde_json::{json, Map, Value};
use serenity::all::{
    Cache, ChannelId, ChannelType, GuildId, Http, MessageId, ReactionType, RoleId, UserId,
};
use serenity::builder::CreateAttachment;

pub struct DiscordAdapter {
    pub http: Arc<Http>,
    /// Gateway-Cache. WICHTIG: serenity erlaubt keine Cache-Injektion und legt
    /// beim Client-Build einen EIGENEN Cache an. Dieser hier wird nach dem Build
    /// via [`Self::link_cache`] an genau diesen serenity-Cache gekoppelt — sonst
    /// läse die gesamte Glue aus einem leeren Cache (alle Lookups None).
    cache: OnceLock<Arc<Cache>>,
    /// Vom Gateway-Handler gesetzt, sobald READY empfangen wurde.
    pub gateway_ready: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReactedUser {
    pub id: u64,
    pub is_bot: bool,
}

impl DiscordAdapter {
    pub fn new(token: &str) -> Arc<Self> {
        Arc::new(Self {
            http: Arc::new(Http::new(token)),
            cache: OnceLock::new(),
            gateway_ready: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Koppelt den Adapter an serenitys vom Gateway befüllten Client-Cache.
    /// Einmalig direkt nach dem Client-Build aufzurufen.
    pub fn link_cache(&self, cache: Arc<Cache>) {
        let _ = self.cache.set(cache);
    }

    /// Lädt die Application-ID per REST und setzt sie auf dem Http-Client.
    ///
    /// Nötig für Interaction-Followups: die gehen an
    /// `POST /webhooks/{application_id}/{token}`. Dieser Adapter-`Http` ist eine
    /// EIGENE Instanz, getrennt vom serenity-Client-`Http` (der die ID beim READY
    /// bekommt) — ohne gesetzte ID schlägt jeder Followup nach einem Defer mit
    /// „Application id was expected but missing" fehl, und der User sieht nichts.
    /// Einmalig beim Start aufzurufen; idempotent.
    pub async fn init_application_id(&self) -> serenity::Result<()> {
        if self.http.application_id().is_some() {
            return Ok(());
        }
        let info = self.http.get_current_application_info().await?;
        self.http.set_application_id(info.id);
        Ok(())
    }

    /// Der Gateway-Cache. Vor dem Koppeln ein leerer Fallback (Lookups → None).
    pub fn cache(&self) -> &Cache {
        self.cache.get().map(Arc::as_ref).unwrap_or_else(|| {
            static EMPTY: OnceLock<Cache> = OnceLock::new();
            EMPTY.get_or_init(Cache::new)
        })
    }

    pub async fn member_role_ids_or_fetch(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        let gid = GuildId::new(guild_id);
        let uid = UserId::new(user_id);
        let cached_roles: Vec<u64> = self
            .cache()
            .guild(gid)
            .and_then(|guild| {
                guild
                    .members
                    .get(&uid)
                    .map(|member| member.roles.iter().map(|role_id| role_id.get()).collect())
            })
            .unwrap_or_default();
        if !cached_roles.is_empty() {
            return cached_roles;
        }

        match self.http.get_member(gid, uid).await {
            Ok(member) => member.roles.iter().map(|role_id| role_id.get()).collect(),
            Err(err) => {
                tracing::debug!(
                    %err,
                    guild_id,
                    user_id,
                    "Discord-Memberrollen per REST-Fallback nicht abrufbar"
                );
                Vec::new()
            }
        }
    }

    fn cache_ready(&self) -> bool {
        self.gateway_ready.load(Ordering::Relaxed)
    }

    /// Erste Guild (Diagnose-Default wie Python `bot.guilds[0]`).
    fn first_guild(&self) -> Option<GuildId> {
        self.cache().guilds().first().copied()
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
            ViewSpec::ScamRevoke { verdict_id, .. } => json!([{
                "type": 1,
                "components": [{
                    "type": 2,
                    "style": 4,
                    "label": "Rückgängig",
                    "custom_id": format!("scam-revoke:{verdict_id}"),
                }],
            }]),
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
        if let Some(components) = &message.components {
            let mut component_rows = components.as_array().cloned().unwrap_or_default();
            if let Some(spec) = &message.view_spec {
                let view_components = Self::view_components(spec);
                if let Some(rows) = view_components.as_array() {
                    component_rows.extend(rows.iter().cloned());
                }
            }
            body.insert("flags".into(), json!(1u64 << 15));
            body.insert("components".into(), Value::Array(component_rows));
            body.insert(
                "allowed_mentions".into(),
                Self::allowed_mentions(&message.allowed_user_ids, &message.allowed_role_ids),
            );
            return body;
        }

        body.insert("content".into(), json!(message.content));
        if message
            .embed
            .as_object()
            .is_some_and(|embed| !embed.is_empty())
        {
            body.insert("embeds".into(), json!([message.embed]));
        }
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

    /// Öffentliche Variante für den Interaction-Dispatch (Panel-Posts).
    pub async fn send_raw_public(
        &self,
        channel_id: u64,
        body: &Map<String, Value>,
    ) -> Result<u64, String> {
        self.send_raw(channel_id, body)
            .await
            .map_err(|err| err.to_string())
    }

    /// Sendet einen rohen Discord-API-Body per DM. Ein- und Ausgabe-IDs bleiben
    /// Snowflake-Strings, damit Aufrufer keine praezisionskritischen JSON-Zahlen bauen.
    pub async fn send_raw_dm_public(
        &self,
        recipient_user_id: &str,
        body: &Map<String, Value>,
    ) -> Result<(String, String), String> {
        let user_id = recipient_user_id
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| "Discord-DM-Recipient-ID ungueltig".to_string())?;
        let dm_channel = self
            .open_dm(user_id)
            .await
            .map_err(|err| format!("Discord-DM konnte nicht geoeffnet werden: {err}"))?;
        let message_id = self
            .send_raw(dm_channel.get(), body)
            .await
            .map_err(|err| err.to_string())?;
        Ok((dm_channel.get().to_string(), message_id.to_string()))
    }

    /// Öffentliche Variante, wenn Aufrufer den typisierten Serenity-Fehler brauchen.
    pub async fn send_raw_public_typed(
        &self,
        channel_id: u64,
        body: &Map<String, Value>,
    ) -> Result<u64, serenity::Error> {
        self.send_raw(channel_id, body).await
    }

    /// Sendet eine lokale Datei als Attachment und liefert die Discord-Message-ID.
    pub async fn send_attachment_public(
        &self,
        channel_id: u64,
        content: Option<&str>,
        path: &Path,
    ) -> Result<u64, serenity::Error> {
        let attachment = CreateAttachment::path(path).await?;
        let mut body = Map::new();
        if let Some(content) = content {
            body.insert("content".into(), json!(content));
        }
        let message = self
            .http
            .send_message(ChannelId::new(channel_id), vec![attachment], &body)
            .await?;
        Ok(message.id.get())
    }

    /// Öffentliche Variante für Panel-Restore/-Edit aus dem Interaction-Dispatch.
    pub async fn edit_raw_public(
        &self,
        channel_id: u64,
        message_id: u64,
        body: &Map<String, Value>,
    ) -> Result<(), serenity::Error> {
        self.http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                body,
                Vec::new(),
            )
            .await
            .map(|_| ())
    }

    pub async fn reaction_users(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionType,
        after: Option<u64>,
    ) -> Result<Vec<ReactedUser>, serenity::Error> {
        let users = self
            .http
            .get_reaction_users(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                emoji,
                100,
                after,
            )
            .await?;
        Ok(users
            .into_iter()
            .map(|user| ReactedUser {
                id: user.id.get(),
                is_bot: user.bot,
            })
            .collect())
    }

    fn move_voice_channel_precheck(
        channel_guild_id: Option<u64>,
        expected_guild_id: u64,
    ) -> Result<(), PortError> {
        match channel_guild_id {
            Some(guild_id) if guild_id == expected_guild_id => Ok(()),
            _ => Err(PortError::ChannelNotFound),
        }
    }

    fn reaction_emoji(reaction_type: &ReactionType) -> String {
        match reaction_type {
            ReactionType::Unicode(value) => value.clone(),
            ReactionType::Custom { id, name, .. } => {
                format!("{}:{}", name.as_deref().unwrap_or_default(), id.get())
            }
            _ => String::new(),
        }
    }

    fn reaction_type_for_rest(emoji: &str) -> ReactionType {
        ReactionType::Unicode(emoji.to_string())
    }

    fn is_unknown_channel(err: &serenity::Error) -> bool {
        // Discord-Fehlercode 10003 = Unknown Channel; 404 generell als
        // "nicht gefunden" werten (wie Pythons get/fetch-Fallbacks).
        if let serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp)) = err {
            return resp.status_code.as_u16() == 404;
        }
        false
    }

    /// REST-404 (Unknown Member / Unknown User) = bestätigt abwesend.
    /// Jeder andere Fehler (Rate-Limit/5xx/Transport) ist NICHT „abwesend".
    fn is_http_404(err: &serenity::Error) -> bool {
        matches!(
            err,
            serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp))
                if resp.status_code.as_u16() == 404
        )
    }

    fn is_unknown_member(err: &serenity::Error) -> bool {
        matches!(
            err,
            serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(resp))
                if Self::is_unknown_member_response(
                    resp.status_code.as_u16(),
                    resp.error.code,
                    &resp.error.message,
                )
        )
    }

    fn is_unknown_member_response(status_code: u16, discord_code: isize, message: &str) -> bool {
        discord_code == 10007 || (status_code == 404 && message == "Unknown Member")
    }

    /// Kickt ein Mitglied aus der Gilde (`DELETE guilds/{}/members/{}`).
    ///
    /// Idempotent: ist das Mitglied bereits weg (Unknown Member / 404), gilt der
    /// Kick als erfolgt und wird auf `Ok` abgebildet. Jeder andere Fehler
    /// (Rate-Limit, 5xx, fehlende Rechte) bleibt ein Fehler, damit ein stiller
    /// Ausfall nicht wie ein erfolgreicher Kick aussieht.
    pub async fn kick(&self, guild_id: u64, user_id: u64, reason: &str) -> Result<(), PortError> {
        match self
            .http
            .kick_member(GuildId::new(guild_id), UserId::new(user_id), Some(reason))
            .await
        {
            Ok(()) => Ok(()),
            Err(err) if Self::is_unknown_member(&err) => Ok(()),
            Err(err) => Err(PortError::Discord(err.to_string())),
        }
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

    async fn delete_message(
        &self,
        channel_id: u64,
        message_id: u64,
        reason: &str,
    ) -> Result<(), PortError> {
        self.http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some(reason),
            )
            .await
            .map_err(|err| {
                if Self::is_http_404(&err) {
                    PortError::MessageNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })
    }

    async fn fetch_message_reactions(
        &self,
        channel_id: u64,
        message_id: u64,
    ) -> Result<Vec<MessageReaction>, PortError> {
        let message = self
            .http
            .get_message(ChannelId::new(channel_id), MessageId::new(message_id))
            .await
            .map_err(|err| {
                if Self::is_http_404(&err) {
                    PortError::MessageNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })?;
        Ok(message
            .reactions
            .into_iter()
            .map(|reaction| MessageReaction {
                emoji: Self::reaction_emoji(&reaction.reaction_type),
                count: reaction.count,
            })
            .collect())
    }

    async fn add_reaction(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), PortError> {
        self.http
            .create_reaction(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &Self::reaction_type_for_rest(emoji),
            )
            .await
            .map_err(|err| {
                if Self::is_http_404(&err) {
                    PortError::MessageNotFound
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

    async fn create_role(
        &self,
        guild_id: u64,
        name: &str,
        mentionable: bool,
        reason: &str,
    ) -> Result<u64, PortError> {
        // Idempotenz: existiert die Rolle bereits (case-sensitiv nach Name,
        // analog Python `discord.utils.get(name=...)`), deren ID zurückgeben.
        // Best-effort — wenn der Cache nicht ready ist (GuildNotFound o. ä.),
        // wird die Suche übersprungen und einfach neu angelegt.
        if let Ok(existing) = self.list_roles(Some(guild_id)).await {
            if let Some(role) = existing.roles.iter().find(|r| r.name == name) {
                return Ok(role.id);
            }
        }
        let role = self
            .http
            .create_role(
                GuildId::new(guild_id),
                &json!({ "name": name, "mentionable": mentionable }),
                Some(reason),
            )
            .await
            .map_err(|err| PortError::Discord(err.to_string()))?;
        Ok(role.id.get())
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
            .map_err(|err| {
                if Self::is_unknown_member(&err) {
                    PortError::MemberNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })
    }

    async fn move_voice(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: Option<u64>,
    ) -> Result<(), PortError> {
        let gid = GuildId::new(guild_id);
        let uid = UserId::new(user_id);
        let guild_cached = self.cache().guild(gid).is_some();
        if !guild_cached {
            self.http.get_guild(gid).await.map_err(|err| {
                if Self::is_http_404(&err) {
                    PortError::GuildNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })?;
        }

        let member_cached = self
            .cache()
            .guild(gid)
            .map(|guild| guild.members.contains_key(&uid))
            .unwrap_or(false);
        if !member_cached {
            self.http.get_member(gid, uid).await.map_err(|err| {
                if Self::is_http_404(&err) {
                    PortError::MemberNotFound
                } else {
                    PortError::Discord(err.to_string())
                }
            })?;
        }

        if let Some(channel_id) = channel_id {
            let cid = ChannelId::new(channel_id);
            let cached_channel_guild_id = self.cache().guild(gid).and_then(|guild| {
                guild
                    .channels
                    .get(&cid)
                    .map(|channel| channel.guild_id.get())
            });
            if cached_channel_guild_id.is_some() {
                Self::move_voice_channel_precheck(cached_channel_guild_id, guild_id)?;
            } else {
                let channel = self.http.get_channel(cid).await.map_err(|err| {
                    if Self::is_http_404(&err) {
                        PortError::ChannelNotFound
                    } else {
                        PortError::Discord(err.to_string())
                    }
                })?;
                let channel_guild_id = channel.guild().map(|channel| channel.guild_id.get());
                Self::move_voice_channel_precheck(channel_guild_id, guild_id)?;
            }
        }

        let body = json!({ "channel_id": channel_id.map(|v| v.to_string()) });
        self.http
            .edit_member(gid, uid, &body, Some("master-broker:move-voice"))
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
        for guild_id in self.cache().guilds() {
            let Some(guild) = self.cache().guild(guild_id) else {
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
        let Some(guild) = self.cache().guild(guild_id) else {
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
        let Some(guild) = self.cache().guild(guild_id) else {
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

    async fn member_access(
        &self,
        guild_id: Option<u64>,
        user_id: u64,
    ) -> Result<MemberAccess, PortError> {
        // Kandidaten-Gilden: eine bestimmte oder alle Bot-Gilden (wie Pythons
        // Fallback auf bot.guilds, wenn keine konfiguriert sind).
        let guild_ids: Vec<GuildId> = match guild_id {
            Some(id) => vec![GuildId::new(id)],
            None => self.cache().guilds(),
        };
        let target = UserId::new(user_id);

        let mut access = MemberAccess {
            user_id,
            ..Default::default()
        };
        for gid in guild_ids {
            let Some(guild) = self.cache().guild(gid) else {
                continue;
            };
            let Some(member) = guild.members.get(&target) else {
                continue;
            };
            // Admin-Permission wird über ALLE Gilden ge-OR-t.
            if guild.member_permissions(member).administrator() {
                access.is_administrator = true;
            }
            // Anzeigename + Rollen aus der ersten Gilde mit Treffer
            // (entspricht _fetch_discord_member_role_ids: erste Fundstelle).
            if !access.found {
                access.found = true;
                access.display_name = Some(member.display_name().to_string());
                access.role_ids = member
                    .roles
                    .iter()
                    .map(|role| role.get())
                    .filter(|rid| *rid != gid.get()) // @everyone hat die Gilden-ID
                    .collect();
            }
        }
        Ok(access)
    }

    async fn member_present(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<MemberPresence, PortError> {
        let gid = GuildId::new(guild_id);
        let uid = UserId::new(user_id);

        // 1. Cache zuerst (Python: guild.get_member) — kostenlos, kein API-Call.
        if let Some(guild) = self.cache().guild(gid) {
            if guild.members.contains_key(&uid) {
                return Ok(MemberPresence::Present);
            }
        }

        // 2. Cache-Miss → Live-REST (Python: guild.fetch_member). 404 = bestätigt
        //    abwesend, jeder andere Fehler = unbekannt (Rate-Limit/temporär).
        match self.http.get_member(gid, uid).await {
            Ok(_) => Ok(MemberPresence::Present),
            Err(err) if Self::is_http_404(&err) => Ok(MemberPresence::Absent),
            Err(_) => Ok(MemberPresence::Unknown),
        }
    }

    async fn resolve_names(&self, user_ids: &[u64]) -> Result<Vec<MemberInfo>, PortError> {
        let guild_ids = self.cache().guilds();
        let mut out = Vec::new();
        for &user_id in user_ids {
            let target = UserId::new(user_id);
            for gid in &guild_ids {
                let Some(guild) = self.cache().guild(*gid) else {
                    continue;
                };
                if let Some(member) = guild.members.get(&target) {
                    out.push(MemberInfo {
                        user_id,
                        display_name: member.display_name().to_string(),
                    });
                    break; // erste Fundstelle genügt
                }
            }
        }
        Ok(out)
    }

    async fn resolve_user(&self, user_id: u64) -> Result<Option<ResolvedUser>, PortError> {
        // Wie Pythons _resolve_user: REST-Lookup; nicht gefunden → Ok(None).
        match self.http.get_user(UserId::new(user_id)).await {
            Ok(user) => {
                let name = user.name.to_string();
                let global_name = user.global_name.as_ref().map(ToString::to_string);
                // Discord-Präzedenz: global_name vor Username.
                let display_name = Some(global_name.clone().unwrap_or_else(|| name.clone()));
                Ok(Some(ResolvedUser {
                    user_id,
                    name,
                    global_name,
                    display_name,
                }))
            }
            Err(_) => Ok(None),
        }
    }

    async fn list_members(&self) -> Result<Vec<GuildMemberInfo>, PortError> {
        // Wie _handle_list_members: alle nicht-Bot-Mitglieder der Default-Gilde
        // aus dem Gateway-Cache (gechunkt beim Bot-Start, wie resolve_names).
        if !self.cache_ready() {
            return Err(PortError::Discord(
                "gateway not connected (member list needs the cache)".to_string(),
            ));
        }
        let Some(gid) = self.cache().guilds().first().copied() else {
            return Err(PortError::GuildNotFound);
        };
        let Some(guild) = self.cache().guild(gid) else {
            return Err(PortError::GuildNotFound);
        };
        let members = guild
            .members
            .iter()
            .filter(|(_, m)| !m.user.bot)
            .map(|(uid, m)| GuildMemberInfo {
                user_id: uid.get(),
                name: m.user.name.to_string(),
                global_name: m.user.global_name.as_ref().map(ToString::to_string),
                nick: m.nick.as_ref().map(ToString::to_string),
            })
            .collect();
        Ok(members)
    }

    async fn guild_stats(&self, guild_id: Option<u64>) -> Result<GuildStats, PortError> {
        let gid = match guild_id {
            Some(id) => GuildId::new(id),
            None => self.first_guild().ok_or(PortError::GuildNotFound)?,
        };
        let Some(guild) = self.cache().guild(gid) else {
            return Err(PortError::GuildNotFound);
        };
        // Online: Präsenzen ungleich Offline (nur falsch befüllt ohne
        // GUILD_PRESENCES-Intent — dann 0, wie Pythons Fallback).
        let online_count = guild
            .presences
            .values()
            .filter(|p| p.status != serenity::all::OnlineStatus::Offline)
            .count() as u64;
        // Voice: Nutzer, die in irgendeinem Voice-Kanal sind.
        let voice_count = guild
            .voice_states
            .values()
            .filter(|vs| vs.channel_id.is_some())
            .count() as u64;
        Ok(GuildStats {
            found: true,
            guild_id: gid.get(),
            name: Some(guild.name.clone()),
            member_count: guild.member_count,
            online_count,
            voice_count,
            vanity_url_code: guild.vanity_url_code.clone().filter(|c| !c.is_empty()),
        })
    }
}

#[async_trait::async_trait]
impl crate::interactions::ChannelSender for DiscordAdapter {
    async fn send_to_channel(
        &self,
        channel_id: u64,
        content: Option<&str>,
        embeds: &[Value],
    ) -> Result<u64, String> {
        let mut body = Map::new();
        if let Some(content) = content {
            body.insert("content".into(), json!(content));
        }
        if !embeds.is_empty() {
            body.insert("embeds".into(), json!(embeds));
        }
        self.send_raw(channel_id, &body)
            .await
            .map_err(|err| err.to_string())
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
        components: Option<&Value>,
    ) -> Result<u64, ChangelogError> {
        let mut body = Map::new();
        if let Some(content) = content {
            body.insert("content".into(), json!(content));
        }
        body.insert("embeds".into(), json!(embeds));
        if let Some(components) = components {
            body.insert("components".into(), components.clone());
        }
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
        self.send_attachment_public(channel_id, Some(content), path)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn twitch_tracking_spec() -> ViewSpec {
        ViewSpec::TwitchLiveTracking {
            streamer_login: "DeadlockTV".to_string(),
            referral_url: "https://twitch.tv/deadlocktv".to_string(),
            tracking_token: "token-123".to_string(),
            button_label: "Auf Twitch ansehen".to_string(),
        }
    }

    #[test]
    fn view_components_rendern_scam_revoke_button_wie_python() {
        let spec = ViewSpec::ScamRevoke {
            verdict_id: 42,
            channel_login: "earlysalty".to_string(),
            chatter_login: "sophiaa_star".to_string(),
            action_taken: "banned".to_string(),
        };

        assert_eq!(
            DiscordAdapter::view_components(&spec),
            json!([{
                "type": 1,
                "components": [{
                    "type": 2,
                    "style": 4,
                    "label": "Rückgängig",
                    "custom_id": "scam-revoke:42",
                }],
            }])
        );
    }

    #[test]
    fn rich_body_components_v2_reicht_components_durch_und_haengt_tracking_button_an() {
        let input_components = json!([{
            "type": 17,
            "accent_color": 0xC8A86B,
            "components": [
                {"type": 10, "content": "LIVE"},
                {"type": 12, "items": [{
                    "media": {"url": "https://example.test/preview.jpg"},
                }]},
            ],
        }]);
        let message = RichMessage {
            channel_id: 123,
            content: Some("legacy content".to_string()),
            embed: json!({"title": "legacy embed"}),
            allowed_user_ids: vec![11],
            allowed_role_ids: vec![22],
            view_spec: Some(twitch_tracking_spec()),
            components: Some(input_components.clone()),
        };

        let body = DiscordAdapter::rich_body(&message);

        assert_eq!(body.get("flags"), Some(&json!(32768)));
        assert!(!body.contains_key("content"));
        assert!(!body.contains_key("embeds"));
        assert_eq!(
            body.get("allowed_mentions"),
            Some(&DiscordAdapter::allowed_mentions(&[11], &[22]))
        );
        let components = body
            .get("components")
            .and_then(Value::as_array)
            .expect("components array");
        let input_array = input_components.as_array().expect("input array");
        assert_eq!(components.len(), input_array.len() + 1);
        assert_eq!(&components[0], &input_array[0]);
        assert_eq!(
            components[1],
            json!({
                "type": 1,
                "components": [{
                    "type": 2,
                    "style": 1,
                    "label": "Auf Twitch ansehen",
                    "custom_id": "twitch-live:deadlocktv:token-123",
                }],
            })
        );
    }

    #[test]
    fn rich_body_ohne_components_bleibt_v1_payload() {
        let message = RichMessage {
            channel_id: 123,
            content: Some("legacy content".to_string()),
            embed: json!({"title": "legacy embed"}),
            allowed_user_ids: vec![11],
            allowed_role_ids: vec![22],
            view_spec: Some(twitch_tracking_spec()),
            components: None,
        };

        let body = DiscordAdapter::rich_body(&message);

        assert_eq!(
            Value::Object(body),
            json!({
                "content": "legacy content",
                "embeds": [{"title": "legacy embed"}],
                "allowed_mentions": {
                    "parse": [],
                    "users": ["11"],
                    "roles": ["22"],
                    "replied_user": false,
                },
                "components": [{
                    "type": 1,
                    "components": [{
                        "type": 2,
                        "style": 1,
                        "label": "Auf Twitch ansehen",
                        "custom_id": "twitch-live:deadlocktv:token-123",
                    }],
                }],
            })
        );
    }

    #[test]
    fn rich_body_haengt_nur_nicht_leere_embeds_an() {
        let mut message = RichMessage {
            channel_id: 123,
            content: Some("content only".to_string()),
            embed: json!({}),
            allowed_user_ids: vec![],
            allowed_role_ids: vec![],
            view_spec: None,
            components: None,
        };

        let body = DiscordAdapter::rich_body(&message);

        assert!(!body.contains_key("embeds"));

        message.embed = json!({"description": "x"});
        let body = DiscordAdapter::rich_body(&message);

        assert_eq!(body.get("embeds"), Some(&json!([{"description": "x"}])));
    }

    #[test]
    fn reaction_type_url_encodes_unicode_for_rest_path() {
        let encoded = DiscordAdapter::reaction_type_for_rest("✅").as_data();

        assert_eq!(encoded, "%E2%9C%85");
        assert!(!encoded.contains('✅'));
    }

    #[test]
    fn move_voice_channel_precheck_liefert_404_semantik() {
        assert_eq!(
            DiscordAdapter::move_voice_channel_precheck(None, 1),
            Err(PortError::ChannelNotFound)
        );
        assert_eq!(
            DiscordAdapter::move_voice_channel_precheck(Some(2), 1),
            Err(PortError::ChannelNotFound)
        );
        assert_eq!(
            DiscordAdapter::move_voice_channel_precheck(Some(1), 1),
            Ok(())
        );
    }

    #[test]
    fn reaction_emoji_maps_unicode_and_custom_values() {
        assert_eq!(
            DiscordAdapter::reaction_emoji(&ReactionType::Unicode("👍".to_string())),
            "👍"
        );
        assert_eq!(
            DiscordAdapter::reaction_emoji(&ReactionType::Custom {
                animated: false,
                id: serenity::all::EmojiId::new(123),
                name: Some("party".to_string()),
            }),
            "party:123"
        );
    }

    #[test]
    fn remove_role_erkennt_unknown_member_als_member_not_found() {
        assert!(DiscordAdapter::is_unknown_member_response(
            404,
            10007,
            "Unknown Member"
        ));
        assert!(DiscordAdapter::is_unknown_member_response(
            404,
            -1,
            "Unknown Member"
        ));
    }

    #[test]
    fn remove_role_mappt_andere_discord_404_nicht_als_member_not_found() {
        assert!(!DiscordAdapter::is_unknown_member_response(
            404,
            10011,
            "Unknown Role"
        ));
        assert!(!DiscordAdapter::is_unknown_member_response(
            404,
            10004,
            "Unknown Guild"
        ));
        assert!(!DiscordAdapter::is_unknown_member_response(
            500,
            0,
            "Internal Server Error"
        ));
    }
}
