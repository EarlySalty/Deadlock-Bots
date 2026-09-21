use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, ffi::OsString, process::Command, time::Duration};
use url::Url;

pub const INFISICAL_BASE_URL: &str = "http://127.0.0.1:8080";

const KNOWLEDGE_KEYS: &[&str] = &[
    "DEADLOCK_CENTRAL_DSN",
    "DL_LLM_MODEL_BOT_PATE",
    "FIREWORK_API_KEY",
    "FIREWORK_BASE_URL",
    "FIREWORK_MODEL",
    "FIREWORKS_API_KEY",
    "FIREWORKS_BASE_URL",
    "FIREWORKS_MODEL",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Profile {
    All,
    Knowledge,
}

pub struct Config {
    pub project_id: String,
    pub environment: String,
    pub service_token: String,
    pub secret_path: String,
    pub timeout: Duration,
    pub retry_delay: Duration,
    pub max_attempts: u64,
}

#[derive(Deserialize)]
struct SecretResponse {
    #[serde(default)]
    secrets: Vec<SecretEntry>,
    #[serde(default)]
    imports: Vec<ImportEntry>,
}

#[derive(Deserialize)]
struct ImportEntry {
    #[serde(default)]
    secrets: Vec<SecretEntry>,
}

#[derive(Deserialize)]
struct SecretEntry {
    #[serde(rename = "secretKey")]
    key: String,
    #[serde(rename = "secretValue")]
    value: Option<String>,
}

pub fn config_from_env() -> Result<Config> {
    if let Ok(input) = std::env::var("INFISICAL_API_URL") {
        validated_base_url(&input)?;
    }
    let timeout = std::env::var("INFISICAL_HTTP_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(10));
    let retry_delay = std::env::var("INFISICAL_RETRY_DELAY")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(5));
    let max_attempts = std::env::var("INFISICAL_MAX_ATTEMPTS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    Ok(Config {
        project_id: required_env("INFISICAL_PROJECT_ID")?,
        environment: required_env("INFISICAL_ENV")?,
        service_token: required_env("INFISICAL_SERVICE_TOKEN")?,
        secret_path: std::env::var("INFISICAL_SECRET_PATH").unwrap_or_else(|_| "/".to_owned()),
        timeout,
        retry_delay,
        max_attempts,
    })
}

pub fn validated_base_url(input: &str) -> Result<Url> {
    let url = Url::parse(input).context("invalid Infisical URL")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port_or_known_default() != Some(8080)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(anyhow!("Infisical URL must be {INFISICAL_BASE_URL}"));
    }
    Ok(url)
}

pub fn secrets_url(base_url: &Url, config: &Config) -> Result<Url> {
    let mut url = base_url
        .join("/api/v4/secrets/")
        .context("failed to build Infisical secrets URL")?;
    url.query_pairs_mut()
        .append_pair("projectId", &config.project_id)
        .append_pair("environment", &config.environment)
        .append_pair("secretPath", &config.secret_path)
        .append_pair("viewSecretValue", "true")
        .append_pair("includeImports", "true")
        .append_pair("recursive", "false");
    Ok(url)
}

pub fn parse_secret_response(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let response: SecretResponse =
        serde_json::from_slice(bytes).context("Infisical response has unexpected shape")?;
    let mut values = BTreeMap::new();
    insert_entries(&mut values, response.secrets);
    for import in response.imports {
        insert_entries(&mut values, import.secrets);
    }
    Ok(values)
}

pub async fn fetch_secrets(
    client: &reqwest::Client,
    base_url: &Url,
    config: &Config,
) -> Result<BTreeMap<String, String>> {
    let url = secrets_url(base_url, config)?;
    let response = client
        .get(url)
        .bearer_auth(&config.service_token)
        .timeout(config.timeout)
        .send()
        .await
        .map_err(|_| anyhow!("Infisical request failed"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Infisical request failed with {}",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| anyhow!("Infisical response read failed"))?;
    parse_secret_response(&bytes)
}

pub async fn fetch_secrets_with_retry(
    client: &reqwest::Client,
    base_url: &Url,
    config: &Config,
) -> Result<BTreeMap<String, String>> {
    let mut attempt = 0;
    loop {
        match fetch_secrets(client, base_url, config).await {
            Ok(secrets) => return Ok(secrets),
            Err(error) => {
                attempt += 1;
                if config.max_attempts > 0 && attempt >= config.max_attempts {
                    return Err(error).context(format!(
                        "Infisical secrets could not be loaded after {attempt} attempt(s)"
                    ));
                }
                eprintln!(
                    "Infisical not ready, retrying in {}s (attempt {attempt}).",
                    config.retry_delay.as_secs()
                );
                tokio::time::sleep(config.retry_delay).await;
            }
        }
    }
}

pub fn selected_secrets(
    profile: Profile,
    fetched: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    match profile {
        Profile::All => fetched.clone(),
        Profile::Knowledge => KNOWLEDGE_KEYS
            .iter()
            .filter_map(|key| {
                fetched
                    .get(*key)
                    .map(|value| ((*key).to_owned(), value.clone()))
            })
            .collect(),
    }
}

pub fn command_with_secrets(
    profile: Profile,
    fetched: &BTreeMap<String, String>,
    argv: &[OsString],
) -> Result<Command> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| anyhow!("missing child command"))?;
    let mut command = Command::new(program);
    command.args(args);
    for key in fetched.keys() {
        command.env_remove(key);
    }
    for (key, value) in selected_secrets(profile, fetched) {
        command.env(key, value);
    }
    command.env_remove("INFISICAL_SERVICE_TOKEN");
    Ok(command)
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} is required"))
}

