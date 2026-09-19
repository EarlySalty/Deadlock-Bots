# Blue/Green-Vergleich für Paket D

Stand: 2026-09-19. Retrieval-only, ohne LLM-Aufruf.

## Historischer Paket-C-Vergleich

Die vorhandene Paket-C-Messung verwendet 224 Golden-Fälle. Sie bleibt als technischer Nachweis für Dense, Hybrid und Hybrid + Reranker erhalten, ist nach der Golden-Erweiterung aber nicht mehr die Entscheidungsbasis für Session 4.

| Metrik | BM25 | Hybrid | Hybrid + Reranker |
|---|---:|---:|---:|
| Recall@1 | 75,47 % | 80,66 % | 80,19 % |
| Recall@3 | 96,70 % | 93,40 % | 95,75 % |
| Recall@5 | 98,11 % | 96,23 % | 97,17 % |
| MRR | 0,8539 | 0,8727 | 0,8744 |
| Citation-Correctness, erwartete Quelle in Top-6 | 98,58 % | 96,23 % | 98,11 % |
| Retrieval-Abstain auf 12 negativen Fällen | 8,33 % | 8,33 % | 8,33 % |

Dieser Stand zeigt das bisherige Muster: bessere erste Treffer und bessere MRR, aber schwächere Tiefe und Quellenabdeckung. Daraus folgt keine Produktionsumschaltung.

## Entscheidungsbasis für Session 4

Session 4 muss Dense, Hybrid und Hybrid + Reranker gegen denselben aktuellen Golden-Stand mit 254 Fällen neu messen. Der Harness lehnt Paket-C-Berichte mit abweichenden Fragen, `answerable`-Labels oder erwarteten Quellen ab.

Für die Aussage, dass ein Kandidat BM25 auf dem aktuellen Stand schlägt, gelten diese Schwellen:

| Metrik | Aktuelle BM25-Baseline | Session-4-Ziel |
|---|---:|---:|
| Recall@1 | 73,31 % | > 73,31 % |
| Recall@3 | 95,76 % | > 95,76 % |
| Recall@5 | 97,46 % | > 97,46 % |
| MRR | 0,8381 | > 0,8381 |
| Citation-Correctness, erwartete Quelle in Top-6 | 97,88 % | > 97,88 % |
| Retrieval-Abstain auf 18 negativen Fällen | 5,56 % | > 5,56 % |
| Fehl-Antwort-Rate auf 18 negativen Fällen | 94,44 % | < 94,44 % |

Ein gemischtes Ergebnis mit Verbesserungen bei einzelnen Metriken und Rückschritten bei anderen erfüllt dieses eindeutige Vergleichskriterium nicht. Die Quellenabdeckung und das Verhalten auf unbeantwortbaren Fällen bleiben Sicherheitskriterien und dürfen für einen besseren ersten Treffer nicht verschlechtert werden.

## Nutzung des D-Harness mit B/C

Paket C schreibt einen vollständigen JSON-Bericht mit getrennten Feldern für Dense-only, Hybrid und Reranker. Paket D liest diesen Vertrag:

```bash
dl-knowledge eval dense <evals> <public> dense.json dense.md MESSUNG-C.json
dl-knowledge eval hybrid <evals> <public> hybrid.json hybrid.md MESSUNG-C.json
dl-knowledge eval hybrid-reranker <evals> <public> reranker.json reranker.md MESSUNG-C.json
```

Für Session 4 muss `MESSUNG-C.json` mit dem 254-Fälle-Golden-Stand neu erzeugt werden. Die alte 224-Fälle-Messung bleibt historische Referenz.
