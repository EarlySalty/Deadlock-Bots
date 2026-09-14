# Plan: Paten-Programm live

status: aktiv
datum: 2026-09-09
contract: CONTRACT.md

## Scope-Warnung vorab (muss der User vor M2 klären)

Drei Anforderungen brauchen Dateien, die NICHT im erlaubten Änderungsbereich des
Contracts stehen. Ohne sie sind REQ-3 (Karte editieren), REQ-4 (Rolle vergeben)
und REQ-7 (neuer Willkommen-Abschnitt) technisch nicht umsetzbar, weil dl-community
den Discord-Adapter nur über die Port-Trait-Implementierungen in dl-bot erreicht und
der Willkommen-Hub seine Sektionen fest in einem serversync-Untermodul definiert:

- `rust/bin/dl-bot/src/modglue.rs`: hier liegen `ConciergeGlue` (impl `ConciergePort`, ab Zeile 2224) und `TeamApplicationGlue` (impl `TeamApplicationPort`, ab 3008). REQ-4 braucht eine neue `add_role`-Portmethode mit Impl hier (Vorlage: `CoachingReqGlue::add_role`, modglue.rs:3280, ruft `add_member_role`). REQ-3 braucht eine neue `edit_channel_v2`-Portmethode mit Impl hier (Vorlage: `edit_rich`/`http.edit_message`, modglue.rs:2481).
- `rust/bin/dl-bot/src/serversync/welcome_publish.rs`: hier stehen `welcome_sections()` und die Text-Struct (ab Zeile 269/306). REQ-7 braucht eine neue Sektion "paten" plus Textfelder; `welcome_texts.toml` allein reicht nicht, weil die Struct `deny_unknown_fields` fährt und die Sektionsliste im Code fest ist.

Empfehlung: Der User legt die Freigabe-Datei `~/.claude/.contract-approvals/2026-09-09-paten-programm-live` an (ein Pfad je Zeile) mit genau diesen zwei Zusatzpfaden, ODER hängt ein `## Amendment` an den Contract, das den erlaubten Bereich um beide Dateien erweitert. `diff-policy.py` liest keine Amendments, deshalb ist die Freigabe-Datei der sichere Weg (Memory `contract-scope-ein-pfad-je-zeile`, `push-scan-gitleaks-target-rmeta`). Bis das vorliegt, stoppt M2 nicht, aber M3 (REQ-3-Teil Karte), M5 (REQ-4) und M8 (REQ-7) dürfen nicht gemerged werden.

## Architekturentscheidungen

