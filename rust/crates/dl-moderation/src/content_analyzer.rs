use std::sync::Arc;

use crate::content_verifier::ContentVerifier;
use crate::moderation_verdict::{
    parse_content_analysis, ContentAnalysis, ModerationCategory, ModerationVerdict,
};

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub(crate) struct LogCapture {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl LogCapture {
        pub(crate) fn text(&self) -> String {
            String::from_utf8_lossy(&self.bytes.lock().expect("log capture")).to_string()
        }
    }

    pub(crate) struct LogCaptureWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for LogCaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes
                .lock()
                .expect("log capture")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
        type Writer = LogCaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            LogCaptureWriter {
                bytes: self.bytes.clone(),
            }
        }
    }
}

pub const ANALYZER_SYSTEM_PROMPT: &str = r#"Du bist ein Discord-Moderations-Analyzer fuer einen Gaming-Server.
Analysiere Text und Bilder knapp und konservativ.
Gib eine Kategorie, Confidence und eine kurze Begruendung auf Deutsch zurueck.
Flagge nur echte Risiken; normaler Gaming-Trash-Talk und harmlose Meldungen sollen game_related_ok oder ragebait_ok sein.
Helden-, Rollen-, Rank- oder Spielergruppen-Spott im Spielkontext ist Trash-Talk/Ragebait, nicht harassment oder hate_speech.
Antworte ausschliesslich als JSON:
{"category":"scam|csam|nsfw_explicit|harassment|hate_speech|ragebait_ok|game_related_ok|other","confidence":0.0,"reason":"kurz auf Deutsch"}"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModerationInput {
    pub content: String,
    pub image_urls: Vec<String>,
}

impl ModerationInput {
    pub fn new(content: impl Into<String>, image_urls: Vec<String>) -> Self {
        Self {
            content: content.into(),
            image_urls,
        }
    }

    pub fn text(content: impl Into<String>) -> Self {
        Self::new(content, Vec::new())
    }

    pub fn is_image_only(&self) -> bool {
        self.content.trim().is_empty() && !self.image_urls.is_empty()
    }

    pub fn has_text(&self) -> bool {
        !self.content.trim().is_empty()
    }

    pub fn text_only(&self) -> Self {
        Self::new(self.content.clone(), Vec::new())
    }

    pub fn prompt_text(&self) -> String {
        let content = self.content.trim();
        if content.is_empty() && !self.image_urls.is_empty() {
            "[Bild ohne Text]".to_string()
        } else {
            content.to_string()
        }
    }

