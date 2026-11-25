# Prüfnachweise

Basis: `5ef78559b335389c9b73498f322f816fb24fa7da` (`origin/main` beim Start). Entwicklung ausschließlich im eigenen Branch `feat/live-streamer-voice-rights-20260924`.

## Lokale Prüfungen
`SQLX_OFFLINE=true cargo test -p dl-voice --no-run` war erfolgreich. Der vollständige Voice-Testlauf mit `rust/scripts/central_test_db.sh cargo test -p dl-voice -- --test-threads=4` bestand mit 492 erfolgreichen Tests, keinem Fehler und vier bereits bestehenden ausgeschlossenen Hilfs-/Umgebungstests. Keine Testdatenbank war mit Produktion verbunden. Das Testskript migrierte und entfernte ausschließlich seinen eigenen Timescale-Wegwerfcontainer.

Die vier ausgeschlossenen Bestandsfälle waren die drei reinen DM-Hilfsausgaben `mate_survey::tests::dump_survey_dm`, `pairing::tests::dump_pairing_dm`, `router::tests::dump_intro_dm` sowie der explizit ausgeschlossene ffmpeg-MP3-Umgebungstest. Keiner der 19 neuen Featuretests ist ausgeschlossen oder überspringt eine fehlende Datenbank stillschweigend.

`SQLX_OFFLINE=true cargo clippy -p dl-voice -p dl-bot --all-targets -- -D warnings` bestand nach den Korrekturen. Neben den neuen Tests wurden zwei schon vorhandene Clippy-Befunde minimal behoben: ein Typalias im Status-Testport und die unveränderte absteigende Längensortierung der Emoji-Zuordnung mit `sort_by_key(Reverse(...))`. Die tatsächliche Main-Binary-Einbindung wurde dadurch ebenfalls kompiliert/geprüft. Keine unterdrückten Lints und keine abgeschwächten Checks.

Die neuen Dateien und das veränderte Panel sind mit rustfmt geprüft. Unbeteiligte vorhandene Formatabweichungen in großen Bestandsdateien wurden nicht pauschal umformatiert. `git diff --check` ist sauber.

## Negative Gegenprobe
In einer isolierten Wegwerf-Datenbank wurden die Live-Prüfung, der Owner-Zielschutz und die Owner-Bindung testweise unwirksam gemacht sowie das Frischefenster unzulässig vergrößert. Der unveränderte neue Testsatz erkannte das mit **9 echten Assertion-Fehlern**, bei 10 weiterhin bestandenen Tests. Kein Compilerfehler oder fehlender DB-Zugang wurde als erfolgreiche Gegenprobe gewertet.

Die Gegenprobe traf Offline-Rechteverlust, Owner-Kick/Owner-Ban, das Ausblenden des Owners, veraltete Ban-/Unban-Menüs, Owner-Wechsel und beide Zeitstempel-Grenztests. Anschließend wurden die Quelldateien bytegetreu wiederhergestellt und der gesamte TempVoice-Teil erneut ausgeführt: **92 bestanden, 0 Fehler, 0 ausgeschlossen**. Dieser Lauf prüfte den endgültigen Stand einschließlich Panel-Texten und korrektem Owner-Anker beim Moduswechsel. Die fehlerhafte Variante wurde weder committed noch gepusht oder gestartet.

## Reproduzierbare Befehle
```sh
cd rust
SQLX_OFFLINE=true cargo clippy -p dl-voice -p dl-bot --all-targets -- -D warnings
scripts/central_test_db.sh cargo test -p dl-voice -- --test-threads=4
scripts/central_test_db.sh cargo test -p dl-voice tempvoice:: -- --test-threads=4
```

Lokale Protokolle dieser Sitzung: `/tmp/db-live-streamer-tests-20260924.log`, `/tmp/db-live-streamer-clippy-final-20260924.log`, `/tmp/db-live-streamer-counterprobe-20260924.log`, `/tmp/db-live-streamer-final-tests-20260924.log`. PR-Nummer, exakter veröffentlichter Head und tatsächliche Actions-Zustände werden im PR dokumentiert; lokale Ergebnisse werden nicht als GitHub-CI-Erfolg ausgegeben.