- **Onboarding-Wahl zum Concierge: Lookup der Frischling-Rolle im Concierge, keine Signaturerweiterung.** `handle_native_onboarding_completed_inner` (concierge.rs:3366) fragt über die bestehende Portmethode `role_member_ids(guild_id, FRESHLING_ROLE_ID)` (neue Konstante `FRESHLING_ROLE_ID = 1522384961481609236`), ob der User Frischling ist. Begründung: der Concierge abonniert `MemberEvent::NativeOnboardingCompleted` (concierge.rs:7080) ohne Rollendaten, `dl-discord` ist außerhalb des Scope, eine Signaturerweiterung bräuchte trotzdem denselben Port-Lookup; er gehört also dorthin, wo die Wahl gebraucht wird, und spart die Änderung am Broadcast-Contract.
- **Datenmodell der Anfrage-Tabelle: neue additive Tabelle `bot.concierge_pate_requests`.** Spalten: `id BIGSERIAL PK`, `user_id BIGINT NOT NULL`, `guild_id BIGINT NOT NULL`, `channel_id BIGINT NOT NULL`, `message_id BIGINT NOT NULL`, `status TEXT NOT NULL DEFAULT 'open'` (Werte `open` / `claimed` / `closed_unbesetzt`), `pate_id BIGINT NULL`, `created_at TIMESTAMPTZ NOT NULL`, `escalated_2h_at TIMESTAMPTZ NULL`, `escalated_24h_at TIMESTAMPTZ NULL`, `claimed_at TIMESTAMPTZ NULL`, `closed_at TIMESTAMPTZ NULL`, `updated_at TIMESTAMPTZ NOT NULL DEFAULT now()`. Indizes: `CREATE INDEX ... ON bot.concierge_pate_requests(status, created_at)` für die Eskalationsabfrage; `CREATE UNIQUE INDEX ... ON bot.concierge_pate_requests(user_id) WHERE status = 'open'` als Einmal-Garantie "eine offene Anfrage je Neuling" (deckungsgleich mit dem bestehenden `pate_requested`-Schutz in `request_pate`). Einmal-je-Stufe: jede Eskalation feuert über ein bedingtes UPDATE `SET escalated_2h_at = now() ... WHERE id = $1 AND escalated_2h_at IS NULL RETURNING id` (bzw. `escalated_24h_at`); nur wer die Zeile zurückbekommt, sendet. Weil der Marker persistiert ist, feuert nichts doppelt, auch über Neustarts hinweg. Additiv, keine Änderung an `concierge_patenschaften`/`concierge_profiles` (Verbotene Änderungen eingehalten).
- **Eskalationsprüfung im bestehenden Scheduler.** `run_scheduler` (concierge.rs:4849) bekommt einen neuen Aufruf `self.run_pate_escalations(now)` NACH dem `if self.config.proactive`-Block und unabhängig davon, weil die Anfrage vom Neuling selbst ausgelöst wurde (reaktiv, INV-1) und nicht am Proaktiv-Schalter hängt. Der 5-Minuten-Takt (`SCHEDULER_INTERVAL`) reicht für Stufen bei 2 h und 24 h. Opt-out des Neulings unterdrückt die 24-h-DM (INV-2).
- **Leitfaden-Poster merkt den Nachrichten-Zustand wie `team_applications::ensure_panel`.** Neue Concierge-Methode `ensure_pate_leitfaden()`, in `main.rs` beim Start aufgerufen (Vorlage: team_applications-Verdrahtung main.rs:1066). Zustand in `bot.kv_store`: Namespace `concierge:pate_leitfaden`, Key `message_id` (gepostete Nachricht) plus Key `fingerprint` (Hash des gerenderten Inhalts). Kein `message_id` gespeichert, dann posten und beide Keys schreiben. `message_id` vorhanden und Fingerprint gleich, dann nichts tun. `message_id` vorhanden und Fingerprint anders, dann Karte per `edit_channel_v2` überschreiben und Fingerprint aktualisieren, statt eine zweite zu posten (REQ-5 "aktualisiert ... statt eine zweite zu posten").
- **Mention-Antwort in Paten-Kanälen nutzt den bestehenden Wissenspfad und schließt persönliche Buttons aus.** `effective_guild_id` (concierge.rs) wird erweitert: ein Kanal zählt als Wissenskanal, wenn `channel_id == PATE_REQUEST_CHANNEL_ID` ODER die `channel_id` als aktive Patenschaft in `bot.concierge_patenschaften` (WHERE `released_at IS NULL`) steht (die privaten `pate-<user_id>`-Kanäle; deren ID wird beim Anlegen dort gespeichert). Zusätzlich Gate: nur wenn der Text den Bot per Mention anspricht. Mention-Erkennung ohne dl-discord-Änderung über den Rohtext (`<@BOT_ID>` / `<@!BOT_ID>`); dazu neues Config-Feld `bot_user_id: Option<u64>` in `ConciergeConfig`, in `main.rs` aus `adapter.cache().current_user().id.get()` gesetzt (Vorlage modglue.rs:1954). Die Antwort läuft durch `answer_dm_question_inner` mit `AnswerOptions { allow_personal_actions: false, route: AnswerRoute::Concierge, ... }` und damit über `knowledge_client::ask(&config.knowledge_url, ...)` (concierge.rs:4415). Ohne Mention bleibt der Concierge still (kein Antwortpfad, `mark_first_message` wie bisher). Kein neuer Kanal, keine persönlichen Aktionsbuttons (REQ-6, deckt die bestehenden Tests 9169/9437 ab).
- **Rollenvergabe bei Annahme einer Paten-Bewerbung in team_applications.** In `change_status` (team_applications.rs:1138), Zweig `ApplicationStatus::Accepted`, wird bei `kind == ApplicationKind::Pate` nach dem erfolgreichen Status-Update `self.port.add_role(self.guild_id, applicant_user_id, PATE_ROLE_ID, "team-bewerbung:pate-angenommen")` aufgerufen und der Accepted-DM-Text für Pate durch eine eigene Fassung mit Leitfaden-Verweis ersetzt (`status_dm_text`, Zweig Accepted plus kind Pate). Portmethode `add_role` existiert im Trait noch nicht (grep leer, EVIDENCE:22); sie wird zu `TeamApplicationPort` hinzugefügt und in `TeamApplicationGlue` (modglue.rs) implementiert (Vorlage `CoachingReqGlue::add_role`, modglue.rs:3280). Für alle anderen Kinds bleibt der Pfad exakt gleich (INV-7): kein add_role-Aufruf, kein geänderter DM-Text.

## Milestones

### M1: Rote Regressionstests und Baseline

Änderungen (Dateien): `rust/crates/dl-community/src/concierge.rs` (Test-Modul, plus Mock-Erweiterungen: `MockConciergePort` bekommt ein Feld `frischling_members: Mutex<Vec<u64>>` das `role_member_ids` bedient, und `sent_dm_v2` speichert künftig den vollen Body statt nur der `user_id`, damit die T0-Buttons prüfbar sind), `rust/crates/dl-community/src/team_applications.rs` (Test-Modul plus Mock-`TeamApplicationPort`, der `add_role`-Aufrufe mitschreibt).

Neue Tests (alle aus REQ-9), jeder muss VOR der Fachänderung rot sein:

