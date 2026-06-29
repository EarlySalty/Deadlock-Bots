use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub id: i64,
    pub discord_id: Option<i64>,
    pub display_name: String,
    pub rank: Option<String>,
    pub rank_source: String,
    pub rank_verified: bool,
    pub roles: Option<String>,
    pub availability: Option<String>,
    pub status: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SeedPlayer {
    pub name: String,
    #[serde(default)]
    pub rank: Option<String>,
    #[serde(default)]
    pub roles: Option<String>,
    #[serde(default)]
    pub availability: BTreeMap<String, String>,
    #[serde(default)]
    pub coach: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SeedTeamMember {
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub is_captain: bool,
    #[serde(default)]
    pub is_bench: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SeedTeam {
    pub name: String,
    #[serde(default)]
    pub coach: Option<String>,
    #[serde(default)]
    pub discord_role_id: Option<i64>,
    #[serde(default)]
    pub discord_channel_id: Option<i64>,
    #[serde(default)]
    pub members: Vec<SeedTeamMember>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SeedMatch {
    pub team_a: String,
    pub team_b: String,
    #[serde(default)]
    pub when_text: Option<String>,
    #[serde(default)]
    pub scheduled_at: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SeedRoster {
    #[serde(default)]
    pub players: Vec<SeedPlayer>,
    #[serde(default)]
    pub pool_unassigned: Vec<SeedPlayer>,
    #[serde(default)]
    pub teams: Vec<SeedTeam>,
    #[serde(default)]
    pub matches: Vec<SeedMatch>,
}
