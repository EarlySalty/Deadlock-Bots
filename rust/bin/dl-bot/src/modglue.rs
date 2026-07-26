//! Discord-Glue für dl-moderation (ModPort + aimod:*-Review-Buttons).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use dl_ai::TextGenerator;
use dl_community::concierge::CONCIERGE_OWNER_TOPIC_PREFIX;
use dl_discord::{BridgeInteraction, BridgeReply, DiscordAdapter, InteractionHandler};
use serde_json::{json, Map, Value};
use serenity::all::{
    ChannelId, CreateAttachment, GuildId, Http, Message, MessageId, PermissionOverwriteType,
    Permissions, ReactionType, RoleId, UserId,
};
use serenity::builder::GetMessages;
use serenity::http::HttpError;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

const INVITE_CACHE_TTL_SECONDS: i64 = 3600;
const INVITE_CACHE_MAX_ENTRIES: usize = 256;
const DISCORD_FIELD_LIMIT: usize = 1024;
const CASE_IMAGE_ATTACHMENT_EXTENSIONS: &[&str] =
    &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif"];
const MODERATION_EVIDENCE_IMAGE_LIMIT: usize = 4;
const MODERATION_EVIDENCE_IMAGE_MAX_BYTES: usize = 7_000_000;
const MODERATION_EVIDENCE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5);
const DISCORD_MESSAGE_SAFE_LIMIT: usize = 1800;
const AUTO_RAGEBAITER_TAG_SET_BY: u64 = 0;
const BRAIN_SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(20);
const BRAIN_USAGE: &str = "🧠 Frag mich was zu Deadlock! Z. B. `!brain wie spiel ich Vindicta?` oder `!brain ist Lash grad stark?`";
const BRAIN_COOLDOWN: &str = "⏳ Ganz ruhig — eine Brain-Frage alle {secs}s. Gleich gehts wieder.";
const BRAIN_TOO_LONG: &str =
    "Das ist ja ein halber Roman 😅 — pack deine Frage in unter {max} Zeichen.";
#[allow(dead_code)]
const BRAIN_WORKING: &str = "🧠 Moment, ich wühl kurz im Brain…";
const BRAIN_THINKING_FRAMES: [&str; 3] = ["💭 .", "💭 . .", "💭 . . ."];
const BRAIN_THINKING_INTERVAL: Duration = Duration::from_millis(1200);
const BRAIN_THINKING_MAX_TICKS: usize = 40;
const BRAIN_BACKEND_ERR: &str = "🧠 Mein Hirn hakt grad — probier's in ein paar Sekunden nochmal.";
const BRAIN_NO_ANSWER: &str =
    "🧠 Dazu find ich grad nichts Handfestes. Frag mal konkreter — Held, Item oder Fähigkeit.";
const BRAIN_OUT_OF_DOMAIN: &str =
    "🧠 Klingt nicht nach Deadlock — dazu hab ich keine gesicherten Infos. Frag mich was zum Spiel: Held, Item, Build oder Mechanik.";
const BRAIN_EMBED_FOOTER: &str = "Deadlock Brain";
const BRAIN_EMBED_COLOR: u32 = 0xE0A340;
const BRAIN_EMBED_TITLE_QUESTION_LIMIT: usize = 250;
const BRAIN_EMBED_DESCRIPTION_LIMIT: usize = 4096;
const BRAIN_EMBED_DESCRIPTION_TRUNCATE_AT: usize = 4080;
const BRAIN_MAX_OUTPUT_TOKENS: u32 = 700;
const BRAIN_DIRECT_ANSWER_OVERRIDE: &str = "---\nWICHTIG — Discord-Antwortstil für normale Fragen:\nBeantworte zuerst die konkrete Frage in 1-2 kurzen Sätzen. Wenn die Frage eine Rechnung enthält, nutze auch Zahlen aus der Nutzerfrage als Annahme und zeige höchstens eine kurze Formel plus Ergebnis. Keine Meta-Abschnitte wie \"Hinweis zur Verifikation\", \"Break-Even-Rechnung\" oder \"laut ground_truth\". Erwähne keine internen Datenquellen, Vertrauensstufen, JSON-Felder oder Faktensammlung. Keine ✅/ℹ️-Labels und keine Quellen-/Vertrauenslegende, außer der Nutzer fragt ausdrücklich danach. Gib keine Build-Tipps, wenn nicht nach Build oder Items gefragt wurde. Wenn etwas unsicher ist, sag es in einem Nebensatz statt als eigenen Abschnitt. Maximal 650 Zeichen, höchstens 4 Stichpunkte.\n---";
const BRAIN_BUILD_OVERRIDE: &str = "---\nWICHTIG — Discord-Antwortstil für Build-Fragen:\nLiefere einen konkreten, spielbaren Build aus den gelieferten Daten. Beginne mit einem kurzen Satz zum Plan, danach early/mid/late mit knappen Stichpunkten. Nenne keine internen Datenquellen, JSON-Felder oder Vertrauensstufen. Keine ✅/ℹ️-Labels und keine Quellen-/Vertrauenslegende. Wenn Daten dünn sind, schreibe vorsichtig, aber ohne Verweigerungsabschnitt. Maximal 900 Zeichen und höchstens 8 Stichpunkte.\n---";

pub struct LfgFreetextGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn lfg_freetext_question_body(message_id: u64, text: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("content".into(), json!(text));
    body.insert(
        "message_reference".into(),
        json!({ "message_id": message_id.to_string() }),
    );
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    body
}

#[async_trait::async_trait]
impl dl_activity::lfg_freetext::FreetextLfgPort for LfgFreetextGlue {
    async fn ask_start_window(
        &self,
        channel_id: u64,
        message_id: u64,
        text: &'static str,
    ) -> Result<(), String> {
        self.adapter
            .send_raw_public(channel_id, &lfg_freetext_question_body(message_id, text))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn private_channel_overwrites_are_owner_only(
    guild_id: u64,
    owner_id: u64,
    bot_id: u64,
    overwrites: &[serenity::all::PermissionOverwrite],
) -> bool {
    let mut everyone_denied = false;
    let mut owner_allowed = false;
    let mut bot_allowed = false;

    for overwrite in overwrites {
        match overwrite.kind {
            PermissionOverwriteType::Role(role_id) if role_id == RoleId::new(guild_id) => {
                if overwrite.allow.contains(Permissions::VIEW_CHANNEL) {
                    return false;
                }
                everyone_denied |= overwrite.deny.contains(Permissions::VIEW_CHANNEL);
            }
            PermissionOverwriteType::Role(_) => {
                if overwrite.allow.contains(Permissions::VIEW_CHANNEL) {
                    return false;
                }
            }
            PermissionOverwriteType::Member(member_id) => {
                let member_id = member_id.get();
                if member_id != owner_id
                    && member_id != bot_id
                    && overwrite.allow.contains(Permissions::VIEW_CHANNEL)
                {
                    return false;
                }
                let required = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
                if member_id == owner_id {
                    if overwrite.deny.intersects(required) {
                        return false;
                    }
                    owner_allowed |= overwrite.allow.contains(required);
                }
                if member_id == bot_id {
                    if overwrite.deny.intersects(required) {
                        return false;
                    }
                    bot_allowed |= overwrite.allow.contains(required);
                }
            }
            _ => {}
        }
    }

    everyone_denied && owner_allowed && bot_allowed
}

pub struct ModGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub tags: Arc<dl_community::tags::TagService>,
}

pub struct BrainRetrieverGlue {
    pub bin: PathBuf,
}

#[async_trait::async_trait]
impl dl_brain::BrainRetriever for BrainRetrieverGlue {
    async fn ask_context(
        &self,
        frage: &str,
    ) -> Result<dl_brain::BrainContext, dl_brain::BrainError> {
        let output = run_brain_cli(&self.bin, frage, BRAIN_SUBPROCESS_TIMEOUT).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                status = ?output.status.code(),
                stderr = %stderr.trim(),
                "Brain-CLI lieferte Fehlerstatus"
            );
            return Err(dl_brain::BrainError::Backend("exit status".to_string()));
        }
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            tracing::warn!("Brain-CLI lieferte leeres stdout");
            return Err(dl_brain::BrainError::Backend("empty stdout".to_string()));
        }

        let value: Value = serde_json::from_slice(&output.stdout).map_err(|err| {
            tracing::warn!(%err, "Brain-CLI JSON konnte nicht geparst werden");
            dl_brain::BrainError::Backend(err.to_string())
        })?;
        brain_context_from_value(value)
    }
}

async fn run_brain_cli(
    bin: &Path,
    frage: &str,
    timeout_duration: Duration,
) -> Result<Output, dl_brain::BrainError> {
    let mut command = Command::new(bin);
    command.kill_on_drop(true);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // Kein --db mehr: deadlock-brain liest die zentrale Postgres via DEADLOCK_CENTRAL_DSN
    command.arg("ask-context").arg("--").arg(frage);

    let mut child = command.spawn().map_err(|err| {
        tracing::warn!(
            %err,
            bin = %bin.display(),
            "Brain-CLI konnte nicht gestartet werden"
        );
        dl_brain::BrainError::Backend(err.to_string())
    })?;

    let stdout = child.stdout.take().map(read_pipe);
    let stderr = child.stderr.take().map(read_pipe);
    let status = match timeout(timeout_duration, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            tracing::warn!(%err, bin = %bin.display(), "Brain-CLI wait fehlgeschlagen");
            return Err(dl_brain::BrainError::Backend(err.to_string()));
        }
        Err(_) => {
            tracing::warn!(
                bin = %bin.display(),
                timeout_secs = timeout_duration.as_secs(),
                "Brain-CLI Timeout"
            );
            if let Err(err) = child.start_kill() {
                tracing::warn!(%err, bin = %bin.display(), "Brain-CLI Kill fehlgeschlagen");
            }
            if let Err(err) = child.wait().await {
                tracing::warn!(%err, bin = %bin.display(), "Brain-CLI Reap nach Timeout fehlgeschlagen");
            }
            let _ = collect_pipe(stdout).await;
            let _ = collect_pipe(stderr).await;
            return Err(dl_brain::BrainError::Backend("timeout".to_string()));
        }
    };

    let stdout = collect_pipe(stdout).await?;
    let stderr = collect_pipe(stderr).await?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe<R>(mut pipe: R) -> JoinHandle<std::io::Result<Vec<u8>>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buffer = Vec::new();
        pipe.read_to_end(&mut buffer).await?;
        Ok(buffer)
    })
}

async fn collect_pipe(
    task: Option<JoinHandle<std::io::Result<Vec<u8>>>>,
) -> Result<Vec<u8>, dl_brain::BrainError> {
    let Some(task) = task else {
        return Ok(Vec::new());
    };
    match task.await {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(err)) => Err(dl_brain::BrainError::Backend(err.to_string())),
        Err(err) => Err(dl_brain::BrainError::Backend(err.to_string())),
    }
}

fn brain_context_from_value(value: Value) -> Result<dl_brain::BrainContext, dl_brain::BrainError> {
    let intent = value
        .get("intent")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let prompt = value
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if intent.is_empty() {
        tracing::warn!("Brain-CLI JSON ohne intent");
        return Err(dl_brain::BrainError::Backend("missing intent".to_string()));
    }
    if prompt.trim().is_empty() && intent != "out_of_domain" {
        tracing::warn!("Brain-CLI JSON ohne prompt");
        return Err(dl_brain::BrainError::Backend("missing prompt".to_string()));
    }
    let sources = match value.get("sources") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(brain_source_to_string)
            .collect::<Vec<_>>(),
        Some(other) => brain_source_to_string(other).into_iter().collect(),
        None => Vec::new(),
    };
    Ok(dl_brain::BrainContext {
        intent,
        prompt,
        sources,
    })
}

fn brain_source_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        }
        other => serde_json::to_string(other).ok(),
    }
}

pub struct BrainAiGlue {
    pub client: Option<Arc<dl_ai::MiniMaxClient>>,
}

#[async_trait::async_trait]
impl dl_brain::AiAnswerer for BrainAiGlue {
    async fn answer(&self, prompt: &str) -> Result<Option<String>, dl_brain::BrainError> {
        let Some(client) = &self.client else {
            tracing::warn!("Brain-Antwort nicht möglich: MiniMax-Client fehlt");
            return Err(dl_brain::BrainError::Backend(
                "missing minimax client".to_string(),
            ));
        };
        let prompt = brain_ai_prompt(prompt);
        let Some(text) = client
            .generate_text(dl_ai::GenerateRequest {
                prompt,
                system_prompt: None,
                model: None,
                max_output_tokens: Some(BRAIN_MAX_OUTPUT_TOKENS),
                reasoning_effort: None,
                temperature: 0.25,
            })
            .await
        else {
            return Err(dl_brain::BrainError::Backend(
                "missing minimax response".to_string(),
            ));
        };
        let cleaned = dl_ai::strip_think(&text);
        if cleaned.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(cleaned))
        }
    }
}

pub struct BrainHandler {
    pub adapter: Arc<DiscordAdapter>,
    pub config: Arc<dl_brain::BrainConfig>,
    pub cooldowns: Arc<dl_brain::BrainCooldowns>,
    pub retriever: Arc<dyn dl_brain::BrainRetriever>,
    pub answerer: Arc<dyn dl_brain::AiAnswerer>,
    pub channel_allowlist: Option<HashSet<u64>>,
}

impl BrainHandler {
    fn channel_allowed(&self, channel_id: u64) -> bool {
        self.channel_allowlist
            .as_ref()
            .map(|allowlist| allowlist.contains(&channel_id))
            .unwrap_or(false)
    }

    async fn outcome_for_question(&self, question: &str, user_id: u64) -> dl_brain::BrainOutcome {
        dl_brain::handle_brain_query(
            question,
            user_id,
            self.config.as_ref(),
            self.cooldowns.as_ref(),
            self.retriever.as_ref(),
            self.answerer.as_ref(),
        )
        .await
    }

