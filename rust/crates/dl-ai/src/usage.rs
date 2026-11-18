use std::sync::Arc;
use std::time::Instant;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::OnceCell;

use crate::tool_chat::FailureKind;
use crate::upstream::{refusal_reason, safe_code, Account};
use crate::{ChatMessage, ChatParams, ChatProvider, ChatProviderError, ChatResponse};

static LEDGER: OnceCell<(PgPool, &'static str)> = OnceCell::const_new();
tokio::task_local! { static PURPOSE: String; }
#[cfg(test)]
tokio::task_local! { static TRACK_LOCAL_FIXTURE: bool; }

pub fn initialize(pool: PgPool, service: &'static str) -> Result<(), &'static str> {
    LEDGER
        .set((pool, service))
        .map_err(|_| "Verbrauchspool bereits gesetzt")?;
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        let mut last_warning = None;
        loop {
            interval.tick().await;
            let replay =
                tokio::time::timeout(std::time::Duration::from_secs(30), replay_pending()).await;
            if !matches!(replay, Ok(Ok(0)))
                && last_warning.is_none_or(|instant: Instant| instant.elapsed().as_secs() >= 345600)
            {
                let unresolved_count = replay.ok().and_then(Result::ok);
                tracing::error!(unresolved_count, "LLM_USAGE_RECOVERY_SCAN_FAILED: Offene Abschlüsse konnten nicht nachgetragen werden");
                last_warning = Some(Instant::now());
            }
        }
    });
    Ok(())
}

pub async fn replay_pending() -> Result<usize, sqlx::Error> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let Some((pool, service)) = LEDGER.get() else {
        return Err(sqlx::Error::PoolClosed);
    };
    let unit = match *service {
        "dl-bot" => "deadlock-bot-rust.service",
        "dl-web" => "deadlock-web-rust.service",
        "dl-brain-feeder" => "dl-brain-feeder.service",
        "dl-knowledge" => "dl-knowledge.service",
        "dl-verbinder" => "dl-verbinder.service",
        "dl-ai-gateway" => "dl-ai-gateway.service",
        _ => return Ok(0),
    };
    let since: Option<String> = sqlx::query_scalar("SELECT min(ts) FROM public.llm_usage WHERE project='Deadlock-Bots' AND service=$1 AND attempt_state='started'")
        .bind(service).fetch_one(pool).await?;
    let Some(since) = since else {
        return Ok(0);
    };
    let mut command = tokio::process::Command::new("/usr/bin/journalctl");
    command.args([
        "--user",
        "-u",
        unit,
        "--since",
        &since,
        "-o",
        "json",
        "--no-pager",
        "--grep",
        "LLM_USAGE_RECOVERY:",
    ]);
    if *service == "dl-verbinder" {
        command.args(["-u", "dl-verbinder-summary.service"]);
    }
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(sqlx::Error::Io)?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| sqlx::Error::Protocol("Journal-Ausgabe fehlt".into()))?;
    let mut lines = BufReader::new(output).lines();
    let mut unresolved_count = 0;
    while let Some(line) = lines.next_line().await.map_err(sqlx::Error::Io)? {
        let Some(completion) = recovery_from_journal(&line) else {
            unresolved_count += 1;
            continue;
        };
        if completion.project != "Deadlock-Bots"
            || completion.service != *service
            || recover(&completion).await.is_err()
        {
            unresolved_count += 1;
        }
    }
    let status = child.wait().await.map_err(sqlx::Error::Io)?;
    if !status.success() {
        return Err(sqlx::Error::Protocol(
            "Journal konnte nicht gelesen werden".into(),
        ));
    }
    let outstanding: i64 = sqlx::query_scalar("SELECT count(*) FROM public.llm_usage WHERE project='Deadlock-Bots' AND service=$1 AND attempt_state='started' AND ts::timestamptz < now() - interval '5 minutes'")
        .bind(service).fetch_one(pool).await?;
    unresolved_count = unresolved_count.max(usize::try_from(outstanding).unwrap_or(usize::MAX));
    Ok(unresolved_count)
}

const RECENT_FAILURES: &str = "WITH settled AS (SELECT provider,attempt_state,error_code,error_reason,http_status,finished_at FROM public.llm_usage WHERE project='Deadlock-Bots' AND ts >= $1 AND attempt_state IN ('succeeded','failed') AND finished_at IS NOT NULL), recovered AS (SELECT provider,max(finished_at) AS at FROM settled WHERE attempt_state='succeeded' GROUP BY provider) SELECT f.provider,f.error_code,f.error_reason,f.http_status,extract(epoch FROM min(f.finished_at))::bigint AS first_at,extract(epoch FROM max(f.finished_at))::bigint AS last_at FROM settled f LEFT JOIN recovered r ON r.provider=f.provider WHERE f.attempt_state='failed' AND (r.at IS NULL OR f.finished_at > r.at) GROUP BY f.provider,f.error_code,f.error_reason,f.http_status ORDER BY last_at DESC LIMIT $2";

