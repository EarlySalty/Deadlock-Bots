# Deterministisches PR-Gate

Stand: 23. September 2026. Gilt für die Konfiguration dieses PRs, noch nicht als
Nachweis eines aktivierten GitHub-Merge-Schutzes. PR-Testbetrieb: kein Merge,
kein main-Push, kein Deploy und kein Neustart. Bestehende Schutzregeln bleiben
unverändert. Messergebnisse und verbleibende Blocker stehen in
[REPORT.md](../.tasks/2026-09-23-deterministic-pr-gate/REPORT.md).

## Bestand statt historischer Annahmen

Der aktive Code ist ein Rust-Workspace unter `rust/` mit 32 Mitgliedern,
`Cargo.lock` und eingecheckten SQLx-Offline-Metadaten. Die ältere Python/SQLite-
Architekturbeschreibung und die Behauptung einer aktiven „50+ Tools“-Suite sind
kein Beleg für den heutigen CI-Umfang. Es existieren fünf getrackte Python-Dateien:
der Infisical-Export-Kompatibilitätscode, sein pytest-Vertrag und drei Dateien
unter `.tasks/2026-09-17-rag-ausbau-visual-brain/`. Die Anwendungsskripte verwenden
die Standardbibliothek; die beiden vorhandenen Testsuiten werden beibehalten.
JavaScript liegt insbesondere unter `service/static/`; zusätzlich existiert
ein altes, weiterhin gescanntes ESLint-Tooling-Lockfile.

## Ein Abschlusscheck, keine stillen Auslassungen

`.github/workflows/required-pr-gate.yml` startet auf jedem `pull_request`, auf
`merge_group`, Push nach main, manuell und montags um 04:17 UTC. Es gibt keine
Workflow-Level-Pfad- oder PR-Zielbranchfilter. Der Abschlussjob heißt exakt
**Required PR Gate** und verwendet `if: always()` sowie direkte `needs` für alle
neun Pflichtbereiche:

| Job-ID | Verpflichtende Prüfung |
| --- | --- |
| actions | actionlint, zizmor, Gate-Vertrag und negative Gate-/SARIF-Proben |
| format | Rustfmt für gesamten Workspace und eigenständige Rust-CI-Helfer |
| rust | vollständiger Build/Clippy, bestehende Config-Verträge, Unit-/DB-/Hybridtests |
| python | Ruff für alle getrackten Python-Dateien, vorhandener pytest- und unittest-Vertrag |
| secrets | Gitleaks Arbeitsbaum/Commitprüfung und Trivy Secrets |
| dependencies | Cargo Audit einschließlich Warnungen, Cargo Deny |
| semgrep | lokale native Rust-, Python- und JavaScript/TypeScript-Regeln |
| trivy | HIGH/CRITICAL Dependency- und Infrastruktur-Befunde |
| codeql | tatsächliche Spracherkennung, vollständige Analysematrix und SARIF-Gate |

`required_gate.rs` akzeptiert ausschließlich exakt diese neun Ergebnisse mit
Wert `success`. `failure`, `cancelled`, `skipped`, `neutral`, `timed_out`, leere,
unbekannte und fehlende Ergebnisse bleiben rot. Im Testbetrieb ist **kein
Job-Skip freigegeben**. Der Vertragstest vergleicht tatsächliche Workflow-Jobs,
`needs` und Ergebnisvariablen mit der festen Pflichtliste; eine neue Prüfung
darf nicht versehentlich außerhalb des Abschlusschecks landen.

Wiederverwendbare Workflows verhindern eine zweite, abweichende PR-Testkette.
CodeQL besitzt zusätzlich einen eigenen `always()`-Abschluss für Detektion und
Matrix. Fehlende SARIF-Dateien, ungültiges JSON, fehlende Ergebnisarrays,
Scannerfehler und Warning/Error-Findings blockieren. HIGH/CRITICAL-
Security-Severity wird auch aus den SARIF-Regelmetadaten ausgewertet.
Mindestens eine ausdrücklich erfolgreiche Scanner-Ausführung muss enthalten
sein; fehlende Invocations, fehlende Erfolgsflags und Scanner-Warnungen sind
keine erfolgreiche Abnahme. Vorhandene Scanner-Meldungen müssen Arrays mit
explizitem `note` oder `none` sein. Fehlende oder unbekannte Meldungslevel sowie
strukturell ungültige Meldungen blockieren ebenfalls.
Die einzige tolerierte Operation ist das separate Archivieren fertiger
CodeQL-Berichte; Analyse und Befundauswertung sind niemals `continue-on-error`.

