use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dl_ai::tool_chat::{AlarmConfig, Delivery, Refusal, RefusalAlarm, RefusalNotifier};
use dl_ai::upstream::{protected_path, Answer, CONTRACT, DAY_SECS, GATEWAY_UNIT, HELPER_BINARY};

const CHILD_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const NO_HELPER: &str = "the release ships no protected notify helper next to the gateway";
const SCAN_LIMIT: i64 = 256;

pub struct ProcessNotifier {
    config: AlarmConfig,
    binary: PathBuf,
}

impl ProcessNotifier {
    pub fn new(config: AlarmConfig, binary: PathBuf) -> Self {
        Self { config, binary }
    }
}

pub fn sibling_helper() -> anyhow::Result<PathBuf> {
    let gateway = std::env::current_exe()?;
    gateway
        .parent()
        .map(|release| release.join(HELPER_BINARY))
        .ok_or_else(|| anyhow::anyhow!("gateway binary {} has no directory", gateway.display()))
}

impl RefusalNotifier for ProcessNotifier {
    fn notify(&self, refusal: Refusal, delivery: Delivery) {
        tracing::warn!(
            provider = %refusal.provider,
            class = refusal.class.as_str(),
            kind = refusal.kind.as_str(),
            reason = %refusal.reason,
            status = ?refusal.status,
            "AI_GATEWAY_PROVIDER_REFUSAL"
        );
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::error!(
                provider = %refusal.provider,
                cause = "no async runtime",
                "AI_GATEWAY_ALARM_FAILED"
            );
            return;
        };
        let command = command(&self.config, &self.binary, &refusal);
        runtime.spawn(run(
            command,
            self.config.notify_timeout(),
            refusal,
            delivery,
        ));
    }
}

