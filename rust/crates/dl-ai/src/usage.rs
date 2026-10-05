//! Dauerhafte Verbrauchsbelege ohne Nachrichten oder Zugangsdaten.
use std::sync::Arc;
use std::time::Instant;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::OnceCell;

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

/// Liest ausschließlich technische Abschlussbelege des eigenen Dienstes.
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
    // Laufende Aufrufe haben fünf Minuten Zeit; ältere offene Versuche ohne
    // Journalbeleg bleiben unbekannt und müssen ebenfalls sichtbar gemeldet werden.
    let outstanding: i64 = sqlx::query_scalar("SELECT count(*) FROM public.llm_usage WHERE project='Deadlock-Bots' AND service=$1 AND attempt_state='started' AND ts::timestamptz < now() - interval '5 minutes'")
        .bind(service).fetch_one(pool).await?;
    unresolved_count = unresolved_count.max(usize::try_from(outstanding).unwrap_or(usize::MAX));
    Ok(unresolved_count)
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
        let unavailable = || {
            ChatProviderError::Provider(
                "Verbrauchserfassung nicht bereit; KI-Aufruf angehalten".into(),
            )
        };
        let (pool, service) = LEDGER.get().ok_or_else(unavailable)?;
        let model = request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .and_then(|body| serde_json::from_slice::<Value>(body).ok())
            .and_then(|body| body["model"].as_str().map(str::to_owned))
            .ok_or_else(unavailable)?;
        let purpose = PURPOSE
            .try_with(Clone::clone)
            .unwrap_or_else(|_| "direct_chat".into());
        let id = sqlx::query_scalar("INSERT INTO public.llm_usage (ts,source,purpose,model,tokens_in,tokens_out,total,success,project,service,provider,attempt_state) VALUES ($1,'deadlock-bots',$2,$3,NULL,NULL,NULL,0,'Deadlock-Bots',$4,'fireworks','started') RETURNING id")
            .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false)).bind(purpose).bind(model).bind(service)
            .fetch_one(pool).await.map_err(|_| {
                tracing::error!("LLM_USAGE_BLOCKED: Aufrufstart konnte nicht gespeichert werden");
                unavailable()
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
        ]
        .into_iter()
        .flatten()
        .any(|value| value < 0)
        || completion
            .request_id
            .as_deref()
            .is_some_and(|value| !safe_id(value))
        || completion.error_code.as_deref().is_some_and(|value| {
            !matches!(
                value,
                "cancelled"
                    | "timeout"
                    | "transport"
                    | "http_error"
                    | "invalid_json"
                    | "response_body"
                    | "invalid_output"
            )
        })
    {
        return Err(sqlx::Error::Protocol(
            "Verbrauchsherkunft passt nicht zum Dienst".into(),
        ));
    }
    let updated = sqlx::query("UPDATE public.llm_usage SET tokens_in=$2,tokens_out=$3,total=$4,request_id=$5,success=$6,attempt_state=$7,finished_at=$8,error_code=$9,http_status=$10,latency_ms=$11 WHERE id=$1 AND attempt_state='started' AND project=$12 AND service=$13")
        .bind(completion.id).bind(completion.tokens_in).bind(completion.tokens_out).bind(completion.total)
        .bind(&completion.request_id).bind(i64::from(completion.success))
        .bind(if completion.success { "succeeded" } else { "failed" })
        .bind(Utc::now()).bind(&completion.error_code)
        .bind(completion.http_status).bind(completion.latency_ms)
        .bind(&completion.project).bind(&completion.service).execute(pool).await?;
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
    use serde_json::json;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

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
        };
        let message = format!("LLM_USAGE_RECOVERY: Abschluss aus dem Journal nachliefern \u{1b}[3mrecovery\u{1b}[0m\u{1b}[2m=\u{1b}[0m{}",serde_json::to_string(&completion).unwrap());
        let record = json!({"MESSAGE":message}).to_string();
        assert_eq!(recovery_from_journal(&record).unwrap().id, 7);
        assert!(
            recovery_from_journal(&json!({"MESSAGE":"kein Verbrauchsbeleg"}).to_string()).is_none()
        );
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
        });
    }

    #[tokio::test]
    #[ignore = "Benötigt lokale PostgreSQL-Peer-Authentifizierung und CREATE DATABASE"]
    async fn lokale_datenbank_belegt_abschluss_abbruch_nullwerte_und_herkunft() {
        let options = PgConnectOptions::new()
            .host("/var/run/postgresql")
            .username("nathanael")
            .database("postgres");
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
        initialize(pool.clone(), "test-fixture").unwrap();
        // Die URL wird nur als Metadatum gebaut; dieser Test sendet keine Anfrage.
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
        let attempts: Vec<(String,Option<i64>)> = sqlx::query_as("SELECT attempt_state,total FROM public.llm_usage WHERE purpose='fixture.retry' ORDER BY id")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(
            attempts,
            vec![("failed".into(), None), ("succeeded".into(), Some(9))]
        );
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