1. `frischling_t0_enthaelt_paten_angebot_und_setzt_pate_offered` (REQ-1): `proactive = true`, Mock liefert User 42 als Frischling-Rollenmitglied; nach `handle_native_onboarding_completed(guild, 42)` enthält der gesendete T0-DM-Body die Buttons `concierge:pate:yes` und `concierge:pate:no`, `pate_offered` ist in der DB gesetzt und ein Journey-Event `pate_offered` existiert. Braucht Test-DB (`test_pool`) und Feature `testing`.
2. `pate_anfrage_eskaliert_nach_2h_genau_einmal` (REQ-3): offene Anfrage mit `created_at = now - 2h5min`; nach `run_pate_escalations(now)` genau ein Kartenupdate mit Owner-Ping (662995601738170389) und `escalated_2h_at` gesetzt; ein zweiter Lauf sendet nichts mehr. Test-DB, Feature testing.
3. `pate_anfrage_schliesst_nach_24h_genau_einmal_mit_dm` (REQ-3): offene Anfrage `created_at = now - 24h5min`; genau eine ehrliche DM an den Neuling, Karte auf `closed_unbesetzt`, `escalated_24h_at` gesetzt; zweiter Lauf sendet nichts. Test-DB, Feature testing.
4. `pate_eskalation_feuert_nach_neustart_nicht_erneut` (REQ-3, Neustart-Idempotenz): Anfrage mit bereits gesetzten `escalated_2h_at`/`escalated_24h_at`; frisch aufgebauter Concierge (neue Instanz, gleiche DB) sendet in `run_pate_escalations` nichts. Test-DB, Feature testing.
5. `uebernehmen_nach_geschlossener_anfrage_bleibt_wirkungslos` (REQ-3): Anfrage `status = closed_unbesetzt`; `claim_pate` mit Paten-Rolle liefert die klare Rückmeldung (neuer Text `PATE_REQUEST_CLOSED_TEXT`), legt keine Patenschaft an. Test-DB, Feature testing.
6. `angenommene_paten_bewerbung_vergibt_paten_rolle_und_schickt_leitfaden_dm` (REQ-4): Pate-Bewerbung im Zustand open; `change_status(accept)` durch einen Mod ruft `add_role(guild, applicant, PATE_ROLE_ID, ...)` genau einmal und die Accepted-DM enthält den Leitfaden-Verweis. Test-DB, Feature testing.

Baseline-Messung der bestehenden Tests (bekannte grüne/rote Ausgangslage festhalten, Memory `tb-bot-build-toolchain`/`rolle-test-waechter`).

Validierungsbefehl (roten Lauf je neuem Test in `RED-BASELINE.md` festhalten, mit Testname und Assertion-Fehlermeldung):
```
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo test -p dl-community --features testing \
  frischling_t0_enthaelt_paten_angebot_und_setzt_pate_offered -- --nocapture
```
(analog je Testname aus 2 bis 6; Skript setzt `CENTRAL_TEST_DSN`, spinnt den Wegwerf-Timescale-Container und wendet `dl-central-migrate` an).

Voller Baseline-Lauf des Crates:
```
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo test -p dl-community --features testing
```

Erwarteter Zwischenzustand: sechs neue Tests kompilieren und sind rot; alle bestehenden Tests unverändert grün (bzw. bekannte Baseline).

Stop-Regel: Ist auch nur einer der neuen Tests von Anfang an grün, trifft er den Bug nicht; Test schärfen, bevor M2 beginnt. Kompiliert das Crate wegen der Mock-Erweiterungen nicht, zuerst das reparieren (kein Fachcode).

### M2: Migration und Store-Funktionen

Änderungen: `rust/crates/dl-central-db/migrations/2026090910_concierge_pate_requests.sql` (Tabelle plus zwei Indizes wie oben), Store-Funktionen in `rust/crates/dl-community/src/concierge.rs` (die bestehenden Concierge-Queries liegen inline dort): `insert_pate_request_tx`, `mark_pate_request_claimed_tx`, `due_pate_escalations(now)` (SELECT offener Anfragen älter als 2 h ohne `escalated_2h_at` bzw. älter als 24 h ohne `escalated_24h_at`), `claim_pate_escalation_stage(id, stage)` (bedingtes UPDATE mit RETURNING), `close_pate_request_unbesetzt_tx`, `pate_request_status(user_id)`, `pate_inventar_counts()`. `rust/.sqlx/` neu erzeugen.

Erwarteter Zwischenzustand: Migration läuft im Test-Container fehlerfrei, Store-Funktionen kompilieren, noch kein neues Verhalten verdrahtet; die neuen Tests bleiben rot.

Validierungsbefehl:
```
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo build -p dl-community -p dl-central-db
```
sqlx-Offline nach migriertem Test-Container:
```
cd /home/nathanael/repos/Deadlock-Bots/rust && ./scripts/central_test_db.sh bash -c \
  'DATABASE_URL="$CENTRAL_TEST_DSN" SQLX_OFFLINE=false /home/nathanael/.cargo/bin/cargo sqlx prepare --workspace'
```

Stop-Regel: Migration nicht idempotent oder Unique-Index kollidiert mit Altbestand (es gibt 0 Zeilen laut EVIDENCE, also erwartet sauber), dann Migration korrigieren, nie eine bestehende Migration editieren.

### M3: Frischling-T0 (REQ-1) plus journeyglue-Prüfung

