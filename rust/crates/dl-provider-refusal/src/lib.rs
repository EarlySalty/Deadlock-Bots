use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

pub const GATEWAY_UNIT: &str = "dl-ai-gateway.service";
pub const CONTRACT: &str = "dl-provider-refusal/1";
pub const BILLING: &str = "billing";
pub const MAX_CODE_LEN: usize = 64;
pub const INFISICAL_TIMEOUT_SECS: u64 = 15;
pub const BROKER_CONNECT_TIMEOUT_SECS: u64 = 3;
pub const BROKER_TIMEOUT_SECS: u64 = 20;
pub const HELPER_NETWORK_BOUND_SECS: u64 = INFISICAL_TIMEOUT_SECS + BROKER_TIMEOUT_SECS;
pub const FAILED_RUN_PAUSE_SECS: u64 = 3_600;
pub const DAY_SECS: u64 = 86_400;
pub const WEEK_SECS: u64 = 7 * DAY_SECS;

pub fn safe_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CODE_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub fn request_error_status(status: u16) -> bool {
    matches!(status, 400 | 404 | 413 | 422)
}

pub fn billing_status(status: u16) -> bool {
    matches!(status, 402 | 412)
}

pub fn status_reason(status: u16) -> &'static str {
    match status {
        status if billing_status(status) => BILLING,
        status if request_error_status(status) => "http_4xx",
        429 => "http_429",
        529 => "http_529",
        401 => "http_401",
        403 => "http_403",
        500..=599 => "http_5xx",
        _ => "http_status",
    }
}

pub fn refusal_status(status: u16) -> bool {
    billing_status(status) || matches!(status, 401 | 403 | 429 | 500..=599)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalKind {
    Limit,
    Overloaded,
    Unavailable,
}

impl RefusalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Limit => "limit",
            Self::Overloaded => "overloaded",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Self::Limit, Self::Overloaded, Self::Unavailable]
            .into_iter()
            .find(|kind| kind.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Billing,
    Other,
}

impl Class {
    pub fn of(reason: &str) -> Self {
        if reason == BILLING {
            Self::Billing
        } else {
            Self::Other
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Billing => "billing",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    Suppressed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Suppressed => "suppressed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer {
    pub outcome: Outcome,
    pub next_at: u64,
}

impl Answer {
    pub fn line(&self) -> String {
        format!("outcome={} next_at={}", self.outcome.as_str(), self.next_at)
    }

    pub fn parse(stdout: &str) -> Option<Self> {
        let mut answers = stdout.lines().filter_map(Self::parse_line);
        let answer = answers.next()?;
        answers.next().is_none().then_some(answer)
    }

    fn parse_line(line: &str) -> Option<Self> {
        let (outcome, next_at) = line.trim().split_once(' ')?;
        let outcome = match outcome.strip_prefix("outcome=")? {
            "sent" => Outcome::Sent,
            "suppressed" => Outcome::Suppressed,
            _ => return None,
        };
        let next_at = next_at.strip_prefix("next_at=")?.parse().ok()?;
        Some(Self { outcome, next_at })
    }
}

pub fn protected_path(path: &Path, directory: bool) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err("Betriebspfad muss absolut sein.");
    }
    let uid = nix::unistd::geteuid().as_raw();
    let components: Vec<_> = path.components().collect();
    let mut current = PathBuf::new();
    let mut after_sticky = false;
    for (index, component) in components.iter().enumerate() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err("Betriebspfad ist nicht geschützt.");
        }
        current.push(component);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| "Betriebspfad ist nicht verfügbar.")?;
        let last = index + 1 == components.len();
        if ![0, uid].contains(&metadata.uid())
            || metadata.file_type().is_symlink()
            || (last && !directory && !metadata.is_file())
            || ((!last || directory) && !metadata.is_dir())
            || (after_sticky && (metadata.uid() != uid || metadata.mode() & 0o077 != 0))
        {
            return Err("Betriebspfad hat keine vertrauenswürdigen Eigentümer oder Rechte.");
        }
        after_sticky = metadata.mode() & 0o022 != 0;
        if after_sticky
            && !(metadata.is_dir() && metadata.uid() == 0 && metadata.mode() & 0o1000 != 0 && !last)
        {
            return Err("Betriebspfad darf nicht gruppen- oder weltbeschreibbar sein.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_table_agrees_on_billing_and_request_errors() {
        for status in [
            0, 200, 400, 401, 402, 403, 404, 412, 413, 422, 429, 500, 529, 599, 600,
        ] {
            assert!(safe_code(status_reason(status)), "{status}");
        }
        for status in [402, 412] {
            assert_eq!(status_reason(status), BILLING);
            assert!(refusal_status(status));
        }
        for status in [400, 404, 413, 422, 200] {
            assert!(!refusal_status(status), "{status}");
        }
        for status in [401, 403, 429, 500, 529] {
            assert!(refusal_status(status), "{status}");
        }
        assert!(safe_code(&"a".repeat(MAX_CODE_LEN)));
        assert!(!safe_code(&"a".repeat(MAX_CODE_LEN + 1)));
        assert!(!safe_code(""));
        assert!(!safe_code("Billing"));
    }

    #[test]
    fn answer_is_one_exact_line_among_other_output() {
        let answer = Answer {
            outcome: Outcome::Sent,
            next_at: 42,
        };
        assert_eq!(
            Answer::parse(&format!("Meldung bestätigt.\n{}\n", answer.line())),
            Some(answer)
        );
        assert_eq!(Answer::parse("outcome=sent"), None);
        assert_eq!(Answer::parse("outcome=sent next_at=-1"), None);
        assert_eq!(
            Answer::parse("outcome=sent next_at=1\noutcome=suppressed next_at=2"),
            None
        );
        assert_eq!(Answer::parse(""), None);
    }
}
