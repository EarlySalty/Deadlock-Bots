use std::sync::Arc;

use crate::content_analyzer::ModerationInput;
use crate::moderation_verdict::{
    high_confidence_scam_verification_conflict, parse_verification_decision, ContentAnalysis,
    VerificationDecision,
};

pub const VERIFIER_SYSTEM_PROMPT: &str = r#"Du bist die zweite Moderationsinstanz.
Widerlege den Verdacht aktiv: Ist die Nachricht wirklich die angegebene Kategorie oder ist sie harmloser Kontext, Reporting, Ironie, Gaming-Trash-Talk oder normales Serverrauschen?
Ein technischer Heuristik-Treffer, mehrere Bilder oder Posts in mehreren Kanälen sind kein Inhaltsbeweis. Bestätige den Verstoß nur anhand des sichtbaren Text- und Bildinhalts.
Normale Screenshots, Social-Media-Posts, Memes, News-Grafiken und Gaming-Bilder sind ohne erkennbaren schädlichen Inhalt harmlos.
Wichtig: Das Screenshot-Format macht schädlichen Inhalt nicht harmlos. Sichtbare Fake-Giveaways, Promi-Impersonation, Krypto-/Casino-Boni, Promo-Codes, garantierte Gewinne oder Auszahlungsversprechen sind als scam zu klassifizieren, wenn die Nachricht sie selbst bewirbt und kein sichtbarer Reporting-/Warnkontext sie klar einordnet.
Kategorie, confirmed und Begründung müssen logisch zusammenpassen. Wenn deine Begründung einen tatsächlichen Scam, Betrugsversuch oder Phishing-Inhalt beschreibt, darf category nicht other sein.
Helden-, Rollen-, Rank- oder Spielergruppen-Spott im Spielkontext ist Trash-Talk/Ragebait, nicht harassment oder hate_speech.
Antworte ausschließlich als JSON:
{"confirmed":true|false,"category":"scam|csam|nsfw_explicit|harassment|hate_speech|other","confidence":0.0,"reason":"kurz auf Deutsch"}"#;

pub const BEHAVIOR_VERIFIER_SYSTEM_PROMPT: &str = r#"Du bist die unabhängige zweite Moderationsinstanz für einen technischen Heuristik-Treffer.
Die Heuristik darf falsch liegen und ist kein Inhaltsbeweis. Prüfe Text und jedes mitgesendete Bild selbst.
Die erste KI-Analyse ist ebenfalls nur ein Verdacht und darf falsch liegen.
Setze confirmed=true ausschließlich dann, wenn der sichtbare Inhalt selbst tatsächlich eine schädliche Kategorie belegt.
Ein normaler Gaming-Screenshot, Social-Media-Post, Meme, News-Bild oder sonstiges harmloses Bild ist confirmed=false und category=other.
Wichtig: Das Screenshot-Format macht schädlichen Inhalt nicht harmlos. Sichtbare Fake-Giveaways, Promi-Impersonation, Krypto-/Casino-Boni, Promo-Codes, garantierte Gewinne oder Auszahlungsversprechen sind als scam zu klassifizieren, wenn die Nachricht sie selbst bewirbt und kein sichtbarer Reporting-/Warnkontext sie klar einordnet.
Kategorie, confirmed und Begründung müssen logisch zusammenpassen. Wenn deine Begründung einen tatsächlichen Scam, Betrugsversuch oder Phishing-Inhalt beschreibt, darf category nicht other sein.
Antworte ausschließlich als JSON:
{"confirmed":true|false,"category":"scam|csam|nsfw_explicit|harassment|hate_speech|other","confidence":0.0,"reason":"kurz auf Deutsch"}"#;