## Rust und isolierte Datenbank

Die nachweislich verwendete CI-Toolchain bleibt **Rust 1.88.0**. Build und Clippy
verwenden `--locked --workspace --all-targets --all-features -j 2`; Clippy endet
mit `-- -D warnings`. SQLx kompiliert mit `SQLX_OFFLINE=true`. Debug-Informationen
und inkrementelle Builds sind für diese ressourcenbegrenzten CI-Jobs abgeschaltet.
Die zuvor erfolgreichen Commands aus `discord-config-ci.yml` bleiben erhalten:
Core-Tests, Steam-Operating-, Dashboard-Visual-Brain-/Broker-Evidence-Verträge,
Twitch-/Bot-Constructor-Verträge, Laufzeit-Consumer-Check, TOML-Validierung und
`node --check service/static/operating-config.js`.

`rust/scripts/central_test_db.sh` startet ausschließlich eine neue Wegwerf-DB:

- Image: `timescale/timescaledb@sha256:252a443e2936039b83dd8da1373d01e59e932d1054fa6adf1bc061f1d56ae60a`.
- Lokal verifiziert: PostgreSQL-16-Image mit TimescaleDB 2.29.1, pgvector 0.8.1
  und pgcrypto 1.3. Alle drei Erweiterungen werden tatsächlich per SQL angelegt.
- Nur zufälliger Loopback-Port, eindeutiger Containername, kein Host-/Produktiv-
  Volume, begrenzte CPU/RAM-Ressourcen, Cleanup auch bei Fehler/Abbruch.
- `CENTRAL_TEST_DSN`, `DATABASE_URL` und `DEADLOCK_CENTRAL_DSN` werden sämtlich
  auf dieselbe neue Test-DSN überschrieben. Keine `.env` und keine produktiven Secrets.
- Readiness auf Host und im Container, Erweiterungsprüfung und echter Migrator
  müssen erfolgreich sein, bevor irgendein Test starten darf.

Dashboard-Tests verwenden nun ebenfalls den zentralen Wegwerf-Testhelfer statt
fest eingebauter Peer-Authentifizierung am Host-Socket. Die dadurch verwaiste
`tests/postgres.json` wurde entfernt. Beim Knowledge-Testhelfer hat eine gesetzte
`CENTRAL_TEST_DSN` Vorrang vor einer lokalen Peer-Konfiguration. Zwei echte
DB-Regressionstests prüfen Loopback-Host, Port und den neu erzeugten Datenbanknamen;
eine absichtlich ungültige lokale Knowledge-Konfiguration darf den Wrapper nicht
aushebeln. Fehlende oder ungültige Wrapper-DSNs werden nicht durch einen lokalen
Host-Login ersetzt.

`rust-tests.sh` akzeptiert ausschließlich die DSN des Wrappers. Workspace-Tests
laufen auch mit `--include-ignored`, damit die zahlreichen DB-Tests nicht als
unbeachtete Ignorierungen verschwinden. Das bisherige stille Überspringen des
KV-Roundtrip-Tests ohne DSN wurde entfernt. Der tatsächliche FFmpeg-Transcoder-
Test läuft mit; FFmpeg wird im CI-Runner bereitgestellt. Schema- und Scrim-
Feature-Verträge sowie die bestehenden Knowledge-Hybrid-Tests sind Pflicht.

### Elf ausdrücklich nicht als bestanden gewertete externe Abnahmen

Diese einzelnen Tests sind keine geheimnisfreie, hermetische PR-Abnahme und
werden beim Include-Ignored-Lauf namentlich ausgeschlossen, nicht ganze Crates:

