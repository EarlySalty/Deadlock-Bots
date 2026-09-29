status: erledigt
Datum: 2026-09-29

# Paket B: C9-Consumer und erhaltener Main-Merge

## Git-Stand und Umfang

- Worktree: `/home/nathanael/.worktrees/bots-c9-consumer-wiring`; Branch: `codex/fix-c9-consumer-wiring`; PR: https://github.com/EarlySalty/Deadlock-Bots/pull/459, weiterhin Draft.
- Der bereits konfliktfrei aufgelöste Merge wurde ohne Abort oder Reset als `00a4e1d7ab1372883a391a7df21cbdd3a07dd588` committed. Eltern: `74cc114290e6490afc5dc922ef660c34025c5f36` und `42175e5fd68a1c83bf4201b4c40cca1099f00652`.
- C9-Nachträge: `4df4e2a9ce9360db7fa7ecb9ccf841cd667252b9` (geschützte Config-Felder und Offline-Prüfung), `553e13491489a3cca9c74118fcc82e54cff3502b` (Unicode-Grenze bei Discord-Ausgabe und präzisierte Betriebsdoku). Letzter geprüfter Code-HEAD: `553e13491489a3cca9c74118fcc82e54cff3502b`.
- Eingehender Main-Merge gegen den ersten Eltern-Commit: 50 Dateien, 4902 Einfügungen, 405 Löschungen. Eigener C9-Code gegen `origin/main`, ohne diesen Bericht: 15 Dateien, 1016 Einfügungen, 82 Löschungen. Die 15 Pfade sind `.github/workflows/brain-adapter-offline.yml`, `.github/workflows/pr-release-gate.yml`, `.gitignore`, `rust/Cargo.lock`, `rust/bin/dl-bot/src/{main,modglue}.rs`, `rust/crates/dl-brain/{BRAIN_API_ADAPTER.md,Cargo.toml,src/brain_api.rs,src/lib.rs}`, `rust/crates/dl-core/src/{admin_settings.rs,runtime_config.rs,settings_catalog.tsv}`, `rust/scripts/check-brain-consumer.sh` und `tests/test_pr_release_gate_policy.py`.

## Verhalten und Grenzen

- Ohne Modusangabe bleibt `legacy` aktiv; ein ausdrücklich leerer oder unbekannter Modus scheitert beim Laden. `typed` antwortet über den lokalen BrainClient ohne Legacy-Rückfall. `shadow` liefert die Legacy-Antwort und startet die typisierte Probe unabhängig davon.
- Zufälliger Instanz-Namespace plus Sequenz verhindern ID-Kollisionen zwischen Prozessen. `build_rejected` bleibt sichtbar; `unavailable` wird als Backendfehler behandelt. Die Discord-Ausgabe begrenzt auch lange Unicode-Antworten in UTF-16-Einheiten.
- Die vier Brain-Betriebsfelder sind im Katalog geschützt und werden nicht als Browserwerte ausgeliefert. Das PR-Release-Gate ist lesend und report-only; die Policy-Suite prüft die Leserechte sowie das Fehlen der bisherigen Merge- und Branch-Update-Aufrufe.
- Keine Produktionskonfiguration, kein produktiver Consumer, kein Bot-Start, keine echten Discord- oder Twitch-Nachrichten, keine Steam-Veröffentlichung, kein Main-Merge und kein Deployment. Das unabhängige Review und jede Integration liegen beim Orchestrator; PR #459 bleibt Draft.

## Prüfungen

Die erfolgreichen Cargo-Test- und Clippy-Läufe verwendeten Cargo 1.97.1, `--locked --offline`, `SQLX_OFFLINE=true` und einen isolierten Test-Kontext ohne produktive Zugangsdaten. Die DB-Tests nutzten ein Wegwerf-Postgres über Unix-Socket `/tmp/c9-pg-1000` mit Peer-Authentifizierung; der Testserver wurde anschließend gestoppt. Testläufe mit `--include-ignored` sind unten ausdrücklich markiert.

