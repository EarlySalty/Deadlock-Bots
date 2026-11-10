use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub const ACCESS_PATH: &str = "/gate/v1/access";
pub const AUTHORIZE_PATH: &str = "/gate/v1/authorize";
pub const OUTCOME_PATH: &str = "/gate/v1/outcome";
pub const MAX_OUTCOME_DETAIL_CHARS: usize = 1000;
pub const MAX_REDACTED_ARGS_BYTES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccessRequest {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Member,
    NotMember,
    LookupFailed,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessResponse {
    pub status: MemberStatus,
    pub member: bool,
    pub owner: bool,
    pub timed_out: bool,
    pub permissions: Vec<String>,
    #[serde(default)]
    pub channel_permissions: Option<Vec<String>>,
    pub role_ids: Vec<String>,
    pub top_role_position: i64,
}

impl AccessResponse {
    pub fn no_rights(status: MemberStatus) -> Self {
        Self {
            status,
            member: false,
            owner: false,
            timed_out: false,
            permissions: Vec::new(),
            channel_permissions: None,
            role_ids: Vec::new(),
            top_role_position: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    DiscordMessage,
    DiscordInteraction,
    None,
}

impl Origin {
    pub fn is_discord(self) -> bool {
        matches!(self, Self::DiscordMessage | Self::DiscordInteraction)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DiscordMessage => "discord_message",
            Self::DiscordInteraction => "discord_interaction",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    Read,
    Write,
    Destructive,
}

impl ActionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuthorizeRequest {
    pub caller: String,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub dm: bool,
    pub capability: String,
    pub class: ActionClass,
    #[serde(default)]
    pub internal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_role_id: Option<String>,
    pub args_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_redacted: Option<Value>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Allowed,
    NoIdentity,
    NotMember,
    LookupFailed,
    AccessFileUnavailable,
    UnknownCaller,
    ConnectorDisabled,
    CapabilityDisabled,
    OwnerOnly,
    Unmapped,
    MissingRight,
    TimedOut,
    Hierarchy,
    TargetLookupFailed,
    RecordUnavailable,
    #[serde(other)]
    Unknown,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::NoIdentity => "no_identity",
            Self::NotMember => "not_member",
            Self::LookupFailed => "lookup_failed",
            Self::AccessFileUnavailable => "access_file_unavailable",
            Self::UnknownCaller => "unknown_caller",
            Self::ConnectorDisabled => "connector_disabled",
            Self::CapabilityDisabled => "capability_disabled",
            Self::OwnerOnly => "owner_only",
            Self::Unmapped => "unmapped",
            Self::MissingRight => "missing_right",
            Self::TimedOut => "timed_out",
            Self::Hierarchy => "hierarchy",
            Self::TargetLookupFailed => "target_lookup_failed",
            Self::RecordUnavailable => "record_unavailable",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizeResponse {
    pub decision_id: String,
    pub allowed: bool,
    pub reason: Reason,
    pub confirm_required: bool,
    pub shadow: bool,
    pub place_ok: bool,
}

impl AuthorizeResponse {
    pub fn permits(&self) -> bool {
        self.allowed && self.reason == Reason::Allowed
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Ok,
    Error,
}

impl OutcomeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutcomeRequest {
    pub decision_id: String,
    pub caller: String,
    pub status: OutcomeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutcomeResponse {
    pub queued: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error("access gate token is empty")]
    EmptyToken,
    #[error("access gate request failed")]
    Transport(#[source] reqwest::Error),
    #[error("access gate answered HTTP {0}")]
    Status(u16),
    #[error("access gate answer is malformed")]
    Malformed(#[source] reqwest::Error),
}

#[derive(Clone)]
pub struct GateClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl GateClient {
    pub fn new(base_url: &str, token: &str, timeout: Duration) -> Result<Self, GateError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(GateError::EmptyToken);
        }
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(GateError::Transport)?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        })
    }

    async fn post<B: Serialize, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, GateError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(GateError::Transport)?;
        if !response.status().is_success() {
            return Err(GateError::Status(response.status().as_u16()));
        }
        response.json().await.map_err(GateError::Malformed)
    }

    pub async fn access(&self, request: &AccessRequest) -> Result<AccessResponse, GateError> {
        self.post(ACCESS_PATH, request).await
    }

    pub async fn authorize(
        &self,
        request: &AuthorizeRequest,
    ) -> Result<AuthorizeResponse, GateError> {
        self.post(AUTHORIZE_PATH, request).await
    }

    pub async fn permits(&self, request: &AuthorizeRequest) -> bool {
        self.authorize(request)
            .await
            .is_ok_and(|answer| answer.permits())
    }

    pub async fn outcome(&self, request: &OutcomeRequest) -> Result<OutcomeResponse, GateError> {
        self.post(OUTCOME_PATH, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_reason_or_missing_allow_is_a_refusal() {
        let answer: AuthorizeResponse = serde_json::from_str(
            r#"{"decision_id":"x","allowed":true,"reason":"later_reason","confirm_required":true,"shadow":true,"place_ok":false,"extra":1}"#,
        )
        .expect("forward compatible answer");
        assert_eq!(answer.reason, Reason::Unknown);
        assert!(!answer.permits());
        let answer = AuthorizeResponse {
            reason: Reason::Allowed,
            ..answer
        };
        assert!(answer.permits());
        assert!(!AuthorizeResponse {
            allowed: false,
            ..answer
        }
        .permits());
    }

    #[tokio::test]
    async fn unreachable_gate_is_a_refusal() {
        let client = GateClient::new("http://127.0.0.1:9", "token", Duration::from_millis(300))
            .expect("client");
        let request = AuthorizeRequest {
            caller: "brain".into(),
            origin: Origin::DiscordMessage,
            user_id: Some("1".into()),
            channel_id: None,
            dm: false,
            capability: "discord.event.write".into(),
            class: ActionClass::Write,
            internal: false,
            target_user_id: None,
            target_role_id: None,
            args_hash: "0".repeat(64),
            args_redacted: None,
        };
        assert!(!client.permits(&request).await);
        assert!(GateClient::new("http://x", " ", Duration::from_secs(1)).is_err());
    }
}
