use std::{collections::BTreeSet, fs, path::Path};

const TIMEOUTS: [&str; 2] = ["lock_timeout", "statement_timeout"];
const APPLIED_LEAKS: [&str; 1] = ["20261021120000_global_dm_optout.sql"];
const RULE: &str = "A migration sets lock_timeout or statement_timeout only with SET LOCAL, or \
    resets it with RESET <name> after its last session-level SET; sqlx runs each migration in \
    its own transaction on a pooled connection, so a session-level SET outlives the migration.";

fn statement_starts(lowered: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(lowered.match_indices(';').map(|(at, _)| at + 1));
    starts
}

fn leaks(sql: &str) -> bool {
    let lowered = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    TIMEOUTS.iter().any(|name| {
        let session_sets = [format!("set {name}"), format!("set session {name}")];
        let last_set = statement_starts(&lowered)
            .into_iter()
            .filter_map(|start| {
                let statement = lowered.get(start..)?.trim_start();
                let offset = lowered.len() - statement.len();
                session_sets
                    .iter()
                    .any(|set| {
                        statement.starts_with(set.as_str())
                            && !statement[set.len()..]
                                .starts_with(|c: char| c == '_' || c.is_ascii_alphanumeric())
                    })
                    .then_some(offset)
            })
            .max();
        last_set.is_some_and(|at| !lowered[at..].contains(&format!("reset {name}")))
    })
}

fn leaking_files() -> BTreeSet<String> {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(&migrations_path).expect("migrations directory must be readable") {
        let path = entry.expect("migration entry must be readable").path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("migration file name must be UTF-8: {}", path.display()))
            .to_owned();
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{name} must be readable as UTF-8: {error}"));
        if leaks(&sql) {
            found.insert(name);
        }
    }
    found
}

#[test]
fn session_timeouts_do_not_outlive_their_migration() {
    let applied: BTreeSet<String> = APPLIED_LEAKS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let found = leaking_files();
    assert_eq!(
        found, applied,
        "{RULE} Only the already applied files in APPLIED_LEAKS may keep a leaking SET."
    );
}

#[test]
fn leak_detection_covers_every_form() {
    let cases: &[(&str, &str, bool)] = &[
        ("no timeout", "CREATE TABLE core.t (id BIGINT);", false),
        (
            "session set",
            "SET lock_timeout TO '5s';\nCREATE TABLE core.t (id BIGINT);",
            true,
        ),
        (
            "session keyword",
            "SET SESSION lock_timeout = '5s'; SELECT 1;",
            true,
        ),
        (
            "statement timeout",
            "set statement_timeout to 1000; select 1;",
            true,
        ),
        ("lower case set", "set lock_timeout = '2s'; select 1;", true),
        (
            "set local",
            "SET LOCAL lock_timeout TO '5s'; SELECT 1;",
            false,
        ),
        (
            "set then reset",
            "SET lock_timeout TO '2s'; SELECT 1; RESET lock_timeout;",
            false,
        ),
        (
            "reset before a second set",
            "SET lock_timeout = '2s'; RESET lock_timeout; SET lock_timeout = '3s';",
            true,
        ),
        (
            "other reset",
            "SET lock_timeout = '2s'; RESET statement_timeout;",
            true,
        ),
        (
            "update set column",
            "UPDATE t\nSET lock_timeout_ms = 1;",
            false,
        ),
        ("similar name", "SET lock_timeout_extra TO 1;", false),
    ];
    for (label, sql, expected) in cases {
        assert_eq!(leaks(sql), *expected, "{label}");
    }
}