type FailureRow = (
    String,
    Option<String>,
    Option<String>,
    Option<i32>,
    i64,
    i64,
);

pub async fn recent_failures(
    lookback_secs: i64,
    limit: i64,
) -> Result<Vec<crate::tool_chat::LedgerFailure>, sqlx::Error> {
    let Some((pool, _)) = LEDGER.get() else {
        return Err(sqlx::Error::PoolClosed);
    };
    let since = (Utc::now() - chrono::Duration::seconds(lookback_secs))
        .to_rfc3339_opts(SecondsFormat::Secs, false);
    let rows: Vec<FailureRow> = sqlx::query_as(RECENT_FAILURES)
        .bind(since)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(provider, error_code, error_reason, http_status, first_at, last_at)| {
                crate::tool_chat::LedgerFailure {
                    provider,
                    error_code,
                    error_reason,
                    http_status,
                    first_at,
                    last_at,
                }
            },
        )
        .collect())
}

fn recovery_from_journal(line: &str) -> Option<Completion> {
    let record: Value = serde_json::from_str(line).ok()?;
    let raw_message = record["MESSAGE"].as_str()?;
    let mut message = String::with_capacity(raw_message.len());
    let mut escape = false;
    for character in raw_message.chars() {
        if character == '\u{1b}' {
            escape = true;
        } else if escape {
            if character == 'm' {
                escape = false;
            }
        } else {
            message.push(character);
        }
    }
    if !message.contains("LLM_USAGE_RECOVERY:") {
        return None;
    }
    let offset = message.find("recovery=")? + "recovery=".len();
    let data = message[offset..].trim_start_matches(|character: char| character != '{');
    serde_json::Deserializer::from_str(data)
        .into_iter::<Completion>()
        .next()?
        .ok()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub project: String,
    pub service: String,
    pub id: i64,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub total: Option<i64>,
    pub request_id: Option<String>,
    pub success: bool,
    pub error_code: Option<String>,
    pub http_status: Option<i32>,
    pub latency_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallOutcome<'a> {
    pub tokens: Option<CallTokens>,
    pub request_id: Option<&'a str>,
    pub error_code: Option<&'a str>,
    pub error_reason: Option<&'a str>,
    pub http_status: Option<u16>,
}

pub struct Attempt {
    completion: Option<Completion>,
    started: Instant,
}

