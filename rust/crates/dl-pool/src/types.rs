use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub guild_id: i64,
    pub discord_id: i64,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Mode {
    Casual,
    Ranked,
    StreetBrawl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum PlayStyle {
    Any,
    Relaxed,
    Competitive,
    Learning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum VoicePreference {
    Any,
    WithVoice,
    WithoutVoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum SessionStatus {
    Pending,
    Open,
    Active,
    Ended,
    Expired,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Availability {
    pub weekday: i16,
    pub start_minute: i16,
    pub end_minute: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub modes: Vec<Mode>,
    pub group_size_min: i16,
    pub group_size_max: i16,
    pub play_style: PlayStyle,
    pub voice_preference: VoicePreference,
    pub languages: Vec<String>,
    pub timezone: String,
    pub availability: Vec<Availability>,
    pub preferred_discord_ids: Vec<i64>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            modes: Vec::new(),
            group_size_min: 2,
            group_size_max: 6,
            play_style: PlayStyle::Any,
            voice_preference: VoicePreference::Any,
            languages: vec!["de".into()],
            timezone: "Europe/Berlin".into(),
            availability: Vec::new(),
            preferred_discord_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Profile {
    pub guild_id: i64,
    pub discord_id: i64,
    pub modes: Vec<Mode>,
    pub group_size_min: i16,
    pub group_size_max: i16,
    pub play_style: PlayStyle,
    pub voice_preference: VoicePreference,
    pub languages: Vec<String>,
    pub timezone: String,
    pub dm_opt_in: bool,
    pub published: bool,
    pub interview_completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PublicProfile {
    pub guild_id: i64,
    pub discord_id: i64,
    pub modes: Vec<Mode>,
    pub group_size_min: i16,
    pub group_size_max: i16,
    pub play_style: PlayStyle,
    pub voice_preference: VoicePreference,
    pub languages: Vec<String>,
    pub timezone: String,
    pub rank_tier: Option<i32>,
    pub rank_subtier: Option<i32>,
    pub games_played: Option<i64>,
    pub total_play_seconds: Option<i64>,
    pub observed_games: Option<i64>,
    pub observed_play_seconds: Option<i64>,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    pub fetched_at: Option<DateTime<Utc>>,
    pub heatmap: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApiSnapshot {
    pub steam_id: String,
    pub rank_tier: Option<i32>,
    pub rank_subtier: Option<i32>,
    pub games_played: Option<i64>,
    pub total_play_seconds: Option<i64>,
    pub observed_games: i64,
    pub observed_play_seconds: i64,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    pub heatmap: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Match {
    pub match_id: i64,
    pub started_at: DateTime<Utc>,
    pub duration_seconds: Option<i32>,
    pub mode: Option<Mode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CoPlayer {
    pub discord_id: i64,
    pub target_discord_id: i64,
    pub games_together: i64,
    pub last_played_at: DateTime<Utc>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SteamLink {
    pub discord_id: i64,
    pub steam_id: String,
    pub primary_account: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PoolFilter {
    pub mode: Option<Mode>,
    pub min_rank: Option<i32>,
    pub max_rank: Option<i32>,
    pub weekday: Option<i16>,
    pub minute: Option<i16>,
    pub timezone: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub session_id: i64,
    pub guild_id: i64,
    pub initiator_discord_id: i64,
    pub mode: Mode,
    pub channel_id: Option<i64>,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Participant {
    pub discord_id: i64,
    pub joined_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnSession {
    pub session: Session,
    pub joined_at: DateTime<Utc>,
    pub feedback: Option<Feedback>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, sqlx::FromRow)]
pub struct Feedback {
    pub play_again: bool,
    pub friendly: bool,
    pub good_communication: bool,
    pub balanced_match: bool,
}
