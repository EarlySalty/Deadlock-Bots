# Paket D: Eval-Harness und Golden-Set

Repo: Deadlock-Bots. Worktree: `~/.worktrees/dl-knowledge-eval`, Branch `feat/dl-knowledge-eval` ab origin/main. Gemeinsame Regeln: PAKETE.md. Bestand: Golden-Suite mit 224 Fällen in `Deadlock-Docs/evals/*.json` (sechs Dateien), geladen in `rust/bin/dl-knowledge/src/main.rs` ab Zeile 1981 (`GoldenCase`, `load_golden_cases`, `GOLDEN_CASE_COUNT`).

## Ziel

Ein reproduzierbarer Eval-Lauf, der BM25 (heute) und Hybrid (Pakete B+C) an demselben Golden-Set vergleicht und die Umschaltentscheidung mit Zahlen trägt.

## Scope

1. **Harness.** Subcommand oder eigenes Bin `dl-knowledge-eval` in Rust: lädt den Korpus wie der Dienst, läuft die Golden-Fälle gegen den Retrieval-Pfad (nicht gegen die Generation, kein LLM-Aufruf im Harness) und schreibt Recall@1/3/5, MRR, Citation-Correctness (erwartete Quelle unter den Sources), Abstain-Rate auf unbeantwortbaren Fällen, Fehl-Antwort-Rate auf unbeantwortbaren Fällen. Ausgabe als JSON plus Markdown-Tabelle. Läuft offline, ohne laufenden Dienst.
2. **Baseline.** Lauf gegen BM25 auf main, Ergebnis als `BASELINE-BM25.md` in die Akte.
3. **Golden-Set erweitern.** Neue Fälle in `Deadlock-Docs/evals/` (eigener Branch `evals/golden-erweiterung` in Deadlock-Docs, Worktree `~/.worktrees/deadlock-docs-evals`): Paraphrasen, Tippfehler, englische Fragen, Fehlercodes, bewusst unbeantwortbare Fragen. Alle Fälle synthetisch aus dem Korpus abgeleitet und als `herkunft: synthetisch` markiert. Echte Concierge-Fragen aus der Datenbank sind Community-Daten und werden in diesem Paket nicht gelesen; das Einspielen echter Fälle macht der Orchestrator lokal in einem eigenen Schritt.
4. **Vergleich.** Der Harness nimmt einen Schalter (BM25, Dense, Hybrid, Hybrid plus Reranker), damit B und C ohne Umbau gemessen werden können. Die Schnittstelle zu B über dessen Fertigmeldung im `REGISTER.md` abgleichen, nicht durch eigene Änderungen an dessen Code.

## Nicht im Scope

- Änderungen am Antwortpfad. Kein LLM im Harness. Keine Datenbank-Lesezugriffe auf Nutzerdaten.

## Fertig

- Harness auf dem Branch, rustfmt auf eigene Dateien, clippy und Tests grün, `BASELINE-BM25.md` in der Akte, neue Golden-Fälle auf dem Docs-Branch mit Zählung.
- Push beider Branches, Fertigmeldung im Thread. Kein Merge.
