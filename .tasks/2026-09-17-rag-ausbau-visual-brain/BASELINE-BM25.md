# Eval: BM25

Golden-Fälle: 224 (davon synthetisch: 0). Retrieval-Limit: 6. Kein LLM-Aufruf.

| Modus | Recall@1 | Recall@3 | Recall@5 | MRR | Citation-Correctness | Abstain-Rate | Fehl-Antwort-Rate |
|---|---:|---:|---:|---:|---:|---:|---:|
| BM25 | 0.755 | 0.967 | 0.981 | 0.854 | 0.986 | 0.083 | 0.917 |

Antwortbare Fälle: 212. Unbeantwortbare Fälle: 12. Citation-Correctness bedeutet: mindestens eine erwartete Quelle liegt unter den ersten 6 Retrieval-Quellen. Abstain/Fehl-Antwort werden ohne Generation an der bestehenden lexikalischen Relevanzprüfung gemessen.