    pub fn trigger_preview(&self) -> String {
        let mut trigger = self.prompt_text();
        if !self.image_urls.is_empty() {
            if !trigger.is_empty() {
                trigger.push('\n');
            }
            trigger.push_str(
                &self
                    .image_urls
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        trigger.chars().take(900).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAnalyzerConfig {
    pub text_model: String,
    pub image_model: String,
}

impl Default for ContentAnalyzerConfig {
    fn default() -> Self {
        Self {
            text_model: dl_ai::DEFAULT_FIREWORKS_MODEL.to_string(),
            image_model: dl_ai::DEFAULT_OPENAI_MODEL.to_string(),
        }
    }
}

pub struct ContentAnalyzer {
    text: Arc<dyn dl_ai::TextGenerator>,
    vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
    config: ContentAnalyzerConfig,
}

impl ContentAnalyzer {
    pub fn new(
        text: Arc<dyn dl_ai::TextGenerator>,
        vision: Option<Arc<dyn dl_ai::VisionGenerator>>,
        config: ContentAnalyzerConfig,
    ) -> Self {
        Self {
            text,
            vision,
            config,
        }
    }

    pub async fn analyze(&self, input: &ModerationInput) -> ContentAnalysis {
        self.analyze_modal(input).await.analysis
    }

    async fn analyze_modal(&self, input: &ModerationInput) -> ModalAnalysis {
        let mut analyses = Vec::new();

        if input.has_text() {
            let prompt = analyzer_prompt("text", input.content.trim(), 0);
            let raw = self
                .text
                .generate_text(dl_ai::GenerateRequest {
                    prompt,
                    system_prompt: Some(ANALYZER_SYSTEM_PROMPT.to_string()),
                    model: Some(self.config.text_model.clone()),
                    max_output_tokens: Some(300),
                    reasoning_effort: None,
                    temperature: 0.0,
                })
                .await;
            if raw.is_none() {
                log_provider_failure(input, "text", &self.config.text_model);
            }
            analyses.push(ModalAnalysis {
                analysis: parse_content_analysis(raw.as_deref()),
                modality: AnalysisModality::Text,
            });
        }

        if !input.image_urls.is_empty() {
            let raw = if let Some(vision) = &self.vision {
                let prompt = analyzer_prompt("image", &input.prompt_text(), input.image_urls.len());
                vision
                    .generate_multimodal(dl_ai::GenerateMultimodalRequest {
                        prompt,
                        image_urls: input.image_urls.clone(),
                        system_prompt: Some(ANALYZER_SYSTEM_PROMPT.to_string()),
                        model: Some(self.config.image_model.clone()),
                        max_output_tokens: Some(300),
                        temperature: 0.0,
                    })
                    .await
            } else {
                None
            };
            if raw.is_none() {
                log_provider_failure(input, "image", &self.config.image_model);
            }
            analyses.push(ModalAnalysis {
                analysis: parse_content_analysis(raw.as_deref()),
                modality: AnalysisModality::Image,
            });
        }

        analyses
            .into_iter()
            .reduce(choose_more_severe)
            .unwrap_or(ModalAnalysis {
                analysis: parse_content_analysis(None),
                modality: AnalysisModality::Text,
            })
    }

    fn image_model(&self) -> &str {
        &self.config.image_model
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalysisModality {
    Text,
    Image,
}

#[derive(Debug, Clone)]
struct ModalAnalysis {
    analysis: ContentAnalysis,
    modality: AnalysisModality,
}

#[derive(Debug, Clone)]
pub(crate) struct ContentModerationEvaluation {
    pub analysis: ContentAnalysis,
    pub verdict: Option<ModerationVerdict>,
}

fn log_provider_failure(input: &ModerationInput, modality: &str, model: &str) {
    let input = input.trigger_preview();
    let verdict = "error";
    let reason = "provider_timeout_or_error";
    tracing::error!(
        input = %input,
        verdict = %verdict,
        reason = %reason,
        modality,
        model,
        "Moderation: Analyzer ohne Provider-Antwort"
    );
}

fn analyzer_prompt(modality: &str, message: &str, image_count: usize) -> String {
    serde_json::json!({
        "modality": modality,
        "message": message,
        "image_count": image_count,
    })
    .to_string()
}

fn choose_more_severe(left: ModalAnalysis, right: ModalAnalysis) -> ModalAnalysis {
    if is_more_severe(&right, &left) {
        right
    } else {
        left
    }
}

fn is_more_severe(candidate: &ModalAnalysis, current: &ModalAnalysis) -> bool {
    let candidate_tier = category_tier(&candidate.analysis.category);
    let current_tier = category_tier(&current.analysis.category);
    if candidate_tier != current_tier {
        return candidate_tier > current_tier;
    }
    if candidate.analysis.confidence != current.analysis.confidence {
        return candidate.analysis.confidence > current.analysis.confidence;
    }
    matches!(candidate.modality, AnalysisModality::Text)
        && matches!(current.modality, AnalysisModality::Image)
}

fn category_tier(category: &ModerationCategory) -> u8 {
    if category.is_high_damage() {
        3
    } else if matches!(
        category,
        ModerationCategory::Harassment | ModerationCategory::HateSpeech
    ) {
        2
    } else if category.is_harmless() {
        0
    } else {
        1
    }
}

pub struct ContentModerationPipeline {
    analyzer: ContentAnalyzer,
    verifier: ContentVerifier,
    analyze_flag_threshold: f64,
}

impl ContentModerationPipeline {
    pub fn new(
        analyzer: ContentAnalyzer,
        verifier: ContentVerifier,
        analyze_flag_threshold: f64,
    ) -> Self {
        Self {
            analyzer,
            verifier,
            analyze_flag_threshold,
        }
    }

    pub async fn evaluate(&self, input: &ModerationInput) -> Option<ModerationVerdict> {
        self.evaluate_with_analysis(input).await.verdict
    }

    pub(crate) async fn evaluate_with_analysis(
        &self,
        input: &ModerationInput,
    ) -> ContentModerationEvaluation {
        let modal_analysis = self.analyzer.analyze_modal(input).await;
        let analysis = modal_analysis.analysis;
        if analysis.confidence < self.analyze_flag_threshold || analysis.category.is_harmless() {
            return ContentModerationEvaluation {
                analysis,
                verdict: None,
            };
        }
        let verification =
            if input.is_image_only() && self.analyzer.image_model() == self.verifier.model() {
                self.verifier.skip_redundant_image_verify(&analysis)
            } else if matches!(modal_analysis.modality, AnalysisModality::Text) {
                self.verifier.verify(&input.text_only(), &analysis).await
            } else {
                self.verifier.verify(input, &analysis).await
            };
        ContentModerationEvaluation {
            analysis: analysis.clone(),
            verdict: Some(ModerationVerdict {
                analysis,
                verification,
                trigger: input.trigger_preview(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_analyzer::test_support::LogCapture;
    use crate::content_verifier::ContentVerifier;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct RecordingText {
        responses: Mutex<Vec<String>>,
        models: Mutex<Vec<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl dl_ai::TextGenerator for RecordingText {
        async fn generate_text(&self, request: dl_ai::GenerateRequest) -> Option<String> {
            self.models.lock().await.push(request.model);
            self.responses.lock().await.pop()
        }
    }

    #[derive(Default)]
    struct RecordingVision {
        responses: Mutex<Vec<String>>,
        models: Mutex<Vec<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl dl_ai::VisionGenerator for RecordingVision {
        async fn generate_multimodal(
            &self,
            request: dl_ai::GenerateMultimodalRequest,
        ) -> Option<String> {
            self.models.lock().await.push(request.model);
            self.responses.lock().await.pop()
        }
    }

    #[tokio::test]
    async fn analyzer_routes_text_to_fireworks_text_model() {
        let text = Arc::new(RecordingText::default());
        let vision = Arc::new(RecordingVision::default());
        text.responses
            .lock()
            .await
            .push(r#"{"category":"scam","confidence":0.7,"reason":"Verdacht"}"#.to_string());
        let analyzer = ContentAnalyzer::new(
            text.clone(),
            Some(vision.clone()),
            ContentAnalyzerConfig {
                text_model: dl_ai::DEFAULT_FIREWORKS_MODEL.to_string(),
                image_model: "gpt-5.4-nano".to_string(),
            },
        );

        let result = analyzer.analyze(&ModerationInput::text("hi")).await;

        assert_eq!(result.category.as_label(), "scam");
        assert_eq!(
            text.models.lock().await.as_slice(),
            &[Some(dl_ai::DEFAULT_FIREWORKS_MODEL.to_string())]
        );
        assert!(vision.models.lock().await.is_empty());
    }

    #[tokio::test]
    async fn analyzer_logs_missing_provider_response_as_error() {
        let capture = LogCapture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .without_time()
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        let analyzer = ContentAnalyzer::new(
            Arc::new(RecordingText::default()),
            None,
            ContentAnalyzerConfig::default(),
        );

        let result = analyzer
            .analyze(&ModerationInput::text("free crypto"))
            .await;
        drop(guard);

        assert_eq!(result.category, ModerationCategory::Other);
        assert_eq!(result.reason, "parse_error");
        let logs = capture.text();
        assert!(logs.contains("ERROR"), "{logs}");
        assert!(logs.contains("input=free crypto"), "{logs}");
        assert!(logs.contains("verdict=error"), "{logs}");
        assert!(logs.contains("reason=provider_timeout_or_error"), "{logs}");
    }

    #[tokio::test]
    async fn analyzer_routes_images_to_openai_vision_model() {
        let text = Arc::new(RecordingText::default());
        let vision = Arc::new(RecordingVision::default());
        vision
            .responses
            .lock()
            .await
            .push(r#"{"category":"nsfw_explicit","confidence":0.8,"reason":"Bild"}"#.to_string());
        let analyzer = ContentAnalyzer::new(
            text.clone(),
            Some(vision.clone()),
            ContentAnalyzerConfig {
                text_model: dl_ai::DEFAULT_FIREWORKS_MODEL.to_string(),
                image_model: "gpt-5.4-nano".to_string(),
            },
        );

        let result = analyzer
            .analyze(&ModerationInput::new(
                "",
                vec!["https://example.test/i.png".to_string()],
            ))
            .await;

        assert_eq!(result.category.as_label(), "nsfw_explicit");
        assert!(text.models.lock().await.is_empty());
        assert_eq!(
            vision.models.lock().await.as_slice(),
            &[Some("gpt-5.4-nano".to_string())]
        );
    }

    #[tokio::test]
    async fn analyzer_routes_mixed_text_and_images_through_both_modalities() {
        let text = Arc::new(RecordingText::default());
        let vision = Arc::new(RecordingVision::default());
        text.responses
            .lock()
            .await
            .push(r#"{"category":"scam","confidence":0.91,"reason":"Text"}"#.to_string());
        vision
            .responses
            .lock()
            .await
            .push(r#"{"category":"harassment","confidence":0.99,"reason":"Bild"}"#.to_string());
        let analyzer = ContentAnalyzer::new(
            text.clone(),
            Some(vision.clone()),
            ContentAnalyzerConfig {
                text_model: dl_ai::DEFAULT_FIREWORKS_MODEL.to_string(),
                image_model: "gpt-5.4-nano".to_string(),
            },
        );

        let result = analyzer
            .analyze(&ModerationInput::new(
                "free crypto",
                vec!["https://example.test/i.png".to_string()],
            ))
            .await;

        assert_eq!(result.category.as_label(), "scam");
        assert_eq!(
            text.models.lock().await.as_slice(),
            &[Some(dl_ai::DEFAULT_FIREWORKS_MODEL.to_string())]
        );
        assert_eq!(
            vision.models.lock().await.as_slice(),
            &[Some("gpt-5.4-nano".to_string())]
        );
    }

    #[tokio::test]
    async fn pipeline_skips_verifier_below_analyze_threshold() {
        let text = Arc::new(RecordingText::default());
        text.responses
            .lock()
            .await
            .push(r#"{"category":"harassment","confidence":0.49,"reason":"Mild"}"#.to_string());
        let verifier_text = Arc::new(RecordingText::default());
        verifier_text.responses.lock().await.push(
            r#"{"confirmed":true,"category":"harassment","confidence":0.9,"reason":"x"}"#
                .to_string(),
        );
        let pipeline = ContentModerationPipeline::new(
            ContentAnalyzer::new(text, None, ContentAnalyzerConfig::default()),
            ContentVerifier::new(verifier_text.clone(), None, Default::default()),
            0.5,
        );

        let result = pipeline
            .evaluate(&ModerationInput::text("trash talk"))
            .await;

        assert!(result.is_none());
        assert!(verifier_text.models.lock().await.is_empty());
    }

    #[tokio::test]
    async fn pipeline_calls_verifier_for_flagged_analysis() {
        let text = Arc::new(RecordingText::default());
        text.responses
            .lock()
            .await
            .push(r#"{"category":"scam","confidence":0.51,"reason":"Scamverdacht"}"#.to_string());
        let verifier_text = Arc::new(RecordingText::default());
        verifier_text.responses.lock().await.push(
            r#"{"confirmed":true,"category":"scam","confidence":0.86,"reason":"Bestätigt"}"#
                .to_string(),
        );
        let pipeline = ContentModerationPipeline::new(
            ContentAnalyzer::new(text, None, ContentAnalyzerConfig::default()),
            ContentVerifier::new(verifier_text.clone(), None, Default::default()),
            0.5,
        );

        let result = pipeline
            .evaluate(&ModerationInput::text("free crypto"))
            .await;

        assert!(result.is_some());
        assert_eq!(verifier_text.models.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn pipeline_verifies_mixed_text_basis_through_text_verifier() {
        let text = Arc::new(RecordingText::default());
        let vision = Arc::new(RecordingVision::default());
        text.responses
            .lock()
            .await
            .push(r#"{"category":"scam","confidence":0.91,"reason":"Text"}"#.to_string());
        vision
            .responses
            .lock()
            .await
            .push(r#"{"category":"harassment","confidence":0.99,"reason":"Bild"}"#.to_string());
        let verifier_text = Arc::new(RecordingText::default());
        verifier_text.responses.lock().await.push(
            r#"{"confirmed":true,"category":"scam","confidence":0.9,"reason":"ok"}"#.to_string(),
        );
        let verifier_vision = Arc::new(RecordingVision::default());
        verifier_vision.responses.lock().await.push(
            r#"{"confirmed":true,"category":"harassment","confidence":0.9,"reason":"wrong"}"#
                .to_string(),
        );
        let pipeline = ContentModerationPipeline::new(
            ContentAnalyzer::new(
                text,
                Some(vision),
                ContentAnalyzerConfig {
                    text_model: dl_ai::DEFAULT_FIREWORKS_MODEL.to_string(),
                    image_model: "gpt-5.4-nano".to_string(),
                },
            ),
            ContentVerifier::new(
                verifier_text.clone(),
                Some(verifier_vision.clone()),
                Default::default(),
            ),
            0.5,
        );

        let result = pipeline
            .evaluate(&ModerationInput::new(
                "free crypto",
                vec!["https://example.test/i.png".to_string()],
            ))
            .await;

        let Some(verdict) = result else {
            panic!("expected mixed flag verdict");
        };
        assert_eq!(verdict.analysis.category.as_label(), "scam");
        assert_eq!(verdict.verification.category.as_label(), "scam");
        assert_eq!(verifier_text.models.lock().await.len(), 1);
        assert_eq!(verifier_vision.models.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn pipeline_skips_redundant_nano_verify_for_pure_image_flags() {
        let text = Arc::new(RecordingText::default());
        let vision = Arc::new(RecordingVision::default());
        vision
            .responses
            .lock()
            .await
            .push(r#"{"category":"scam","confidence":0.9,"reason":"Bildscam"}"#.to_string());
        let verifier_text = Arc::new(RecordingText::default());
        let verifier_vision = Arc::new(RecordingVision::default());
        let pipeline = ContentModerationPipeline::new(
            ContentAnalyzer::new(
                text,
                Some(vision),
                ContentAnalyzerConfig {
                    text_model: dl_ai::DEFAULT_FIREWORKS_MODEL.to_string(),
                    image_model: "gpt-5.4-nano".to_string(),
                },
            ),
            ContentVerifier::new(
                verifier_text.clone(),
                Some(verifier_vision.clone()),
                Default::default(),
            ),
            0.5,
        );

        let result = pipeline
            .evaluate(&ModerationInput::new(
                "",
                vec!["https://example.test/i.png".to_string()],
            ))
            .await;

        let Some(verdict) = result else {
            panic!("expected pure image flag verdict");
        };
        assert!(verdict.verification.confirmed);
        assert_eq!(verdict.verification.category.as_label(), "scam");
        assert_eq!(verifier_text.models.lock().await.len(), 0);
        assert_eq!(verifier_vision.models.lock().await.len(), 0);
    }
}
