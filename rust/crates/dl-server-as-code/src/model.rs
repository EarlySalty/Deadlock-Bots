use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelType, PermissionOverwriteType};

use crate::{Result, ServerAsCodeError};

pub type DiscordId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Category,
    Channel,
    Role,
    PermissionOverwrite,
    BotMessage,
}

impl ObjectKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Channel => "channel",
            Self::Role => "role",
            Self::PermissionOverwrite => "permission_overwrite",
            Self::BotMessage => "bot_message",
        }
    }

    pub fn from_db(value: &str) -> Result<Self> {
        match value {
            "category" => Ok(Self::Category),
            "channel" => Ok(Self::Channel),
            "role" => Ok(Self::Role),
            "permission_overwrite" => Ok(Self::PermissionOverwrite),
            "bot_message" => Ok(Self::BotMessage),
            other => Err(ServerAsCodeError::UnknownObjectKind(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
    Category,
    News,
    Stage,
    Forum,
    Directory,
    Unknown(String),
}

impl ChannelKind {
    pub fn as_db(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Category => "category",
            Self::News => "news",
            Self::Stage => "stage",
            Self::Forum => "forum",
            Self::Directory => "directory",
            Self::Unknown(value) => value.as_str(),
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "text" => Self::Text,
            "voice" => Self::Voice,
            "category" => Self::Category,
            "news" => Self::News,
            "stage" => Self::Stage,
            "forum" => Self::Forum,
            "directory" => Self::Directory,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn from_serenity(kind: ChannelType) -> Self {
        match kind {
            ChannelType::Text => Self::Text,
            ChannelType::Voice => Self::Voice,
            ChannelType::Category => Self::Category,
            ChannelType::News => Self::News,
            ChannelType::Stage => Self::Stage,
            ChannelType::Forum => Self::Forum,
            ChannelType::Directory => Self::Directory,
            ChannelType::Unknown(value) => Self::Unknown(format!("unknown_{value}")),
            _ => Self::Unknown(kind.name().to_string()),
        }
    }

    pub fn discord_type_code(&self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Voice => 2,
            Self::Category => 4,
            Self::News => 5,
            Self::Stage => 13,
            Self::Directory => 14,
            Self::Forum => 15,
            Self::Unknown(_) => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Role,
    Member,
}

impl TargetKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::Member => "member",
        }
    }

    pub fn from_db(value: &str) -> Result<Self> {
        match value {
            "role" => Ok(Self::Role),
            "member" => Ok(Self::Member),
            other => Err(ServerAsCodeError::UnknownTargetKind(other.to_string())),
        }
    }

    pub fn from_serenity(kind: PermissionOverwriteType) -> Self {
        match kind {
            PermissionOverwriteType::Role(_) => Self::Role,
            PermissionOverwriteType::Member(_) => Self::Member,
            _ => Self::Member,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CategorySpec {
    pub guild_id: DiscordId,
    pub category_id: DiscordId,
    pub name: String,
    pub position: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChannelSpec {
    pub guild_id: DiscordId,
    pub channel_id: DiscordId,
    pub name: String,
    pub kind: ChannelKind,
    pub topic: Option<String>,
    pub position: i32,
    pub parent_category_id: Option<DiscordId>,
    pub nsfw: bool,
    pub bitrate: Option<i32>,
    pub user_limit: Option<i32>,
    pub rate_limit_per_user: Option<i32>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RoleSpec {
    pub guild_id: DiscordId,
    pub role_id: DiscordId,
    pub name: String,
    pub color: i32,
    pub hoist: bool,
    pub mentionable: bool,
    pub managed: bool,
    pub permissions_bitmask: u64,
    pub position: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OverwriteKey {
    pub channel_id: DiscordId,
    pub target_kind: TargetKind,
    pub target_id: DiscordId,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PermissionOverwriteSpec {
    pub guild_id: DiscordId,
    pub key: OverwriteKey,
    pub allow_bits: u64,
    pub deny_bits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BotMessageSpec {
    pub guild_id: DiscordId,
    pub channel_id: DiscordId,
    pub message_key: String,
    pub message_kind: String,
    pub message_id: Option<DiscordId>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub kind: ObjectKind,
    pub guild_id: DiscordId,
    pub object_id: DiscordId,
    pub channel_id: Option<DiscordId>,
    pub target_kind: Option<TargetKind>,
    pub target_id: Option<DiscordId>,
    pub message_key: Option<String>,
}

impl ObjectRef {
    pub fn category(spec: &CategorySpec) -> Self {
        Self {
            kind: ObjectKind::Category,
            guild_id: spec.guild_id,
            object_id: spec.category_id,
            channel_id: None,
            target_kind: None,
            target_id: None,
            message_key: None,
        }
    }

    pub fn channel(spec: &ChannelSpec) -> Self {
        Self {
            kind: ObjectKind::Channel,
            guild_id: spec.guild_id,
            object_id: spec.channel_id,
            channel_id: Some(spec.channel_id),
            target_kind: None,
            target_id: None,
            message_key: None,
        }
    }

    pub fn role(spec: &RoleSpec) -> Self {
        Self {
            kind: ObjectKind::Role,
            guild_id: spec.guild_id,
            object_id: spec.role_id,
            channel_id: None,
            target_kind: None,
            target_id: None,
            message_key: None,
        }
    }

    pub fn overwrite(spec: &PermissionOverwriteSpec) -> Self {
        Self {
            kind: ObjectKind::PermissionOverwrite,
            guild_id: spec.guild_id,
            object_id: spec.key.channel_id,
            channel_id: Some(spec.key.channel_id),
            target_kind: Some(spec.key.target_kind),
            target_id: Some(spec.key.target_id),
            message_key: None,
        }
    }

    pub fn bot_message(spec: &BotMessageSpec) -> Self {
        Self {
            kind: ObjectKind::BotMessage,
            guild_id: spec.guild_id,
            object_id: spec.channel_id,
            channel_id: Some(spec.channel_id),
            target_kind: None,
            target_id: None,
            message_key: Some(spec.message_key.clone()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildModel {
    pub guild_id: DiscordId,
    pub categories: BTreeMap<DiscordId, CategorySpec>,
    pub channels: BTreeMap<DiscordId, ChannelSpec>,
    pub roles: BTreeMap<DiscordId, RoleSpec>,
    pub overwrites: BTreeMap<OverwriteKey, PermissionOverwriteSpec>,
    pub bot_messages: BTreeMap<(DiscordId, String), BotMessageSpec>,
}

impl GuildModel {
    pub fn new(guild_id: DiscordId) -> Self {
        Self {
            guild_id,
            ..Self::default()
        }
    }

    pub fn channel_name(&self, channel_id: DiscordId) -> Option<&str> {
        self.channels
            .get(&channel_id)
            .map(|channel| channel.name.as_str())
            .or_else(|| {
                self.categories
                    .get(&channel_id)
                    .map(|category| category.name.as_str())
            })
    }

    pub fn channel_parent(&self, channel_id: DiscordId) -> Option<DiscordId> {
        self.channels
            .get(&channel_id)
            .and_then(|channel| channel.parent_category_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicNamespace {
    pub namespace_id: Option<i64>,
    pub namespace_key: String,
    pub system_name: String,
    pub match_rule: NamespaceMatch,
}

impl DynamicNamespace {
    pub fn matches_channel(&self, model: &GuildModel, channel_id: DiscordId) -> Result<bool> {
        match &self.match_rule {
            NamespaceMatch::ParentCategory(category_id) => {
                Ok(model.channel_parent(channel_id) == Some(*category_id))
            }
            NamespaceMatch::NamePrefix(prefix) => Ok(model
                .channel_name(channel_id)
                .is_some_and(|name| name.starts_with(prefix))),
            NamespaceMatch::NamePattern(pattern) => {
                let regex = Regex::new(pattern).map_err(|source| ServerAsCodeError::Regex {
                    pattern: pattern.clone(),
                    source,
                })?;
                Ok(model
                    .channel_name(channel_id)
                    .is_some_and(|name| regex.is_match(name)))
            }
            NamespaceMatch::ChannelId(matched_id) => Ok(channel_id == *matched_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum NamespaceMatch {
    ParentCategory(DiscordId),
    NamePrefix(String),
    NamePattern(String),
    ChannelId(DiscordId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentedException {
    pub exception_id: Option<i64>,
    pub exception_key: String,
    pub object_kind: ObjectKind,
    pub channel_id: Option<DiscordId>,
    pub target_kind: Option<TargetKind>,
    pub target_id: Option<DiscordId>,
    pub allow_bits: Option<u64>,
    pub deny_bits: Option<u64>,
    pub reason: String,
}

impl DocumentedException {
    pub fn matches_overwrite(&self, spec: &PermissionOverwriteSpec) -> bool {
        if self.object_kind != ObjectKind::PermissionOverwrite {
            return false;
        }
        if self.channel_id.is_some_and(|id| id != spec.key.channel_id) {
            return false;
        }
        if self
            .target_kind
            .is_some_and(|target_kind| target_kind != spec.key.target_kind)
        {
            return false;
        }
        if self.target_id.is_some_and(|id| id != spec.key.target_id) {
            return false;
        }
        if self.allow_bits.is_some_and(|bits| bits != spec.allow_bits) {
            return false;
        }
        if self.deny_bits.is_some_and(|bits| bits != spec.deny_bits) {
            return false;
        }
        true
    }
}