    fn public_bodies_for_outcome(
        &self,
        question: &str,
        outcome: dl_brain::BrainOutcome,
    ) -> Vec<Map<String, Value>> {
        match outcome {
            dl_brain::BrainOutcome::Usage => vec![brain_public_message_body(BRAIN_USAGE)],
            dl_brain::BrainOutcome::TooLong { .. } => vec![brain_public_message_body(
                &BRAIN_TOO_LONG.replace("{max}", &self.config.max_question_len.to_string()),
            )],
            dl_brain::BrainOutcome::Cooldown { remaining_secs } => vec![brain_public_message_body(
                &BRAIN_COOLDOWN.replace("{secs}", &remaining_secs.to_string()),
            )],
            dl_brain::BrainOutcome::Answer(answer) => {
                match brain_answer_embed_body(question, &answer) {
                    Some(body) => vec![body],
                    None => vec![brain_public_message_body(BRAIN_NO_ANSWER)],
                }
            }
            dl_brain::BrainOutcome::OutOfDomain => {
                vec![brain_public_message_body(BRAIN_OUT_OF_DOMAIN)]
            }
            dl_brain::BrainOutcome::NoAnswer => vec![brain_public_message_body(BRAIN_NO_ANSWER)],
            dl_brain::BrainOutcome::BackendError => {
                vec![brain_public_message_body(BRAIN_BACKEND_ERR)]
            }
        }
    }

    fn public_body_for_outcome(
        &self,
        question: &str,
        outcome: dl_brain::BrainOutcome,
    ) -> Map<String, Value> {
        let mut bodies = self.public_bodies_for_outcome(question, outcome);
        bodies
            .pop()
            .unwrap_or_else(|| brain_public_message_body(BRAIN_NO_ANSWER))
    }

    async fn send_public_bodies(&self, channel_id: u64, bodies: &[Map<String, Value>]) {
        for body in bodies {
            if let Err(err) = self.adapter.send_raw_public(channel_id, body).await {
                tracing::warn!(%err, channel_id, "Brain-Antwort konnte nicht gesendet werden");
                break;
            }
        }
    }

    async fn handle_brain_question(&self, channel_id: u64, user_id: u64, question: &str) {
        if question.trim().is_empty() {
            let bodies = vec![brain_public_message_body(BRAIN_USAGE)];
            self.send_public_bodies(channel_id, &bodies).await;
            return;
        }

        let placeholder = brain_public_message_body(thinking_frame(0));
        let message_id = match self.adapter.send_raw_public(channel_id, &placeholder).await {
            Ok(message_id) => message_id,
            Err(err) => {
                tracing::warn!(%err, channel_id, "Brain-Denk-Platzhalter konnte nicht gesendet werden");
                let outcome = self.outcome_for_question(question, user_id).await;
                let bodies = self.public_bodies_for_outcome(question, outcome);
                self.send_public_bodies(channel_id, &bodies).await;
                return;
            }
        };

        let cancelled = Arc::new(AtomicBool::new(false));
        let animation = spawn_brain_thinking_animation(
            self.adapter.clone(),
            channel_id,
            message_id,
            cancelled.clone(),
        );

        let outcome = self.outcome_for_question(question, user_id).await;
        cancelled.store(true, Ordering::SeqCst);
        if let Err(err) = animation.await {
            tracing::warn!(%err, channel_id, message_id, "Brain-Denk-Animation Task fehlgeschlagen");
        }

        let body = self.public_body_for_outcome(question, outcome);
        if let Err(err) = self
            .adapter
            .edit_raw_public(channel_id, message_id, &body)
            .await
        {
            tracing::warn!(%err, channel_id, message_id, "Brain-Antwort konnte nicht editiert werden");
            self.send_public_bodies(channel_id, &[body]).await;
        }
    }

    pub async fn handle_message_event(&self, event: &dl_discord::MessageEvent) {
        if event.guild_id.is_none() {
            return;
        }
        if !self.channel_allowed(event.channel_id) {
            return;
        }
        let Some(question) = parse_brain_question(&event.content) else {
            return;
        };
        self.handle_brain_question(event.channel_id, event.author_id, &question)
            .await;
    }
}

#[async_trait::async_trait]
impl InteractionHandler for BrainHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !self.channel_allowed(interaction.channel_id) {
            return BridgeReply::default();
        }
        let question = parse_brain_question(&interaction.content)
            .unwrap_or_else(|| interaction.content.trim().to_string());
        self.handle_brain_question(interaction.channel_id, interaction.user_id, &question)
            .await;
        BridgeReply::default()
    }
}

fn brain_ai_prompt(prompt: &str) -> String {
    format!("{prompt}\n\n{}", brain_answer_style_override(prompt))
}

fn brain_answer_style_override(prompt: &str) -> &'static str {
    if prompt_is_build_answer(prompt) {
        BRAIN_BUILD_OVERRIDE
    } else {
        BRAIN_DIRECT_ANSWER_OVERRIDE
    }
}

fn prompt_is_build_answer(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("erkannte absicht: build_recommendation")
        || lower.contains("build_context_json:")
        || lower.contains("berechneten build")
}

fn brain_public_message_body(message: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("content".into(), json!(message));
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    body
}

fn brain_answer_embed_body(question: &str, raw_answer: &str) -> Option<Map<String, Value>> {
    let description = truncate_brain_description(&clean_brain_markdown(raw_answer));
    if description.trim().is_empty() {
        return None;
    }

    let embed = json!({
        "title": brain_embed_title(question),
        "description": description,
        "color": BRAIN_EMBED_COLOR,
        "footer": { "text": BRAIN_EMBED_FOOTER },
    });
    let mut body = Map::new();
    body.insert("content".into(), json!(""));
    body.insert("embeds".into(), json!([embed]));
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    Some(body)
}

fn spawn_brain_thinking_animation(
    adapter: Arc<DiscordAdapter>,
    channel_id: u64,
    message_id: u64,
    cancelled: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        for tick in 1..=BRAIN_THINKING_MAX_TICKS {
            sleep(BRAIN_THINKING_INTERVAL).await;
            if cancelled.load(Ordering::SeqCst) {
                break;
            }

            let body = brain_public_message_body(thinking_frame(tick));
            if let Err(err) = adapter.edit_raw_public(channel_id, message_id, &body).await {
                tracing::warn!(%err, channel_id, message_id, "Brain-Denk-Animation konnte nicht editiert werden");
                break;
            }
        }
    })
}

fn thinking_frame(tick: usize) -> &'static str {
    BRAIN_THINKING_FRAMES[tick % BRAIN_THINKING_FRAMES.len()]
}

fn brain_embed_title(question: &str) -> String {
    format!(
        "🧠 {}",
        truncate_brain_chars(question.trim(), BRAIN_EMBED_TITLE_QUESTION_LIMIT, "…")
    )
}

fn truncate_brain_description(description: &str) -> String {
    let description = description.trim();
    if description.chars().count() <= BRAIN_EMBED_DESCRIPTION_LIMIT {
        return description.to_string();
    }

    let mut truncated = description
        .chars()
        .take(BRAIN_EMBED_DESCRIPTION_TRUNCATE_AT)
        .collect::<String>();
    let trimmed_len = truncated.trim_end().len();
    truncated.truncate(trimmed_len);
    truncated.push_str(" …");
    truncated
}

fn truncate_brain_chars(value: &str, max_chars: usize, suffix: &str) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let suffix_len = suffix.chars().count();
    let take_chars = max_chars.saturating_sub(suffix_len);
    let mut truncated = value.chars().take(take_chars).collect::<String>();
    truncated.push_str(suffix);
    truncated
}

fn clean_brain_markdown(input: &str) -> String {
    let mut lines = Vec::new();
    let mut blank_count = 0usize;

    for raw_line in input.lines() {
        let line = raw_line.trim_end();
        let line = brain_heading_as_bold(line).unwrap_or_else(|| line.to_string());
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                lines.push(String::new());
            }
        } else {
            blank_count = 0;
            lines.push(line);
        }
    }

    lines.join("\n").trim().to_string()
}

fn brain_heading_as_bold(line: &str) -> Option<String> {
    let line = line.trim_start();
    let heading = line.strip_prefix("### ")?.trim();
    if heading.is_empty() {
        Some(String::new())
    } else {
        Some(format!("**{heading}**"))
    }
}

fn parse_brain_question(content: &str) -> Option<String> {
    let trimmed = content.trim();
    let rest = trimmed.strip_prefix("!brain")?;
    if !rest.is_empty() {
        let first = rest.chars().next()?;
        if !first.is_whitespace() {
            return None;
        }
    }
    Some(rest.trim().to_string())
}

pub fn parse_brain_channel_allowlist(raw: &str) -> Option<HashSet<u64>> {
    if raw.trim().is_empty() {
        return None;
    }
    raw.split([',', ';', '\n', '\r', '\t', ' '])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().ok().filter(|id| *id > 0))
        .collect::<Option<HashSet<_>>>()
        .filter(|ids| !ids.is_empty())
}

pub fn spawn_brain_command(
    handler: Arc<BrainHandler>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => handler.handle_message_event(&event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Brain-Command: Message-Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[async_trait::async_trait]
impl dl_moderation::ModPort for ModGlue {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn timeout_member(&self, guild_id: u64, user_id: u64, minutes: i64) -> bool {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": until.to_rfc3339() }),
                Some("AI-Moderation: Timeout"),
            )
            .await
            .is_ok()
    }

    async fn ban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .ban_user(
                GuildId::new(guild_id),
                UserId::new(user_id),
                1, // 1 Tag Nachrichten löschen (wie delete_message_days=1)
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn post_review(
        &self,
        case: &dl_moderation::store::CaseDraft,
        case_id: &str,
    ) -> Option<u64> {
        let preview: String = case.content.chars().take(900).collect();
        let mut embed = json!({
            "title": format!("🛡️ Moderationsvorschlag — {}", case.category),
            "description": format!(
                "**User:** <@{}> (`{}`)\n**Kanal:** <#{}>\n**Sicherheit:** {:.0}%\n**Begründung:** {}\n\n**Nachricht:**\n{}",
                case.user_id, case.user_tag, case.channel_id,
                case.confidence * 100.0, case.reason, preview
            ),
            "color": 0xE67E22,
        });
        apply_case_attachment_rendering(&mut embed, &case.attachments);
        let components = json!([{ "type": 1, "components": [
            { "type": 2, "style": 3, "label": "Annehmen (Löschen + Timeout)",
              "custom_id": format!("aimod:accept:{case_id}") },
            { "type": 2, "style": 4, "label": "Ban",
              "custom_id": format!("aimod:ban:{case_id}") },
            { "type": 2, "style": 2, "label": "Ablehnen",
              "custom_id": format!("aimod:deny:{case_id}") },
        ]}]);
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        self.adapter
            .send_raw_public(dl_moderation::MOD_REVIEW_CHANNEL_ID, &body)
            .await
            .ok()
    }

    async fn post_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &body)
            .await;
    }

    async fn post_case_log(
        &self,
        case: &dl_moderation::store::CaseDraft,
        case_id: &str,
        action: &str,
    ) -> Option<u64> {
        let mut embed = build_case_log_embed(case, case_id, action);
        apply_case_attachment_rendering(&mut embed, &case.attachments);
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        let log_message_id = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &body)
            .await
            .ok();

        let original = safe_message_text(&case.content, DISCORD_MESSAGE_SAFE_LIMIT);
        let mut content_message = format!(">>> {original}");
        if case.content.chars().count() > DISCORD_MESSAGE_SAFE_LIMIT {
            content_message.push_str("\nNachricht gekuerzt");
        }
        let mut original_body = serde_json::Map::new();
        original_body.insert("content".into(), json!(content_message));
        original_body.insert("allowed_mentions".into(), json!({ "parse": [] }));
        let _ = self
            .adapter
            .send_raw_public(dl_moderation::LOG_CHANNEL_ID, &original_body)
            .await;
        log_message_id
    }

    async fn send_dm(&self, user_id: u64, text: String) {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return;
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self.adapter.send_raw_public(channel.id.get(), &body).await;
    }

    async fn add_mod_tag(&self, user_id: u64, tag: &str, reason: &str) -> bool {
        self.tags
            .add_mod_tag(
                user_id,
                tag,
                AUTO_RAGEBAITER_TAG_SET_BY,
                Some(reason.to_string()),
                None,
            )
            .await
            .is_ok()
    }

    async fn fetch_context_lines(
        &self,
        guild_id: u64,
        channel_id: u64,
        before_message_id: u64,
        author_id: u64,
        message_created_at: i64,
        limit: usize,
    ) -> Vec<String> {
        let fetch_limit = limit.saturating_add(1).min(100) as u8;
        let Ok(mut messages) = ChannelId::new(channel_id)
            .messages(
                &self.adapter.http,
                GetMessages::new()
                    .before(MessageId::new(before_message_id))
                    .limit(fetch_limit),
            )
            .await
        else {
            return Vec::new();
        };
        messages.sort_by_key(|message| message.timestamp.unix_timestamp());

        let mut lines = Vec::new();
        for previous in messages {
            let mut preview = strip_mentions(&previous.content);
            if preview.is_empty() && !previous.attachments.is_empty() {
                preview = "[Anhang]".to_string();
            }
            if preview.is_empty() {
                continue;
            }

            let delta_s = message_created_at - previous.timestamp.unix_timestamp();
            let mins = (delta_s.max(0)) / 60;
            let time_tag = if mins < 60 {
                format!("[{mins}min ago]")
            } else {
                format!("[{}h ago]", mins / 60)
            };
            let prefix = if previous.author.id.get() == author_id {
                ">>>"
            } else {
                "   "
            };
            let display_name = self.display_name_for_message(guild_id, &previous);
            lines.push(format!(
                "{prefix} {time_tag} {display_name}: {}",
                truncate_chars(&preview, 150)
            ));
        }
        if lines.len() > limit {
            lines.split_off(lines.len() - limit)
        } else {
            lines
        }
    }

    async fn fetch_reply_context(
        &self,
        guild_id: u64,
        channel_id: u64,
        reply_channel_id: Option<u64>,
        reply_message_id: Option<u64>,
    ) -> Option<dl_moderation::ReplyContext> {
        let message_id = reply_message_id?;
        let channel_id = reply_channel_id.unwrap_or(channel_id);
        let message = ChannelId::new(channel_id)
            .message(&self.adapter.http, MessageId::new(message_id))
            .await
            .ok()?;
        let mut content = strip_mentions(&message.content);
        if content.is_empty() {
            content = if message.attachments.is_empty() {
                "[kein Text]".to_string()
            } else {
                "[Anhang]".to_string()
            };
        }
        Some(dl_moderation::ReplyContext {
            author: truncate_chars(&self.display_name_for_message(guild_id, &message), 60),
            content: truncate_chars(&content, 300),
        })
    }
}

