# BM25-Baseline für Paket D

Stand: 2026-09-19. Retrieval-only, ohne LLM-Aufruf und ohne externes Embedding.

## Messbasis

Die aktuelle Baseline läuft gegen den nach Paket D erweiterten Golden-Stand aus Deadlock-Docs `main`:

- Deadlock-Bots: `980d229545c65a80ac5bea7aa9676223acce2d56`
- Deadlock-Docs: `9674a3cd51ebd4255782f56ff2d3da9d44fe3c5c`
- Golden-Fälle: 254
- Antwortbar: 236
- Bewusst unbeantwortbar: 18
- Synthetische Erweiterungen: 30

Der GitHub-Actions-Lauf verwendet Rust 1.88, checkt Deadlock-Docs `main` separat aus und führt den BM25-Harness gegen `evals/` und `public/` aus. Der Harness startet vor der Produktionsinitialisierung und ruft weder Antwortgeneration noch einen Embedding-Endpunkt auf.

## Aktuelle Baseline

| Metrik | BM25 |
|---|---:|
| Recall@1 | 73,31 % |
| Recall@3 | 95,76 % |
| Recall@5 | 97,46 % |
| MRR | 0,8381 |
| Citation-Correctness, erwartete Quelle in Top-6 | 97,88 % |
| Abstain-Rate auf unbeantwortbaren Fällen, Retrieval-Surrogat | 5,56 % |
| Fehl-Antwort-Rate auf unbeantwortbaren Fällen, Retrieval-Surrogat | 94,44 % |

Abstain- und Fehl-Antwort-Rate sind Retrieval-Metriken. Ein Fall gilt als Retrieval-Abstain, wenn nach der bestehenden lexikalischen `candidate_is_relevant`-Prüfung kein Kandidat übrig bleibt. Daraus wird keine Live-LLM-Abstention abgeleitet.

## Reproduktion

Aus `Deadlock-Bots/rust` mit einem aktuellen Checkout von Deadlock-Docs:

```bash
cargo run --package dl-knowledge -- eval bm25 ../Deadlock-Docs/evals ../Deadlock-Docs/public /tmp/bm25.json /tmp/bm25.md
```

Der CI-Lauf prüft davor Rustfmt, `cargo test --package dl-knowledge` und Clippy für den eigenen `dl-knowledge`-Code mit `-D warnings`.
