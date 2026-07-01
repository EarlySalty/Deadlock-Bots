use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, FixedOffset, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{ColumnStatus, Ledger, LedgerError, TableLedger};

const CONSERVATIVE_CUTOFF_RFC3339: &str = "2026-06-30T10:27:05Z";
const CONSERVATIVE_CUTOFF_DISPLAY: &str = "2026-06-30 12:27:05 +0200";
const KNOWN_LIVE_START_DISPLAY: &str = "2026-07-01 03:29:38 +0200";

const SCOPED_LEDGER_FILES: &[(&str, &str)] = &[
    ("deadlock-sqlite3", "core.toml"),
    ("deadlock-sqlite3", "steam.toml"),
    ("deadlock-sqlite3", "bot.toml"),
    ("deadlock-sqlite3", "patchnotes.toml"),
    ("deadlock-sqlite3", "tierlist.toml"),
    ("deadlock-sqlite3", "activity.toml"),
    ("website", "coaching.toml"),
    ("website", "tierlist.toml"),
    ("website", "content.toml"),
    ("website", "patchnotes.toml"),
    ("website", "core.toml"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestOptions {
    pub ledger_dir: PathBuf,
    pub snapshot_root: PathBuf,
    pub snapshot_dir: Option<PathBuf>,
    pub generated_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationManifest {
    pub generated_at: String,
    pub mode: String,
    pub cutoff: CutoffManifest,
    pub snapshots: Vec<SnapshotDirectoryManifest>,
    pub ledger_files: Vec<LedgerFileManifest>,
    pub tables: Vec<TableManifest>,
    pub next_ticket_recommendations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CutoffManifest {
    pub selected_cutoff: String,
    pub selected_cutoff_unix_nanos: Option<u128>,
    pub source: String,
    pub confidence: String,
    pub fallback_cutoff: String,
    pub fallback_cutoff_display: String,
    pub known_live_start_display: String,
    pub selected_snapshot_dir: Option<String>,
    pub snapshot_window_start: Option<String>,
    pub snapshot_window_end: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotDirectoryManifest {
    pub path: String,
    pub selected: bool,
    pub directory_timestamp: Option<String>,
    pub directory_timestamp_unix_nanos: Option<u128>,
    pub file_count: usize,
    pub window_start: Option<String>,
    pub window_end: Option<String>,
    pub files: Vec<SnapshotFileManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotFileManifest {
    pub source_db: String,
    pub path: String,
    pub size_bytes: u64,
    pub mtime: String,
    pub mtime_unix_nanos: u128,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerFileManifest {
    pub source_db: String,
    pub ledger_file: String,
    pub path: String,
    pub table_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableManifest {
    pub source_db: String,
    pub ledger_file: String,
    pub source_table: String,
    pub target_tables: Vec<String>,
    pub classification: TableClassification,
    pub classification_reason: String,
    pub mapped_columns: usize,
    pub dropped_columns: usize,
    pub clock_columns: Vec<String>,
    pub append_identity_columns: Vec<String>,
    pub queue_state_indicators: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TableClassification {
    ClockPresent,
    AppendOnly,
    NoClock,
    QueueState,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("Manifest-I/O fehlgeschlagen fuer {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Ledger {path} konnte nicht geladen werden")]
    Ledger {
        path: PathBuf,
        #[source]
        source: LedgerError,
    },
    #[error("Ledger-Ziel {target} fuer {source_db}/{ledger_file}:{table}.{column} ist nicht schema.table.column")]
    InvalidTarget {
        source_db: String,
        ledger_file: String,
        table: String,
        column: String,
        target: String,
    },
    #[error("Snapshot-Datei {path} hat keinen gueltigen source_db-Dateinamen")]
    InvalidSnapshotFileName { path: PathBuf },
    #[error("mtime fuer {path} liegt vor UNIX_EPOCH")]
    MtimeBeforeEpoch { path: PathBuf },
}

pub fn build_reconciliation_manifest(
    options: &ManifestOptions,
) -> Result<ReconciliationManifest, ManifestError> {
    let snapshot_scan = scan_snapshots(&options.snapshot_root, options.snapshot_dir.as_deref())?;
    let scoped_ledgers = load_scoped_ledgers(&options.ledger_dir)?;
    let (ledger_files, tables) = table_manifests(scoped_ledgers)?;

    Ok(ReconciliationManifest {
        generated_at: system_time_rfc3339(options.generated_at),
        mode: "read_only_manifest_no_db_connections".to_string(),
        cutoff: cutoff_manifest(&snapshot_scan),
        snapshots: snapshot_scan.directories,
        ledger_files,
        tables,
        next_ticket_recommendations: vec![
            "T1 sollte Zeitfilter nur als Vorfilter verwenden; Entscheidung bleibt Hash-/Target-Vergleich."
                .to_string(),
            "Tabellen mit classification=no_clock oder queue_state brauchen vor Apply eine explizite Tabellenpolicy."
                .to_string(),
        ],
    })
}

fn load_scoped_ledgers(ledger_dir: &Path) -> Result<Vec<ScopedLedger>, ManifestError> {
    let mut ledgers = Vec::new();

    for (source_db, ledger_file) in SCOPED_LEDGER_FILES {
        let path = ledger_dir.join(source_db).join(ledger_file);
        let contents = fs::read_to_string(&path).map_err(|source| ManifestError::Io {
            path: path.clone(),
            source,
        })?;
        let ledger = Ledger::from_toml_str(&contents).map_err(|source| ManifestError::Ledger {
            path: path.clone(),
            source,
        })?;
        ledgers.push(ScopedLedger {
            source_db: (*source_db).to_string(),
            ledger_file: (*ledger_file).to_string(),
            path,
            ledger,
        });
    }

    Ok(ledgers)
}

fn table_manifests(
    scoped_ledgers: Vec<ScopedLedger>,
) -> Result<(Vec<LedgerFileManifest>, Vec<TableManifest>), ManifestError> {
    let mut ledger_files = Vec::new();
    let mut tables = Vec::new();

    for scoped in scoped_ledgers {
        ledger_files.push(LedgerFileManifest {
            source_db: scoped.source_db.clone(),
            ledger_file: scoped.ledger_file.clone(),
            path: path_string(&scoped.path),
            table_count: scoped.ledger.tables.len(),
        });

        for (source_table, table) in scoped.ledger.tables {
            let table_ref = TableRef {
                source_db: &scoped.source_db,
                ledger_file: &scoped.ledger_file,
                source_table: &source_table,
            };
            tables.push(table_manifest(table_ref, table)?);
        }
    }

    tables.sort_by(|left, right| {
        (
            &left.source_db,
            &left.ledger_file,
            &left.source_table,
            &left.target_tables,
        )
            .cmp(&(
                &right.source_db,
                &right.ledger_file,
                &right.source_table,
                &right.target_tables,
            ))
    });

    Ok((ledger_files, tables))
}

fn table_manifest(
    table_ref: TableRef<'_>,
    table: TableLedger,
) -> Result<TableManifest, ManifestError> {
    let mut target_tables = BTreeSet::new();
    let mut mapped_columns = 0;
    let mut dropped_columns = 0;
    let mut source_column_names = Vec::new();
    let mut target_column_names = Vec::new();

    for (source_column, status) in table.columns {
        match status {
            ColumnStatus::Mapped { to } => {
                let target = parse_target(&table_ref, &source_column, &to)?;
                target_tables.insert(target.table_name);
                source_column_names.push(source_column);
                target_column_names.push(target.column_name);
                mapped_columns += 1;
            }
            ColumnStatus::Dropped { .. } => {
                dropped_columns += 1;
            }
        }
    }

    let target_tables = target_tables.into_iter().collect::<Vec<_>>();
    let classifier_input = ClassifierInput {
        source_table: table_ref.source_table,
        target_tables: &target_tables,
        source_columns: &source_column_names,
        target_columns: &target_column_names,
    };
    let classification = classify_table(classifier_input);

    Ok(TableManifest {
        source_db: table_ref.source_db.to_string(),
        ledger_file: table_ref.ledger_file.to_string(),
        source_table: table_ref.source_table.to_string(),
        target_tables,
        classification: classification.classification,
        classification_reason: classification.reason,
        mapped_columns,
        dropped_columns,
        clock_columns: classification.clock_columns,
        append_identity_columns: classification.append_identity_columns,
        queue_state_indicators: classification.queue_state_indicators,
    })
}

fn parse_target(
    table_ref: &TableRef<'_>,
    source_column: &str,
    target: &str,
) -> Result<TargetColumn, ManifestError> {
    let parts = target.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.trim().is_empty()) {
        return Err(ManifestError::InvalidTarget {
            source_db: table_ref.source_db.to_string(),
            ledger_file: table_ref.ledger_file.to_string(),
            table: table_ref.source_table.to_string(),
            column: source_column.to_string(),
            target: target.to_string(),
        });
    }

    Ok(TargetColumn {
        table_name: format!("{}.{}", parts[0], parts[1]),
        column_name: parts[2].to_string(),
    })
}

fn classify_table(input: ClassifierInput<'_>) -> ClassificationResult {
    let table_names = input
        .target_tables
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(input.source_table))
        .collect::<Vec<_>>();
    let all_columns = input
        .source_columns
        .iter()
        .map(String::as_str)
        .chain(input.target_columns.iter().map(String::as_str))
        .collect::<Vec<_>>();

    let queue_state_indicators = queue_state_indicators(&table_names, &all_columns);
    if !queue_state_indicators.is_empty() {
        return ClassificationResult {
            classification: TableClassification::QueueState,
            reason: format!(
                "Queue-/State-Heuristik getroffen: {}",
                queue_state_indicators.join(", ")
            ),
            clock_columns: clock_columns(&all_columns),
            append_identity_columns: append_identity_columns(&table_names, &all_columns),
            queue_state_indicators,
        };
    }

    let clock_columns = clock_columns(&all_columns);
    if !clock_columns.is_empty() {
        return ClassificationResult {
            classification: TableClassification::ClockPresent,
            reason: format!("Aenderungsuhr vorhanden: {}", clock_columns.join(", ")),
            clock_columns,
            append_identity_columns: append_identity_columns(&table_names, &all_columns),
            queue_state_indicators,
        };
    }

    let append_identity_columns = append_identity_columns(&table_names, &all_columns);
    if !append_identity_columns.is_empty() {
        return ClassificationResult {
            classification: TableClassification::AppendOnly,
            reason: format!(
                "Append-only-Heuristik getroffen: {}",
                append_identity_columns.join(", ")
            ),
            clock_columns,
            append_identity_columns,
            queue_state_indicators,
        };
    }

    ClassificationResult {
        classification: TableClassification::NoClock,
        reason: "Keine Aenderungsuhr und keine Append-only-Identitaet aus Ledger-Spalten ableitbar"
            .to_string(),
        clock_columns,
        append_identity_columns,
        queue_state_indicators,
    }
}

fn queue_state_indicators(table_names: &[&str], columns: &[&str]) -> Vec<String> {
    let mut indicators = BTreeSet::new();
    let name_patterns = [
        "steam_tasks",
        "queue",
        "pending",
        "poll_state",
        "_state",
        "watch",
        "watchlist",
        "throttle",
        "cache",
        "miss_tracker",
        "friend_requests",
        "steam_beta_invites",
        "beta_invite_auto_failure_alerts",
        "beta_invite_friendship_auto_poll",
        "beta_invite_intent",
        "beta_invite_tickets",
        "role_cleanup",
    ];
    for table_name in table_names {
        let lower = table_name.to_ascii_lowercase();
        for pattern in name_patterns {
            if lower.contains(pattern) {
                indicators.insert(format!("table:{pattern}"));
            }
        }
    }

    let column_patterns = [
        "next_check_at",
        "attempts",
        "attempts_completed",
        "dispatch_attempts",
        "reserved_at",
        "locked",
    ];
    for column in columns {
        let lower = column.to_ascii_lowercase();
        for pattern in column_patterns {
            if lower == pattern {
                indicators.insert(format!("column:{pattern}"));
            }
        }
    }

    indicators.into_iter().collect()
}

fn clock_columns(columns: &[&str]) -> Vec<String> {
    let mut clocks = BTreeSet::new();
    let exact = [
        "updated_at",
        "modified_at",
        "last_update",
        "last_updated",
        "last_seen",
        "last_seen_at",
        "last_polled_at",
        "last_checked_at",
        "last_miss_at",
        "last_action_at",
        "last_applied_at",
        "last_payment_at",
        "deadlock_rank_updated_at",
    ];

    for column in columns {
        let lower = column.to_ascii_lowercase();
        if exact.contains(&lower.as_str()) || lower.ends_with("_updated_at") {
            clocks.insert(lower);
        }
    }

    clocks.into_iter().collect()
}

fn append_identity_columns(table_names: &[&str], columns: &[&str]) -> Vec<String> {
    let mut indicators = BTreeSet::new();
    let append_table_patterns = [
        "audit",
        "history",
        "archive",
        "log",
        "entries",
        "messages",
        "reviews",
        "changelog",
        "posts",
    ];
    for table_name in table_names {
        let lower = table_name.to_ascii_lowercase();
        for pattern in append_table_patterns {
            if lower.contains(pattern) {
                indicators.insert(format!("table:{pattern}"));
            }
        }
    }

    let append_columns = [
        "id",
        "created_at",
        "captured_at",
        "posted_at",
        "invited_at",
        "requested_at",
        "set_at",
        "granted_at",
        "decided_at",
        "first_clicked_at",
        "scheduled_at",
        "message_id",
        "url",
    ];
    for column in columns {
        let lower = column.to_ascii_lowercase();
        if append_columns.contains(&lower.as_str()) {
            indicators.insert(format!("column:{lower}"));
        }
    }

    indicators.into_iter().collect()
}

fn scan_snapshots(
    snapshot_root: &Path,
    explicit_snapshot_dir: Option<&Path>,
) -> Result<SnapshotScan, ManifestError> {
    let selected_path = explicit_snapshot_dir.map(Path::to_path_buf);
    let candidate_dirs = if let Some(snapshot_dir) = explicit_snapshot_dir {
        vec![snapshot_dir.to_path_buf()]
    } else {
        sorted_child_dirs(snapshot_root)?
    };

    let mut directories = Vec::new();
    for path in candidate_dirs {
        let mut files = Vec::new();
        for file_path in sorted_sqlite_files(&path)? {
            files.push(snapshot_file_manifest(&file_path)?);
        }

        let (window_start, window_end) = file_window(&files);
        let directory_timestamp = snapshot_dir_timestamp(&path);
        directories.push(SnapshotDirectoryManifest {
            path: path_string(&path),
            selected: false,
            directory_timestamp: directory_timestamp.map(system_time_rfc3339),
            directory_timestamp_unix_nanos: directory_timestamp.and_then(system_time_unix_nanos),
            file_count: files.len(),
            window_start,
            window_end,
            files,
        });
    }

    let selected_index = if selected_path.is_some() {
        directories.iter().position(|dir| dir.file_count > 0)
    } else {
        select_snapshot_index(&directories)
    };

    if let Some(index) = selected_index {
        if let Some(directory) = directories.get_mut(index) {
            directory.selected = true;
        }
    }

    Ok(SnapshotScan {
        directories,
        selected_index,
        explicit_snapshot_dir: selected_path,
    })
}

fn sorted_child_dirs(path: &Path) -> Result<Vec<PathBuf>, ManifestError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut dirs = Vec::new();
    for entry in fs::read_dir(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let entry_path = entry.path();
        let file_type = entry.file_type().map_err(|source| ManifestError::Io {
            path: entry_path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            dirs.push(entry_path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn sorted_sqlite_files(path: &Path) -> Result<Vec<PathBuf>, ManifestError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let entry_path = entry.path();
        let file_type = entry.file_type().map_err(|source| ManifestError::Io {
            path: entry_path.clone(),
            source,
        })?;
        if file_type.is_file()
            && entry_path.extension().and_then(|ext| ext.to_str()) == Some("sqlite3")
        {
            files.push(entry_path);
        }
    }
    files.sort();
    Ok(files)
}

fn snapshot_file_manifest(path: &Path) -> Result<SnapshotFileManifest, ManifestError> {
    let source_db = path
        .file_stem()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| ManifestError::InvalidSnapshotFileName {
            path: path.to_path_buf(),
        })?;
    let metadata = fs::metadata(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let modified = metadata.modified().map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mtime_unix_nanos = unix_nanos(path, modified)?;
    let sha256 = sha256_file(path)?;

    Ok(SnapshotFileManifest {
        source_db,
        path: path_string(path),
        size_bytes: metadata.len(),
        mtime: system_time_rfc3339(modified),
        mtime_unix_nanos,
        sha256,
    })
}

fn sha256_file(path: &Path) -> Result<String, ManifestError> {
    let mut file = fs::File::open(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| ManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn select_snapshot_index(directories: &[SnapshotDirectoryManifest]) -> Option<usize> {
    let mut candidates = directories
        .iter()
        .enumerate()
        .filter(|(_, dir)| dir.file_count > 0)
        .collect::<Vec<_>>();
    candidates.sort_by(|(_, left), (_, right)| {
        snapshot_rank(left)
            .cmp(&snapshot_rank(right))
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates.last().map(|(index, _)| *index)
}

fn snapshot_rank(directory: &SnapshotDirectoryManifest) -> (u8, u128) {
    let final_rank = if directory.path.contains("p4-final") {
        2
    } else if directory.path.contains("p4-") {
        1
    } else {
        0
    };
    let newest_mtime = directory
        .files
        .iter()
        .map(|file| file.mtime_unix_nanos)
        .max()
        .unwrap_or(0);
    (final_rank, newest_mtime)
}

fn cutoff_manifest(scan: &SnapshotScan) -> CutoffManifest {
    if let Some(selected_index) = scan.selected_index {
        let selected = &scan.directories[selected_index];
        let earliest = selected
            .files
            .iter()
            .map(|file| file.mtime_unix_nanos)
            .min()
            .and_then(system_time_from_unix_nanos);
        let selected_cutoff_nanos = selected
            .files
            .iter()
            .map(|file| file.mtime_unix_nanos)
            .min()
            .into_iter()
            .chain(selected.directory_timestamp_unix_nanos)
            .min();
        let latest = selected
            .files
            .iter()
            .map(|file| file.mtime_unix_nanos)
            .max()
            .and_then(system_time_from_unix_nanos);
        let selected_cutoff = selected_cutoff_nanos
            .and_then(system_time_from_unix_nanos)
            .map(system_time_rfc3339)
            .unwrap_or_else(|| CONSERVATIVE_CUTOFF_RFC3339.to_string());
        let evidence_source = if scan.explicit_snapshot_dir.is_some() {
            "explicit_snapshot_dir"
        } else {
            "auto_selected_snapshot_dir"
        };

        return CutoffManifest {
            selected_cutoff,
            selected_cutoff_unix_nanos: selected_cutoff_nanos,
            source: evidence_source.to_string(),
            confidence: "filesystem_snapshot_manifested_no_runner_log".to_string(),
            fallback_cutoff: CONSERVATIVE_CUTOFF_RFC3339.to_string(),
            fallback_cutoff_display: CONSERVATIVE_CUTOFF_DISPLAY.to_string(),
            known_live_start_display: KNOWN_LIVE_START_DISPLAY.to_string(),
            selected_snapshot_dir: Some(selected.path.clone()),
            snapshot_window_start: earliest.map(system_time_rfc3339),
            snapshot_window_end: latest.map(system_time_rfc3339),
            evidence: snapshot_evidence(selected),
        };
    }

    CutoffManifest {
        selected_cutoff: CONSERVATIVE_CUTOFF_RFC3339.to_string(),
        selected_cutoff_unix_nanos: None,
        source: "plan_fallback_no_snapshot_files".to_string(),
        confidence: "conservative_fallback".to_string(),
        fallback_cutoff: CONSERVATIVE_CUTOFF_RFC3339.to_string(),
        fallback_cutoff_display: CONSERVATIVE_CUTOFF_DISPLAY.to_string(),
        known_live_start_display: KNOWN_LIVE_START_DISPLAY.to_string(),
        selected_snapshot_dir: None,
        snapshot_window_start: None,
        snapshot_window_end: None,
        evidence: vec![
            "Kein Snapshot-Verzeichnis mit .sqlite3-Dateien gefunden".to_string(),
            "Konservativer Kandidaten-Cutoff aus Plan verwendet".to_string(),
        ],
    }
}

fn snapshot_evidence(selected: &SnapshotDirectoryManifest) -> Vec<String> {
    let mut evidence = vec![
        "Snapshot-Dateien mit SHA256 und mtime manifestiert".to_string(),
        format!(
            "Ausgewaehltes Verzeichnis: {} ({} Dateien)",
            selected.path, selected.file_count
        ),
    ];
    if let Some(directory_timestamp) = &selected.directory_timestamp {
        evidence.push(format!(
            "Snapshot-Start aus Verzeichnisname rekonstruiert: {directory_timestamp}"
        ));
    }
    evidence.push(
        "Runner-Log nicht vorausgesetzt; ohne Snapshot-Artefakt waere Plan-Fallback aktiv"
            .to_string(),
    );
    evidence
}

fn file_window(files: &[SnapshotFileManifest]) -> (Option<String>, Option<String>) {
    let start = files
        .iter()
        .map(|file| file.mtime_unix_nanos)
        .min()
        .and_then(system_time_from_unix_nanos)
        .map(system_time_rfc3339);
    let end = files
        .iter()
        .map(|file| file.mtime_unix_nanos)
        .max()
        .and_then(system_time_from_unix_nanos)
        .map(system_time_rfc3339);
    (start, end)
}

fn unix_nanos(path: &Path, time: SystemTime) -> Result<u128, ManifestError> {
    system_time_unix_nanos(time).ok_or_else(|| ManifestError::MtimeBeforeEpoch {
        path: path.to_path_buf(),
    })
}

fn system_time_unix_nanos(time: SystemTime) -> Option<u128> {
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    Some(duration.as_secs() as u128 * 1_000_000_000 + duration.subsec_nanos() as u128)
}

fn system_time_from_unix_nanos(nanos: u128) -> Option<SystemTime> {
    let secs = nanos / 1_000_000_000;
    let subsec_nanos = nanos % 1_000_000_000;
    let secs = u64::try_from(secs).ok()?;
    let subsec_nanos = u32::try_from(subsec_nanos).ok()?;
    Some(UNIX_EPOCH + std::time::Duration::new(secs, subsec_nanos))
}

fn system_time_rfc3339(time: SystemTime) -> String {
    let datetime = DateTime::<Utc>::from(time);
    datetime.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn snapshot_dir_timestamp(path: &Path) -> Option<SystemTime> {
    let file_name = path.file_name()?.to_str()?;
    let parts = file_name.split('-').collect::<Vec<_>>();
    let (date, time) = parts.windows(2).find_map(|window| {
        let date = window[0];
        let time = window[1];
        if date.len() == 8
            && time.len() == 6
            && date.bytes().all(|byte| byte.is_ascii_digit())
            && time.bytes().all(|byte| byte.is_ascii_digit())
        {
            Some((date, time))
        } else {
            None
        }
    })?;
    let naive = NaiveDateTime::parse_from_str(&format!("{date}{time}"), "%Y%m%d%H%M%S").ok()?;
    let offset = FixedOffset::east_opt(2 * 60 * 60)?;
    let local = offset.from_local_datetime(&naive).single()?;
    Some(SystemTime::from(local.with_timezone(&Utc)))
}

fn path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

#[derive(Debug)]
struct ScopedLedger {
    source_db: String,
    ledger_file: String,
    path: PathBuf,
    ledger: Ledger,
}

#[derive(Debug, Clone, Copy)]
struct TableRef<'a> {
    source_db: &'a str,
    ledger_file: &'a str,
    source_table: &'a str,
}

#[derive(Debug)]
struct TargetColumn {
    table_name: String,
    column_name: String,
}

#[derive(Debug, Clone, Copy)]
struct ClassifierInput<'a> {
    source_table: &'a str,
    target_tables: &'a [String],
    source_columns: &'a [String],
    target_columns: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassificationResult {
    classification: TableClassification,
    reason: String,
    clock_columns: Vec<String>,
    append_identity_columns: Vec<String>,
    queue_state_indicators: Vec<String>,
}

#[derive(Debug)]
struct SnapshotScan {
    directories: Vec<SnapshotDirectoryManifest>,
    selected_index: Option<usize>,
    explicit_snapshot_dir: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn classify_updated_at_as_clock_present() {
        let result = classify_table(ClassifierInput {
            source_table: "coach_applications",
            target_tables: &[String::from("coaching.coach_applications")],
            source_columns: &[String::from("id"), String::from("updated_at")],
            target_columns: &[String::from("id"), String::from("updated_at")],
        });

        assert_eq!(result.classification, TableClassification::ClockPresent);
        assert!(result.clock_columns.contains(&String::from("updated_at")));
    }

    #[test]
    fn classify_changelog_posts_as_append_only() {
        let result = classify_table(ClassifierInput {
            source_table: "changelog_posts",
            target_tables: &[String::from("patchnotes.changelog_posts")],
            source_columns: &[
                String::from("id"),
                String::from("url"),
                String::from("posted_at"),
            ],
            target_columns: &[
                String::from("id"),
                String::from("url"),
                String::from("posted_at"),
            ],
        });

        assert_eq!(result.classification, TableClassification::AppendOnly);
        assert!(result
            .append_identity_columns
            .contains(&String::from("column:posted_at")));
    }

    #[test]
    fn classify_steam_tasks_as_queue_state() {
        let result = classify_table(ClassifierInput {
            source_table: "steam_tasks",
            target_tables: &[String::from("steam.steam_tasks")],
            source_columns: &[
                String::from("id"),
                String::from("status"),
                String::from("attempts"),
                String::from("created_at"),
            ],
            target_columns: &[
                String::from("id"),
                String::from("status"),
                String::from("attempts"),
                String::from("created_at"),
            ],
        });

        assert_eq!(result.classification, TableClassification::QueueState);
        assert!(result
            .queue_state_indicators
            .contains(&String::from("table:steam_tasks")));
    }

    #[test]
    fn classify_table_without_clock_as_no_clock() {
        let result = classify_table(ClassifierInput {
            source_table: "kv_store",
            target_tables: &[String::from("bot.kv_store")],
            source_columns: &[
                String::from("ns"),
                String::from("key"),
                String::from("value"),
            ],
            target_columns: &[
                String::from("ns"),
                String::from("key"),
                String::from("value"),
            ],
        });

        assert_eq!(result.classification, TableClassification::NoClock);
    }

    #[test]
    fn selects_latest_p4_final_snapshot_and_hashes_files() {
        let root = tempdir().expect("tempdir");
        let old_dir = root.path().join("p4-20260701-023900");
        let final_dir = root.path().join("p4-final-20260701-032848");
        fs::create_dir_all(&old_dir).expect("old dir");
        fs::create_dir_all(&final_dir).expect("final dir");
        write_file(&old_dir.join("deadlock-sqlite3.sqlite3"), b"old");
        write_file(&final_dir.join("deadlock-sqlite3.sqlite3"), b"final");

        let scan = scan_snapshots(root.path(), None).expect("scan snapshots");
        let selected = scan
            .directories
            .iter()
            .find(|dir| dir.selected)
            .expect("selected snapshot");

        assert!(selected.path.ends_with("p4-final-20260701-032848"));
        assert_eq!(selected.files.len(), 1);
        assert_eq!(
            selected.files[0].sha256,
            "2443630b4620165c8b173e7265e17526fe2787ae594364dd6d839ad58f2fc007"
        );
    }

    fn write_file(path: &Path, contents: &[u8]) {
        let mut file = File::create(path).expect("create file");
        file.write_all(contents).expect("write file");
    }
}