#[async_trait::async_trait]
impl dl_moderation::moderation_system::ModerationPort for ModGlue {
    async fn delete_message(&self, channel_id: u64, message_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn mirror_evidence_images(
        &self,
        image_urls: &[String],
    ) -> Vec<dl_moderation::moderation_system::ModerationEvidenceFile> {
        let client = reqwest::Client::new();
        let mut files = Vec::new();
        for url in image_urls.iter().take(MODERATION_EVIDENCE_IMAGE_LIMIT) {
            let Ok(Ok(response)) =
                timeout(MODERATION_EVIDENCE_DOWNLOAD_TIMEOUT, client.get(url).send()).await
            else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_string();
            if response
                .content_length()
                .is_some_and(|size| size > MODERATION_EVIDENCE_IMAGE_MAX_BYTES as u64)
            {
                continue;
            }
            let Ok(Ok(bytes)) =
                timeout(MODERATION_EVIDENCE_DOWNLOAD_TIMEOUT, response.bytes()).await
            else {
                continue;
            };
            if bytes.len() > MODERATION_EVIDENCE_IMAGE_MAX_BYTES {
                continue;
            }
            files.push(dl_moderation::moderation_system::ModerationEvidenceFile {
                filename: moderation_evidence_filename(files.len(), &content_type, url),
                data: bytes.to_vec(),
            });
        }
        files
    }

    async fn timeout_member(
        &self,
        guild_id: u64,
        user_id: u64,
        minutes: i64,
        reason: &str,
    ) -> bool {
        let until = chrono::Utc::now() + chrono::Duration::minutes(minutes);
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": until.to_rfc3339() }),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn ban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .ban_user(
                GuildId::new(guild_id),
                UserId::new(user_id),
                1,
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn untimeout_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .edit_member(
                GuildId::new(guild_id),
                UserId::new(user_id),
                &json!({ "communication_disabled_until": null }),
                Some(reason),
            )
            .await
            .is_ok()
    }

    async fn unban_member(&self, guild_id: u64, user_id: u64, reason: &str) -> bool {
        self.adapter
            .http
            .remove_ban(GuildId::new(guild_id), UserId::new(user_id), Some(reason))
            .await
            .is_ok()
    }

    async fn post_moderation_case(
        &self,
        channel_id: u64,
        embed: Value,
        components: Value,
        files: Vec<dl_moderation::moderation_system::ModerationEvidenceFile>,
    ) -> Option<u64> {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        let attachments = files
            .into_iter()
            .map(|file| CreateAttachment::bytes(file.data, file.filename))
            .collect::<Vec<_>>();
        self.adapter
            .http
            .send_message(ChannelId::new(channel_id), attachments, &body)
            .await
            .ok()
            .map(|message| message.id.get())
    }
}

fn moderation_evidence_filename(index: usize, content_type: &str, url: &str) -> String {
    let ext = if content_type.contains("png") {
        "png"
    } else if content_type.contains("gif") {
        "gif"
    } else if content_type.contains("webp") {
        "webp"
    } else if content_type.contains("avif") {
        "avif"
    } else {
        url.rsplit('?')
            .next()
            .and_then(|clean| clean.rsplit('.').next())
            .filter(|ext| matches!(*ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif"))
            .unwrap_or("jpg")
    };
    format!("moderation-evidence-{}.{}", index + 1, ext)
}

impl ModGlue {
    fn display_name_for_message(&self, guild_id: u64, message: &Message) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|guild| {
                guild
                    .members
                    .get(&message.author.id)
                    .map(|member| member.display_name().to_string())
            })
            .unwrap_or_else(|| message.author.name.to_string())
    }
}

/// Review-Buttons: aimod:accept|ban|deny:{case_id} (Mod-Guard via Rechte).
/// `deny` öffnet ein Modal (Pflicht-Grund) → Submit kommt als
/// `aimod:denysubmit:{case_id}` über dieselbe Prefix-Route zurück.
#[async_trait::async_trait]
trait AimodReviewActions: Send + Sync {
    async fn accept_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
    async fn ban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
    async fn deny_case(
        &self,
        case_id: &str,
        mod_id: u64,
        reason: &str,
    ) -> dl_moderation::ReviewOutcome;
    async fn untimeout_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
    async fn unban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome;
}

#[async_trait::async_trait]
impl AimodReviewActions for dl_moderation::ModerationSystem {
    async fn accept_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.accept_case(case_id, mod_id).await
    }

    async fn ban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.ban_case(case_id, mod_id).await
    }

    async fn deny_case(
        &self,
        case_id: &str,
        mod_id: u64,
        reason: &str,
    ) -> dl_moderation::ReviewOutcome {
        self.deny_case(case_id, mod_id, reason).await
    }

    async fn untimeout_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.untimeout_case(case_id, mod_id).await
    }

    async fn unban_case(&self, case_id: &str, mod_id: u64) -> dl_moderation::ReviewOutcome {
        self.unban_case(case_id, mod_id).await
    }
}

pub struct ReviewHandler {
    moderator: Arc<dyn AimodReviewActions>,
}

impl ReviewHandler {
    pub fn new(moderator: Arc<dl_moderation::ModerationSystem>) -> Self {
        Self { moderator }
    }

    #[cfg(test)]
    fn with_actions(moderator: Arc<dyn AimodReviewActions>) -> Self {
        Self { moderator }
    }

    fn outcome_reply(outcome: dl_moderation::ReviewOutcome) -> BridgeReply {
        use dl_moderation::ReviewOutcome;
        match outcome {
            ReviewOutcome::NotFound => BridgeReply::ephemeral_text("Fall nicht gefunden."),
            ReviewOutcome::AlreadyHandled => {
                BridgeReply::ephemeral_text("Dieser Fall wurde bereits bearbeitet.")
            }
            ReviewOutcome::Done(text) => BridgeReply::ephemeral_text(text),
        }
    }
}

#[async_trait::async_trait]
impl InteractionHandler for ReviewHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let rest = interaction
            .custom_id
            .strip_prefix("aimod:")
            .unwrap_or_default();
        let Some((action, case_id)) = rest.split_once(':') else {
            return BridgeReply::ephemeral_text("Unbekannte Aktion.");
        };
        let authorized = match action {
            "accept" | "deny" | "denysubmit" | "untimeout" => {
                interaction.author_can_moderate_members
            }
            "ban" | "unban" => interaction.author_can_ban_members,
            _ => true,
        };
        if !authorized {
            return BridgeReply::ephemeral_text("Dir fehlt die Berechtigung für diese Aktion.");
        }
        match action {
            "accept" => {
                let outcome = self
                    .moderator
                    .accept_case(case_id, interaction.user_id)
                    .await;
                Self::outcome_reply(outcome)
            }
            "ban" => {
                let outcome = self.moderator.ban_case(case_id, interaction.user_id).await;
                Self::outcome_reply(outcome)
            }
            // Button: Modal mit Pflicht-Grund öffnen (Original: DenyReasonModal,
            // required, min 4 / max 500, mehrzeilig).
            "deny" => BridgeReply {
                modal: Some(dl_discord::ModalSpec {
                    custom_id: format!("aimod:denysubmit:{case_id}"),
                    title: "Fall verwerfen".to_string(),
                    fields: vec![dl_discord::ModalField {
                        custom_id: "reason".to_string(),
                        label: "Grund".to_string(),
                        placeholder: "Warum ist das kein Verstoß?".to_string(),
                        required: true,
                        min_length: 4,
                        max_length: 500,
                        paragraph: true,
                    }],
                }),
                ..BridgeReply::default()
            },
            // Modal-Submit: Grund speichern + Log posten.
            "denysubmit" => {
                let reason = interaction
                    .options
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let outcome = self
                    .moderator
                    .deny_case(case_id, interaction.user_id, &reason)
                    .await;
                Self::outcome_reply(outcome)
            }
            "untimeout" => {
                let outcome = self
                    .moderator
                    .untimeout_case(case_id, interaction.user_id)
                    .await;
                Self::outcome_reply(outcome)
            }
            "unban" => {
                let outcome = self
                    .moderator
                    .unban_case(case_id, interaction.user_id)
                    .await;
                Self::outcome_reply(outcome)
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }
}

// ── Moderation-Hilfen ──────────────────────────────────────────────────────

fn normalize_text(value: &str) -> String {
    value
        .replace(['\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_discord_mention(candidate: &str) -> bool {
    let Some(first) = candidate.chars().next() else {
        return false;
    };
    if first != '@' && first != '#' {
        return false;
    }
    let rest = &candidate[first.len_utf8()..];
    let rest = rest
        .strip_prefix('!')
        .or_else(|| rest.strip_prefix('&'))
        .unwrap_or(rest);
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())
}

fn strip_mentions(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('>') else {
            out.push_str(&rest[open..]);
            return normalize_text(&out);
        };
        let candidate = &after_open[..close];
        if !is_discord_mention(candidate) {
            out.push('<');
            out.push_str(candidate);
            out.push('>');
        }
        rest = &after_open[close + 1..];
    }
    out.push_str(rest);
    normalize_text(&out)
}

fn category_label(category: &str) -> &'static str {
    match category {
        "nsfw_explicit" => "NSFW",
        "csam" => "CSAM",
        "raping" => "Raping",
        "epstein_child" => "Epstein/Child",
        "racism" => "Racism",
        "harassment" => "Harassment",
        "hate_speech" => "Hate Speech",
        "ragebait_ok" => "Ragebait OK",
        "game_related_ok" => "Game Related OK",
        "scam" => "Scam",
        "persistent_ragebait" => "Persistent Ragebait",
        _ => "Other",
    }
}

fn case_jump_url(guild_id: u64, channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
}

fn safe_message_text(value: &str, limit: usize) -> String {
    let text = value.trim();
    if text.is_empty() {
        return "[kein Text]".to_string();
    }
    truncate_chars(text, limit)
}

fn embed_fields_mut(embed: &mut Value) -> Option<&mut Vec<Value>> {
    let object = embed.as_object_mut()?;
    object
        .entry("fields")
        .or_insert_with(|| json!([]))
        .as_array_mut()
}

fn push_embed_field(embed: &mut Value, name: &str, value: String, inline: bool) {
    if value.is_empty() {
        return;
    }
    let Some(fields) = embed_fields_mut(embed) else {
        return;
    };
    fields.push(json!({
        "name": name,
        "value": truncate_chars(&value, DISCORD_FIELD_LIMIT),
        "inline": inline,
    }));
}

fn case_attachment_is_image(attachment: &dl_moderation::store::CaseAttachment) -> bool {
    attachment.content_type.to_lowercase().starts_with("image/")
        || CASE_IMAGE_ATTACHMENT_EXTENSIONS
            .iter()
            .any(|ext| attachment.filename.to_lowercase().ends_with(ext))
}

fn apply_case_attachment_rendering(
    embed: &mut Value,
    attachments: &[dl_moderation::store::CaseAttachment],
) {
    if attachments.is_empty() {
        return;
    }
    let image_urls: Vec<&str> = attachments
        .iter()
        .filter(|attachment| case_attachment_is_image(attachment))
        .map(|attachment| attachment.url.as_str())
        .collect();
    let other_urls: Vec<String> = attachments
        .iter()
        .filter(|attachment| !case_attachment_is_image(attachment))
        .map(|attachment| {
            let filename = if attachment.filename.trim().is_empty() {
                "attachment"
            } else {
                attachment.filename.as_str()
            };
            format!("[{}]({})", truncate_chars(filename, 80), attachment.url)
        })
        .collect();

    if let Some(first_image) = image_urls.first() {
        if let Some(object) = embed.as_object_mut() {
            object.insert("image".into(), json!({ "url": first_image }));
        }
    }
    if image_urls.len() > 1 {
        push_embed_field(embed, "Weitere Bilder", image_urls[1..].join("\n"), false);
    }
    if !other_urls.is_empty() {
        push_embed_field(embed, "Attachments", other_urls.join("\n"), false);
    }
}

