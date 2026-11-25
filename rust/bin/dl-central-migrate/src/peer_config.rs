//! Explicit local peer options using the existing normal JSON database_url format.
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use std::{ffi::OsString, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    database_url: String,
}

pub fn from_args(args: &[OsString]) -> Result<Option<PgConnectOptions>> {
    if args.is_empty() {
        return Ok(None);
    }
    ensure!(
        args.len() == 2 && args[0] == "--config",
        "Erwartet: --config <JSON-Datei>"
    );
    let bytes = std::fs::read(Path::new(&args[1])).context("Migrationskonfiguration lesen")?;
    Ok(Some(parse(&bytes)?))
}

fn parse(bytes: &[u8]) -> Result<PgConnectOptions> {
    // Never include configuration contents or a connection address in errors.
    let config: Config = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("Ungültige Migrationskonfiguration"))?;
    let address = url::Url::parse(&config.database_url)
        .map_err(|_| anyhow::anyhow!("Ungültige lokale Datenbankadresse"))?;
    let database = address.path().strip_prefix('/').unwrap_or_default();
    let identifier =
        |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    ensure!(
        matches!(address.scheme(), "postgres" | "postgresql")
            && matches!(address.host_str(), None | Some("localhost"))
            && address.password().is_none()
            && address.port().is_none()
            && address.fragment().is_none()
            && identifier(address.username())
            && identifier(database),
        "Migration benötigt eine explizite lokale Peer-Identität ohne Secrets"
    );
    let pairs: Vec<_> = address.query_pairs().collect();
    let [(key, socket)] = pairs.as_slice() else {
        bail!("Migration benötigt genau einen lokalen Socketpfad");
    };
    ensure!(
        key == "host" && Path::new(socket.as_ref()).is_absolute(),
        "Migration benötigt einen absoluten lokalen Socketpfad"
    );
    Ok(PgConnectOptions::new_without_pgpass()
        .host(socket)
        .port(5432)
        .username(address.username())
        .password("")
        .database(database)
        .ssl_mode(PgSslMode::Disable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_explicit_peer_and_rejects_credentials_or_remote_options() {
        let options = parse(br#"{"database_url":"postgresql://nathanael@localhost/scratch?host=/var/run/postgresql"}"#).unwrap();
        assert_eq!(options.get_username(), "nathanael");
        assert_eq!(options.get_database(), Some("scratch"));
        for address in [
            "postgresql://user:secret@localhost/scratch?host=/var/run/postgresql",
            "postgresql://user@remote/scratch?host=/var/run/postgresql",
            "postgresql://user@localhost/scratch",
            "postgresql://user@localhost/scratch?host=/var/run/postgresql&password=secret",
            "postgresql://user@localhost/scratch?host=/var/run/postgresql&host=/tmp",
            "postgresql:///scratch?host=/var/run/postgresql",
        ] {
            let bytes = serde_json::to_vec(&serde_json::json!({"database_url": address})).unwrap();
            let error = parse(&bytes).unwrap_err().to_string();
            assert!(!error.contains(address));
            assert!(!error.contains("secret"));
        }
        assert!(from_args(&[OsString::from("--unknown")]).is_err());
    }
}
