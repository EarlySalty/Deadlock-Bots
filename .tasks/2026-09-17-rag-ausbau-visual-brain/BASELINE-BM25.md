# BM25-Baseline für Paket D

Stand: 2026-09-18. Retrieval-only, kein LLM-Aufruf.

## Messbasis

Die Baseline verwendet die 224 unveränderten Golden-Fälle des bisherigen Stands: 212 antwortbare und 12 bewusst unbeantwortbare Fragen. Die Rohwerte stammen aus dem BM25-Zweig der vollständigen Paket-C-Messung, deren BM25-Pfad gegenüber `main` unverändert ist. Korpus: 109 öffentliche HTML-Dateien, 603 Chunks.

Korpus-SHA-256: `0b49cedcf95529b13013d407a71186214ae332b0fdb467cffa8aef055ffdd6ce`

Golden-SHA-256: `53c03b22b5f1b00ece02491dd1c76cfd5f247544ee8bc5a47fc2767fae9b8841`

## Baseline

| Metrik | BM25 |
|---|---:|
| Recall@1 | 75,47 % |
| Recall@3 | 96,70 % |
| Recall@5 | 98,11 % |
| MRR | 0,8539 |
| Citation-Correctness, erwartete Quelle in Top-6 | 98,58 % |
| Abstain-Rate auf unbeantwortbaren Fällen, Retrieval-Surrogat | 8,33 % |
| Fehl-Antwort-Rate auf unbeantwortbaren Fällen, Retrieval-Surrogat | 91,67 % |

Abstain- und Fehl-Antwort-Rate sind im Harness bewusst Retrieval-Metriken: Ein Fall gilt als Retrieval-Abstain, wenn nach der bestehenden lexikalischen `candidate_is_relevant`-Prüfung kein Kandidat übrig bleibt. Daraus wird keine Live-LLM-Abstention behauptet.

## Reproduktion mit Paket D

Nach Merge des Harness auf einen Teststand:

```bash
cargo run --package dl-knowledge --   eval bm25   /pfad/zu/Deadlock-Docs/evals   /pfad/zum/public-korpus   /tmp/bm25.json   /tmp/bm25.md
```

Der Harness lädt den Korpus lokal wie der Dienst und ruft keine Antwortgeneration auf.
