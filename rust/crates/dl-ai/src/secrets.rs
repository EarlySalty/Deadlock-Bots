//! Central in-memory secret access through the existing protected Infisical bridge.
use serde::Deserialize;
use std::{
    fs::File,
    io::Read,
    os::fd::FromRawFd,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, thiserror::Error)]
pub enum SecretLoadError {
    #[error("Wissensrouter-Konfiguration fehlt oder ist ungültig")]
    Config,
    #[error("Infisical-Credential-FD fehlt oder ist ungültig")]
    Credential,
    #[error("Infisical-Gegenstelle ist nicht vertrauenswürdig")]
    Transport,
    #[error("Infisical-Zugriff fehlgeschlagen")]
    Request,
    #[error("Infisical-Antwort ist ungültig")]
    Response,
    #[error("OpenRouter-Zugang fehlt in Infisical")]
    Missing,
    #[error("Wissensrouter konnte nicht gestartet werden")]
    Router,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    enabled: bool,
    project_id: String,
    environment: String,
    secret_path: String,
    credential_fd: i32,
    socket_path: PathBuf,
}

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
    value: Option<String>,
}
impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(value) = &mut self.value {
            value.zeroize();
        }
    }
}

fn validate_socket(path: &Path) -> Result<(), SecretLoadError> {
    // Same fixed bridge and root-owned path contract as uplink-infisical-transport.
    if path != Path::new("/run/uplink-infisical/api.sock") {
        return Err(SecretLoadError::Transport);
    }
    for parent in [Path::new("/run"), Path::new("/run/uplink-infisical")] {
        let metadata = std::fs::symlink_metadata(parent).map_err(|_| SecretLoadError::Transport)?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(SecretLoadError::Transport);
        }
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| SecretLoadError::Transport)?;
    if !metadata.file_type().is_socket() || metadata.uid() != 0 || metadata.mode() & 0o007 != 0 {
        return Err(SecretLoadError::Transport);
    }
    Ok(())
}

pub struct InheritedKnowledgeCredential(File);

/// Takes the launcher's reserved descriptor before the runtime or any I/O starts.
///
/// # Safety
/// Call exactly once as the first operation of the synchronous process entrypoint.
/// FD9 must be absent or exclusively supplied by the documented service launcher.
pub unsafe fn take_knowledge_credential() -> Option<InheritedKnowledgeCredential> {
    let fd = 9;
    // SAFETY: guaranteed by the process entrypoint contract above.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return None;
    }
    // SAFETY: this startup API consumes the exclusively reserved inherited FD9.
    let file = unsafe { File::from_raw_fd(fd) };
    // SAFETY: the descriptor is now exclusively owned by `file`.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return None;
    }
    Some(InheritedKnowledgeCredential(file))
}

fn credential(
    inherited: InheritedKnowledgeCredential,
) -> Result<Zeroizing<String>, SecretLoadError> {
    let file = inherited.0;
    if !file
        .metadata()
        .map_err(|_| SecretLoadError::Credential)?
        .is_file()
    {
        return Err(SecretLoadError::Credential);
    }
    let mut value = Zeroizing::new(String::new());
    file.take(8193)
        .read_to_string(&mut value)
        .map_err(|_| SecretLoadError::Credential)?;
    if value.trim().is_empty() || value.len() > 8192 {
        return Err(SecretLoadError::Credential);
    }
    Ok(value)
}

/// Call once during process startup, before spawning child processes. Consumes FD9.
/// No secret, response body, or transport error is exposed through this API.
pub async fn load_knowledge_router(
    path: &Path,
    inherited: Option<InheritedKnowledgeCredential>,
) -> Result<Option<Arc<dyn crate::KnowledgeRouter>>, SecretLoadError> {
    Ok(load_knowledge_services(path, inherited)
        .await?
        .map(|services| services.router))
}

pub struct KnowledgeServices {
    pub router: Arc<dyn crate::KnowledgeRouter>,
    pub generator: Option<Arc<dyn crate::ChatProvider>>,
}

