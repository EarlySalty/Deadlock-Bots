//! Read-only Messung echter Wissensantworten; keinerlei Discord-/Twitch-Transport.
use dl_ai::{ChatMessage, ChatParams, ChatProvider};
use dl_answer::{Answer, AnswerEngine, Retriever, Scope};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct MeasuredProvider {
    inner: Arc<dyn ChatProvider>,
    calls: std::sync::atomic::AtomicUsize,
    elapsed_ms: std::sync::atomic::AtomicU64,
    show_evidence: bool,
}
#[async_trait::async_trait]
impl ChatProvider for MeasuredProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<dl_ai::ChatResponse, dl_ai::ChatProviderError> {
        let start = Instant::now();
        if self.show_evidence {
            for message in messages {
                if message.role == dl_ai::ChatRole::User {
                    if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&message.content)
                    {
                        if let Some(evidence) = payload.get("evidence") {
                            eprintln!("{}", json!({"normalized_evidence": evidence}));
                        }
                    }
                }
            }
        }
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let result = self.inner.chat(messages, params).await;
        if let Err(error) = &result {
            // Ausschließlich feste Kategorien/Statusziffern, niemals Anbietertext ausgeben.
            let category = match error {
                dl_ai::ChatProviderError::Timeout => "timeout".to_string(),
                dl_ai::ChatProviderError::RateLimit => "rate_limit".to_string(),
                dl_ai::ChatProviderError::Auth => "authentication".to_string(),
                dl_ai::ChatProviderError::Provider(detail) => {
                    if let Some(code) = detail
                        .strip_prefix("HTTP ")
                        .and_then(|rest| rest.get(..3))
                        .filter(|code| code.bytes().all(|b| b.is_ascii_digit()))
                    {
                        format!("http_{code}")
                    } else if detail == "response truncated (max_tokens)" {
                        "truncated".to_string()
                    } else if detail == "missing chat response content" {
                        "empty_content".to_string()
                    } else if detail.contains("decode") || detail.contains("JSON") {
                        "decode".to_string()
                    } else {
                        "provider_other".to_string()
                    }
                }
            };
            eprintln!("{}", json!({"provider_error_category": category}));
        }
        self.elapsed_ms.fetch_add(
            start.elapsed().as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        result
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let base = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Aufruf: qa_probe WISSENSDIENST BRAIN-BIN FRAGEN.json"))?;
    let brain = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Brain-Bin fehlt"))?;
    let questions = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Fragen-Datei fehlt"))?;
    let flags = args.collect::<Vec<_>>();
    let legacy = flags.iter().any(|flag| flag == "--legacy");
    let show_evidence = flags.iter().any(|flag| flag == "--evidence");
    // Bestehender Connector/Infisical-Prozessvertrag; keine Ausgabe oder Ablage von Zugangsdaten.
    let config = dl_ai::LlmProviderConfig::from_env(|key| std::env::var(key).ok())?;
    let provider = Arc::new(MeasuredProvider {
        inner: config
            .build_provider_for_env(dl_ai::LlmUseCase::BotPate, |key| std::env::var(key).ok())?,
        calls: Default::default(),
        elapsed_ms: Default::default(),
        show_evidence,
    });
    let game: Arc<dyn Retriever> = Arc::new(dl_answer::game::CliRetriever {
        bin: brain.clone().into(),
    });
    let engine = AnswerEngine::new(
        Some(provider.clone()),
        Arc::new(dl_community::knowledge_client::CommunityRetriever {
            base_url: base.clone(),
            timeout: Duration::from_secs(20),
        }),
        Some(game),
        Duration::from_secs(100),
    )
    .with_persona(dl_community::concierge::ANSWER_PERSONA.to_string());
    let questions: Vec<serde_json::Value> =
        serde_json::from_slice(&tokio::fs::read(questions).await?)?;
    for item in questions {
        let question = item["question"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Frage fehlt"))?;
        let scope = if item["scope"] == "game" {
            Scope::GameOnly
        } else {
            Scope::CommunityAndGame
        };
        provider
            .calls
            .store(0, std::sync::atomic::Ordering::Relaxed);
        provider
            .elapsed_ms
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let start = Instant::now();
        if legacy {
            let result = legacy_answer(&base, &brain, question, scope, provider.as_ref()).await;
            let model_ms = provider
                .elapsed_ms
                .load(std::sync::atomic::Ordering::Relaxed);
            println!(
                "{}",
                json!({"mode":"legacy", "id":item["id"],"question":question,"result":result,"total_ms":start.elapsed().as_millis(),"local_model_ms":model_ms,"local_model_calls":provider.calls.load(std::sync::atomic::Ordering::Relaxed),"upstream_ask_model_calls":"0 oder 1; Wissensdienst-Telemetrie zuordnen"})
            );
            continue;
        }
        let result = engine.answer(question, scope).await;
        let (status, answer, sources) = match result {
            Ok(Answer::Grounded { text, sources, .. }) => ("answered", Some(text), sources),
            Ok(Answer::NoEvidence) => ("no_evidence", None, vec![]),
            Ok(Answer::OutOfDomain) => ("out_of_domain", None, vec![]),
            Err(error) => ("error", Some(error.to_string()), vec![]),
        };
        let total_ms = start.elapsed().as_millis();
        let model_ms = provider
            .elapsed_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        println!(
            "{}",
            json!({"id":item["id"],"question":question,"status":status,"answer":answer,"sources":sources,"total_ms":total_ms,"model_ms":model_ms,"retrieval_and_validation_ms":total_ms.saturating_sub(u128::from(model_ms)),"model_calls":provider.calls.load(std::sync::atomic::Ordering::Relaxed)})
        );
    }
    Ok(())
}

// Eingefrorener vorheriger Antwortstil nur für den expliziten Messmodus.
const BRAIN_DIRECT_ANSWER_OVERRIDE: &str = "---\nWICHTIG — Discord-Antwortstil für normale Fragen:\nBeantworte zuerst die konkrete Frage in 1-2 kurzen Sätzen. Wenn die Frage eine Rechnung enthält, nutze auch Zahlen aus der Nutzerfrage als Annahme und zeige höchstens eine kurze Formel plus Ergebnis. Keine Meta-Abschnitte wie \"Hinweis zur Verifikation\", \"Break-Even-Rechnung\" oder \"laut ground_truth\". Erwähne keine internen Datenquellen, Vertrauensstufen, JSON-Felder oder Faktensammlung. Keine ✅/ℹ️-Labels und keine Quellen-/Vertrauenslegende, außer der Nutzer fragt ausdrücklich danach. Gib keine Build-Tipps, wenn nicht nach Build oder Items gefragt wurde. Wenn etwas unsicher ist, sag es in einem Nebensatz statt als eigenen Abschnitt. Maximal 650 Zeichen, höchstens 4 Stichpunkte.\n---";
const BRAIN_BUILD_OVERRIDE: &str = "---\nWICHTIG — Discord-Antwortstil für Build-Fragen:\nLiefere einen konkreten, spielbaren Build aus den gelieferten Daten. Beginne mit einem kurzen Satz zum Plan, danach early/mid/late mit knappen Stichpunkten. Nenne keine internen Datenquellen, JSON-Felder oder Vertrauensstufen. Keine ✅/ℹ️-Labels und keine Quellen-/Vertrauenslegende. Wenn Daten dünn sind, schreibe vorsichtig, aber ohne Verweigerungsabschnitt. Maximal 900 Zeichen und höchstens 8 Stichpunkte.\n---";

async fn legacy_answer(
    base: &str,
    brain: &str,
    question: &str,
    scope: Scope,
    provider: &MeasuredProvider,
) -> serde_json::Value {
    let outcome = match scope {
        Scope::GameOnly => legacy_brain(brain, question, provider).await,
        Scope::CommunityAndGame => legacy_concierge(base, question, provider).await,
    };
    match outcome {
        Ok(answer) => answer,
        Err(_) => json!({"status":"error"}),
    }
}

async fn legacy_concierge(
    base: &str,
    question: &str,
    provider: &MeasuredProvider,
) -> anyhow::Result<serde_json::Value> {
    use dl_community::concierge::{
        ANTI_INVENT_RULE, GAP_GUIDANCE, PATE_REQUEST_RULE, SYSTEM_PROMPT,
    };
    let url = reqwest::Url::parse(base)?;
    anyhow::ensure!(
        url.host_str().is_some_and(|host| host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())),
        "Nur Loopback"
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()?;
    let start = Instant::now();
    let knowledge: Option<serde_json::Value> = match client
        .post(format!("{}/public/v1/ask", base.trim_end_matches('/')))
        .json(&json!({"question":question}))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response.json().await.ok(),
        _ => None,
    };
    let upstream_ms = start.elapsed().as_millis();
    let context = knowledge
        .as_ref()
        .filter(|value| value["answerable"] == true)
        .and_then(|value| value["answer"].as_str());
    let extra = context
        .map(|text| format!("Wissenskontext aus dl-knowledge:\n{text}"))
        .unwrap_or_else(|| GAP_GUIDANCE.into());
    let schema = format!("{SYSTEM_PROMPT}\n{ANTI_INVENT_RULE}\n{PATE_REQUEST_RULE}\n\nAntworte als JSON: {{\"reply\":\"Text fuer den User\", \"intent\":\"improve|mates|learn|casual\", \"pate_request\":false}}. Das Feld intent muss genau einen der vier Werte haben. reply ist die einzige sichtbare Antwort.\n\n{extra}");
    let answer = tokio::time::timeout(
        Duration::from_secs(100),
        provider.chat(
            &[ChatMessage::system(schema), ChatMessage::user(question)],
            ChatParams {
                json_mode: true,
                temperature: 0.2,
                ..Default::default()
            },
        ),
    )
    .await??;
    Ok(
        json!({"answer":answer.content,"knowledge":knowledge,"upstream_ms":upstream_ms,"upstream_model_calls_if_answered":if context.is_some() {Some(1)} else {None}}),
    )
}

