# RAG-Ausbau + Visual Brain

Stand: 2026-09-17. Klasse: riesig, mehrtaegig, mehrere Phasen. Modus: voll autonom, keine Rueckfragen an den Nutzer ausser bei Geldwirkung, Datenverlust oder Produktfrage.

## Ziel

Saubere, versionierte Wissensbasis fuer die Zukunft, die zwei Dinge sauber bedient: die FAQ und das Beantworten von Deadlock-Fragen. Dazu eine visuelle Obsidian-artige Brain-Ansicht (Force-Directed-Graph) obendrauf. Nicht neu bauen, sondern den Bestand ausbauen und verbessern.

## Leitentscheidung (aus Deep-Research-Report abgeleitet)

Kein Greenfield-Vektor-System. Der Hebel ist die Retrieval-Pipeline, nicht die DB:

kuratierte FAQ -> Hybrid BM25 + Dense -> Metadaten/Version-Filter -> Reranker -> wenige Evidenz-Chunks -> geerdete Antwort mit Zitat oder ehrliches Abstain.

Code gehoert nicht in den direkten Antwort-Pfad. Code -> saubere interne Doku + kuratierte oeffentliche FAQ -> Retrieval.

## Bestand (Graphify-Befund, nicht neu bauen)

- dl-knowledge (Rust, Deadlock-Bots, rust/bin/dl-knowledge, Port 8896): FAQ-RAG-Dienst. Hat BM25-Index, HTML-Korpus-Chunking, Abstain (ohne BM25-Treffer kein Generator), Antwort ueber Fireworks/Deepseek. Fehlt: Dense/Embeddings, Hybrid-Fusion, Reranker, Metadaten/Version-Filter.
- Concierge (dl-community, rust/crates/dl-community/src/concierge.rs): DM-Frontend, ruft die Wissensbasis, hat Unsicher-/Abstain-Logik.
- deadlock-brain: Deadlock-Spielwissen, hat eigene Embeddings/Retrieval. Python-Anteil wird separat nach Rust portiert (eigene Akte), hier nicht anfassen.
- graphify: globaler Code-Wissensgraph (~89k Knoten, ~/.graphify/global-graph.json), Architekturkarte auf :8787. Basis fuer die visuelle Brain-Ansicht.

## Harte Leitplanken

- Produktions-Inferenz bleibt Deepseek V4 Flash ueber den zentralen Provider (tb-llm bzw. dl-ai). Union Alpha ist nur freier Batch-Generator und Eval-Werkzeug, nie in einem Bot-Pfad.
- Union Alpha (stealth/union-alpha ueber openrouter-workers) ist ein Stealth-Modell, das mitloggen darf: nur Code und interne Doku hineingeben, nie Community- oder Nutzerdaten.
- Query-Text ist Community-Daten. Embeddings zur Antwortzeit laufen lokal, nie ueber einen externen Anbieter. Lokales Embedding-Modell, Rust-nativ.
- public/ in Deadlock-Docs ist die Concierge-Quelle und wird nie automatisch beschrieben. Union Alpha schreibt nur internal/ und liefert public/-FAQ nur als Entwurf, der ueber die Schreib-Skills und ein Review laeuft.
- Persistenz Postgres (pgvector fuer Dense). Kein SQLite. Alles in Rust.
- Nutzersichtbare Texte: echte Umlaute, keine Em-Dashes, ueber community-ankuendigung bzw. rolle-doku-redakteur.

## Phasen

### Phase 1 - Korpus-Regeneration (Union Alpha, frei, Batch)
Union Alpha liest den Code der Bot-Repos und erzeugt saubere, versionierte internal/-Doku als Fundament, plus einen public/-FAQ-Entwurf (nur Entwurf). Skills: documenting-code-for-support-agents, rolle-doku-redakteur, no-em-dashes. Als Hintergrund-Skript mit eigenem Log, nicht als kurzlebiger Lauf.
Fertig: internal/-Korpus je Repo aktuell und konsistent, FAQ-Entwurf liegt zum Review bereit, nichts in public/ automatisch ueberschrieben.

### Phase 2 - Retrieval-Ausbau in dl-knowledge (Rust, Opus 4.8)
BM25 bleibt, dazu: lokales Dense-Embedding (Rust-nativ, pgvector), Hybrid-Fusion (RRF), Reranker, Metadaten- und Version-Filter, Zitat plus Abstain schaerfen. Generation bleibt am zentralen Provider.
Fertig: Hybrid zeigt am eigenen Testset messbaren Mehrwert gegenueber reinem BM25, Zitate stimmen, Abstain bei fehlender Evidenz.

### Phase 3 - Eval und Umschaltung
Golden-Set aus echten Concierge-Fragen (haeufig, Long-Tail, Tippfehler, DE/EN, Fehlercodes, bewusst unbeantwortbar). Metriken: Recall@k, MRR, Faithfulness, Citation-Correctness, Abstain-Rate. Blue/Green: neuer Index parallel, erst nach Offline-Eval umschalten.
Fertig: Scores dokumentiert, Regressionen blockieren die Umschaltung.

### Phase 4 - Visual Brain (Obsidian-Optik)
Force-Directed-Graph ueber Korpus und Code-Entitaeten, gespeist aus graphify (global-graph.json) plus den Korpus-Entitaeten. graphify-Architekturdienst (:8787) als Basis wiederverwenden und aufhuebschen, nicht neu bauen. Lokal, Rust/TS-Frontend, prefers-reduced-motion beachten.
Fertig: navigierbare Brain-Ansicht, die Korpus und Code sichtbar verbindet.

## Modell-Routing

- Korpus/Doku (Phase 1): Union Alpha (frei) ueber openrouter-workers.
- Rust-Coding (Phase 2, 4): Opus 4.8 (opus48-coder) bzw. Pyramide, Reviews durch frisches Modell.
- Eval-Skripte (Phase 3): guenstige Modelle.
- Merge/Deploy: nach Review-Gate autonom, je Phase als normaler contracted Build.

## Register

Siehe REGISTER.md.
