use std::path::Path;

use super::*;

fn chunks(path: &str, body: &str, stand: &str, quelle: &str) -> Vec<Chunk> {
    let raw = format!(
        "<html><head><title>Support</title><meta name=\"tags\" content=\"support\">\
         <meta name=\"stand\" content=\"{stand}\"><meta name=\"quelle\" content=\"{quelle}\">\
         </head><body><main><h1>Support</h1><p>Öffentliche Hilfe.</p>{body}</main></body></html>"
    );
    crate::parse_html_file(
        Path::new("/docs/public"),
        &Path::new("/docs/public").join(path),
        &raw,
    )
    .expect("Gültige Testfixture erwartet")
}

fn sample() -> Vec<Chunk> {
    chunks(
        "support.html",
        "<section><h2>Steam</h2><p>Steam im Konto verknüpfen.</p></section>",
        "2026-09-18",
        "Öffentliche Dokumentation",
    )
}

#[test]
fn dense_uebernimmt_gepruefte_metadaten_ohne_textumbau() {
    let original = sample();
    let dense = from_chunks(&original).expect("Gültige Testfixture erwartet");
    assert_eq!(dense.len(), original.len());
    for (record, chunk) in dense.iter().zip(original) {
        assert_eq!(record.stand, "2026-09-18");
        assert_eq!(record.quelle, "Öffentliche Dokumentation");
        assert_eq!(record.doc_path, chunk.path);
        assert_eq!(record.text, chunk.text);
        assert_eq!(record.chunk_id.len(), 64);
        assert_eq!(record.content_hash.len(), 64);
    }
}

#[test]
fn chunk_id_bleibt_bei_inhaltsaenderung_und_fremder_einfuegung_stabil() {
    let original = sample();
    let before = from_chunks(&original).expect("Gültige Testfixture erwartet");
    let mut changed = original.clone();
    changed[1].text = "Anderer Inhalt für Steam.".to_string();
    let mut all = chunks("anderes.html", "", "2026-09-18", "Doku");
    all.extend(changed);
    let after = from_chunks(&all).expect("Gültige Testfixture erwartet");
    assert!(!before.is_empty());
    for record in before {
        assert!(after.iter().any(|next| next.chunk_id == record.chunk_id));
    }
}

#[test]
fn content_hash_aendert_sich_nur_mit_embedding_eingabe() {
    let original = sample();
    let before = from_chunks(&original).expect("Gültige Testfixture erwartet");
    let new_metadata = chunks(
        "support.html",
        "<section><h2>Steam</h2><p>Steam im Konto verknüpfen.</p></section>",
        "2026-09-19",
        "Andere Quelle",
    );
    let after = from_chunks(&new_metadata).expect("Gültige Testfixture erwartet");
    assert_eq!(before[1].content_hash, after[1].content_hash);
    let mut changed = original;
    changed[1].text.push_str(" Neues Detail.");
    assert_ne!(
        before[1].content_hash,
        from_chunks(&changed).expect("Gültige Testfixture erwartet")[1].content_hash
    );
}

#[test]
fn gleiche_ueberschriften_bekommen_eindeutige_stabile_ids() {
    let mut input = sample();
    input.push(input[1].clone());
    let first = from_chunks(&input).expect("Gültige Testfixture erwartet");
    assert_eq!(first.len(), 3);
    assert_ne!(first[1].chunk_id, first[2].chunk_id);
    assert_eq!(
        first,
        from_chunks(&input).expect("Gültige Testfixture erwartet")
    );
}

#[test]
fn dense_lehnt_markdown_und_internal_auch_beim_direkten_aufruf_ab() {
    for path in [
        "test.md",
        "internal/test.html",
        "PUBLIC/INTERNAL/test.html",
        "../test.html",
        "/test.html",
    ] {
        let mut input = sample();
        input[0].path = path.to_string();
        assert!(
            from_chunks(&input).is_err(),
            "Unsicherer Pfad akzeptiert: {path}"
        );
    }
}

#[test]
fn vektoren_brauchen_384_endliche_dimensionen_und_eine_norm() {
    for invalid in [
        vec![1.0; DIMENSIONS - 1],
        vec![f32::NAN; DIMENSIONS],
        vec![f32::INFINITY; DIMENSIONS],
        vec![0.0; DIMENSIONS],
    ] {
        assert!(validate_embedding(&invalid).is_err());
    }
    assert!(validate_embedding(&vec![1.0; DIMENSIONS]).is_ok());
}

#[test]
fn embedding_modul_hat_keinen_http_oder_download_pfad() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/dense/local.rs"))
        .expect("Gültige Testfixture erwartet");
    for forbidden in [
        "reqwest",
        "hf_hub",
        "TcpStream",
        "std::process",
        "ureq",
        "try_new(",
        "http://",
        "https://",
    ] {
        assert!(
            !source.contains(forbidden),
            "Netzwerkpfad im Embedder: {forbidden}"
        );
    }
    let manifest =
        std::fs::read_to_string(root.join("Cargo.toml")).expect("Gültige Testfixture erwartet");
    let dependency = manifest
        .lines()
        .find(|line| line.starts_with("fastembed ="))
        .expect("Gültige Testfixture erwartet");
    assert!(dependency.contains("default-features = false"));
    assert!(!dependency.contains("hf-hub"));
}
