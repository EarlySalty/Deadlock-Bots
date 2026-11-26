use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::backend::{BackendError, CallTarget, FailureKind, ToolBackend};
use super::emulation;
use super::types::{ToolChatRequest, ToolChatResponse, Usage};

const PASSED_ENV: [&str; 5] = ["HOME", "USER", "PATH", "LANG", "XDG_RUNTIME_DIR"];
const ENVELOPE_TOKENS: u32 = 512;

static PROMPT_FILES: AtomicU64 = AtomicU64::new(0);
static CLI_CALLS: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct TurnObservation {
    start_ms: u64,
    first_content_ms: Option<u64>,
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl TurnObservation {
    fn observe_usage(&mut self, usage: &Value) {
        for (name, target) in [
            ("input_tokens", &mut self.input_tokens),
            ("cache_read_input_tokens", &mut self.cache_read_tokens),
            (
                "cache_creation_input_tokens",
                &mut self.cache_creation_tokens,
            ),
            ("output_tokens", &mut self.output_tokens),
        ] {
            if let Some(value) = usage.get(name).and_then(Value::as_u64) {
                *target = Some(value);
            }
        }
    }
}

struct CliObservation {
    id: u64,
    started: Instant,
    turn: Option<TurnObservation>,
    turns: u64,
    assistant_messages: HashSet<String>,
    initialized: bool,
    terminal: bool,
    completed: bool,
}

impl CliObservation {
    fn new() -> Self {
        Self {
            id: CLI_CALLS.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
            turn: None,
            turns: 0,
            assistant_messages: HashSet::new(),
            initialized: false,
            terminal: false,
            completed: false,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn stage(&self, stage: &'static str) {
        tracing::info!(
            cli_call = self.id,
            stage,
            elapsed_ms = self.elapsed_ms(),
            "AI_CLI_STAGE"
        );
    }

    fn finish_turn(&mut self, complete: bool) {
        if let Some(turn) = self.turn.take() {
            tracing::info!(
                cli_call = self.id,
                turn = self.turns,
                complete,
                start_ms = turn.start_ms,
                first_content_ms = ?turn.first_content_ms,
                end_ms = self.elapsed_ms(),
                input_tokens = ?turn.input_tokens,
                cache_read_tokens = ?turn.cache_read_tokens,
                cache_creation_tokens = ?turn.cache_creation_tokens,
                output_tokens = ?turn.output_tokens,
                "AI_CLI_TURN"
            );
        }
    }

    fn observe(&mut self, value: &Value) {
        match value.get("type").and_then(Value::as_str) {
            Some("system") if value.get("subtype").and_then(Value::as_str) == Some("init") => {
                if !self.initialized {
                    self.initialized = true;
                    self.stage("cli_initialized");
                }
            }
            Some("stream_event") => {
                let event = &value["event"];
                let now = self.elapsed_ms();
                match event.get("type").and_then(Value::as_str) {
                    Some("message_start") => {
                        self.finish_turn(false);
                        self.turns = self.turns.saturating_add(1);
                        let mut turn = TurnObservation {
                            start_ms: now,
                            ..TurnObservation::default()
                        };
                        turn.observe_usage(&event["message"]["usage"]);
                        self.turn = Some(turn);
                    }
                    Some("content_block_delta") => {
                        if let Some(turn) = &mut self.turn {
                            turn.first_content_ms.get_or_insert(now);
                        }
                    }
                    Some("message_delta") => {
                        if let Some(turn) = &mut self.turn {
                            turn.observe_usage(&event["usage"]);
                        }
                    }
                    Some("message_stop") => self.finish_turn(true),
                    _ => {}
                }
            }
            Some("assistant") => {
                let message = &value["message"];
                if let Some(id) = message.get("id").and_then(Value::as_str) {
                    if self.assistant_messages.insert(id.to_owned()) {
                        let mut usage = TurnObservation::default();
                        usage.observe_usage(&message["usage"]);
                        tracing::info!(
                            cli_call = self.id,
                            assistant_message = self.assistant_messages.len(),
                            elapsed_ms = self.elapsed_ms(),
                            input_tokens = ?usage.input_tokens,
                            cache_read_tokens = ?usage.cache_read_tokens,
                            cache_creation_tokens = ?usage.cache_creation_tokens,
                            output_tokens = ?usage.output_tokens,
                            "AI_CLI_ASSISTANT"
                        );
                    }
                }
            }
            Some("result") => {
                self.terminal = true;
                self.finish_turn(false);
                tracing::info!(
                    cli_call = self.id,
                    elapsed_ms = self.elapsed_ms(),
                    reported_duration_ms = ?value.get("duration_ms").and_then(serde_json::Value::as_u64),
                    reported_api_ms = ?value.get("duration_api_ms").and_then(serde_json::Value::as_u64),
                    reported_turns = ?value.get("num_turns").and_then(serde_json::Value::as_u64),
                    observed_stream_turns = self.turns,
                    observed_assistant_messages = self.assistant_messages.len(),
                    input_tokens = ?value["usage"].get("input_tokens").and_then(serde_json::Value::as_u64),
                    cache_read_tokens = ?value["usage"].get("cache_read_input_tokens").and_then(serde_json::Value::as_u64),
                    cache_creation_tokens = ?value["usage"].get("cache_creation_input_tokens").and_then(serde_json::Value::as_u64),
                    output_tokens = ?value["usage"].get("output_tokens").and_then(serde_json::Value::as_u64),
                    "AI_CLI_RESULT"
                );
            }
            _ => {}
        }
    }
}

impl Drop for CliObservation {
    fn drop(&mut self) {
        self.finish_turn(false);
        tracing::info!(
            cli_call = self.id,
            elapsed_ms = self.elapsed_ms(),
            initialized = self.initialized,
            terminal = self.terminal,
            completed = self.completed,
            observed_stream_turns = self.turns,
            "AI_CLI_FINISHED"
        );
    }
}

struct PromptFile(PathBuf);

impl PromptFile {
    async fn write(dir: &Path, text: &str) -> Result<Self, BackendError> {
        let failed = |_| BackendError::new(FailureKind::Unavailable, "prompt_file");
        let id = PROMPT_FILES.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("system-{}-{id}.txt", std::process::id()));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .await
            .map_err(failed)?;
        let prompt = Self(path);
        file.write_all(text.as_bytes()).await.map_err(failed)?;
        file.flush().await.map_err(failed)?;
        Ok(prompt)
    }
}

impl Drop for PromptFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct ClaudePlanBackend {
    binary: PathBuf,
    work_dir: PathBuf,
    line_limit: usize,
    env: Vec<(&'static str, OsString)>,
}

struct Outcome {
    model: String,
    structured_output: Value,
    usage: Usage,
    session_id: Option<String>,
}

impl ClaudePlanBackend {
    pub fn new(binary: PathBuf, work_dir: PathBuf, line_limit: usize) -> Self {
        let env = PASSED_ENV
            .iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (*name, value)))
            .collect();
        Self {
            binary,
            work_dir,
            line_limit,
            env,
        }
    }

    fn command(&self, model: &str, system: &Path, schema: &str, max_tokens: u32) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args([
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--json-schema",
                schema,
                "--model",
                model,
                "--tools",
                "",
                "--strict-mcp-config",
                "--setting-sources",
                "",
                "--disable-slash-commands",
                "--no-session-persistence",
                "--system-prompt-file",
            ])
            .arg(system)
            .current_dir(&self.work_dir)
            .env_clear()
            .envs(self.env.iter().map(|(name, value)| (*name, value)))
            .env(
                "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
                max_tokens.saturating_add(ENVELOPE_TOKENS).to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }

    async fn run(
        &self,
        model: &str,
        prompt: &emulation::EmulatedPrompt,
        max_tokens: u32,
        observation: &mut CliObservation,
    ) -> Result<Outcome, BackendError> {
        let unavailable = |reason| BackendError::new(FailureKind::Unavailable, reason);
        let schema = serde_json::to_string(&prompt.schema)
            .map_err(|_| BackendError::new(FailureKind::InvalidRequest, "schema"))?;
        let mut content: Vec<Value> = prompt
            .images
            .iter()
            .map(|source| json!({"type": "image", "source": source}))
            .collect();
        content.push(json!({"type": "text", "text": prompt.user}));
        let mut input = serde_json::to_vec(&json!({
            "type": "user",
            "message": {"role": "user", "content": content},
        }))
        .map_err(|_| BackendError::new(FailureKind::InvalidRequest, "content"))?;
        input.push(b'\n');
        tracing::info!(
            cli_call = observation.id,
            system_bytes = prompt.system.len(),
            transcript_bytes = prompt.user.len(),
            schema_bytes = schema.len(),
            input_bytes = input.len(),
            elapsed_ms = observation.elapsed_ms(),
            "AI_CLI_INPUT"
        );
        let system = PromptFile::write(&self.work_dir, &prompt.system).await?;
        observation.stage("prompt_file_written");
        let mut child = self
            .command(model, &system.0, &schema, max_tokens)
            .spawn()
            .map_err(|_| unavailable("spawn_failed"))?;
        observation.stage("process_spawned");
        let mut stdin = child.stdin.take().ok_or_else(|| unavailable("no_stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| unavailable("no_stdout"))?;
        let call_id = observation.id;
        let started = observation.started;
        let feed = async move {
            let written = stdin.write_all(&input).await;
            drop(stdin);
            tracing::info!(
                cli_call = call_id,
                stage = "stdin_closed",
                elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                success = written.is_ok(),
                "AI_CLI_STAGE"
            );
            written
        };
        let (written, outcome) = tokio::join!(
            feed,
            last_result(BufReader::new(stdout), self.line_limit, model, observation)
        );
        let status = child.wait().await.map_err(|_| unavailable("wait_failed"))?;
        observation.stage("process_exited");
        let outcome = outcome?;
        if written.is_err() && outcome.is_none() {
            return Err(unavailable("stdin_failed"));
        }
        match outcome {
            Some(result) => result,
            None if status.success() => Err(unavailable("no_result")),
            None => Err(unavailable("exit_failure")),
        }
    }
}

#[async_trait]
impl ToolBackend for ClaudePlanBackend {
    async fn complete(
        &self,
        request: &ToolChatRequest,
        target: &CallTarget,
    ) -> Result<ToolChatResponse, BackendError> {
        let mut observation = CliObservation::new();
        let prompt = emulation::prompt(request)?;
        observation.stage("prompt_emulated");
        let outcome = self
            .run(&target.model, &prompt, target.max_tokens, &mut observation)
            .await?;
        let (content, stop_reason) = emulation::decision(
            &outcome.structured_output,
            &request.tools,
            request.output_schema.as_ref(),
        )?;
        observation.stage("decision_validated");
        observation.completed = true;
        Ok(ToolChatResponse {
            model: outcome.model,
            content,
            stop_reason,
            usage: Some(outcome.usage),
            request_id: outcome.session_id,
        })
    }
}

async fn last_result<R: AsyncBufRead + Unpin>(
    mut reader: R,
    limit: usize,
    model: &str,
    observation: &mut CliObservation,
) -> Result<Option<Result<Outcome, BackendError>>, BackendError> {
    let mut last = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = (&mut reader)
            .take(limit as u64 + 1)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| BackendError::new(FailureKind::Unavailable, "stdout_failed"))?;
        if read == 0 {
            return Ok(last);
        }
        if line.len() > limit {
            return Err(BackendError::new(
                FailureKind::InvalidOutput,
                "oversized_output",
            ));
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&line) {
            observation.observe(&value);
            if let Some(result) = parse_result(&value, model) {
                last = Some(result);
            }
        }
    }
}

fn mentions_limit(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["usage limit", "rate limit", "rate_limit", "limit reached"]
        .iter()
        .any(|needle| text.contains(needle))
}

fn served_model(usage: Option<&Map<String, Value>>, requested: &str) -> Option<String> {
    let usage = usage?;
    if usage.contains_key(requested) {
        return Some(requested.to_owned());
    }
    let mut keys = usage.keys();
    match (keys.next(), keys.next()) {
        (Some(only), None) => Some(only.clone()),
        _ => None,
    }
}

fn usage(result: &Value) -> Usage {
    let field = |name: &str| result["usage"][name].as_u64().unwrap_or(0);
    Usage {
        input_tokens: field("input_tokens"),
        output_tokens: field("output_tokens"),
        cache_creation_input_tokens: field("cache_creation_input_tokens"),
        cache_read_input_tokens: field("cache_read_input_tokens"),
    }
}

fn parse_result(line: &Value, requested_model: &str) -> Option<Result<Outcome, BackendError>> {
    if line.get("type").and_then(Value::as_str) != Some("result") {
        return None;
    }
    let failure = |kind, reason| Some(Err(BackendError::new(kind, reason)));
    if line.get("is_error").and_then(Value::as_bool) != Some(false) {
        let status = line.get("api_error_status").and_then(Value::as_u64);
        let overloaded = status == Some(529)
            || line
                .get("result")
                .and_then(Value::as_str)
                .is_some_and(|text| text.to_ascii_lowercase().contains("overloaded"));
        let limited = status == Some(429)
            || line
                .get("result")
                .and_then(Value::as_str)
                .is_some_and(mentions_limit);
        let truncated = line
            .get("result")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("output token maximum"));
        return if overloaded {
            failure(FailureKind::Overloaded, "upstream_overloaded")
        } else if limited {
            failure(FailureKind::Limit, "usage_limit")
        } else if truncated {
            failure(FailureKind::InvalidOutput, "output_token_maximum")
        } else {
            failure(FailureKind::Unavailable, "result_error")
        };
    }
    if line.get("subtype").and_then(Value::as_str) != Some("success") {
        return failure(FailureKind::InvalidOutput, "unexpected_subtype");
    }
    let Some(structured_output) = line.get("structured_output").filter(|v| !v.is_null()) else {
        return failure(FailureKind::InvalidOutput, "missing_structured_output");
    };
    let Some(model) = served_model(
        line.get("modelUsage").and_then(Value::as_object),
        requested_model,
    ) else {
        return failure(FailureKind::InvalidOutput, "unknown_model");
    };
    Some(Ok(Outcome {
        model,
        structured_output: structured_output.clone(),
        usage: usage(line),
        session_id: line
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "claude-haiku-5-5";

    #[test]
    fn stream_observation_tracks_turns_and_unknown_usage() {
        let mut observation = CliObservation::new();
        observation.observe(&json!({"type":"system","subtype":"init"}));
        assert!(observation.initialized);
        observation.observe(&json!({"type":"stream_event","event":{"type":"message_start","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":3}}}}));
        assert_eq!(observation.turns, 1);
        let turn = observation.turn.as_ref().expect("turn");
        assert_eq!(turn.input_tokens, Some(12));
        assert_eq!(turn.cache_read_tokens, Some(3));
        assert_eq!(turn.cache_creation_tokens, None);
        assert_eq!(turn.output_tokens, None);
        observation.observe(&json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"synthetic private text"}}}));
        observation.observe(&json!({"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":7}}}));
        let turn = observation.turn.as_ref().expect("turn");
        assert!(turn.first_content_ms.is_some());
        assert_eq!(turn.output_tokens, Some(7));
        observation.observe(&json!({"type":"stream_event","event":{"type":"message_stop"}}));
        assert!(observation.turn.is_none());
        for _ in 0..2 {
            observation.observe(&json!({"type":"assistant","message":{"id":"synthetic-message","usage":{"input_tokens":12}}}));
        }
        assert_eq!(observation.assistant_messages.len(), 1);
        observation
            .observe(&json!({"type":"stream_event","event":{"type":"message_start","message":{}}}));
        assert_eq!(observation.turns, 2);
        observation.observe(&json!({"type":"result","num_turns":2}));
        assert!(observation.terminal);
        assert!(observation.turn.is_none());
        assert!(!observation.completed);
    }

    #[test]
    fn success_carries_output_model_usage_and_session() {
        let line = json!({
            "type": "result", "subtype": "success", "is_error": false,
            "structured_output": {"answer": "ok"}, "session_id": "abc-1",
            "usage": {"input_tokens": 12, "output_tokens": 7, "cache_creation_input_tokens": 3, "cache_read_input_tokens": 1},
            "modelUsage": {MODEL: {}}
        });
        let outcome = parse_result(&line, MODEL)
            .and_then(Result::ok)
            .expect("success");
        assert_eq!(outcome.model, MODEL);
        assert_eq!(outcome.structured_output, json!({"answer": "ok"}));
        assert_eq!(outcome.usage.total_input(), 16);
        assert_eq!(outcome.session_id.as_deref(), Some("abc-1"));
    }

    #[test]
    fn limits_and_errors_are_typed() {
        for line in [
            json!({"type": "result", "is_error": true, "api_error_status": 429}),
            json!({"type": "result", "is_error": true, "result": "Claude AI usage limit reached|1"}),
        ] {
            assert_eq!(
                parse_result(&line, MODEL)
                    .and_then(Result::err)
                    .map(|e| e.kind),
                Some(FailureKind::Limit)
            );
        }
        assert_eq!(
            parse_result(
                &json!({"type": "result", "is_error": true, "result": "boom"}),
                MODEL
            )
            .and_then(Result::err)
            .map(|e| e.kind),
            Some(FailureKind::Unavailable)
        );
        for line in [
            json!({"type": "result", "is_error": true, "api_error_status": 529}),
            json!({"type": "result", "is_error": true, "result": "API Error: Repeated 529 Overloaded errors"}),
        ] {
            assert_eq!(
                parse_result(&line, MODEL)
                    .and_then(Result::err)
                    .map(|e| e.kind),
                Some(FailureKind::Overloaded)
            );
        }
        assert!(parse_result(&json!({"type": "assistant"}), MODEL).is_none());
    }

    #[tokio::test]
    async fn fake_cli_answers_through_the_pool_without_inherited_environment() {
        let dir = std::env::temp_dir().join(format!("dl-ai-claude-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let script = dir.join("claude");
        std::fs::write(
            &script,
            "#!/bin/sh\ncat >/dev/null\nif [ -n \"$DL_SECRET_PROBE\" ]; then leak=1; else leak=0; fi\neval last=\\${$#}\nif grep -q sys \"$last\"; then file=1; else file=0; fi\nsettings=missing; prev=; penult=\nfor a in \"$@\"; do if [ \"$prev\" = --setting-sources ]; then settings=\"[$a]\"; fi; penult=\"$prev\"; prev=\"$a\"; done\nif [ \"$penult\" = --system-prompt-file ]; then prompt_last=1; else prompt_last=0; fi\nprintf '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"structured_output\":{\"answer\":\"leak=%s max=%s file=%s settings=%s prompt_last=%s\"},\"usage\":{\"input_tokens\":5,\"output_tokens\":2},\"modelUsage\":{\"claude-haiku-5-5\":{}}}\\n' \"$leak\" \"$CLAUDE_CODE_MAX_OUTPUT_TOKENS\" \"$file\" \"$settings\" \"$prompt_last\"\n",
        )
        .expect("script");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        std::env::set_var("DL_SECRET_PROBE", "1");
        let backend = ClaudePlanBackend::new(script, dir.clone(), 1 << 20);
        let request = ToolChatRequest {
            system: "sys".into(),
            messages: vec![super::super::types::Message {
                role: super::super::types::Role::User,
                content: vec![super::super::types::ContentBlock::Text { text: "hi".into() }],
            }],
            tools: Vec::new(),
            max_tokens: None,
            output_schema: None,
        };
        let response = backend
            .complete(
                &request,
                &CallTarget {
                    model: MODEL.into(),
                    max_tokens: 321,
                },
            )
            .await
            .expect("fake cli answer");
        assert_eq!(
            response.content,
            vec![super::super::types::ContentBlock::Text {
                text: "leak=0 max=833 file=1 settings=[] prompt_last=1".into()
            }]
        );
        assert_eq!(response.usage.map(|u| u.input_tokens), Some(5));
        let leftover = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("system-"))
            .count();
        assert_eq!(leftover, 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