fn build_case_log_embed(
    case: &dl_moderation::store::CaseDraft,
    case_id: &str,
    action: &str,
) -> Value {
    let title = match action {
        "auto_delete" => format!(
            "🚨 Auto-Delete: {} ({:.2})",
            category_label(&case.category),
            case.confidence
        ),
        "auto_delete_failed" => format!(
            "⚠️ Auto-Delete fehlgeschlagen: {} ({:.2})",
            category_label(&case.category),
            case.confidence
        ),
        "proposed" => format!(
            "📝 Vorschlag: {} ({:.2})",
            category_label(&case.category),
            case.confidence
        ),
        "ragebait_escalated" => "📝 Ragebait eskaliert".to_string(),
        other => format!("📝 KI-Moderation: {other}"),
    };
    let color = match action {
        "auto_delete" => 0xE74C3C,
        "auto_delete_failed" => 0x992D22,
        "proposed" | "ragebait_escalated" => 0xE67E22,
        _ => 0x5865F2,
    };
    let mut embed = json!({
        "title": title,
        "color": color,
        "fields": [
            { "name": "Author", "value": format!("<@{}> (`{}`)", case.user_id, case.user_tag), "inline": false },
            { "name": "Channel", "value": format!("<#{}> | [Jump]({})", case.channel_id, case_jump_url(case.guild_id, case.channel_id, case.message_id)), "inline": false },
            { "name": "Kategorie", "value": category_label(&case.category), "inline": true },
            { "name": "Confidence", "value": format!("{:.2}", case.confidence), "inline": true },
            { "name": "AI-Reason", "value": truncate_chars(&case.reason, DISCORD_FIELD_LIMIT), "inline": false },
            { "name": "Aktion", "value": action, "inline": true },
            { "name": "Case-ID", "value": case_id, "inline": true }
        ]
    });
    if case.escalated_with_context {
        push_embed_field(
            &mut embed,
            "Detail",
            "context_escalation".to_string(),
            false,
        );
    }
    embed
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

#[derive(Debug, Clone)]
struct InviteCacheEntry {
    guild_id: u64,
    expires_at: i64,
    last_seen_at: i64,
}

struct BehaviorInviteResolver {
    our_guild_id: u64,
    fallback_codes: HashSet<String>,
    allowlist: RwLock<HashSet<String>>,
    cache: Mutex<HashMap<String, InviteCacheEntry>>,
}

impl BehaviorInviteResolver {
    fn new(our_guild_id: u64, fallback_codes: Vec<String>) -> Self {
        let fallback_codes: HashSet<String> = fallback_codes.into_iter().collect();
        Self {
            our_guild_id,
            allowlist: RwLock::new(fallback_codes.clone()),
            fallback_codes,
            cache: Mutex::new(HashMap::new()),
        }
    }

    async fn refresh_allowlist(&self, http: &Http) {
        let mut next = self.fallback_codes.clone();
        match http
            .get_guild_invites(GuildId::new(self.our_guild_id))
            .await
        {
            Ok(invites) => {
                for invite in invites {
                    next.insert(invite.code);
                }
            }
            Err(err) => {
                tracing::warn!(%err, guild_id = self.our_guild_id, "BehaviorDetector: eigene Invites nicht abrufbar");
            }
        }
        match http
            .get_guild_vanity_url(GuildId::new(self.our_guild_id))
            .await
        {
            Ok(code) if !code.trim().is_empty() => {
                next.insert(code.trim().to_string());
            }
            Ok(_) => {}
            Err(err) => {
                tracing::debug!(%err, guild_id = self.our_guild_id, "BehaviorDetector: Vanity-Invite nicht abrufbar");
            }
        }
        *self.allowlist.write().await = next;
    }

    async fn cached_guild_id(&self, code: &str, now: i64) -> Option<u64> {
        let mut cache = self.cache.lock().await;
        prune_invite_cache(&mut cache, now);
        let entry = cache.get_mut(code).filter(|entry| entry.expires_at > now)?;
        entry.last_seen_at = now;
        Some(entry.guild_id)
    }

    async fn remember_resolve_result(&self, code: &str, guild_id: Option<u64>, now: i64) {
        let Some(guild_id) = guild_id else {
            return;
        };
        let mut cache = self.cache.lock().await;
        prune_invite_cache(&mut cache, now);
        cache.insert(
            code.to_string(),
            InviteCacheEntry {
                guild_id,
                expires_at: now + INVITE_CACHE_TTL_SECONDS,
                last_seen_at: now,
            },
        );
        prune_invite_cache(&mut cache, now);
    }

    async fn resolve(&self, http: &Http, code: &str) -> Option<u64> {
        let code = code.trim();
        if code.is_empty() {
            return None;
        }
        if self.allowlist.read().await.contains(code) {
            return Some(self.our_guild_id);
        }

        let now = chrono::Utc::now().timestamp();
        if let Some(guild_id) = self.cached_guild_id(code, now).await {
            return Some(guild_id);
        }

        let guild_id = match http.get_invite(code, false, false, None).await {
            Ok(invite) => invite.guild.map(|guild| guild.id.get()),
            Err(err) => {
                tracing::debug!(%err, %code, "BehaviorDetector: Invite nicht aufloesbar");
                None
            }
        };
        if guild_id == Some(self.our_guild_id) {
            self.allowlist.write().await.insert(code.to_string());
        }
        self.remember_resolve_result(code, guild_id, now).await;
        guild_id
    }
}

fn prune_invite_cache(cache: &mut HashMap<String, InviteCacheEntry>, now: i64) {
    cache.retain(|_, entry| entry.expires_at > now);
    while cache.len() > INVITE_CACHE_MAX_ENTRIES {
        let Some(victim) = cache
            .iter()
            .min_by_key(|(code, entry)| (entry.last_seen_at, entry.expires_at, (*code).clone()))
            .map(|(code, _)| code.clone())
        else {
            break;
        };
        cache.remove(&victim);
    }
}

fn looks_like_invite_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

pub fn parse_invite_allowlist_fallback(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for token in raw.split([',', ';', '\n', '\r', '\t', ' ']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let extracted = dl_moderation::behavior_detector::extract_invite_codes(token);
        if extracted.is_empty() && looks_like_invite_code(token) {
            if seen.insert(token.to_string()) {
                out.push(token.to_string());
            }
            continue;
        }
        for code in extracted {
            if seen.insert(code.clone()) {
                out.push(code);
            }
        }
    }
    out
}

pub struct BehaviorDetectorGlue {
    pub adapter: Arc<DiscordAdapter>,
    invite_resolver: Arc<BehaviorInviteResolver>,
}

impl BehaviorDetectorGlue {
    pub fn new(
        adapter: Arc<DiscordAdapter>,
        our_guild_id: u64,
        fallback_codes: Vec<String>,
    ) -> Self {
        Self {
            adapter,
            invite_resolver: Arc::new(BehaviorInviteResolver::new(our_guild_id, fallback_codes)),
        }
    }

    pub async fn refresh_invite_allowlist(&self) {
        self.invite_resolver
            .refresh_allowlist(&self.adapter.http)
            .await;
    }
}

#[async_trait::async_trait]
impl dl_moderation::behavior_detector::BehaviorDetectorPort for BehaviorDetectorGlue {
    async fn resolve_invite_guild(&self, code: &str) -> Option<u64> {
        self.invite_resolver.resolve(&self.adapter.http, code).await
    }
}

// ── Coaching-Plattform-Anbindung ───────────────────────────────────────────

pub struct CoachingGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub guild_id: u64,
}

fn coach_member_tuple(member: &serenity::all::Member) -> (u64, String, String, String) {
    let avatar = member
        .user
        .avatar_url()
        .unwrap_or_else(|| member.user.default_avatar_url());
    let avatar = if avatar.contains('?') {
        format!("{avatar}&size=256")
    } else {
        format!("{avatar}?size=256")
    };
    (
        member.user.id.get(),
        member.user.name.to_string(),
        member.display_name().to_string(),
        avatar,
    )
}

#[async_trait::async_trait]
impl dl_community::coaching::CoachingPort for CoachingGlue {
    async fn coach_members(&self, role_id: u64) -> Vec<(u64, String, String, String)> {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(self.guild_id)) else {
            return Vec::new();
        };
        let role = serenity::all::RoleId::new(role_id);
        guild
            .members
            .values()
            .filter(|member| member.roles.contains(&role))
            .map(coach_member_tuple)
            .collect()
    }

    async fn coach_members_fetch_fallback(
        &self,
        role_id: u64,
    ) -> Vec<(u64, String, String, String)> {
        let role = serenity::all::RoleId::new(role_id);
        let guild_id = GuildId::new(self.guild_id);
        let mut after = None;
        let mut coaches = Vec::new();
        loop {
            let page = match self
                .adapter
                .http
                .get_guild_members(guild_id, Some(1000), after)
                .await
            {
                Ok(page) => page,
                Err(err) => {
                    tracing::warn!(%err, guild_id = self.guild_id, "Coach-Member-Fetch-Fallback fehlgeschlagen");
                    break;
                }
            };
            if page.is_empty() {
                break;
            }
            after = page.last().map(|member| member.user.id.get());
            coaches.extend(
                page.iter()
                    .filter(|member| member.roles.contains(&role))
                    .map(coach_member_tuple),
            );
            if page.len() < 1000 || after.is_none() {
                break;
            }
        }
        coaches
    }

    async fn send_dm(&self, user_id: u64, text: String) -> Result<bool, String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|e| e.to_string())?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => Ok(true),
            // 50007 = Cannot send messages to this user (DMs zu) → ackbar
            Err(err) if err.to_string().contains("50007") => Ok(false),
            Err(err) => Err(err.to_string()),
        }
    }
}

// ── Website-Invite-Anbindung ───────────────────────────────────────────────

pub struct InviteGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::invites::InvitePort for InviteGlue {
    async fn guild_invite_codes(&self, guild_id: u64) -> Vec<String> {
        self.adapter
            .http
            .get_guild_invites(GuildId::new(guild_id))
            .await
            .map(|invites| invites.into_iter().map(|i| i.code).collect())
            .unwrap_or_default()
    }

    async fn create_permanent_invite(
        &self,
        channel_id: u64,
        reason: &str,
    ) -> Result<String, String> {
        self.adapter
            .http
            .create_invite(
                ChannelId::new(channel_id),
                &json!({ "max_age": 0, "max_uses": 0, "unique": true }),
                Some(reason),
            )
            .await
            .map(|invite| invite.code)
            .map_err(|e| e.to_string())
    }
}

// ── Leave-Survey-Anbindung ─────────────────────────────────────────────────

pub struct SurveyGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn is_discord_cannot_send_messages(err: &serenity::Error) -> bool {
    matches!(
        err,
        serenity::Error::Http(HttpError::UnsuccessfulRequest(resp)) if resp.error.code == 50007
    )
}

fn failed_survey_dm(
    user_id: u64,
    err: serenity::Error,
) -> dl_community::leave_survey::SurveyDmDelivery {
    let error = err.to_string();
    tracing::warn!(user_id, %error, "Leave-Survey-DM fehlgeschlagen");
    dl_community::leave_survey::SurveyDmDelivery::Failed(error)
}

#[async_trait::async_trait]
impl dl_community::leave_survey::SurveyPort for SurveyGlue {
    async fn send_survey_dm(
        &self,
        user_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> dl_community::leave_survey::SurveyDmDelivery {
        use dl_community::leave_survey::SurveyDmDelivery;

        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) if is_discord_cannot_send_messages(&err) => return SurveyDmDelivery::Blocked,
            Err(err) => return failed_survey_dm(user_id, err),
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        match self
            .adapter
            .send_raw_public_typed(channel.id.get(), &body)
            .await
        {
            Ok(_) => SurveyDmDelivery::Sent,
            Err(err) if is_discord_cannot_send_messages(&err) => SurveyDmDelivery::Blocked,
            Err(err) => failed_survey_dm(user_id, err),
        }
    }

    async fn post_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_community::leave_survey::LOGS_CHANNEL_ID, &body)
            .await;
    }

    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .members
            .get(&UserId::new(user_id))
            .map(|m| m.display_name().to_string())
    }
}

// ── Clip-Einsendungen-Anbindung ────────────────────────────────────────────

pub struct ClipGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::clips::ClipPort for ClipGlue {
    async fn upsert_interface(
        &self,
        channel_id: u64,
        existing_message_id: Option<u64>,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        if let Some(message_id) = existing_message_id {
            let edited = self
                .adapter
                .http
                .edit_message(
                    ChannelId::new(channel_id),
                    serenity::all::MessageId::new(message_id),
                    &body,
                    Vec::new(),
                )
                .await;
            match edited {
                Ok(message) => return Ok(message.id.get()),
                Err(err) if err.to_string().contains("10008") => {} // Nachricht weg → neu posten
                Err(err) => return Err(err.to_string()),
            }
        }
        self.adapter
            .send_raw_public(channel_id, &body)
            .await
            .map_err(|e| e.to_string())
    }

    async fn send_dump(
        &self,
        user_id: u64,
        fallback_channel_id: u64,
        caption: String,
        filename: String,
        content: String,
    ) {
        let attachment = serenity::all::CreateAttachment::bytes(content.into_bytes(), filename);
        let dm = async {
            let channel = self
                .adapter
                .http
                .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
                .await
                .map_err(|e| e.to_string())?;
            channel
                .send_files(
                    &self.adapter.http,
                    vec![attachment.clone()],
                    serenity::all::CreateMessage::new().content(caption.clone()),
                )
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(err) = dm {
            tracing::warn!(%err, "Clip-Dump-DM fehlgeschlagen — Fallback in den Submit-Kanal");
            let _ = ChannelId::new(fallback_channel_id)
                .send_files(
                    &self.adapter.http,
                    vec![attachment],
                    serenity::all::CreateMessage::new().content("📦 **Wochen-Dump (Clips)**"),
                )
                .await;
        }
    }

    async fn guild_name(&self, guild_id: u64) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| g.name.to_string())
            .unwrap_or_else(|| guild_id.to_string())
    }
}

// ── FAQ-Chat-Anbindung ─────────────────────────────────────────────────────

/// Gemeinsamer Payload für alle sichtbaren FAQ/Knowledge/Shadow-Nachrichten (rohe Nutzerfrage,
/// Modelltext, Shadow-Ausgabe). Ein Sendepfad, ein Kontrakt.
fn faq_message_body(content: &str, components: Option<serde_json::Value>) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("content".into(), json!(content));
    if let Some(components) = components {
        body.insert("components".into(), components);
    }
    // Rohe Nutzerfrage, Modelltext und Shadow-Ausgabe dürfen niemals Rollen, @everyone oder
    // einzelne User pingen: leeres parse, kein replied_user.
    body.insert(
        "allowed_mentions".into(),
        json!({ "parse": [], "replied_user": false }),
    );
    body
}

pub struct FaqGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::faq::FaqPort for FaqGlue {
    async fn create_faq_channel(
        &self,
        guild_id: u64,
        user_id: u64,
        channel_name: &str,
    ) -> Result<u64, String> {
        let bot_id = self.adapter.cache().current_user().id.get();
        // VIEW=1024, SEND=2048, HISTORY=65536, MANAGE_CHANNELS=16
        let body = json!({
            "name": channel_name,
            "type": 0,
            "parent_id": dl_community::faq::FAQ_CATEGORY_ID.to_string(),
            "permission_overwrites": [
                { "id": guild_id.to_string(), "type": 0, "deny": "1024" },
                { "id": user_id.to_string(), "type": 1, "allow": "68608" },
                { "id": bot_id.to_string(), "type": 1, "allow": "68624" },
            ],
        });
        self.adapter
            .http
            .create_channel(
                GuildId::new(guild_id),
                body.as_object().expect("json object"),
                Some("FAQ Chat"),
            )
            .await
            .map(|c| c.id.get())
            .map_err(|e| e.to_string())
    }

