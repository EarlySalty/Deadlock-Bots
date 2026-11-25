# Endgültige unabhängige Abnahme

Geprüfter SHA: `779e701421aee7b4c7cb5e92bdc48c7412d798f5`

Basis: `dd58c0afd4f410ae5624b669137d4ef46d859079`

Fertig: nein. Fix nötig: ja. Keine Merge-Freigabe für diesen SHA.

## Bestätigte Fehler

### B1: Versandauftrag kann dauerhaft verschwinden

`rust/crates/dl-bridges/src/matcher.rs:969`, `:1140` und `:820` setzen `delivered=true` vor dem tatsächlichen Discord-Aufruf und speichern diesen Zustand dauerhaft. `deliver_saved_prompts` berücksichtigt an Zeile 787 und 804 ausschließlich `delivered=false`.

Konkreter Ablauf: Ein negativer Versuch oder Review wird gespeichert; unmittelbar nach der Speicherung des vermeintlichen Versandabschlusses beendet sich der Prozess, bevor Discord die Nachricht erhalten hat. Nach dem Neustart ist die Twitch-ID fachlich abgeschlossen, der Auftrag gilt als zugestellt und wird nie wieder verschickt. Ein Review bleibt ohne benutzbaren Button; ein negativer Fall bleibt ohne manuelle Eingabeaufforderung. Der Fehler betrifft den ersten Versand und den Wiederholungsversand.

Dieselbe Verlustwirkung entsteht ohne Prozessabbruch, wenn die Speicherung im manuellen Versand fehlschlägt: Zeile 1140 verändert den Speicherzustand, Zeile 1141 kann scheitern, und der Rückweg an Zeile 1143 setzt den Zustand nicht zurück. Ein später erfolgreiches `save` schreibt dann `delivered=true`, obwohl kein Versand erfolgt ist.

Der Discord-Port besitzt derzeit keine Versandidentität: `AdapterGlue::notify` in `rust/crates/dl-bridges/src/glue.rs:112` erzeugt nur Embeds und Komponenten. `send_raw_public` reicht das JSON über `rust/crates/dl-discord/src/adapter.rs:327` unverändert an Serenity weiter. Eine persistente `nonce` mit `enforce_nonce` lässt sich darüber übermitteln. Die lokale Serenity-0.12.5-Dokumentation in `builder/create_message.rs` beschreibt deren Deduplizierung ausdrücklich nur für wenige Minuten.

Der Fix muss einen offenen Versandauftrag erhalten, bis die Zustellung belegt ist, und ungewisse Zustellungen wieder auflösen. Einfach erst nach dem Versand `delivered=true` zu speichern reicht für die geforderte Einmaligkeit nicht: Zwischen erfolgreichem Discord-Versand und Speicherung der Nachrichten-ID liegt ein unvermeidbares Ausfallfenster. Für dieses Fenster braucht es eine stabile Auftragsidentität und einen Abgleich mit Discord, besonders nach längeren Neustarts. Ein offener oder ungewisser Auftrag darf weder blind als erledigt gelten noch ohne Abgleich erneut geschickt werden. Die fachliche Entscheidung bleibt dabei abgeschlossen; sie wird nicht nochmals berechnet.

### B2: Unterbrochener Versuch bleibt nach Namenswechsel offen

`rust/crates/dl-bridges/src/matcher.rs:323` erlaubt den Wiederanlauf vorhandener `evaluating`- und `retry_auto`-Einträge, aktualisiert aber deren gespeicherten Login nicht. `LinkState::mark` schließt den ID-Eintrag an Zeile 379 ausschließlich über einen Login-Vergleich.

Konkreter Ablauf: `claim("123", "altername")` wird dauerhaft gespeichert, danach endet der Prozess vor dem Ergebnis. Das Konto heißt beim Wiederanlauf `neuername`. `claim("123", "neuername")` lässt die Prüfung zu, behält aber `altername` im ID-Eintrag. `mark("neuername", "no_match", ...)` speichert das Nein und einen manuellen Auftrag, lässt den ID-Eintrag jedoch auf `evaluating`. Jeder weitere Scan lässt dieselbe Twitch-ID wieder zu und sendet erneut die Negativmeldung. Bei Review entsteht entsprechend ein weiterer Vorschlag.

Der Abschluss des automatischen Versuchs muss dieselbe Twitch-ID verwenden wie dessen Aufnahme. Der Namenswechsel darf weder den Abschluss verhindern noch eine andere ID beeinflussen. Der vorhandene Namenswechseltest deckt nur bereits abgeschlossene Versuche ab.

## Übrige geprüfte Abläufe

Der Scan ruft den Matcher-Scorer nicht mehr auf; das Startup-Wiring verwendet `NoAi`. Im Diff bleiben die übrigen KI-Funktionen erhalten. Nur eindeutige exakte Namen können automatisch verknüpft werden; mehrdeutige Namen benötigen Review. Der vollständig paginierte Discord-REST-Abruf bricht bei Fehlern oder leerem Bestand ab, bevor ein fachliches Nein gespeichert wird. Der automatische Startscan wartet auf `CacheReady` für die Ziel-Guild. Der Admin-Reset hält den Scan-Lock.

Die atomare Dateispeicherung synchronisiert Datei und Verzeichnis und unterstützt einen Dateinamen ohne Elternverzeichnis. Beschädigte Zustandsdateien werden nicht still ersetzt. Alte `processed`, `pending` und `manual_pending`-Einträge werden übernommen und unterdrücken neue automatische Versuche. Bestehende manuelle Eingabe- und Reviewpfade bleiben registriert und bedienbar. Der angekündigte dauerhafte Config-Pfad und die tatsächliche Übernahme der 20/6 Alteinträge müssen beim Deploy geprüft werden; dieser Bericht behauptet keinen Live-Nachweis.

## Durchgeführte Prüfungen

- Tatsächlichen Diff aller drei veränderten Dateien gegen die genannte Basis und die zugehörigen Speicher-, Versand-, Button- und Startup-Pfade gelesen.
- `cargo test -p dl-bridges matcher::tests --jobs 3` mit `/home/nathanael/.cargo/bin/cargo`: 19 Tests grün.
- `cargo check -p dl-bot --jobs 3` mit derselben Toolchain: grün. Bestehende Deprecation-Warnung in `dl-brain/src/brain_api.rs`, kein neuer Fehler.
- Explizites `/home/nathanael/.cargo/bin/rustfmt --edition 2021 --check` nur für `matcher.rs`, `glue.rs` und `dl-bot/src/main.rs`: grün.
- `git diff --check` gegen die genannte Basis: grün. Worktree unverändert.

Die beiden Fehler sind durch den tatsächlichen Kontrollfluss belegt. Es wurden keine produktiven Dateien geändert und keine neuen Tests geschrieben. Nach einem Fix ist eine neue unabhängige Abnahme für den neuen SHA erforderlich.
