# Pakete: RAG-Ausbau + Visual Brain

Stand: 2026-09-18. Aufteilung des Programms aus AUFTRAG.md in fünf Pakete für Codex (Astra) in T3. Orchestrator ist die Claude-Hauptsession; je Paket genau ein T3-Thread, Briefing in `PAKET-<Buchstabe>.md` in diesem Ordner.

| Paket | Inhalt | Repo / Projekt | Start | Hängt ab von |
|---|---|---|---|---|
| A | Korpus-Rest: internal-Doku der übrigen Kern-Crates plus FAQ-Entwurf für die drei Lücken | Deadlock-Docs | sofort | nichts |
| B | Lokales Embedding (Rust-nativ) plus pgvector-Persistenz in dl-knowledge, Laufweg-Messung fastembed gegen candle | Deadlock-Bots | sofort | nichts |
| C | Hybrid-Fusion (RRF), lokaler Reranker, Metadaten- und Version-Filter in dl-knowledge | Deadlock-Bots | nach Fertigmeldung B, gleicher Branch | B |
| D | Eval-Harness: Metriken, Golden-Set-Erweiterung, BM25-Baseline, Blue/Green-Vergleich | Deadlock-Bots | sofort | nichts (misst später B+C) |
| E | Visual Brain: Force-Graph über Code plus Korpus auf der graphify-Architekturkarte | claude-config (tools/graphify-site) | sofort | nichts |

## Merge-Reihenfolge

1. A (Doku, kein Produktionscode) sobald Review durch.
2. D (Eval, nur Testpfad) sobald Review durch; liefert die Baseline-Zahlen für B+C.
3. B und C werden zusammen reviewt und zusammen gemergt (Schreib- und Lesepfad), Umschaltung nur, wenn D messbaren Mehrwert gegen BM25 zeigt. Vor dem Merge kurzer Hinweis an den Nutzer (Produktionsbot).
4. E unabhängig.

## Deploy-Voraussetzung (Betreiber)

- `postgresql-16-pgvector` ist auf dem Host nicht installiert (Kandidat 0.8.6 im pgdg-Repo, Server 16.14). Ohne das Paket kann B nicht live gehen. Installation braucht root; Paket B liefert die Migration und meldet die Voraussetzung.

## Gemeinsame Regeln für alle Pakete

- Du bist der einzige Thread für dein Paket. Keine Unter-Threads, keine Unter-Agenten.
- Arbeiten nur im eigenen Worktree und Feature-Branch. Nie nach main mergen, nur den Feature-Branch pushen. Fremde Worktrees und den geteilten Checkout nicht anfassen.
- Keine Code-Kommentare. Rust: `rustfmt <datei>` nur auf eigene Dateien (kein repoweites Formatieren, die Repos haben rote Formatter-Baselines), `cargo clippy --package <crate>`, `cargo test --package <crate>`. Persistenz nur Postgres.
- Produktions-Inferenz bleibt Deepseek V4 Flash über den zentralen Provider (dl-ai). Kein Modellwechsel, keine neue LLM-Anbindung, keine externen Embedding- oder Rerank-Endpunkte. Query-Text ist Community-Daten und bleibt lokal.
- `public/` in Deadlock-Docs wird nie automatisch beschrieben.
- Nutzersichtbare Texte und Doku: echte Umlaute, keine Em-Dashes, kein KI-Sprech.
- Secrets nie lesen oder ausgeben. Keine ENV-Dateien.
- Meldungen in den eigenen Thread: `[Fertig] Paket <X>: Branch, Commits, geprüfte Punkte, offene Punkte` oder `[Bump-up] Paket <X>: Grund, Erledigt, Worktree, Offen`. Nicht bauen, was der Auftrag nicht nennt; ist der Auftrag nicht baubar, `ABWEICHUNG:` melden und stoppen.
