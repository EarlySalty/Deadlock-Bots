use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const EMPTY_DIGEST_PREFIX: &str = "e3b0c44298fc";

#[derive(Parser, Debug)]
struct Args {
    #[arg(long)]
    knowledge_url: String,
    #[arg(long)]
    base: PathBuf,
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Health {
    #[serde(default)]
    chunks: u64,
}

fn healthz(base_url: &str) -> Result<Health> {
    let url = format!("{}/healthz", base_url.trim_end_matches('/'));
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("HTTP-Client")?
        .get(&url)
        .send()
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    let body = response.text().context("healthz Body")?;
    if !status.is_success() {
        bail!("GET {url} lieferte HTTP {status}: {body}");
    }
    serde_json::from_str(&body).context("healthz JSON")
}

fn prune_empty_snapshots(base: &Path, current: Option<&Path>) -> Result<usize> {
    let mut removed = 0usize;
    for entry in fs::read_dir(base).with_context(|| format!("Korpuswurzel lesen: {}", base.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.contains(EMPTY_DIGEST_PREFIX) {
            continue;
        }
        if current.is_some_and(|live| live == path.as_path()) {
            continue;
        }
        fs::remove_dir_all(&path)
            .with_context(|| format!("leeren Snapshot entfernen: {}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

fn current_snapshot(base: &Path) -> Option<PathBuf> {
    let link = base.join("current");
    fs::canonicalize(link).ok()
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.command.is_empty() {
        bail!("Refresh-Befehl fehlt");
    }
    if !args.base.is_absolute() {
        bail!("Korpuswurzel muss absolut sein");
    }
    let parsed = url::Url::parse(&args.knowledge_url).context("Knowledge-Adresse")?;
    let loopback = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain("localhost")) => true,
        _ => false,
    };
    if parsed.scheme() != "http" || !loopback {
        bail!("Knowledge-Adresse muss eine lokale HTTP-Adresse sein");
    }

    let before = healthz(&args.knowledge_url).ok();
    let status = Command::new(&args.command[0])
        .args(&args.command[1..])
        .status()
        .with_context(|| format!("Refresh starten: {}", args.command[0]))?;
    let after = healthz(&args.knowledge_url).ok();
    let live = current_snapshot(&args.base);
    let pruned = prune_empty_snapshots(&args.base, live.as_deref()).unwrap_or(0);
    if pruned > 0 {
        eprintln!("Leere Korpus-Snapshots entfernt: {pruned}");
    }
    if status.success() {
        return Ok(());
    }
    let chunks = after
        .as_ref()
        .or(before.as_ref())
        .map(|health| health.chunks)
        .unwrap_or(0);
    if chunks > 0 {
        eprintln!(
            "Korpus-Refresh nicht aktiviert, letzter gültiger Index bleibt ({chunks} Abschnitte)."
        );
        return Ok(());
    }
    bail!(
        "Korpus-Refresh endete mit {} und es liegt kein gültiger Index vor",
        status
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn prune_skips_current_and_removes_empty_digest_dirs() {
        let root = tempfile::tempdir().unwrap();
        let keep = root.path().join("keep-good");
        let empty = root.path().join("75358d57e665-aaaa-e3b0c44298fc-bbbb");
        let current = root.path().join("75358d57e665-cccc-e3b0c44298fc-dddd");
        fs::create_dir(&keep).unwrap();
        fs::create_dir(&empty).unwrap();
        fs::create_dir(&current).unwrap();
        let removed = prune_empty_snapshots(root.path(), Some(&current)).unwrap();
        assert_eq!(removed, 1);
        assert!(keep.exists());
        assert!(current.exists());
        assert!(!empty.exists());
    }

    #[test]
    fn healthz_reads_chunk_count() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 512];
            let _ = stream.read(&mut buf);
            let body = br#"{"generation":"abc","chunks":178}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        let health = healthz(&format!("http://{addr}")).unwrap();
        assert_eq!(health.chunks, 178);
    }
}
