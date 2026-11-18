use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

macro_rules! gateway_binary {
    () => {
        "dl-ai-gateway"
    };
}

#[macro_export]
macro_rules! worded_enum {
    ($(#[$meta:meta])* $visibility:vis enum $name:ident { $($variant:ident => $word:expr),+ $(,)? }) => {
        $(#[$meta])*
        $visibility enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $word),+
                }
            }
        }
    };
}

pub const GATEWAY_BINARY: &str = gateway_binary!();
pub const GATEWAY_UNIT: &str = concat!(gateway_binary!(), ".service");
pub const HELPER_BINARY: &str = "dl-unit-failure-notify";
pub const HEALTH_PATH: &str = "/healthz";
pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
pub const CONTRACT: &str = "dl-provider-refusal/1";
pub const BILLING: &str = "billing";
pub const USAGE_LIMIT: &str = "usage_limit";
pub const MAX_CODE_LEN: usize = 64;
pub const RATE_LIMITED: u16 = 429;
pub const OVERLOADED: u16 = 529;
pub const INFISICAL_TIMEOUT_SECS: u64 = 15;
pub const BROKER_CONNECT_TIMEOUT_SECS: u64 = 3;
pub const BROKER_TIMEOUT_SECS: u64 = 20;
pub const HELPER_NETWORK_BOUND_SECS: u64 = INFISICAL_TIMEOUT_SECS + BROKER_TIMEOUT_SECS;
pub const FAILED_RUN_PAUSE_SECS: u64 = 3_600;
pub const DAY_SECS: u64 = 86_400;
pub const WEEK_SECS: u64 = 7 * DAY_SECS;

const CODE_BYTES: [(u8, u8); 3] = [(b'a', b'z'), (b'0', b'9'), (b'_', b'_')];
const BILLING_TEXTS: [&str; 3] = ["credit balance", "billing", "spend limit"];
const USAGE_LIMIT_TEXT: &str = "usage limit";
const LIMIT_REACHED_TEXT: &str = "limit reached";
const RATE_LIMIT_TEXTS: [&str; 2] = ["rate limit", "rate_limit"];

pub fn safe_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CODE_LEN
        && value.bytes().all(|byte| {
            CODE_BYTES
                .iter()
                .any(|(first, last)| (*first..=*last).contains(&byte))
        })
}

pub fn code_pattern() -> String {
    let class: String = CODE_BYTES
        .iter()
        .map(|(first, last)| {
            if first == last {
                char::from(*first).to_string()
            } else {
                format!("{}-{}", char::from(*first), char::from(*last))
            }
        })
        .collect();
    format!("^[{class}]{{1,{MAX_CODE_LEN}}}$")
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
        RATE_LIMITED => "http_429",
        OVERLOADED => "http_529",
        401 => "http_401",
        403 => "http_403",
        500..=599 => "http_5xx",
        _ => "http_status",
    }
}

pub fn refusal_status(status: u16) -> bool {
    billing_status(status) || matches!(status, 401 | 403 | RATE_LIMITED | 500..=599)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Account {
    Metered,
    Plan,
}

fn names_rate_limit(lowered: &str) -> bool {
    RATE_LIMIT_TEXTS
        .iter()
        .any(|marker| lowered.contains(marker))
}

pub fn rate_limit_text(text: &str) -> bool {
    names_rate_limit(&text.to_ascii_lowercase())
}

pub fn account_refusal(status: Option<u16>, text: &str, account: Account) -> Option<&'static str> {
    let text = text.to_ascii_lowercase();
    let billing_named = BILLING_TEXTS.iter().any(|marker| text.contains(marker));
    let usage_limit = text.contains(USAGE_LIMIT_TEXT);
    let plan_quota = usage_limit || (text.contains(LIMIT_REACHED_TEXT) && !names_rate_limit(&text));
    if account == Account::Plan && plan_quota && !billing_named {
        return Some(USAGE_LIMIT);
    }
    let named = billing_named || usage_limit;
    status
        .is_some_and(|status| billing_status(status) || (named && matches!(status, 400 | 403)))
        .then_some(BILLING)
}

