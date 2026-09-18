# Paket A: Korpus-Rest und FAQ-Entwurf

Repo: Deadlock-Docs (`/home/nathanael/repos/Deadlock-Docs`). Worktree: `~/.worktrees/deadlock-docs-wissensbasis`, Branch `docs/wissensbasis-phase-1-rest` ab origin/main. Gemeinsame Regeln: PAKETE.md in `Deadlock-Bots/.tasks/2026-09-17-rag-ausbau-visual-brain/`.

## Ziel

Die interne Wissensbasis `internal/wissensbasis/` vervollständigen und einen FAQ-Entwurf für die drei bekannten Lücken des öffentlichen Korpus liefern. Fundament für Retrieval (Pakete B bis D), nicht für Endnutzer.

## Bestand (nicht neu schreiben)

Auf main liegen fünf Docs: `dl-knowledge-engine.md`, `concierge-frontend.md`, `deadlock-brain-retrieval.md`, `dl-ai-provider.md`, `faq-korpus-abdeckung.md`. Stil, Kopfzeilen (`stand`, `quelle`) und Belegform (Datei plus Zeile) dieser Docs sind der Maßstab. `faq-korpus-abdeckung.md` nennt die drei Lücken.

## Scope

1. Neue internal-Docs im gleichen Stil für die noch fehlenden Kern-Bausteine des Community-Bots aus `/home/nathanael/repos/Deadlock-Bots`: dl-bot-Kern (Eventloop, Interaktions-Router, Panels), dl-community (Verify, Onboarding, Paten, Voice-Router, Privacy-Erase), dl-central-db (Schema, Migrationsweg, Rollen), dl-web (Login-Broker, Ports). Je Baustein ein Doc, jede Aussage mit Datei und Zeile belegt, gegen den Code geprüft.
2. FAQ-Entwurf unter `internal/wissensbasis/faq-entwurf/` (drei Dateien: `spielmechanik-matchablauf.md`, `item-referenz.md`, `ranks-matchmaking.md`). Nutzersprache, so wie ein Spieler es im Discord sagen würde, echte Umlaute, keine Em-Dashes. Quellen für Spielwissen: `public/` Heldenguides, der deadlock-brain-Korpus, Patchnotes über den dl-brain-Bestand. Klar als Entwurf gekennzeichnet (`status: entwurf`), Review und Übernahme nach `public/` macht ein Mensch.
3. `quellen.json` und `tools/check_referenzen.py` für die neuen Docs nachziehen, Referenzprüfung grün.

## Nicht im Scope

- Nichts in `public/` ändern. Kein Anfassen von deadlock-brain-Python (eigene Akte). Keine Änderung an Bot-Code.

## Fertig

- Neue Docs plus FAQ-Entwurf auf dem Branch, `python3 tools/check_referenzen.py` grün, Frische-Check läuft durch.
- Nachweis im Report: je Doc die geprüften Codestellen, je FAQ-Entwurf die Quellen.
- Push des Branches, Fertigmeldung im Thread. Kein Merge.