| Test | Erforderliche externe Voraussetzung |
| --- | --- |
| six_live_questions_against_approved_snapshot | freigegebener öffentlicher Produktionskorpus am Betriebs-Pfad |
| golden_retrieval_corpus | gesonderter Deadlock-Docs-Korpus plus Golden-Dateien |
| golden_live_api | dieser Korpus plus laufende externe Knowledge-API |
| deadlock_sqlite3_real_snapshot_loads_and_verifies | reale alte Deadlock-SQLite-Quelle |
| tournament_snapshot_loads_and_verifies_real_data | reale alte Turnier-Datenbank |
| website_source_snapshot_loads_and_round_trips_real_data | reale alte Website-Datenbank |
| final_reconciliation_dry_run_real_copies_is_read_only | reale Quellen und freigegebener p4-final-Snapshot |
| known_source_snapshots_preserve_original_hashes_and_mtimes | reale alte Deadlock-, Website- und Turnier-Datenbanken; `etl_engine.rs` |
| barrier_full_real_run_reconciles_idempotently_and_preserves_schema | reale Quellen für vollständige ETL-Abnahme; `etl_barrier.rs` |
| barrier_converter_matrix_covers_all_real_mapped_pairs | Schema-Matrix aus realen Quellsnapshots; `etl_barrier.rs` |
| real_text_timestamptz_unix_seconds_text_fallback_is_limited_to_known_columns | Werteprüfung realer Quellsnapshots; `etl_barrier.rs` |

Die vier zusätzlichen ETL-Fälle waren im ersten PR-Stand nicht abgegrenzt.
Die Liste liegt jetzt in `.github/ci/external-rust-tests.txt`. Vor dem
Workspace-Test prüft `test_inventory.rs` die tatsächliche Cargo-Testliste:
Jede Ausnahme muss genau einen bestehenden Test treffen, auch bei libtests
Substring-Semantik für `--skip`. Fehlende, umbenannte oder kollidierende Namen
blockieren. Der Include-Ignored-Lauf verwendet einen Testthread, da ältere
ETL-Verträge dieselbe Wegwerf-Datenbank verwenden und Tabellen leeren.

`run-cargo-tests.sh` erhält Fehler- und Abbruchcodes des Testprozesses.
Ein erfolgreicher Cargo-Aufruf ohne tatsächlich bestandene Tests wird abgelehnt.
Acht negative Gegenproben decken unter anderem leere Filter, nur ignorierte
Tests, fehlende Ausführung und Abbruch ab. Sechs DB-Gegenproben prüfen fehlendes
Docker und fehlende beziehungsweise unzulässige Test-DSNs, ohne einen Docker-
Daemon oder eine Datenbank anzusprechen. Vier Inventar-Unit-Tests sichern die
enge Ausnahmeauswahl ab.

Keine dieser Voraussetzungen wird durch produktive Secrets, veränderliche
Checkouts fremder privater Repositories oder einen erfolgreichen Leer-Test
ersetzt. Die hermetischen Parser-, Retrieval-, SQLite-ETL-, DB- und Unit-Tests
bleiben im Workspace-Gate. Eine spätere Korpus-Abnahme benötigt separat
versionierte, freigegebene Fixtures; der alte private Checkout von
`Deadlock-Docs/main` ist kein reproduzierbarer PR-Vertrag.

## Scanner, Rechte und Gegenproben

Alle PR-Jobs verwenden nur `contents: read`, keine geerbten Secrets und keine
produktiven Zugangsdaten. Checkout persistiert keine Credentials. Actions sind
auf vollständige Commits gepinnt: checkout v7.0.1, CodeQL Action v4.38.0 und
upload-artifact v7.0.1. Ubuntu ist auf `ubuntu-24.04` festgelegt. Binärscanner
werden als versionierte Archive geladen und gegen eingecheckte SHA-256-Werte
geprüft, nicht über veränderliche Installationsskripte.

Tool-Versionen: Gitleaks 8.30.1, Trivy 0.74.0, actionlint 1.7.12, zizmor 1.30.1,
Semgrep 1.173.0, Cargo Audit 0.22.2, Cargo Deny 0.20.2, Ruff 0.16.3 und pytest
8.4.2. Die beiden Cargo-Scanner unterstützen Rust 1.88. Python-Test- und
Semgrep-Umgebungen besitzen separat vollständig gehashte transitive Locks.
RustSec-/Trivy-Advisory-Daten werden weiterhin aktualisiert: deterministisch
ist die Entscheidung bei gegebenen Befunden, nicht das dauerhafte Einfrieren
von Sicherheitswissen. Download-, Datenbank-, Parser- und Scannerfehler sind
kein grünes Ergebnis.