impl Attempt {
    pub async fn begin_before(
        request: &reqwest::Request,
        deadline: Instant,
    ) -> Result<Self, ChatProviderError> {
        tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            Self::begin(request),
        )
        .await
        .map_err(|_| ChatProviderError::Timeout)?
    }

    pub async fn begin(request: &reqwest::Request) -> Result<Self, ChatProviderError> {
        #[cfg(test)]
        if request
            .url()
            .host_str()
            .is_some_and(|host| host == "127.0.0.1" || host == "localhost")
            && !TRACK_LOCAL_FIXTURE
                .try_with(|enabled| *enabled)
                .unwrap_or(false)
        {
            return Ok(Self {
                completion: None,
                started: Instant::now(),
            });
        }
        let model = request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .and_then(|body| serde_json::from_slice::<Value>(body).ok())
            .and_then(|body| body["model"].as_str().map(str::to_owned))
            .ok_or_else(ledger_unavailable)?;
        let purpose = PURPOSE
            .try_with(Clone::clone)
            .unwrap_or_else(|_| "direct_chat".into());
        Self::begin_call("fireworks", &purpose, &model).await
    }

    pub async fn begin_call(
        provider: &str,
        purpose: &str,
        model: &str,
    ) -> Result<Self, ChatProviderError> {
        let (pool, service) = LEDGER.get().ok_or_else(ledger_unavailable)?;
        let id = sqlx::query_scalar("INSERT INTO public.llm_usage (ts,source,purpose,model,tokens_in,tokens_out,total,success,project,service,provider,attempt_state) VALUES ($1,'deadlock-bots',$2,$3,NULL,NULL,NULL,0,'Deadlock-Bots',$4,$5,'started') RETURNING id")
            .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false)).bind(purpose).bind(model).bind(service).bind(provider)
            .fetch_one(pool).await.map_err(|_| {
                tracing::error!("LLM_USAGE_BLOCKED: Aufrufstart konnte nicht gespeichert werden");
                ledger_unavailable()
            })?;
        Ok(Self {
            completion: Some(Completion {
                project: "Deadlock-Bots".into(),
                service: (*service).into(),
                id,
                tokens_in: None,
                tokens_out: None,
                total: None,
                request_id: None,
                success: false,
                error_code: Some("cancelled".into()),
                http_status: None,
                latency_ms: 0,
                error_reason: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            started: Instant::now(),
        })
    }

    pub fn response(&mut self, response: &reqwest::Response) {
        if let Some(completion) = &mut self.completion {
            completion.http_status = Some(i32::from(response.status().as_u16()));
            completion.request_id = response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .filter(|value| safe_id(value))
                .map(str::to_owned);
        }
    }

    pub async fn finish(mut self, body: Option<&Value>, error_code: Option<&str>) {
        if let Some(completion) = &mut self.completion {
            completion.latency_ms =
                i64::try_from(self.started.elapsed().as_millis()).unwrap_or(i64::MAX);
            completion.success = error_code.is_none();
            completion.error_code = error_code.map(str::to_owned);
            if let Some(body) = body {
                completion.tokens_in = count(body, "prompt_tokens");
                completion.tokens_out = count(body, "completion_tokens");
                completion.total = count(body, "total_tokens");
                completion.cache_read_tokens =
                    cached_tokens(body).and_then(|value| i64::try_from(value).ok());
                if completion.request_id.is_none() {
                    completion.request_id = body["id"]
                        .as_str()
                        .filter(|value| safe_id(value))
                        .map(str::to_owned);
                }
            }
            recovery_log(completion);
            let _ = recover(completion).await;
            self.completion = None;
        }
    }

    pub async fn finish_call(mut self, outcome: CallOutcome<'_>) {
        if let Some(completion) = &mut self.completion {
            completion.latency_ms =
                i64::try_from(self.started.elapsed().as_millis()).unwrap_or(i64::MAX);
            completion.success = outcome.error_code.is_none();
            completion.error_code = outcome.error_code.map(str::to_owned);
            completion.error_reason = outcome
                .error_reason
                .filter(|reason| safe_code(reason))
                .map(str::to_owned);
            completion.http_status = outcome.http_status.map(i32::from);
            if let Some(tokens) = outcome.tokens {
                let tokens_in = i64::try_from(tokens.input).ok();
                let tokens_out = i64::try_from(tokens.output).ok();
                completion.tokens_in = tokens_in;
                completion.tokens_out = tokens_out;
                completion.total = tokens_in
                    .zip(tokens_out)
                    .and_then(|(a, b)| a.checked_add(b));
                completion.cache_read_tokens = tokens
                    .cache_read
                    .and_then(|value| i64::try_from(value).ok());
                completion.cache_write_tokens = tokens
                    .cache_write
                    .and_then(|value| i64::try_from(value).ok());
            }
            completion.request_id = outcome
                .request_id
                .filter(|value| safe_id(value))
                .map(str::to_owned);
            recovery_log(completion);
            let _ = recover(completion).await;
            self.completion = None;
        }
    }

    pub async fn finish_before(
        self,
        body: Option<&Value>,
        error_code: Option<&str>,
        deadline: Instant,
    ) {
        let _ = tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            self.finish(body, error_code),
        )
        .await;
    }

    pub async fn finish_refused_before(mut self, body: &[u8], account: Account, deadline: Instant) {
        self.refuse(body, account);
        self.finish_before(None, Some("http_error"), deadline).await;
    }

    fn refuse(&mut self, body: &[u8], account: Account) {
        if let Some(completion) = &mut self.completion {
            completion.error_reason = completion
                .http_status
                .and_then(|status| u16::try_from(status).ok())
                .map(|status| refusal_reason(status, &error_words(body), account).to_owned());
        }
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if let Some(mut completion) = self.completion.take() {
            completion.latency_ms =
                i64::try_from(self.started.elapsed().as_millis()).unwrap_or(i64::MAX);
            recovery_log(&completion);
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = recover(&completion).await;
                });
            }
        }
    }
}

fn ledger_unavailable() -> ChatProviderError {
    ChatProviderError::Provider("Verbrauchserfassung nicht bereit; KI-Aufruf angehalten".into())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}