fn helper(config: &AlarmConfig, binary: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command
        .env_clear()
        .env("PATH", CHILD_PATH)
        .env("LANG", "C.UTF-8")
        .arg("--config")
        .arg(&config.notify_config)
        .arg("--credential")
        .arg(&config.credential);
    if let Some(home) = std::env::var_os("HOME") {
        command.env("HOME", home);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    command
}

fn command(config: &AlarmConfig, binary: &Path, refusal: &Refusal) -> tokio::process::Command {
    let mut command = helper(config, binary);
    command
        .arg("--unit")
        .arg(GATEWAY_UNIT)
        .arg("--provider-refusal")
        .arg("--provider")
        .arg(&refusal.provider)
        .arg("--kind")
        .arg(refusal.kind.as_str())
        .arg("--reason")
        .arg(&refusal.reason)
        .arg("--event-id")
        .arg(refusal.event_id());
    if let Some(status) = refusal.status {
        command.arg("--status").arg(status.to_string());
    }
    command
}

async fn output(
    mut command: tokio::process::Command,
    budget: Duration,
) -> Result<String, &'static str> {
    let child = command.spawn().map_err(|_| "helper could not start")?;
    let output = tokio::time::timeout(budget, child.wait_with_output())
        .await
        .map_err(|_| "helper timed out")?
        .map_err(|_| "helper unreachable")?;
    if !output.status.success() {
        return Err("helper reported a failure");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn deliver(
    command: tokio::process::Command,
    budget: Duration,
) -> Result<Answer, &'static str> {
    Answer::parse(&output(command, budget).await?).ok_or("helper gave no answer")
}

async fn run(
    command: tokio::process::Command,
    budget: Duration,
    refusal: Refusal,
    delivery: Delivery,
) {
    match deliver(command, budget).await {
        Ok(answer) => {
            tracing::info!(
                provider = %refusal.provider,
                class = refusal.class.as_str(),
                outcome = ?answer.outcome,
                next_at = answer.next_at,
                "AI_GATEWAY_ALARM_ANSWERED"
            );
            delivery.answered(answer);
        }
        Err(cause) => {
            tracing::error!(
                provider = %refusal.provider,
                class = refusal.class.as_str(),
                cause,
                "AI_GATEWAY_ALARM_FAILED"
            );
            drop(delivery);
        }
    }
}

pub async fn readiness(config: &AlarmConfig, binary: &Path) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if protected_path(binary, false).is_err() || !executable(binary) {
        missing.push(NO_HELPER);
    }
    let mut handshake = helper(config, binary);
    handshake.arg("--contract");
    let spoken = output(handshake, config.notify_timeout())
        .await
        .is_ok_and(|stdout| stdout.lines().any(|line| line.trim() == CONTRACT));
    if !spoken {
        missing.push("notify helper does not accept this config, credential and contract");
    }
    if !on_failure_wired(config.notify_timeout()).await {
        missing.push("dl-ai-gateway.service has no OnFailure alarm");
    }
    missing
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn on_failure_unit() -> String {
    format!("unit-failure-notify@{GATEWAY_UNIT}.service")
}

async fn on_failure_wired(budget: Duration) -> bool {
    let mut command = tokio::process::Command::new("/usr/bin/systemctl");
    command
        .env_clear()
        .env("PATH", CHILD_PATH)
        .args(["--user", "show", GATEWAY_UNIT, "-p", "OnFailure", "--value"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for name in ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "HOME"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    output(command, budget).await.is_ok_and(|stdout| {
        let wanted = on_failure_unit();
        stdout.split_whitespace().any(|unit| unit == wanted)
    })
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

pub async fn scan(alarm: &RefusalAlarm, budget: Duration) -> Result<(), String> {
    let lookback = i64::try_from(DAY_SECS).unwrap_or(i64::MAX);
    let rows = tokio::time::timeout(budget, dl_ai::usage::recent_failures(lookback, SCAN_LIMIT))
        .await
        .map_err(|_| "ledger scan timed out".to_owned())?
        .map_err(|error| error.to_string())?;
    alarm.observe(&rows, now());
    Ok(())
}

pub async fn watch(alarm: Arc<RefusalAlarm>, config: AlarmConfig) -> anyhow::Error {
    let mut interval = tokio::time::interval(config.scan_interval());
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    let mut healthy = tokio::time::Instant::now();
    loop {
        interval.tick().await;
        match scan(&alarm, config.scan_interval()).await {
            Ok(()) => healthy = tokio::time::Instant::now(),
            Err(cause) => {
                tracing::error!(cause = %cause, "AI_GATEWAY_ALARM_SCAN_FAILED");
                if healthy.elapsed() >= config.standing_after() {
                    return anyhow::anyhow!(
                        "provider refusal scan failed for longer than {} s",
                        config.standing_after_secs
                    );
                }
            }
        }
    }
}

pub fn unready(missing: &[&'static str]) -> Option<anyhow::Error> {
    (!missing.is_empty()).then(|| anyhow::anyhow!("alarm not ready: {}", missing.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_ai::upstream::{Class, Outcome, RefusalKind};
    use std::ffi::OsStr;

    fn config(credential: &Path) -> AlarmConfig {
        AlarmConfig {
            notify_config: "/etc/hostname".into(),
            credential: credential.into(),
            standing_after_secs: 600,
            scan_interval_secs: 60,
            notify_timeout_secs: 75,
        }
    }

    fn refusal() -> Refusal {
        Refusal {
            provider: "anthropic_api".into(),
            class: Class::Billing,
            kind: RefusalKind::Limit,
            reason: "billing".into(),
            status: Some(400),
            first_at: 100,
            last_at: 160,
        }
    }

    fn script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        path
    }

    fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .map(String::as_str)
    }

    #[test]
    fn helper_gets_the_refusal_and_its_event_id() {
        let command = command(
            &config(Path::new("/run/c")),
            Path::new("/bin/notify"),
            &refusal(),
        );
        assert_eq!(command.as_std().get_program(), OsStr::new("/bin/notify"));
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(OsStr::to_string_lossy)
            .map(|arg| arg.into_owned())
            .collect();
        assert_eq!(arg(&args, "--unit"), Some(GATEWAY_UNIT));
        assert_eq!(arg(&args, "--provider"), Some("anthropic_api"));
        assert_eq!(arg(&args, "--kind"), Some("limit"));
        assert_eq!(arg(&args, "--reason"), Some("billing"));
        assert_eq!(arg(&args, "--status"), Some("400"));
        assert_eq!(arg(&args, "--event-id"), Some("100-160"));
        assert_eq!(arg(&args, "--credential"), Some("/run/c"));
    }

    #[tokio::test]
    async fn only_an_exact_answer_line_counts_and_no_parent_environment_leaks() {
        let dir = tempfile::tempdir().expect("dir");
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        let budget = Duration::from_secs(5);
        let run = |body: &str, name: &str| {
            command(
                &config(Path::new("/run/c")),
                &script(dir.path(), name, body),
                &refusal(),
            )
        };
        assert_eq!(
            deliver(
                run("echo noise\necho outcome=sent next_at=7", "sent"),
                budget
            )
            .await,
            Ok(Answer {
                outcome: Outcome::Sent,
                next_at: 7
            })
        );
        assert_eq!(
            deliver(run("echo outcome=sent", "old"), budget).await,
            Err("helper gave no answer")
        );
        assert_eq!(
            deliver(
                run("echo outcome=sent next_at=7\nexit 1", "failing"),
                budget
            )
            .await,
            Err("helper reported a failure")
        );
        assert_eq!(
            deliver(
                run(
                    "if [ -n \"$CARGO_MANIFEST_DIR\" ]; then exit 3; fi\necho outcome=suppressed next_at=9",
                    "inherit"
                ),
                budget
            )
            .await,
            Ok(Answer {
                outcome: Outcome::Suppressed,
                next_at: 9
            })
        );
        assert_eq!(
            deliver(
                run("sleep 5\necho outcome=sent next_at=7", "slow"),
                Duration::from_millis(200)
            )
            .await,
            Err("helper timed out")
        );
    }

    #[test]
    fn the_checked_on_failure_unit_is_the_one_the_shipped_unit_file_names() {
        let unit = include_str!("../../../../service/systemd/dl-ai-gateway.service");
        let wired: Vec<String> = unit
            .lines()
            .filter_map(|line| line.strip_prefix("OnFailure="))
            .map(|target| target.replace("%n", GATEWAY_UNIT))
            .collect();
        assert_eq!(wired, [on_failure_unit()]);
    }

    #[tokio::test]
    async fn an_unprepared_host_is_reported_not_ready() {
        let dir = tempfile::tempdir().expect("dir");
        let old_helper = script(dir.path(), "old", "exit 2");
        let missing = readiness(&config(Path::new("/nonexistent/c")), &old_helper).await;
        assert!(
            missing.contains(&"notify helper does not accept this config, credential and contract")
        );
        let absent = Path::new("/nonexistent/notify");
        let missing = readiness(&config(Path::new("/run/c")), absent).await;
        assert!(missing.contains(&NO_HELPER));
        let shipped = sibling_helper().expect("sibling");
        assert_eq!(shipped.file_name(), Some(OsStr::new(HELPER_BINARY)));
        assert_eq!(
            shipped.parent(),
            std::env::current_exe().expect("exe").parent()
        );
        assert!(unready(&missing).is_some());
        assert!(unready(&[]).is_none());
    }
}
