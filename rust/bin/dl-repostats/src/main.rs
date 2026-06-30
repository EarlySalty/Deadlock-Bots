//! `dl-repostats` — Git-Aktivitäts-Collector fürs Admin-Dashboard.
//!
//! Läuft per `systemd --user`-Timer (siehe `service/dl-repostats.*`) und auf
//! Abruf. Sweept die eigenen Repos, aggregiert pro Repo **Commits**, **Code-
//! Churn** (hinzugefügte/entfernte Zeilen) und **Datei-Änderungen** in
//! **Tagesbuckets** und schreibt ein kompaktes JSON-Artefakt, das
//! `service/dashboard.py` über `/api/repo-activity` ausliefert.
//!
//! Bewusst dependency-arm (kein tokio/DB): ruft nur `git log --numstat` als
//! Subprozess und schreibt eine Datei. Die eigentliche Aggregation ist eine
//! pure Funktion über den `git log`-Text ([`parse_log`]) — ohne echtes Repo
//! unit-testbar.
//!
//! Noise-Filter: Lockfiles, Vendored-/Build-Verzeichnisse, minifizierte und
//! generierte Dateien fließen NICHT in den Code-Churn und die Datei-Zahl ein
//! (sonst misst man Maschinen-Output statt Handarbeit). Commits bleiben roh
//! gezählt. Die angewandte Ausschlussliste steht im Artefakt (`excludes`),
//! damit die Anzeige ehrlich bleibt.
//!
//! Env:
//! - `REPOSTATS_BASE` (Default `/home/naniadm/Documents`) — Basisverzeichnis.
//! - `REPOSTATS_OUT`  (Default `<BASE>/Deadlock-Bots/data/repo_activity.json`).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// Die ausgewerteten Repos: (Verzeichnisname unter `REPOSTATS_BASE`, Anzeigename).
/// Bewusst die 7 eigenen Repos — ohne TradingBot, ohne vendored/fremde Repos.
const REPOS: &[(&str, &str)] = &[
    ("Deadlock-Bots", "Deadlock-Bots"),
    ("Deadlock-Twitch-Bot", "Twitch-Bot"),
    ("Deadlock-Steam-Bot", "Steam-Bot"),
    ("Deadlock-Turniere", "Turniere"),
    ("Deadlock--Patchnotes-Bot", "Patchnotes-Bot"),
    ("Deadlock-Brain", "Deadlock-Brain"),
    ("Website", "Website"),
];

/// Verzeichnis-Segmente, die als generiert/vendored gelten (Pfad enthält sie als Segment).
const NOISE_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "target",
    "vendor",
    "__pycache__",
    ".next",
    ".svelte-kit",
    "coverage",
    "venv",
    ".venv",
    "site-packages",
    ".mypy_cache",
    ".pytest_cache",
];

/// Exakte Dateinamen, die als Lock-/Generat gelten.
const NOISE_FILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "Pipfile.lock",
    "composer.lock",
    "uv.lock",
    "requirements-locked.txt",
    "bun.lockb",
    "go.sum",
];

/// Datei-Endungen, die als generiert/binär/Lock gelten.
const NOISE_SUFFIXES: &[&str] = &[
    ".lock", ".min.js", ".min.css", ".map", ".pyc", ".pyo", ".lockb", ".sqlite3", ".sqlite", ".db",
];

/// Menschlich lesbare Ausschlussliste fürs Artefakt/die Anzeige.
fn exclude_display() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    v.extend(NOISE_DIRS.iter().map(|d| format!("{d}/")));
    v.extend(NOISE_FILES.iter().map(|f| (*f).to_string()));
    v.extend(NOISE_SUFFIXES.iter().map(|s| format!("*{s}")));
    v
}