fn count(body: &Value, name: &str) -> Option<i64> {
    body["usage"][name].as_i64().filter(|value| *value >= 0)
}
pub(crate) fn cached_tokens(body: &Value) -> Option<u64> {
    body["usage"]["prompt_tokens_details"]["cached_tokens"].as_u64()
}
pub(crate) fn error_words(body: &[u8]) -> String {
    let Ok(body) = serde_json::from_slice::<Value>(body) else {
        return String::new();
    };
    ["type", "message"]
        .iter()
        .filter_map(|field| body["error"][*field].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
fn recovery_log(completion: &Completion) {
    if let Ok(recovery) = serde_json::to_string(completion) {
        eprintln!("LLM_USAGE_RECOVERY: recovery={recovery}");
    }
}
pub async fn recover(completion: &Completion) -> Result<(), sqlx::Error> {
    let Some((pool, service)) = LEDGER.get() else {
        return Err(sqlx::Error::PoolClosed);
    };
    if completion.project != "Deadlock-Bots"
        || completion.service != *service
        || completion.id <= 0
        || completion.latency_ms < 0
        || [
            completion.tokens_in,
            completion.tokens_out,
            completion.total,
            completion.cache_read_tokens,
            completion.cache_write_tokens,
        ]
        .into_iter()
        .flatten()
        .any(|value| value < 0)
        || completion
            .request_id
            .as_deref()
            .is_some_and(|value| !safe_id(value))
        || completion
            .error_reason
            .as_deref()
            .is_some_and(|value| !safe_code(value))
        || completion.error_code.as_deref().is_some_and(|value| {
            !matches!(
                value,
                "cancelled" | "http_error" | "invalid_json" | "response_body"
            ) && !FailureKind::ALL
                .iter()
                .any(|kind| kind.ledger_code() == value)
        })
    {
        return Err(sqlx::Error::Protocol(
            "Verbrauchsherkunft passt nicht zum Dienst".into(),
        ));
    }
    let updated = sqlx::query("UPDATE public.llm_usage SET tokens_in=$2,tokens_out=$3,total=$4,request_id=$5,success=$6,attempt_state=$7,finished_at=$8,error_code=$9,http_status=$10,latency_ms=$11,error_reason=$14,cache_read_tokens=$15,cache_write_tokens=$16 WHERE id=$1 AND attempt_state='started' AND project=$12 AND service=$13")
        .bind(completion.id).bind(completion.tokens_in).bind(completion.tokens_out).bind(completion.total)
        .bind(&completion.request_id).bind(i64::from(completion.success))
        .bind(if completion.success { "succeeded" } else { "failed" })
        .bind(Utc::now()).bind(&completion.error_code)
        .bind(completion.http_status).bind(completion.latency_ms)
        .bind(&completion.project).bind(&completion.service)
        .bind(&completion.error_reason).bind(completion.cache_read_tokens).bind(completion.cache_write_tokens)
        .execute(pool).await?;
    if updated.rows_affected() == 0 {
        let completed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.llm_usage WHERE id=$1 AND project=$2 AND service=$3 AND attempt_state IN ('succeeded','failed'))")
            .bind(completion.id).bind(&completion.project).bind(&completion.service).fetch_one(pool).await?;
        if !completed {
            return Err(sqlx::Error::RowNotFound);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_chat::{Delivery, Refusal, RefusalAlarm, RefusalNotifier};
    use serde_json::json;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::sync::atomic::{AtomicUsize, Ordering};

    type AttemptRow = (String, Option<i64>, Option<i32>, Option<String>);
    type CacheRow = (
        i64,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );

    struct Counting(AtomicUsize);

    impl RefusalNotifier for Counting {
        fn notify(&self, refusal: Refusal, delivery: Delivery) {
            assert_eq!(refusal.reason, "billing");
            self.0.fetch_add(1, Ordering::SeqCst);
            drop(delivery);
        }
    }

    #[test]
    fn unbekannte_zaehler_bleiben_unbekannt_und_ids_enthalten_keine_inhalte() {
        assert_eq!(count(&json!({}), "prompt_tokens"), None);
        assert_eq!(
            count(&json!({"usage":{"prompt_tokens":-1}}), "prompt_tokens"),
            None
        );
        assert_eq!(
            count(&json!({"usage":{"prompt_tokens":0}}), "prompt_tokens"),
            Some(0)
        );
        assert!(safe_id("req-fixture_123"));
        assert!(!safe_id("Bearer geheimes token"));
        assert!(!safe_id(&"a".repeat(201)));
        let completion = Completion {
            project: "Deadlock-Bots".into(),
            service: "dl-bot".into(),
            id: 7,
            tokens_in: None,
            tokens_out: None,
            total: None,
            request_id: None,
            success: false,
            error_code: Some("cancelled".into()),
            http_status: None,
            latency_ms: 12,
            error_reason: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        let message = format!("LLM_USAGE_RECOVERY: Abschluss aus dem Journal nachliefern \u{1b}[3mrecovery\u{1b}[0m\u{1b}[2m=\u{1b}[0m{}",serde_json::to_string(&completion).unwrap());
        let record = json!({"MESSAGE":message}).to_string();
        assert_eq!(recovery_from_journal(&record).unwrap().id, 7);
        assert!(
            recovery_from_journal(&json!({"MESSAGE":"kein Verbrauchsbeleg"}).to_string()).is_none()
        );
    }

    #[test]
    fn a_refused_attempt_stores_the_reason_of_status_body_and_account() {
        let stored = |http_status: Option<i32>, text: &str, account| {
            let mut attempt = Attempt {
                completion: Some(Completion {
                    project: "Deadlock-Bots".into(),
                    service: "dl-bot".into(),
                    id: 7,
                    tokens_in: None,
                    tokens_out: None,
                    total: None,
                    request_id: None,
                    success: false,
                    error_code: Some("cancelled".into()),
                    http_status,
                    latency_ms: 0,
                    error_reason: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                }),
                started: Instant::now(),
            };
            attempt.refuse(text.as_bytes(), account);
            attempt
                .completion
                .take()
                .and_then(|completion| completion.error_reason)
        };
        let credit = r#"{"error":{"message":"Your credit balance is too low."}}"#;
        let usage = r#"{"error":{"type":"invalid_request_error","message":"usage limit reached"}}"#;
        let typed = r#"{"error":{"type":"billing_error","message":"x"}}"#;
        let echoed = r#"{"error":{"type":"invalid_request_error","message":"bad field"},"input":"my billing budget"}"#;
        for (status, text, account, reason) in [
            (Some(400), credit, Account::Metered, Some("billing")),
            (Some(403), credit, Account::Metered, Some("billing")),
            (Some(400), typed, Account::Metered, Some("billing")),
            (Some(400), usage, Account::Metered, Some("billing")),
            (Some(400), usage, Account::Plan, Some("usage_limit")),
            (Some(412), "", Account::Metered, Some("billing")),
            (Some(400), "", Account::Metered, Some("http_4xx")),
            (Some(400), echoed, Account::Metered, Some("http_4xx")),
            (
                Some(400),
                "credit balance too low",
                Account::Metered,
                Some("http_4xx"),
            ),
            (
                Some(400),
                r#"{"error":"credit balance too low"}"#,
                Account::Plan,
                Some("http_4xx"),
            ),
            (Some(429), credit, Account::Metered, Some("http_429")),
            (Some(500), "", Account::Metered, Some("http_5xx")),
            (Some(0), credit, Account::Metered, Some("http_status")),
            (Some(-1), credit, Account::Metered, None),
            (Some(70_000), credit, Account::Metered, None),
            (None, credit, Account::Metered, None),
        ] {
            assert_eq!(
                stored(status, text, account).as_deref(),
                reason,
                "{status:?} {text}"
            );
        }
        assert_eq!(
            cached_tokens(&json!({"usage":{"prompt_tokens_details":{"cached_tokens":0}}})),
            Some(0)
        );
        assert_eq!(error_words(typed.as_bytes()), "billing_error\nx");
        assert_eq!(
            error_words(credit.as_bytes()),
            "Your credit balance is too low."
        );
        for body in [&b""[..], b"null", b"[1]", b"\xff", br#"{"error":"flat"}"#] {
            assert_eq!(error_words(body), "");
        }
        assert_eq!(cached_tokens(&json!({"usage":{}})), None);
        assert_eq!(
            cached_tokens(&json!({"usage":{"prompt_tokens_details":{"cached_tokens":-3}}})),
            None
        );
    }

    #[test]
    fn the_migration_checks_reasons_with_the_shared_code_shape() {
        let migration = include_str!(
            "../../dl-central-db/migrations/20261117120000_llm_usage_refusal_reason_cache_tokens.sql"
        );
        let check = format!(
            "CHECK (error_reason ~ '{}')",
            crate::upstream::code_pattern()
        );
        assert_eq!(migration.matches(&check).count(), 1, "{check}");
        assert_eq!(migration.matches("error_reason ~").count(), 1);
    }

    #[test]
    fn journalbeleg_bleibt_bei_error_filter_sichtbar() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "usage::tests::journalbeleg_stderr_kind",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        let message = stderr
            .lines()
            .find(|line| line.starts_with("LLM_USAGE_RECOVERY: recovery="))
            .expect("Vorabbeleg wird unabhängig vom Tracingfilter geschrieben");
        let record = json!({"MESSAGE":message}).to_string();
        let completion = recovery_from_journal(&record).expect("Echter Stderrbeleg ist replaybar");
        assert_eq!(completion.total, Some(15));
    }

    #[test]
    #[ignore = "Isolierter Kindprozess für den Error-Filter"]
    fn journalbeleg_stderr_kind() {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::ERROR)
            .init();
        recovery_log(&Completion {
            project: "Deadlock-Bots".into(),
            service: "test-fixture".into(),
            id: 7,
            tokens_in: Some(12),
            tokens_out: Some(3),
            total: Some(15),
            request_id: None,
            success: true,
            error_code: None,
            http_status: Some(200),
            latency_ms: 1,
            error_reason: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        });
    }

    #[tokio::test]
    #[ignore = "Benötigt die Wegwerf-Datenbank aus test-db-run (DATABASE_URL)"]
    async fn lokale_datenbank_belegt_abschluss_abbruch_nullwerte_und_herkunft() {
        let url = std::env::var("DATABASE_URL").unwrap();
        let options = url.parse::<PgConnectOptions>().unwrap();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .unwrap();
        let database = format!("fireworks_bots_usage_test_{}", std::process::id());
        sqlx::query(&format!("CREATE DATABASE {database}"))
            .execute(&admin)
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options.database(&database))
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../dl-central-db/migrations/20261005120200_llm_usage_attempts.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../dl-central-db/migrations/20261117120000_llm_usage_refusal_reason_cache_tokens.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        initialize(pool.clone(), "test-fixture").unwrap();
        let request = reqwest::Client::new()
            .post("https://api.fireworks.ai/inference/v1/chat/completions")
            .json(&json!({"model":"fixture-model","messages":[{"content":"nicht speichern"}]}))
            .build()
            .unwrap();
        let mut lock = pool.begin().await.unwrap();
        sqlx::query("LOCK TABLE public.llm_usage IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *lock)
            .await
            .unwrap();
        let started = Instant::now();
        let result =
            Attempt::begin_before(&request, started + std::time::Duration::from_millis(100)).await;
        assert!(matches!(result, Err(ChatProviderError::Timeout)));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        lock.rollback().await.unwrap();

        let delayed = Attempt::begin(&request).await.unwrap();
        let delayed_id = delayed.completion.as_ref().unwrap().id;
        let mut lock = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM public.llm_usage WHERE id=$1 FOR UPDATE")
            .bind(delayed_id)
            .fetch_one(&mut *lock)
            .await
            .unwrap();
        let started = Instant::now();
        delayed
            .finish_before(
                Some(
                    &json!({"usage":{"prompt_tokens":19,"completion_tokens":5,"total_tokens":24}}),
                ),
                None,
                started + std::time::Duration::from_millis(100),
            )
            .await;
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        lock.rollback().await.unwrap();
        let total = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let total: Option<i64> =
                    sqlx::query_scalar("SELECT total FROM public.llm_usage WHERE id=$1")
                        .bind(delayed_id)
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                if let Some(total) = total {
                    break total;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("Drop-Recovery liefert echte Fixtureusage nach");
        assert_eq!(total, 24);

        let attempt = with_purpose("fixture.text", Attempt::begin(&request))
            .await
            .unwrap();
        let completion = attempt.completion.as_ref().unwrap().clone();
        attempt.finish(Some(&json!({"id":"fixture-request","usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}})), None).await;
        let row: (String,Option<i64>,Option<i64>,Option<i64>,String,String,String) = sqlx::query_as("SELECT attempt_state,tokens_in,tokens_out,total,project,service,purpose FROM public.llm_usage WHERE id=$1").bind(completion.id).fetch_one(&pool).await.unwrap();
        assert_eq!(
            row,
            (
                "succeeded".into(),
                Some(12),
                Some(3),
                Some(15),
                "Deadlock-Bots".into(),
                "test-fixture".into(),
                "fixture.text".into()
            )
        );
        recover(&completion).await.unwrap();
        let state: String =
            sqlx::query_scalar("SELECT attempt_state FROM public.llm_usage WHERE id=$1")
                .bind(completion.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "succeeded");
        let mut foreign = completion.clone();
        foreign.project = "Deadlock-Twitch-Bot".into();
        assert!(recover(&foreign).await.is_err());
        let mut missing = completion.clone();
        missing.id = i64::MAX;
        assert!(matches!(
            recover(&missing).await,
            Err(sqlx::Error::RowNotFound)
        ));
        let unknown = Attempt::begin(&request).await.unwrap();
        let unknown_id = unknown.completion.as_ref().unwrap().id;
        unknown.finish(Some(&json!({})), None).await;
        let tokens: Option<i64> =
            sqlx::query_scalar("SELECT total FROM public.llm_usage WHERE id=$1")
                .bind(unknown_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tokens, None);
        let cancelled = Attempt::begin(&request).await.unwrap();
        let id = cancelled.completion.as_ref().unwrap().id;
        drop(cancelled);
        for _ in 0..100 {
            let state: String =
                sqlx::query_scalar("SELECT attempt_state FROM public.llm_usage WHERE id=$1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            if state == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let error: String = sqlx::query_scalar(
            "SELECT error_code FROM public.llm_usage WHERE id=$1 AND attempt_state='failed'",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(error, "cancelled");
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server_calls = calls.clone();
        let server_pool = pool.clone();
        let app = axum::Router::new().route("/chat/completions", axum::routing::post(move || {
            let calls = server_calls.clone();
            let pool = server_pool.clone();
            async move {
                let started: i64 = sqlx::query_scalar("SELECT count(*) FROM public.llm_usage WHERE purpose='fixture.retry' AND attempt_state='started'")
                    .fetch_one(&pool).await.unwrap();
                assert_eq!(started,1,"Vor jeder HTTP-Anfrage muss genau ihr Start stehen");
                if calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0 {
                    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({})))
                } else {
                    (axum::http::StatusCode::OK, axum::Json(json!({"choices":[{"message":{"content":"Fixtureantwort"}}],"usage":{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9}})))
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let provider = crate::OpenAiChatProvider::from_fireworks_env(
            |key| match key {
                "FIREWORK_API_KEY" => Some("synthetischer-fixture-schluessel".into()),
                "FIREWORK_BASE_URL" => Some(base.clone()),
                _ => None,
            },
            crate::RetryConfig {
                request_timeout: std::time::Duration::from_secs(2),
                max_retries: 1,
                base_backoff: std::time::Duration::from_millis(1),
            },
        )
        .unwrap();
        TRACK_LOCAL_FIXTURE
            .scope(
                true,
                with_purpose(
                    "fixture.retry",
                    provider.chat(&[ChatMessage::user("Fixture")], ChatParams::default()),
                ),
            )
            .await
            .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        let attempts: Vec<AttemptRow> = sqlx::query_as("SELECT attempt_state,total,http_status,error_reason FROM public.llm_usage WHERE purpose='fixture.retry' ORDER BY id")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(
            attempts,
            vec![
                ("failed".into(), None, Some(500), Some("http_5xx".into())),
                ("succeeded".into(), Some(9), Some(200), None)
            ]
        );
        let refused = Attempt::begin_call("anthropic_api", "brain_answer", "fixture-model")
            .await
            .unwrap();
        let refused_id = refused.completion.as_ref().map(|c| c.id).unwrap();
        refused
            .finish_call(CallOutcome {
                error_code: Some("rate_limited"),
                error_reason: Some("billing"),
                http_status: Some(400),
                ..CallOutcome::default()
            })
            .await;
        let served = Attempt::begin_call("anthropic_api", "brain_answer", "fixture-model")
            .await
            .unwrap();
        let served_id = served.completion.as_ref().map(|c| c.id).unwrap();
        served
            .finish_call(CallOutcome {
                tokens: Some(CallTokens {
                    input: 21_000,
                    output: 40,
                    cache_read: Some(20_800),
                    cache_write: Some(0),
                }),
                ..CallOutcome::default()
            })
            .await;
        let unknown_cache = Attempt::begin_call("codex_plan", "brain_answer", "fixture-model")
            .await
            .unwrap();
        let unknown_cache_id = unknown_cache.completion.as_ref().map(|c| c.id).unwrap();
        unknown_cache
            .finish_call(CallOutcome {
                tokens: Some(CallTokens {
                    input: 10,
                    output: 2,
                    ..CallTokens::default()
                }),
                http_status: Some(200),
                ..CallOutcome::default()
            })
            .await;
        let rows: Vec<CacheRow> =
            sqlx::query_as("SELECT id,error_code,error_reason,http_status,tokens_in,cache_read_tokens,cache_write_tokens FROM public.llm_usage WHERE id IN ($1,$2,$3) ORDER BY id")
                .bind(refused_id)
                .bind(served_id)
                .bind(unknown_cache_id)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    refused_id,
                    Some("rate_limited".into()),
                    Some("billing".into()),
                    Some(400),
                    None,
                    None,
                    None
                ),
                (
                    served_id,
                    None,
                    None,
                    None,
                    Some(21_000),
                    Some(20_800),
                    Some(0)
                ),
                (
                    unknown_cache_id,
                    None,
                    None,
                    Some(200),
                    Some(10),
                    None,
                    None
                ),
            ]
        );
        for (provider, reason, status) in [
            ("anthropic_api", "billing", 400),
            ("claude_plan", "http_429", 429),
        ] {
            Attempt::begin_call(provider, "brain_answer", "fixture-model")
                .await
                .unwrap()
                .finish_call(CallOutcome {
                    error_code: Some("rate_limited"),
                    error_reason: Some(reason),
                    http_status: Some(status),
                    ..CallOutcome::default()
                })
                .await;
        }
        let mut failures: Vec<(String, Option<String>, Option<i32>)> = recent_failures(3_600, 256)
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.provider == "anthropic_api" || row.provider == "claude_plan")
            .map(|row| (row.provider, row.error_reason, row.http_status))
            .collect();
        failures.sort();
        assert_eq!(
            failures,
            vec![
                ("anthropic_api".into(), Some("billing".into()), Some(400)),
                ("claude_plan".into(), Some("http_429".into()), Some(429)),
            ],
            "only refusals after the last success of a provider count"
        );
        let counting = Arc::new(Counting(AtomicUsize::new(0)));
        let alarm = RefusalAlarm::new(counting.clone(), 600);
        let rows = recent_failures(3_600, 256).await.unwrap();
        let now = Utc::now().timestamp();
        alarm.observe(&rows, now);
        alarm.observe(&rows, now + 60);
        assert_eq!(counting.0.load(Ordering::SeqCst), 1);
        sqlx::query("DROP TABLE public.llm_usage")
            .execute(&pool)
            .await
            .unwrap();
        assert!(Attempt::begin(&request).await.is_err());
        assert!(TRACK_LOCAL_FIXTURE
            .scope(
                true,
                provider.chat(&[ChatMessage::user("Fixture")], ChatParams::default())
            )
            .await
            .is_err());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "Ohne gespeicherten Start darf keine weitere HTTP-Anfrage rausgehen"
        );
        server.abort();
        pool.close().await;
        sqlx::query(&format!("DROP DATABASE {database}"))
            .execute(&admin)
            .await
            .unwrap();
    }
}

pub(crate) fn wrap(inner: Arc<dyn ChatProvider>, purpose: String) -> Arc<dyn ChatProvider> {
    Arc::new(UsageProvider { inner, purpose })
}
struct UsageProvider {
    inner: Arc<dyn ChatProvider>,
    purpose: String,
}
#[async_trait::async_trait]
impl ChatProvider for UsageProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<ChatResponse, ChatProviderError> {
        PURPOSE
            .scope(self.purpose.clone(), self.inner.chat(messages, params))
            .await
    }
    async fn chat_with_status(
        &self,
        messages: &[ChatMessage],
        params: ChatParams,
    ) -> Result<(ChatResponse, Option<u16>), ChatProviderError> {
        PURPOSE
            .scope(
                self.purpose.clone(),
                self.inner.chat_with_status(messages, params),
            )
            .await
    }
    fn effective_model(&self, params: &ChatParams) -> Option<String> {
        self.inner.effective_model(params)
    }
}
pub async fn with_purpose<T>(purpose: &str, future: impl std::future::Future<Output = T>) -> T {
    PURPOSE.scope(purpose.to_owned(), future).await
}

pub fn text(
    inner: Arc<dyn crate::TextGenerator>,
    purpose: &'static str,
) -> Arc<dyn crate::TextGenerator> {
    Arc::new(PurposeText { inner, purpose })
}
struct PurposeText {
    inner: Arc<dyn crate::TextGenerator>,
    purpose: &'static str,
}
#[async_trait::async_trait]
impl crate::TextGenerator for PurposeText {
    async fn generate_text(&self, request: crate::GenerateRequest) -> Option<String> {
        with_purpose(self.purpose, self.inner.generate_text(request)).await
    }
}
pub fn vision(
    inner: Arc<dyn crate::VisionGenerator>,
    purpose: &'static str,
) -> Arc<dyn crate::VisionGenerator> {
    Arc::new(PurposeVision { inner, purpose })
}
struct PurposeVision {
    inner: Arc<dyn crate::VisionGenerator>,
    purpose: &'static str,
}
#[async_trait::async_trait]
impl crate::VisionGenerator for PurposeVision {
    async fn generate_multimodal(
        &self,
        request: crate::GenerateMultimodalRequest,
    ) -> Option<String> {
        with_purpose(self.purpose, self.inner.generate_multimodal(request)).await
    }
}