pub const CONSISTENCY_SYSTEM_PROMPT: &str = r#"Du prüfst ausschließlich eine widersprüchliche Moderationsentscheidung erneut.
Die vorherige Antwort hat eine nicht passende Kategorie oder confirmed=false geliefert, obwohl ihre eigene Begründung mit hoher Sicherheit ein konkretes Scam-/Betrugs-/Phishing-Muster beschrieben hat.
Bewerte den sichtbaren Inhalt neu und löse diesen Widerspruch auf. Das Screenshot-Format ist kein Entlastungsgrund: sichtbare Fake-Giveaways, Promi-Impersonation, Krypto-/Casino-Boni, Promo-Codes, garantierte Gewinne oder Auszahlungsversprechen sind scam, sofern kein sichtbarer Reporting-/Warnkontext sie klar als Bericht oder Warnung einordnet.
Ein technischer Heuristik-Treffer ist für sich kein Beweis. Entscheidend bleibt immer der sichtbare Inhalt.
Kategorie, confirmed und Begründung müssen logisch zusammenpassen.
Antworte ausschließlich als JSON:
{"confirmed":true|false,"category":"scam|csam|nsfw_explicit|harassment|hate_speech|other","confidence":0.0,"reason":"kurz auf Deutsch"}"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentVerifierConfig {
    pub model: String,
}

impl Default for ContentVerifierConfig {
    fn default() -> Self {
        Self {
            model: dl_ai::DEFAULT_OPENAI_MODEL.to_string(),
        }
    }
}

pub struct ContentVerifier {
    text: Arc<dyn dl_ai::TextGenerator>,
    vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
    config: ContentVerifierConfig,
}

impl ContentVerifier {
    pub fn new(
        text: Arc<dyn dl_ai::TextGenerator>,
        vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
        config: ContentVerifierConfig,
    ) -> Self {
        Self {
            text,
            vision,
            config,
        }
    }

    pub async fn verify(
        &self,
        input: &ModerationInput,
        analysis: &ContentAnalysis,
    ) -> VerificationDecision {
        let prompt = build_verifier_prompt(
            &input.prompt_text(),
            analysis.category.as_label(),
            &analysis.reason,
        );
        let raw = self.request(input, prompt, VERIFIER_SYSTEM_PROMPT).await;
        let verification = parse_verification_decision(raw.as_deref(), analysis.category.clone());

        if !high_confidence_scam_verification_conflict(&verification) {
            return verification;
        }

        tracing::warn!(
            category = verification.category.as_label(),
            confidence = verification.confidence,
            "Moderation: widersprüchliches Scam-Verifier-Urteil, Konsistenzprüfung wird wiederholt"
        );
        let retry_prompt = build_behavior_consistency_prompt(
            &input.prompt_text(),
            "content_scan",
            analysis,
            &verification,
        );
        let retry_raw = self
            .request(input, retry_prompt, CONSISTENCY_SYSTEM_PROMPT)
            .await;
        let retry = parse_verification_decision(retry_raw.as_deref(), analysis.category.clone());

        if retry.reason == "parse_error" {
            verification
        } else {
            retry
        }
    }

    pub async fn verify_behavior_trigger(
        &self,
        input: &ModerationInput,
        analysis: &ContentAnalysis,
        behavior_trigger: &str,
    ) -> VerificationDecision {
        let prompt = build_behavior_verifier_prompt(
            &input.prompt_text(),
            behavior_trigger,
            analysis.category.as_label(),
            analysis.confidence,
            &analysis.reason,
        );
        let raw = self
            .request(input, prompt, BEHAVIOR_VERIFIER_SYSTEM_PROMPT)
            .await;
        let verification = parse_verification_decision(raw.as_deref(), analysis.category.clone());

        if !high_confidence_scam_verification_conflict(&verification) {
            return verification;
        }

        tracing::warn!(
            behavior_trigger,
            category = verification.category.as_label(),
            confidence = verification.confidence,
            "Moderation: widersprüchliches Scam-Verifier-Urteil, Konsistenzprüfung wird wiederholt"
        );
        let retry_prompt = build_behavior_consistency_prompt(
            &input.prompt_text(),
            behavior_trigger,
            analysis,
            &verification,
        );
        let retry_raw = self
            .request(input, retry_prompt, CONSISTENCY_SYSTEM_PROMPT)
            .await;
        let retry = parse_verification_decision(retry_raw.as_deref(), analysis.category.clone());

        if retry.reason == "parse_error" {
            verification
        } else {
            retry
        }
    }

