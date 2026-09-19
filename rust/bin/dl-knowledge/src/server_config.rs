//! Non-secret settings for a loopback retrieval shadow instance.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{ffi::OsString, net::SocketAddr, path::Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerConfig {
    pub bind: SocketAddr,
    #[serde(default)]
    pub retrieval_only: bool,
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
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_accepts_only_explicit_loopback_and_non_secret_fields() {
        let config =
            ServerConfig::parse(br#"{"bind":"127.0.0.1:18896","retrieval_only":true}"#).unwrap();
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
        assert!(ServerConfig::from_args(&[]).unwrap().is_none());
        assert!(ServerConfig::from_args(&[OsString::from("/tmp/public")]).is_err());
        assert!(ServerConfig::from_args(&[OsString::from("--config")]).is_err());
    }
}
