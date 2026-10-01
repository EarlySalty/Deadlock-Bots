use anyhow::{anyhow, Result};
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use serde::Deserialize;
use std::io::Read;
use std::os::unix::fs::FileTypeExt;
use std::{
    collections::BTreeMap,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    secret_values_fd: Option<i32>,
    project_id: String,
    environment: String,
    secret_path: String,
    #[serde(default)]
    credential_fd: Option<i32>,
    #[serde(default = "default_credential_name", rename = "credential_name")]
    _credential_name: String,
    #[serde(default)]
    credential_path: Option<PathBuf>,
    socket_path: PathBuf,
    #[serde(default, rename = "database_secret")]
    _database_secret: String,
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
    value: String,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

fn default_credential_name() -> String {
    "infisical-token".to_string()
}

pub async fn values(path: &Path) -> Result<Vec<(String, Zeroizing<String>)>> {
    let config = load_config(path)?;
    values_with_config(config).await
}

/// Trusted launchers parse the already-opened normal config, preventing races.
pub async fn values_from_config(bytes: &[u8]) -> Result<Vec<(String, Zeroizing<String>)>> {
    let config: Config = serde_json::from_slice(bytes)
        .map_err(|_| anyhow!("Infisical Konfiguration ist ungültig."))?;
    values_with_config(config).await
}

async fn values_with_config(config: Config) -> Result<Vec<(String, Zeroizing<String>)>> {
    if let Some(fd) = config.secret_values_fd {
        return pipe_values(fd);
    }
    let values = fetch_values(config).await?;
    Ok(values.into_iter().collect())
}

fn pipe_values(fd: i32) -> Result<Vec<(String, Zeroizing<String>)>> {
    if fd < 3 {
        return Err(anyhow!("Private Secret-Pipe fehlt."));
    }
    let descriptor = filedescriptor::FileDescriptor::dup(&fd)
        .map_err(|_| anyhow!("Private Secret-Pipe ist nicht verfügbar."))?;
    let mut file = descriptor
        .as_file()
        .map_err(|_| anyhow!("Private Secret-Pipe ist nicht lesbar."))?;
    if !file
        .metadata()
        .map_err(|_| anyhow!("Private Secret-Pipe ist nicht prüfbar."))?
        .file_type()
        .is_fifo()
    {
        return Err(anyhow!("Secretquelle muss eine private Pipe sein."));
    }
    let flags =
        fcntl(fd, FcntlArg::F_GETFD).map_err(|_| anyhow!("Secret-Pipe ist nicht verfügbar."))?;
    fcntl(
        fd,
        FcntlArg::F_SETFD(FdFlag::from_bits_retain(flags) | FdFlag::FD_CLOEXEC),
    )
    .map_err(|_| anyhow!("Secret-Pipe konnte nicht geschützt werden."))?;
    let mut body = Zeroizing::new(Vec::new());
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        use nix::poll::{poll, PollFd, PollFlags};
        use std::os::fd::AsFd;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let timeout = u16::try_from(remaining.as_millis()).unwrap_or(u16::MAX);
        let mut descriptors = [PollFd::new(file.as_fd(), PollFlags::POLLIN)];
        if timeout == 0
            || poll(&mut descriptors, timeout)
                .map_err(|_| anyhow!("Secret-Pipe wurde unterbrochen."))?
                == 0
        {
            return Err(anyhow!("Secret-Pipe antwortet nicht rechtzeitig."));
        }
        let mut buffer = Zeroizing::new([0u8; 65536]);
        let length = file
            .read(buffer.as_mut())
            .map_err(|_| anyhow!("Secret-Pipe wurde unterbrochen."))?;
        if length == 0 {
            break;
        }
        if body.len().saturating_add(length) > 2 * 1024 * 1024 {
            return Err(anyhow!("Secret-Pipe ist zu groß."));
        }
        body.extend_from_slice(&buffer[..length]);
    }
    let mut values: BTreeMap<String, String> =
        serde_json::from_slice(&body).map_err(|_| anyhow!("Secret-Pipe ist ungültig."))?;
    let result = values
        .iter_mut()
        .map(|(name, value)| (name.clone(), Zeroizing::new(std::mem::take(value))))
        .collect();
    Ok(result)
}

fn load_config(path: &Path) -> Result<Config> {
    let data =
        std::fs::read(path).map_err(|_| anyhow!("Infisical Konfiguration ist nicht lesbar."))?;
    serde_json::from_slice(&data).map_err(|_| anyhow!("Infisical Konfiguration ist ungültig."))
}

