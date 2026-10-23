use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

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
                "--json-schema",
                schema,
                "--model",
                model,
                "--tools",
                "",
                "--strict-mcp-config",
                "--setting-sources",
                "project",
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
    ) -> Result<Outcome, BackendError> {
        let unavailable = |reason| BackendError::new(FailureKind::Unavailable, reason);
        let schema = serde_json::to_string(&prompt.schema)
            .map_err(|_| BackendError::new(FailureKind::InvalidRequest, "schema"))?;
        let mut input = serde_json::to_vec(&json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": prompt.user}]},
        }))
        .map_err(|_| BackendError::new(FailureKind::InvalidRequest, "content"))?;
        input.push(b'\n');
        let system = PromptFile::write(&self.work_dir, &prompt.system).await?;
        let mut child = self
            .command(model, &system.0, &schema, max_tokens)
            .spawn()
            .map_err(|_| unavailable("spawn_failed"))?;
        let mut stdin = child.stdin.take().ok_or_else(|| unavailable("no_stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| unavailable("no_stdout"))?;
        stdin
            .write_all(&input)
            .await
            .map_err(|_| unavailable("stdin_failed"))?;
        drop(stdin);
        let outcome = last_result(BufReader::new(stdout), self.line_limit, model).await;
        let status = child.wait().await.map_err(|_| unavailable("wait_failed"))?;
        match outcome? {
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
        let prompt = emulation::prompt(request)?;
        let outcome = self.run(&target.model, &prompt, target.max_tokens).await?;
        let (content, stop_reason) =
            emulation::decision(&outcome.structured_output, &request.tools)?;
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
        if let Some(result) = serde_json::from_slice::<Value>(&line)
            .ok()
            .and_then(|value| parse_result(&value, model))
        {
            last = Some(result);
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
        let limited = line.get("api_error_status").and_then(Value::as_u64) == Some(429)
            || line
                .get("result")
                .and_then(Value::as_str)
                .is_some_and(mentions_limit);
        let truncated = line
            .get("result")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("output token maximum"));
        return if limited {
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
        assert!(parse_result(&json!({"type": "assistant"}), MODEL).is_none());
    }

    #[tokio::test]
    async fn fake_cli_answers_through_the_pool_without_inherited_environment() {
        let dir = std::env::temp_dir().join(format!("dl-ai-claude-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let script = dir.join("claude");
        std::fs::write(
            &script,
            "#!/bin/sh\ncat >/dev/null\nif [ -n \"$DL_SECRET_PROBE\" ]; then leak=1; else leak=0; fi\neval last=\\${$#}\nif grep -q sys \"$last\"; then file=1; else file=0; fi\nprintf '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"structured_output\":{\"answer\":\"leak=%s max=%s file=%s\"},\"usage\":{\"input_tokens\":5,\"output_tokens\":2},\"modelUsage\":{\"claude-haiku-5-5\":{}}}\\n' \"$leak\" \"$CLAUDE_CODE_MAX_OUTPUT_TOKENS\" \"$file\"\n",
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
                text: "leak=0 max=833 file=1".into()
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