Gitleaks prüft den gesamten Arbeitsbaum und bei PRs zusätzlich `base.sha..head.sha`.
Tägliche Läufe um 03:23 UTC prüfen die vollständige Historie; keine historischen
Commits werden pauschal ignoriert. Trivy Secrets läuft unabhängig davon über
den Arbeitsbaum. Dateinamen für eingecheckte Secret-Materialien werden ebenfalls
abgelehnt. Gitleaks-Ausgaben sind redigiert; es werden keine Klartext-Secrets als
Reports hochgeladen.

Semgrep nutzt geprüfte lokale Regeln statt eines veränderlichen Registry-Packs.
Die Filter `ERROR` **und** `WARNING` enthalten native Rust-Regeln. Eine erzeugte
Rust-TLS-Gegenprobe muss mit exakt dieser Auswahl den erwarteten Rust-Regeltreffer
und Exit 1 liefern. Das Repository wird mit `--no-git-ignore` und ohne die
standardmäßigen Test-Ausschlüsse gescannt; dadurch bleibt auch der durch die
alte `.gitignore` sonst verdeckte Infisical-Exporter sichtbar. Parserfehler und
Timeouts blockieren durch `--strict`; sie werden nicht wegkonfiguriert.

Die Gegenproben erzeugen nur temporäre, nicht kompilierte Dateien in `.ci/`,
führen kein unsicheres Programm aus und löschen die Fixtures wieder. Gitleaks,
Semgrep und Trivy müssen jeweils saubere Eingaben akzeptieren, echte Testbefunde
mit Exit 1 melden und bei ungültiger Scanner-Konfiguration fehlschlagen. Trivy
prüft zusätzlich eine HIGH-Dockerfile-Gegenprobe. Hinzu kommen vier Rust-Gate-
Tests, 63 negative Gate-Prozessprüfungen, zwei CodeQL-Detektor-Tests sowie zwei
positive und 25 negative SARIF-Proben. Getrackte `.ci`-/Rust-Build-Artefakte werden
im Actions-Job abgelehnt.

CodeQL erkennt ausschließlich vorhandene getrackte Dateien für Rust, Python,
JavaScript/TypeScript und Actions. Keine erfundenen `cogs/`-Pfade und keine
Go-/Java-/Swift-/C#-Toolchains. Die gebündelten Security-Extended- und
Security-and-Quality-Suites werden analysiert und lokal ausgewertet, unabhängig
von einem SARIF-Upload. Das umgeht keine GitHub-/CodeQL-Lizenzvoraussetzungen:
Kann die Analyse nicht starten, bleibt das Gate rot. Der zusätzliche regelmäßige
CodeQL-Lauf montags um 10:44 UTC bleibt erhalten.

## Eng begründete Ausnahmen

Keine Advisory-IDs sind freigegeben. Audit-Warnungen bleiben blockierend.
HIGH/CRITICAL-Befunde werden auch ohne verfügbare Patch-Version nicht ignoriert.
Cargo Deny verbietet Wildcards sowie unbekannte Git-/Registry-Quellen. Die
21 lokalen Pfad-Deklarationen enthalten nun zusätzlich ihre bereits bestehende
Version 0.1.0; es wurde keine Paketversion erhöht und keine Publish-Regel geändert.
Parallele Semver-Linien allein sind eine Warnung, kein Sicherheitsbefund.
Die SPDX-Liste enthält die konkreten
Lizenzen der vorhandenen Abhängigkeiten, einschließlich NCSA und
CDLA-Permissive-2.0; drei interne Binaries erben die vorhandene Workspace-Lizenz.

Gitleaks: Nur generierte CI-/Build-/Git-Artefakte sowie folgende konkrete
False Positives sind konfiguriert, jeweils Pfad **UND** exakt passender Wert:

| Ort | Begründung |
| --- | --- |
| dl-ai/src/transparency.rs | genau zwei synthetische Werte bestehender Secret-Redaktions-Unit-Tests |
| rust/Cargo.lock | die zwei benachbarten öffentlichen Crate-Namen libcrux-secrets/libcrux-sha3 wurden als API-Key interpretiert |
| 2026-07-02-final-apply-report.json | rein numerische Discord-User-IDs sind keine Credentials |

