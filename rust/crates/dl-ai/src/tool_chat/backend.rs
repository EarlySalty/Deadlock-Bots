use async_trait::async_trait;

use super::types::{ToolChatRequest, ToolChatResponse};
use crate::upstream::RefusalKind;

crate::upstream::worded_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailureKind {
        Limit => RefusalKind::Limit.as_str(),
        Unavailable => RefusalKind::Unavailable.as_str(),
        Busy => "busy",
        Overloaded => RefusalKind::Overloaded.as_str(),
        Timeout => "timeout",
        InvalidRequest => "invalid_request",
        InvalidOutput => "invalid_output",
    }
}

impl FailureKind {
    pub fn retryable(self) -> bool {
        matches!(self, Self::Overloaded)
    }

    pub fn allows_fallback(self) -> bool {
        !matches!(self, Self::InvalidRequest)
    }

    pub fn ledger_code(self) -> &'static str {
        match self {
            Self::Limit => "rate_limited",
            Self::Unavailable => "transport",
            Self::Busy
            | Self::Overloaded
            | Self::Timeout
            | Self::InvalidRequest
            | Self::InvalidOutput => self.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendError {
    pub kind: FailureKind,
    pub reason: &'static str,
    pub status: Option<u16>,
}

impl BackendError {
    pub fn new(kind: FailureKind, reason: &'static str) -> Self {
        Self {
            kind,
            reason,
            status: None,
        }
    }

    pub fn upstream(kind: FailureKind, reason: &'static str, status: u16) -> Self {
        Self {
            kind,
            reason,
            status: Some(status),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.kind.as_str(), self.reason)?;
        match self.status {
            Some(status) => write!(formatter, " (upstream {status})"),
            None => Ok(()),
        }
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