    async fn send_message(
        &self,
        channel_id: u64,
        content: &str,
        components: Option<serde_json::Value>,
    ) -> Result<u64, String> {
        let body = faq_message_body(content, components);
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn channel_category(&self, guild_id: u64, channel_id: u64) -> Option<u64> {
        if guild_id == 0 {
            return None;
        }
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))?
            .channels
            .get(&ChannelId::new(channel_id))
            .and_then(|c| c.parent_id.map(|p| p.get()))
    }

    async fn private_faq_channel_owned_by_user(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
    ) -> bool {
        if guild_id == 0 {
            return false;
        }
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return false;
        };
        let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) else {
            return false;
        };
        let bot_id = self.adapter.cache().current_user().id.get();
        channel.parent_id == Some(ChannelId::new(dl_community::faq::FAQ_CATEGORY_ID))
            && !channel
                .topic
                .as_deref()
                .is_some_and(|topic| topic.starts_with(CONCIERGE_OWNER_TOPIC_PREFIX))
            && private_channel_overwrites_are_owner_only(
                guild_id,
                user_id,
                bot_id,
                &channel.permission_overwrites,
            )
    }

    async fn user_name(&self, user_id: u64) -> String {
        self.adapter
            .cache()
            .user(UserId::new(user_id))
            .map(|u| u.name.to_string())
            .unwrap_or_else(|| format!("user-{user_id}"))
    }

    async fn delete_channel(&self, channel_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_channel(
                ChannelId::new(channel_id),
                Some("FAQ-Transaktion fehlgeschlagen"),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("FAQ-Transaktion fehlgeschlagen"),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn post_rich(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn delete_panel(&self, channel_id: u64, message_id: u64) {
        let _ = self
            .adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("FAQ: Duplikat-Panel aufräumen"),
            )
            .await;
    }
}

// ── DM-Assistent-Anbindung ─────────────────────────────────────────────────

pub struct DmGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::dm_assistant::DmPort for DmGlue {
    async fn send_dm(&self, channel_id: u64, body: serde_json::Map<String, serde_json::Value>) {
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }
}

// ── Voice-DM-Anbindung mit Concierge-Gedächtnis ───────────────────────────

async fn record_concierge_system_dm(
    store: &Option<dl_community::concierge::ConciergeStore>,
    guild_id: u64,
    user_id: u64,
    marker: &str,
) {
    if let Some(store) = store {
        if let Err(err) = store.record_system_dm(user_id, guild_id, marker).await {
            tracing::warn!(%err, user_id, "Concierge: System-DM-Marker konnte nicht gespeichert werden");
        }
    }
}

pub struct VoiceNudgeGlue {
    pub inner: dl_voice::glue::NudgeGlue,
    pub concierge_store: Option<dl_community::concierge::ConciergeStore>,
    pub concierge_guild_id: u64,
}

#[async_trait::async_trait]
impl dl_voice::nudge::NudgePort for VoiceNudgeGlue {
    async fn is_in_voice(&self, guild_id: u64, user_id: u64) -> bool {
        dl_voice::nudge::NudgePort::is_in_voice(&self.inner, guild_id, user_id).await
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        dl_voice::nudge::NudgePort::member_role_ids(&self.inner, guild_id, user_id).await
    }

    async fn send_dm(
        &self,
        user_id: u64,
        embeds: &[serde_json::Value],
        components: &serde_json::Value,
    ) -> Result<(u64, u64), String> {
        let sent =
            dl_voice::nudge::NudgePort::send_dm(&self.inner, user_id, embeds, components).await?;
        record_concierge_system_dm(
            &self.concierge_store,
            self.concierge_guild_id,
            user_id,
            dl_community::concierge::STEAM_NUDGE_MEMORY_MARKER,
        )
        .await;
        Ok(sent)
    }

    async fn send_log(&self, text: String) {
        dl_voice::nudge::NudgePort::send_log(&self.inner, text).await;
    }

    async fn fetch_steam_link_url(&self, user_id: u64) -> Option<String> {
        dl_voice::nudge::NudgePort::fetch_steam_link_url(&self.inner, user_id).await
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) {
        dl_voice::nudge::NudgePort::delete_message(&self.inner, channel_id, message_id).await;
    }

    async fn refresh_dm(
        &self,
        channel_id: u64,
        message_id: u64,
        embeds: &[serde_json::Value],
        components: &serde_json::Value,
    ) -> Result<bool, String> {
        dl_voice::nudge::NudgePort::refresh_dm(
            &self.inner,
            channel_id,
            message_id,
            embeds,
            components,
        )
        .await
    }
}

pub struct VoiceFeedbackGlue {
    pub inner: dl_voice::glue::FeedbackGlue,
    pub concierge_store: Option<dl_community::concierge::ConciergeStore>,
    pub concierge_guild_id: u64,
}

#[async_trait::async_trait]
impl dl_voice::feedback::FeedbackPort for VoiceFeedbackGlue {
    async fn send_feedback_dm(&self, user_id: u64, text: String) -> (String, Option<u64>) {
        let sent =
            dl_voice::feedback::FeedbackPort::send_feedback_dm(&self.inner, user_id, text).await;
        if sent.0 == "sent" {
            record_concierge_system_dm(
                &self.concierge_store,
                self.concierge_guild_id,
                user_id,
                dl_community::concierge::VOICE_FEEDBACK_MEMORY_MARKER,
            )
            .await;
        }
        sent
    }

    async fn forward_to_owner(&self, owner_id: u64, text: String) {
        dl_voice::feedback::FeedbackPort::forward_to_owner(&self.inner, owner_id, text).await;
    }

    async fn delete_feedback_prompt(&self, user_id: u64, message_id: u64) {
        dl_voice::feedback::FeedbackPort::delete_feedback_prompt(&self.inner, user_id, message_id)
            .await;
    }

    async fn display_name(&self, guild_id: u64, user_id: u64) -> Option<String> {
        dl_voice::feedback::FeedbackPort::display_name(&self.inner, guild_id, user_id).await
    }
}

// ── Concierge-Onboarding-Anbindung ─────────────────────────────────────────

pub struct ConciergeGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::concierge::ConciergePort for ConciergeGlue {
    async fn send_dm_v2(
        &self,
        user_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> dl_community::concierge::ConciergeDmDelivery {
        use dl_community::concierge::ConciergeDmDelivery;

        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) if is_discord_cannot_send_messages(&err) => {
                return ConciergeDmDelivery::CannotSend50007;
            }
            Err(err) => return ConciergeDmDelivery::Failed(err.to_string()),
        };
        match self
            .adapter
            .send_raw_public_typed(channel.id.get(), &body)
            .await
        {
            Ok(message_id) => ConciergeDmDelivery::Sent {
                channel_id: channel.id.get(),
                message_id,
            },
            Err(err) if is_discord_cannot_send_messages(&err) => {
                ConciergeDmDelivery::CannotSend50007
            }
            Err(err) => ConciergeDmDelivery::Failed(err.to_string()),
        }
    }

    async fn create_private_channel(
        &self,
        guild_id: u64,
        user_id: u64,
        extra_user_id: Option<u64>,
        category_id: u64,
        name: &str,
    ) -> Result<u64, String> {
        let bot_id = self.adapter.cache().current_user().id.get();
        let mut overwrites = vec![
            json!({ "id": guild_id.to_string(), "type": 0, "deny": "1024" }),
            json!({ "id": user_id.to_string(), "type": 1, "allow": "68608" }),
            json!({ "id": bot_id.to_string(), "type": 1, "allow": "68624" }),
        ];
        if let Some(extra_user_id) = extra_user_id.filter(|id| *id != user_id) {
            overwrites.insert(
                2,
                json!({ "id": extra_user_id.to_string(), "type": 1, "allow": "68608" }),
            );
        }
        let body = json!({
            "name": name,
            "type": 0,
            "parent_id": category_id.to_string(),
            "topic": if extra_user_id.is_none() {
                Some(format!("{CONCIERGE_OWNER_TOPIC_PREFIX}{user_id}"))
            } else {
                None
            },
            "permission_overwrites": overwrites,
        });
        let Some(body) = body.as_object() else {
            return Err("invalid channel body".to_string());
        };
        self.adapter
            .http
            .create_channel(GuildId::new(guild_id), body, Some("Concierge Fallback"))
            .await
            .map(|channel| channel.id.get())
            .map_err(|err| err.to_string())
    }

    async fn role_member_ids(&self, guild_id: u64, role_id: u64) -> Result<Vec<u64>, String> {
        let guild = GuildId::new(guild_id);
        let role = RoleId::new(role_id);
        if let Some(cached) = self.adapter.cache().guild(guild) {
            let ids = cached
                .members
                .values()
                .filter(|member| member.roles.contains(&role))
                .map(|member| member.user.id.get())
                .collect::<Vec<_>>();
            if !ids.is_empty() {
                return Ok(ids);
            }
        }

        let mut after = None;
        let mut ids = Vec::new();
        loop {
            let page = self
                .adapter
                .http
                .get_guild_members(guild, Some(1000), after)
                .await
                .map_err(|err| err.to_string())?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|member| member.user.id.get());
            ids.extend(
                page.into_iter()
                    .filter(|member| member.roles.contains(&role))
                    .map(|member| member.user.id.get()),
            );
        }
        Ok(ids)
    }

    async fn private_channel_owned_by_user(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
        category_id: u64,
    ) -> bool {
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return false;
        };
        let Some(channel) = guild.channels.get(&ChannelId::new(channel_id)) else {
            return false;
        };
        let bot_id = self.adapter.cache().current_user().id.get();
        let expected_topic = format!("{CONCIERGE_OWNER_TOPIC_PREFIX}{user_id}");
        channel.parent_id == Some(ChannelId::new(category_id))
            && channel.topic.as_deref() == Some(expected_topic.as_str())
            && private_channel_overwrites_are_owner_only(
                guild_id,
                user_id,
                bot_id,
                &channel.permission_overwrites,
            )
    }

    async fn send_channel_v2(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn send_channel_text(&self, channel_id: u64, content: &str) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("allowed_mentions".into(), json!({ "parse": [] }));
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn delete_channel(&self, channel_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_channel(
                ChannelId::new(channel_id),
                Some("Concierge-Transaktion fehlgeschlagen"),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn delete_message(&self, channel_id: u64, message_id: u64) -> Result<(), String> {
        self.adapter
            .http
            .delete_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                Some("Concierge-Transaktion fehlgeschlagen"),
            )
            .await
            .map_err(|err| err.to_string())
    }

    async fn add_reaction(&self, channel_id: u64, message_id: u64, emoji: &str) {
        if let Err(err) = self
            .adapter
            .http
            .create_reaction(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &ReactionType::Unicode(emoji.to_string()),
            )
            .await
        {
            tracing::warn!(%err, channel_id, message_id, emoji, "Concierge-Reaktion fehlgeschlagen");
        }
    }

    async fn reply_to_message(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
        allowed_role_id: Option<u64>,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert(
            "message_reference".into(),
            json!({ "channel_id": channel_id.to_string(), "message_id": message_id.to_string() }),
        );
        body.insert(
            "allowed_mentions".into(),
            allowed_role_id.map_or_else(
                || json!({ "parse": [], "replied_user": false }),
                |role_id| {
                    json!({
                        "parse": [],
                        "roles": [role_id.to_string()],
                        "replied_user": false
                    })
                },
            ),
        );
        self.adapter.send_raw_public(channel_id, &body).await
    }
}

// ── Anonymes-Feedback-Anbindung ────────────────────────────────────────────

pub struct FeedbackGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::feedback_hub::FeedbackPort for FeedbackGlue {
    async fn send_dm_text(&self, user_id: u64, text: String) -> Result<(), String> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(|e| e.to_string())?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => Ok(()),
            // 50007 = Cannot send messages to this user (DMs zu) → nicht actionbar
            Err(err) if err.to_string().contains("50007") => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }

    async fn post_rich(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

// ── Reaction-Roles-Anbindung ───────────────────────────────────────────────

pub struct ReactionRoleGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn reaction_role_port_err(err: serenity::Error) -> dl_community::reaction_roles::PortErr {
    dl_community::reaction_roles::PortErr::Discord(err.to_string())
}

fn reaction_role_dm_err(err: serenity::Error) -> dl_community::reaction_roles::DmErr {
    let permanent = matches!(
        &err,
        serenity::Error::Http(HttpError::UnsuccessfulRequest(resp))
            if resp.status_code.as_u16() == 403
                || resp.status_code.as_u16() == 404
                || matches!(resp.error.code, 50007 | 10013)
    );
    let text = err.to_string();
    if permanent {
        dl_community::reaction_roles::DmErr::Permanent(text)
    } else {
        dl_community::reaction_roles::DmErr::Transient(text)
    }
}

#[async_trait::async_trait]
impl dl_community::reaction_roles::ReactionRolePort for ReactionRoleGlue {
    async fn add_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
    ) -> Result<(), dl_community::reaction_roles::PortErr> {
        self.adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some("reaction-role:add"),
            )
            .await
            .map_err(reaction_role_port_err)
    }

    async fn remove_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
    ) -> Result<(), dl_community::reaction_roles::PortErr> {
        self.adapter
            .http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some("reaction-role:remove"),
            )
            .await
            .map_err(reaction_role_port_err)
    }

    async fn send_dm(
        &self,
        user_id: u64,
        content: &str,
    ) -> Result<(), dl_community::reaction_roles::DmErr> {
        let channel = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
            .map_err(reaction_role_dm_err)?;
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        self.adapter
            .http
            .send_message(channel.id, Vec::new(), &body)
            .await
            .map(|_| ())
            .map_err(reaction_role_dm_err)
    }

    async fn reaction_users(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionType,
        after: Option<u64>,
    ) -> Result<Vec<dl_community::reaction_roles::ReactedUser>, dl_community::reaction_roles::PortErr>
    {
        let users = self
            .adapter
            .reaction_users(channel_id, message_id, emoji, after)
            .await
            .map_err(reaction_role_port_err)?;
        Ok(users
            .into_iter()
            .map(|user| dl_community::reaction_roles::ReactedUser {
                id: user.id,
                is_bot: user.is_bot,
            })
            .collect())
    }
}

pub struct ReactionRoleGatewayGlue {
    pub service: Arc<dl_community::reaction_roles::ReactionRoleService>,
}

#[async_trait::async_trait]
impl dl_discord::gateway::ReactionRoleGatewayPort for ReactionRoleGatewayGlue {
    async fn reaction_add(&self, event: dl_discord::gateway::ReactionRoleAddEvent) {
        if let Err(err) = self
            .service
            .handle_reaction_add_with_display_name(
                event.guild_id,
                event.message_id,
                event.user_id,
                event.display_name.as_deref(),
                &event.emoji,
                event.is_bot,
            )
            .await
        {
            tracing::warn!(
                %err,
                guild_id = event.guild_id,
                channel_id = event.channel_id,
                message_id = event.message_id,
                user_id = event.user_id,
                "Reaction-Role-Add fehlgeschlagen"
            );
        }
    }

    async fn reaction_remove(
        &self,
        guild_id: u64,
        channel_id: u64,
        message_id: u64,
        user_id: u64,
        emoji: ReactionType,
        is_bot: bool,
    ) {
        if let Err(err) = self
            .service
            .handle_reaction_remove(guild_id, channel_id, message_id, user_id, &emoji, is_bot)
            .await
        {
            tracing::warn!(
                %err,
                guild_id,
                channel_id,
                message_id,
                user_id,
                "Reaction-Role-Remove fehlgeschlagen"
            );
        }
    }
}

// ── LFG-Lobby-Finder-Anbindung ─────────────────────────────────────────────

pub struct LfgGlue {
    pub adapter: Arc<DiscordAdapter>,
}

