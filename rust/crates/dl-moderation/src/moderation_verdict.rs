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
    !matches!(category, ModerationCategory::Scam)
        && confidence >= 0.80
        && explicit_scam_reason(reason)
}

pub(crate) fn high_confidence_scam_verification_conflict(
    verification: &VerificationDecision,
) -> bool {
    if verification.confidence < 0.80 || !explicit_scam_reason(&verification.reason) {
        return false;
    }

    !verification.confirmed || !matches!(verification.category, ModerationCategory::Scam)
}

const EXPLICIT_SCAM: &[&str] = &[
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
    "sichtbarer scam",
    "scam bestätigt",
    "scam bestaetigt",
];

const CONTRAST_CONJUNCTIONS: &[&str] = &[
    " aber ",
    " doch ",
    " stattdessen ",
    " jedoch ",
    " hingegen ",
    " sondern ",
    " allerdings ",
];

fn explicit_scam_reason(reason: &str) -> bool {
    let reason = reason.to_lowercase();

    reason
        .split([';', '.', '!', '?', '\n'])
        .flat_map(split_assertion_scopes)
        .any(|part| {
            EXPLICIT_SCAM.iter().any(|needle| {
                part.match_indices(needle)
                    .any(|(index, _)| scam_mention_is_asserted(&part, index, needle.len()))
            })
        })
}

fn split_assertion_scopes(part: &str) -> Vec<String> {
    let mut scopes = Vec::new();
    let mut current = String::new();
    let mut remaining = part;
    while !remaining.is_empty() {
        let next_conjunction = CONTRAST_CONJUNCTIONS
            .iter()
            .filter_map(|conjunction| {
                remaining
                    .find(conjunction)
                    .map(|index| (index, *conjunction))
            })
            .min_by_key(|(index, _)| *index);
        let Some((index, conjunction)) = next_conjunction else {
            current.push_str(remaining);
            break;
        };

        current.push_str(&remaining[..index]);
        let continuation = &remaining[index + conjunction.len()..];
        let next_boundary = CONTRAST_CONJUNCTIONS
            .iter()
            .filter_map(|candidate| continuation.find(candidate))
            .min()
            .unwrap_or(continuation.len());
        let next_clause = &continuation[..next_boundary];
        if !current.is_empty()
            && EXPLICIT_SCAM
                .iter()
                .any(|needle| next_clause.contains(needle))
        {
            scopes.push(std::mem::take(&mut current));
        } else {
            current.push_str(conjunction);
        }
        remaining = continuation;
    }
    if !current.is_empty() {
        scopes.push(current);
    }
    scopes
}