Änderungen: `rust/crates/dl-community/src/concierge.rs` (neue Konstante `FRESHLING_ROLE_ID`, Frischling-Erkennung via `role_member_ids` im Inner-Handler, neuer T0-Body mit Pate-Angebot und zweiter Button-Reihe, `set_pate_offered_tx` plus Journey `PateOffered` im bestätigten Erfolgspfad wie die übrigen `PendingDiscordEffect`), `rust/bin/dl-bot/src/journeyglue.rs` nur prüfen (die Concierge-Subscription läuft eigenständig über concierge.rs:7080; journeyglue reicht die Wahl nicht durch und muss dafür auch nicht angefasst werden, EVIDENCE:21 ist damit als "Lookup statt Durchreichen" entschieden).

Erwarteter Zwischenzustand: Test 1 grün; Nicht-Frischlinge bekommen unverändert das alte T0 und das Pate-Angebot weiter erst per T2.

Validierungsbefehl:
```
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo test -p dl-community --features testing frischling_t0 -- --nocapture
```

Stop-Regel: Der Rollen-Lookup steht beim Handler-Aufruf noch nicht zur Verfügung (Timing, siehe Risiken), dann nicht mit einem Poll pro Aufruf pfuschen, sondern als Risiko dokumentieren und mit dem User klären.

### M4: Anfrage-Persistenz und Eskalation (REQ-3)

Änderungen: `rust/crates/dl-community/src/concierge.rs`. In `request_pate` (5450) nach erhaltenem `post_message_id` und vor `tx.commit()` `insert_pate_request_tx(&mut tx, user_id, guild_id, target, post_message_id, now)` (atomar mit `set_pate_requested_tx`). In `claim_pate` (5707) im Erfolgspfad `mark_pate_request_claimed_tx` in derselben `tx` wie der Patenschafts-Insert; vor dem Insert prüfen, ob die Anfrage bereits `closed_unbesetzt` ist, und dann `PATE_REQUEST_CLOSED_TEXT` zurückgeben (Button wirkungslos). Neu `run_pate_escalations(now)`, in `run_scheduler` nach dem Proaktiv-Block aufgerufen: für jede fällige Anfrage die Stufe atomar claimen (`claim_pate_escalation_stage`), dann 2 h = Karte per `edit_channel_v2` um Hinweis plus Owner-Ping ergänzen, 24 h = ehrliche DM an den Neuling (Opt-out überspringt sie, INV-2), Karte auf `closed_unbesetzt` editieren. Neue Textkonstanten `PATE_ESCALATION_2H_TEXT`, `PATE_ESCALATION_24H_CARD_TEXT`, `PATE_UNBESETZT_DM_TEXT`, `PATE_REQUEST_CLOSED_TEXT`. Owner-Ping braucht `allowed_mentions` mit `users: [662995601738170389]` im editierten Body. `edit_channel_v2` als neue `ConciergePort`-Methode plus Impl in `modglue.rs` (Scope-Freigabe nötig, siehe oben).

Erwarteter Zwischenzustand: Tests 2 bis 5 grün.

Validierungsbefehl:
```
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo test -p dl-community --features testing pate_anfrage pate_eskalation uebernehmen_nach -- --nocapture
```

Stop-Regel: Scope-Freigabe für `modglue.rs` liegt nicht vor, dann M4-Teil "Karte editieren" nicht committen; die Persistenz und die Store-Logik (ohne Discord-Edit) dürfen vorlaufen, der Discord-Edit wartet.

### M5: Team-Bewerbung Pate (REQ-4)

Änderungen: `rust/crates/dl-community/src/team_applications.rs` (`ApplicationKind::Pate` in Enum, `ALL`, `slug` = "pate", `label` = "Pate"; Panel-Button mit dl_*-Brand-Emoji statt Unicode, INV-6; Pate-Modalfragen aus TOML statt hartcodiert; `add_role` zum `TeamApplicationPort`-Trait; `change_status`-Accept-Zweig ruft `add_role` und nutzt Pate-Accepted-DM mit Leitfaden-Verweis), `assets/team_application_texts.toml` (neuer Abschnitt `[pate]` mit `intro`, `frage_erfahrung`, `frage_verfuegbarkeit`; Text-Struct in team_applications.rs um ein optionales Pate-Feld erweitern, `deny_unknown_fields` beachten), `rust/bin/dl-bot/src/modglue.rs` (`add_role`-Impl in `TeamApplicationGlue`, Vorlage `CoachingReqGlue::add_role`).

Erwarteter Zwischenzustand: Test 6 grün; alle anderen Bewerbungsarten unverändert (INV-7), die bestehenden Team-Bewerbungs-Tests bleiben grün (INV-5).

Validierungsbefehl:
```
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo test -p dl-community --features testing team_appl paten_bewerbung -- --nocapture
```

Stop-Regel: Scope-Freigabe für `modglue.rs` fehlt, dann M5 nicht mergen (Kern von REQ-4 hängt an `add_role`).

### M6: Leitfaden-Poster und TOML (REQ-5)

