# Welle2b W1 Onboarding-Fundament (2026-07-02)

## Ziel
Native Discord-Onboarding-Welle 2b lokal vorbereiten: neue Soll-Rollen, hash-gated Onboarding-Preview/Apply, `COMPLETED_ONBOARDING`-Journey-Event mit persistentem Dedupe, Legacy-Wizard-Einstiege deaktivieren. Kein Commit/Push; `CHANGELOG.md` bleibt unberuehrt.

## Fortschritt
- Rework-Implementierungsworker fuer Kritiker-Befunde 1-8 gestartet. Verbindlich: nur Worktree `Deadlock-Bots-welle2b`, kein Commit/Push, `CHANGELOG.md` unberuehrt; Abschluss mit uncommitted Working Tree fuer Claude-Review.
- Pflichtkontext gelesen: Konzept §3, `serversync.rs`, `dl-server-as-code::rules`, `dl-discord` Gateway/Dispatcher, `dl-activity::journey`, `journeyglue.rs`, `dl-community::onboarding` und Privacy-KV-Pfade.
- Umsetzungsrichtung: Onboarding-Diff als `ServerDiff`-Preview mit kuenstlichem `native-onboarding`-Objekt in `server_config.diff_previews`; Apply laedt die gebundene Ziel-Config und schreibt per Discord-REST-PUT nur bei Hash-Match. Dedupe fuer Completed-Onboarding ueber `bot.kv_store`.
- TDD gestartet: rote Tests fuer neue Rollen, Onboarding-Payload/7-5-Validierung, Completed-Onboarding-Klassifikation/Dedupe und deaktivierte Legacy-Wizard-Einstiege werden angelegt.
- Implementiert: neue Soll-Rollen `Invite-Gast`, `Frischling`, `Streams`; Native-Onboarding-Preview/Apply fuer Slash und HTTP mit 7/5-Pruefung, Platzhalter-Snowflakes und Rang-Prompt-Uebernahme; `COMPLETED_ONBOARDING`-Event mit Nachlese, weicher Klassifikation und persistentem KV-Dedupe; Alt-Wizard-Auto-Start sowie `/publish_rules_panel`/`rp:panel:start` deaktiviert, Legacy-Wizard/Privacy-Pfade bleiben erhalten.
- Platzhalterstelle aus dieser Welle: `rust/crates/dl-community/src/onboarding.rs` antwortet fuer alte Panel-/Command-Einstiege exakt mit `"Platzhalter"`, bis Claude den finalen Text liefert.
- Verifikation gruen: `cargo fmt --all`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test --workspace`; `./scripts/central_test_db.sh cargo test -p dl-bot --bin dl-bot journeyglue::tests::native_onboarding_dedupe_marker_ist_persistent_und_einmalig -- --ignored`; `git diff --check`.
- Rework umgesetzt: Onboarding-PUT serialisiert Discord-konform mit `emoji_*` statt GET-`emoji`, neue Prompts/Optionen lassen IDs weg, Rollenmatching ist exakt, Preview-Apply ist an Guild/Objekt/Message-Key/unapplied gebunden und revalidiert Live-Rollen/-Kanaele vor PUT.
- Rework umgesetzt: Native-Onboarding-MemberFlags pruefen Prozess-Dedupe und KV vor Sleep/REST; KV-Claim und Journey-Record committen atomar in einer Transaktion. Welle2b-Rolle `Streams` erbt keine Template-Permissions mehr; `/publish_rules_panel`-Deregistrierung ist am Bulk-Overwrite-Startup-Sync dokumentiert.
- Rework-Verifikation gruen: `cargo fmt --all`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test --workspace`; `./scripts/central_test_db.sh cargo test -p dl-bot --bin dl-bot onboarding -- --ignored`; `git diff --check`. Kein Commit/Push.

# Soll-Modell-Regeltransformation dl-server-as-code (2026-07-02)

