# Plan: Discord-Onboarding aktuelle Ränge

status: aktiv
datum: 2026-09-07
klasse: mittel
research: RESEARCH.md

## Ziel

Fertig, wenn ein Join die Rang-Frage mit Acolyte/Sentinel/Mystic/… und den
aktuellen Rangbildern sieht, die Extra-Prompts noch da sind, und
`onboarding-apply` denselben Stand nicht zurückdreht.

## Nicht-Ziele

- Keine neuen Rollen, kein Obscurus, kein Rang-Guide, kein Steam-Bot.

## Milestones

### M1 — Overlay-Test wird rot, weil der Builder den Rang-Prompt 1:1 kopiert
Änderungen: Test in `serversync.rs` umschreiben (Titel Acolyte bei echter
Unverifiziert-Rollen-ID, Extra-Prompts bleiben). Noch kein Produktivcode.
Erwarteter Zwischenzustand: neuer/angepasster Test fällt auf main durch,
weil der Builder Alchemist durchreicht.
Validierung: `cargo test -p dl-bot --features testing --include-ignored overlay_rank` bzw. der konkrete Testfilter nach dem Schreiben.
Stop-Regel: Test ist schon grün ohne Codeänderung → Overlay-Annahme falsch, Plan stoppen.

### M2 — Builder overlayt Rang-Titel/Emojis und behält Extra-Prompts
Änderungen: `overlay_rank_prompt` in `serversync.rs`, Extra-Prompt-Kopie für
Starthilfe und Steam, Konstanten-Tabelle Rolle→Titel/Emoji (IDs nach M3).
Erwarteter Zwischenzustand: M1-Test grün mit Platzhalter-Emoji-IDs oder nach
M3 mit echten IDs.
Validierung: derselbe Test plus bestehende Onboarding-Tests.
Stop-Regel: Extra-Prompt geht verloren oder Option-IDs fallen weg.

### M3 — Guild-Emojis aus der Deadlock-API, IDs in den Konstanten
Änderungen: Live-Upload (alte Rang-Emojis umbenennen, neue mit aktuellem Bild
anlegen, Onboarding+LFG auf neue IDs, alte löschen). Konstanten in
`serversync.rs` und `lfg_panel.rs`.
Erwarteter Zwischenzustand: Guild hat `acolyte`/`sentinel`/`mystic`/… mit
API-Bildern; Code kennt die IDs.
Validierung: GET `/guilds/{id}/emojis` listet die neuen Namen; Onboarding-GET
noch alt, bis M4.
Stop-Regel: Upload 403/Payload-zu-groß → Bild verkleinern, nicht überspringen.

### M4 — Namenslisten und Live-Onboarding-PUT
Änderungen: RANK_ORDER und Parser auf aktuelle Namen plus Aliasse; Live-PUT
der Onboarding-Config (volle fünf Prompts, nur Rang-Optionen Titel+Emoji
geändert).
Erwarteter Zwischenzustand: GET Onboarding zeigt Acolyte statt Alchemist;
LFG-Select-Label Acolyte; `rank_index("acolyte") == 3`.
Validierung: GET Onboarding; `cargo test` der angefassten Crates.
Stop-Regel: PUT ändert andere Prompts oder Role-IDs.

## Verlauf

- 2026-09-07: Plan angelegt, Implementierung startet bei M1.
- 2026-09-07: M1/M2 Overlay + Extra-Prompts im Builder, Tests grün.
- 2026-09-07: Rang-Namenslisten auf API-Stand. Assets unter assets/rank-emojis/.
- 2026-09-07: Live-Emoji-Upload und Onboarding-PUT folgen beim Deploy (Guild-Mutation).
