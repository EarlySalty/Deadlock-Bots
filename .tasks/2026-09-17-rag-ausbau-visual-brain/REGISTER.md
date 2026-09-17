# Register

Auftrag: RAG-Ausbau + Visual Brain (AUFTRAG.md). Orchestrator: Hauptsession.

## Status

| Phase | Inhalt | Modell | Status |
|---|---|---|---|
| 0 | Bestandssuche, Spec, Union-Alpha-Einbindung | - | fertig 2026-09-17 |
| 1 | Korpus-Regeneration internal/ + FAQ-Entwurf | Union Alpha schreibt, Grok CLI prueft | in Arbeit: 5 Docs auf main (Deadlock-Docs ab8cae8), 2 Grok-geprueft und korrigiert |
| 2 | dl-knowledge: Hybrid + Dense + Reranker | Opus 4.8 | offen |
| 3 | Eval Golden-Set + Blue/Green | guenstig | offen |
| 4 | Visual Brain (Force-Graph auf graphify) | Opus 4.8 | offen |

## Fakten

- Union Alpha: stealth/union-alpha, 262k Kontext, frei (0/0), Tool-Calling, ueber openrouter-workers per model-String.
- dl-knowledge: rust/bin/dl-knowledge/src/main.rs, BM25Index ab L609, load_corpus L1108, Chunk L553, Bm25Index::new L1761. Port 8896.
- Concierge: rust/crates/dl-community/src/concierge.rs, .llm_answer_with_patience L4550, .answer_dm_question_inner L3909.
- graphify: ~/.graphify/global-graph.json, Architekturkarte :8787.

## Phase-1-Befunde (aus den ersten Docs, de-riskt Phase 2)

- Live-Korpus liegt unter .../current/public/ als HTML. dl-knowledge lehnt internal/ und Nicht-HTML hart ab, unsere internal-Docs landen also nie im Live-Korpus.
- Concierge -> dl-knowledge: POST http://127.0.0.1:8896/public/v1/ask mit {"question": ...}, Loopback-only, no_proxy, keine Redirects. URL fest verdrahtet, DL_KNOWLEDGE_URL wird ignoriert.
- Antwortvertrag {answerable, answer, sources}, fail-closed. Abstain -> KNOWLEDGE_GAP_TEXT. Concierge zeigt dem Nutzer keine Quellen (nur Decision-Log).
- dl-knowledge Retrieval: reiner BM25, Generator waehlt nur Passagen-IDs, Server rendert. Golden-Suite mit 224 Faellen vorhanden (Basis fuer Phase-3-Eval).
- Union Alpha: 262k Kontext zwingt zu chunked Ingestion, keine ganzen grossen Crates am Stueck (v1-Pilot scheiterte an 515k Input).

## Worker-Threads (openrouter-workers, model stealth/union-alpha)

- 60f3bdfd9d84: Pilot v1, failed (Kontext gesprengt). Lehre: chunked lesen.
- f26fcd3d5d33: dl-knowledge-engine.md, fertig, committet.
- 906b53461eec: concierge-frontend.md, fertig, committet.

## Phase-2-Weichenstellung (aus deadlock-brain-Doku)

- Embeddings sind im Haus schon lokal geloest: sentence-transformers all-MiniLM-L6-v2, model.encode, in retrieval.py (Deadlock-Brain, Zeilen 19 bis 28). Speicher dort in SQLite.
- Phase 2 nutzt lokales Query-Embedding Rust-nativ (nicht den Python-Pfad), Modell-Baseline all-MiniLM-L6-v2 (384 Dim), Speicher pgvector in Postgres.
- Fuer den Brain-Rust-Port: SQLite dort widerspricht der Postgres-Regel, Umstieg auf Postgres gehoert in die Portierung (Akte Deadlock-Brain/.tasks/2026-09-17-brain-python-nach-rust).

## Worker-Threads (Fortsetzung)

- 5db29f15085a: deadlock-brain-retrieval.md, fertig, committet.

## Docs auf main (Stand 2026-09-17)

1. dl-knowledge-engine.md (Grok-geprueft, korrigiert)
2. concierge-frontend.md
3. deadlock-brain-retrieval.md
4. dl-ai-provider.md (Grok-geprueft, korrigiert)
5. faq-korpus-abdeckung.md (109 HTML, 8 Gruppen, 36 Heldenguides)

## Grok-Review-Befunde (Autor-Prüfer-Paar)

- Pipeline steht: Union Alpha schreibt, Grok CLI prueft headless gegen den Code, ich fixe. Prompts und Reviews liegen im Akte-Ordner (grok-review-*.md/.txt).
- dl-ai-Kernbefund: dl-ai loest Modelle NICHT dynamisch auf (einkompilierter Default deepseek-v4-flash-0731 plus Env/Param-Override), anders als tb-llm im Twitch-Bot. Drei getrennte Konsumentenpfade: dl-knowledge Direktclient (60s HTTP + 7s App, reasoning_effort none, umgeht Fabrik/Retry/Transparenz), Concierge Fabrik BotPate (110s, Retry, eigener 100s-Timeout DL_CONCIERGE_AI_TIMEOUT_SECS Deckel 110), Faq/BrainAntwort Fabrik+ChatTextGenerator (45s, reasoning_effort verworfen).
- FAQ-Luecken (public-Korpus): Spielmechaniken/Matchablauf, eigenstaendige Item-Referenz mit patchbezogenen Build-Begruendungen, Ranks/Matchmaking-Erklaerung. Heldenwissen ist stark (36 Guides).

## Naechster Schritt

Phase 1 weiter: internal-Docs fuer dl-bot-Kern und die uebrigen Kern-Crates, je chunked ueber Union Alpha, je Grok-geprueft. Danach FAQ-Entwurf (nur Entwurf, nie public/ automatisch), der die drei Luecken schliesst. Dann Phase 2 als normaler Rust-Build mit Review, Gate und Live-Beweis; vor dem Merge kurzer Hinweis an den Nutzer, da Produktionscode am Community-Bot.

## Getrennte Aufträge

- deadlock-brain Python -> Rust Portierung: eigene Akte im Repo Deadlock-Brain (nicht Teil dieses Programms).