| Befehl | Exit | Ergebnis |
| --- | ---: | --- |
| `bash rust/scripts/check-brain-consumer.sh` | 0 | Sechs Schritte: C9-Format, Bot/Core-Format, `dl-brain --all-targets --include-ignored`, Bot-`brain_ --include-ignored`, Bot-`shadow_typed_probe --include-ignored`, Brain-Clippy. 12 + 17 + 1 Tests bestanden, 0 ignoriert. |
| `cargo test --manifest-path rust/Cargo.toml -p dl-core --all-targets --locked --offline -- --include-ignored --test-threads=2` | 0 | 58 bestanden, 0 ignoriert; einschließlich Config-Katalogschutz. |
| `cargo test --manifest-path rust/Cargo.toml -p dl-knowledge --all-features --locked --offline -- --include-ignored --skip golden_retrieval_corpus --skip golden_live_api --skip six_live_questions_against_approved_snapshot --test-threads=2` | 0 | 142 bestanden, 0 ignoriert, 3 gezielt gefiltert; darunter 27 Hybrid- und 13 Eval-Tests. |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked --offline --no-run` | 0 | 79 Test-Binärziele gebaut, 0 Tests ausgeführt. Kein unkontrollierter Integrationslauf gegen Fremddienste. |
| `cargo clippy --manifest-path rust/Cargo.toml -p dl-core -p dl-brain -p dl-bot --all-targets --locked --offline -- -D warnings` | 0 | C9-betroffene Crates ohne Warnungsfehler. |
| `cargo fmt --manifest-path rust/Cargo.toml -p dl-brain -p dl-bot -p dl-core -- --check` | 0 | C9-betroffene Rust-Pfade formatiert. |
| `python3 -m unittest tests/test_pr_release_gate_policy.py` | 0 | 2 bestanden. Bestehende Policy-Tests wurden ausgeführt, nicht erweitert. |
| `gate_hook.py --review --repo <dieser Worktree> --base origin/main --head HEAD` | 0 | `ALLOW` auf Code-HEAD `553e1349`; zwei nicht blockierende Hinweise, unten eingeordnet. |

Beim ersten Knowledge-Lauf schlug der lokale DSN ohne Host schon beim Verbindungsaufbau fehl: Exit 101, 129 bestanden, 13 fehlgeschlagen. Mit `localhost` als URI-Host und unverändertem Unix-Socket lief derselbe Testsatz mit 142/142 erfolgreich. Dies war ein Fehler im Testaufruf, kein Produktfix. Die drei ausgeschlossenen Tests brauchen einen Golden-Korpus, eine freigegebene Live-API beziehungsweise einen freigegebenen Produktionssnapshot; sie zählen nicht als bestanden.

Die vollständige Workspace-Formatprüfung endet mit Exit 1: 75 Abweichungen in 15 Dateien, alle außerhalb der 15 C9-Pfade und unverändert gegenüber `origin/main`. Kein pauschales Formatieren fremder Dateien. Workspace-Clippy endet mit Exit 101 bei vier `unwrap_used`-Befunden in `dl-insights-sync/src/{brave_cdp,human}.rs`; ein früherer Durchlauf erreichte zudem `await_holding_lock` in `dl-verbinder`. Diese Pfade sind gegenüber `origin/main` unverändert. Der gezielte C9-Clippy-Lauf ist grün. Der Workspace-Testlauf wurde aus Sicherheitsgründen nur kompiliert, nicht als vollständig ausgeführte Suite ausgegeben.

Die Selbstprüfung hat einen fehlerhaften Satz zur Startregistrierung in der Betriebsdoku korrigiert und die Discord-Grenze für Unicode-Antworten abgesichert. Zum verbliebenen Längenhinweis: Der gepinnte API-Vertrag akzeptiert Antworttext bis 64 KiB, die Ausgabe kürzt jetzt nach UTF-16-Länge auf das Discord-Limit. Die URL-Regel für `answered` wurde nicht ungeprüft auf `build_rejected` übertragen, damit eine erklärende Ablehnung nicht in einen generischen Backendfehler umschlägt. Zum Policy-Hinweis: Die Suite prüft die beiden bisher verwendeten mutierenden SDK-Aufrufe; das Workflow-Rechteset selbst ist zusätzlich lesend.

TESTNACHWEIS[TW-1]: 232 passed, 0 ignored | Baseline: 0 C9-Tests rot; 3 Knowledge-Tests gefiltert
WIRKUNGSPRUEFUNG[WP-1]: 2 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 2/2 geprüft
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 6 belegt | Senke: repo-interne C9-Betriebsdoku