Änderungen: `assets/paten_leitfaden.toml` (Texte, siehe Entwürfe), `rust/crates/dl-community/src/concierge.rs` (`ensure_pate_leitfaden()`, Components-V2-Renderer im Brand-Look, kv_store-Zustand `concierge:pate_leitfaden` message_id plus fingerprint, Edit statt Doppelpost), `rust/bin/dl-bot/src/main.rs` (Aufruf beim Start nach dem Concierge-Aufbau, Vorlage team_applications main.rs:1066), `rust/crates/dl-community/src/lib.rs` falls ein Re-Export nötig ist.

Erwarteter Zwischenzustand: Beim Start erscheint genau eine angepinnte Leitfaden-Karte in paten-zentrale; TOML-Änderung führt beim nächsten Start zu Edit, nicht zu einem zweiten Post. Kanal-IDs im Leitfaden, die noch fehlen (Neue-Spieler-Lane, Rang-Verknüpfung, Regelwerk, Custom Games, Scrims), holt der Orchestrator per dl-bot-MCP `list_channels` und trägt sie in die TOML ein.

Validierungsbefehl (Einheit): `cd rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh /home/nathanael/.cargo/bin/cargo test -p dl-community --features testing leitfaden -- --nocapture`; Live-Prüfung erst im Deploy-Milestone.

Stop-Regel: Kein Edit-Weg verfügbar (Scope), dann Poster nur "posten, wenn nicht vorhanden" ausliefern und die Edit-Funktion nachziehen, sobald `modglue.rs` freigegeben ist; nie eine zweite Karte posten.

### M7: Mention-Wissensantwort (REQ-6)

Änderungen: `rust/crates/dl-community/src/concierge.rs` (`bot_user_id` in `ConciergeConfig`, Mention-Erkennung im Rohtext, `effective_guild_id` erweitert um paten-zentrale und aktive Patenschaftskanäle, Antwort mit `allow_personal_actions: false`), `rust/bin/dl-bot/src/main.rs` (`bot_user_id` aus `adapter.cache().current_user().id.get()` in die Config setzen, nach dem Adapter-Aufbau).

Erwarteter Zwischenzustand: Mention in paten-zentrale und in `pate-<user_id>`-Kanälen liefert eine Wissensantwort ohne persönliche Buttons; ohne Mention bleibt der Bot still; die bestehenden Tests 9169/9437 bleiben grün.

Validierungsbefehl: `cd rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh /home/nathanael/.cargo/bin/cargo test -p dl-community --features testing wissensfrage paten_ mention -- --nocapture`

Stop-Regel: `current_user()` liefert vor dem Gateway-Connect keine ID, dann `bot_user_id` erst nach Cache-Warmup setzen oder als `None` behandeln (dann kein Mention-Match, kein Fehlverhalten), nicht raten.

### M8: Willkommen-Hub und Onboarding-Optionstext (REQ-7)

Änderungen: `assets/welcome_texts.toml` (Abschnitt Paten, Titel und Text), `rust/bin/dl-bot/src/serversync/welcome_publish.rs` (neue Sektion "paten" in `welcome_sections()` plus Textfelder in der Struct; Scope-Freigabe nötig), ggf. `rust/bin/dl-bot/src/serversync.rs` falls die Sektions-Verdrahtung dort sitzt. Der Optionstext der Onboarding-Wahl 🌱 wird NICHT im Code gesetzt, sondern vom Orchestrator per Discord-API (`PUT /guilds/1289721245281292288/onboarding`), Text siehe Entwurf.

Erwarteter Zwischenzustand: `/serversync/welcome-apply` rendert den Paten-Abschnitt; Optionsbeschreibung im Discord-Onboarding aktualisiert.

Validierungsbefehl: `cd rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh /home/nathanael/.cargo/bin/cargo build -p dl-bot`; Live-Prüfung im Deploy-Milestone.

Stop-Regel: Scope-Freigabe für `welcome_publish.rs` fehlt, dann Abschnitt nicht ausliefern, TOML-Text vorbereiten und warten.

### M9: Inventarzeile (REQ-8) und Startskript (REQ-2)

Änderungen: `rust/bin/dl-bot/src/aiglue.rs` (neuer Formatter `paten_inventory_line(counts)`), `rust/crates/dl-community/src/concierge.rs` (async `paten_inventar()` liefert Paten-Rollen-Anzahl via `role_member_ids(PATE_ROLE_ID)`, offene Anfragen, aktive Patenschaften, Leitfaden gepostet ja/nein aus kv_store), `rust/bin/dl-bot/src/main.rs` (Counts holen und Zeile loggen), `scripts/run_dl_bot_service.sh` (`DL_CONCIERGE_PROACTIVE=1` ergänzen, direkt bei `DL_CONCIERGE_ENABLED=1`).

Erwarteter Zwischenzustand: Startzeile nennt "Paten: N mit Rolle, M offene Anfragen, K aktive Patenschaften, Leitfaden: gepostet/fehlt"; Proaktiv-Schalter an.

Validierungsbefehl: `cd rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh /home/nathanael/.cargo/bin/cargo test -p dl-bot aiglue inventar -- --nocapture`