const LFG_CATEGORIES: [(u64, &str); 4] = [
    (1289721245281292290, "Casual"),
    (1412804540994162789, "Ranked"),
    (1357422957017698478, "Street Brawl"),
    (1465839366634209361, "New Player"),
];
const LFG_STAGINGS: [u64; 3] = [
    1501089974093873232,
    1412804671432818890,
    1357422958544420944,
];
const JUICE_KAMMER_ID: u64 = 1493690350580138114;
const OFFTOPIC_NAME_SUBSTRING: &str = "off topic voice";
const DISCORD_RANK_ROLES: [(u64, &str, i64); 12] = [
    (1331457571118387210, "Initiate", 1),
    (1331457652877955072, "Seeker", 2),
    (1331457699992436829, "Alchemist", 3),
    (1331457724848017539, "Arcanist", 4),
    (1331457879345070110, "Ritualist", 5),
    (1331457898781474836, "Emissary", 6),
    (1331457949654319114, "Archon", 7),
    (1316966867033653338, "Oracle", 8),
    (1331458016356208680, "Phantom", 9),
    (1331458049637875785, "Ascendant", 10),
    (1331458087349129296, "Eternus", 11),
    (1397687886580547745, "Unbekannt", 0),
];
const UNVERIFIED_RANK_ROLES: [(u64, &str, i64); 11] = [
    (1492959003700101180, "Eternus", 11),
    (1491935935414276198, "Ascendant", 10),
    (1492959474468655134, "Phantom", 9),
    (1492959889767534602, "Oracle", 8),
    (1492959936513052672, "Archon", 7),
    (1492960184920834110, "Emissary", 6),
    (1492960262184239178, "Ritualist", 5),
    (1492960274096066831, "Arcanist", 4),
    (1492960350755225730, "Alchemist", 3),
    (1492959966284218611, "Seeker", 2),
    (1492960891619250408, "Initiate", 1),
];
const RANK_SHORT_NAMES: [(&str, &str); 11] = [
    ("ini", "Initiate"),
    ("see", "Seeker"),
    ("alc", "Alchemist"),
    ("arc", "Arcanist"),
    ("rit", "Ritualist"),
    ("emi", "Emissary"),
    ("arch", "Archon"),
    ("ora", "Oracle"),
    ("pha", "Phantom"),
    ("asc", "Ascendant"),
    ("ete", "Eternus"),
];

fn rank_value_by_name(name: &str) -> Option<(&'static str, i64)> {
    let lower = name.trim().to_lowercase();
    dl_activity::lfg::RANK_NAMES
        .iter()
        .find(|(rank, _)| *rank == lower)
        .map(|(rank, value)| match *rank {
            "initiate" => ("Initiate", *value),
            "seeker" => ("Seeker", *value),
            "alchemist" => ("Alchemist", *value),
            "arcanist" => ("Arcanist", *value),
            "ritualist" => ("Ritualist", *value),
            "emissary" => ("Emissary", *value),
            "archon" => ("Archon", *value),
            "oracle" => ("Oracle", *value),
            "phantom" => ("Phantom", *value),
            "ascendant" => ("Ascendant", *value),
            "eternus" => ("Eternus", *value),
            _ => ("Unbekannt", 0),
        })
}

fn parse_subrank_role_name(role_name: &str) -> Option<(&'static str, i64, i64)> {
    let mut parts = role_name.split_whitespace();
    let rank_raw = parts.next()?;
    let sub = parts
        .next()
        .and_then(|raw| raw.trim_end_matches('+').parse::<i64>().ok())
        .filter(|value| (1..=6).contains(value))?;
    if parts.next().is_some() {
        return None;
    }
    let rank_lower = rank_raw.to_lowercase();
    let rank_name = RANK_SHORT_NAMES
        .iter()
        .find(|(short, _)| *short == rank_lower)
        .map(|(_, full)| *full)
        .unwrap_or(rank_raw);
    let (name, value) = rank_value_by_name(rank_name)?;
    Some((name, value, sub))
}

fn is_lfg_offtopic_channel(name: &str) -> bool {
    name.to_lowercase().contains(OFFTOPIC_NAME_SUBSTRING)
}

fn visible_lfg_member_ids(members: &[(u64, bool)]) -> Vec<u64> {
    members
        .iter()
        .filter_map(|(user_id, is_bot)| (!*is_bot).then_some(*user_id))
        .collect()
}

fn rank_from_roles(roles: &[(u64, String)]) -> (String, i64, Option<i64>) {
    let mut best: (String, i64, Option<i64>, i64) = (String::new(), 0, None, -1);
    for (role_id, name) in roles {
        let mut candidate: Option<(&str, i64, Option<i64>, i64)> = None;
        if let Some((rank_name, value, sub)) = parse_subrank_role_name(name) {
            candidate = Some((rank_name, value, Some(sub), value * 10 + sub));
        }
        if candidate.is_none() {
            if let Some((_, rank_name, value)) =
                DISCORD_RANK_ROLES.iter().find(|(id, _, _)| id == role_id)
            {
                candidate = Some((*rank_name, *value, None, value * 10 + 5));
            }
        }
        if candidate.is_none() {
            if let Some((_, rank_name, value)) = UNVERIFIED_RANK_ROLES
                .iter()
                .find(|(id, _, _)| id == role_id)
            {
                candidate = Some((*rank_name, *value, Some(3), value * 10 + 3));
            }
        }
        if candidate.is_none() {
            let trimmed = name.trim();
            let lower = trimmed.to_lowercase();
            if lower.starts_with("unverifiziert ") {
                let rank_name = trimmed
                    .split_once(char::is_whitespace)
                    .map(|(_, rest)| rest.trim())
                    .unwrap_or_default();
                if let Some((rank_name, value)) = rank_value_by_name(rank_name) {
                    candidate = Some((rank_name, value, Some(3), value * 10 + 3));
                }
            }
        }
        let Some((rank_name, value, sub, score)) = candidate else {
            continue;
        };
        if score > best.3 {
            best = (rank_name.to_string(), value, sub, score);
        }
    }
    if best.1 == 0 {
        ("Unbekannt".to_string(), 0, None)
    } else {
        (best.0, best.1, best.2)
    }
}

#[async_trait::async_trait]
impl dl_activity::lfg::LfgPort for LfgGlue {
    async fn scan_lanes(
        &self,
        guild_id: u64,
        co_player_ids: &[u64],
    ) -> Vec<dl_activity::lfg::LaneInfo> {
        use dl_activity::lfg::{LaneInfo, LaneLabel};
        let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) else {
            return Vec::new();
        };
        let co_set: std::collections::HashSet<u64> = co_player_ids.iter().copied().collect();
        let mut lanes = Vec::new();
        for channel in guild.channels.values() {
            if channel.kind != serenity::all::ChannelType::Voice {
                continue;
            }
            if is_lfg_offtopic_channel(&channel.name) {
                continue;
            }
            let Some((category_id, label)) = channel.parent_id.and_then(|parent| {
                LFG_CATEGORIES
                    .iter()
                    .find(|(id, _)| *id == parent.get())
                    .copied()
            }) else {
                continue;
            };
            let label = match label {
                "Ranked" => LaneLabel::Ranked,
                "Street Brawl" => LaneLabel::StreetBrawl,
                "New Player" => LaneLabel::NewPlayer,
                _ => LaneLabel::Casual,
            };
            let voice_members: Vec<(u64, bool)> = guild
                .voice_states
                .iter()
                .filter(|(_, vs)| vs.channel_id == Some(channel.id))
                .filter_map(|(user_id, _)| {
                    guild
                        .members
                        .get(user_id)
                        .map(|member| (user_id.get(), member.user.bot))
                })
                .collect();
            let member_ids = visible_lfg_member_ids(&voice_members);
            let mut ranks: Vec<i64> = Vec::new();
            let mut co_names: Vec<String> = Vec::new();
            for user_id in &member_ids {
                if let Some(member) = guild.members.get(&UserId::new(*user_id)) {
                    let roles: Vec<(u64, String)> = member
                        .roles
                        .iter()
                        .filter_map(|rid| {
                            guild
                                .roles
                                .get(rid)
                                .map(|r| (rid.get(), r.name.to_string()))
                        })
                        .collect();
                    let (_, value, _) = rank_from_roles(&roles);
                    if value > 0 {
                        ranks.push(value);
                    }
                    if co_set.contains(user_id) {
                        co_names.push(member.display_name().to_string());
                    }
                }
            }
            let member_count = member_ids.len();
            let mut avg = if ranks.is_empty() {
                0.0
            } else {
                ranks.iter().sum::<i64>() as f64 / ranks.len() as f64
            };
            let avg_label = if channel.id.get() == JUICE_KAMMER_ID {
                avg = 11.0;
                "Eternus".to_string()
            } else if avg == 0.0 {
                "Leer".to_string()
            } else {
                let tier = (avg.round() as i64).clamp(1, 11);
                let mut name = dl_activity::lfg::RANK_NAMES
                    .iter()
                    .find(|(_, value)| *value == tier)
                    .map(|(rank, _)| rank.to_string())
                    .unwrap_or_else(|| "Unbekannt".to_string());
                if let Some(head) = name.get_mut(0..1) {
                    head.make_ascii_uppercase();
                }
                name
            };
            let mut limit = match channel.user_limit {
                Some(0) | None => 99,
                Some(value) => value as usize,
            };
            if label == LaneLabel::NewPlayer {
                limit = limit.min(6);
            }
            lanes.push(LaneInfo {
                channel_id: channel.id.get(),
                label,
                member_count,
                user_limit: limit,
                avg_rank_value: avg,
                co_players_present: co_names.len(),
                name: channel.name.to_string(),
                avg_rank_label: avg_label,
                category_id,
                position: channel.position as i64,
                is_staging: LFG_STAGINGS.contains(&channel.id.get()),
                co_player_names: co_names,
            });
        }
        lanes.sort_by_key(|lane| (lane.category_id, lane.position, lane.channel_id));
        lanes
    }

    async fn member_rank(&self, guild_id: u64, user_id: u64) -> (String, i64, Option<i64>) {
        let roles: Vec<(u64, String)> = self
            .adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members.get(&UserId::new(user_id)).map(|m| {
                    m.roles
                        .iter()
                        .filter_map(|rid| g.roles.get(rid).map(|r| (rid.get(), r.name.to_string())))
                        .collect()
                })
            })
            .unwrap_or_default();
        rank_from_roles(&roles)
    }

    async fn member_in_voice(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.voice_states
                    .get(&UserId::new(user_id))
                    .map(|vs| vs.channel_id.is_some())
            })
            .unwrap_or(false)
    }

    async fn post_embed(&self, channel_id: u64, embed: serde_json::Value) {
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn post_text(&self, channel_id: u64, content: &str) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }
}

// ── Coaching-Anfragen-Anbindung ────────────────────────────────────────────

pub struct CoachingReqGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::coaching_requests::CoachingPort for CoachingReqGlue {
    async fn post_panel(
        &self,
        channel_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<u64, String> {
        self.adapter.send_raw_public(channel_id, &body).await
    }

    async fn edit_panel(
        &self,
        channel_id: u64,
        message_id: u64,
        body: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        self.adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn coach_member_ids(&self, guild_id: u64) -> Vec<u64> {
        use dl_community::coaching_requests::{COACH_ROLE_ID, OWNER_EXCLUDE_ID};
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|g| {
                g.members
                    .values()
                    .filter(|m| {
                        !m.user.bot
                            && m.user.id.get() != OWNER_EXCLUDE_ID
                            && m.roles.iter().any(|r| r.get() == COACH_ROLE_ID)
                    })
                    .map(|m| m.user.id.get())
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.roles.iter().map(|r| r.get()).collect())
            })
            .unwrap_or_default()
    }

    async fn member_display_name(&self, guild_id: u64, user_id: u64) -> String {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .and_then(|g| {
                g.members
                    .get(&UserId::new(user_id))
                    .map(|m| m.display_name().to_string())
            })
            .unwrap_or_else(|| format!("User {user_id}"))
    }

    async fn member_is_admin(&self, guild_id: u64, user_id: u64) -> bool {
        let guild_id = GuildId::new(guild_id);
        let Some(guild) = self.adapter.cache().guild(guild_id) else {
            return false;
        };
        if guild.owner_id.get() == user_id {
            return true;
        }
        guild
            .members
            .get(&UserId::new(user_id))
            .map(|m| {
                m.roles.iter().any(|rid| {
                    guild
                        .roles
                        .get(rid)
                        .map(|r| r.permissions.administrator())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    async fn send_request_message(
        &self,
        channel_id: u64,
        content: &str,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> Result<u64, String> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        self.adapter
            .send_raw_public(channel_id, &body)
            .await
            .map_err(|e| e.to_string())
    }

    async fn edit_request_message(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        let _ = self
            .adapter
            .http
            .edit_message(
                ChannelId::new(channel_id),
                serenity::all::MessageId::new(message_id),
                &body,
                Vec::new(),
            )
            .await;
    }

    async fn send_channel_text(&self, channel_id: u64, content: &str) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        body.insert("allowed_mentions".into(), json!({ "parse": ["users"] }));
        let _ = self.adapter.send_raw_public(channel_id, &body).await;
    }

    async fn send_dm(&self, user_id: u64, content: &str) -> bool {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return false;
        };
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(content));
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .is_ok()
    }

    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
        let _ = self
            .adapter
            .http
            .add_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                serenity::all::RoleId::new(role_id),
                Some(reason),
            )
            .await;
    }

    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64, reason: &str) {
        let _ = self
            .adapter
            .http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                serenity::all::RoleId::new(role_id),
                Some(reason),
            )
            .await;
    }

    async fn member_voice_channel_in_category(
        &self,
        guild_id: u64,
        user_id: u64,
        category_id: u64,
    ) -> Option<u64> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let channel_id = guild.voice_states.get(&UserId::new(user_id))?.channel_id?;
        let parent = guild.channels.get(&channel_id)?.parent_id?;
        (parent.get() == category_id).then(|| channel_id.get())
    }

    async fn send_dm_embed(&self, user_id: u64, embed: serde_json::Value) -> bool {
        let Ok(channel) = self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        else {
            return false;
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        self.adapter
            .send_raw_public(channel.id.get(), &body)
            .await
            .is_ok()
    }
}

pub struct ActivityBackfillGlue {
    pub adapter: Arc<DiscordAdapter>,
}

