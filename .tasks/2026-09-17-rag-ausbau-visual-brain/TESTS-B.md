# Paket B: Testnachweise und offene Golden-Abnahme

Stand: 18. September 2026.

## Neue Tests mit Rot-Grün-Nachweis

| Gruppe | Vor Implementierung | Nach Implementierung |
|---|---|---|
| Dense-Schema und DML-Rolle | 0 bestanden, 4 fehlgeschlagen: Migration beziehungsweise Tabelle fehlte | 4 bestanden |
| Chunk-Identität, Metadaten, Vektoren, Offline-Grenze | 0 bestanden, 7 fehlgeschlagen: noch fehlende Implementierung | 7 bestanden |
| Indexierung, Aktivierung, Suche und Fehlerfälle | 0 bestanden, 6 fehlgeschlagen: Indexer noch nicht implementiert | 6 bestanden |

Die 17 neuen Tests prüfen insbesondere Metadatenübernahme ohne Textumbau, stabile IDs bei Inhaltsänderungen und eingefügten anderen Dokumenten, Inhalts-Hashes, interne oder unsichere Pfade, ungültige Vektoren, Wiederverwendung unveränderter Eingaben, Entfernen weggefallener Chunks, Modellwechsel, Rollback bei Fehlern und explizite Aktivierung mit erwartetem vorherigen Stand. Indexierung und Suche wurden außerdem unter der reinen DML-Gruppenrolle ausgeführt; ein tatsächliches CREATE TABLE wurde dabei abgelehnt.

## Paketläufe

Ausgangsprüfung vor den Änderungen, mit Standardfeatures und der Wegwerf-Testdatenbank: 105 bestanden, 0 fehlgeschlagen, 14 ignoriert.

Abschlusslauf mit allen Features:

```text
cargo test --package dl-knowledge --package dl-central-db --all-features -- --test-threads=2
127 bestanden, 0 fehlgeschlagen, 26 ignoriert, Exit 0
```

Zusätzlicher Lauf der sonst ignorierten zentralen Datenbanktests, ebenfalls über `rust/scripts/central_test_db.sh`:

```text
cargo test --package dl-central-db --all-features -- --ignored --test-threads=2
24 bestanden, 0 fehlgeschlagen, Exit 0
```

Damit wurden 151 verschiedene Tests erfolgreich ausgeführt. Die verschiedenen Ausgangs- und Abschlusszahlen sind nicht nur durch neue Tests bedingt: `--all-features` schaltet zusätzliche vorhandene DB-Tests frei.

```text
cargo clippy --package dl-knowledge --package dl-central-db --all-features --jobs 2
Exit 0, keine Warnungen
```

Die neuen Rust-Module und das neue Schema-Testfile wurden einzeln mit rustfmt formatiert. Die vorhandene `main.rs` erhielt nur den Dense-CLI-Einstieg, die aufbewahrten Metadaten und die dafür nötigen Initialisierungen in vorhandenen Fixtures. Es gab keine globale Formatierung des Repositories.

## Modell- und Offline-Prüfungen

`sha256sum --check MODEL-B.sha256`: alle sechs lokalen Modelldateien korrekt.

Beide Release-Binaries wurden in schreibgeschützten Containern mit `--network none`, ohne zusätzliche Linux-Capabilities und als UID 1000 ausgeführt. Die Modelle waren ausschließlich schreibgeschützt eingebunden. `dense check` lieferte für beide Laufwege 384 Dimensionen und den erwarteten Modell-Fingerabdruck. Fastembed wurde in `rust:1.88-bookworm`, Candle wegen seiner glibc-Anforderung in `ubuntu:24.04` geprüft.

Ein weiterer netzloser Fastembed-Lauf mit fehlendem Modellverzeichnis scheiterte wie vorgesehen mit Exit 1 und `Lokale Modelldatei fehlt oder ist unlesbar: model.onnx; Setup vor dem Dienststart ausführen`. Es wurden keine Ersatzdateien heruntergeladen.

Diese Laufprüfungen ergänzen den Quellcode-Test gegen HTTP-/Download-Pfade im lokalen Embedder und gegen aktivierte Hub-Features der fastembed-Abhängigkeit.

## Zusätzlicher Golden-Test: identischer Fehler auf der Ausgangsbasis

Der sonst ignorierte `tests::golden_retrieval_corpus` wurde zusätzlich gegen den echten öffentlichen Snapshot und die vorhandenen Eval-Dateien ausgeführt. Er scheitert am ersten beobachteten Fehler:

```text
Frage: Wie melde ich mich für Scrims an?
Fehlender Kontextterm: /scrim-signup
```

Gefunden werden dabei `discord-server/scrims.html`, `discord-server/module/dashboard-login.html`, `website/website-portale.html` und `discord-server/steam-integration.html`. Das erwartete Kommando fehlt im geprüften Kontext.

Zur Eingrenzung wurde die unveränderte `main.rs` aus Commit `bb03deb53b28a7266de83748c3b834bff9c9a205` als separater temporärer Test-Target mit derselben Cargo-Abhängigkeitsauflösung gebaut. Der aktive B-Quelltext wurde dabei nicht zurückgesetzt. Ergebnis auf der Ausgangsbasis: ebenfalls 0 bestanden, 1 fehlgeschlagen, Exit 101, dieselbe Frage, derselbe fehlende Begriff und dieselbe Trefferliste. Der temporäre Target und seine Quelldatei wurden anschließend entfernt.

Das ist keine durch B eingeführte Abweichung. Trotzdem liegt damit keine vollständig grüne Golden-Abnahme vor. Der Test bricht am ersten Fehler ab; weitere mögliche Abweichungen danach sind nicht ausgeschlossen. Weder Korpus noch Erwartungslabels wurden durch B angepasst. Dieser Abgleich bleibt für die spätere Abnahme durch D beziehungsweise die verantwortlichen Dokumentationsbearbeiter offen.

`golden_live_api` wurde nicht ausgeführt. Es gab keine Live-LLM-Abnahme, keinen Dienstneustart und keine Produktionsumschaltung.