## Fix Server-Sync Live-Findings (Rollback-Serialisierung + Namens-Matching)
- Implementierungsworker gestartet fuer zwei Live-Dry-Run-Bugs: Rollback-Artefakt darf kein rohes `GuildModel` mit Nicht-String-Map-Keys serialisieren; Regeln muessen Emoji-/Case-/Alias-Namen nur beim Matching erkennen. Verbindlich: keine Commits/Pushes/Deploys/Restarts/Live-Guild-Aufrufe.
- Kontext gelesen: `serversync.rs`, `rules.rs`, `format.rs`, `model.rs`, Diff/Apply-Pfade und Onboarding-Dokumente. Umsetzung geplant ueber Vec-basiertes Rollback-DTO, normalisierte Match-Keys plus kleine Alias-Tabelle, und differenziertes Delete-Label fuer Permission-Overwrites.
- Implementiert: Rollback-Artefakt nutzt `RollbackGuildModel` mit Vecs fuer Kategorien, Kanaele, Rollen, Overwrites und BotMessages; Verify/Restore baut daraus validiert wieder ein `GuildModel`. Pflicht-Roundtrip mit echten OverwriteKeys und Hash-Stabilitaet ergaenzt.
- Implementiert: Matching normalisiert nur Lookup-Schluessel (Deko an Raendern strippen, trim, lowercase); Kategorie-Aliases eng auf `Streamer Only` -> `Streamer` und `Support`/`❓Support` -> `Support/Tickets`; Kanal-Renames und Struktur-Moves nutzen denselben Matching-Pfad, Soll-Namen bleiben unveraendert ausser dokumentierten Kanal-Renames.
- Implementiert: `human_summary` unterscheidet Delete-Labels fuer Struktur/BotMessage/PermissionOverwrite; Overwrite-Delete wird nicht mehr als manueller Archivierungsschritt gelabelt.
- Verifikation gruen: `cargo fmt`; `SQLX_OFFLINE=true cargo test -p dl-server-as-code`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test --workspace --all-features --no-fail-fast`; `cargo fmt --check`; `git diff --check`.

## Welle-2a Server-Sync-Command dl-bot (2026-07-02)
- Rework-Implementierungsworker gestartet fuer Kritiker-Befunde aus `/tmp/serversync-review-report.md`: Restore-Pfad, 180d-Retention/Privacy, Snapshot-Bindung, Attachment-Guard, Auth-vor-Parse und Tokenvergleich. Keine Commits/Pushes.
- Rework umgesetzt: `/serversync restore` und `POST /serversync/restore` erzeugen eine normale Diff-Preview aus verifiziertem Rollback-Artefakt v2; v1-Artefakte werden mit klarer Meldung abgelehnt, Hash-Mismatch blockiert. Rollback-Artefakte speichern jetzt DynamicNamespaces und DocumentedExceptions.
- Rework umgesetzt: Diff/Restore persistieren den frisch geholten Live-Snapshot und binden `diff_previews.snapshot_id`; Slash-Attachments haben einen 10-MiB-Guard; HTTP-Apply/Restore authen vor JSON-Parsing; Tokenvergleich nutzt SHA256-Digests.
- Rework umgesetzt: `server_config.rollback_exports.expires_at` mit 180d-Default plus Index; Privacy-Retention-Purge in `dl-community::privacy`, Startup-Wiring im Bot und Privacy-Vertragstest/DB-Purge-Test ergaenzt.
- Verifikation gruen: `cargo fmt --check`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync`; `SQLX_OFFLINE=true cargo test -p dl-community privacy_contract_tests`; `./scripts/central_test_db.sh cargo test -p dl-community --features testing rollback_export_retention_purged_abgelaufene_artefakte`; `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored`; `SQLX_OFFLINE=true cargo clippy -p dl-bot -p dl-community -p dl-central-db --all-targets -- -D warnings`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test --workspace --all-features --no-fail-fast`; `git diff --check`.
- Zusatzbefund bestaetigt: `SQLX_OFFLINE=true cargo clippy --workspace --all-features --all-targets -- -D warnings` scheitert weiterhin an bestehenden, unberuehrten `dl-community`-Testlints (`coaching_requests.rs` await-holding-lock, `faq.rs` unwrap_used).
- Implementierungsworker gestartet auf Branch `feat/server-sync-welle2a`; verbindliche Worker-Regel: keine Commits/Pushes, Aenderungen bleiben uncommitted.
- Adversarialer Kritiker-Review fuer Commit `b4ea79f` gestartet: nur Code/Diff lesen und Bericht schreiben, keine Code-Aenderungen, kein Commit/Push.
- Kritiker-Review abgeschlossen: Bericht liegt unter `/tmp/serversync-review-report.md`. Hauptbefunde: kein Restore-Pfad trotz Rollback-DoD, Rollback-JSON speichert Member-IDs ohne Retention/Privacy-Pfad, Diff-Preview ohne Snapshot-Verknuepfung, fehlender Discord-Attachment-Size-Guard.
- Pflichtkontext gelesen: Konzept §5.1/§7, Soll-Modell/Owner-Entscheidungen 6.1-6.15, `dl-server-as-code` API, `dl-discord` Command-Dispatch sowie Broker-/Changelog-HTTP-Muster.
- Umsetzungsrichtung: gemeinsame Bot-Service-Schicht fuer Snapshot, Rollback-Export, Diff und Apply; Slash-Commands owner-only fuer Guild `1289721245281292288`; interne loopback HTTP-API auf Port 8901 mit `SERVERSYNC_INTERNAL_TOKEN`.
- Implementiert: Migration `2026070240_server_sync_rollback_exports.sql`, Fresh-Schema-Vertrag, `serversync`-Service im dl-bot, Slash-Command-Gruppe `/serversync` und interne HTTP-Routen `/serversync/*` auf Port 8901. Verifikation laeuft noch; kein Commit/Push.
- DSGVO-Vertrag angepasst: `server_config.rollback_exports.created_by_user_id` ist analog zu Diff-/Apply-/Adoption-Auditspalten als server_config-Auditreferenz allowlisted; Rollback-Artefakt speichert Member-Rollen nur als IDs.
- Verifikation gruen: `cargo fmt --check`; `SQLX_OFFLINE=true cargo check -p dl-bot`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync` (6 Tests); `./scripts/central_test_db.sh cargo test -p dl-bot --bin dl-bot` (39 Tests); `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored` (2 Tests); `SQLX_OFFLINE=true cargo test -p dl-community privacy_contract_tests::alle_migration_user_id_spalten_sind_im_privacy_vertrag`; `./scripts/central_test_db.sh cargo test --workspace --all-features --no-fail-fast`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `git diff --check`.
- Zusatzbefund: `SQLX_OFFLINE=true cargo clippy --workspace --all-features --all-targets -- -D warnings` scheitert an bestehenden, unberuehrten `dl-community`-Testlints (`coaching_requests.rs` await-holding-lock, `faq.rs` unwrap_used). Nicht gefixt, weil ausserhalb Scope.

## Mini-Rework Rollen-Fallback + Filter-Randtests
- Mini-Rework gestartet: Scope ist nur konservativer Coaching-User-Overwrite-Fallback bei fehlender Ersatzrolle, zwei Exception-Filter-Randtests und diese WORKFLOW-Notiz. Kein Commit/Push.
- Implementiert: fehlende Coaching-Ersatzrolle bzw. nicht aufloesbare Ersatzzuordnung behaelt den Original-User-Overwrite unveraendert im Soll-Modell und warnt mit "Ersatz nicht möglich, manuell klären".
- Tests ergaenzt: Rollen-Fallback ohne Team-Leo-Rolle prueft byte-identischen Overwrite-Erhalt + Warning; Diff-Engine prueft entfernten dokumentierten Ban-Overwrite und `allow_bits`+`deny_bits`-Registry-Exaktheit.
- Verifikation gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code` = 18 passed + 8 ignored; `SQLX_OFFLINE=true cargo clippy -p dl-server-as-code --all-targets -- -D warnings`; `cargo fmt -p dl-server-as-code -- --check`; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored` = 28 passed; `git diff --check`. Punkt 2 fand keinen `diff.rs`-Bug, daher keine Diff-Code-Aenderung.

## Ziel
Dokumentierte Rechte-Regeln aus `docs/onboarding-redesign/phase1-rechte-soll-modell.ENTWURF.md` als pure Ist->Soll-Transformation fuer `dl-server-as-code` plus transaktionaler Bulk-Persist des Soll-Modells. Kein Commit/Push.

## Fortschritt
- Pflichtlektuere gelesen: Soll-Modell-Doku komplett, danach `model.rs`, `db.rs`, `diff.rs`, `import.rs`; zusaetzlich Schema, Exporte und bestehende Tests gesichtet.
- Umsetzung gestartet: neues `rules.rs` fuer ID-erbende Transformation; DB-Persist soll vorhandene `desired_*`, `dynamic_namespaces` und `documented_exceptions` idempotent upserten.
- Implementiert: `derive_desired_model` klont das Ist-Modell, setzt @everyone-Basisrechte nach §2/6.1, wendet Kategorie-/Kanal-Overwrite-Regeln aus §3/§6 namebasiert an, erzeugt DynamicNamespaces fuer TempVoice/Tickets/Bot-Pate und DocumentedExceptions fuer User-Bans/funktionale 6.8-User-Allows.
- Implementiert: `persist_desired_model` transaktional mit Upserts fuer `desired_*`, `dynamic_namespaces`, `documented_exceptions`; stale Desired-Zeilen werden fuer die Guild entfernt, stale Namespaces/Ausnahmen inaktiv gesetzt. `adopt_change` blieb unveraendert.
- Tests ergaenzt: pure Regeltests fuer Profile, @everyone ohne TTS/private Threads, unbekannte Kategorien, Ban-Ausnahmen und Struktur-Invariante; ignorierter DB-Test fuer idempotenten Bulk-Persist + Registry-Roundtrip.
- Verifikation gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code` = 11 passed + 8 ignored; `SQLX_OFFLINE=true cargo clippy -p dl-server-as-code --all-targets -- -D warnings`; `cargo fmt --check -p dl-server-as-code`; `git diff --check`; zusaetzlich `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored` = 21 passed.

## Rework nach adversarialem Review
- Soll-Doku-Stellen erneut gelesen: §3.3/3.4/3.6/3.10/3.11, §4.1/4.2, §5 und Owner-Entscheidungen 6.4/6.8/6.9/6.10. Keine Commits/Pushes.
- Regeln gehaertet: dokumentierte Struktur-Umzuege `deadlock-rang` -> Eingangsbereich, `stream-updates` -> Medien, `deadlock-invite` -> Chat setzen jetzt `parent_category_id`; Kinderregeln laufen danach gegen das Soll-Modell, sodass verschobene Kanaele die neue Kategorieklasse bekommen.
- Coaching-Rework: leere und ADMIN-redundante Overwrites werden entfernt; ueberbreite Coaching-Masken werden reduziert; Team-Kapitaens-/Coach-User-Overwrites werden rollenbasiert ersetzt oder geloescht. X1/X2/X4 bleiben wegen 6.9/6.10 1:1; X3 bleibt nur als Coaching-Kategorie `-VIEW`.
- Diff-Rework: DocumentedExceptions filtern nur noch, wenn die Actual-Seite exakt dem Registry-Zustand entspricht; DynamicNamespaces filtern Kanal-Existenz und nur TempVoice-Overwrite-Freiheit, Ticket-/Bot-Pate-Overwrite-Drift bleibt sichtbar.
- Tests erweitert: Struktur-Invariante prueft IDs, Parent-Erhalt fuer nicht umgezogene Kanaele, BotMessage-Objekte und 6.9/6.10-bytegleiche Overwrites; DB-Test prueft Stale-Sweep fuer Kategorien, Rollen und Overwrites; Diff-Tests decken abgeschwaechte Exceptions und Ticket-Overwrite-Drift ab.
- Rework-Verifikation gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code` = 15 passed + 8 ignored; `SQLX_OFFLINE=true cargo clippy -p dl-server-as-code --all-targets -- -D warnings`; `cargo fmt -p dl-server-as-code -- --check`; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored` = 25 passed; `SQLX_OFFLINE=true cargo check --workspace --all-features`; `git diff --check`.
- Verify-Kritiker 2026-07-02 gestartet: Fixes werden read-only gegen Soll-Doku, Code und Tests geprueft; keine Code-/Testaenderungen, kein Commit/Push.
- Verify-Kritiker Ergebnis: keine BLOCKER gefunden. Fix 2 und 4 sind funktional weitgehend umgesetzt, aber test-/sicherheitsseitig nur teilweise belegt: fehlende Coaching-Ersatzrolle fuehrt zu Warning+Overwrite-Entfall, und Exception-Filter-Randfaelle `actual fehlt` sowie `allow+deny` sind nicht direkt getestet.
- Verify-Kritiker Verifikation gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code` = 15 passed + 8 ignored; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored` = 25 passed; `SQLX_OFFLINE=true cargo clippy -p dl-server-as-code --all-targets -- -D warnings`; `SQLX_OFFLINE=true cargo check --workspace --all-features`; `git diff --check`.

# P1 LLM-Provider-Abstraktion (2026-07-02)

## Ziel
Phase-1-Arbeitspaket fuer `dl-ai`: neue Chat-Provider-Abstraktion mit MiniMax-, Mistral- und Mock-Provider, pro Einsatzzweck per Env umschaltbar. Kein Commit/Push.

## Fortschritt
- Pflichtkonzept gelesen: §4.1 Bot-Pate und §5.6 KI-Compliance-Gate. Ziel ist Mistral Small 4; MiniMax darf nur synthetische Dev-/Testdaten sehen.
- Bestehende Rust-LLM-Nutzung gesichtet: `rust/crates/dl-ai` enthaelt MiniMax/OpenAI/Gemini-Clients und alte `TextGenerator`-/`VisionGenerator`-Traits; bestehende Call-Sites bleiben im Scope unveraendert.
- Implementiert: neues `ChatProvider`-Trait mit `ChatMessage`, `ChatParams`, `ChatResponse`, `TokenUsage` und `ChatProviderError` (`Timeout`, `RateLimit`, `Auth`, `Provider`) via `thiserror`.
- Implementiert: `MiniMaxChatProvider` (Token-Plan + Standard), `MistralChatProvider` (`/v1/chat/completions`, `MISTRAL_API_KEY`, Default `mistral-small-2603`) und `MockChatProvider`.
- Implementiert: `LlmProviderConfig` mit Provider-Auswahl pro Zweck (`bot_pate`, `cockpit_vorschlag`, `faq`) ueber `DL_LLM_PROVIDER_*`; Default zentral Mistral, Compliance-Kommentar zu MiniMax nur synthetisch/Dev.
- Robustheit: Request-Timeout, begrenzte Retries mit Backoff fuer 429/5xx, typisierte Auth/RateLimit-Fehler, Logs nur mit Provider/Status/Dauer/Tokens bzw. Fehlerklasse, keine Prompt-Inhalte.
- Tests ohne echte API-Calls: lokale axum-Mocks fuer Mistral/MiniMax und Trait-Mock. Verifikation gruen: aus `rust/crates/dl-ai` `cargo build`; `cargo clippy --all-targets -- -D warnings`; `cargo test` (15 Tests + 0 Doctests). Zusatz: `cargo fmt -p dl-ai -- --check`; `git diff --check`.

## Kritiker-Review
- Review gestartet: uncommitted Diff in `dl-ai` wird nur geprueft, keine Implementierung, kein Commit/Push.
- Review abgeschlossen: keine Secret-/Prompt-Logging-Leaks in der neuen Provider-Schicht gefunden; MiniMax-Wire-Format entspricht dem bestehenden `MiniMaxClient`.
- Wichtige Risiken: MiniMax-Dev-only ist nur kommentiert, nicht technisch gegated; Mistral/OpenAI-Response-Parser akzeptiert nur String-Content, obwohl Mistral auch Content-Chunks dokumentiert.
- Verifikation im Review gruen: aus `rust/crates/dl-ai` `cargo test` (15 Tests + 0 Doctests) und `cargo clippy --all-targets -- -D warnings`; zusaetzlich `cargo fmt -p dl-ai -- --check` und `git diff --check`.

## Rework Fortschritt
- Rework gestartet: Baseline `cargo test` aus `rust/crates/dl-ai` gruen mit 15 Tests + 0 Doctests.
- MiniMax-Compliance-Gate technisch umgesetzt: alle drei bestehenden Use-Cases bleiben `user_content`; MiniMax fuer `user_content` liefert `ComplianceViolation`, synthetische Klassifizierung bleibt erlaubt, Dev-Escape nur ueber `DL_LLM_ALLOW_MINIMAX_USER_CONTENT_DEV_ONLY=ich-weiss-was-ich-tue` mit `warn!`.
- Mistral/OpenAI-Parser umgesetzt: `message.content` akzeptiert String oder Text-Chunk-Array, ignoriert Nicht-Text-Chunks und behaelt den sauberen Fehler fuer `null`/fehlenden Inhalt.
- Testluecken geschlossen: Parser-Cases fuer String/Chunk/Mixed/null/leere Choices; Compliance-Cases fuer Block/Synthetic/Escape-Warnung; Retry-Cases fuer 5xx-Obergrenze und kein Retry bei 400/401.
- Rework-Verifikation gruen: `cargo test` aus `rust/crates/dl-ai` 22 Tests + 0 Doctests (vorher 15); `cargo clippy --all-targets -- -D warnings`; `cargo fmt -p dl-ai -- --check`; `git diff --check`.

## Verify-Kritiker nach Rework
- Review 2026-07-02: Compliance-Gate, Parser, Retry-Matrix, neue warn!-Pfade und Scope geprueft; kein Blocker/Wichtig gefunden.
- Verifikation erneut gruen: aus `rust/crates/dl-ai` `cargo test` (22 Tests + 0 Doctests), `cargo clippy --all-targets -- -D warnings`; aus `rust/` `cargo fmt -p dl-ai -- --check`.
- Rest-Risiko: 403-Auth ist im Code gemeinsam mit 401 behandelt, aber nicht als eigener Testfall abgedeckt; direkte Provider-Konstruktoren bleiben Low-Level-API und umgehen die Config-Policy.
# Kritiker P1 Server-as-Code (2026-07-02)

## Ziel
Frischer Review der uncommitted Phase-1-Server-as-Code-Aenderungen auf Branch `p1-server-as-code` gegen `origin/main`. Keine Implementierungsaenderung, kein Commit/Push.

## Frische Verifikation nach Rework
- Gestartet: unabhaengige reine Verifikation der Rework-Behauptungen. Keine Code-Fixes, kein Commit/Push; nur `WORKFLOW.md` wird fuer Fortschritt aktualisiert.
- Code-Review-Zwischenstand: Hash-Rebind und private Apply-Ports wirken umgesetzt; ein Sicherheitsbefund bleibt offen, weil Apply PermissionOverwrite-Deletes live an Serenity weiterreicht, waehrend nur Kategorie/Kanal/Rolle-Deletes geskippt werden.
- Verifikation ausgefuehrt: `cargo test -p dl-server-as-code` gruen mit 6 passed + 7 ignored; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored` gruen mit 15 passed; `cargo clippy -p dl-server-as-code --all-targets -- -D warnings` gruen; Fresh-Migration-Test gruen mit 2 passed; `cargo fmt --check -p dl-server-as-code` gruen.
- Weitere Testabdeckungsluecken: Drift-Dedupe-Test prueft nicht explizit die Aktualisierung von `last_seen_at`; Format-Test prueft nur den leeren Diff, nicht alle drei Aktionen und fuenf `ObjectKind`-Labels.

## Rework-Implementierung
- Rework gestartet: 2 Blocker + 6 wichtige Befunde werden direkt in `dl-server-as-code`, server_config-Migration und Fresh-Migration-Test bearbeitet. Keine Commit-/Push-/Rebase-Aktion durch diesen Worker.
- Befundkontext gelesen: `apply.rs`, `db.rs`, `diff.rs`, `drift.rs`, `lib.rs`, `model.rs`, server_config-Migration und bestehende `dl-server-as-code`-Tests.
- Implementiert: Drei-Wege-Diff-Hash-Pruefung beim Apply-Laden, private Apply-Ports, Create-ID-Rueckschreiben, Delete-/BotMessage-Skip-Ergebnisse, Drift-Dedupe/Resolve, Adopt-Orphan-Cleanup, Positionsdiff-Default aus und Fresh-Migration-Check fuer alle 18 `server_config`-Tabellen.
- Dokumentiert: BotMessages/Panels bleiben Phase-2-Folgearbeit; Apply meldet sie als `skipped: not implemented`.
- Verifikation nach Rework gruen: `cargo test -p dl-server-as-code` mit 6 passed + 7 ignored; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored` mit 15 passed; `cargo clippy -p dl-server-as-code --all-targets -- -D warnings` gruen; Zusatz `cargo clippy -p dl-server-as-code --features testing --all-targets -- -D warnings` gruen; Fresh-Migration-Test mit 2 passed; `cargo fmt --check -p dl-server-as-code -p dl-central-db` gruen.
- `git diff --check` wegen ausdruecklicher No-Git-Regel nicht ausgefuehrt; Ersatzpruefung der beruehrten Dateien auf trailing whitespace und Konfliktmarker war sauber.

## Fortschritt
- Review gestartet: Worktree/Branch, bestehendes `WORKFLOW.md` und Diff-Scope werden geprueft; Fokus liegt auf Discord-Write-Sicherheit, Diff-Engine, Migration, Import, Drift/Adopt und geforderter Verifikation.
- Lokale Verifikation bisher: `cargo test -p dl-server-as-code` gruen mit 5 passed + 4 ignored; `cargo clippy -p dl-server-as-code --all-targets -- -D warnings` gruen; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored` gruen mit 9 passed.
- Zusatzverifikation: `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored` gruen mit 2 passed; `git diff --check` gruen.
- Baseline per `git stash -u`: sauberer Branch ist gegen `origin/main` bereits 1 Commit hinten (`0015_steam_links_one_primary.sql` + Test fehlt). Diff-Zaehler aktuell vs. origin/main: 2 D / 4 M + 12 untracked; nach Stash: 2 D / 1 M + 0 untracked.
# P1 Journey+Ingestion+Analytics DSGVO (2026-07-02)

## Ziel
Phase-1-Fundament fuer Journey-State-Machine, Message-/Voice-/Interaction-Metadaten, Analytics-Grundgeruest, 180d-Retention und privacy.rs-Vertrag im Worktree `dl-bots-p1-journey-ingest`. Kein Commit/Push.

## Fortschritt
- Rework-Implementierung 2026-07-02 gestartet: Fix-Gruppen sind Interaction-Route-Sanitizing, Aktivierung `first_message`/`first_voice`, Event-Wiring fuer Privacy/Onboarding/Weiche, Privacy-Migration-Heuristik, DB-harter First-X-Dedupe und dokumentierte Aggregat-Distinct-Approximation. Kein Commit/Push/Rebase.
- Rework umgesetzt: custom_id-Routen werden vor Raw-Write sanitisiert und bei Retention-Kompaktierung erneut SQL-seitig sanitisiert; `pct_communicated` zaehlt `first_message` ODER `first_voice`; First-X nutzt partiellen Unique-Index + `ON CONFLICT DO NOTHING`; Privacy-Test scannt alle Migrationen auf User-ID-Spalten; Opt-out wird anonym aggregiert, Onboarding/Weiche ueber Role-/Tag-Events verdrahtet; externe Journey-Record-API dokumentiert.
- Rework-Verifikation gruen: Offline-Tests 95 passed (vorher 94); DB-Wrapper `dl-activity` 55 (vorher 52), `dl-community` 107, `dl-central-db` 8, `dl-bot` 33; Clippy `-D warnings` auf `dl-discord`, `dl-activity`, `dl-community`, `dl-central-db` gruen plus Zusatz `dl-bot` gruen; `cargo fmt --check` gruen; `git diff --check` gruen.
- Kritiker-Review 2026-07-02 gestartet: uncommitted Diff wird hart gegen DSGVO, Ingestion-Stabilitaet, Retention, State-Machine, Analytics, Migration und Testmatrix geprueft. Keine Implementierungsfixes, kein Commit/Push.
- Kritiker-Review Befunde: BLOCKER bei Interaction-Route/custom_id-Persistenz, weil raw custom_ids User-IDs enthalten koennen und spaeter in `interaction_daily_aggregates.route` ohne user_id weiterleben; WICHTIG bei Aktivierungsmetrik (nur first_message, nicht Voice/Interaction) und unverdrahteten Journey-Events ausser Join/Screening/First-Message/First-Voice/Interaction.
- Review-Verifikation: aktueller Diff gruen fuer `SQLX_OFFLINE=true cargo test -p dl-discord -p dl-activity -p dl-community -p dl-central-db`, DB-Wrapper (`dl-activity` 52, `dl-community` 107, `dl-central-db` 8, `dl-bot` 33), Clippy `-D warnings`, `git diff --check`. Baseline per `git stash`: Offline-Test Diff 94 passed vs Baseline 92 passed; Clippy beidseitig gruen.
- Frischer Verify-Kritiker 2026-07-02 gestartet: Rework wird nur geprueft, keine Code-Fixes/Commits; Fokus auf Sanitizing, Aktivierung <=14d, Event-Wiring, Privacy-Diff-Test, First-X-Dedupe, Startup-/Event-Pfad und Migration-Koordination.
- Frischer Verify-Kritiker Ergebnis: kein BLOCKER im Rework gefunden; Interaction-Routen werden vor Raw-Write und bei SQL-Kompaktierung sanitisiert, First-X-Dedupe ist DB-hart, Opt-out schreibt nur anonymes Aggregat, Weiche-/Onboarding-Wiring ist fire-and-forget. NICE: Aktivierungs-DB-Test prueft Voice-getriebene Aktivierung, aber keinen strikten Voice-only-User mit `first_message_at IS NULL`.
- Frische Verifikation gruen: Offline `cargo test -p dl-discord -p dl-activity -p dl-community -p dl-central-db` = 95 passed; DB-Wrapper mit `SQLX_OFFLINE=false` im Testprozess: `dl-activity` 55, `dl-community` 107, `dl-central-db` 8, `dl-bot` 33; Clippy `-D warnings` fuer vier Crates plus `dl-bot` gruen; `cargo fmt --check` und `git diff --check` gruen. Migration-Koordination: `origin/main` enthaelt `2026070210_server_config_schema.sql` im eigenen Schema `server_config.*`; lokale `2026070220` liegt danach und nutzt `activity.*`, kein Nummern-/Schema-Konflikt erkennbar.
- Konzept/IST/Discord-Insights-README gelesen; relevante Pflichtpunkte: Metadaten-first, 180d-Retention, privacy.rs Delete+Export, Diff-Test, Aktivierung <=14 Tage.
- Bestehende Eventpfade gesichtet: `dl-discord` normalisiert Message/Member/Voice ueber `Dispatcher`; Interactions laufen bisher direkt durch den Router. Privacy nutzt statische `USER_TABLES`/Export-Iteration in `dl-community/src/privacy.rs`.
- Umsetzung gestartet: neue zentrale Migration im erlaubten Bereich `202607022*`; Ingestion soll in `dl-activity::journey` als nicht-blockierender Dispatcher-Subscriber liegen.
- Migration `2026070220_journey_ingestion_analytics.sql` angelegt: Journey-Events/State, Message-/Voice-/Interaction-Metadaten, Voice-Open-Sessions und anonyme Tagesaggregate.
- `dl-discord` publiziert jetzt minimale `InteractionEvent`-Metadaten; `dl-activity::journey` konsumiert Message/Member/Voice/Interaction mit Opt-out-Check vor jedem Write, First-Message/First-Voice-State, 180d-Retention und Analytics-Queries.
- `dl-bot` startet Journey-Ingestion + Retention im Gateway-Pfad.
- `privacy.rs` um alle neuen userbezogenen Tabellen fuer Delete+Export erweitert; Opt-out/Delete entfernt bereits ingestierte Journey-/Metadata-Rows. Diff-Test parst die Journey-Migration gegen `USER_TABLES`.
- Verifikation gruen: `SQLX_OFFLINE=true cargo build -p dl-discord -p dl-activity -p dl-community -p dl-bot`; `SQLX_OFFLINE=true cargo clippy -p dl-discord -p dl-activity -p dl-community -p dl-bot --all-targets -- -D warnings`; `SQLX_OFFLINE=true cargo test -p dl-discord -p dl-activity -p dl-community -p dl-central-db`; `./scripts/central_test_db.sh bash -lc 'SQLX_OFFLINE=false DATABASE_URL="$DEADLOCK_CENTRAL_DSN" cargo test -p dl-activity -p dl-community -p dl-central-db --features testing -- --include-ignored'`; gleicher Wrapper fuer `cargo test -p dl-bot --bin dl-bot`; `git diff --check`.

## Offen
- Erfolgreiche Steam-Link-/Invite-Statusereignisse kommen weiterhin aus externen Steam-Bot-/Invite-Flows; Phase-1 stellt Eventtypen und `record_journey_event` bereit, verdrahtet aber nur die heute im dl-bot vorhandenen Gateway-Handler direkt.

# Rework core.users Upsert Nonblocking (2026-07-02)

## Ziel
Core-User-Upsert aus Discord-Gateway-Handlern entkoppeln: nur In-Memory-Reserve bleibt im Event-Pfad, DB-Write laeuft im Hintergrund. Kein Commit/Push.

## Fortschritt
- Bestehende uncommitted Core-User-Sync-Aenderungen und Kritikerbefund gelesen.
- `CoreUserSync::record` auf synchronen Reserve+`tokio::spawn`-Pfad umgestellt; Upsert-Fehler loggen weiter und loeschen Reservierungen nur noch timestamp-genau.
- Gateway-Call-Sites auf nicht-async `record_core_user` umgestellt; Trigger bleibt vor fachlichem Dispatch, blockiert aber nicht mehr auf Postgres.
- Tests werden angepasst: bestehende DB-Tests pollen den asynchronen Write; neuer Unit-Test simuliert einen blockierenden/fehlenden Upsert ohne echte DB.
- Verifikation gruen: `SQLX_OFFLINE=true cargo build -p dl-discord`; `SQLX_OFFLINE=true cargo clippy -p dl-discord --all-targets -- -D warnings`; `cargo fmt --check`; `./scripts/central_test_db.sh cargo test -p dl-discord --features testing -- --include-ignored`.

# Adversarial Critic: core.users Upsert-Wiring (2026-07-02)

## Ziel
Unabhaengiger Review der uncommitted Aenderungen fuer `core.users`-Upsert-Wiring in `dl-discord`. Keine Implementierungsaenderung, kein Commit/Push.

## Fortschritt
- Review gestartet: Worktree/Branch geprueft, relevante Dateien und Pflicht-Verifikation werden eigenstaendig gelesen/ausgefuehrt.
- `core_user_sync.rs`, kompletter Diff in `gateway.rs`/`lib.rs`, `dl_central_db::upsert_user`, Schema/Tests und Serenity-Dispatch lokal gelesen.
- Befund: Mutex wird nicht ueber `.await` gehalten und Upsert-Fehler werden geloggt/Reservierung geloescht; aber Gateway wartet vor Command-/Message-/Join-Dispatch synchron auf den Core-User-Upsert.
- Verifikation: `SQLX_OFFLINE=true cargo build -p dl-discord` gruen; `SQLX_OFFLINE=true cargo clippy -p dl-discord --all-targets -- -D warnings` gruen; Auftragspfad `./scripts/central_test_db.sh` fehlt, aequivalenter Lauf aus `rust/` mit `./scripts/central_test_db.sh cargo test -p dl-discord --features testing -- --include-ignored` gruen.

# Patchnotes IDENTITY Migration (2026-07-01)

## Ziel
Patchnotes T3-Blocker beheben: `patchnotes.changelog_posts.id` und `patchnotes.deadlock_changelogs.id` per zentraler Migration auf `GENERATED BY DEFAULT AS IDENTITY` umstellen, Sequenzen auf bestehende Max-IDs ausrichten, Test nur gegen Wegwerf-Postgres. Kein Live-DB-Apply, kein Commit/Push.

## Fortschritt
- Bestehende Patchnotes-DDL in `0010_activity_moderation_content_patchnotes.sql`, `dl_central_db::testing::test_pool` und DB-Testmuster gesichtet.
- Migration `0012_patchnotes_identity_sequences.sql` angelegt: beide `id`-Spalten bekommen Identity, danach `setval(pg_get_serial_sequence(...), max(id)+1, false)`.
- Integrationstest `patchnotes_identity.rs` angelegt: Inserts ohne ID und mit expliziter ID fuer beide Tabellen sowie Sequenzfortsetzung nach simuliertem Pre-0012-Bestand mit `id=500`.
- Fresh-Migration-Schema-Test auf Migration 12 erweitert.
- Verifikation gruen: `cargo test -p dl-central-db`; `cargo clippy -p dl-central-db --all-targets -- -D warnings`; `cargo fmt --check -p dl-central-db`; `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test patchnotes_identity -- --ignored`; `./scripts/central_fresh_schema.sh`; `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing -- --include-ignored`; zusaetzlich `cargo clippy -p dl-central-db --features testing --all-targets -- -D warnings`.
- Keine Live-DB-Migration ausgefuehrt; nur Wegwerf-Postgres aus dem vorhandenen Test-Wrapper genutzt.

# T0 Reconciliation Baseline-Tooling (2026-07-01)

## Ziel
Nur Ticket T0 aus `rust/docs/plans/2026-07-01-central-db-final-reconciliation.md`: Live-ETL-Snapshot rekonstruieren und read-only Manifest-Tooling fuer Snapshot-Hashes/mtimes sowie Ledger-Tabellenklassifikation bauen. Kein Postgres-/SQLite-Schreibzugriff, kein Commit/Push.

## Fortschritt
- Branch/Arbeitsbaum geprueft; bestehende uncommitted `WORKFLOW.md`-Aenderung bleibt erhalten.
- Snapshot-Artefakte gefunden: `data/central-etl-snapshots/p4-final-20260701-032848` mit Dateien um `2026-07-01 03:28:49 +0200`, direkt vor bekanntem Live-Start `2026-07-01 03:29:38 +0200`.
- Journald- und Shell-History-Pruefung lieferten keine eindeutigen `dl-central-sync`-Runner-Zeilen; Dateisystem-Artefakt wird als beste rekonstruierbare Baseline verwendet, konservativer Plan-Cutoff bleibt Fallback.
- Tooling umgesetzt: `dl-central-etl` hat nun das read-only Binary `dl-reconciliation-manifest` plus Manifest-Modul. Es liest nur die 11 T0-Ledger-Fragmente und Snapshot-Dateien, erzeugt SHA256/mtime-Metadaten und klassifiziert Tabellen als `clock_present`, `append_only`, `no_clock` oder `queue_state`.
- Baseline-Manifest erzeugt: `rust/docs/_work/sp1/reconciliation/2026-07-01-t0-baseline-manifest.json`. Ausgewaehlter Cutoff ist `2026-07-01T01:28:48Z` (`2026-07-01 03:28:48 +0200`) aus `p4-final-20260701-032848`; Snapshot-Dateifenster `2026-07-01T01:28:49.480754435Z` bis `2026-07-01T01:28:49.504754299Z`.
- Manifest-Zahlen: 3 Snapshot-Dateien, 11 Ledger-Fragmente, 103 Tabellen; Klassen: 43 append-only, 27 Uhr vorhanden, 9 ohne Uhr, 24 Queue/State.
- Verifikation gruen: `cargo test -p dl-central-etl`; `cargo clippy -p dl-central-etl --all-targets -- -D warnings`; `cargo fmt --check -p dl-central-etl`; `git diff --check`; Manifest-Scan ohne `DEADLOCK_CENTRAL_DSN`/Postgres-URL.

## Offen
- T1-Empfehlung: Zeitfilter nur als Vorfilter verwenden; `no_clock` und `queue_state` vor Apply mit expliziten Tabellenpolicies behandeln. Kein T1-Code in diesem Schritt implementiert.

# T1 Reconciliation Kandidaten-Erkennung (2026-07-01)

## Ziel
Ticket T1 aus `rust/docs/plans/2026-07-01-central-db-final-reconciliation.md`: read-only Kandidaten-Report in `dl-central-etl` fuer die aktiven SQLite-Quellen gegen den T0-Original-Snapshot und aktuelle PG-Targets. Kein Apply, kein Commit/Push, keine DSN-/Secret-Ausgabe.

## Kritiker-Review
- Unabhaengiger Review gestartet: uncommitted Diff, Plan T1/T2-Regeln und Kandidatenmodul werden geprueft. Keine Live-Daten/Postgres-Zugriffe, kein Commit.
- Review abgeschlossen: Blocker gefunden. Report gibt nur aggregierte Tabellenzaehler aus, keine zeilenweisen Kandidaten/Hashes/Entscheidungen; Queue-/State-Klassifikation wird nicht konservativ gegatet; Patchnotes-URL-Konfliktregel fehlt.
- Verifikation im Review: `cargo test -p dl-central-etl`; `cargo fmt --check -p dl-central-etl`; `cargo clippy -p dl-central-etl --all-targets -- -D warnings`; `git diff --check`.

## Fortschritt
- Branch/Arbeitsbaum geprueft; T0-Manifest und Plan gelesen.
- Bestehende ETL-Konvertierung, Ledger-Mapping, Manifest-Struktur und Source-/Target-Reader gesichtet.
- Umsetzungsentscheidung: Kandidaten werden per Vollvergleich aktueller SQLite-Quelle gegen Original-Snapshot erkannt; Zeitfilter bleibt nicht noetig fuer Korrektheit. Hashes entstehen nach bestehender ETL-Normalisierung in Zieltypen.
- Implementiert: neues Modul `reconciliation_candidates` und Binary `dl-reconciliation-candidates`. Das Binary liest T0-Manifest, Ledger, Original-Snapshot, aktuelle SQLite-Quellen und PG-Targets read-only; Report enthaelt nur aggregierte Tabellenzaehler, keine Row-Nutzdaten und keine DSN.
- Safe-Merge-Klassifikation umgesetzt: source_new/source_changed plus `insert_allowed`, `update_allowed`, `noop`, `conflict`; Tabellen ohne Target-PK werden als manual/skipped markiert.
- Tests ergaenzt fuer Insert/Update/Noop/Conflict/Unchanged sowie kanonische JSON-Hash-Normalisierung.
- Verifikation gruen: `cargo test -p dl-central-etl`; `cargo clippy -p dl-central-etl --all-targets -- -D warnings`; `cargo fmt --check -p dl-central-etl`; `git diff --check`.
- Rework nach Kritiker-Befunden umgesetzt: Report enthaelt jetzt pro Kandidat Quelle, Ziel-PK, Source-Hash, optionalen Original-/Target-vorher-Hash, Aktion, Entscheidung und optionalen URL-Konflikt-PK/-Hash.
- `classification=no_clock` und `classification=queue_state` werden fuer Kandidaten immer auf `manual` gesetzt; `bot.kv_store` ist im T1-Scope auf `ns='patchnotes_bot'` begrenzt.
- Patchnotes-Tabellen erzwingen Konflikt bei gleicher `url` mit abweichender `id`, auch wenn die PK-basierte Entscheidung sonst Insert/Update/Noop waere.
- Regressionstests ergaenzt fuer zeilenweisen Source/Original/Target-Fall, Queue-/NoClock-Manual-Policy, Patchnotes-URL-Konflikt und KV-Namespace-Trennung.
- Rework-Verifikation gruen: `cargo test -p dl-central-etl`; `cargo clippy -p dl-central-etl --all-targets -- -D warnings`; `cargo fmt --check -p dl-central-etl`.

## Offen
- Kein Dry-Run gegen Live-PG ausgefuehrt, weil die geforderte Verifikation nur Build/Test/Lint/Format umfasst und keine Secret-/Infisical-Nutzung verlangt.

# Central DB Final Reconciliation Plan (2026-07-01)

## Ziel
Nur Doku-Plan fuer sichere Delta-Reconciliation und koordinierten finalen Cutover von Steam-Bot, Patchnotes und Website-Backend nach SP1. Kein DB-Zugriff, keine Live-Aenderung, kein Push. Wegen widerspruechlicher Vorgaben wird nicht committed; Aenderungen bleiben fuer Claude-Review uncommitted.

## Fortschritt
- Bestehende SP0/SP1-Doku gelesen: zentrale Architektur, Ledger-Format, data-landscape, SP1 Phase 1/2/3 sowie Migrations- und ETL-Historie.
- Git-Historie seit 2026-06-30 geprueft: P2-ETL-Abschluss `3da3a01` am 2026-06-30 12:27:05 +0200; P3-Barriere `99c556f` am 2026-07-01 01:58:48 +0200; Live-Sync-Runner `641d469` am 2026-07-01 03:03:38 +0200; Merge `e7d2ab8` am 2026-07-01 03:23:04 +0200; Live-Start laut Auftrag 2026-07-01 03:29:38.
- Ziel-DDL/Ledger fuer `core.steam_links`, `core.users`, `steam.*`, `coaching.*`, `patchnotes.*` und `bot.kv_store` gesichtet.
- Statische Cross-Repo-Pruefung: Steam-Units zeigen auf geteilte SQLite via `DEADLOCK_DB_PATH`; Patchnotes schreibt `changelog_posts`, `deadlock_changelogs`, `kv_store(ns='patchnotes_bot')`; Website-Backend nutzt eigene `aiosqlite`-DB `builds/backend/deadlock.db`.
- Plan-Datei angelegt: `rust/docs/plans/2026-07-01-central-db-final-reconciliation.md`.
- Verifikation: `git diff --check` sauber; Secret-Scan der neuen Plan-Datei ohne DSN/Secret-Werte.

## Offen
- Claude-Review; kein Commit durch diesen Worker wegen verbindlicher Worker-Regel.

# Deadlock-Bots Enforcement-Gaps (2026-06-30)

## Phase-0 P0-Fixes (2026-07-02)

## Ziel
Leave-Survey-DM-Fehler 50007 typisiert als `blocked` klassifizieren und die geforderten deutschen user-sichtbaren Rust-Texte/Prompts umstellen. Kein Commit/Push.

## Fortschritt
- Worktree/Branch geprüft: `/home/naniadm/.worktrees/deadlock-bots-phase0`, `phase0-fixes`.
- Repo-weite Suche nach den zu ändernden Mod-Log-Feldnamen/Titeln durchgeführt: Treffer nur in Rust-Erzeugung, Python-Referenz und Audit-/Changelog-Doku; keine Rust-Dashboard-/Parser-Kontrakte gefunden. Guard-Reason-Tests referenzieren alte englische Reasons und werden mit angepasst.
- Implementiert: typisierte Leave-Survey-DM-Delivery (`sent`/`blocked`/`failed`), Adapter-Methode mit `serenity::Error`, 50007-Match auf `HttpError::UnsuccessfulRequest`, neutrale Blocked-Logzeile und Fehlertext bei echten Failures.
- Implementiert: geforderte deutschen Voice-/Guard-/Mod-Log-Strings und Guard-Prompt-Sprachvorgabe; betroffene Guard-Reason-Tests angepasst.
- Verifikation: `SQLX_OFFLINE=true cargo build --workspace` grün; `cargo fmt --all` und `cargo fmt --all -- --check` grün; `git diff --check` grün.
- Pflicht-`clippy --all-targets` und Pflicht-`test --workspace` blockieren im bestehenden `dl-bot`-Testtarget: fehlende SQLx-Offline-Metadaten in `bin/dl-bot/src/build_publisher.rs` plus bestehender `use dl_db::Db`-Import in `bin/dl-bot/src/modglue.rs`.
- Zusatzverifikation für geänderte Ziele: `SQLX_OFFLINE=true cargo clippy -p dl-bot --bin dl-bot -- -D warnings` grün; `SQLX_OFFLINE=true cargo clippy -p dl-discord -p dl-community -p dl-moderation -p dl-voice --all-targets -- -D warnings` grün; `dl-community`/`dl-discord`/`dl-moderation`-Tests grün im gezielten Lauf, `dl-voice`-Tests scheitern an fehlender Test-DB-DSN (`CENTRAL_TEST_DSN`/`DATABASE_URL`/`DEADLOCK_CENTRAL_DSN`).
- Review 2026-07-02 gestartet: uncommitted Diff wird nur geprüft, keine Implementierungsänderungen; Pflicht-Baseline-Vergleich folgt mit Stash/Pop.
- Review 2026-07-02 Baseline: `SQLX_OFFLINE=true cargo clippy --all-targets -- -D warnings` und `SQLX_OFFLINE=true cargo test --workspace` jeweils Diff 23 Compile-Errors vs Baseline 23 Compile-Errors; Fehlerlisten gleich (22 fehlende SQLx-Offline-Caches in `bin/dl-bot/src/build_publisher.rs` + bestehender `dl_db`-Import im `dl-bot`-Testmodul). Vor Baseline-Test wegen voller Platte nur `rust/target` per `cargo clean` entfernt.
- Review 2026-07-02 Zusatztest: `SQLX_OFFLINE=true cargo test -p dl-community survey_dm_delivery_logtexte` grün; Test prüft Status/Logfragment, nicht den typisierten Serenity-50007-Match im Glue.

## Ziel
Genau drei Rust-Enforcement-Befunde im isolierten Worktree `fix/enforcement-gaps` fixen: Review-Button-Rechte, Coaching-No-Show-Ban im Website-Intake, atomarer Coaching-Claim. Kein Commit/Push/Deploy. TempVoice bleibt unberuehrt.

## Root-Cause
- #1 Review-Buttons: `rust/bin/dl-bot/src/modglue.rs:860` und `:1704` pruefen `author_can_manage_roles`; die Aktionen fuehren aber Timeout/Ban/Unban aus. Die Interaction-Bridge liefert bisher nur `author_can_manage_roles` (`rust/crates/dl-discord/src/interactions.rs:38-43`, `rust/crates/dl-discord/src/dispatch.rs:61-78`, `:120-137`, `:184-200`).
- #2 Website-Intake: `rust/crates/dl-community/src/coaching_requests.rs:1005-1023` nimmt `request_created` an; `upsert_request_created_notification` schreibt bei `:974-995` ohne aktive `coaching_bans` (`rust/docs/db-schema.sql:286-292`) zu pruefen. No-Show-Bans entstehen bei Cancel in `coaching_requests.rs:1857-1868`.
- #3 Claim-Race: `coaching_requests.rs:1661-1681` liest Status/Reservierung vorab; `:1697-1711` legt danach Session an und setzt `status='matched'` ohne konditionales Claim-Update. Zwei parallele Handler koennen denselben alten Status sehen.

## TDD Rot
- `cargo test -p dl-bot aimod_ -- --nocapture`: beide neuen Tests rot, `Manage Roles` reicht aktuell bis zum Case-Lookup (`Case nicht gefunden.` statt `Keine Berechtigung.`).
- `cargo test -p dl-community request_created_notification_lehnt_aktiven_no_show_ban_ab -- --nocapture`: rot, aktiver Ban wird angenommen (`expect_err` bekam `Ok(())`).
- `cargo test -p dl-community parallele_claims_lassen_nur_einen_coach_gewinnen -- --nocapture`: rot, zwei parallele Claims gewinnen (`left: 2`, `right: 1`).

## Fortschritt
- Rote Tests fuer alle drei Befunde ergaenzt.
- Fix umgesetzt:
  - Review-Buttons nutzen jetzt `Moderate Members` fuer Timeout/Timeout-Aufhebung und `Ban Members` fuer Ban/Unban; `Manage Roles` bleibt fuer andere Pfade unveraendert.
  - Website-`request_created` lehnt aktive No-Show-Bans mit `"Platzhalter"` ab und schreibt keinen Request.
  - Coach-Claim ist per Transaktion/konditionalem `UPDATE ... WHERE status='analyzed'` atomar; nur der Gewinner legt eine Session an.
- Verifikation gruen: `cargo test -p dl-discord -p dl-community -p dl-bot`; `cargo build --release -p dl-bot`; `rustfmt --edition 2021 --check` fuer geaenderte Rust-Dateien.
- Clippy: `cargo clippy -p dl-discord -p dl-community -p dl-bot --all-targets` laeuft mit Exit 0, zeigt bestehende Warnungen ausserhalb der Aenderungen (`unwrap_used` in alten dl-community-Tests, `too_many_arguments` in build_publisher). Strenger Zusatzlauf mit diesen bestehenden Warnklassen unterdrueckt: gruen mit `-D warnings`.
- Platzhalterstellen: `rust/crates/dl-community/src/coaching_requests.rs` No-Show-Reject und Claim-DB-Fehler.

# Invite/New-Account SecurityGuard (2026-06-21)

## Ziel
Rust-SecurityGuard nach `rust/docs/specs/2026-06-21-invite-newaccount-moderation-design.md` erweitern: Fremd-Discord-Invites erkennen, neue Account-Klassifikation (<30d Account und <7d Join), Soft-Warn vs. Hijack nach Channel-Streuung, DM-vor-Ban.

## Fortschritt
- Spec vollständig gelesen; relevante Rust-Dateien `guard.rs`, `lib.rs`, `store.rs`, Glue `modglue.rs` und Python-Referenz `cogs/security_guard.py` gesichtet.
- Bestehende Welle-1-Pfade identifiziert: `GuardAction::{Enforce, Propose, EstablishedScam, Takeover}`, History-Fenster, DM/Aktion/Delete-Reihenfolge, Serenity-REST fuer Invites/Vanity.
- Implementiert: pure Invite-Code-Extraktion, neue Account-Klassifikation, Streuungsrouting, `SoftWarn`/`Hijack`-Konsolidierung, Ban-DM vor Ban, selbstloeschende Soft-Warn-Notiz.
- Glue implementiert: `OUR_GUILD_ID`, `INVITE_ALLOWLIST_FALLBACK`, `ESCALATION_CONTACT_HANDLE`, Auto-Allowlist eigener Invites + Vanity und TTL-Cache fuer Invite-Aufloesung.
- Verifikation gruen: `cargo test -p dl-moderation` (19 Tests) und `cargo check --workspace`.

## Offen
- Claude-Review; kein Commit/Push durch diesen Worker.

# Wave1 SecurityGuard + AI-Moderator Parity (2026-06-21)

## Ziel
Rust-Port unter `rust/` an sieben Audit-Punkten mit Python-Live-Code abgleichen: SG-9 bis SG-12 und AM-6 bis AM-8. Python-Referenz (`cogs/`, `service/`) bleibt read-only; kein Commit/Push.

## Fortschritt
- `cogs/security_guard.py`, `cogs/ai_moderator.py`, `rust/crates/dl-moderation/src/guard.rs`, `store.rs`, `lib.rs` und `rust/crates/dl-discord/src/gateway.rs` vollständig gelesen.
- Audit-Kontext fuer Security Guard und AI Moderator in `rust/docs/audit/2026-06-21-py-rust-discord-parity-audit.md` abgeglichen.
- Bestehende Rust-Glue-Struktur (`rust/bin/dl-bot/src/modglue.rs`) und `MessageEvent`-Dispatcher geprueft.
- SG-9 bis SG-12 umgesetzt: separater Staff-Skip, Bild-Multichannel-Scam-Pfad, Young-Burst-Bildbestaetigung und etablierter Scam-Pfad mit 24h-Timeout + Platzhalter-DM.
- AM-6 bis AM-8 umgesetzt: AutoDelete timeoutet 24h, Kontext-Backfill mit 12 Zeilen, reiches Prompt-Payload inklusive `recent_context`, `>>>`-Prefix, Zeitstempel und Reply-Kontext.
- Verifikation gruen: `cargo check --workspace`; `cargo test -p dl-moderation -p dl-discord`.

## Erledigt (Claude)
- Finale deutsche Texte an beiden `Platzhalter`-Stellen eingesetzt: etablierter-Scam Warn-DM (guard.rs) + Mod-Embed-Titel (modglue.rs), Wortlaut aus Python-Vorlage `_handle_scam_proposal`.
- Kritischer Sub-Agent-Review: alle 7 Fixes paritätstreu. clippy sauber, cargo check/test grün (14+7).

# FAQ-Doku W1 – Voice + Community (2026-05-23)

## Ziel
Zwei neue FAQ-Dateien fuer den FAQ-Bot: `docs/voice-features.md` und `docs/community-tools.md`, jeweils auf echter Cog-Logik statt Alt-Doku-Raten basierend.

## Fortschritt
- Referenz-Plan aus `~/.claude/plans/ich-m-chte-das-wir-floating-mountain.md` gelesen und Template/Stilregeln uebernommen.
- Relevante Voice-Cogs gesichtet: `cogs/tempvoice/*`, `deadlock_team_balancer.py`, `deadlock_voice_status.py`, `rank_voice_manager.py`, `voice_activity_tracker.py`, `voice_reaction_dm.py`, `steam_link_voice_nudge.py`.
- Relevante Community-Cogs gesichtet: `cogs/tags/*`, `bug_reporter.py`, `lfg.py`, `feedback_hub.py`, `clip_submission.py`, `player_finder.py`, `leave_survey.py`.
- Alt-Doku fuer TempVoice und Community konsolidiert; Fokus bleibt auf User-Sicht, Admin-/Mod-Mechanik wird ausgespart.

## Offen
- Wortzahl und Stil der zwei neuen Markdown-Dateien nach dem Schreiben gegen das Worker-Briefing gegenpruefen.

# AI-Moderator Cog (2026-04-24)

# Member-Source-Tracking ehrlich + Website-Bucket (2026-05-11)

## Ziel
Backfill von historischen Joins ehrlich auf `unknown` halten, Website-Invites pro Subseite aufteilen und das Admin-Dashboard um einen eigenen Website-Bucket mit Subseiten-Breakdown erweitern.

## Fortschritt
- `cogs/user_activity_analyzer.py`: Backfill schreibt bei historischen Twitch-Treffern nur noch Audit-Hinweise (`twitch_streamer_login`, `matched_twitch_hint`), aber bucketed rueckwirkende Joins konsequent als `unknown/backfilled`.
- `cogs/website_invite_cog.py`: Multi-Subpage-Invite-Verwaltung fuer `landing`, `streamer`, `mitspieler`, `coaching`, `helden`, `guides`; `main` bleibt Alias fuer `landing`. `/website-invite` zeigt jetzt alle Codes, `/website-invite-recreate` rotiert gezielt pro Subseite, `/join-quellen` erkennt alle Website-Codes mit Label `Website: <Subseite>`.
- `service/db.py`: neue Hilfsfunktion `list_kv(ns)` fuer Namespace-Enumerierung.
- `service/dashboard.py`: Website-Codes werden aus `website_invites` geladen, alte Events bei Website-Code nachtraeglich von `personal` auf `website` rebucketed und als `website_breakdown` aggregiert.
- `service/static/dashboard.html`: Doughnut-Series um `Website` erweitert, Tooltip-Subbreakdown fuer Website eingebaut und eigene Website-Liste im Detailbereich ergaenzt.

## Verifikation
- `python -m py_compile ...` war nicht moeglich, weil in der Umgebung kein `python`-Binary existiert.
- Fallback erfolgreich: `python3 -m py_compile cogs/user_activity_analyzer.py cogs/website_invite_cog.py service/dashboard.py service/db.py`
- Keine passenden Tests unter `tests/` fuer `user_activity_analyzer`, `website_invite` oder `dashboard` gefunden.

## Offen
- Kein Live-Discord-Smoke-Test und kein Dashboard-Browser-Check in diesem Worker-Pass.

## Ziel
Neues Cog `cogs/ai_moderator.py`, das Nachrichten in Chat-Channel `1289721245281292291` via MiniMax-M3 (Text + Bilder) klassifiziert und je nach Konfidenz auto-moderiert oder Moderatoren per Accept/Deny-Buttons im Channel `1315684135175716978` einbindet. Alle Aktionen werden im Log-Channel `1374364800817303632` festgehalten (ohne Buttons, nur Infos + Original-Message). Ragebait wird pro User mit einer 2h-Rolling-Window-Schwelle (4 Hits) zu `persistent_ragebait` eskaliert.

## Plan
`/home/naniadm/.claude/plans/ich-m-chte-f-r-meinen-dapper-hennessy.md`

## Architektur
- Einzelnes File: `cogs/ai_moderator.py` (Config, Cog, Views, Modal, SQL-Schema) – Blaupause: `cogs/security_guard.py`
- `cogs/ai_connector.py` bekommt neue Methode `generate_multimodal(provider, prompt, images, ...)` für MiniMax-Bildinput (Anthropic- und OpenAI-kompatibles Content-Array)
- Persistenz: neue Tabellen `ai_moderation_cases` + `ai_moderation_ragebait_hits` in `data/deadlock.sqlite3`
- Persistente Views (`custom_id` mit Case-ID) für Bot-Restart-Resilienz

## Flow
1. `on_message` – skip Bots/Mods/andere Channels; Cooldown pro User (2s)
2. Stufe 1: MiniMax-Klassifikation mit `verdict/category/confidence/reason/needs_context`
3. Bei mittlerer Konfidenz oder `needs_context` → Stufe 2 mit 25-Messages-Kontext
4. Verdict-Handling: Auto-Delete + 24h-Timeout bei NSFW/Kat. ≥0.90; Mod-Vorschlag bei 0.55–0.89; Ragebait → Counter; sonst nichts
5. Mod-Buttons (`manage_messages`): Accept = 1D-Timeout + Delete + Log; Deny = Modal mit Pflicht-Begründung + Log
6. Log-Channel erhält Embed + forwarded Original bei jeder Aktion

## Status
GPT-Worker 1 (`cogs/ai_connector.py`) und GPT-Worker 2 (`cogs/ai_moderator.py`) haben die Implementierung geliefert. Statische Verifikation fuer beide Files erfolgreich.

## Fortschritt
- GPT-Worker 1 erweitert `cogs/ai_connector.py` um `generate_multimodal(...)` fuer MiniMax inkl. Bild-Content-Arrays fuer Token-Plan und Standard-Endpoint.
- Verifikation fuer `cogs/ai_connector.py` erfolgreich: `py_compile` + Signatur-Check fuer `AIConnector.generate_multimodal`.
- GPT-Worker 2 hat `cogs/ai_moderator.py` komplett neu angelegt: Config, deutsches Moderations-Prompt, `on_message`-Flow, SQLite-Schema, Ragebait-Counter, persistente Accept/Deny-UI, Deny-Modal und Log-Embeds.
- Verifikation fuer `cogs/ai_moderator.py` erfolgreich: `python3 -m py_compile` + `ast.parse(...)`.

## Offen
- Live-Smoke-Test im Zielchannel `1289721245281292291` (passiert beim naechsten echten Chat)

## Fortschritt 2026-06-02
- `cogs/ai_moderator.py`: `scam` als Auto-Delete-Kategorie + Prompt-Kategorie ergaenzt und Mod-Review um `Ban`-Button samt Ban-Handler/DB-Markierung/DynamicItem-Registrierung erweitert.

## Erledigt nach Review
- Review durch Claude: DynamicItem-basierte persistente Buttons, saubere DB-Operationen, defensives AI-JSON-Parsing, korrektes `manage_messages`-Gate, Cleanup-Loop aktiv.
- Bot-Restart via `deadlock-services.sh restart bot`. systemd: active, 47/47 Cogs geladen, `cogs.ai_connector` und `cogs.ai_moderator` beide geladen.
- DB-Tabellen `ai_moderation_cases` + `ai_moderation_ragebait_hits` im `data/deadlock.sqlite3` verifiziert.
- Keine Runtime-Errors in journalctl.

---

# Tierlist Public Backend Cog (2026-04-28)

## Ziel
Neuer Backend-Cog `tierlist_public` für die öffentliche Deadlock-Tierliste: dedizierter aiohttp-Service, Snapshot-Refresh aus deadlock-api, Public- und Admin-API auf Basis des bestehenden Dashboard-Discord-OAuth-Cookies.

## Fortschritt
- Spec, bestehende Service-Patterns (`turnier_public`, `public_stats`), Dashboard-Session-Handling und DB-Schema-Setup gesichtet.
- `service/db.py` um `tierlist_*`-Tabellen + Snapshot-Indizes erweitert.
- `service/dashboard.py` exportiert jetzt `validate_discord_session(...)` für Cookie-Wiederverwendung.
- Neuer `service/tierlist_public.py` mit Refresh-Loop, Public-/Admin-API, In-Memory-Vote-Rate-Limit und on-the-fly Tier-Berechnung.
- Neuer Cog `cogs/tierlist_public_cog.py`; Loader-Anpassung war nicht nötig, da `bot_core/cog_loader.py` bereits Auto-Discovery über `setup()` macht.
- Tests `tests/test_tierlist_refresh.py` und `tests/test_tierlist_endpoints.py` ergänzt; lokaler `unittest`-Lauf grün.

## Offen
- Umgebung hat kein `python`-Alias und kein installiertes `pytest`; Verifikation lief daher mit `python3` bzw. `python3 -m unittest`.

---

# LFG Lobby/Lane Vorschlags-Overhaul (2026-04-22)

## Ziel
`cogs/lfg.py` Vorschlagslogik bereinigen: Flow-Trennung Lobby-Suche vs Spielersuche, Offtopic-Filter, Juice-Kammer als Eternus-Preset, Präsenz-basierte Füll-Anzeige, Voll-Hinweis, Staging-Verlinkung, Rank-Warnung.

## Plan
`/home/naniadm/.claude/plans/lfg-py-ich-bin-nicht-delegated-breeze.md`

## Status
Implementierung durch GPT-Worker abgeschlossen. Statische Verifikation (`py_compile`, `ast.parse`) erfolgreich.

## Offen
- Claude-Review.
- Live-Tests (A-H) im Bot-Prozess.
- Commit+Push.

## Entscheidungen (aus Rückfragen)
- **Flow-Trennung**: User im gescannten VC = Spielersuche (nur Ping, keine Lobby-Vorschläge). User nicht im VC = Lobby-Suche (nur Lobbys, keine Mitspielerliste, kein Ping).
- **Offtopic**: Substring `"off topic voice"` (case-insensitive) im Channel-Namen filtern.
- **Juice Kammer** (Channel-ID `1493690350580138114`): fest als Eternus (Rank 11) einstufen.
- **Staging-IDs**: Casual `1501089974093873232`, Street Brawl `1357422958544420944`, Ranked `1412804671432818890`. New Player: Kategorie `1465839366634209361` scannen, ersten Channel mit <6 Leuten (adaptive Channels).
- **Voll-Hinweis**: ab 6 Leuten im VC.
- **Füll-Anzeige**: kombiniert "Deadlock-aktiv / VC-Gesamt" via Steam-Presence.
- **Rank-Warnung**: ab >1.5 Ränge Diff Suffix `⚠️ höher als dein Rang`.
- **Neue-Spieler-Erkennung**: bestehend ok, nicht anfassen.

## Erledigt
- Konstanten für Staging, Juice Kammer, Offtopic-Filter, Voll-Schwelle und Rank-Warnung ergänzt.
- `LaneInfo` um `deadlock_active_count` erweitert; VC-Scan zählt jetzt Deadlock-aktive User via Steam-Presence.
- Offtopic-Channels werden in allen relevanten Kategorie-Scans übersprungen; Juice Kammer wird fest als `Eternus (fix)` gerankt.
- Lobby-Feldtext auf `aktiv im VC` umgestellt, inkl. Voll-Hinweis ab 6 Leuten.
- Neue Helper für Presence-Load und Staging-Auflösung eingebaut; Staging-Hinweise verlinken jetzt mit `<#channel>`.
- Dispatcher trennt jetzt sauber zwischen Spielersuche (nur Mitspieler-Embed + `Deine Lobby`) und Lobby-Suche (nur Lobby-Embed).
- Decision-Log enthält jetzt den Mode `player` oder `lobby`.

---

# Aktivitäts-Tracking: Text-Scoring + Leaderboards + Public-API (2026-04-17)

## Ziel
Server-Grinding fair machen: Text wird konversations-qualität-gescored (nicht Spam-Count), getrennte Leaderboards Voice/Text auf Discord, Public-API + Discord-OAuth damit die Website (dl-activity) Leaderboard + Personal-Dashboard zeigen kann.

## Plan
`/home/naniadm/.claude/plans/wir-haben-ja-aktivit-ts-piped-crown.md`

## Arbeitsteilung
- **Claude (Orchestrator + Frontend):** Website `dl-activity` Vite-Subprojekt
- **GPT-Worker A (Backend-Tracking):** DB-Schema (`text_stats`, `text_conversation_log`), on_message Hybrid-Scoring (10-Min-Sessions, sqrt-Diminishing, Multi-User-Bonus ×1.5, Reply-Bonus), `!tleaderboard` + Footer-Eigenposition in `!vleaderboard`
- **GPT-Worker B (Backend-API):** 8 neue `/api/public/*` Endpoints in `service/public_stats.py` + Discord-OAuth (login/callback/logout, signed Session-Cookie)

## Erledigt
- GPT-Worker A: DB-Schema für Text-Scoring ergänzt (`text_stats`, `text_conversation_log` inkl. Indizes).
- GPT-Worker A: `UserActivityAnalyzer` um Hybrid-Text-Scoring mit 10-Min-Sessionfenstern, Reply-Bonus, Interaktionsbonus und periodischem Flush erweitert.
- GPT-Worker A: Discord-Commands ergänzt/erweitert: `!tleaderboard` neu, `!vleaderboard` Footer mit eigener Position + Embed-Empty-State.
- GPT-Worker B: `service/public_stats.py` um `/api/public/leaderboard/voice`, `/api/public/leaderboard/text`, `/api/public/me`, `/api/public/me/stats`, `/api/public/me/voice-history`, `/api/public/me/text-history`, `/api/public/me/heatmap`, `/api/public/me/co-players` erweitert.
- GPT-Worker B: Discord-OAuth in `service/public_stats.py` ergänzt (`/auth/discord/login`, `/auth/discord/callback`, `/auth/discord/logout`) inkl. signiertem `dl_session`-Cookie, signiertem OAuth-State-Cookie und CORS/Preflight für Dev-Origins.
- GPT-Worker B: Smoke-Verifikation lokal sauber: `python3 -m py_compile service/public_stats.py`, `python3 -c "from service import public_stats"`, `grep -n "def _handle_" service/public_stats.py`.

## Offen
- Discord Developer Portal: Redirect-URI `http://127.0.0.1:8768/auth/discord/callback` (bzw. Prod-URL) als OAuth-Redirect eintragen
- Live E2E im Bot-Prozess (Bot-Neustart erforderlich, bewusst verschoben)

## Entscheidungen
- Session-Signing-Secret: `SESSIONS_ENCRYPTION_KEY` wird wiederverwendet (Fallback im Code eingebaut, `PUBLIC_STATS_SESSION_SECRET` bleibt optional).
- `DISCORD_OAUTH_CLIENT_ID` + `DISCORD_OAUTH_CLIENT_SECRET` existieren bereits in Infisical.
- Redirect-URI: Default `http://127.0.0.1:8768/auth/discord/callback` genügt lokal; Prod-URL via `DISCORD_OAUTH_REDIRECT_URI` setzbar.

---

# Coaching Overhaul (2026-04-16)

## Ziel
Coaching-System Bot+Website stabilisieren: Parsing raus, echte Ränge, kritische Bugs weg, API abgesichert.

## Status
Durchgang 1 abgeschlossen. Änderungen liegen unstaged — noch nicht committed, User review offen.

## Erledigt

### Bot (`Deadlock-Bots/`)
- `cogs/coaching_panel.py`: `_split_rank_input` + `_split_games_hours` entfernt. Modal-Placeholder mit echten Deadlock-Rängen (Archon/Ascendant/Emissary als Beispiele). Rohtext wird jetzt direkt in `rank` / `games_played` gespeichert, `subrank`/`hours_played` bleiben leer.
- `cogs/coaching_request.py`: AI-Prompt und Embed auf Rohdaten umgestellt (kein künstliches `Subrank N/A` mehr). `_get_availability_label` entfernt. CoachClaim-Callback komplett umgebaut: `defer()` + `followup.send` überall (eliminiert Double-Response-Crash). Thread-Create-Fehler werden sauber gemeldet, DB bleibt konsistent. DM-Fail an den User ist kein fataler Fehler mehr — Session + Thread bleiben aktiv, Coach wird informiert. Zusätzlich outer try/except, damit der Button nie stumm crasht.
- `cogs/coaching_survey.py`: `on_voice_state_update` hat jetzt `@commands.Cog.listener()` — Voice-Events werden endlich empfangen, Survey-Trigger funktioniert wieder in Echtzeit.
- `Docs/deadlock-bots/coaching.md` komplett auf den echten Flow umgeschrieben (Panel → Modal → AI → Coach-Claim → Thread, inkl. Rang-Liste).

### Website-Backend (`Website/builds/backend/app/routers/coaching.py`) — via GPT-Worker `36de3803fac3`
- `require_bot_token()` Dependency (Header `X-Bot-Token`, hmac.compare_digest, 503 wenn ENV fehlt, 401 bei falschem Token) an `POST /requests`, `PATCH /requests/{id}/match`, `POST /surveys`.
- Anonymitäts-Leak in Reviews gefixt: stabiles `sha256(user_id+coach_id)[:6]`-Label statt Username-Präfix.
- Neue ENV: `COACHING_BOT_TOKEN`.

## Offen / bewusst verschoben
- `GET /api/coaching/requests` weiterhin public — nicht akut, aber sollte in einem Folge-Pass auch bot-gated werden.
- Sync-Layer Bot↔Website (aktuell getrennte DBs).
- Frontend-Routen `/coaching/apply`, `/coaching/dashboard`, Coaching-anfragen-Button-Logik.
- Discord-Rolle automatisch bei approvter Coach-Application vergeben.
- User-seitiges Cancel bewusst ausgelassen (User-Entscheidung).

## Verifikation
- `python3 -m py_compile` sauber für: `coaching_panel.py`, `coaching_request.py`, `coaching_survey.py`, `coaching_role_manager.py`, `builds/backend/app/routers/coaching.py`.
- Kein Commit / kein Push bisher.

## Nächster Schritt
User reviewt Änderungen. Bei OK: `COACHING_BOT_TOKEN` setzen (Infisical), dann commit+push in beiden Repos.

---

# Tag-System Phase 1 — Welle 1 (2026-04-29)

## Ziel
Nur Foundation aus Phase 1 umsetzen: DB-Schema, neues `cogs/tags/`-Cog mit `TagService`, Cache/Rehydration/Cleanup und dedizierte Tests.

## Fortschritt
- Plan und Spec für Phase 1 gesichtet, Scope auf Welle 1 begrenzt.
- Bestehende Patterns in `service/db.py`, `cogs/tempvoice/` und den vorhandenen Async-Tests abgeglichen.
- TDD durchgezogen: `tests/test_tag_service.py` zuerst angelegt, danach `service/db.py` und `cogs/tags/` bis zum grünen Lauf ergänzt.
- `TagService` implementiert: User-/Mod-Tag-CRUD, In-Memory-Cache, Rehydration via `cog_load()`, 5-Minuten-Cleanup-Loop und `bot.dispatch(...)`-Events.
- Kein separater Loader-Patch nötig: `bot_core/cog_loader.py` nutzt Auto-Discovery; `cogs/tags/__init__.py` stellt dafür ein konsistentes `setup()` bereit.
- Verifikation grün: `pytest tests/test_tag_service.py -v` (in temporärer venv mit `pytest`) und `ruff check service/db.py cogs/tags tests/test_tag_service.py`.

## Offen
- Welle 2 ist bewusst offen: Commands, Onboarding-, TempVoice-, AI-Mod- und LFG-Integration wurden in dieser Welle nicht angefasst.
- Änderungen sind absichtlich uncommitted; Commit/Push bleibt beim Orchestrator.

---

# Tag-System Phase 1 — Welle 2 / TempVoice Tag-Filter (2026-04-29)

## Fortschritt
- `cogs/tempvoice/core.py` um `LaneTagFilter`, DB-Rehydration/Persistenz, `_apply_tag_filter()`, Join-Enforcement und `on_mod_tag_added`-Cleanup ergänzt.
- `cogs/tempvoice/interface.py` um den Button `🛡️ Tag-Filter` sowie eine Config-View mit drei Single-Selects und Save-Flow erweitert.
- Neuer Test `tests/test_tempvoice_tag_filter.py` deckt Persistenz, Min-Age-Block und Ragebaiter-Cleanup ab.
- Verifikation lokal grün: `pytest tests/test_tempvoice_tag_filter.py -v`, `pytest tests/test_tempvoice_core.py tests/test_tempvoice_lane_sorting.py -v`, `ruff check cogs/tempvoice/core.py cogs/tempvoice/interface.py tests/test_tempvoice_tag_filter.py`.

## Offen
- Kein Live-Discord-Smoke-Test in dieser Worker-Phase.

---

# CodeQL HTML/JS/YAML Fixes (2026-05-11)

## Fortschritt
- `service/static/activity_stats.html`: Chart.js-CDN-Script auf verifizierten `sha384`-SRI umgestellt und `renderBestTimes()` gegen DOM-XSS abgesichert, indem `rank` und `color` vor `innerHTML` escaped werden.
- `cogs/steam/steam_presence/index.js`: ungenutzten `crypto`-Import entfernt.
- `.github/workflows/automerge-trusted-prs.yml`: `run:`-Zeile bei der Skip-Note auf Block-Scalar umgestellt, damit der Doppelpunkt im Echo keine YAML-Syntax mehr bricht.

## Verifikation
- SRI-Hash per `curl | openssl dgst -sha384 -binary | base64` gegen jsDelivr verifiziert.
- HTML-Tag-Check fuer `service/static/activity_stats.html` erfolgreich.
- `node --check cogs/steam/steam_presence/index.js` erfolgreich.
- `python3 -c "import yaml; yaml.safe_load(...)"` fuer `.github/workflows/automerge-trusted-prs.yml` erfolgreich.

---

# CodeQL-Fixpass Python (2026-05-11)

## Ziel
Mehrere CodeQL-Sicherheits- und Quality-Alerts in der Python-Codebase mit minimalen, chirurgischen Aenderungen beheben.

## Fortschritt
- Leere `except`-Bloecke werden mit begruendenden Kommentaren versehen.
- Zwei `bare except`-Stellen in `cogs/faq_chat.py` werden auf `except Exception` eingegrenzt.
- Gezielte Cleanups fuer `inner_self`, ungenutzte Variablen, Test-Imports und Log-Sanitizing umgesetzt.

## Verifikation
- `python3 -m py_compile ...` fuer alle betroffenen Python-Dateien erfolgreich.
- `python3 -m pytest tests/ -x -q 2>&1 | tail -20` nicht ausfuehrbar: `/usr/bin/python3: No module named pytest`.

---

# FAQ-Doku W2 – Coaching/Onboarding/Stats/Tierlist/Rules/FAQ-Meta (2026-05-23)

## Ziel
Sechs neue user-facing Markdown-Dateien fuer den FAQ-Bot in `docs/` erstellen: Coaching, Onboarding/Invites, Stats/Privacy, Tierlist/Builds, Rules/Channels und FAQ-Bot-Meta.

## Fortschritt
- Referenz-Plan unter `/home/naniadm/.claude/plans/ich-m-chte-das-wir-floating-mountain.md` gelesen und Template/Scope uebernommen.
- Relevante Cogs und bestehende Doku gesichtet: `cogs/coaching_*`, `cogs/onboarding.py`, `cogs/ai_onboarding.py`, `cogs/welcome_dm/*`, `cogs/website_invite_cog.py`, `cogs/public_stats_cog.py`, `cogs/privacy_*`, `cogs/user_activity_analyzer.py`, `cogs/user_retention.py`, `cogs/tierlist_public_cog.py`, `cogs/turnier_public_cog.py`, `cogs/build_publisher.py`, `cogs/rules_channel.py`, `cogs/faq_chat.py`, `cogs/server_faq.py`.
- Channel-/User-Flows, Slash-Commands, Session-Laufzeiten, sichtbare Rollen- und Feedback-Regeln fuer die neue Doku extrahiert.

## Offen
- Sechs Markdown-Dateien jetzt schreiben und danach Wortzahlen/Inventur pruefen.

---

# FAQ-Doku W3 - Master-Bot Internal-Doku (2026-05-23)

## Ziel
Acht technische Internal-Dokus unter `docs/internal/` fuer Admins, Mods und Devs erstellen: Admin-Commands, Security Guard, AI-Moderator, Build-Publisher, Rename-Manager, Claim-System, AI-Onboarding-Pipeline und Steam-Bridge-Watchdog.

## Fortschritt
- Referenz-Plan gelesen und Scope auf Internal-Doku im Master-Bot begrenzt.
- Relevante Quellen gesichtet: `docs/admin_commands.md`, `docs/build-publishing/AUTONOMER_BETRIEB.md`, `docs/steam-bridge-watchdog.md`, `cogs/security_guard.py`, `cogs/ai_moderator.py`, `cogs/ai_connector.py`, `cogs/build_publisher.py`, `cogs/rename_manager.py`, `cogs/claim_system.py`, `cogs/ai_onboarding.py`, `standalone/steam_bridge_watchdog.py`.
- Command-Scan fuer Admin-/Owner-relevante Commands und Sonderpfade (persistente Views, SQLite-Tabellen, Socket-Listener, Watchdog-Restarts) durchgefuehrt.
- Auffaelligkeiten notiert: `rename_manager.py` hat aktuell keine In-Repo-Caller; `claim_system.py` wirkt als externer Socket-Entry-Point; Build-Publisher-Runbook enthaelt teils veraltete Windows-Pfade.

## Offen
- Acht Markdown-Dateien jetzt schreiben, Wortzahlen pruefen und kurzen Verifikationslauf machen.

# P1 Server-as-Code (Schema+Import+Diff+Apply-Geruest) (2026-07-02)

## Ziel
Phase-1-Backend fuer deklarative Discord-Serverstruktur: Central-DB-Schema, Ist-Import, Soll/Ist-Diff, Zwei-Schritt-Apply-Geruest, Drift-Erkennung und Adopt-Funktion. Kein Commit/Push, keine Live-Guild-Tests.

## Fortschritt
- Pflichtdokumente gelesen: Konzept §5.1/§5.3/§7, Ist-Zustand und Rechte-Soll-Entwurf.
- Bestehende `dl-central-db`-Migrationen/Test-Harness gesichtet; neue Migrationen werden im geforderten 202607021*-Bereich angelegt.
- Neues Crate `dl-server-as-code` angelegt: reine Modelle, Diff-Engine, Placeholder-Human-Summary, Serenity-Ist-Import, Preview-Persistenz, confirmed-hash Apply mit Dry-Run-Default, Drift-Events und Adopt-Funktion.
- Migration `2026070210_server_config_schema.sql` angelegt: `server_config`-Schema fuer Soll/Ist-Struktur, dynamische Namespaces, dokumentierte Ausnahmen, Diff-Previews, Apply-Runs, Auto-Revert-Whitelist, Drift- und Adoption-Events.
- Tests umgesetzt: 5 pure Diff-Tests plus 4 ignored DB-Workflow-Tests gegen `dl_central_db::testing`; Fresh-Migration-Schema-Test in `dl-central-db` erweitert.
- Verifikation gruen: `cargo build -p dl-server-as-code`; `cargo clippy -p dl-server-as-code --all-targets -- -D warnings`; `cargo clippy -p dl-central-db --features testing --all-targets -- -D warnings`; `cargo test -p dl-server-as-code`; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored`; `cargo test -p dl-central-db`; `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored`; `cargo fmt -p dl-server-as-code -p dl-central-db -- --check`; `git diff --check`.
