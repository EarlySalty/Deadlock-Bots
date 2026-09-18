# Blue/Green-Vergleich für Paket D

Stand: 2026-09-18. Vergleich auf denselben 224 bisherigen Golden-Fällen, ohne LLM-Aufruf.

| Metrik | BM25 | Hybrid | Hybrid + Reranker |
|---|---:|---:|---:|
| Recall@1 | 75,47 % | 80,66 % | 80,19 % |
| Recall@3 | 96,70 % | 93,40 % | 95,75 % |
| Recall@5 | 98,11 % | 96,23 % | 97,17 % |
| MRR | 0,8539 | 0,8727 | 0,8744 |
| Citation-Correctness, erwartete Quelle in Top-6 | 98,58 % | 96,23 % | 98,11 % |
| Retrieval-Abstain auf 12 negativen Fällen | 8,33 % | 8,33 % | 8,33 % |

Der Hybrid-Pfad verbessert den ersten Treffer und MRR, verliert aber gegenüber BM25 Recall@3/5 und Top-6-Citation-Correctness. Der Reranker holt einen Teil davon zurück, liegt bei Citation-Correctness aber weiterhin leicht unter BM25. Zusätzlich dokumentiert Paket C deutlich höhere Latenz und weniger tatsächlich renderbare Golden-Belegfälle als BM25.

Damit ist die Umschaltbedingung aus der Phase-2-Spec auf diesem Stand nicht erfüllt. Das ist keine Produktionsumschaltung: Hybrid bleibt standardmäßig aus.

## Nutzung des D-Harness mit B/C

Paket C schreibt bereits einen vollständigen JSON-Bericht mit getrennten Feldern für Dense-only, Hybrid und Reranker. Paket D liest genau diesen Vertrag:

```bash
dl-knowledge eval dense <evals> <public> dense.json dense.md MESSUNG-C.json
dl-knowledge eval hybrid <evals> <public> hybrid.json hybrid.md MESSUNG-C.json
dl-knowledge eval hybrid-reranker <evals> <public> reranker.json reranker.md MESSUNG-C.json
```

Der Harness verlangt Fall-für-Fall dieselbe Frage, dasselbe `answerable`-Label und dieselben erwarteten Quellen. Ein Bericht eines anderen Golden-Stands wird fail-closed abgelehnt.
