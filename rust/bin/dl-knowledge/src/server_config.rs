//! Non-secret settings for the loopback knowledge service and shadow evaluation.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::path::PathBuf;
use std::{ffi::OsString, net::SocketAddr, path::Path};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HybridSettings {
    /// Omitted in production: use the existing central Infisical-backed connector.
    /// Explicit URLs are reserved for local peer-authenticated evaluation.
    pub database_url: Option<String>,
    pub embedding_model: PathBuf,
    #[serde(default)]
    pub embedding_profile: crate::dense::local::Profile,
    pub reranker_model: PathBuf,
    #[serde(default)]
    pub options: crate::hybrid::config::Config,
}

impl HybridSettings {
    pub fn validate(&self) -> Result<()> {
        if let Some(address) = &self.database_url {
            let url = url::Url::parse(address).context("Ungültige lokale Indexadresse")?;
            ensure!(
                matches!(url.scheme(), "postgres" | "postgresql")
                    && url.password().is_none()
                    && url.host_str().is_none()
                    && url
                        .query_pairs()
                        .all(|(key, value)| key == "host" && value.starts_with('/')),
                "Suchindex benötigt lokale Peer-Authentifizierung ohne Secrets"
            );
        }
        ensure!(
            self.embedding_model.is_absolute() && self.reranker_model.is_absolute(),
            "Suchmodelle benötigen absolute lokale Pfade"
        );
        self.options.validate()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerConfig {
    pub bind: SocketAddr,
    #[serde(default)]
    pub retrieval_only: bool,
    #[serde(default)]
    pub hybrid: Option<HybridSettings>,
}

impl ServerConfig {
    pub fn from_args(args: &[OsString]) -> Result<Option<Self>> {
        if args.is_empty() {
            return Ok(None);
        }
        ensure!(
            args.len() == 2 && args[0] == "--config",
            "Erwartet: --config <JSON-Datei>; alternative Korpuspfade sind nicht erlaubt"
        );
        let bytes = std::fs::read(Path::new(&args[1])).context("Dienstkonfiguration lesen")?;
        Ok(Some(Self::parse(&bytes)?))
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let config: Self =
            serde_json::from_slice(bytes).context("Dienstkonfiguration ist ungültig")?;
        ensure!(
            config.bind.ip().is_loopback(),
            "Bind-Adresse muss Loopback sein"
        );
        ensure!(config.bind.port() != 0, "Bind-Port darf nicht 0 sein");
        if let Some(settings) = &config.hybrid {
            settings.validate()?;
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_accepts_only_explicit_loopback_and_non_secret_fields() {
        let config = ServerConfig::parse(br#"{"bind":"127.0.0.1:18896","retrieval_only":true}"#)
            .expect("Gültige Testfixture erwartet");
        assert_eq!(config.bind.port(), 18896);
        assert!(config.retrieval_only);
        assert!(ServerConfig::parse(br#"{"bind":"[::1]:18896"}"#).is_ok());
        for raw in [
            r#"{"bind":"0.0.0.0:18896"}"#,
            r#"{"bind":"192.0.2.1:18896"}"#,
            r#"{"bind":"127.0.0.1:0"}"#,
            r#"{"bind":"127.0.0.1:18896","corpus":"internal"}"#,
            r#"{"bind":"127.0.0.1:18896","api_key":"not-allowed"}"#,
        ] {
            assert!(ServerConfig::parse(raw.as_bytes()).is_err());
        }
    }

    #[test]
    fn config_cli_keeps_the_default_and_rejects_corpus_overrides() {
        assert!(ServerConfig::from_args(&[])
            .expect("Gültige Testfixture erwartet")
            .is_none());
        assert!(ServerConfig::from_args(&[OsString::from("/tmp/public")]).is_err());
        assert!(ServerConfig::from_args(&[OsString::from("--config")]).is_err());
    }

    #[test]
    fn hybrid_config_allows_only_explicit_local_peer_index() {
        for (address, accepted) in [
            (
                "postgresql:///dl_knowledge_eval_fixture?host=/var/run/postgresql",
                true,
            ),
            ("postgresql://remote.example/knowledge", false),
            ("postgresql://user:password@localhost/knowledge", false),
            ("postgresql:///knowledge?password=hidden", false),
            ("postgresql:///knowledge?host=remote.example", false),
        ] {
            let raw = serde_json::json!({"bind":"127.0.0.1:18896","hybrid":{
                "database_url":address,"embedding_model":"/models/embed","reranker_model":"/models/rerank"
            }});
            assert_eq!(
                ServerConfig::parse(raw.to_string().as_bytes()).is_ok(),
                accepted
            );
        }
    }
}
