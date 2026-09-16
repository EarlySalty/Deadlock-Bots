# Register

Auftrag: RAG-Ausbau + Visual Brain (AUFTRAG.md). Orchestrator: Hauptsession.

## Status

| Phase | Inhalt | Modell | Status |
|---|---|---|---|
| 0 | Bestandssuche, Spec, Union-Alpha-Einbindung | - | fertig 2026-09-17 |
| 1 | Korpus-Regeneration internal/ + FAQ-Entwurf | Union Alpha (frei) | offen |
| 2 | dl-knowledge: Hybrid + Dense + Reranker | Opus 4.8 | offen |
| 3 | Eval Golden-Set + Blue/Green | guenstig | offen |
| 4 | Visual Brain (Force-Graph auf graphify) | Opus 4.8 | offen |

## Fakten

- Union Alpha: stealth/union-alpha, 262k Kontext, frei (0/0), Tool-Calling, ueber openrouter-workers per model-String.
- dl-knowledge: rust/bin/dl-knowledge/src/main.rs, BM25Index ab L609, load_corpus L1108, Chunk L553, Bm25Index::new L1761. Port 8896.
- Concierge: rust/crates/dl-community/src/concierge.rs, .llm_answer_with_patience L4550, .answer_dm_question_inner L3909.
- graphify: ~/.graphify/global-graph.json, Architekturkarte :8787.

## Worker-Threads

Noch keine. Je Paket ein Eintrag (Thread-ID, Worktree, Status).

## Getrennte Aufträge

- deadlock-brain Python -> Rust Portierung: eigene Akte im Repo Deadlock-Brain (nicht Teil dieses Programms).
