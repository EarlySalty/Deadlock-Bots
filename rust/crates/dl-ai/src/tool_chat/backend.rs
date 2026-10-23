use async_trait::async_trait;

use super::types::{ToolChatRequest, ToolChatResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Limit,
    Unavailable,
    Busy,
    Timeout,
    InvalidRequest,
    InvalidOutput,
}

impl FailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Limit => "limit",
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::InvalidRequest => "invalid_request",
            Self::InvalidOutput => "invalid_output",
        }
    }

    pub fn allows_fallback(self) -> bool {
        !matches!(self, Self::InvalidRequest)
    }

    pub fn ledger_code(self) -> &'static str {
        match self {
            Self::Limit => "rate_limited",
            Self::Unavailable => "transport",
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::InvalidRequest => "invalid_request",
            Self::InvalidOutput => "invalid_output",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendError {
    pub kind: FailureKind,
    pub reason: &'static str,
}

impl BackendError {
    pub fn new(kind: FailureKind, reason: &'static str) -> Self {
        Self { kind, reason }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.kind.as_str(), self.reason)
    }
}

impl std::error::Error for BackendError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallTarget {
    pub model: String,
    pub max_tokens: u32,
}

#[async_trait]
pub trait ToolBackend: Send + Sync {
    async fn complete(
        &self,
        request: &ToolChatRequest,
        target: &CallTarget,
    ) -> Result<ToolChatResponse, BackendError>;

    fn records_own_usage(&self) -> bool {
        false
    }
}