    async fn request(
        &self,
        input: &ModerationInput,
        prompt: String,
        system_prompt: &str,
    ) -> Option<String> {
        if input.image_urls.is_empty() {
            self.text
                .generate_text(dl_ai::GenerateRequest {
                    prompt,
                    system_prompt: Some(system_prompt.to_string()),
                    model: Some(self.config.model.clone()),
                    max_output_tokens: Some(300),
                    reasoning_effort: None,
                    temperature: 0.0,
                })
                .await
        } else if let Some(vision) = &self.vision {
            vision
                .generate_multimodal(dl_ai::GenerateMultimodalRequest {
                    prompt,
                    image_urls: input.image_urls.clone(),
                    system_prompt: Some(system_prompt.to_string()),
                    model: Some(self.config.model.clone()),
                    max_output_tokens: Some(300),
                    temperature: 0.0,
                })
                .await
        } else {
            None
        }
    }
}

pub fn build_verifier_prompt(message: &str, category: &str, analysis_reason: &str) -> String {
    serde_json::json!({
        "task": "Widerlege den Verdacht; bestätige nur, wenn der Verstoß wirklich vorliegt.",
        "suspected_category": category,
        "analysis_reason": analysis_reason,
        "message_or_image_context": message,
        "safe_alternatives": [
            "harmlos",
            "normaler Screenshot oder Social-Media-Post",
            "News-Grafik oder Meme",
            "Trash-Talk",
            "Gaming-Kontext",
            "Reporting oder Zitat",
            "Ironie"
        ],
    })
    .to_string()
}

pub fn build_behavior_verifier_prompt(
    message: &str,
    behavior_trigger: &str,
    analysis_category: &str,
    analysis_confidence: f64,
    analysis_reason: &str,
) -> String {
    serde_json::json!({
        "task": "Prüfe den Heuristik-Treffer unabhängig anhand des sichtbaren Inhalts.",
        "behavior_trigger": behavior_trigger,
        "analysis": {
            "category": analysis_category,
            "confidence": analysis_confidence,
            "reason": analysis_reason,
        },
        "message_or_image_context": message,
        "instruction": "Die Heuristik und die erste Analyse sind kein Beweis. Bilder selbst prüfen. Das Screenshot-Format allein ist kein Entlastungsgrund.",
    })
    .to_string()
}

pub fn build_behavior_consistency_prompt(
    message: &str,
    behavior_trigger: &str,
    analysis: &ContentAnalysis,
    verification: &VerificationDecision,
) -> String {
    serde_json::json!({
        "task": "Löse den Widerspruch in deiner vorherigen Entscheidung auf und klassifiziere den sichtbaren Inhalt erneut.",
        "behavior_trigger": behavior_trigger,
        "analysis": {
            "category": analysis.category.as_label(),
            "confidence": analysis.confidence,
            "reason": analysis.reason,
        },
        "previous_verification": {
            "confirmed": verification.confirmed,
            "category": verification.category.as_label(),
            "confidence": verification.confidence,
            "reason": verification.reason,
        },
        "message_or_image_context": message,
        "instruction": "Wenn die Begründung einen tatsächlichen Scam/Betrug/Phishing-Inhalt beschreibt, müssen category und confirmed dazu passen. Reporting oder Warnkontext darf weiterhin als other/unconfirmed bewertet werden.",
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_prompt_is_adversarial_refute_prompt() {
        let prompt = build_verifier_prompt("msg", "scam", "Verdacht");

        assert!(prompt.contains("Widerlege"));
        assert!(prompt.contains("harmlos"));
        assert!(prompt.contains("scam"));
        assert!(prompt.contains("Verdacht"));
    }

    #[test]
    fn behavior_verifier_prompt_marks_heuristic_and_analysis_as_untrusted() {
        let prompt =
            build_behavior_verifier_prompt("msg", "account_takeover", "scam", 0.99, "Verdacht");

        assert!(prompt.contains("account_takeover"));
        assert!(prompt.contains("unabhängig"));
        assert!(prompt.contains("kein Beweis"));
        assert!(prompt.contains("0.99"));
    }
}