/// Uses the same credential/transport for read-only end-to-end measurements.
pub async fn load_knowledge_services(
    path: &Path,
    inherited: Option<InheritedKnowledgeCredential>,
) -> Result<Option<KnowledgeServices>, SecretLoadError> {
    let file = File::open(path).map_err(|_| SecretLoadError::Config)?;
    // The owned CLOEXEC handle is dropped on every early return, including bad config.
    let config: Config =
        serde_json::from_reader(file.take(16385)).map_err(|_| SecretLoadError::Config)?;
    if config.credential_fd != 9 {
        return Err(SecretLoadError::Config);
    }
    if !config.enabled {
        return Ok(None);
    }
    let token = credential(inherited.ok_or(SecretLoadError::Credential)?)?;
    if config.project_id.is_empty()
        || config.environment.is_empty()
        || !config.secret_path.starts_with('/')
    {
        return Err(SecretLoadError::Config);
    }
    validate_socket(&config.socket_path)?;
    let client = reqwest::Client::builder()
        .unix_socket(config.socket_path)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| SecretLoadError::Transport)?;
    let mut response = client
        .get("http://infisical.local/api/v4/secrets/")
        .query(&[
            ("projectId", config.project_id.as_str()),
            ("environment", config.environment.as_str()),
            ("secretPath", config.secret_path.as_str()),
            ("viewSecretValue", "true"),
            ("includeImports", "true"),
            ("recursive", "false"),
        ])
        .bearer_auth(token.trim())
        .send()
        .await
        .map_err(|_| SecretLoadError::Request)?;
    if !response.status().is_success() {
        return Err(SecretLoadError::Request);
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| SecretLoadError::Response)?
    {
        if body.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
            return Err(SecretLoadError::Response);
        }
        body.extend_from_slice(&chunk);
    }
    let reply: Reply = serde_json::from_slice(&body).map_err(|_| SecretLoadError::Response)?;
    let mut key = None;
    let mut generator_key = None;
    for mut entry in reply
        .secrets
        .into_iter()
        .chain(reply.imports.into_iter().flat_map(|i| i.secrets))
    {
        if entry.name == "OPENROUTER_API_KEY" {
            key = entry
                .value
                .take()
                .filter(|value| !value.trim().is_empty())
                .map(Zeroizing::new);
        }
        if matches!(
            entry.name.as_str(),
            "FIREWORK_API_KEY" | "FIREWORKS_API_KEY"
        ) {
            if let Some(value) = entry.value.take().filter(|value| !value.trim().is_empty()) {
                generator_key = Some(Zeroizing::new(value));
            }
        }
    }
    let mut key = key.ok_or(SecretLoadError::Missing)?;
    let router = crate::JevKnowledgeRouter::new(std::mem::take(&mut *key))
        .map_err(|_| SecretLoadError::Router)?;
    let generator = generator_key
        .and_then(|key| {
            crate::OpenAiChatProvider::from_fireworks_env(
                |name| (name == "FIREWORKS_API_KEY").then(|| key.to_string()),
                crate::RetryConfig::default(),
            )
            .ok()
        })
        .map(|provider| provider as Arc<dyn crate::ChatProvider>);
    Ok(Some(KnowledgeServices {
        router: Arc::new(router),
        generator,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn invalid_or_missing_config_closes_owned_credential() {
        use std::os::fd::AsRawFd;
        for path in [
            "/this-config-does-not-exist/knowledge.json",
            "/proc/self/exe",
        ] {
            let file = File::open("/dev/null").expect("fixture");
            let fd = file.as_raw_fd();
            assert!(load_knowledge_router(
                Path::new(path),
                Some(InheritedKnowledgeCredential(file))
            )
            .await
            .is_err());
            // SAFETY: inspection only, ownership was transferred to the startup loader.
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
        }
    }
    #[test]
    fn rejects_untrusted_socket_paths_and_reserved_descriptors() {
        for path in [
            "/tmp/api.sock",
            "/run/uplink-infisical/../api.sock",
            "api.sock",
        ] {
            assert!(validate_socket(Path::new(path)).is_err());
        }
    }
    #[test]
    fn malformed_config_and_null_secrets_are_safe() {
        assert!(
            serde_json::from_str::<Config>(r#"{"enabled":true,"api_key":"forbidden"}"#).is_err()
        );
        let reply: Reply = serde_json::from_str(
            r#"{"secrets":[{"secretKey":"OPENROUTER_API_KEY","secretValue":null}]}"#,
        )
        .expect("fixture");
        assert!(reply.secrets[0].value.is_none());
    }
}
