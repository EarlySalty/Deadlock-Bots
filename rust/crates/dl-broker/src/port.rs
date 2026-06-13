//! Der Discord-Port: entkoppelt den Broker von der Gateway-/REST-Schicht.
//!
//! Die HTTP-/Idempotency-Logik kennt nur dieses Trait; die echte
//! Implementierung (serenity) liefert dl-discord, Tests nutzen einen Mock.

use serde_json::Value;

/// Fehlerkategorien — der Handler übersetzt sie in die exakten
/// Python-Fehlertexte (404 not_found / 502 discord_error / 400 bad_request).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    #[error("channel not found")]
    ChannelNotFound,
    #[error("category not found")]
    CategoryNotFound,
    #[error("user not found")]
    UserNotFound,
    #[error("guild not found")]
    GuildNotFound,
    #[error("role not found")]
    RoleNotFound,
    #[error("member not found")]
    MemberNotFound,
    #[error("message not found")]
    MessageNotFound,
    #[error("failed to open DM channel")]
    DmOpenFailed,
    #[error("channel does not expose voice members")]
    NoVoiceMembers,
    #[error("guild unavailable for channel creation")]
    GuildUnavailable,
    #[error("discord error: {0}")]
    Discord(String),
}

/// Geparster view_spec (Vertrag wie Python _parse_view_spec).
#[derive(Debug, Clone, PartialEq)]
pub enum ViewSpec {
    LinkButton {
        label: String,
        url: String,
    },
    TwitchLiveTracking {
        streamer_login: String,
        referral_url: String,
        tracking_token: String,
        button_label: String,
    },
}

#[derive(Debug, Clone)]
pub struct RichMessage {
    pub channel_id: u64,
    pub content: Option<String>,
    /// Roh-Embed-Dict (Discord-Embed-Format) — Tiefenvalidierung macht Discord.
    pub embed: Value,
    pub allowed_user_ids: Vec<u64>,
    pub allowed_role_ids: Vec<u64>,
    pub view_spec: Option<ViewSpec>,
}

#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub user_id: u64,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct InviteInfo {
    pub invite_url: String,
    pub code: String,
    pub guild_id: u64,
}

#[derive(Debug, Clone)]
pub struct RoleInfo {
    pub id: u64,
    pub name: String,
    pub position: i64,
    pub member_count: usize,
}

#[derive(Debug, Clone)]
pub struct GuildRoles {
    pub guild_id: u64,
    pub chunked: bool,
    pub roles: Vec<RoleInfo>,
}

#[derive(Debug, Clone)]
pub struct RoleMembers {
    pub role_id: u64,
    pub name: String,
    pub members: Vec<MemberInfo>,
}

/// Zugriffsstatus eines Mitglieds für die Dashboard-Auth: Admin-Permission
/// und Rollen-IDs, aus dem Member-Cache abgeleitet (ersetzt Pythons
/// `guild.get_member(...).guild_permissions` / `.roles`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberAccess {
    /// Ob das Mitglied in (mind.) einer geprüften Gilde gefunden wurde.
    pub found: bool,
    pub user_id: u64,
    pub display_name: Option<String>,
    /// `administrator`-Permission in irgendeiner geprüften Gilde.
    pub is_administrator: bool,
    /// Rollen-IDs aus der ersten Gilde mit Treffer (ohne `@everyone`).
    pub role_ids: Vec<u64>,
}

#[async_trait::async_trait]
pub trait DiscordPort: Send + Sync {
    async fn is_ready(&self) -> bool;
    /// → message_id
    async fn send_channel_message(&self, channel_id: u64, content: &str) -> Result<u64, PortError>;
    /// → (dm_channel_id, message_id)
    async fn send_dm(&self, user_id: u64, content: &str) -> Result<(Option<u64>, u64), PortError>;
    /// → channel_id
    async fn create_text_channel(
        &self,
        category_id: u64,
        name: &str,
        topic: Option<&str>,
    ) -> Result<u64, PortError>;
    async fn delete_channel(&self, channel_id: u64) -> Result<(), PortError>;
    /// → message_id
    async fn send_rich_message(&self, message: &RichMessage) -> Result<u64, PortError>;
    async fn edit_rich_message(
        &self,
        message_id: u64,
        message: &RichMessage,
    ) -> Result<(), PortError>;
    async fn add_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), PortError>;
    async fn remove_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        reason: &str,
    ) -> Result<(), PortError>;
    /// channel_id None = aus Voice kicken.
    async fn move_voice(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_id: Option<u64>,
    ) -> Result<(), PortError>;
    async fn voice_members(&self, channel_id: u64) -> Result<Vec<MemberInfo>, PortError>;
    async fn create_invite(&self, channel_id: u64, reason: &str) -> Result<InviteInfo, PortError>;
    /// Diagnose: guild_id None = erste Guild des Bots.
    async fn list_roles(&self, guild_id: Option<u64>) -> Result<GuildRoles, PortError>;
    async fn role_members(
        &self,
        guild_id: Option<u64>,
        role_id: u64,
    ) -> Result<RoleMembers, PortError>;
    /// Zugriffsstatus eines Mitglieds (Admin + Rollen) für die Dashboard-Auth.
    /// guild_id None = über alle Bot-Gilden aggregieren.
    async fn member_access(
        &self,
        guild_id: Option<u64>,
        user_id: u64,
    ) -> Result<MemberAccess, PortError>;
    /// Anzeigenamen zu mehreren User-IDs aus dem Cache (für Dashboard-
    /// Analytics). Nur gefundene Mitglieder werden zurückgegeben; der Aufrufer
    /// füllt fehlende selbst auf (`User <id>`).
    async fn resolve_names(&self, user_ids: &[u64]) -> Result<Vec<MemberInfo>, PortError>;
}
