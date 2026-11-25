status: aktiv (2026-10-02)

# Briefing: Spielerpool A

[Orchestrator] Paket A: Schema und Datenschicht. Auftrag vollständig lesen:
`/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/AUFTRAG.md`.
Paketgrenzen und Session-Vertrag stehen in `PAKETE.md` desselben Ordners.

- Worktree: `/home/nathanael/.worktrees/Deadlock-Bots-pool-a`
- Branch: `feat/spielerpool-a`
- Intent-Thread für Rückfragen, Bump-ups und Fertigmeldungen: `011b713c-266a-46b6-b0ce-1d5faa060234`
- Rolle: `worker_mittel`

## Aufgabe und Grenzen

Eigenen Worktree von frisch geholtem `origin/main` anlegen. Im geteilten Checkout
liegt fremde uncommittete Arbeit. Du bist nicht allein im Repo; fremde Änderungen
nicht zurücksetzen und keinen Checkout-Wechsel im geteilten Baum ausführen.

Du besitzt die neue Migration in `rust/crates/dl-central-db/migrations/`, das neue
Crate `rust/crates/dl-pool/`, Workspace-`Cargo.toml`, nötige `.sqlx`-Metadaten und
den Löschpfad `rust/crates/dl-community/src/privacy.rs`. Nötige Cargo-Abhängigkeiten
für diese Verdrahtung gehören dazu. Keine Dateien der Pakete B, C, D oder E bauen.

Die Datenschicht muss die Anforderungen aller Pakete tragen: strukturierte
Präferenzen je Discord-ID, bestehende `core.steam_links`, Rang, Spiele, Stunden,
Spielzeiten, Mitspieler-Paare, Session mit Teilnehmern und Kanal, Feedback per
Checkboxen, DM-Opt-in mit Default aus und vollständiges Löschen aller Pool-Daten.
Datenschutz und Guild-Zuordnung müssen für die späteren Leser eindeutig sein.
Migration erst nach frischem Fetch nummerieren, angewandte Migrationen erhalten.
Für PostgreSQL die vorhandenen Repo-Regeln nutzen.

Schreibe den verbindlichen Tabellen- und Rust-API-Vertrag nach
`/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/VERTRAG-A.md`,
damit B, C, E und D denselben Stand verwenden. Feststehende technische Grenzen
oder offene Datenfelder dort benennen, keine API-Daten erfinden.

## Regeln

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder Unter-Agenten spawnen.
- Codesuche zuerst über den Skill `code-suche` und graphify.
- Produktiven Code ausschließlich in Rust bauen; Python-Bestände nur lesen.
- Keine Code-Kommentare schreiben.
- Secrets niemals im Klartext lesen, ausgeben oder schreiben. Keine ENV-Dateien und keine Environment-Variablen für Config. Secrets aus Infisical, normale Werte aus Config-Dateien.
- Keine neuen LLM-Clients und keinen Modellwechsel.
- Alle eigenen Texte auf natürlichem Deutsch mit echten Umlauten verfassen. Die Skills `/home/nathanael/.codex/skills/humanizer/SKILL.md` und `/home/nathanael/.codex/skills/no-em-dashes/SKILL.md` lesen und anwenden. Keine Gedankenstriche als Satzpause.
- Keine Pull Requests anlegen; GitHub Actions sind kein Arbeitsauftrag.
- Nur eigenen Branch pushen. Niemals ohne Gate-ALLOW nach main mergen.
- Höchstens ein Cargo-Release-Build gleichzeitig auf dem Host. Laufende Builds prüfen und erforderliche Abstimmung an mich melden.

## Bump-up

Wird das Paket größer als beschrieben, aktuellen Stand sichern, Nachricht an den
oben genannten Delegator senden und stoppen:

`[Bump-up] Paket A: Grund: ... Erledigt: ... Worktree: ... Offen: ...`

## Fertig: Review durch den Merge-Gate

1. Angemessen prüfen, committen und eigenen Branch pushen. Aufgabenartefakte gezielt mitnehmen; REGISTER.md und PAKETE.md pflegt der Delegator, nicht überschreiben.
2. Im Worktree `python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review` ausführen. Das ist der einzige Review, keinen zusätzlichen Reviewer starten.
3. Bei ALLOW nach der Akte `/home/nathanael/.claude/skills/rolle-merge-schleuse/SKILL.md` abschließen: Merge, Push, Migration als postgres in die zentrale DB, erforderlicher Deploy und Neustart von `deadlock-bot-rust`, Live-Prüfung der Datenschicht. Branch und Worktree nach überprüftem Merge und Live-Beweis löschen. Merge-Protokoll und main-SHA melden. Für dieses Schema-Paket sind Interview und Website noch nicht live prüfbar; das ehrlich benennen.
4. Bei BLOCK nicht selbst fixen. Funde mit `pfad:zeile` als `## Paket A, Runde 1` in `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/REVIEW-A.md` schreiben, sichern und `[Review] Paket A Runde 1 BLOCK: ...` an mich senden. Auf frischen Fixer warten. Nach `[Freigabe] Merge, Deploy, Live-Prüfung, Aufräumen` dessen Branch-Stand holen und abschließen.
5. Abschluss per `python3 /home/nathanael/Documents/tools/t3-thread.py send --thread 011b713c-266a-46b6-b0ce-1d5faa060234 "[Fertig] Spielerpool A: <SHA auf main>, live geprüft: <Nachweis>, Vertrag: VERTRAG-A.md"` melden. `send` nie mit `--model`.
6. Allerletzter Schritt: `python3 /home/nathanael/Documents/tools/t3-thread.py settle --selbst`.
