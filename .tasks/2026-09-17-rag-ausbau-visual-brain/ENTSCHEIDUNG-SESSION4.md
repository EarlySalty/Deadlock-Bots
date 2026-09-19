# Session 4: Produktionsentscheidung Retrieval

Stand: 19. September 2026. Entscheidungsbasis ist der aktuelle Paket-D-Golden-Stand mit 254 Fällen, davon 236 beantwortbar und 18 bewusst unbeantwortbar.

## Ergebnis

BM25 bleibt der Produktionspfad. `DL_KNOWLEDGE_HYBRID` bleibt standardmäßig `false`.

Die lokale Dense- und Hybrid-Infrastruktur bleibt hinter dem Schalter verfügbar. Es wird keine Grounding-Regel gelockert. `candidate_is_relevant`, `grounded_response`, serverseitiges Rendering und der Antwortvertrag bleiben unverändert.

## Baseline

| Metrik | BM25 |
|---|---:|
| Recall@1 | 73,31 % |
| Recall@3 | 95,76 % |
| Recall@5 | 97,46 % |
| MRR | 0,8381 |
| Citation-Correctness | 97,88 % |
| Retrieval-Abstain auf 18 negativen Fällen | 5,56 % |
| Fehl-Antwort-Rate auf 18 negativen Fällen | 94,44 % |

## Gemessene Optimierungsrichtungen

Die schnelle Hybrid-Suche ohne Cross-Encoder brachte bessere erste Treffer und teilweise bessere MRR-Werte, verlor aber Tiefe oder Quellenabdeckung. Ein Beispiel mit gleichen RRF-Gewichten erreichte Recall@1 77,97 % und MRR 0,8533, fiel aber bei Recall@3 auf 91,53 %, Recall@5 auf 95,34 % und Citation-Correctness auf 96,19 %.

Eine BM25-lastigere Variante mit `bm25_weight=1.08` erreichte Recall@1 76,69 %, Recall@5 97,03 %, MRR 0,8495 und Citation-Correctness 97,46 %. Auch diese Variante unterschreitet die BM25-Baseline bei Recall@3, Recall@5 und Citation-Correctness.

Der zusätzliche Dense-Relevanzfilter verbesserte einzelne Belegfälle, erreichte in der besten getesteten Variante aber weiterhin nur 97,46 % Citation-Correctness und 96,61 % Recall@5. Quellenkonsens und Metadatengewichtung lieferten ebenfalls keinen Kandidaten, der die Baseline ohne Rückschritt übertrifft.

Der kleinere lokale Cross-Encoder `cross-encoder/ms-marco-TinyBERT-L2-v2` senkte die Reranker-Latenz deutlich gegenüber dem zuerst gemessenen MiniLM-Reranker. Mit acht Fusionskandidaten lag die Gesamt-Retrievallatenz bei ungefähr 91 ms p50 und 105 ms p95. Citation-Correctness stieg dabei auf 98,31 %, gleichzeitig fielen Recall@1 auf 71,61 %, Recall@3 auf 92,80 %, Recall@5 auf 96,19 % und MRR auf 0,8258.

Keine gemessene Variante verbesserte die Retrieval-Abstain-Rate gegenüber BM25. Sie blieb bei 5,56 %.

## Entscheidung

Die Umschaltbedingung aus Paket D ist nicht erfüllt. Ein gemischtes Ergebnis mit besseren ersten Treffern oder besserer Citation-Correctness bei gleichzeitig schlechterem Recall oder MRR zählt nicht als Produktionsgewinn.

Deshalb wird Hybrid nicht aktiviert. Die sichere Infrastruktur kann gemergt werden, weil sie standardmäßig ausgeschaltet ist und BM25 ohne gesetzten Schalter das Live-Verhalten bleibt.

## Spätere Wiederaufnahme

Eine erneute Produktionsprüfung braucht einen Kandidaten, der gegen denselben Golden-Stand die Sicherheitsmetriken nicht verschlechtert. Sinnvolle nächste Richtungen sind ein anderes lokales Embedding-Modell, ein auf deutschsprachige Retrieval-Fragen abgestimmter Reranker oder ein selektiver Hybridpfad, der BM25-Treffer nicht aus den sechs Ausgabeslots verdrängt.