Stop-Regel: `run_dl_bot_service.sh` setzt schon einen anderen Proaktiv-Wert, dann nicht doppelt exportieren.

### M10: Abschluss

Änderungen: keine Fachänderung; Formatierung und Endprüfung.

Validierungsbefehle (in Reihenfolge):
```
cd /home/nathanael/repos/Deadlock-Bots/rust && /home/nathanael/.cargo/bin/cargo fmt --all
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
cd /home/nathanael/repos/Deadlock-Bots/rust && SQLX_OFFLINE=1 ./scripts/central_test_db.sh \
  /home/nathanael/.cargo/bin/cargo test --workspace --all-features
cd /home/nathanael/repos/Deadlock-Bots/rust && ./scripts/central_test_db.sh bash -c \
  'DATABASE_URL="$CENTRAL_TEST_DSN" SQLX_OFFLINE=false /home/nathanael/.cargo/bin/cargo sqlx prepare --workspace'
```
Dann Selbst-Review durch den Fixer (`gate_hook.py --review` gegen die eigene Arbeit), danach frischer adversarialer Review gegen Diff plus Contract. Keine `#[allow(...)]`, keine Warnungsunterdrückung, keine Code-Kommentare.

Deploy (NICHT im Plan-Agent, nur Weg): Prod-Migration separat über `cargo run -p dl-central-migrate` gegen die zentrale DSN (der Bot migriert nicht selbst, `connect_pool` verbindet nur, pool.rs:11); Release-Build mit Toolchain 1.97.1 im PATH (Memory `dl-bot-build-und-deploy`), Binary `rust/target/release/dl-bot`, Service `deadlock-bot-rust.service` per `systemctl --user restart`; Live-Prüfung: Frischling-Onboarding mit Zweitkonto, Karte in paten-zentrale, 2-h/24-h-Simulation gegen Testdaten, Bewerbung Pate annehmen und Rolle prüfen, Leitfaden-Post, Mention-Antwort, Willkommen-Abschnitt. Neuer öffentlicher Kanalpfad ist nicht betroffen (kein Caddy-Eintrag nötig).

## Fertige deutsche Texte (Implementierer übernimmt wörtlich)

### Frischling-T0 Pate-Absatz (an T0_TEXT angehängt, Buttons "Ja, gern" / "Nee, ich komm klar")
"Und weil du hier ganz neu bist: Ich kann dir direkt einen Paten an die Seite stellen. Das ist ein Mensch aus der Community, der dir alles zeigt und mit dir die ersten Runden dreht. Kein Programm, kein fester Termin, einfach jemand, der dir den Anfang leicht macht. Magst du?"

### Leitfaden (paten_leitfaden.toml, Components V2 in paten-zentrale)
Titel: "Dein Leitfaden als Pate"

Was du machst: "Du nimmst einen neuen Menschen an die Hand und zeigst ihm, wie hier alles laeuft. Du beantwortest seine Fragen, drehst mit ihm die ersten Runden und sorgst dafuer, dass er sich willkommen fuehlt. Du musst kein Coach sein und keine festen Zeiten einhalten, es reicht, ansprechbar und freundlich zu sein."

Was nicht deine Aufgabe ist: "Du bist kein Support und kein Moderator. Bei Regelverstoessen, Streit oder Meldungen holst du das Team dazu, statt selbst einzugreifen. Und du gibst nie weiter, was dir dein Patenkind privat schreibt."

So uebernimmst du: "Wenn ein Neuling einen Paten moechte, erscheint hier im Kanal eine Karte mit einem Uebernehmen-Knopf. Wer zuerst drueckt, bekommt die Patenschaft. Der Bot legt danach einen privaten Kanal nur fuer euch beide an, dort geht es weiter."

Dein Limit: "Du kannst hoechstens drei Patenkinder gleichzeitig haben, damit fuer jeden genug Zeit bleibt. Ist ein Einstieg geschafft, wird ein Platz wieder frei."

Wenn du nicht weiterkommst: "Frag mich einfach mit einem @ hier im Kanal oder in eurem Patenkanal, ich ziehe die Antwort direkt aus unserer Wissensbasis. Bei allem, was Moderation braucht, wendest du dich ans Team."

Die wichtigsten Ecken (Kanal-Links, bekannte IDs eingesetzt, fehlende vom Orchestrator ergaenzen):
"Deadlock Router (Voice, verteilt automatisch in eine Lane): <#1513468587195633674>
Neue-Spieler-Lane: <#NEUE_SPIELER_LANE_ID>
Kostenloses Coaching: <#1494373349944459355>
Rang verknuepfen: <#RANG_VERKNUEPFUNG_ID>
Regelwerk: <#REGELWERK_ID>
Support und Tickets: <#1459628609705738539>
Mitspieler suchen: <#1522769149208821881>
Custom Games: <#CUSTOM_GAMES_ID>
Scrims: <#SCRIMS_ID>
Fragen an die Community: <#1426220702054355077>"

Eskalation: "Kommst du selbst nicht weiter oder braucht eine Situation einen Mod, meldest du dich beim Team, statt es allein zu loesen."