Die tatsächlich enthaltenen **257 Steam-Launch-Token-Primärschlüssel** im alten
Migrationsbericht wurden dagegen aus dem Arbeitsbaum redigiert, nicht freigestellt.
Zeilen-/Entscheidungsstruktur und vorhandene Hashes bleiben erhalten. Die Git-
Historie wurde nicht umgeschrieben; Gültigkeit oder Widerruf dieser historischen
Tokens ist nicht nachgewiesen. Ein Vollhistorien-Scan darf deshalb weiterhin
anschlagen. Historien-/Credential-Bereinigung erfordert eine gesonderte,
autorisierte Entscheidung und darf nicht mit einer pauschalen Baseline verschwinden.

Semgrep: Genau der konstante Shell-Ausdruck `umask 077; exec ffmpeg "$@"` ist
vom Rust-Shell-Hinweis ausgenommen. Er setzt restriktive Dateirechte; Pfade folgen
als getrennte Argumente durch zitiertes `$@`, nicht als interpolierter Shellcode.
Der echte FFmpeg-Integrationstest bleibt Pflicht. Andere Shell-Kommandos und
abgeschaltete TLS-Prüfungen bleiben Befunde.

zizmor: Fünf einzelne lokale Reusable-Workflow-Aufrufe tragen ausschließlich
`ignore[self-repository]`. zizmor 1.30.1 empfiehlt die neue `$/`-Syntax, während
actionlint 1.7.12 sie noch als ungültig verwirft. Die verwendeten `./`-Aufrufe
referenzieren denselben Commit und sind keine veränderlichen lokalen Actions.
Die Ausnahme ist auf diese Stellen begrenzt und nach gemeinsamer Unterstützung
der neuen Syntax zu entfernen. Alle übrigen zizmor-Findings ab Low blockieren.

## GitHub-Merge-Schutz: noch nicht abgenommen

Am 23. September 2026 lieferten sowohl die Ruleset-API als auch die
Branch-Protection-API für main HTTP 403 mit der Aufforderung, GitHub Pro zu
aktivieren oder das Repository öffentlich zu machen. Das ist **kein Nachweis,
dass keine Regeln existieren**, und keine Erlaubnis, Schutzregeln zu entfernen.
Repository-Sichtbarkeit, Abrechnung und Schutzregeln wurden nicht verändert.

Der zuletzt vor dem Umbau untersuchte Main-Run 35820441759 startete wegen
fehlgeschlagener Zahlungen beziehungsweise Ausgabenlimit keinen einzigen Job.
Ein solches Infrastrukturproblem zählt nicht als erfolgreiche CI. Die neue
Prüfkette muss nach Freischaltung auf ihrem tatsächlichen PR-SHA laufen; CodeQL-
Verfügbarkeit und vollständige Runner-Abnahme sind gesondert zu belegen.

Erst nach grünem PR-Lauf, nachgewiesenen negativen Gegenproben und bestätigtem
Zugriff auf die Schutzkonfiguration darf `Required PR Gate` mit Quelle GitHub
Actions als verpflichtender Check für main eingetragen werden. Vorhandene
Required Checks/Reviews bleiben bis zur bewiesenen Ersatzabsicherung erhalten.
Die GitHub-Codeowners-Fehlerprüfung meldete zwei ungültige Einträge für den
früheren Account. `EarlySalty` wurde als Repository-Administrator bestätigt und
ersetzt diese Einträge. Die Eigentümerzuordnung benennt Workflows, Gate-Helfer,
Scannerregeln und Ausnahmekonfigurationen ausdrücklich. Sie ersetzt weder einen
aktivierten Required-Check noch eine serverseitige Review-Pflicht. Änderungen
an Workflow und Gate benötigen nach Schutzaktivierung ein Eigentümerreview.
Es wird keine neue LLM-/Copilot-Pflicht eingeführt. Ohne diese
Schutzaktivierung darf ein grüner oder roter Check nicht als bereits
nachgewiesene serverseitige Merge-Sperre bezeichnet werden.
