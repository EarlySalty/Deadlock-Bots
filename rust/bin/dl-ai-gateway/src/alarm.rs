use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use dl_ai::tool_chat::{AlarmConfig, Delivery, Refusal, RefusalNotifier};

const GATEWAY_UNIT: &str = "dl-ai-gateway.service";
const CHILD_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

pub struct ProcessNotifier {
    config: AlarmConfig,
}

impl ProcessNotifier {
    pub fn new(config: AlarmConfig) -> Self {
        Self { config }
    }
}

impl RefusalNotifier for ProcessNotifier {
    fn notify(&self, refusal: Refusal, delivery: Delivery) {
        tracing::warn!(
            provider = %refusal.provider,
            kind = refusal.kind,
            reason = refusal.reason,
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
        let command = command(&self.config, &refusal);
        let budget = Duration::from_secs(self.config.notify_timeout_secs.get());
        runtime.spawn(run(command, budget, refusal, delivery));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Sent,
    Suppressed,
}

fn command(config: &AlarmConfig, refusal: &Refusal) -> tokio::process::Command {
    let event_id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_micros())
            .unwrap_or_default()
    );
    let mut command = tokio::process::Command::new(&config.notify_binary);
    command
        .env_clear()
        .env("PATH", CHILD_PATH)
        .env("LANG", "C.UTF-8")
        .arg("--config")
        .arg(&config.notify_config)
        .arg("--credential")
        .arg(&config.credential)
        .arg("--unit")
        .arg(GATEWAY_UNIT)
        .arg("--provider-refusal")
        .arg("--provider")
        .arg(&refusal.provider)
        .arg("--kind")
        .arg(refusal.kind)
        .arg("--reason")
        .arg(refusal.reason)
        .arg("--event-id")
        .arg(event_id);
    if let Some(home) = std::env::var_os("HOME") {
        command.env("HOME", home);
    }
    if let Some(status) = refusal.status {
        command.arg("--status").arg(status.to_string());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    command
}

async fn deliver(
    mut command: tokio::process::Command,
    budget: Duration,
) -> Result<Outcome, &'static str> {
    let child = command
        .spawn()
        .map_err(|_| "notify helper could not start")?;
    let output = tokio::time::timeout(budget, child.wait_with_output())
        .await
        .map_err(|_| "notify helper timed out")?
        .map_err(|_| "notify helper unreachable")?;
    if !output.status.success() {
        return Err("notify helper reported a failure");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .rev()
        .find_map(|line| match line.trim() {
            "outcome=sent" => Some(Outcome::Sent),
            "outcome=suppressed" => Some(Outcome::Suppressed),
            _ => None,
        })
        .ok_or("notify helper gave no outcome")
}

async fn run(
    command: tokio::process::Command,
    budget: Duration,
    refusal: Refusal,
    delivery: Delivery,
) {
    match deliver(command, budget).await {
        Ok(Outcome::Sent) => {
            tracing::info!(
                provider = %refusal.provider,
                reason = refusal.reason,
                "AI_GATEWAY_ALARM_SENT"
            );
            delivery.confirm();
        }
        Ok(Outcome::Suppressed) => {
            tracing::info!(
                provider = %refusal.provider,
                reason = refusal.reason,
                "AI_GATEWAY_ALARM_SUPPRESSED"
            );
            delivery.confirm();
        }
        Err(cause) => {
            tracing::error!(
                provider = %refusal.provider,
                reason = refusal.reason,
                cause,
                "AI_GATEWAY_ALARM_FAILED"
            );
            drop(delivery);
        }
    }
}

pub fn readiness(config: &AlarmConfig) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !executable(&config.notify_binary) {
        missing.push("notify_binary is not an executable file");
    }
    if std::fs::File::open(&config.notify_config).is_err() {
        missing.push("notify_config is not readable");
    }
    if std::fs::File::open(&config.credential).is_err() {
        missing.push("credential is not readable");
    }
    missing
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

pub fn report_readiness(config: &AlarmConfig) {
    for cause in readiness(config) {
        tracing::error!(cause, "AI_GATEWAY_ALARM_UNREADY");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::{NonZeroU32, NonZeroU64};

    fn config(binary: &str) -> AlarmConfig {
        AlarmConfig {
            notify_binary: binary.into(),
            notify_config: "/etc/hostname".into(),
            credential: "/etc/hostname".into(),
            refusal_streak: NonZeroU32::MIN,
            notify_timeout_secs: NonZeroU64::MIN,
        }
    }

    fn refusal() -> Refusal {
        Refusal {
            provider: "anthropic_api".into(),
            kind: "limit",
            reason: "billing",
            status: Some(400),
        }
    }

    fn script(name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("dl-ai-gateway-alarm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        path
    }

    #[test]
    fn helper_gets_unit_provider_kind_reason_status_and_no_inherited_environment() {
        let command = command(&config("/bin/notify"), &refusal());
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let position = |flag: &str| {
            args.iter()
                .position(|arg| arg == flag)
                .map(|index| args[index + 1].as_str())
        };
        assert_eq!(position("--unit"), Some(GATEWAY_UNIT));
        assert_eq!(position("--provider"), Some("anthropic_api"));
        assert_eq!(position("--kind"), Some("limit"));
        assert_eq!(position("--reason"), Some("billing"));
        assert_eq!(position("--status"), Some("400"));
        assert!(args.contains(&"--provider-refusal".to_owned()));
        assert!(!args.contains(&"--refusals".to_owned()));
        let envs: Vec<String> = command
            .as_std()
            .get_envs()
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        assert!(envs
            .iter()
            .all(|key| ["PATH", "LANG", "HOME"].contains(&key.as_str())));
    }

    #[tokio::test]
    async fn only_a_stated_outcome_counts_as_delivered() {
        let budget = Duration::from_secs(5);
        let sent = script("sent", "echo noise\necho outcome=sent");
        let suppressed = script("suppressed", "echo outcome=suppressed");
        let silent = script("silent", "exit 0");
        let failing = script("failing", "echo outcome=sent\nexit 1");
        let secret = script(
            "secret",
            "if [ -n \"$DL_GATEWAY_SECRET_PROBE\" ]; then exit 3; fi\necho outcome=sent",
        );
        let slow = script("slow", "sleep 5\necho outcome=sent");
        std::env::set_var("DL_GATEWAY_SECRET_PROBE", "1");
        let outcome =
            |path: &std::path::Path| command(&config(&path.to_string_lossy()), &refusal());
        assert_eq!(deliver(outcome(&sent), budget).await, Ok(Outcome::Sent));
        assert_eq!(
            deliver(outcome(&suppressed), budget).await,
            Ok(Outcome::Suppressed)
        );
        assert_eq!(
            deliver(outcome(&silent), budget).await,
            Err("notify helper gave no outcome")
        );
        assert_eq!(
            deliver(outcome(&failing), budget).await,
            Err("notify helper reported a failure")
        );
        assert_eq!(deliver(outcome(&secret), budget).await, Ok(Outcome::Sent));
        assert_eq!(
            deliver(outcome(&slow), Duration::from_millis(200)).await,
            Err("notify helper timed out")
        );
        assert_eq!(
            deliver(command(&config("/nonexistent/notify"), &refusal()), budget).await,
            Err("notify helper could not start")
        );
        assert_eq!(
            readiness(&config(&sent.to_string_lossy())),
            Vec::<&str>::new()
        );
        assert_eq!(
            readiness(&AlarmConfig {
                credential: "/nonexistent/credential".into(),
                ..config("/nonexistent/notify")
            }),
            vec![
                "notify_binary is not an executable file",
                "credential is not readable"
            ]
        );
        if let Some(dir) = sent.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
