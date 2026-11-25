//! Kein Rohlog verlässt den Host. Eine feste Ursachenliste verhindert, dass
//! Konfigurationsdumps, Zugangsdaten oder untrusted Logtexte zu Discord gelangen.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{collections::HashMap, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

pub async fn command(program: &str, args: &[&str], limit: usize) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Lokale Dienstdiagnose konnte nicht gestartet werden.")?;
    let stdout = child
        .stdout
        .take()
        .context("Lokale Dienstdiagnose fehlt.")?;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        stdout
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > limit {
            bail!("Lokale Dienstdiagnose überschreitet die Größenbegrenzung.");
        }
        let status = child.wait().await?;
        if !status.success() {
            bail!("Lokale Dienstdiagnose ist fehlgeschlagen.");
        }
        Ok(bytes)
    })
    .await
    .context("Lokale Dienstdiagnose hat nicht rechtzeitig geantwortet.")?;
    result
}

pub fn validate_invocation(id: &str) -> Result<&str> {
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Die fehlgeschlagene Dienstinstanz ist nicht eindeutig verfügbar.");
    }
    Ok(id)
}

pub async fn cause(unit: &str, invocation: &str) -> Result<&'static str> {
    let matcher = format!("_SYSTEMD_INVOCATION_ID={invocation}");
    // Der Invocation-Matcher verhindert Vermischung mit älteren Startversuchen.
    let bytes = command(
        "/usr/bin/journalctl",
        &[
            "--user",
            "--unit",
            unit,
            &matcher,
            "--output=json",
            "--no-pager",
            "--all",
            "-n",
            "160",
        ],
        2 * 1024 * 1024,
    )
    .await?;
    Ok(classify(&bytes))
}

pub fn classify(bytes: &[u8]) -> &'static str {
    let mut categories = HashMap::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some(message) = record.get("MESSAGE").and_then(Value::as_str) else {
            continue;
        };
        let message = message.trim();
        // JSON-/TOML-Konfiguration auch dann ausschließen, wenn sie Fehlerwörter
        // enthält. Von übrigen Texten werden ausschließlich feste Kategorien gesendet.
        if message.starts_with('{')
            || message.starts_with('[')
            || message.contains("\"schema_version\"")
            || message.contains("\"secret_profile\"")
        {
            continue;
        }
        let lower = message.to_lowercase();
        let matched = if lower.contains("preflight")
            && lower.contains("head")
            && lower.contains("origin/main")
            && lower.contains("!=")
        {
            Some((0, "Die Startprüfung hat den Dienststart abgebrochen, weil der lokale main-Stand vom Remote-Stand abweicht."))
        } else if lower.contains("preflight")
            && (lower.contains("mismatch")
                || lower.contains("abweich")
                || lower.contains("nicht überein"))
        {
            Some((0, "Die Startprüfung meldet eine Abweichung zwischen der Dienstkonfiguration und dem gestarteten Programm."))
        } else if lower.contains("konfiguration")
            && (lower.contains("ungültig")
                || lower.contains("fehlt")
                || lower.contains("nicht lesbar"))
        {
            Some((
                1,
                "Die Dienstkonfiguration fehlt, ist nicht lesbar oder enthält ungültige Werte.",
            ))
        } else if lower.contains("credential")
            && (lower.contains("fehlt")
                || lower.contains("failed")
                || lower.contains("ungültig")
                || lower.contains("nicht"))
        {
            Some((
                2,
                "Der bestehende Dienstzugang konnte beim Start nicht geladen werden.",
            ))
        } else if lower.contains("infisical")
            && (lower.contains("fehl")
                || lower.contains("nicht")
                || lower.contains("error")
                || lower.contains("refused"))
        {
            Some((
                3,
                "Der Dienst konnte seinen Zugang nicht aus Infisical laden.",
            ))
        } else if lower.contains("address already in use") || lower.contains("adresse bereits") {
            Some((
                4,
                "Die benötigte lokale Adresse ist bereits von einem anderen Prozess belegt.",
            ))
        } else if lower.contains("migration")
            && (lower.contains("fehl")
                || lower.contains("failed")
                || lower.contains("error")
                || lower.contains("checksum"))
        {
            Some((5, "Die Datenbankmigration ist fehlgeschlagen."))
        } else if (lower.contains("database")
            || lower.contains("postgres")
            || lower.contains("datenbank"))
            && (lower.contains("refused")
                || lower.contains("fehl")
                || lower.contains("failed")
                || lower.contains("error"))
        {
            Some((
                6,
                "Der Dienst konnte seine Datenbank nicht erreichen oder öffnen.",
            ))
        } else if lower.contains("permission denied") || lower.contains("zugriff verweigert") {
            Some((7, "Dem Dienst fehlt eine benötigte Dateiberechtigung."))
        } else if lower.contains("out of memory") || lower.contains("oom-kill") {
            Some((8, "Der Dienst wurde wegen Speichermangel beendet."))
        } else if lower.contains("panicked at") {
            Some((9, "Das Programm ist mit einem internen Fehler abgebrochen."))
        } else {
            None
        };
        if let Some((priority, text)) = matched {
            categories.insert(priority, text);
        }
    }
    categories.into_iter().min_by_key(|(priority, _)| *priority).map(|(_, text)| text)
        .unwrap_or("Im Journal dieser Dienstinstanz steht keine sicher weitergebbare Ursache. Das lokale Journal muss geprüft werden.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_cause_wins_over_dumps_and_generic_failure_without_leaking_secrets() {
        let log = [
            serde_json::json!({"MESSAGE":"preflight mismatch credential synthetic-secret-value"}),
            serde_json::json!({"MESSAGE":"{\"schema_version\":1,\"secret_profile\":\"panicked at\"}"}),
            serde_json::json!({"MESSAGE":"Main process exited, status=1/FAILURE"}),
        ].iter().map(|record| record.to_string()).collect::<Vec<_>>().join("\n");
        let result = classify(log.as_bytes());
        assert!(result.contains("Startprüfung"));
        assert!(!result.contains("synthetic-secret"));
        assert!(!result.contains("schema_version"));
    }
    #[test]
    fn unknown_and_json_only_logs_fail_closed() {
        let log = serde_json::json!({"MESSAGE":"{\"error\":\"infisical failed secret-value\"}"})
            .to_string();
        assert!(classify(log.as_bytes()).contains("keine sicher weitergebbare Ursache"));
    }

    #[test]
    fn deployed_steam_preflight_failure_is_reported_before_config_noise() {
        let mut records = vec![serde_json::json!({"MESSAGE":"deploy-preflight [steam-core]: FEHLER: HEAD synthetic-sha != origin/main other-sha — unpushter/ungeprüfter Commit auf main, lief nie durchs Merge-Gate"}).to_string()];
        for _ in 0..30 {
            records.push(serde_json::json!({"MESSAGE":"{\"schema_version\":1,\"anchor\":\"steam-global-toml-v1\",\"secret_profile\":\"primary\"}"}).to_string());
        }
        records.push(
            serde_json::json!({"MESSAGE":"Main process exited, status=1/FAILURE"}).to_string(),
        );
        let text = classify(records.join("\n").as_bytes());
        assert!(text.contains("lokale main-Stand vom Remote-Stand abweicht"));
        assert!(!text.contains("unpushter"));
        assert!(!text.contains("primary"));
    }
}