/// True, wenn der Pfad als Noise (generiert/vendored/Lock) gilt und damit nicht
/// in Churn und Datei-Zahl einfließen soll.
fn is_noise(path: &str) -> bool {
    let p = path.trim();
    for seg in NOISE_DIRS {
        if p == *seg || p.starts_with(&format!("{seg}/")) || p.contains(&format!("/{seg}/")) {
            return true;
        }
    }
    let file = p.rsplit('/').next().unwrap_or(p);
    if NOISE_FILES.contains(&file) {
        return true;
    }
    NOISE_SUFFIXES.iter().any(|suf| file.ends_with(suf))
}

#[derive(Default)]
struct Day {
    c: u64,
    a: u64,
    r: u64,
    f: u64,
}

#[derive(Default)]
struct RepoAgg {
    days: BTreeMap<String, Day>,
    total_commits: u64,
    total_add: u64,
    total_del: u64,
    total_files: u64,
}

/// Erkennt eine `--numstat`-Zeile: `<adds>\t<dels>\t<pfad>` (adds/dels sind
/// Zahlen oder `-` für Binärdateien).
fn parse_numstat(line: &str) -> Option<(u64, u64, &str)> {
    let mut it = line.splitn(3, '\t');
    let adds = it.next()?;
    let dels = it.next()?;
    let path = it.next()?;
    let parse_count = |s: &str| -> Option<u64> {
        if s == "-" {
            Some(0)
        } else {
            s.parse::<u64>().ok()
        }
    };
    Some((parse_count(adds)?, parse_count(dels)?, path))
}

/// Aggregiert die Ausgabe von
/// `git log --no-merges --numstat --date=short --pretty=format:'\x01%H %ad'`.
fn parse_log(output: &str) -> RepoAgg {
    let mut agg = RepoAgg::default();
    let mut current_date: Option<String> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix('\u{1}') {
            // Commit-Kopf: "<sha> <yyyy-mm-dd>" — Datum ist das letzte Token.
            let date = rest.rsplit(' ').next().unwrap_or("").trim().to_string();
            if date.is_empty() {
                current_date = None;
                continue;
            }
            agg.total_commits += 1;
            agg.days.entry(date.clone()).or_default().c += 1;
            current_date = Some(date);
            continue;
        }
        let Some(date) = current_date.as_ref() else {
            continue;
        };
        let Some((adds, dels, path)) = parse_numstat(line) else {
            continue;
        };
        if is_noise(path) {
            continue;
        }
        let day = agg.days.entry(date.clone()).or_default();
        day.a += adds;
        day.r += dels;
        day.f += 1;
        agg.total_add += adds;
        agg.total_del += dels;
        agg.total_files += 1;
    }
    agg
}

#[derive(Serialize)]
struct DayOut {
    d: String,
    c: u64,
    a: u64,
    r: u64,
    f: u64,
}

#[derive(Serialize)]
struct RepoOut {
    name: String,
    dir: String,
    first_commit: Option<String>,
    last_commit: Option<String>,
    total_commits: u64,
    total_add: u64,
    total_del: u64,
    total_files: u64,
    days: Vec<DayOut>,
}

impl RepoAgg {
    fn into_out(self, name: &str, dir: &str) -> RepoOut {
        let first_commit = self.days.keys().next().cloned();
        let last_commit = self.days.keys().next_back().cloned();
        let days = self
            .days
            .into_iter()
            .map(|(d, v)| DayOut {
                d,
                c: v.c,
                a: v.a,
                r: v.r,
                f: v.f,
            })
            .collect();
        RepoOut {
            name: name.to_string(),
            dir: dir.to_string(),
            first_commit,
            last_commit,
            total_commits: self.total_commits,
            total_add: self.total_add,
            total_del: self.total_del,
            total_files: self.total_files,
            days,
        }
    }
}

#[derive(Serialize)]
struct Artifact {
    generated_at: String,
    generated_unix: i64,
    excludes: Vec<String>,
    repos: Vec<RepoOut>,
}

