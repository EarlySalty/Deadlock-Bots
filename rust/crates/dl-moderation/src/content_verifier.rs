use std::sync::Arc;

use crate::content_analyzer::ModerationInput;
use crate::moderation_verdict::{
    parse_verification_decision, ContentAnalysis, VerificationDecision,
};

pub const VERIFIER_SYSTEM_PROMPT: &str = r#"Du bist die zweite Moderationsinstanz.
Widerlege den Verdacht aktiv: Ist die Nachricht wirklich die angegebene Kategorie oder ist sie harmloser Kontext, Reporting, Ironie, Gaming-Trash-Talk oder normales Serverrauschen?
Helden-, Rollen-, Rank- oder Spielergruppen-Spott im Spielkontext ist Trash-Talk/Ragebait, nicht harassment oder hate_speech.
Antworte ausschliesslich als JSON:
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
        let raw = if input.image_urls.is_empty() {
            self.text
                .generate_text(dl_ai::GenerateRequest {
                    prompt,
                    system_prompt: Some(VERIFIER_SYSTEM_PROMPT.to_string()),
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
                    system_prompt: Some(VERIFIER_SYSTEM_PROMPT.to_string()),
                    model: Some(self.config.model.clone()),
                    max_output_tokens: Some(300),
                    temperature: 0.0,
                })
                .await
        } else {
            None
        };
        parse_verification_decision(raw.as_deref(), analysis.category.clone())
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn skip_redundant_image_verify(&self, analysis: &ContentAnalysis) -> VerificationDecision {
        VerificationDecision {
            confirmed: true,
            category: analysis.category.clone(),
            confidence: analysis.confidence,
            reason: analysis.reason.clone(),
            raw_json: serde_json::json!({
                "skipped": true,
                "reason": "redundant_image_verify_same_model",
                "model": self.config.model.clone(),
            })
            .to_string(),
        }
    }
}

pub fn build_verifier_prompt(message: &str, category: &str, analysis_reason: &str) -> String {
    serde_json::json!({
        "task": "Widerlege den Verdacht; bestaetige nur, wenn der Verstoss wirklich vorliegt.",
        "suspected_category": category,
        "analysis_reason": analysis_reason,
        "message_or_image_context": message,
        "safe_alternatives": [
            "harmlos",
            "Trash-Talk",
            "Gaming-Kontext",
            "Reporting oder Zitat",
            "Ironie"
        ],
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
}
