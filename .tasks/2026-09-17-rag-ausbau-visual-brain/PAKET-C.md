# Paket C: Hybrid-Fusion, Reranker, Metadaten-Filter in dl-knowledge

Repo: Deadlock-Bots. Worktree: `~/.worktrees/dl-knowledge-hybrid`, Branch `feat/dl-knowledge-hybrid` (Fortsetzung von Paket B, dessen Fertigmeldung und MESSUNG-B.md zuerst lesen). Spec: PHASE-2-SPEC.md, gemeinsame Regeln: PAKETE.md.

## Ziel

Der Antwortpfad von dl-knowledge wird hybrid: BM25 und Dense getrennt, per RRF fusioniert, lokal rerankt, mit Metadaten als Filter- und Rangsignal. Abstain, Grounding (`candidate_is_relevant`, `grounded_response`), serverseitiges Rendern und der Antwortvertrag `{answerable, answer, sources}` bleiben unverändert.

## Scope

1. **Fusion.** RRF über die Top-k von BM25 (bestehend) und Dense (Paket B). k und Gewichte als Konfiguration mit Default, nicht hart im Pfad.
2. **Reranker.** Lokaler Cross-Encoder auf die fusionierte Kandidatenmenge (Vorschlag ein kleines MiniLM- oder bge-Reranker-Modell, Laufweg wie in B). Messen und in `MESSUNG-C.md` dokumentieren: Latenz p50/p95 und Qualität an den Golden-Fällen mit und ohne Reranker. Die Modellwahl trifft der Nutzer, bis dahin der gemessene Default.
3. **Metadaten.** `stand` und `quelle` aus dem Chunk in den Retrieval-Pfad: Filter (etwa nur aktueller Stand) und Rangsignal (Aktualität, Quellenpriorität). Kein neues Pflichtfeld im Korpus.
4. **Schalter.** Hybrid hinter einem Konfigurationsschalter mit Default `aus`, damit Blue/Green über D läuft: BM25-only bleibt das Live-Verhalten, bis D den Mehrwert belegt.
5. **Grounding-Wechselwirkung.** Dense-Treffer können an `candidate_is_relevant` scheitern. Nicht lockern, sondern messen und im Report zeigen, wie viele Golden-Fälle daran hängen.

## Nicht im Scope

- Änderung an Abstain-Regeln, Concierge, dl-ai, Modell der Generation. Keine externen Dienste.

## Fertig

- Hybrid-Pfad hinter Schalter, Tests grün, `MESSUNG-C.md` in der Akte.
- Push des Branches, Fertigmeldung im Thread. Kein Merge; B und C werden zusammen reviewt.
