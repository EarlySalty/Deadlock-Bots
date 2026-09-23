# Abnahmeprotokoll – deterministisches PR-Gate

Stand: 23. September 2026. **Keine vollständige grüne Gesamtabnahme.**
Scope ausschließlich `EarlySalty/Deadlock-Bots`, eigener Worktree
`/home/nathanael/.worktrees/db-deterministic-pr-gate-20260923`, Branch
`ci/deterministic-pr-gate-20260923`. Basis:
`ff635f7b354cb09909c01ddd6f773d0682dd89c9`.

Der verifizierte reine Rustfmt-Commit `5c632f7f` wurde unmittelbar gepusht.
Der jeweils abschließend geprüfte Head und die Run-Zuordnung werden im PR
festgehalten; dieses Protokoll enthält keine selbstreferenzielle Commit-SHA.

## Lokal nachgewiesen

| Prüfung | Ergebnis |
| --- | --- |
| gesamter Workspace, Rustfmt 1.88 | bestanden; 20 alte Formatierungsabweichungen korrigiert |
| vollständiger Build, alle Workspace-Targets und Features | erster Zwischenstand bestanden, 10m49s; nach Dependency-/Clippy-Korrekturen erneut zu prüfen |
| bestehende gezielte Config-Testcommands | 55 Rust-Tests bestanden |
| isolierte Datenbank | gestartet, SQL-Erweiterungen tatsächlich angelegt, Migrator erfolgreich |
| Erweiterungen im geprüften Image | TimescaleDB 2.29.1, pgvector 0.8.1, pgcrypto 1.3 |
| gesamte Rust-Testsuite | erster Gesamtaufruf während Workspace-Kompilierung am MCP-Limit beendet, Exit 143; ausdrücklich nicht bestanden |
| Clippy mit -D warnings | zusätzliche Bestandswarnungen aufgedeckt; Korrektur und erneute Gesamtauswertung erforderlich |
| Python | pytest 1/1, bestehende unittest-Suite 4/4, Ruff E4/E7/E9/F bestanden |
| JavaScript | node --check für operating-config.js und visual-brain.js bestanden |
| actionlint 1.7.12 | bestanden, einschließlich aller Reusable-Workflow-Aufrufe |
| zizmor 1.30.1 | keine nicht freigegebenen Findings ab Low; fünf eng dokumentierte Syntax-Kompatibilitätsausnahmen |
| Gate-Vertrag | vier Rust-Tests, eine positive und 63 negative Prozess-Proben bestanden |
| CodeQL-Detektion | zwei Rust-Tests bestanden; tatsächliche Matrix Actions, JS/TS, Python, Rust |
| SARIF-Vertrag | eine positive und 15 negative Proben bestanden, einschließlich Severity in Regelmetadaten |
| Gitleaks 8.30.1 Arbeitsbaum | keine Findings nach echter Redigierung und enger False-Positive-Prüfung |
| Trivy Secrets 0.74.0 Arbeitsbaum | keine Findings |
| Semgrep 1.173.0 | 315 Dateien, keine Findings, keine Parser-/Timeoutfehler; alle fünf Python-Dateien enthalten |
| Scanner-Gegenproben | Gitleaks, Semgrep und Trivy akzeptieren saubere Eingaben, erkennen Testbefunde mit Exit 1 und blockieren ungültige Konfiguration |

Die vollständigen lokalen Rohprotokolle liegen ausschließlich unter `.ci/`
im eigenen Worktree und werden nicht mit potenziell sensiblen Details gepusht.
Die Tests verwenden keine Produktions-DSNs, Produktiv-Volumes oder aktiven
Bot-Zugangsdaten. Die sieben ausdrücklich externen Korpus-/Snapshot-/API-
Abnahmen sind in SECURITY-CI.md einzeln aufgeführt und **nicht** als bestanden
gezählt.

## Sicherheitsbefunde: Gate bleibt bewusst rot

