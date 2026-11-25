//! Bestehender LoadCredential-Bootstrap und vorhandener Uplink-UDS-Transport.
//! Ausschließlich der Broker-Token wird in RAM gehalten; Fehler enthalten nie
//! Credentialwerte, Antworttexte, URLs oder JSON-Parserauszüge.
use crate::Config;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path, time::Duration};
use zeroize::{Zeroize, Zeroizing};

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    secrets: Vec<Entry>,
    #[serde(default)]
    imports: Vec<Import>,
}
#[derive(Deserialize)]
struct Import {
    #[serde(default)]
    secrets: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "secretKey")]
    name: String,
    #[serde(rename = "secretValue")]
    value: String,
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

fn credential(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| anyhow::anyhow!("Bestehender Infisical-Credential ist nicht verfügbar."))?;
    if !file.metadata()?.is_file() {
        bail!("Infisical-Credential ist keine reguläre Quelle.");
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Infisical-Credential konnte nicht gelesen werden."))?;
    if bytes.is_empty() || bytes.len() > 8192 {
        bail!("Infisical-Credential hat eine ungültige Größe.");
    }
    Ok(bytes)
}

pub async fn broker_token(config: &Config, path: &Path) -> Result<Zeroizing<String>> {
    let credential = credential(path)?;
    let token = std::str::from_utf8(&credential)
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_whitespace))
        .context("Infisical-Credential ist ungültig.")?;
    let client = uplink_infisical_transport::client_builder(&config.infisical_socket, 0)
        .map_err(|_| anyhow::anyhow!("Geschützte Infisical-Gegenstelle ist nicht verfügbar."))?
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| anyhow::anyhow!("Infisical-Verbindung konnte nicht vorbereitet werden."))?;
    let mut response = client
        .get(format!(
            "{}/api/v4/secrets/",
            uplink_infisical_transport::BASE_URL
        ))
        .query(&[
            ("projectId", config.project_id.as_str()),
            ("environment", config.environment.as_str()),
            ("secretPath", config.secret_path.as_str()),
            ("viewSecretValue", "true"),
            ("includeImports", "true"),
            ("recursive", "false"),
        ])
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Infisical ist nicht erreichbar."))?;
    if !response.status().is_success() {
        bail!("Infisical hat den bestehenden Zugang abgelehnt.");
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Infisical-Antwort ist unvollständig."))?
    {
        if body.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
            bail!("Infisical-Antwort ist zu groß.");
        }
        body.extend_from_slice(&chunk);
    }
    let reply: Reply = serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("Infisical-Antwort ist ungültig."))?;
    let mut values = std::collections::HashMap::new();
    // Wie der vorhandene Dashboard-Leser: lokale Werte gewinnen vor Imports.
    for mut entry in reply
        .imports
        .into_iter()
        .flat_map(|import| import.secrets)
        .chain(reply.secrets)
    {
        if [
            "MASTER_BROKER_TOKEN",
            "MAIN_BOT_INTERNAL_TOKEN",
            "TWITCH_INTERNAL_API_TOKEN",
        ]
        .contains(&entry.name.as_str())
        {
            values.insert(
                entry.name.clone(),
                Zeroizing::new(std::mem::take(&mut entry.value)),
            );
        }
    }
    for name in [
        "MASTER_BROKER_TOKEN",
        "MAIN_BOT_INTERNAL_TOKEN",
        "TWITCH_INTERNAL_API_TOKEN",
    ] {
        if let Some(value) = values.remove(name).filter(|value| !value.trim().is_empty()) {
            return Ok(value);
        }
    }
    bail!("Bestehender Broker-Zugang fehlt in Infisical.")
}

pub async fn send(config: &Config, pending: &crate::state::Pending, token: &str) -> Result<()> {
    let mut token_header = reqwest::header::HeaderValue::from_str(token.trim())
        .map_err(|_| anyhow::anyhow!("Bestehender Broker-Zugang ist ungültig."))?;
    token_header.set_sensitive(true);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| anyhow::anyhow!("Broker-Verbindung konnte nicht vorbereitet werden."))?;
    let response = client
        .post(format!(
            "{}/internal/master/v1/discord/send-message",
            config.broker_origin
        ))
        .header("X-Internal-Token", token_header)
        .header("X-Idempotency-Key", &pending.key)
        .json(&serde_json::json!({ "channel_id": config.channel_id, "content": pending.content }))
        .send()
        .await
        .map_err(|_| {
            anyhow::anyhow!("Versand ist nicht bestätigt; der Meldezustand bleibt offen.")
        })?;
    if !response.status().is_success() {
        bail!("Broker hat die Meldung nicht bestätigt; der Meldezustand bleibt offen.");
    }
    // Der Broker bestätigt nur erfolgreiche Aktionen mit ok=true und Message-ID.
    let body = bounded_reply(response).await?;
    let reply: serde_json::Value = serde_json::from_slice(&body).map_err(|_| {
        anyhow::anyhow!("Broker-Bestätigung ist ungültig; der Meldezustand bleibt offen.")
    })?;
    if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        || reply
            .pointer("/result/message_id")
            .and_then(serde_json::Value::as_u64)
            .filter(|value| *value > 0)
            .is_none()
    {
        bail!("Broker hat den Nachrichtenversand nicht bestätigt; der Meldezustand bleibt offen.");
    }
    Ok(())
}

async fn bounded_reply(mut response: reqwest::Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("Broker-Bestätigung ist unvollständig."))?
    {
        if body.len().saturating_add(chunk.len()) > 32_768 {
            bail!("Broker-Bestätigung ist zu groß.");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};

    async fn mock(status: u16, body: &'static str) -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                let text = String::from_utf8_lossy(&bytes);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let length = text[..header_end]
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .unwrap();
                    if bytes.len() >= header_end + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(
                request.starts_with("POST /internal/master/v1/discord/send-message HTTP/1.1\r\n")
            );
            let lower = request.to_lowercase();
            assert!(lower.contains("\r\nx-internal-token: synthetic-token\r\n"));
            assert!(lower.contains("\r\nx-idempotency-key: stable-test-key\r\n"));
            assert!(request.contains("\"channel_id\":42"));
            let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).unwrap();
        });
        let config = Config {
            broker_origin: format!("http://{address}"),
            channel_id: 42,
            state_directory: "/unused".into(),
            legacy_state_directory: None,
            infisical_socket: "/unused".into(),
            project_id: "synthetic-project".into(),
            environment: "test".into(),
            secret_path: "/".into(),
            host_label: "test".into(),
        };
        let pending = crate::state::Pending {
            key: "stable-test-key".into(),
            content: "Sichere Testmeldung".into(),
            included: 1,
        };
        let result = send(&config, &pending, "synthetic-token").await;
        worker.join().unwrap();
        result
    }

    #[tokio::test]
    async fn actual_broker_contract_requires_success_and_real_message_id() {
        assert!(mock(200, r#"{"ok":true,"result":{"message_id":123}}"#)
            .await
            .is_ok());
        assert!(mock(200, r#"{"ok":true,"result":{}}"#).await.is_err());
        assert!(mock(200, r#"{"ok":false,"result":{"message_id":123}}"#)
            .await
            .is_err());
        assert!(mock(
            502,
            r#"{"ok":false,"error":{"message":"untrusted secret text"}}"#
        )
        .await
        .is_err());
    }
}
