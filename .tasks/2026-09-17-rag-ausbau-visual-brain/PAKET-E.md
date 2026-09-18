# Paket E: Visual Brain

Repo: claude-config (`/home/nathanael/Documents/claude-config`), Ordner `tools/graphify-site/` (node: `app.mjs`, `graph-model.mjs`, `build-site.mjs`, `refresh-all.mjs`, `styles.css`, `tests/`). Worktree: `~/.worktrees/claude-config-visual-brain`, Branch `feat/visual-brain` ab origin/main. Live: `graphify-architecture.service` liefert `~/.graphify/site` auf 127.0.0.1:8787, `graphify-refresh.service` baut die Seite über `refresh-all.mjs`. Datenquelle `~/.graphify/global-graph.json` (rund 89k Knoten). Gemeinsame Regeln: PAKETE.md in `Deadlock-Bots/.tasks/2026-09-17-rag-ausbau-visual-brain/`.

## Ziel

Eine Obsidian-artige Brain-Ansicht: Force-Directed-Graph, der Code-Entitäten aus graphify und Korpus-Entitäten aus Deadlock-Docs (`public/` Seiten, `internal/wissensbasis/` Docs) verbindet und navigierbar macht. Bestand ausbauen, nicht neu bauen: die vorhandene Architekturkarte bekommt die Ansicht als zusätzliche Seite oder Modus.

## Scope

1. **Korpus-Knoten.** `graph-model.mjs` um Korpus-Entitäten erweitern: je Docs-Seite ein Knoten mit Titel, Pfad, `stand`, `quelle`; Kanten zu Code-Knoten aus den in den Docs genannten Datei- und Symbolbelegen (Datei plus Zeile) und aus `quellen.json`. Deadlock-Docs liegt unter `/home/nathanael/repos/Deadlock-Docs`.
2. **Ansicht.** Force-Graph (Canvas oder WebGL, vorhandene Vendor-Bibliothek bevorzugen, sonst eine kleine, lokal vendored), Suche, Fokus auf einen Knoten mit Nachbarschaft, Cluster-Färbung nach Repo bzw. Korpusgruppe, Detailleiste mit Links in Datei und Doc. Schwarz-Gold im dl-brand-Look, `prefers-reduced-motion` schaltet die Simulation auf ein statisches Layout. Bei 89k Knoten nur Nachbarschaften rendern, nie den ganzen Graphen auf einmal.
3. **Refresh.** Der Bau der Korpus-Knoten hängt am bestehenden `refresh-all.mjs`, kein zweiter Timer. Tests in `tests/` für das Graph-Modell nachziehen.
4. **Betrieb.** Lokal und loopback-only bleibt. Keine externen Abrufe zur Laufzeit, keine CDN-Skripte.

## Nicht im Scope

- Kein Rust-Neubau des Dienstes (bestehendes Node-Werkzeug wird erweitert, bewusste Ausnahme). Keine Änderung an dl-knowledge oder an Docs-Inhalten.

## Fertig

- Brain-Ansicht baut über `refresh-all.mjs`, Tests grün, Screenshot des Ergebnisses in der Akte (`VISUAL-BRAIN.png`) plus kurze Bedienbeschreibung `VISUAL-BRAIN.md`.
- Push des Branches, Fertigmeldung im Thread. Kein Merge, kein Restart des Live-Dienstes.
