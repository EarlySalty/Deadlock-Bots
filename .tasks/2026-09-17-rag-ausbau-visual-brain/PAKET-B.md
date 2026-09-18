# Paket B: Lokales Embedding und pgvector in dl-knowledge

Repo: Deadlock-Bots. Worktree: `~/.worktrees/dl-knowledge-hybrid`, Branch `feat/dl-knowledge-hybrid` (existiert, liegt hinter main: zuerst auf origin/main rebasen). Dienst: `rust/bin/dl-knowledge/src/main.rs` (eine Datei, 4383 Zeilen, Port 8896, Unit `dl-knowledge.service`, Start über `scripts/run_dl_knowledge_service.sh`). Spec: PHASE-2-SPEC.md, gemeinsame Regeln: PAKETE.md. Doku zum Bestand: `Deadlock-Docs/internal/wissensbasis/dl-knowledge-engine.md`.

## Ziel

dl-knowledge bekommt einen lokalen Dense-Pfad: Chunk-Embeddings in Postgres (pgvector), Query-Embedding zur Antwortzeit lokal im Prozess. Noch keine Fusion, noch kein Reranker (Paket C). Der BM25-Pfad und alle Fail-Closed-Garantien bleiben unverändert.

## Scope

1. **Laufweg-Messung.** all-MiniLM-L6-v2 (384 Dim) mit fastembed-rs (ONNX) gegen candle, am echten Korpus (`load_corpus`, HTML unter dem Pfad aus `DEFAULT_DOCS_PATH`, rund 109 Dateien). Messen: Index-Zeit gesamt, Query-Latenz p50/p95, RSS, Binärgröße, Trefferqualität an 30 Fällen aus `Deadlock-Docs/evals/`. Ergebnis als `MESSUNG-B.md` in die Akte. Die Wahl trifft der Nutzer; bis dahin der Weg mit der besseren Latenz bei gleicher Qualität als Default hinter einem `Embedder`-Trait, der andere Weg bleibt austauschbar. Modelldateien lokal unter `~/.local/share/dl-knowledge/models/`, kein Download zur Laufzeit im Dienst (Download als eigener Schritt im Startskript oder in der Setup-Doku).
2. **Persistenz.** Migration in `rust/crates/dl-central-db/migrations/` (Datenbank `deadlock`): Tabelle für Chunk-Embeddings mit stabiler `chunk_id`, `content_hash`, `embedding vector(384)`, `stand`, `quelle`, `doc_path`, `index_generation`. `index_generation` trägt Blue/Green: ein neuer Lauf schreibt eine neue Generation, aktiv ist genau eine. Inkrementell: unveränderter `content_hash` wird nicht neu gerechnet. Rollen: die Dienstrolle bekommt nur DML, kein CREATE. `CREATE EXTENSION IF NOT EXISTS vector` steht in der Migration; das Betriebssystempaket `postgresql-16-pgvector` fehlt auf dem Host und wird vom Betreiber installiert, im Report als Deploy-Voraussetzung nennen. Für lokale Tests eine Test-DB über `scripts/central_test_db.sh` nutzen.
3. **Dense-Suche.** Funktion, die für eine Query die Top-k Chunks per Cosine aus der aktiven Generation holt, mit `stand` und `quelle` als Rückgabefelder. Query-Text verlässt den Prozess nicht. Ein Wächtertest stellt sicher, dass es keinen HTTP-Aufruf für Embeddings gibt.
4. **Anschluss ohne Verhaltensänderung.** Der bestehende Antwortpfad nutzt den Dense-Pfad noch nicht (kommt in C). Ein Loopback-Debug-Endpunkt oder ein Subcommand für `dense_search(query)` reicht, damit D messen kann.

## Nicht im Scope

- Fusion, Reranker, Grounding-Änderungen, Concierge, dl-ai. Kein Modellwechsel bei der Generation.

## Fertig

- MESSUNG-B.md mit den Zahlen beider Laufwege.
- Migration plus Indexer plus Dense-Suche auf dem Branch, rustfmt auf eigene Dateien, clippy und Tests für dl-knowledge und dl-central-db grün, Wächtertest gegen externe Embedding-Aufrufe.
- Push des Branches, Fertigmeldung im Thread mit Deploy-Voraussetzungen. Kein Merge. Der Worktree geht danach an Paket C über.