pub fn refusal_reason(status: u16, text: &str, account: Account) -> &'static str {
    account_refusal(Some(status), text, account).unwrap_or_else(|| status_reason(status))
}

pub fn is_account_refusal(reason: &str) -> bool {
    reason == BILLING || reason == USAGE_LIMIT
}

pub fn limit_refusal(status: Option<u16>, reason: &str) -> bool {
    status == Some(RATE_LIMITED) || is_account_refusal(reason)
}

fn worded<T: Copy>(all: &[T], word: fn(T) -> &'static str, value: &str) -> Option<T> {
    all.iter().copied().find(|item| word(*item) == value)
}

worded_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum RefusalKind {
        Limit => "limit",
        Overloaded => "overloaded",
        Unavailable => "unavailable",
    }
}

impl RefusalKind {
    pub fn parse(value: &str) -> Option<Self> {
        worded(Self::ALL, Self::as_str, value)
    }
}

worded_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Class {
        Billing => BILLING,
        Other => "other",
    }
}

impl Class {
    pub fn of(reason: &str) -> Self {
        if reason == BILLING {
            Self::Billing
        } else {
            Self::Other
        }
    }
}

worded_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Outcome {
        Sent => "sent",
        Suppressed => "suppressed",
    }
}

impl Outcome {
    pub fn parse(value: &str) -> Option<Self> {
        worded(Self::ALL, Self::as_str, value)
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
        let outcome = Outcome::parse(outcome.strip_prefix("outcome=")?)?;
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
    fn code_shape_is_one_alphabet_and_one_length() {
        let allowed = "abcdefghijklmnopqrstuvwxyz0123456789_";
        for byte in 0..=127u8 {
            let symbol = char::from(byte);
            assert_eq!(
                safe_code(&symbol.to_string()),
                allowed.contains(symbol),
                "{byte}"
            );
        }
        assert!(!safe_code("ä"));
        assert!(!safe_code(""));
        assert!(safe_code(&"a".repeat(MAX_CODE_LEN)));
        assert!(!safe_code(&"a".repeat(MAX_CODE_LEN + 1)));
        assert_eq!(code_pattern(), "^[a-z0-9_]{1,64}$");
    }

    #[test]
    fn contract_words_round_trip_for_every_variant() {
        for kind in RefusalKind::ALL.iter().copied() {
            assert_eq!(RefusalKind::parse(kind.as_str()), Some(kind));
            assert!(safe_code(kind.as_str()));
        }
        assert_eq!(RefusalKind::parse(""), None);
        assert_eq!(RefusalKind::parse("Limit"), None);
        for class in Class::ALL.iter().copied() {
            assert_eq!(Class::of(class.as_str()), class);
            assert!(safe_code(class.as_str()));
        }
        assert_eq!(Class::Billing.as_str(), BILLING);
        assert_eq!(Class::of(""), Class::Other);
        assert_eq!(Class::of(USAGE_LIMIT), Class::Other);
        for outcome in Outcome::ALL.iter().copied() {
            assert_eq!(Outcome::parse(outcome.as_str()), Some(outcome));
            for next_at in [0, 42, u64::MAX] {
                let answer = Answer { outcome, next_at };
                assert_eq!(Answer::parse(&answer.line()), Some(answer));
            }
        }
        assert_eq!(Outcome::parse(""), None);
        assert_eq!(Outcome::parse("Sent"), None);
        assert_eq!(Answer::parse("outcome=unknown next_at=1"), None);
    }

    #[test]
    fn the_shared_rule_decides_billing_and_plan_quota_from_status_text_and_account() {
        let credit = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the API."}}"#;
        let typed = r#"{"error":{"type":"billing_error","message":"x"}}"#;
        let limit = r#"{"error":{"message":"You have reached your specified API usage limits."}}"#;
        let plain = "Claude AI usage limit reached|1";
        for account in [Account::Metered, Account::Plan] {
            for status in [402, 412] {
                assert_eq!(refusal_reason(status, "", account), BILLING);
            }
            for status in [400, 403] {
                assert_eq!(refusal_reason(status, credit, account), BILLING);
                assert_eq!(refusal_reason(status, typed, account), BILLING);
                assert_eq!(
                    refusal_reason(status, "Spend LIMIT reached", account),
                    BILLING
                );
                for text in ["", "null", "[1,2]", "<html>", r#"{"error":"flat"}"#] {
                    assert_eq!(refusal_reason(status, text, account), status_reason(status));
                }
            }
            for status in [0, 200, 401, 404, 413, 422, 429, 500, 529, 600] {
                assert_eq!(
                    refusal_reason(status, credit, account),
                    status_reason(status)
                );
            }
            assert_eq!(account_refusal(None, credit, account), None);
            assert_eq!(account_refusal(None, "", account), None);
        }
        for text in [limit, plain] {
            for status in [400, 403] {
                assert_eq!(refusal_reason(status, text, Account::Metered), BILLING);
            }
            for status in [401, 429, 500] {
                assert_eq!(
                    refusal_reason(status, text, Account::Metered),
                    status_reason(status)
                );
            }
            assert_eq!(account_refusal(None, text, Account::Metered), None);
            for status in [None, Some(400), Some(402), Some(403), Some(429), Some(500)] {
                assert_eq!(
                    account_refusal(status, text, Account::Plan),
                    Some(USAGE_LIMIT)
                );
            }
        }
        assert_eq!(
            [400, 404, 413, 422].map(status_reason),
            ["http_4xx"; 4],
            "request errors share one word"
        );
        assert_eq!(status_reason(429), "http_429");
        assert_eq!(status_reason(529), "http_529");
        let thinking = r#"{"error":{"type":"invalid_request_error","message":"thinking.budget_tokens: Input should be greater than or equal to 1024"}}"#;
        for account in [Account::Metered, Account::Plan] {
            for status in [400, 403] {
                assert_eq!(
                    refusal_reason(status, thinking, account),
                    status_reason(status)
                );
            }
            assert_eq!(
                account_refusal(Some(429), "Rate limit reached", account),
                None
            );
            assert_eq!(
                account_refusal(None, "rate_limit_error: limit reached", account),
                None
            );
            assert_eq!(
                account_refusal(Some(402), "Spend limit reached", account),
                Some(BILLING)
            );
            assert_eq!(
                account_refusal(Some(500), "Spend limit reached", account),
                None
            );
            assert_eq!(account_refusal(None, "Spend limit reached", account), None);
        }
        for status in [None, Some(400), Some(429), Some(500)] {
            assert_eq!(
                account_refusal(status, "5-hour limit reached", Account::Plan),
                Some(USAGE_LIMIT)
            );
        }
        assert_eq!(
            account_refusal(Some(400), "5-hour limit reached", Account::Metered),
            None
        );
        assert!(rate_limit_text("API Error: Rate_Limit_Error"));
        assert!(!rate_limit_text("limit reached"));
        assert!(limit_refusal(
            Some(RATE_LIMITED),
            status_reason(RATE_LIMITED)
        ));
        assert!(limit_refusal(None, BILLING));
        assert!(limit_refusal(Some(500), USAGE_LIMIT));
        assert!(!limit_refusal(Some(OVERLOADED), status_reason(OVERLOADED)));
        assert!(!limit_refusal(None, ""));
        assert!(is_account_refusal(BILLING));
        assert!(is_account_refusal(USAGE_LIMIT));
        for status in [0, 200, 400, 401, 403, 404, 429, 500, 529, 600] {
            assert!(!is_account_refusal(status_reason(status)), "{status}");
        }
        assert!(!is_account_refusal(""));
        assert_eq!(
            Class::of(refusal_reason(400, limit, Account::Plan)),
            Class::Other
        );
        assert_eq!(
            Class::of(refusal_reason(400, limit, Account::Metered)),
            Class::Billing
        );
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
