use serde::{Deserialize, Serialize};
use serde_json::Value;

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

pub(crate) fn high_confidence_scam_reason_conflict(
    category: &ModerationCategory,
    confidence: f64,
    reason: &str,
) -> bool {
    if !matches!(category, ModerationCategory::Other) || confidence < 0.80 {
        return false;
    }

    let reason = reason.to_ascii_lowercase();
    let explicit_scam = [
        "scam-muster",
        "scammuster",
        "scam-merkmal",
        "scammerkmal",
        "betrugsmuster",
        "betrugs-muster",
        "betrugsversuch",
        "phishing",
        "krypto-scam",
        "crypto-scam",
        "klarer scam",
        "eindeutiger scam",
    ]
    .iter()
    .any(|needle| reason.contains(needle));

    if !explicit_scam {
        return false;
    }

    let uncertainty_or_safe_context = [
        "kein scam",
        "kein betrug",
        "kein phishing",
        "nicht eindeutig",
        "nicht belegt",
        "nicht klar",
        "möglich",
        "moeglich",
        "könnte",
        "koennte",
        "verdacht",
        "unklar",
        "wirkt wie",
        "reporting",
        "warnung",
        "warnt",
        "zitat",
    ]
    .iter()
    .any(|needle| reason.contains(needle));

    !uncertainty_or_safe_context
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
            reason: "parse_error".to_string(),
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

pub fn parse_verification_decision(
    raw_text: Option<&str>,
    _suspected_category: ModerationCategory,
) -> VerificationDecision {
    let Some(payload) = extract_json_object(raw_text) else {
        return VerificationDecision {
            confirmed: false,
            category: ModerationCategory::Other,
            confidence: 0.0,
            reason: "parse_error".to_string(),
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
    fn detects_live_scam_reason_category_contradiction_without_flagging_uncertainty() {
        assert!(high_confidence_scam_reason_conflict(
            &ModerationCategory::Other,
            0.90,
            "Screenshot zeigt Krypto-Casino, Promo-Code und Auszahlung, typisches Betrugs-/Scam-Muster."
        ));
        assert!(high_confidence_scam_reason_conflict(
            &ModerationCategory::Other,
            0.84,
            "Sichtbarer Werbetext verspricht $100K+ within a week und verlangt Telegram-Kontakt; typische unseriöse Anzeige mit Scam-Merkmalen."
        ));
        assert!(!high_confidence_scam_reason_conflict(
            &ModerationCategory::Other,
            0.62,
            "Wirkt wie möglicher Scam, aber der Betrugscharakter ist nicht eindeutig belegt."
        ));
        assert!(!high_confidence_scam_reason_conflict(
            &ModerationCategory::Other,
            0.99,
            "Kein Phishing sichtbar, normaler Screenshot."
        ));
        assert!(!high_confidence_scam_reason_conflict(
            &ModerationCategory::Scam,
            0.99,
            "Eindeutiger Scam."
        ));
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
