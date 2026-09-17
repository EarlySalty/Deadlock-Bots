# Phase 2: Retrieval-Ausbau in dl-knowledge

Grundlage: die Grok-verifizierten Phase-1-Docs (internal/wissensbasis). Leitlinie: erweitern, nicht neu bauen. Produktionscode am Community-Bot, daher vor dem Merge kurzer Hinweis an den Nutzer.

## Ziel

Aus dem reinen BM25-Retrieval von dl-knowledge wird ein Hybrid-Retrieval mit Reranker, ohne die Fail-Closed-Garantien (Abstain, Grounding, serverseitiges Rendern, Zitatbindung) zu verlieren.

## Was bleibt unangetastet

- Korpusquelle .../current/public/ als HTML, Chunking und Passagenlogik (`parse_html_file`), Produktionsvalidierung (internal/ und Nicht-HTML werden abgelehnt).
- Abstain-Kette und `grounded_response`: kein Treffer, keine relevante Passage, ungueltige Auswahl fuehren weiter zu answerable:false.
- `candidate_is_relevant` (lexikalische Belegpruefung) bleibt als Sicherheitsnetz.
- Generation: direkter `FireworksClient::from_env` (60s HTTP + 7s App-Timeout, reasoning_effort none). Kein Modellwechsel, keine neue LLM-Anbindung.
- Antwortvertrag {answerable, answer, sources} und der Concierge-Aufruf POST /public/v1/ask.

## Was dazukommt

1. **Lokales Dense-Embedding, Rust-nativ.** Query-Text ist Community-Daten und bleibt lokal, kein externer Embedding-Endpunkt. Baseline-Modell all-MiniLM-L6-v2 (384 Dim), wie in deadlock-brain bereits erprobt. Laufweg empirisch entscheiden: fastembed-rs (ONNX) gegen candle, an echtem Korpus messen (Latenz, Speicher, Trefferqualitaet), das Ergebnis dokumentieren, der Nutzer waehlt die Konfiguration.
2. **Persistenz Postgres + pgvector.** Chunk-Embeddings mit stabiler chunk_id, content_hash fuer inkrementelles Reembedding. Kein SQLite. Blue/Green: neuer Index parallel, Umschaltung erst nach Offline-Eval.
3. **Hybrid-Fusion.** BM25 (bestehend) und Dense getrennt abfragen, per RRF zusammenfuehren. BM25 bleibt der Anker fuer exakte Begriffe (Bot-Namen, Fehlertexte, Ranks), Dense faengt Paraphrasen.
4. **Reranker.** Auf die fusionierte Top-k-Kandidatenmenge ein Cross-Encoder-Rerank, dann die bestehende Passagen- und Grounding-Stufe. Reranker lokal, kein externer Dienst.
5. **Metadaten/Version-Filter.** `stand` und `quelle` sind schon Pflicht-Metadaten, werden aber nicht gespeichert oder genutzt. Sie werden in den Chunk uebernommen und als Filter- und Rangsignal verfuegbar (Aktualitaet, Quellenprioritaet).

## Grounding-Wechselwirkung (Grok-Befund)

Rein semantisch gefundene Passagen koennen weiterhin an `candidate_is_relevant` scheitern. Das Zusammenspiel Dense/Reranker mit der lexikalischen Belegpruefung muss am Golden-Set evaluiert werden, bevor die Belegpruefung gelockert wird. Quellenbindung und Abstain haben Vorrang vor Recall.

## Eval (Bruecke zu Phase 3)

Vorhandene Golden-Suite mit 224 Faellen ist der Startpunkt. Metriken Recall@k, MRR, Faithfulness, Citation-Correctness, Abstain-Rate. Hybrid muss am eigenen Testset einen messbaren Mehrwert gegenueber reinem BM25 zeigen, sonst wird nicht umgeschaltet.

## Bauweg

- Klasse gross, Rust: Opus 4.8 (opus48-coder) bzw. Pyramide, Review durch ein anderes Modell als der Autor.
- cargo fmt, clippy, test nur auf die eigenen Aenderungen.
- Merge erst nach Review, Merge-Gate und Live-Beweis; vor dem Merge Hinweis an den Nutzer (Produktionsbot).

## Offene Entscheidungen (Nutzer)

- Embedding-Laufweg fastembed-rs vs candle: wird gemessen, die Konfiguration waehlt der Nutzer.
- Reranker-Modell: Vorschlag ein kleines lokales Cross-Encoder-Modell, Auswahl nach Messung.