fn insert_entries(values: &mut BTreeMap<String, String>, entries: Vec<SecretEntry>) {
    for entry in entries {
        if let Some(value) = entry.value {
            if entry.key == "MINIMAX_TOKEN_PLAN_KEY" {
                values.insert("MINIMAX_API_KEY".to_owned(), value.clone());
            }
            values.insert(entry.key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_config() -> Config {
        Config {
            project_id: "project-test".to_owned(),
            environment: "prod".to_owned(),
            service_token: "synthetic-bearer".to_owned(),
            secret_path: "/".to_owned(),
            timeout: Duration::from_secs(10),
            retry_delay: Duration::from_secs(5),
            max_attempts: 0,
        }
    }

    #[test]
    fn rejects_any_non_loopback_url() {
        for candidate in [
            "https://127.0.0.1:8080",
            "http://localhost:8080",
            "http://127.0.0.1:8081",
            "http://user@127.0.0.1:8080",
            "http://127.0.0.1:8080/extra",
        ] {
            assert!(
                validated_base_url(candidate).is_err(),
                "accepted {candidate}"
            );
        }
        assert_eq!(
            validated_base_url(INFISICAL_BASE_URL).unwrap().as_str(),
            "http://127.0.0.1:8080/"
        );
    }

    #[test]
    fn builds_v4_query_with_imports_and_non_recursive_mode() {
        let url = secrets_url(
            &validated_base_url(INFISICAL_BASE_URL).unwrap(),
            &fixture_config(),
        )
        .unwrap();
        let pairs: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.path(), "/api/v4/secrets/");
        assert_eq!(
            pairs.get("projectId").map(String::as_str),
            Some("project-test")
        );
        assert_eq!(pairs.get("environment").map(String::as_str), Some("prod"));
        assert_eq!(pairs.get("secretPath").map(String::as_str), Some("/"));
        assert_eq!(
            pairs.get("viewSecretValue").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            pairs.get("includeImports").map(String::as_str),
            Some("true")
        );
        assert_eq!(pairs.get("recursive").map(String::as_str), Some("false"));
    }

    #[test]
    fn merges_imports_after_top_level_secrets_and_aliases_minimax() {
        let json = br#"{"secrets":[{"secretKey":"A","secretValue":"top"},{"secretKey":"MINIMAX_TOKEN_PLAN_KEY","secretValue":"plan"}],"imports":[{"secrets":[{"secretKey":"A","secretValue":"import"}]}]}"#;
        let values = parse_secret_response(json).unwrap();
        assert_eq!(values.get("A").map(String::as_str), Some("import"));
        assert_eq!(
            values.get("MINIMAX_API_KEY").map(String::as_str),
            Some("plan")
        );
    }

    #[test]
    fn knowledge_profile_keeps_only_allowed_names() {
        let fetched = BTreeMap::from([
            ("FIREWORK_MODEL".to_owned(), "model".to_owned()),
            ("DEADLOCK_CENTRAL_DSN".to_owned(), "test-dsn".to_owned()),
            ("UNRELATED_SECRET".to_owned(), "hidden".to_owned()),
        ]);
        assert_eq!(
            selected_secrets(Profile::Knowledge, &fetched),
            BTreeMap::from([
                ("FIREWORK_MODEL".to_owned(), "model".to_owned()),
                ("DEADLOCK_CENTRAL_DSN".to_owned(), "test-dsn".to_owned()),
            ]),
        );
        let command = command_with_secrets(
            Profile::Knowledge,
            &fetched,
            &[OsString::from("/usr/bin/true")],
        )
        .unwrap();
        let changes: BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
            .collect();
        assert_eq!(
            changes.get(std::ffi::OsStr::new("UNRELATED_SECRET")),
            Some(&None)
        );
        assert_eq!(
            changes
                .get(std::ffi::OsStr::new("FIREWORK_MODEL"))
                .and_then(|value| value.as_deref()),
            Some(std::ffi::OsStr::new("model"))
        );
    }

    #[test]
    fn child_environment_removes_bootstrap_and_preserves_arguments() {
        let fetched = BTreeMap::from([("FAKE_SECRET".to_owned(), "synthetic".to_owned())]);
        let argv = [
            OsString::from("/usr/bin/printf"),
            OsString::from("%s"),
            OsString::from("argument"),
        ];
        let command = command_with_secrets(Profile::All, &fetched, &argv).unwrap();
        assert_eq!(command.get_program(), "/usr/bin/printf");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec!["%s", "argument"]
        );
        let changes: BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
            .collect();
        assert_eq!(
            changes
                .get(std::ffi::OsStr::new("FAKE_SECRET"))
                .and_then(|value| value.as_deref()),
            Some(std::ffi::OsStr::new("synthetic"))
        );
        assert_eq!(
            changes.get(std::ffi::OsStr::new("INFISICAL_SERVICE_TOKEN")),
            Some(&None)
        );
    }
}
