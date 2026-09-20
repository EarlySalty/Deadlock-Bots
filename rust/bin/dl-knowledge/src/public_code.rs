//! Explicit public behavior excerpts, never repository search at request time.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Read, path::Path};

include!(concat!(env!("OUT_DIR"), "/source_revision.rs"));

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub id: String,
    pub title: String,
    pub public_help_path: String,
    pub public_help_section: String,
    pub public_help_sha256: String,
    pub repository: String,
    pub source_path: String,
    pub symbol: String,
    pub blob_sha256: String,
    pub excerpt: String,
    pub explanation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    policy_revision: String,
    source_policy_version: u32,
    release_commit: String,
    entries: Vec<Entry>,
}

pub(crate) fn load(
    root: &Path,
    expected_hash: &str,
    hashes: &HashMap<String, String>,
) -> Result<Vec<Entry>> {
    let file = std::fs::File::open(
        root.parent()
            .context("Snapshotwurzel fehlt")?
            .join("public-code-manifest.json"),
    )?;
    let mut bytes = Vec::new();
    file.take(32769).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 32768 && crate::dense::sha256(&bytes) == expected_hash,
        "Öffentliche Codebelege gehören nicht zum FAQ-Snapshot"
    );
    parse(&bytes, hashes, SOURCE_REVISION)
}

fn parse(bytes: &[u8], hashes: &HashMap<String, String>, revision: &str) -> Result<Vec<Entry>> {
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    ensure!(
        manifest.schema_version == 1
            && manifest.source_policy_version == 1
            && !manifest.policy_revision.trim().is_empty(),
        "Unbekannte öffentliche Codepolicy"
    );
    ensure!(
        manifest.entries.len() <= 4,
        "Zu viele öffentliche Codebelege"
    );
    if manifest.entries.is_empty() {
        return Ok(Vec::new());
    }
    if revision.is_empty() || manifest.release_commit != revision {
        tracing::warn!(
            "Öffentliche Codebelege deaktiviert: Freigabe gehört zu anderem Dienstrelease"
        );
        return Ok(Vec::new());
    }
    let mut ids = std::collections::HashSet::new();
    for entry in &manifest.entries {
        ensure!(
            entry
                .id
                .strip_prefix('P')
                .is_some_and(|id| matches!(id, "1" | "2" | "3" | "4"))
                && ids.insert(&entry.id),
            "Ungültige Codebeleg-ID"
        );
        ensure!(
            entry.repository == "discord" && allowed_symbol(&entry.source_path, &entry.symbol),
            "Codequelle nicht öffentlich freigegeben"
        );
        ensure!(
            crate::retrieval::public_html_path(&entry.public_help_path)
                && hashes.get(&entry.public_help_path) == Some(&entry.public_help_sha256),
            "Öffentliche Codehilfe ist nicht im geprüften Snapshot"
        );
        ensure!(
            !entry.title.trim().is_empty()
                && !entry.public_help_section.trim().is_empty()
                && !entry.excerpt.trim().is_empty()
                && !entry.explanation.trim().is_empty(),
            "Unvollständiger öffentlicher Codebeleg"
        );
        ensure!(
            entry.blob_sha256.len() == 64
                && entry.blob_sha256.bytes().all(|c| c.is_ascii_hexdigit()),
            "Ungültiger Codehash"
        );
        ensure!(
            entry.excerpt.encode_utf16().count() + entry.explanation.encode_utf16().count() + 2
                <= 2000,
            "Öffentlicher Codebeleg zu lang"
        );
    }
    Ok(manifest.entries)
}

fn allowed_symbol(path: &str, symbol: &str) -> bool {
    matches!(
        (path, symbol),
        ("rust/crates/dl-community/src/faq.rs", "register:faq")
            | (
                "rust/crates/dl-voice/src/lfg_panel.rs",
                "LFG_ERR_SCHON_AKTIVE_SUCHE" | "LFG_WATCH_ERR_UNVOLLSTAENDIG"
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"source_policy_version":1,"policy_revision":"reviewed","release_commit":"a".repeat(40),"entries":[{"id":"P1","title":"FAQ","public_help_path":"discord/faq.html","public_help_section":"start","public_help_sha256":"b".repeat(64),"repository":"discord","source_path":"rust/crates/dl-community/src/faq.rs","symbol":"register:faq","blob_sha256":"c".repeat(64),"excerpt":"Privater Fragechat","explanation":"Öffnet einen privaten Fragechat."}]})
    }
    #[test]
    fn only_current_reviewed_sources_and_matching_help_are_admitted() -> Result<()> {
        let hashes = HashMap::from([("discord/faq.html".into(), "b".repeat(64))]);
        let valid = fixture();
        let bytes = serde_json::to_vec(&valid)?;
        assert_eq!(parse(&bytes, &hashes, &"a".repeat(40))?.len(), 1);
        assert!(parse(&bytes, &hashes, &"d".repeat(40))?.is_empty());
        assert!(parse(&bytes, &HashMap::new(), &"a".repeat(40)).is_err());
        let mut forbidden = valid;
        forbidden["entries"][0]["source_path"] =
            serde_json::json!("rust/crates/dl-ai/src/secrets.rs");
        assert!(parse(&serde_json::to_vec(&forbidden)?, &hashes, &"a".repeat(40)).is_err());
        Ok(())
    }
    #[test]
    fn altered_code_manifest_cannot_join_faq_snapshot() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("public");
        std::fs::create_dir(&root)?;
        let bytes = serde_json::to_vec(&fixture())?;
        std::fs::write(directory.path().join("public-code-manifest.json"), &bytes)?;
        assert!(load(&root, &"0".repeat(64), &HashMap::new()).is_err());
        Ok(())
    }
}