fn unix_to_db_ts(ts: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

#[async_trait::async_trait]
impl dl_activity::analyzer::MemberBackfillPort for ActivityBackfillGlue {
    async fn cache_ready(&self) -> bool {
        self.adapter
            .gateway_ready
            .load(std::sync::atomic::Ordering::Relaxed)
            && !self.adapter.cache().guilds().is_empty()
    }

    async fn current_members(&self) -> Vec<dl_activity::analyzer::BackfillMember> {
        let mut out = Vec::new();
        for guild_id in self.adapter.cache().guilds() {
            let Some(guild) = self.adapter.cache().guild(guild_id) else {
                continue;
            };
            for member in guild.members.values() {
                out.push(dl_activity::analyzer::BackfillMember {
                    guild_id: guild_id.get(),
                    user_id: member.user.id.get(),
                    display_name: member.display_name().to_string(),
                    joined_at: member
                        .joined_at
                        .and_then(|ts| unix_to_db_ts(ts.unix_timestamp())),
                    account_created_at: unix_to_db_ts(member.user.id.created_at().unix_timestamp()),
                    is_bot: member.user.bot,
                });
            }
        }
        out
    }
}

// ── Retention-Miss-You-Anbindung ───────────────────────────────────────────

pub struct RetentionGlue {
    pub adapter: Arc<DiscordAdapter>,
}

#[async_trait::async_trait]
impl dl_community::retention::RetentionPort for RetentionGlue {
    async fn guild_member_count(&self, guild_id: u64) -> Option<usize> {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .map(|guild| guild.members.len())
    }

    async fn is_guild_member(&self, guild_id: u64, user_id: u64) -> bool {
        self.adapter
            .cache()
            .guild(GuildId::new(guild_id))
            .is_some_and(|guild| guild.members.contains_key(&UserId::new(user_id)))
    }

    async fn member_info(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Option<dl_community::retention::RetentionMember> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let member = guild.members.get(&UserId::new(user_id))?;
        Some(dl_community::retention::RetentionMember {
            display_name: member.display_name().to_string(),
            role_ids: member.roles.iter().map(|r| r.get()).collect(),
        })
    }

    async fn fetch_user_name(&self, user_id: u64) -> Option<String> {
        let user = self
            .adapter
            .http
            .get_user(UserId::new(user_id))
            .await
            .ok()?;
        // Discord-Präzedenz: global_name vor Username (wie resolve_user).
        Some(
            user.global_name
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| user.name.to_string()),
        )
    }

    async fn guild_label(&self, guild_id: u64) -> (String, Option<String>) {
        match self.adapter.cache().guild(GuildId::new(guild_id)) {
            Some(g) => (g.name.to_string(), g.icon_url()),
            // Python-Fallback, wenn die Gilde nicht im Cache ist.
            None => ("unserem Server".to_string(), None),
        }
    }

    async fn send_miss_you_dm(
        &self,
        user_id: u64,
        embed: serde_json::Value,
        components: serde_json::Value,
    ) -> dl_community::retention::MissYouDelivery {
        use dl_community::retention::MissYouDelivery;
        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) => return MissYouDelivery::Failed(err.to_string()),
        };
        let mut body = serde_json::Map::new();
        body.insert("embeds".into(), json!([embed]));
        body.insert("components".into(), components);
        match self.adapter.send_raw_public(channel.id.get(), &body).await {
            Ok(_) => MissYouDelivery::Sent,
            // 50007 = Cannot send messages to this user (DMs deaktiviert).
            Err(err)
                if err.contains("50007")
                    && !err.to_ascii_lowercase().contains("no mutual guild") =>
            {
                MissYouDelivery::Blocked
            }
            Err(err) => MissYouDelivery::Failed(err),
        }
    }

    async fn send_log(&self, text: String) {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!(text));
        let _ = self
            .adapter
            .send_raw_public(dl_community::retention::LOG_CHANNEL_ID, &body)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    use dl_brain::BrainRetriever as _;

    #[test]
    fn lfg_freitext_rueckfrage_antwortet_ohne_mentions_auf_die_quellnachricht() {
        let body = lfg_freetext_question_body(555, "PLATZHALTER");

        assert_eq!(body["content"], "PLATZHALTER");
        assert_eq!(body["message_reference"]["message_id"], "555");
        assert_eq!(body["allowed_mentions"]["parse"], json!([]));
        assert_eq!(body["allowed_mentions"]["replied_user"], false);
    }

    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    #[derive(Default)]
    struct FakeReviewActions;

    #[async_trait::async_trait]
    impl AimodReviewActions for FakeReviewActions {
        async fn accept_case(&self, _case_id: &str, _mod_id: u64) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("accepted".to_string())
        }

        async fn ban_case(&self, _case_id: &str, _mod_id: u64) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("banned".to_string())
        }

        async fn deny_case(
            &self,
            _case_id: &str,
            _mod_id: u64,
            _reason: &str,
        ) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("denied".to_string())
        }

        async fn untimeout_case(
            &self,
            _case_id: &str,
            _mod_id: u64,
        ) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("untimeout".to_string())
        }

        async fn unban_case(&self, _case_id: &str, _mod_id: u64) -> dl_moderation::ReviewOutcome {
            dl_moderation::ReviewOutcome::Done("unban".to_string())
        }
    }

    struct CountingBrainRetriever {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl dl_brain::BrainRetriever for CountingBrainRetriever {
        async fn ask_context(
            &self,
            _frage: &str,
        ) -> Result<dl_brain::BrainContext, dl_brain::BrainError> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(dl_brain::BrainContext {
                intent: "answer".to_string(),
                prompt: "prompt".to_string(),
                sources: Vec::new(),
            })
        }
    }

    struct CountingBrainAnswerer {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl dl_brain::AiAnswerer for CountingBrainAnswerer {
        async fn answer(&self, _prompt: &str) -> Result<Option<String>, dl_brain::BrainError> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Some("Antwort".to_string()))
        }
    }

    fn test_brain_handler(
        channel_allowlist: Option<HashSet<u64>>,
        retriever_calls: Arc<AtomicUsize>,
        answerer_calls: Arc<AtomicUsize>,
    ) -> BrainHandler {
        BrainHandler {
            adapter: dl_discord::DiscordAdapter::new("test-token"),
            config: Arc::new(dl_brain::BrainConfig {
                max_question_len: 300,
                cooldown_secs: 20,
            }),
            cooldowns: Arc::new(dl_brain::BrainCooldowns::default()),
            retriever: Arc::new(CountingBrainRetriever {
                calls: retriever_calls,
            }),
            answerer: Arc::new(CountingBrainAnswerer {
                calls: answerer_calls,
            }),
            channel_allowlist,
        }
    }

    fn test_message_event(guild_id: Option<u64>, content: &str) -> dl_discord::MessageEvent {
        dl_discord::MessageEvent {
            guild_id,
            channel_id: 1,
            message_id: 2,
            author_id: 3,
            author_display_name: "user".to_string(),
            author_is_admin: false,
            author_can_manage_messages: false,
            author_can_manage_guild: false,
            author_is_staff: false,
            author_staff_status_known: true,
            content: content.to_string(),
            message_created_at: 0,
            is_reply: false,
            reply_message_id: None,
            reply_channel_id: None,
            attachment_count: 0,
            image_attachment_count: 0,
            image_attachment_urls: Vec::new(),
            attachments: Vec::new(),
            author_created_at: 0,
            author_joined_at: None,
        }
    }

    fn test_review_handler() -> (tempfile::TempDir, ReviewHandler) {
        let dir = tempfile::tempdir().expect("tempdir");
        (
            dir,
            ReviewHandler::with_actions(Arc::new(FakeReviewActions)),
        )
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
    }

    #[test]
    fn clean_brain_markdown_wandelt_headings_und_kollabiert_leerzeilen() {
        let cleaned =
            clean_brain_markdown("# Seven\n\n\n\n## Items\nText\n### Timing\n#### Kein Heading");

        assert_eq!(
            cleaned,
            "# Seven\n\n\n## Items\nText\n**Timing**\n#### Kein Heading"
        );
    }

    #[test]
    fn brain_ai_prompt_nutzt_direktstil_fuer_normale_fragen() {
        let prompt = "FRAGE: Ab wie viel Spirit macht Scourge gleich viel Schaden?\nERKANNTE ABSICHT: mechanic\nFAKTEN (JSON, vertrauenssortiert): {}";

        let styled = brain_ai_prompt(prompt);

        assert!(styled.contains(BRAIN_DIRECT_ANSWER_OVERRIDE));
        assert!(!styled.contains(BRAIN_BUILD_OVERRIDE));
        assert!(styled.contains("Zahlen aus der Nutzerfrage als Annahme"));
        assert!(styled.contains("Keine Meta-Abschnitte"));
    }

    #[test]
    fn brain_ai_prompt_nutzt_buildstil_nur_fuer_buildfragen() {
        let ask_prompt = "FRAGE: Seven build\nERKANNTE ABSICHT: build_recommendation";
        let engine_prompt =
            "Erkläre den folgenden, bereits berechneten Build verständlich auf Deutsch.\n\nBUILD_CONTEXT_JSON:{}";

        let styled_ask = brain_ai_prompt(ask_prompt);
        let styled_engine = brain_ai_prompt(engine_prompt);

        assert!(styled_ask.contains(BRAIN_BUILD_OVERRIDE));
        assert!(styled_engine.contains(BRAIN_BUILD_OVERRIDE));
        assert!(!styled_ask.contains(BRAIN_DIRECT_ANSWER_OVERRIDE));
        assert!(!styled_engine.contains(BRAIN_DIRECT_ANSWER_OVERRIDE));
    }

    #[tokio::test]
    async fn brain_handler_ignoriert_dms() {
        let retriever_calls = Arc::new(AtomicUsize::new(0));
        let handler =
            test_brain_handler(None, retriever_calls.clone(), Arc::new(AtomicUsize::new(0)));

        handler
            .handle_message_event(&test_message_event(None, "!brain Abrams"))
            .await;

        assert_eq!(
            retriever_calls.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn brain_deny_vermeidet_message_und_interaction_io() {
        for (label, allowlist) in [
            ("fehlend", None),
            ("leer", parse_brain_channel_allowlist(" \n\t")),
            ("ungueltig", parse_brain_channel_allowlist("#brain, nope")),
            ("anderer Kanal", parse_brain_channel_allowlist("999")),
        ] {
            let retriever_calls = Arc::new(AtomicUsize::new(0));
            let answerer_calls = Arc::new(AtomicUsize::new(0));
            let handler =
                test_brain_handler(allowlist, retriever_calls.clone(), answerer_calls.clone());
            assert!(!handler.channel_allowed(1), "{label}");

            let message = tokio::time::timeout(
                Duration::from_millis(100),
                handler.handle_message_event(&test_message_event(Some(1), "!brain Abrams")),
            )
            .await;
            assert!(
                message.is_ok(),
                "Message-Deny muss ohne Discord-I/O enden: {label}"
            );

            let interaction = tokio::time::timeout(
                Duration::from_millis(100),
                handler.handle(BridgeInteraction {
                    guild_id: 1,
                    channel_id: 1,
                    user_id: 3,
                    content: "!brain Abrams".to_string(),
                    ..BridgeInteraction::default()
                }),
            )
            .await;
            assert!(
                interaction.is_ok(),
                "Interaction-Deny muss ohne Discord-I/O enden: {label}"
            );
            assert_eq!(
                retriever_calls.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "Retriever: {label}"
            );
            assert_eq!(
                answerer_calls.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "Answerer: {label}"
            );
        }
    }

    #[test]
    fn attachment_rendering_erkennt_bilder_mit_ungenauem_content_type() {
        let mut embed = json!({});
        apply_case_attachment_rendering(
            &mut embed,
            &[
                dl_moderation::store::CaseAttachment {
                    url: "https://cdn.example/scam.PNG".to_string(),
                    content_type: "application/octet-stream".to_string(),
                    filename: "scam.PNG".to_string(),
                },
                dl_moderation::store::CaseAttachment {
                    url: "https://cdn.example/readme.txt".to_string(),
                    content_type: "text/plain".to_string(),
                    filename: "readme.txt".to_string(),
                },
            ],
        );

        assert_eq!(
            embed["image"]["url"].as_str(),
            Some("https://cdn.example/scam.PNG")
        );
        let fields = embed["fields"].as_array().expect("fields");
        let attachment_field = fields
            .iter()
            .find(|field| field["name"].as_str() == Some("Attachments"))
            .expect("attachment field");
        let value = attachment_field["value"].as_str().expect("field value");
        assert!(value.contains("readme.txt"));
        assert!(!value.contains("scam.PNG"));
    }

    #[test]
    fn brain_answer_embed_body_setzt_embed_und_deaktiviert_mentions() {
        let body = brain_answer_embed_body(
            "Wie spiel ich Seven?",
            "### Build\n\n✅ **Seven** startet stabil.",
        )
        .unwrap_or_else(|| panic!("answer should create embed body"));

        assert_eq!(body.get("content"), Some(&json!("")));
        assert_eq!(
            body.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
        let embeds = body
            .get("embeds")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("embeds array missing"));
        assert_eq!(embeds.len(), 1);
        let embed = &embeds[0];
        assert_eq!(embed.get("title"), Some(&json!("🧠 Wie spiel ich Seven?")));
        assert_eq!(
            embed.get("description"),
            Some(&json!("**Build**\n\n✅ **Seven** startet stabil."))
        );
        assert_eq!(embed.get("color"), Some(&json!(BRAIN_EMBED_COLOR)));
        assert_eq!(
            embed.get("footer").and_then(|footer| footer.get("text")),
            Some(&json!(BRAIN_EMBED_FOOTER))
        );
    }

    #[test]
    fn brain_answer_embed_body_erhaelt_stichpunkt_newlines() {
        let body = brain_answer_embed_body("Items?", "- a\n- b\n- c")
            .unwrap_or_else(|| panic!("answer should create embed body"));
        let description = body
            .get("embeds")
            .and_then(Value::as_array)
            .and_then(|embeds| embeds.first())
            .and_then(|embed| embed.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("description missing"));

        assert_eq!(description, "- a\n- b\n- c");
        assert!(!description.contains("- a - b"));
    }

    #[test]
    fn thinking_frame_rotiert_ueber_alle_frames() {
        assert_eq!(thinking_frame(0), BRAIN_THINKING_FRAMES[0]);
        assert_eq!(thinking_frame(1), BRAIN_THINKING_FRAMES[1]);
        assert_eq!(thinking_frame(2), BRAIN_THINKING_FRAMES[2]);
        assert_eq!(thinking_frame(3), BRAIN_THINKING_FRAMES[0]);
        assert_eq!(thinking_frame(4), BRAIN_THINKING_FRAMES[1]);
    }

    #[test]
    fn brain_embed_truncation_bleibt_char_boundary_sicher() {
        let title = brain_embed_title(&"ä".repeat(BRAIN_EMBED_TITLE_QUESTION_LIMIT + 5));
        assert_eq!(
            title.chars().count(),
            "🧠 ".chars().count() + BRAIN_EMBED_TITLE_QUESTION_LIMIT
        );
        assert!(title.ends_with('…'));

        let description =
            truncate_brain_description(&"ä".repeat(BRAIN_EMBED_DESCRIPTION_LIMIT + 1));
        assert_eq!(
            description.chars().count(),
            BRAIN_EMBED_DESCRIPTION_TRUNCATE_AT + " …".chars().count()
        );
        assert!(description.ends_with(" …"));
    }

    #[tokio::test]
    async fn invite_resolver_cacht_nur_definitive_guild_ids() {
        let resolver = BehaviorInviteResolver::new(1, Vec::new());
        let now = 1_000_000;

        resolver.remember_resolve_result("expired", None, now).await;
        assert!(resolver.cache.lock().await.is_empty());
        assert_eq!(resolver.cached_guild_id("expired", now).await, None);

        resolver
            .remember_resolve_result("foreign", Some(2), now)
            .await;
        assert_eq!(resolver.cached_guild_id("foreign", now).await, Some(2));
    }

    #[tokio::test]
    async fn invite_resolver_cache_bleibt_global_begrenzt() {
        let resolver = BehaviorInviteResolver::new(1, Vec::new());
        let now = 1_000_000;

        for idx in 0..300 {
            resolver
                .remember_resolve_result(&format!("code-{idx}"), Some(2), now)
                .await;
        }

        assert!(resolver.cache.lock().await.len() <= 256);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn brain_retriever_uebergibt_separator_vor_dash_frage_ohne_db_flag(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let script = dir.path().join("brain-cli");
        let argv_log = dir.path().join("argv.log");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\n{{\n  printf '%s\\n' \"$#\"\n  for arg in \"$@\"; do printf '%s\\n' \"$arg\"; done\n}} > {}\nprintf '%s\\n' '{{\"intent\":\"general\",\"prompt\":\"ok\",\"sources\":[]}}'\n",
                shell_quote(&argv_log)
            ),
        )?;
        make_executable(&script)?;

        let retriever = BrainRetrieverGlue { bin: script };
        let context = retriever.ask_context("- Spirit Lifesteal?").await?;

        assert_eq!(context.prompt, "ok");
        let argv = fs::read_to_string(argv_log)?;
        let lines = argv.lines().collect::<Vec<_>>();
        assert_eq!(lines, vec!["3", "ask-context", "--", "- Spirit Lifesteal?"]);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn brain_retriever_cancellation_killt_kindprozess(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let script = dir.path().join("slow-brain-cli");
        let pid_file = dir.path().join("pid");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$$\" > {}\nexec sleep 5\n",
                shell_quote(&pid_file)
            ),
        )?;
        make_executable(&script)?;

        let retriever = BrainRetrieverGlue { bin: script };
        let timed_out =
            tokio::time::timeout(Duration::from_millis(200), retriever.ask_context("frage")).await;
        assert!(timed_out.is_err());

        let started = Instant::now();
        while !pid_file.exists() && started.elapsed() < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = fs::read_to_string(&pid_file)?;
        let pid = pid.trim();
        let mut gone = false;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) {
            if std::process::Command::new("kill")
                .arg("-0")
                .arg(pid)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| !status.success())
                .unwrap_or(true)
            {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if !gone {
            let _ = std::process::Command::new("kill")
                .arg(pid)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        assert!(gone);
        Ok(())
    }

    #[test]
    fn brain_allowlist_invalid_config_wird_deny_all() {
        assert!(parse_brain_channel_allowlist(" \n\t").is_none());
        assert!(parse_brain_channel_allowlist("#brain, nope").is_none());
        assert!(parse_brain_channel_allowlist("0").is_none());
        assert!(parse_brain_channel_allowlist("123, nope, 456").is_none());
        assert!(parse_brain_channel_allowlist("123, 0, 456").is_none());

        let parsed = parse_brain_channel_allowlist("123, 456")
            .unwrap_or_else(|| panic!("valid IDs should be kept"));
        assert_eq!(parsed, HashSet::from([123, 456]));
    }

    #[tokio::test]
    async fn aimod_accept_braucht_moderate_members_nicht_manage_roles() {
        let (_dir, handler) = test_review_handler();

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "aimod:accept:test-case".to_string(),
                user_id: 7,
                author_can_manage_roles: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Dir fehlt die Berechtigung für diese Aktion.")
        );
    }

    #[tokio::test]
    async fn aimod_ban_braucht_ban_members_nicht_manage_roles() {
        let (_dir, handler) = test_review_handler();

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "aimod:ban:test-case".to_string(),
                user_id: 7,
                author_can_manage_roles: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Dir fehlt die Berechtigung für diese Aktion.")
        );
    }

    #[tokio::test]
    async fn aimod_unban_braucht_ban_members_nicht_manage_roles() {
        let (_dir, handler) = test_review_handler();

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "aimod:unban:test-case".to_string(),
                user_id: 7,
                author_can_manage_roles: true,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply.content.as_deref(),
            Some("Dir fehlt die Berechtigung für diese Aktion.")
        );
    }

    #[test]
    fn brain_public_message_body_deaktiviert_mentions() {
        let body = brain_public_message_body("@everyone <@123> <@&456>");

        assert_eq!(
            body.get("content"),
            Some(&json!("@everyone <@123> <@&456>"))
        );
        assert_eq!(
            body.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
    }

    #[test]
    fn faq_message_body_deaktiviert_mentions() {
        // Rohe Nutzerfrage, Modelltext und Shadow-Ausgabe laufen alle durch diesen einen
        // Sendepfad: er muss standardmäßig Rollen, @everyone und User-Pings unterbinden.
        let body = faq_message_body("@everyone <@123> <@&456>", None);
        assert_eq!(
            body.get("content"),
            Some(&json!("@everyone <@123> <@&456>"))
        );
        assert_eq!(
            body.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
        assert!(body.get("components").is_none());

        // Mit Components (z. B. Close-Button) bleibt der Mention-Schutz erhalten.
        let with_components = faq_message_body("Antwort", Some(json!([{ "type": 1 }])));
        assert_eq!(
            with_components.get("allowed_mentions"),
            Some(&json!({ "parse": [], "replied_user": false }))
        );
        assert_eq!(
            with_components.get("components"),
            Some(&json!([{ "type": 1 }]))
        );
    }

    fn private_overwrite_fixture() -> Vec<serenity::all::PermissionOverwrite> {
        private_overwrites_for(1, 42, 99)
    }

    fn private_overwrites_for(
        guild_id: u64,
        owner_id: u64,
        bot_id: u64,
    ) -> Vec<serenity::all::PermissionOverwrite> {
        vec![
            serenity::all::PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
                kind: PermissionOverwriteType::Role(RoleId::new(guild_id)),
            },
            serenity::all::PermissionOverwrite {
                allow: Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
                kind: PermissionOverwriteType::Member(UserId::new(owner_id)),
            },
            serenity::all::PermissionOverwrite {
                allow: Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
                kind: PermissionOverwriteType::Member(UserId::new(bot_id)),
            },
        ]
    }

    fn test_adapter_with_channels(
        guild_id: u64,
        channels: Vec<serenity::all::GuildChannel>,
    ) -> Arc<DiscordAdapter> {
        let adapter = DiscordAdapter::new("test-token");
        let cache = Arc::new(serenity::all::Cache::new());
        let mut guild = serenity::all::Guild::default();
        guild.id = GuildId::new(guild_id);
        guild.channels = channels
            .into_iter()
            .map(|channel| (channel.id, channel))
            .collect();
        let mut event: serenity::all::GuildCreateEvent = serde_json::from_value(
            serde_json::to_value(guild).unwrap_or_else(|err| panic!("Guild JSON: {err}")),
        )
        .unwrap_or_else(|err| panic!("GuildCreateEvent JSON: {err}"));
        cache.update(&mut event);
        adapter.link_cache(cache);
        adapter
    }

    fn private_text_channel(
        guild_id: u64,
        channel_id: u64,
        owner_id: u64,
        topic: Option<&str>,
    ) -> serenity::all::GuildChannel {
        let mut channel = serenity::all::GuildChannel::default();
        channel.id = ChannelId::new(channel_id);
        channel.guild_id = GuildId::new(guild_id);
        channel.parent_id = Some(ChannelId::new(dl_community::faq::FAQ_CATEGORY_ID));
        channel.topic = topic.map(str::to_string);
        channel.permission_overwrites = private_overwrites_for(guild_id, owner_id, 1);
        channel
    }

    #[tokio::test]
    async fn faq_channel_category_lehnt_guild_null_ohne_panic_ab() {
        use dl_community::faq::FaqPort as _;

        let faq = FaqGlue {
            adapter: DiscordAdapter::new("test-token"),
        };

        assert_eq!(faq.channel_category(0, 1).await, None);
    }

    #[tokio::test]
    async fn faq_private_channel_lehnt_guild_null_ohne_panic_ab() {
        use dl_community::faq::FaqPort as _;

        let faq = FaqGlue {
            adapter: DiscordAdapter::new("test-token"),
        };

        assert!(!faq.private_faq_channel_owned_by_user(0, 1, 2).await);
    }

    #[tokio::test]
    async fn support_ownership_predicate_cache_matrix_hat_exakt_einen_owner() {
        use dl_community::{concierge::ConciergePort as _, faq::FaqPort as _};

        // Kein Dispatcher-Harness: Diese Matrix kombiniert die echten Cache-Prädikate.
        // Die Brain-Entry-Points deckt der Test `brain_deny_vermeidet_message_und_interaction_io` separat.
        let guild_id = crate::onboardglue::MAIN_GUILD_ID;
        let support_id = dl_community::concierge::SERVER_BOT_FRAGEN_CHANNEL_ID;
        let faq_id = 100;
        let concierge_id = 101;
        let gameplay_id = 102;
        let mut support_channel = serenity::all::GuildChannel::default();
        support_channel.id = ChannelId::new(support_id);
        support_channel.guild_id = GuildId::new(guild_id);
        let mut gameplay_channel = serenity::all::GuildChannel::default();
        gameplay_channel.id = ChannelId::new(gameplay_id);
        gameplay_channel.guild_id = GuildId::new(guild_id);
        let adapter = test_adapter_with_channels(
            guild_id,
            vec![
                support_channel,
                private_text_channel(guild_id, faq_id, 42, None),
                private_text_channel(guild_id, concierge_id, 42, Some("dl-concierge-owner:42")),
                gameplay_channel,
            ],
        );
        let faq = FaqGlue {
            adapter: adapter.clone(),
        };
        let concierge = ConciergeGlue { adapter };
        let retriever_calls = Arc::new(AtomicUsize::new(0));
        let answerer_calls = Arc::new(AtomicUsize::new(0));
        let brain = test_brain_handler(
            Some(HashSet::from([gameplay_id])),
            retriever_calls.clone(),
            answerer_calls.clone(),
        );
        let mut brain_calls = Vec::new();

        for (label, channel_id) in [
            ("Support", support_id),
            ("FAQ", faq_id),
            ("Concierge-Fallback", concierge_id),
            ("Gameplay", gameplay_id),
        ] {
            let brain_owner = brain.channel_allowed(channel_id);
            let owners = [
                channel_id == support_id,
                faq.private_faq_channel_owned_by_user(guild_id, channel_id, 42)
                    .await,
                concierge
                    .private_channel_owned_by_user(
                        guild_id,
                        channel_id,
                        42,
                        dl_community::faq::FAQ_CATEGORY_ID,
                    )
                    .await,
                brain_owner,
            ];
            assert_eq!(
                owners.into_iter().filter(|owned| *owned).count(),
                1,
                "{label}: {owners:?}"
            );
            if brain_owner {
                let _ = brain.outcome_for_question("Abrams?", 42).await;
            }
            brain_calls.push((
                retriever_calls.load(std::sync::atomic::Ordering::Relaxed),
                answerer_calls.load(std::sync::atomic::Ordering::Relaxed),
            ));
        }

        assert_eq!(brain_calls, vec![(0, 0), (0, 0), (0, 0), (1, 1)]);
    }

    #[test]
    fn private_overwrites_erlauben_nur_owner_und_bot() {
        assert!(private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &private_overwrite_fixture(),
        ));
    }

    #[test]
    fn private_overwrites_brauchen_everyone_view_deny() {
        let mut overwrites = private_overwrite_fixture();
        overwrites.remove(0);

        assert!(!private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &overwrites,
        ));
    }

    #[test]
    fn private_overwrites_verbieten_breite_rollensicht() {
        let mut overwrites = private_overwrite_fixture();
        overwrites.push(serenity::all::PermissionOverwrite {
            allow: Permissions::VIEW_CHANNEL,
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Role(RoleId::new(7)),
        });

        assert!(!private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &overwrites,
        ));
    }

    #[test]
    fn private_overwrites_verbieten_fremde_membersicht() {
        let mut overwrites = private_overwrite_fixture();
        overwrites.push(serenity::all::PermissionOverwrite {
            allow: Permissions::VIEW_CHANNEL,
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Member(UserId::new(7)),
        });

        assert!(!private_channel_overwrites_are_owner_only(
            1,
            42,
            99,
            &overwrites,
        ));
    }

    #[test]
    fn lfg_rankrollen_erkennen_ids_subranks_und_unverifiziert() {
        assert_eq!(
            rank_from_roles(&[(1331458016356208680, "irgendein name".to_string())]),
            ("Phantom".to_string(), 9, None)
        );
        assert_eq!(
            rank_from_roles(&[(0, "Asc 3".to_string())]),
            ("Ascendant".to_string(), 10, Some(3))
        );
        assert_eq!(
            rank_from_roles(&[(1492959889767534602, "x".to_string())]),
            ("Oracle".to_string(), 8, Some(3))
        );
        assert_eq!(
            rank_from_roles(&[(0, "Unverifiziert Emissary".to_string())]),
            ("Emissary".to_string(), 6, Some(3))
        );
    }

    #[test]
    fn lfg_offtopic_channels_werden_erkannt() {
        assert!(is_lfg_offtopic_channel("Off Topic Voice 1"));
        assert!(!is_lfg_offtopic_channel("Casual Lane 1"));
    }

    #[test]
    fn lfg_member_count_filtert_bots() {
        let members = visible_lfg_member_ids(&[(1, false), (2, true), (3, false)]);
        assert_eq!(members, vec![1, 3]);
    }
}
