# Paket B: Messung lokaler Embeddings und pgvector

Stand: 18. September 2026. Branch: `feat/dl-knowledge-hybrid`. Basis nach Rebase: `bb03deb53b28a7266de83748c3b834bff9c9a205`.

## Abnahmestand

151 verschiedene Paket- und Datenbanktests sind grün. Die zusätzlich aktivierte Golden-Retrieval-Prüfung scheitert an einer bereits auf dem unveränderten Ausgangscode vorhandenen Scrim-Erwartung (`/scrim-signup`). Es gibt keine vollständige Golden- oder Live-Abnahme. Der genaue Rot-Grün-Nachweis und die Gegenprobe auf der Ausgangsbasis stehen in `TESTS-B.md`.

## Ergebnis und vorläufiger Default

Auf den ausgewählten 30 Fällen erreichen fastembed und Candle dieselben Trefferkennzahlen. Fastembed bleibt der vorläufige Default hinter dem `Embedder`-Trait: niedrigerer Query-Median und deutlich kürzerer Indexaufbau. Das ist kein Sieg bei allen Messgrößen. Candle hat in diesem Lauf eine etwas niedrigere p95-Latenz, einen kleineren residenten Speicherbedarf nach der Messung und ein kleineres Binary. Die endgültige Wahl bleibt beim Nutzer.

Der normale Antwortpfad verwendet Dense weiterhin nicht. Aus diesen Zahlen folgt keine Freigabe einer Produktionsumschaltung. Fusion, Reranker und der Vergleich gegen BM25 gehören zu C und D.

| Messgröße | fastembed | Candle |
|---|---:|---:|
| Öffentliche HTML-Dateien | 109 | 109 |
| Chunks | 603 | 603 |
| Frisch berechnete / wiederverwendete Embeddings | 603 / 0 | 603 / 0 |
| Indexaufbau inklusive pgvector-Schreibpfad | 56,056 s | 192,765 s |
| Modell laden und Fingerabdruck berechnen | 659,887 ms | 196,476 ms |
| Korpus laden und Dense-Datensätze vorbereiten | 133,363 ms | 47,182 ms |
| Query-Embedding p50 | 10,584 ms | 32,007 ms |
| Query-Embedding p95 | 58,016 ms | 55,863 ms |
| Query inklusive pgvector-Suche p50 | 29,820 ms | 51,146 ms |
| Query inklusive pgvector-Suche p95 | 79,349 ms | 73,153 ms |
| RSS am Ende | 341,42 MiB | 111,81 MiB |
| Höchster Prozess-RSS | 345,29 MiB | 440,73 MiB |
| Release-Binary, Bytes | 44.477.824 | 20.373.584 |
| Erwartete Quelle auf Rang 1 | 19/30 | 19/30 |
| Erwartete Quelle unter den ersten 6 Chunks | 27/30 | 27/30 |
| Mittlerer Quellen-Recall@6 | 0,761111 | 0,761111 |
| MRR@6 | 0,715000 | 0,715000 |

## Messgrundlage und Grenzen

Der reale Korpus wurde über den unveränderten Produktionslader eingelesen. Der bestehende `DEFAULT_DOCS_PATH` löste auf `/home/nathanael/.local/share/dl-knowledge/ab8cae892c186bba4a0a097ea505cef880a87d6e/public` auf. Es wurden keine Ersatzdokumente und kein synthetischer Korpus verwendet.

Gemessen wurde je ein frischer Prozess und eine eigene, frisch migrierte `deadlock_test` über `rust/scripts/central_test_db.sh`. Es gab vor dem Lauf keine Indexgeneration und keinen Embedding-Cache. Der gemessene Indexaufbau umfasst lokale Inferenz und SQL-Persistenz, aber nicht das separat ausgewiesene Laden des Modells und Korpus. Betriebssystem-Dateicaches wurden nicht geleert.

Nach drei Aufwärm-Queries wurden fünf Runden mit denselben 30 Fragen ausgeführt, insgesamt 150 Samples pro Laufweg. Die Query-Zeiten gelten bei bereits geladenem Modell. Ein neu gestartetes CLI-Kommando benötigt zusätzlich Modellinitialisierung und Verbindungsaufbau. Die Quantile verwenden den sortierten Nearest-Rank-Wert. Es sind Einzelmessungen auf einem gemeinsam genutzten Server, keine Konfidenzintervalle oder Garantien für andere Lastzustände.

Aus jeder der sechs nach Dateiname sortierten JSON-Dateien unter `Deadlock-Docs/evals/` wurden die ersten fünf beantwortbaren Fälle genommen. Die Auswahl erfolgte nicht anhand der Retrieval-Ergebnisse. Die Rohberichte nennen Fragen, Originaldatei, Zeilenposition im JSON-Array, erwartete Quellen und gerankte Dokumentpfade. Es wurden keine neuen Qualitätslabels erfunden.

Hit@6 bedeutet mindestens eine erwartete Quelle unter sechs Chunk-Treffern. Quellen-Recall@6 ist der Anteil der verschiedenen erwarteten Dokumentpfade, die unter diesen Treffern vorkommen. MRR@6 verwendet den ersten passenden Chunk-Rang. Diese Größen messen keine Antworttreue, kein Grounding und keine Abstain-Qualität. Alle ausgewählten Fälle sind laut vorhandenen Labels beantwortbar; die Gesamtsuite mit 224 Fällen wird dadurch nicht ersetzt.

Korpus-Hash beider Läufe: `0b49cedcf95529b13013d407a71186214ae332b0fdb467cffa8aef055ffdd6ce`.

Hash der ausgewählten Fälle: `6242304b39c3c961e0e5c424528332c70886c4d00fee3dbab85c90dd8d753e2e`.

## Laufwege und Umgebung

Modell: `sentence-transformers/all-MiniLM-L6-v2`, 384 Dimensionen, Revision `1110a243fdf4706b3f48f1d95db1a4f5529b4d41`. Nichtquantisiertes ONNX und Safetensors stammen aus derselben Revision. Maximal 256 Tokens, maskiertes Mean-Pooling und L2-Normalisierung. Dateiprüfsummen stehen in `MODEL-B.sha256`, der getrennte Setup-Schritt in `BETRIEB-B.md`.

Rust-Bibliotheken: fastembed 5.2.0 mit ort 2.0.0-rc.10, Candle 0.9.1, pgvector 0.4.2. Gebaut mit rustc 1.97.1, Release-Profil, getrennten Binaries mit jeweils nur einem Embedding-Feature. Eine Kompatibilitätsprüfung mit dem älteren Workspace-MSRV 1.85 wurde nicht durchgeführt.

Der Host meldete AMD EPYC 9334, 16 logische CPUs und 48 GiB RAM. Beide Messprozesse waren über `taskset -c 0,1` auf dieselben zwei logischen CPUs begrenzt; `RAYON_NUM_THREADS=2`. Testimage: `timescale/timescaledb:2.17.2-pg16`, PostgreSQL 16.6, mit vorhandener vector-Erweiterung.

Der Candle-Build wartete zunächst im Compiler-Cache. Der erfolgreiche Build lief mit `RUSTC_WRAPPER=` direkt. Das änderte keine Produktionskonfiguration. Das Candle-Binary benötigt glibc 2.39; sein Offline-Test wurde deshalb im Ubuntu-24.04-Container ausgeführt, nicht im älteren Bookworm-Container.

Rohdaten: `MESSUNG-B-fastembed.json` und `MESSUNG-B-candle.json` enthalten zusätzlich alle Einzelzeiten, Prozess-Speicherwerte und Modell-Fingerabdrücke.
