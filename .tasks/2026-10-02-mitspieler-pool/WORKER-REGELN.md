status: aktiv (2026-10-02)

# Gemeinsame Regeln für die Spielerpool-Worker

Diese Datei zusammen mit AUFTRAG.md, PAKETE.md und dem eigenen Paketbriefing vollständig lesen.
Der Delegator ist `011b713c-266a-46b6-b0ce-1d5faa060234`. Alle Rückfragen, Bump-ups,
Review-Meldungen und Fertigmeldungen gehen an diese vollständige T3-ID.

## Arbeit

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder Unter-Agenten spawnen.
- Eigenen Worktree und Branch verwenden. Du bist nicht allein im Repo. Fremde Änderungen erhalten und die eigene Arbeit daran anpassen, nichts zurücksetzen. Im geteilten Checkout keinen Branch wechseln.
- Vor Beginn frisch fetchen und den von A gemergten Tabellen- und Rust-API-Vertrag VERTRAG-A.md lesen. Fehlende Vertragsfelder vor Änderung mit dem Delegator klären.
- Nur eigene Paketdateien ändern. Benötigte Modulregistrierung und Cargo-Abhängigkeiten im eigenen Crate gehören zur Verdrahtung. Gemeinsame Dateien vorab mit dem Delegator abstimmen. Cargo.lock beim Merge auf den aktuellen main-Stand bringen.
- Codesuche zuerst über den Skill code-suche und graphify. Rust ist die Sprache für produktiven Anwendungs- und Service-Code. Python-Bestände ausschließlich lesen.
- Keine Code-Kommentare schreiben. Keine neuen LLM-Clients und kein Modellwechsel.
- Secrets niemals im Klartext lesen, ausgeben oder schreiben. Keine ENV-Dateien und keine Environment-Variablen für Config. Secrets aus Infisical, normale Werte aus Config-Dateien. Bestehende interne Tokens wiederverwenden.
- Eigene Texte auf natürlichem Deutsch mit echten Umlauten schreiben. Die Skills `/home/nathanael/.codex/skills/humanizer/SKILL.md` und `/home/nathanael/.codex/skills/no-em-dashes/SKILL.md` lesen und anwenden. Keine Gedankenstriche als Satzpause.
- Höchstens ein Cargo-Release-Build gleichzeitig auf dem Host. Vor einem Release-Build laufende Builds prüfen und erforderliche Koordination melden.
- Für die gesamte Cargo-Release-Buildphase `/tmp/deadlock-cargo-release.lock` mit flock halten. Bestehende Wrapper-Locks erhalten, identische Locks nicht erneut im Kindprozess erwerben. Fremde laufende Builds zusätzlich über ihren tatsächlichen Prozessabschluss prüfen; eine vorhandene Lockdatei beweist keinen gehaltenen Lock.
- Vom finalen Nachziehen vor Merge bis zum abgeschlossenen Live-Nachweis den Repo-Lock `/home/nathanael/Documents/.tasks/2026-10-02-offene-branches/locks/<repo>.lock` mit flock durchgehend halten. Für Deadlock-Bots, Website und caddy-config jeweils den zuständigen Lock verwenden. Einheitliche Erwerbsreihenfolge und vorhandene Deploy-Locks beachten. Slots mit dem Delegator abstimmen.
- Keine Pull Requests anlegen. GitHub Actions sind kein Arbeitsauftrag. Nur eigenen Branch pushen.
- REGISTER.md und PAKETE.md pflegt der Delegator. Nicht mit einem veralteten Worktree-Stand überschreiben.

## Bump-up

Wird das Paket größer als beschrieben, Stand sichern und an den Delegator senden,
dann stoppen:

`[Bump-up] Paket <Buchstabe>: Grund: ... Erledigt: ... Worktree: ... Offen: ...`

## Review und Abschluss

1. Angemessen prüfen, committen und eigenen Branch pushen.
2. Im Worktree `python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review` ausführen. Das ist der einzige Review. Keinen zusätzlichen Reviewer starten.
3. Bei BLOCK nicht selbst fixen. Funde mit `pfad:zeile` unter `## Paket <Buchstabe>, Runde 1` in `REVIEW-<Buchstabe>.md` des zentralen Aufgabenordners ablegen und sichern. `[Review] Paket <Buchstabe> Runde 1 BLOCK: ...` an den Delegator senden und auf den frischen Fixer warten.
4. Bei ALLOW die im Paketbriefing genannte Merge-Reihenfolge einhalten. Nach der Akte `/home/nathanael/.claude/skills/rolle-merge-schleuse/SKILL.md` mergen und pushen. Deploy, nötigen Service-Neustart und Live-Prüfung ausführen. Nach Merge und Live-Beweis Branch und Worktree wirklich löschen. Merge-Protokoll und main-SHA melden.
5. Nach einer Fix-Runde kommt `[Freigabe] Merge, Deploy, Live-Prüfung, Aufräumen` vom Delegator. Den reparierten Branch-Stand holen und ab Schritt 4 abschließen.
6. Fertigmeldung mit SHA, Nachweis und Ort im UI per `python3 /home/nathanael/Documents/tools/t3-thread.py send --thread 011b713c-266a-46b6-b0ce-1d5faa060234 "[Fertig] Spielerpool <Buchstabe>: ..."`. `send` nie mit `--model`.
7. Allerletzter Schritt: `python3 /home/nathanael/Documents/tools/t3-thread.py settle --selbst`.

Einen nur gebauten oder isoliert getesteten Pfad als solchen benennen. Steam-Login,
Einwilligungen oder Nutzeraktionen nicht behaupten, wenn sie noch nicht erfolgt sind.
