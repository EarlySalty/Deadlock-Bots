use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OpenFlags};

pub const DEADLOCK_SQLITE3_SOURCE: &str =
    "/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3";
pub const WEBSITE_SOURCE: &str = "/home/naniadm/Documents/Website/builds/backend/deadlock.db";
pub const TOURNAMENT_SOURCE: &str =
    "/home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db";

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("Snapshot-Quelle fehlt: {path}")]
    MissingSource { path: PathBuf },
    #[error("Snapshot-Pfad {path} ist kein gueltiger UTF-8-Pfad")]
    NonUtf8Path { path: PathBuf },
    #[error("Snapshot-Ziel ist identisch mit der Quelle: {path}")]
    SourceEqualsDestination { path: PathBuf },
    #[error("Snapshot-I/O fehlgeschlagen fuer {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub fn snapshot_db(src: &Path, dst: &Path) -> Result<(), SnapshotError> {
    if !src.exists() {
        return Err(SnapshotError::MissingSource {
            path: src.to_path_buf(),
        });
    }

    if src == dst || same_existing_file(src, dst)? {
        return Err(SnapshotError::SourceEqualsDestination {
            path: src.to_path_buf(),
        });
    }

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|source| SnapshotError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    if dst.exists() {
        fs::remove_file(dst).map_err(|source| SnapshotError::Io {
            path: dst.to_path_buf(),
            source,
        })?;
    }

    let dst_text = dst.to_str().ok_or_else(|| SnapshotError::NonUtf8Path {
        path: dst.to_path_buf(),
    })?;
    let conn = Connection::open_with_flags(
        src,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    conn.execute("VACUUM main INTO ?1", params![dst_text])?;
    Ok(())
}

fn same_existing_file(src: &Path, dst: &Path) -> Result<bool, SnapshotError> {
    if !dst.exists() {
        return Ok(false);
    }

    let src_metadata = fs::metadata(src).map_err(|source| SnapshotError::Io {
        path: src.to_path_buf(),
        source,
    })?;
    let dst_metadata = fs::metadata(dst).map_err(|source| SnapshotError::Io {
        path: dst.to_path_buf(),
        source,
    })?;

    Ok(src_metadata.dev() == dst_metadata.dev() && src_metadata.ino() == dst_metadata.ino())
}

pub fn source_snapshot_path(snapshot_dir: &Path, source_db: &str) -> PathBuf {
    snapshot_dir.join(format!("{source_db}.sqlite3"))
}

pub fn snapshot_known_sources(
    snapshot_dir: &Path,
) -> Result<BTreeMap<String, PathBuf>, SnapshotError> {
    let sources = [
        ("deadlock-sqlite3", DEADLOCK_SQLITE3_SOURCE),
        ("website", WEBSITE_SOURCE),
        ("tournament", TOURNAMENT_SOURCE),
    ];
    let mut snapshots = BTreeMap::new();

    for (source_db, source_path) in sources {
        let dst = source_snapshot_path(snapshot_dir, source_db);
        snapshot_db(Path::new(source_path), &dst)?;
        snapshots.insert(source_db.to_string(), dst);
    }

    Ok(snapshots)
}
