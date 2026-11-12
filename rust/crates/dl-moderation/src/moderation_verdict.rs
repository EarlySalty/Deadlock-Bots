use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PARSE_ERROR_REASON: &str = "parse_error";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModerationCategory {
    Scam,
    Csam,
    NsfwExplicit,
    Harassment,
    HateSpeech,
    RagebaitOk,
    GameRelatedOk,
    Other,
}

impl ModerationCategory {
    pub fn from_label(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "scam" => Self::Scam,
            "csam" => Self::Csam,
            "nsfw_explicit" => Self::NsfwExplicit,
            "harassment" | "insult" | "abuse" => Self::Harassment,
            "hate_speech" | "hate" => Self::HateSpeech,
            "ragebait_ok" | "ragebait" => Self::RagebaitOk,
            "game_related_ok" | "ok" | "safe" => Self::GameRelatedOk,
            _ => Self::Other,
        }
    }

    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Scam => "scam",
            Self::Csam => "csam",
            Self::NsfwExplicit => "nsfw_explicit",
            Self::Harassment => "harassment",
            Self::HateSpeech => "hate_speech",
            Self::RagebaitOk => "ragebait_ok",
            Self::GameRelatedOk => "game_related_ok",
            Self::Other => "other",
        }
    }

    pub fn is_high_damage(&self) -> bool {
        matches!(self, Self::Scam | Self::Csam | Self::NsfwExplicit)
    }

    pub fn is_harmless(&self) -> bool {
        matches!(self, Self::RagebaitOk | Self::GameRelatedOk)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContentAnalysis {
    pub category: ModerationCategory,
    pub confidence: f64,
    pub reason: String,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationDecision {
    pub confirmed: bool,
    pub category: ModerationCategory,
    pub confidence: f64,
    pub reason: String,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModerationVerdict {
    pub analysis: ContentAnalysis,
    pub verification: VerificationDecision,
    pub trigger: String,
}

fn raw_envelope(raw_text: Option<&str>, parsed: Option<Value>) -> String {
    let mut envelope = serde_json::Map::new();
    envelope.insert("response_text".to_string(), serde_json::json!(raw_text));
    if let Some(parsed) = parsed {
        envelope.insert("parsed".to_string(), parsed);
    }
    Value::Object(envelope).to_string()
}

fn extract_json_object(raw_text: Option<&str>) -> Option<Value> {
    let raw = raw_text?;
    let cleaned = dl_ai::strip_think(raw);
    let open = cleaned.find('{')?;
    let close = cleaned.rfind('}')?;
    if close <= open {
        return None;
    }
    serde_json::from_str::<Value>(&cleaned[open..=close]).ok()
}

fn normalized_reason(payload: &Value) -> String {
    let reason: String = payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect();
    if reason.is_empty() {
        "Keine Begruendung geliefert.".to_string()
    } else {
        reason
    }
}

fn confidence(payload: &Value) -> f64 {
    let value = payload
        .get("confidence")
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
        })
        .unwrap_or(0.0);
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub fn parse_content_analysis(raw_text: Option<&str>) -> ContentAnalysis {
    let Some(payload) = extract_json_object(raw_text) else {
        return ContentAnalysis {
            category: ModerationCategory::Other,
            confidence: 0.0,
            reason: PARSE_ERROR_REASON.to_string(),
            raw_json: raw_envelope(raw_text, None),
        };
    };
    let category = payload
        .get("category")
        .and_then(Value::as_str)
        .map(ModerationCategory::from_label)
        .unwrap_or(ModerationCategory::Other);
    ContentAnalysis {
        category,
        confidence: confidence(&payload),
        reason: normalized_reason(&payload),
        raw_json: raw_envelope(raw_text, Some(payload)),
    }
}

const SCREENSHOT_NAME_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScreenshotAccount {
    pub user_id: Option<u64>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

pub fn parse_screenshot_account(raw_text: Option<&str>) -> ScreenshotAccount {
    let Some(account) =
        extract_json_object(raw_text).and_then(|payload| payload.get("reported_account").cloned())
    else {
        return ScreenshotAccount::default();
    };
    ScreenshotAccount {
        user_id: account.get("user_id").and_then(screenshot_user_id),
        username: account.get("username").and_then(screenshot_name),
        display_name: account.get("display_name").and_then(screenshot_name),
    }
}

fn screenshot_user_id(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn screenshot_name(value: &Value) -> Option<String> {
    let name: String = value
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(SCREENSHOT_NAME_MAX_CHARS)
        .collect();
    (!name.is_empty()).then_some(name)
}

pub fn parse_verification_decision(
    raw_text: Option<&str>,
    _suspected_category: ModerationCategory,
) -> VerificationDecision {
    let Some(payload) = extract_json_object(raw_text) else {
        return VerificationDecision {
            confirmed: false,
            category: ModerationCategory::Other,
            confidence: 0.0,
            reason: PARSE_ERROR_REASON.to_string(),
            raw_json: raw_envelope(raw_text, None),
        };
    };
    let category = payload
        .get("category")
        .and_then(Value::as_str)
        .map(ModerationCategory::from_label)
        .unwrap_or(ModerationCategory::Other);
    VerificationDecision {
        confirmed: payload
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        category,
        confidence: confidence(&payload),
        reason: normalized_reason(&payload),
        raw_json: raw_envelope(raw_text, Some(payload)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_account_reads_only_the_reported_account_object() {
        let parsed = parse_screenshot_account(Some(
            r#"{"confirmed":true,"category":"scam","confidence":0.9,"reason":"x","reported_account":{"user_id":"123456789012345678","username":"  fake_support ","display_name":null}}"#,
        ));
        assert_eq!(
            parsed,
            ScreenshotAccount {
                user_id: Some(123456789012345678),
                username: Some("fake_support".to_string()),
                display_name: None,
            }
        );
        assert_eq!(
            parse_screenshot_account(Some(r#"{"confirmed":true,"username":"x"}"#)),
            ScreenshotAccount::default()
        );
        assert_eq!(parse_screenshot_account(None), ScreenshotAccount::default());
        let only_display = parse_screenshot_account(Some(
            r#"{"reported_account":{"user_id":"nicht lesbar","username":"","display_name":"Steam   Support"}}"#,
        ));
        assert_eq!(only_display.user_id, None);
        assert_eq!(only_display.username, None);
        assert_eq!(only_display.display_name.as_deref(), Some("Steam Support"));
    }

    #[test]
    fn parses_analyzer_json_and_clamps_confidence() {
        let parsed = parse_content_analysis(Some(
            r#"noise {"category":"scam","confidence":"1.4","reason":"  Crypto   Scam  "} tail"#,
        ));

        assert_eq!(parsed.category, ModerationCategory::Scam);
        assert_eq!(parsed.confidence, 1.0);
        assert_eq!(parsed.reason, "Crypto Scam");
        assert!(parsed.raw_json.contains("response_text"));
    }

    #[test]
    fn non_finite_confidence_becomes_zero() {
        let parsed = parse_content_analysis(Some(
            r#"{"category":"scam","confidence":"NaN","reason":"nan"}"#,
        ));

        assert_eq!(parsed.confidence, 0.0);
    }

    #[test]
    fn bare_nsfw_is_not_high_damage() {
        assert!(!ModerationCategory::from_label("nsfw").is_high_damage());
        assert!(ModerationCategory::from_label("nsfw_explicit").is_high_damage());
        assert!(!ModerationCategory::from_label("nsfw_suggestive").is_high_damage());
        assert!(!ModerationCategory::from_label("suggestive").is_high_damage());
    }

    #[test]
    fn parses_verifier_json_with_refute_result() {
        let parsed = parse_verification_decision(
            Some(
                r#"{"confirmed":true,"category":"harassment","confidence":0.64,"reason":"Gezielter Angriff"}"#,
            ),
            ModerationCategory::Harassment,
        );

        assert!(parsed.confirmed);
        assert_eq!(parsed.category, ModerationCategory::Harassment);
        assert_eq!(parsed.confidence, 0.64);
        assert_eq!(parsed.reason, "Gezielter Angriff");
    }

    #[test]
    fn parse_errors_become_unconfirmed_other() {
        let parsed = parse_content_analysis(Some("kein json"));
        assert_eq!(parsed.category, ModerationCategory::Other);
        assert_eq!(parsed.confidence, 0.0);
        assert_eq!(parsed.reason, "parse_error");

        let verify = parse_verification_decision(None, ModerationCategory::Scam);
        assert!(!verify.confirmed);
        assert_eq!(verify.category, ModerationCategory::Other);
        assert_eq!(verify.confidence, 0.0);
    }
}