async fn fetch_values(config: Config) -> Result<BTreeMap<String, Zeroizing<String>>> {
    let token = load_credential(&config)?;
    if token.is_empty() || token.len() > 8192 {
        return Err(anyhow!("Infisical Credential hat eine ungültige Größe."));
    }
    let token_text = std::str::from_utf8(&token)
        .map_err(|_| anyhow!("Infisical Credential ist ungültig."))?
        .trim();

    let client = uplink_infisical_transport::client_builder(&config.socket_path, 0)
        .map_err(|message| anyhow!(message))?
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| anyhow!("Infisical Client konnte nicht gestartet werden."))?;
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
        .bearer_auth(token_text)
        .send()
        .await
        .map_err(|_| anyhow!("Infisical ist nicht erreichbar."))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Infisical Zugriff fehlgeschlagen (HTTP {}).",
            response.status().as_u16()
        ));
    }

    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Infisical Antwort wurde unterbrochen."))?
    {
        if body.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
            return Err(anyhow!("Infisical Antwort ist zu groß."));
        }
        body.extend_from_slice(&chunk);
    }

    let reply: Reply =
        serde_json::from_slice(&body).map_err(|_| anyhow!("Infisical Antwort ist ungültig."))?;
    let mut values = BTreeMap::new();
    for mut entry in reply
        .imports
        .into_iter()
        .flat_map(|import| import.secrets)
        .chain(reply.secrets)
    {
        let name = entry.name.trim();
        if name.is_empty() {
            continue;
        }
        if !valid_environment_name(name) {
            return Err(anyhow!(
                "Infisical enthält einen ungültigen Variablennamen."
            ));
        }
        values.insert(
            name.to_string(),
            Zeroizing::new(std::mem::take(&mut entry.value)),
        );
    }
    Ok(values)
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn load_credential(config: &Config) -> Result<Zeroizing<Vec<u8>>> {
    if let Some(fd) = config.credential_fd.filter(|fd| *fd >= 3) {
        if let Ok(flags) = fcntl(fd, FcntlArg::F_GETFD) {
            fcntl(
                fd,
                FcntlArg::F_SETFD(FdFlag::from_bits_retain(flags) | FdFlag::FD_CLOEXEC),
            )
            .map_err(|_| anyhow!("Infisical Credential FD konnte nicht geschützt werden."))?;
            let descriptor = filedescriptor::FileDescriptor::dup(&fd)
                .map_err(|_| anyhow!("Infisical Credential FD ist nicht verfügbar."))?;
            let file = descriptor
                .as_file()
                .map_err(|_| anyhow!("Infisical Credential FD ist nicht lesbar."))?;
            return read_credential(&file);
        }
    }

    if let Some(path) = &config.credential_path {
        if path.is_absolute() {
            return read_credential_path(path);
        }
    }

    Err(anyhow!(
        "Infisical Credential ist weder als FD noch als Runtime Credential verfügbar."
    ))
}

fn read_credential_path(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| anyhow!("Infisical Runtime Credential ist nicht lesbar."))?;
    read_credential(&file)
}

fn read_credential(file: &std::fs::File) -> Result<Zeroizing<Vec<u8>>> {
    if !file
        .metadata()
        .map_err(|_| anyhow!("Infisical Credential konnte nicht geprüft werden."))?
        .is_file()
    {
        return Err(anyhow!(
            "Infisical benötigt ein reguläres Runtime Credential."
        ));
    }
    let mut bytes = Zeroizing::new(vec![0; 8193]);
    let mut length = 0;
    while length < bytes.len() {
        let read = file
            .read_at(&mut bytes[length..], length as u64)
            .map_err(|_| anyhow!("Infisical Credential ist nicht lesbar."))?;
        if read == 0 {
            break;
        }
        length += read;
    }
    bytes.truncate(length);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    #[test]
    fn credential_reads_preserve_offset_and_allow_repeated_pool_setup() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"synthetic-fixture").unwrap();
        let offset = file.stream_position().unwrap();
        assert_eq!(
            read_credential(&file).unwrap().as_slice(),
            b"synthetic-fixture"
        );
        assert_eq!(
            read_credential(&file).unwrap().as_slice(),
            b"synthetic-fixture"
        );
        assert_eq!(file.stream_position().unwrap(), offset);
    }

    #[test]
    fn environment_names_are_strict() {
        assert!(valid_environment_name("DEADLOCK_CENTRAL_DSN"));
        assert!(valid_environment_name("_TOKEN_2"));
        assert!(!valid_environment_name("2TOKEN"));
        assert!(!valid_environment_name("BAD-NAME"));
        assert!(!valid_environment_name(""));
    }
}