fn text_before_word(text: &str, word_position: usize) -> &str {
    let mut words_seen = 0;
    let mut in_word = false;
    for (index, character) in text.char_indices() {
        if character.is_alphanumeric() {
            if !in_word {
                if words_seen == word_position {
                    return &text[..index];
                }
                words_seen += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    text
}

fn direct_warn_report_context(prefix: &str) -> bool {
    let punctuation_boundary = prefix
        .char_indices()
        .filter_map(|(index, character)| {
            matches!(character, ';' | '.' | '!' | '?' | ',' | ':' | '\n')
                .then_some(index + character.len_utf8())
        })
        .max()
        .unwrap_or(0);
    let conjunction_boundary = CONTRAST_CONJUNCTIONS
        .iter()
        .filter_map(|conjunction| {
            prefix
                .rfind(conjunction)
                .map(|index| index + conjunction.len())
        })
        .max()
        .unwrap_or(0);
    let clause_prefix = &prefix[punctuation_boundary.max(conjunction_boundary)..];
    let words = clause_prefix
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let Some(preposition) = words.iter().rposition(|word| matches!(*word, "in" | "im")) else {
        return false;
    };

    words[preposition + 1..].iter().all(|word| {
        matches!(
            *word,
            "ein"
                | "eine"
                | "einer"
                | "einem"
                | "einen"
                | "eines"
                | "der"
                | "die"
                | "das"
                | "dem"
                | "den"
                | "dieser"
                | "diese"
                | "dieses"
                | "diesem"
                | "diesen"
                | "jeder"
                | "jede"
                | "jedes"
                | "jedem"
                | "jeden"
        ) || ["e", "en", "er", "es", "em"]
            .iter()
            .any(|ending| word.ends_with(ending))
    })
}

fn scam_mention_is_asserted(part: &str, index: usize, length: usize) -> bool {
    let prefix = &part[..index];
    let suffix = part[index + length..].trim_start_matches('-');
    if prefix.ends_with("anti-")
        || prefix.ends_with("gegen-")
        || ["schutz", "hinweis", "warnung", "prävention", "praevention"]
            .iter()
            .any(|safe_suffix| suffix.starts_with(safe_suffix))
    {
        return false;
    }
    let before = prefix
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let context = before.as_slice();
    if context.iter().enumerate().any(|(position, word)| {
        if (*word == "nicht"
            && context.get(position + 1).is_some_and(|next| {
                matches!(*next, "harmlos" | "im" | "in" | "auf" | "am" | "bei")
            }))
            || (matches!(*word, "kein" | "keine" | "keinen")
                && context.get(position + 1) == Some(&"zweifel"))
        {
            return false;
        }
        let trailing = &context[position + 1..];
        let assertion = [
            "sichtbarer scam",
            "klarer scam",
            "eindeutiger scam",
            "krypto-scam",
            "crypto-scam",
        ]
        .iter()
        .any(|assertion| part[index..].starts_with(assertion));
        if assertion
            && (trailing
                .iter()
                .rev()
                .take(2)
                .any(|word| matches!(*word, "und" | "oder"))
                || prefix
                    .rsplit_once(',')
                    .is_some_and(|(_, tail)| tail.trim().is_empty()))
        {
            return false;
        }
        if let Some(previous_mention) = trailing
            .iter()
            .rposition(|word| matches!(*word, "scam" | "phishing" | "betrug" | "betrugsversuch"))
        {
            if !trailing[previous_mention + 1..]
                .iter()
                .all(|word| matches!(*word, "und" | "oder" | "sowie"))
            {
                return false;
            }
        }
        matches!(
            *word,
            "kein"
                | "keine"
                | "keinen"
                | "keinem"
                | "keiner"
                | "keines"
                | "nicht"
                | "ohne"
                | "weder"
                | "fehlt"
                | "fehlend"
                | "unklar"
                | "verdacht"
                | "möglich"
                | "moeglich"
                | "möglicher"
                | "moeglicher"
                | "könnte"
                | "koennte"
                | "wirkt"
                | "vermutlich"
                | "reporting"
                | "bericht"
                | "berichtet"
                | "warnung"
                | "warnt"
                | "warnen"
                | "zitat"
                | "zitiert"
                | "zitierte"
        )
    }) {
        return false;
    }

    let after = part[index + length..]
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(5)
        .collect::<Vec<_>>();
    if after.iter().enumerate().any(|(position, word)| {
        matches!(
            *word,
            "unbelegt" | "unbestätigt" | "unbestaetigt" | "zitiert" | "zitierte"
        ) && !after[..position].iter().any(|prior| {
            matches!(
                *prior,
                "und" | "oder" | "scam" | "phishing" | "betrugsversuch"
            )
        })
    }) || after.iter().enumerate().any(|(position, word)| {
        let lead = &after[..position];
        matches!(*word, "warnung" | "bericht" | "reporting" | "zitat")
            && !lead.contains(&"ohne")
            && !lead.contains(&"und")
            && !lead.contains(&"oder")
            && (position == 0
                || lead.contains(&"als")
                || direct_warn_report_context(text_before_word(&part[index + length..], position)))
    }) {
        return false;
    }
    !after.iter().take(3).enumerate().any(|(position, word)| {
        if *word == "weder" {
            return true;
        }
        *word == "nicht"
            && after[position + 1..].iter().take(3).any(|word| {
                matches!(
                    *word,
                    "eindeutig"
                        | "belegt"
                        | "klar"
                        | "sichtbar"
                        | "bestätigt"
                        | "bestaetigt"
                        | "nachweisbar"
                        | "erkennbar"
                        | "vorhanden"
                        | "sehen"
                        | "erkennen"
                        | "festgestellt"
                        | "nachweisen"
                )
            })
    })
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
        assert!(high_confidence_scam_reason_conflict(
            &ModerationCategory::Harassment,
            0.91,
            "Sichtbarer Scam mit Promo-Code und Auszahlungsversprechen."
        ));
        assert!(!high_confidence_scam_reason_conflict(
            &ModerationCategory::Scam,
            0.99,
            "Eindeutiger Scam."
        ));
    }

    #[test]
    fn negated_scam_mentions_do_not_create_conflicts() {
        let reasons = [
            "Nur ein harmloses Voting-/Event-Poster mit Pizzen („Who’s next?“, „You have 0 votes“, „Vote for your favorite pizza“); kein sichtbarer Scam-/Phishing-, Gewinn- oder Auszahlungsversprechen.",
            "Keine konkreten Scam-Merkmale im Screenshot.",
            "Ohne Anzeichen für einen Betrugsversuch oder Phishing.",
            "Phishing ist nicht sichtbar.",
            "Ein Scam-Muster ist nicht belegt.",
            "Warnung vor sichtbarem Phishing, keine Werbung.",
            "Der Screenshot zitiert einen sichtbaren Scam als Warnung.",
            "Eine Phishing-Warnung im Bericht.",
            "Phishing in einer Warnung vor Betrugsmaschen.",
            "Phishing im Bericht über Betrugsmaschen.",
            "Phishing in der offiziellen Warnung vor Betrugsmaschen.",
            "Kein Scam und Phishing sichtbar.",
            "Keine Anzeichen für Phishing, Scam oder Betrugsversuch.",
            "Phishing ist weder sichtbar noch plausibel.",
            "Phishing ist nicht zu sehen.",
            "Anti-Phishing-Hinweis ohne Werbelink.",
            "Phishing-Schutz ist hier die einzige Aussage.",
            "Phishing lässt sich nicht nachweisen.",
            "Keine auf dem Poster oder in den Bildunterschriften erkennbaren Hinweise auf Phishing.",
            "Phishing ist aber nicht sichtbar.",
            "Scam-Muster ist jedoch nicht belegt.",
        ];
        for reason in reasons {
            assert!(
                !high_confidence_scam_reason_conflict(&ModerationCategory::Other, 0.92, reason),
                "{reason}"
            );
            let verification = VerificationDecision {
                confirmed: false,
                category: ModerationCategory::Other,
                confidence: 0.92,
                reason: reason.to_string(),
                raw_json: "{}".to_string(),
            };
            assert!(
                !high_confidence_scam_verification_conflict(&verification),
                "{reason}"
            );
        }
    }

    #[test]
    fn asserted_scam_survives_unrelated_negation_and_warning_context() {
        for reason in [
            "Sichtbarer Scam in der Anzeige: Warnung an Moderatoren.",
            "Sichtbarer Scam in der Gruppe: Warnung an Moderatoren.",
        ] {
            assert!(
                high_confidence_scam_reason_conflict(&ModerationCategory::Other, 0.90, reason),
                "{reason}"
            );
        }
        for separator in [";", ".", "!", "?", "\n", ",", ":"] {
            let reason =
                format!("Sichtbarer Scam in der Anzeige{separator} Warnung an Moderatoren.");
            assert!(
                high_confidence_scam_reason_conflict(&ModerationCategory::Other, 0.90, &reason),
                "{reason}"
            );
        }
        for conjunction in CONTRAST_CONJUNCTIONS {
            let reason =
                format!("Sichtbarer Scam in einer Anzeige{conjunction}Warnung an Moderatoren.");
            assert!(
                high_confidence_scam_reason_conflict(&ModerationCategory::Other, 0.90, &reason),
                "{reason}"
            );
        }
        for reason in [
            "Kein Phishing, aber ein sichtbarer Scam mit Auszahlungsversprechen.",
            "Keine Scam-Merkmale im ersten Bild; im zweiten ein klarer Scam.",
            "Eine Warnung vor Phishing; danach bewirbt der Post einen Krypto-Scam.",
            "Das ist nicht harmlos: sichtbarer Scam mit Promo-Code.",
            "Sichtbarer Scam ohne Reporting-Kontext.",
            "Ohne Phishing ist dies ein sichtbarer Scam.",
            "Kein Spam und sichtbarer Scam.",
            "Sichtbarer Scam und Warnung an Moderatoren.",
            "Kein Phishing, doch sichtbarer Scam mit Promo-Code.",
            "Kein Phishing, stattdessen sichtbarer Scam mit Auszahlungsversprechen.",
            "Nicht im ersten Bild, sichtbarer Scam im zweiten.",
            "Kein Phishing und Krypto-Scam.",
            "Phishing ist aber nicht sichtbar, jedoch ein sichtbarer Scam.",
            "Sichtbarer Scam im Bild und Warnung an Moderatoren.",
            "Sichtbarer Scam im Bild, Warnung an Moderatoren.",
            "Kein Zweifel: sichtbarer Scam.",
            "Sichtbarer Scam und Phishing ist unbestätigt.",
        ] {
            assert!(
                high_confidence_scam_reason_conflict(&ModerationCategory::Other, 0.90, reason),
                "{reason}"
            );
        }
    }

    #[test]
    fn contrast_conjunctions_preserve_negation_and_positive_reframing() {
        for conjunction in [
            " aber ",
            " doch ",
            " stattdessen ",
            " jedoch ",
            " hingegen ",
            " sondern ",
            " allerdings ",
        ] {
            let negated = format!("Phishing{conjunction}nicht sichtbar.");
            assert!(!high_confidence_scam_reason_conflict(
                &ModerationCategory::Other,
                0.92,
                &negated
            ));
            let verification = VerificationDecision {
                confirmed: false,
                category: ModerationCategory::Other,
                confidence: 0.92,
                reason: negated.clone(),
                raw_json: "{}".to_string(),
            };
            assert!(!high_confidence_scam_verification_conflict(&verification));

            let asserted =
                format!("Kein Phishing,{conjunction}sichtbarer Scam mit Auszahlungsversprechen.");
            assert!(high_confidence_scam_reason_conflict(
                &ModerationCategory::Other,
                0.92,
                &asserted
            ));
            let verification = VerificationDecision {
                confirmed: false,
                category: ModerationCategory::Other,
                confidence: 0.92,
                reason: asserted,
                raw_json: "{}".to_string(),
            };
            assert!(high_confidence_scam_verification_conflict(&verification));
        }
    }

    #[test]
    fn detects_unconfirmed_scam_verification_contradiction() {
        let contradictory = VerificationDecision {
            confirmed: false,
            category: ModerationCategory::Scam,
            confidence: 0.91,
            reason: "Sichtbarer Scam mit Promo-Code und Auszahlungsversprechen.".to_string(),
            raw_json: "{}".to_string(),
        };
        assert!(high_confidence_scam_verification_conflict(&contradictory));

        let uncertain = VerificationDecision {
            confirmed: false,
            category: ModerationCategory::Scam,
            confidence: 0.91,
            reason: "Möglicher Scam, aber nicht eindeutig belegt.".to_string(),
            raw_json: "{}".to_string(),
        };
        assert!(!high_confidence_scam_verification_conflict(&uncertain));

        let wrong_category = VerificationDecision {
            confirmed: false,
            category: ModerationCategory::Harassment,
            confidence: 0.91,
            reason: "Sichtbarer Scam mit Promo-Code und Auszahlungsversprechen.".to_string(),
            raw_json: "{}".to_string(),
        };
        assert!(high_confidence_scam_verification_conflict(&wrong_category));

        let confirmed = VerificationDecision {
            confirmed: true,
            category: ModerationCategory::Scam,
            confidence: 0.91,
            reason: "Sichtbarer Scam.".to_string(),
            raw_json: "{}".to_string(),
        };
        assert!(!high_confidence_scam_verification_conflict(&confirmed));
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
