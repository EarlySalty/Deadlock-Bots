use super::*;
use crate::{Passage, PassageKind};

fn chunk(path: &str, stand: &str, text: &str) -> Chunk {
    Chunk {
        title: "Hilfe".into(),
        section: "Anleitung".into(),
        path: path.into(),
        tags: Vec::new(),
        stand: stand.into(),
        quelle: "docs".into(),
        text: text.into(),
        passages: vec![Passage {
            heading: None,
            context: None,
            body: text.into(),
            kind: PassageKind::Paragraph,
        }],
    }
}

fn knowledge() -> KnowledgeBase {
    KnowledgeBase::from_chunks(vec![
        chunk("a.html", "2026-08-01", "Steam verknüpfen"),
        chunk("b.html", "2026-09-18", "Steam mit Discord verbinden"),
        chunk("c.html", "2026-09-18", "Turniere anmelden"),
    ])
}

#[test]
fn hybrid_default_aus_ignoriert_optionen() -> Result<()> {
    assert!(Config::from_values(None, Some("ungültig"))?.is_none());
    assert!(Config::from_values(Some("0"), None)?.is_none());
    let config = Config::from_values(Some("true"), None)?.expect("Gültiger Testwert erwartet");
    assert_eq!(config.output_k, 6);
    assert_eq!(config.fusion_k, 12);
    assert!(config.rerank);
    Ok(())
}

