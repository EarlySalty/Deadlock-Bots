use std::sync::Arc;

use crate::content_analyzer::ModerationInput;
use crate::moderation_verdict::{
    parse_verification_decision, ContentAnalysis, VerificationDecision,
};

pub const BEHAVIOR_VERIFIER_SYSTEM_PROMPT: &str = r#"Du bist die unabhängige zweite Moderationsinstanz für einen technischen Heuristik-Treffer.
Die Heuristik darf falsch liegen und ist kein Inhaltsbeweis. Prüfe Text und jedes mitgesendete Bild selbst.
Die erste KI-Analyse ist ebenfalls nur ein Verdacht und darf falsch liegen.
Setze confirmed=true ausschließlich dann, wenn der sichtbare Inhalt selbst tatsächlich eine schädliche Kategorie belegt.
Ein normaler Gaming-Screenshot, Social-Media-Post, Meme, News-Bild oder sonstiges harmloses Bild ist confirmed=false und category=other.
Wichtig: Das Screenshot-Format macht schädlichen Inhalt nicht harmlos. Sichtbare Fake-Giveaways, Promi-Impersonation, Krypto-/Casino-Boni, Promo-Codes, garantierte Gewinne oder Auszahlungsversprechen sind als scam zu klassifizieren, wenn die Nachricht sie selbst bewirbt und kein sichtbarer Reporting-/Warnkontext sie klar einordnet.
Kategorie, confirmed und Begründung müssen logisch zusammenpassen. Wenn deine Begründung einen tatsächlichen Scam, Betrugsversuch oder Phishing-Inhalt beschreibt, darf category nicht other sein.
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

    pub(crate) async fn verify_behavior_trigger(
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
        parse_verification_decision(raw.as_deref(), analysis.category.clone())
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

#[cfg(test)]
mod tests {
    use super::*;

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
