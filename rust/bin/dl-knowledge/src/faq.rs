//! Exact, curated FAQ shortcuts bound to the same immutable HTML snapshot.
use std::{collections::HashMap, io::Read, path::Path};

use anyhow::{ensure, Context, Result};
use serde::Deserialize;

use crate::{dense, html_selector, html_text, Chunk};

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    question: String,
    pub chunk: Chunk,
}

#[cfg(test)]
pub(crate) fn fixture(question: &str, chunk: Chunk) -> Entry {
    Entry {
        question: normalize(question),
        chunk,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    policy_revision: String,
    entries: Vec<Source>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    question: String,
    path: String,
    section_id: String,
    source_sha256: String,
}

fn normalize(question: &str) -> String {
    // Deliberately no token sorting, stemming, punctuation removal or aliases:
    // negation and appended requests must never become an exact FAQ hit.
    question
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn exact<'a>(entries: &'a [Entry], question: &str) -> Option<&'a Chunk> {
    let normalized = normalize(question);
    let mut matches = entries.iter().filter(|entry| entry.question == normalized);
    let found = matches.next()?;
    matches.next().is_none().then_some(&found.chunk)
}

pub(crate) fn load(
    root: &Path,
    chunks: &[Chunk],
    hashes: &HashMap<String, String>,
) -> Result<(Vec<Entry>, Option<String>)> {
    let path = root
        .parent()
        .context("FAQ-Snapshotwurzel fehlt")?
        .join("faq-manifest.json");
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), None))
        }
        Err(error) => return Err(error).context("FAQ-Manifest öffnen"),
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "FAQ-Manifest zu groß");
    let manifest: Manifest = serde_json::from_slice(&bytes).context("FAQ-Manifest lesen")?;
    ensure!(
        manifest.schema_version == 1 && !manifest.policy_revision.trim().is_empty(),
        "Ungültige FAQ-Version"
    );
    let selector = html_selector("main > section")?;
    let heading_selector = html_selector("h2")?;
    let mut entries = Vec::new();
    for source in manifest.entries {
        ensure!(
            crate::retrieval::public_html_path(&source.path),
            "FAQ-Pfad ist nicht öffentlich"
        );
        ensure!(
            hashes.get(&source.path) == Some(&source.source_sha256),
            "FAQ-Quelle gehört nicht zum geladenen Snapshot"
        );
        ensure!(
            source.question.trim().ends_with('?') && !source.section_id.is_empty(),
            "FAQ benötigt ausdrückliche Frage und Abschnitt"
        );
        let raw =
            std::fs::read_to_string(root.join(&source.path)).context("FAQ-Quellabschnitt lesen")?;
        ensure!(
            dense::sha256(raw.as_bytes()) == source.source_sha256,
            "FAQ-Quelle während des Ladens geändert"
        );
        let document = scraper::Html::parse_document(&raw);
        let matching = document
            .select(&selector)
            .filter(|section| section.value().attr("id") == Some(source.section_id.as_str()))
            .collect::<Vec<_>>();
        ensure!(
            matching.len() == 1,
            "FAQ-Abschnitt fehlt oder ist mehrdeutig"
        );
        let headings = matching[0]
            .select(&heading_selector)
            .map(|heading| html_text(&heading))
            .collect::<Vec<_>>();
        ensure!(
            headings == [source.question.clone()],
            "FAQ-Frage passt nicht zum wörtlichen Abschnittstitel"
        );
        let matching = chunks
            .iter()
            .filter(|chunk| chunk.path == source.path && chunk.section == source.question)
            .collect::<Vec<_>>();
        ensure!(
            matching.len() == 1 && !matching[0].passages.is_empty(),
            "FAQ-Abschnitt nicht eindeutig im öffentlichen Index"
        );
        entries.push(Entry {
            question: normalize(&source.question),
            chunk: matching[0].clone(),
        });
    }
    Ok((entries, Some(dense::sha256(&bytes))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faq_is_exact_current_and_public() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("public");
        std::fs::create_dir(&root)?;
        let raw = r#"<html><head><title>Hilfe</title><meta name="tags" content="hilfe"><meta name="stand" content="2026-09-20"><meta name="quelle" content="Öffentliche Hilfe"></head><body><main><h1>Hilfe</h1><section id="pause"><h2>Wie pausiere ich den Bot?</h2><p>Öffne die Einstellungen und wähle Pause.</p></section></main></body></html>"#;
        std::fs::write(root.join("hilfe.html"), raw)?;
        let manifest = serde_json::json!({"schema_version":1,"policy_revision":"test-v1","entries":[{
            "question":"Wie pausiere ich den Bot?","path":"hilfe.html","section_id":"pause","source_sha256":dense::sha256(raw.as_bytes())
        }]});
        let manifest_path = dir.path().join("faq-manifest.json");
        std::fs::write(&manifest_path, manifest.to_string())?;
        let kb = crate::load_production_corpus(&root)?;
        assert_eq!(
            kb.faq_generation,
            Some(dense::sha256(manifest.to_string().as_bytes()))
        );
        std::fs::remove_file(&manifest_path)?;
        let withdrawn = crate::load_production_corpus(&root)?;
        assert_eq!(withdrawn.corpus_digest, kb.corpus_digest);
        assert_eq!(withdrawn.faq_generation, None);
        assert!(withdrawn.faq.is_empty());
        std::fs::write(&manifest_path, manifest.to_string())?;
        assert!(exact(&kb.faq, " WIE  pausiere ich den Bot? ").is_some());
        for question in [
            "Wie pausiere ich den Bot nicht?",
            "Wie pausiere ich den Bot? Und wie starte ich ihn?",
            "Wie aktiviere ich den Bot?",
            "Wie pausiere ich den Bot",
        ] {
            assert!(exact(&kb.faq, question).is_none(), "{question}");
        }
        let digest = kb.corpus_digest;
        std::fs::write(root.join("hilfe.html"), raw.replace("Pause.", "Stop."))?;
        assert!(
            crate::load_production_corpus(&root).is_err(),
            "veralteter FAQ-Hash darf nicht aktiv werden"
        );
        std::fs::remove_file(&manifest_path)?;
        let changed = crate::load_production_corpus(&root)?;
        assert_ne!(digest, changed.corpus_digest);
        assert!(
            changed.faq.is_empty(),
            "entzogene FAQ verschwindet vollständig"
        );
        let expected_line = format!(
            "hilfe.html\0{}\n",
            dense::sha256(raw.replace("Pause.", "Stop.").as_bytes())
        );
        assert_eq!(
            changed.corpus_digest,
            dense::sha256(expected_line.as_bytes())
        );
        Ok(())
    }
}