Der Rustls-0.23-Zweig wurde kompatibel von 0.23.40 auf 0.23.45 aktualisiert;
rustls-webpki in diesem Zweig aktualisierte sich auf 0.103.15. Damit verschwindet
RUSTSEC-2026-0285. Das vorhandene ESLint-Lockfile verwendet jetzt
brace-expansion 5.0.12 statt 5.0.6; die drei dort gemeldeten HIGH-Befunde sind
im erneuten Trivy-Scan weg. Das alte CFLite-Compile-Marker-Dockerfile ist nicht
mehr als Root konfiguriert; Trivy meldet dafür keine HIGH/CRITICAL-
Fehlkonfiguration mehr. Dies ist kein Nachweis eines gebauten/ausgeführten
CFLite-Images und keine neue behauptete Fuzzing-Abdeckung.

**Cargo Audit bleibt mit zwölf Vulnerability-Einträgen rot:**

| Paket | Version | RustSec-IDs |
| --- | --- | --- |
| libcrux-aesgcm | 0.0.7 | 2026-0211, 2026-0209 |
| libcrux-chacha20poly1305 | 0.0.7 | 2026-0124 |
| libcrux-secrets | 0.0.5 | 2026-0212 |
| libcrux-sha3 | 0.0.8 | 2026-0208, 2026-0207 |
| ringbuf | 0.4.8 | 2026-0293 |
| rsa | 0.9.10 | 2023-0071 |
| rustls-webpki | 0.102.8 | 2026-0099, 2026-0049, 2026-0098, 2026-0104 |

Dazu kommen gemeldete Unmaintained-/Unsound-/Yanked-Warnungen. Cargo Deny
blockiert die entsprechenden Befunde im aufgelösten Graphen ebenfalls.
Lizenz-/Wildcard-Konfigurationsfehler wurden dagegen sachlich behoben: drei
interne Binaries erben die vorhandene Workspace-Lizenz, konkrete benötigte
SPDX-IDs sind gelistet, und 21 lokale Pfaddeklarationen nennen die bereits
bestehende Paketversion, ohne Versionserhöhung oder Änderung von Publish-Regeln.
Es gibt keine pauschale Advisory-Ausnahme.

**Trivy bleibt mit zwei HIGH-Befunden im Cargo-Lock rot:**
`GHSA-hc3c-63hc-2r9f` (libcrux-chacha20poly1305) und
`GHSA-82j2-j2ch-gfr8` (rustls-webpki 0.102.8). Der alte Webpki-Zweig kommt über
Serenity 0.12.5 → tokio-tungstenite 0.21.0 → Rustls 0.22.4. Ein pauschales
Update auf inkompatible Kryptobibliotheken ohne Regressionstest ist keine
Sicherheitsabnahme. Einige Advisory-Einträge nennen keine gepatchte Version.

Im historischen ETL-Bericht wurden 257 tatsächliche Credential-Primärschlüssel
redigiert, nicht als False Positive freigegeben. Die übrige Struktur und Hashes
bleiben erhalten. Keine Historienumschreibung, kein Token-Widerruf und keine
Produktivrotation wurden durchgeführt. Historische Gültigkeit ist ungeprüft;
die periodische Vollhistorien-Prüfung darf weiterhin anschlagen.

## GitHub und Schutzregeln

Vor dem Umbau geprüfter Main-Run:
https://github.com/EarlySalty/Deadlock-Bots/actions/runs/35820441759

Der Check 107050931281 hatte keine ausgeführten Schritte. GitHub meldete:
„The job was not started because recent account payments have failed or your
spending limit needs to be increased.“ Das ist ein Plattformblocker, kein
bestandener Test. Die neue PR-Run-Auswertung wird nach dem Push ergänzt.

Rulesets und Branch-Protection für main lieferten HTTP 403: Upgrade auf GitHub
Pro oder öffentliche Repository-Sichtbarkeit verlangt. Daraus lässt sich keine
vorhandene Schutzkonfiguration zuverlässig abnehmen. Sichtbarkeit, Abrechnung
und Schutzregeln bleiben unangetastet. `Required PR Gate` darf daher noch nicht
als bereits nachgewiesene serverseitige Merge-Sperre bezeichnet werden.

PR offen lassen. Kein Auto-Merge, kein main-Push, kein Deploy, kein Neustart,
keine Hook-Umgehung und keine neu eingeführte LLM-/Copilot-Pflicht.