Datenschutz: "Was im Patenkanal oder in DMs besprochen wird, bleibt unter euch. Gib keine privaten Nachrichten weiter und poste nichts aus euren Gespraechen oeffentlich."

### Bewerbungsformular Pate (team_application_texts.toml, Abschnitt [pate])
intro: "Als Pate nimmst du neue Mitglieder an die Hand und zeigst ihnen unseren Server und die ersten Schritte im Spiel. Du brauchst keine festen Zeiten, nur Lust, ansprechbar und freundlich zu sein. Rechne mit ein paar Minuten hier und da, wenn ein Neuling Fragen hat."
frage_erfahrung: "Wie gut kennst du Deadlock und unseren Server schon?"
frage_verfuegbarkeit: "Wann bist du meistens da und wie schnell kannst du auf Fragen antworten?"

### Pate-Accepted-DM (status_dm_text, Accepted plus kind Pate)
"Schoen, dass du dabei bist. Ab jetzt bist du Pate in der Deutschen Deadlock Community. Wie alles laeuft, steht in deinem Leitfaden in der Paten-Zentrale: <#1524083665838276860>. Schau kurz rein, dann kann es losgehen."

### Willkommen-Abschnitt Paten (welcome_texts.toml)
Titel: "Paten"
Text: "Ein Pate ist jemand aus der Community, der dir den Einstieg leicht macht. Er zeigt dir den Server, beantwortet deine Fragen und dreht mit dir die ersten Runden. Waehlst du beim Start die Option, dass dich jemand an die Hand nimmt, bekommst du direkt einen Paten angeboten. Du kannst mir aber auch jederzeit per DM schreiben \"ich haette gern einen Paten\", dann kuemmere ich mich darum."

### Onboarding-Option 🌱 Beschreibung (per Discord-API vom Orchestrator gesetzt)
"Ich bin ganz neu und will's lernen, ein Pate aus der Community zeigt mir alles"

### 2-h-Hinweis (Kartenupdate in paten-zentrale, mit Owner-Ping)
"Seit zwei Stunden wartet <@USER_ID> noch auf einen Paten. <@662995601738170389>, magst du kurz schauen, ob jemand Zeit hat?"

### 24-h-Kartenmarkierung (unbesetzt geschlossen)
"Diese Anfrage haben wir nach 24 Stunden ohne Uebernahme geschlossen. Wir haben uns direkt bei der Person gemeldet."

### 24-h-DM an den Neuling (ehrlich, drei Kanaele)
"Hey, ich will ehrlich zu dir sein: gerade hat sich noch kein Pate fuer dich frei gemacht. Das liegt nicht an dir, manchmal ist einfach viel los. Damit du trotzdem sofort weiterkommst, hier die drei Ecken, wo dir direkt geholfen wird. In <#1426220702054355077> stellst du deine Fragen an die ganze Community, in <#1522769149208821881> findest du Mitspieler fuer eine Runde, und wenn du besser werden willst, melden sich in <#1494373349944459355> unsere Coaches bei dir. Und wenn du magst, schreib mir einfach nochmal, ich bleib dran."

### Uebernehmen nach geschlossener Anfrage (PATE_REQUEST_CLOSED_TEXT, ephemeral)
"Diese Patenanfrage wurde bereits geschlossen, weil sie 24 Stunden offen war. Die Person hat schon eine Nachricht von uns bekommen, hier ist gerade nichts mehr zu tun."

## Risiken und Stop-Regeln

- **Scope-Luecke (groesstes Risiko):** REQ-3 (Karte editieren), REQ-4 (Rolle vergeben) und REQ-7 (Willkommen-Abschnitt) brauchen `modglue.rs` und `serversync/welcome_publish.rs`, beide nicht im erlaubten Bereich. Ohne Freigabe-Datei oder Amendment blockt der Merge-Gate. Vor M4/M5/M8 klaeren.
- **Proaktiv an bedeutet echte DMs an echte neue Mitglieder.** `DL_CONCIERGE_PROACTIVE=1` schaltet ab Deploy die T0-Begruessung und die Kadenz scharf; Opt-out, Drei-Kontakte-Grenze und Privacy-Locks bleiben (INV-2), aber der erste Live-Lauf trifft reale Neulinge. Live erst nach Review deployen und die erste Stunde beobachten. Nie mit einem echten Streamer-Konto testen, nur mit Wegwerf-Zweitkonto (Memory `live-beweis-nur-wegwerf-konten`).
- **Timing der Frischling-Erkennung:** Der Concierge verarbeitet `NativeOnboardingCompleted` sofort, ohne die 7-Sekunden-Nachlese der journeyglue. Ist die Frischling-Rolle im Cache noch nicht sichtbar, faellt der User faelschlich auf Nicht-Frischling-T0 zurueck. Vor Deploy am Zweitkonto messen; wenn flaky, mit dem User eine kleine Lookup-Verzoegerung im Inner-Handler abstimmen, nicht heimlich einbauen.
- **Deploy erst nach Review.** Prod-Migration und `systemctl --user restart` laufen nicht im Implementierer- oder Plan-Agent; erst Review, dann Merge-Gate, dann Deploy, dann Live-Beweis. Der Aufruf "Wer hat Bock, Pate zu werden" laeuft separat ueber den Skill community-ankuendigung und nur mit Go des Users (Nicht-Ziel im Contract).