async fn legacy_brain(
    brain: &str,
    question: &str,
    provider: &MeasuredProvider,
) -> anyhow::Result<serde_json::Value> {
    let start = Instant::now();
    let mut command = tokio::process::Command::new(brain);
    command
        .kill_on_drop(true)
        .arg("ask-context")
        .arg("--")
        .arg(question);
    let output = tokio::time::timeout(Duration::from_secs(20), command.output()).await??;
    anyhow::ensure!(output.status.success(), "Brain-Abruf fehlgeschlagen");
    let context: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    if context["intent"] == "out_of_domain" {
        return Ok(json!({"status":"out_of_domain"}));
    }
    let prompt = context["prompt"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Prompt fehlt"))?;
    let lower = prompt.to_ascii_lowercase();
    let style = if lower.contains("erkannte absicht: build_recommendation")
        || lower.contains("build_context_json:")
        || lower.contains("berechneten build")
    {
        BRAIN_BUILD_OVERRIDE
    } else {
        BRAIN_DIRECT_ANSWER_OVERRIDE
    };
    let retrieval_ms = start.elapsed().as_millis();
    let answer = tokio::time::timeout(
        Duration::from_secs(100),
        provider.chat(
            &[ChatMessage::user(format!("{prompt}\n\n{style}"))],
            ChatParams {
                max_tokens: Some(700),
                temperature: 0.25,
                ..Default::default()
            },
        ),
    )
    .await??;
    Ok(json!({"answer":answer.content,"sources":context["sources"],"retrieval_ms":retrieval_ms}))
}