#[test]
fn konfiguration_lehnt_tippfehler_und_unbegrenzte_last_ab() {
    assert!(Config::from_values(Some("tru"), None).is_err());
    for json in [
        r#"{"dense_k":0}"#,
        r#"{"fusion_k":101}"#,
        r#"{"output_k":25}"#,
        r#"{"rrf_k":0}"#,
        r#"{"bm25_weight":0,"dense_weight":0}"#,
        r#"{"source_consensus_weight":-1}"#,
        r#"{"timeout_ms":10001}"#,
        r#"{"rerank_batch_size":0}"#,
        r#"{"rerank_max_tokens":513}"#,
        r#"{"unknown":true}"#,
        r#"{"metadata":{"stand_min":"2026-02-30"}}"#,
    ] {
        assert!(
            Config::from_values(Some("true"), Some(json)).is_err(),
            "{json}"
        );
    }
    let config = Config {
        bm25_weight: f64::NAN,
        ..Config::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn datum_prueft_kalender_und_utf8() {
    use config::day;
    assert_eq!(
        day("2026-03-01").expect("Gültiger Testwert erwartet")
            - day("2026-02-28").expect("Gültiger Testwert erwartet"),
        1
    );
    assert_eq!(
        day("2024-03-01").expect("Gültiger Testwert erwartet")
            - day("2024-02-28").expect("Gültiger Testwert erwartet"),
        2
    );
    assert!(day("2000-02-29").is_some());
    for value in [
        "1900-02-29",
        "2026-02-29",
        "2026-13-01",
        "0000-01-01",
        "2026-00-01",
        "2026-01-00",
        "ä26-01-01",
        "aktuell",
    ] {
        assert!(day(value).is_none());
    }
}

#[test]
fn metadaten_filter_sind_optional_aber_strikt() {
    let mut metadata = config::Metadata::default();
    assert!(metadata.accepts("Revision abc", "docs"));
    metadata.stand_min = Some("2026-09-01".into());
    assert!(!metadata.accepts("Revision abc", "docs"));
    assert!(!metadata.accepts("2026-08-31", "docs"));
    metadata.stand_exact = Some("2026-09-18".into());
    metadata.quellen = vec!["docs".into()];
    assert!(metadata.accepts("2026-09-18", "docs"));
    assert!(!metadata.accepts("2026-09-19", "docs"));
    assert!(!metadata.accepts("2026-09-18", "andere"));
}

#[test]
fn rrf_gewichtet_raenge_statt_rohscores() -> Result<()> {
    let config = Config::default();
    let catalog = rank::Catalog::new(&knowledge(), &config)?;
    let hits = rank::fuse(&[0, 1], &[1, 2], &catalog, &config);
    assert_eq!(hits[0].index, 1);
    assert!((hits[0].score - (1.0 / 61.0 + 1.0 / 62.0)).abs() < 1e-12);
    Ok(())
}

#[test]
fn rrf_dedupliziert_und_sortiert_deterministisch() -> Result<()> {
    let config = Config::default();
    let catalog = rank::Catalog::new(&knowledge(), &config)?;
    let first = rank::fuse(&[0, 0, 1], &[2, 1, 1], &catalog, &config);
    let second = rank::fuse(&[0, 1], &[2, 1], &catalog, &config);
    assert_eq!(
        first
            .iter()
            .map(|hit| (hit.index, hit.score))
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|hit| (hit.index, hit.score))
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn quellenkonsens_bestaetigt_den_bm25_chunk_ohne_dense_chunk_zu_duplizieren() -> Result<()> {
    let knowledge = KnowledgeBase::from_chunks(vec![
        chunk("a.html", "2026-09-18", "Lexikalischer Treffer"),
        chunk("a.html", "2026-09-18", "Semantischer Treffer"),
        chunk("b.html", "2026-09-18", "Andere Quelle"),
    ]);
    let mut config = Config {
        source_consensus_weight: 0.5,
        ..Config::default()
    };
    let catalog = rank::Catalog::new(&knowledge, &config)?;
    let boosted = rank::fuse(&[0, 2], &[1], &catalog, &config);
    let boosted_bm25 = boosted
        .iter()
        .find(|hit| hit.index == 0)
        .expect("BM25-Repräsentant fehlt")
        .score;
    let dense_same_source = boosted
        .iter()
        .find(|hit| hit.index == 1)
        .expect("Dense-Treffer fehlt")
        .score;

    config.source_consensus_weight = 0.0;
    let neutral = rank::fuse(&[0, 2], &[1], &catalog, &config);
    let neutral_bm25 = neutral
        .iter()
        .find(|hit| hit.index == 0)
        .expect("Neutraler BM25-Repräsentant fehlt")
        .score;

    assert!(boosted_bm25 > neutral_bm25);
    assert_eq!(
        dense_same_source,
        neutral
            .iter()
            .find(|hit| hit.index == 1)
            .expect("Neutraler Dense-Treffer fehlt")
            .score
    );
    Ok(())
}

#[test]
fn bm25_filtert_vor_top_k() -> Result<()> {
    let mut config = Config {
        bm25_k: 1,
        ..Config::default()
    };
    config.metadata.stand_min = Some("2026-09-18".into());
    let knowledge = knowledge();
    let catalog = rank::Catalog::new(&knowledge, &config)?;
    assert_eq!(
        catalog.bm25(&knowledge, "Steam verknüpfen", &config),
        vec![1]
    );
    Ok(())
}

#[test]
fn nullgewicht_entfernt_einen_retrievalzweig() -> Result<()> {
    let config = Config {
        dense_weight: 0.0,
        ..Config::default()
    };
    let catalog = rank::Catalog::new(&knowledge(), &config)?;
    assert_eq!(rank::fuse(&[0], &[1], &catalog, &config).len(), 1);
    Ok(())
}

#[test]
fn aktualitaet_und_quellenprioritaet_werden_rangsignal() -> Result<()> {
    let mut config = Config::default();
    config.metadata.recency_weight = 1.0;
    let catalog = rank::Catalog::new(&knowledge(), &config)?;
    let ranked = rank::fuse(&[0], &[1], &catalog, &config);
    assert_eq!(ranked[0].index, 1);
    let neutral = config
        .metadata
        .multiplier("2026-09-18", "docs", catalog.newest);
    config.metadata.source_weights.insert("docs".into(), 2.0);
    assert_eq!(
        config
            .metadata
            .multiplier("2026-09-18", "docs", catalog.newest),
        neutral * 2.0
    );
    Ok(())
}

#[test]
fn reranker_sortiert_ohne_neue_kandidaten_zu_erfinden() -> Result<()> {
    let config = Config::default();
    let catalog = rank::Catalog::new(&knowledge(), &config)?;
    let ranked = rank::fuse(&[0, 1], &[], &catalog, &config);
    let result = rank::apply_rerank(&ranked, &[-4.0, 5.0], &catalog, &config)?;
    assert_eq!(result[0].index, ranked[1].index);
    let tied = rank::apply_rerank(&ranked, &[0.0, 0.0], &catalog, &config)?;
    assert_eq!(tied[0].index, ranked[0].index);
    Ok(())
}

#[test]
fn reranker_lehnt_nan_und_falsche_anzahl_ab() -> Result<()> {
    let config = Config::default();
    let catalog = rank::Catalog::new(&knowledge(), &config)?;
    let ranked = rank::fuse(&[0], &[], &catalog, &config);
    assert!(rank::apply_rerank(&ranked, &[], &catalog, &config).is_err());
    assert!(rank::apply_rerank(&ranked, &[f32::NAN], &catalog, &config).is_err());
    Ok(())
}

#[test]
fn fehlendes_rerankmodell_ist_fehler_ohne_download() -> Result<()> {
    let dir = tempfile::tempdir()?;
    assert!(rerank::load(dir.path(), &Config::default()).is_err());
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
    let source = include_str!("rerank.rs");
    assert!(
        !source.contains("reqwest") && !source.contains("hf_hub") && !source.contains("https://")
    );
    Ok(())
}

#[test]
fn messung_trennt_quellentreffer_von_lexikalisch_zulaessigem_beleg() {
    let case = quality::Case {
        question: "Steam verknüpfen".into(),
        answerable: true,
        expected_sources: vec!["a.html".into()],
        context_terms: vec!["Steam".into()],
        answer_terms: vec!["Steam".into()],
        forbidden_terms: Vec::new(),
        herkunft: None,
    };
    let mut chunks = vec![chunk("a.html", "2026-09-18", "Steam verknüpfen")];
    let valid = quality::measure(&case, &chunks);
    assert!(valid.hit);
    assert_eq!(valid.grounded_selection_possible, Some(true));
    chunks[0].passages[0].body = "Steam".into();
    let blocked = quality::measure(&case, &chunks);
    assert!(blocked.hit && blocked.expected_source_lost_to_relevance);
    assert!(blocked.answer_terms_lost_to_relevance);
    assert_eq!(blocked.grounded_selection_possible, Some(false));
    let negative = quality::Case {
        answerable: false,
        expected_sources: Vec::new(),
        context_terms: Vec::new(),
        answer_terms: Vec::new(),
        ..case
    };
    assert_eq!(
        quality::measure(&negative, &chunks).grounded_selection_possible,
        None
    );
}

#[test]
fn messbereiche_verweigern_luecken_ausserhalb_der_golden_suite() -> Result<()> {
    assert_eq!(
        bench::Plan {
            rounds: 1,
            first: 112,
            count: Some(142)
        }
        .range(254)?,
        112..254
    );
    assert_eq!(
        bench::Plan {
            rounds: 1,
            first: 112,
            count: None
        }
        .range(254)?,
        112..254
    );
    for (first, count) in [
        (0, Some(0)),
        (0, Some(255)),
        (254, None),
        (254, Some(1)),
        (usize::MAX, Some(1)),
    ] {
        assert!(bench::Plan {
            rounds: 1,
            first,
            count
        }
        .range(254)
        .is_err());
    }
    Ok(())
}

#[path = "tests_async.rs"]
mod asynchronous;