## Status

- M1 rote Regressionstests und Baseline: erledigt (8 Tests gruen, Rot-Gegenprobe in TESTS.md; Baseline zwei vorbestehende rote Tests festgehalten)
- M2 Migration und Store-Funktionen: erledigt (2026090918_concierge_pate_requests, 2026090919_team_applications_pate_kind, Store-Funktionen inline; USER_TABLES-Eintrag im Privacy-Vertrag)
- M3 Frischling-T0 (REQ-1): erledigt (Frischling-Lookup mit begrenztem Warten, t0_body_frischling, pate_offered plus Journey)
- M4 Anfrage-Persistenz und Eskalation (REQ-3): erledigt (insert in request_pate, claim-Guard fuer geschlossene Anfrage, run_pate_escalations 2h/24h idempotent)
- M5 Team-Bewerbung Pate (REQ-4): erledigt (ApplicationKind::Pate, add_role bei Annahme, Leitfaden-DM, Pate-Modal aus TOML)
- M6 Leitfaden-Poster und TOML (REQ-5): erledigt (ensure_pate_leitfaden mit Fingerprint-Edit statt Doppelpost, angepinnt; Kanal-IDs im TOML aufgeloest)
- M7 Mention-Wissensantwort (REQ-6): erledigt (bot_user_id-Mention-Gate in paten-zentrale und Patenkanaelen, allow_personal_actions false)
- M8 Willkommen-Hub und Onboarding-Optionstext (REQ-7): erledigt (Sektion paten in welcome_publish plus welcome_texts.toml; Optionstext setzt der Orchestrator per Discord-API)
- M9 Inventarzeile (REQ-8) und Startskript (REQ-2): teils erledigt (paten_inventory_line im Start-Log; DL_CONCIERGE_PROACTIVE=1 gehoert in das gitignorierte Live-Startskript, macht der Deployer)
- M10 Abschluss: erledigt (rustfmt der eigenen Dateien, clippy -D warnings fuer dl-community/dl-central-db/dl-bot rein, volle Testlaeufe gruen gegen die zwei Baseline-Rote plus ein lastbedingter coaching-Flake, sqlx prepare als No-op mangels query!-Makros, gate_hook --review ALLOW mit zwei behobenen Nits; Branch feat/paten-programm-live gepusht). Offen fuer den Deployer: Prod-Migration via dl-central-migrate, DL_CONCIERGE_PROACTIVE=1 im gitignorierten Live-Startskript, Release-Build, Restart, Live-Beweis, Onboarding-Optionstext per Discord-API.

## Status Fixer-Runde (Review-Maengel, 2026-09-09)

- Mangel A (INV-8, ae/oe/ue in neuen Nutzertexten): behoben. paten_leitfaden.toml (Werte, Schluessel unveraendert), welcome_texts.toml paten_intro, concierge.rs PATE_ESCALATION_24H_CARD_TEXT/PATE_UNBESETZT_DM_TEXT und Render-Label "So uebernimmst du", team_applications.rs PATE_ACCEPTED_DM_TEXT auf echte Umlaute. Startlog war bereits umlautrein. Keine Gedankenstriche in neuen Texten. Interne Log-Meldungen und Bezeichner bleiben.
- Mangel B (24h-Schliessung ohne Reset von pate_requested): behoben. claim_pate_escalation_stage TwentyFourHours laeuft jetzt als Transaktion, RETURNING user_id, und setzt concierge_profiles.pate_requested=FALSE und pate_request_uncertain=FALSE in derselben tx. Regressionstest reaktiver_wunsch_nach_24h_schliessung_erzeugt_neue_anfrage (rot vor Fix, gruen danach).
- Mangel C (Onboarding-Option 🌱): serversync.rs Beschreibung auf REQ-7-Wortlaut ohne Gedankenstrich, Titel unveraendert. Kein Snapshot-Test haengt am Beschreibungstext. Server-Sync wendet die Onboarding-Konfiguration NICHT beim Start an, nur auf Befehl/HTTP mit confirm (siehe Abgabe).
- Sollte D (T2 trotz pate_offered): cadence_due plant T2 nicht mehr bei pate_offered=TRUE, T7 unveraendert. Neuer Test kadenz_t2_entfaellt_wenn_pate_schon_angeboten_wurde.
- Sollte E (stille Seiteneffekte): 24h-DM wertet ConciergeDmDelivery aus und loggt Fehlerfaelle mit user_id und Stufe; 2h- und 24h-Kartenwarnung um user_id und Stufe ergaenzt. Kein Retry, keine Markerruecknahme.
- Mangel F (DL_CONCIERGE_PROACTIVE im Startskript): keine committete Vorlage/Doku des gitignorierten scripts/run_dl_bot_service.sh im Repo (scripts/* ist gitignored, nur einzelne run_*.sh per Allowlist; run_dl_bot_service.sh fehlt ganz). Bleibt Deployer-Schritt.