fn run_git_log(repo: &str) -> anyhow::Result<String> {
    let out = Command::new("git")
        .args([
            "-C",
            repo,
            "log",
            "--no-merges",
            "--numstat",
            "--date=short",
            "--pretty=format:\u{1}%H %ad",
        ])
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git log fehlgeschlagen für {repo}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn main() -> anyhow::Result<()> {
    let base =
        std::env::var("REPOSTATS_BASE").unwrap_or_else(|_| "/home/naniadm/Documents".to_string());
    let out_path = std::env::var("REPOSTATS_OUT")
        .unwrap_or_else(|_| format!("{base}/Deadlock-Bots/data/repo_activity.json"));

    let mut repos = Vec::new();
    for (dir, name) in REPOS {
        let repo_path = format!("{base}/{dir}");
        if !Path::new(&repo_path).join(".git").exists() {
            eprintln!("skip {name}: kein Git-Repo unter {repo_path}");
            continue;
        }
        let log = run_git_log(&repo_path)?;
        repos.push(parse_log(&log).into_out(name, &repo_path));
    }

    let now = chrono::Utc::now();
    let artifact = Artifact {
        generated_at: now.to_rfc3339(),
        generated_unix: now.timestamp(),
        excludes: exclude_display(),
        repos,
    };

    let json = serde_json::to_string(&artifact)?;
    let tmp = format!("{out_path}.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &out_path)?;
    eprintln!(
        "repo_activity.json geschrieben: {out_path} ({} Repos, {} Commits gesamt)",
        artifact.repos.len(),
        artifact.repos.iter().map(|r| r.total_commits).sum::<u64>()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_filter_classifies_paths() {
        assert!(is_noise("Cargo.lock"));
        assert!(is_noise("frontend/package-lock.json"));
        assert!(is_noise("node_modules/x/y.js"));
        assert!(is_noise("a/dist/bundle.js"));
        assert!(is_noise("app.min.js"));
        assert!(is_noise("data/state.sqlite3"));
        assert!(is_noise("rust/target/debug/foo"));

        assert!(!is_noise("src/main.rs"));
        assert!(!is_noise("README.md"));
        assert!(!is_noise("service/static/dashboard.html"));
        assert!(!is_noise("assets/logo.png")); // hand-added Asset zählt
    }

    #[test]
    fn parse_log_aggregates_days_and_filters_noise() {
        let log = "\u{1}aaaaaa 2026-06-14\n\
                   10\t2\tsrc/main.rs\n\
                   0\t0\tCargo.lock\n\
                   \u{1}bbbbbb 2026-06-13\n\
                   5\t1\tsrc/lib.rs\n\
                   -\t-\tassets/logo.png\n";
        let agg = parse_log(log);

        assert_eq!(agg.total_commits, 2);
        assert_eq!(agg.total_add, 15); // 10 + 5 (Cargo.lock raus)
        assert_eq!(agg.total_del, 3); // 2 + 1
        assert_eq!(agg.total_files, 3); // main.rs, lib.rs, logo.png (Cargo.lock raus)

        let d14 = agg.days.get("2026-06-14").unwrap();
        assert_eq!((d14.c, d14.a, d14.r, d14.f), (1, 10, 2, 1));
        let d13 = agg.days.get("2026-06-13").unwrap();
        assert_eq!((d13.c, d13.a, d13.r, d13.f), (1, 5, 1, 2));

        let out = agg.into_out("Demo", "/tmp/demo");
        assert_eq!(out.first_commit.as_deref(), Some("2026-06-13"));
        assert_eq!(out.last_commit.as_deref(), Some("2026-06-14"));
        assert_eq!(out.days.len(), 2);
    }

    #[test]
    fn parse_log_handles_empty_output() {
        let agg = parse_log("");
        assert_eq!(agg.total_commits, 0);
        let out = agg.into_out("Empty", "/tmp/empty");
        assert_eq!(out.first_commit, None);
        assert!(out.days.is_empty());
    }
}
