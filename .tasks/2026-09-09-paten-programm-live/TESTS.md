# TESTS: Paten-Programm live

Nachweis der roten Regressionstests (REQ-9) und der vorbestehenden roten Baseline.
Toolchain: rustc/cargo 1.97.1 aus ~/.rustup/toolchains, Testbefehl ueber
`rust/scripts/central_test_db.sh` (Wegwerf-Timescale-Container), `--features testing`,
`SQLX_OFFLINE=1`.

## Vorbestehende rote Baseline (ohne meine Aenderungen, sauberer origin/main)

Nachgemessen am 2026-09-09 ohne die neuen Migrationen (deterministisch, 2,58 s):

- `dl-community` `concierge::tests::globaler_optout_antwortet_stateless_ohne_neue_daten_oder_cooldown`: FAILED (Grund: `assertion failed: sent.iter().all(|body| sent_v2_content(body) == SMALLTALK_TEXT)`, concierge.rs:11723; vorbestehend, kein Bezug zum Paten-Programm)
- `dl-community` `reaction_roles::tests::scrim_pool_upsert_fehler_bleibt_fail_open`: FAILED (Grund: Test-DB-Teardown `cannot drop table scrim.participants because other objects depend on it` (2BP01); vorbestehend, kein Bezug zum Paten-Programm)

Voller Baseline-Lauf `dl-community --features testing` (mit versehentlich schon vorhandenen neuen Migrationen): `test result: FAILED. 450 passed; 3 failed`. Die dritte Fehlermeldung `privacy::privacy_contract_tests::alle_migration_user_id_spalten_sind_im_privacy_vertrag` war durch die neue Migration `concierge_pate_requests` ausgeloest (user_id-Spalte noch nicht im Privacy-Vertrag) und wird von M2 mit dem USER_TABLES-Eintrag geschlossen, also kein echter Baseline-Fehler.

Baseline dl-central-db und dl-bot: siehe Abschnitt M10.

## REQ-9 Tests

Implementierung und Tests entstanden in einer Sitzung. Die Rot-vor-der-Aenderung-Eigenschaft
je Test ist per gezielter Gegenprobe (Sabotage der jeweiligen Produktionsstelle, Rolle-Test-Waechter)
nachgewiesen: mit Fachcode gruen, ohne die jeweilige Stelle rot.

Gruener Lauf aller acht Tests (`dl-community --features testing`, Filter je Testname):
`test result: ok. 8 passed; 0 failed` (10,96 s).

Neue Tests:
- `frischling_t0_enthaelt_paten_angebot_und_setzt_pate_offered` (REQ-1)
- `frischling_t0_wartet_bis_die_rolle_beim_dritten_lookup_sichtbar_ist` (REQ-1, Timing/Retry)
- `pate_anfrage_eskaliert_nach_2h_genau_einmal` (REQ-3)
- `pate_anfrage_schliesst_nach_24h_genau_einmal_mit_dm` (REQ-3)
- `pate_eskalation_feuert_nach_neustart_nicht_erneut` (REQ-3, Idempotenz)
- `uebernehmen_nach_geschlossener_anfrage_bleibt_wirkungslos` (REQ-3)
- `angenommene_paten_bewerbung_vergibt_paten_rolle_und_schickt_leitfaden_dm` (REQ-4)
- `pate_bewerbungsformular_stellt_erfahrung_und_verfuegbarkeit` (REQ-4, reiner Unit-Test)

### Gegenprobe A (T0 ohne Frischling-Body, Eskalation als No-op, Closed-Guard entfernt, add_role entfernt)

`test result: FAILED. 0 passed; 6 failed` (6,51 s):
- `frischling_t0_enthaelt_paten_angebot_und_setzt_pate_offered` FAILED (concierge.rs:15037)
- `frischling_t0_wartet_bis_die_rolle_beim_dritten_lookup_sichtbar_ist` FAILED (concierge.rs:15084)
- `pate_anfrage_eskaliert_nach_2h_genau_einmal` FAILED (concierge.rs:15107)
- `pate_anfrage_schliesst_nach_24h_genau_einmal_mit_dm` FAILED (concierge.rs:15136)
- `uebernehmen_nach_geschlossener_anfrage_bleibt_wirkungslos` FAILED (concierge.rs:15204)
- `angenommene_paten_bewerbung_vergibt_paten_rolle_und_schickt_leitfaden_dm` FAILED (team_applications.rs:1923)

### Gegenprobe B (Eskalations-Idempotenz defekt: Marker-Filter, is_none-Guard und Claim-Stufe umgangen)

`test result: FAILED. 0 passed; 1 failed` (2,99 s):
- `pate_eskalation_feuert_nach_neustart_nicht_erneut` FAILED (concierge.rs:15182)

Nach beiden Gegenproben wurde der Arbeitsstand per `git checkout` wiederhergestellt.

## M10 Abschluss

- rustfmt: `rustfmt --check` fuer alle beruehrten Dateien sauber (origin/main war bei concierge.rs und team_applications.rs formatierungsrein, nur meine Zeilen wurden formatiert; `cargo fmt --all` ist per Guardrail gesperrt wegen roter Formatter-Baselines anderer Crates).
- clippy `-D warnings`: dl-community (mit `--features testing`), dl-central-db und dl-bot je `--all-targets` ohne Warnungen. Zwei eigene Funde behoben: `clippy::type_complexity` im Mock-Port (Type-Alias) und `clippy::unwrap_used` in den Team-Bewerbungstests (`expect` statt `unwrap`, kein `#[allow]`).
- Volle Testlaeufe:
  - `dl-community --features testing`: `458 passed; 3 failed` (237,67 s). Die drei roten sind die zwei vorbestehenden Baseline-Tests (`globaler_optout_antwortet_stateless_ohne_neue_daten_oder_cooldown`, `reaction_roles::tests::scrim_pool_upsert_fehler_bleibt_fail_open`) plus `coaching_requests::pg_tests::complete_button_ackt_bevor_abschlussarbeit_haengt`, ein last-abhaengiger Timing-Flake (im ersten vollen Lauf gruen, im gleichzeitig gebauten Re-Run rot). Alle zehn neuen Paten-Tests gruen, auch die zuvor kurz regressierten `laufendes_t0_beendet_sich_vor_forget...` und `t0_haelt_privacy_lock_bis_nach_dm...`.
  - `dl-central-db`: alle gruen (Integrationstests mit `#[ignore]` laufen nur in CI-Phase D mit echter DSN).
  - `dl-bot`: `269 passed; 0 failed`.
- sqlx prepare: `cargo-sqlx` ist auf dem Host nicht installiert. Der Diff enthaelt keine compile-time-geprueften `query!`/`query_as!`-Makros (nur Laufzeit-`sqlx::query`), daher aendern sich keine Dateien unter `rust/.sqlx/`; prepare ist ein No-op.
- gate_hook `--review` (`--repo <worktree> --base origin/main`, Standardmodell gpt-6-astra): Ergebnis ALLOW ("No repository-verified blockers"), der Review lief nur teilweise, weil der Repo-Tool-Host abgeschaltet war. Drei unverifizierte NITs, davon zwei behoben:
  - NIT close/claim-Race und "offen steckenbleiben": die 24h-Stufe setzt `escalated_24h_at`, `status='closed_unbesetzt'` und `closed_at` jetzt in einem atomaren bedingten UPDATE (`claim_pate_escalation_stage`), die separate `close_pate_request_unbesetzt` entfaellt. Damit gibt es keinen offenen Zwischenzustand mehr, und Claim gegen Schliessung schliessen sich am Status-Spaltenlock gegenseitig aus.
  - NIT DM ohne Rolle: schlaegt `add_role` bei Annahme einer Paten-Bewerbung fehl, bricht `change_status` mit klarer Mod-Rueckmeldung ab, bevor die Willkommens-DM rausgeht; erneutes Annehmen wiederholt idempotent (add_role ist idempotent, DM noch nicht versendet).
  - NIT Marker vor Seiteneffekt (Kartenedit/DM): bewusst beibehalten. Der Marker sichert "je Stufe hoechstens einmal" auch ueber Neustarts; ein Retry wuerde Doppel-DMs riskieren, was der Nutzerpraeferenz "eine Meldung, keine Wiederholung" widerspricht. Karten-Edit und DM sind best-effort nach dem atomaren Statuswechsel.

Nach den Review-Fixes: clippy dl-community `-D warnings` erneut RC=0. Re-Lauf der von den Fixes beruehrten Tests (Eskalation 2h/24h, Neustart-Idempotenz, Uebernahme nach Schliessung, Paten-Bewerbung, beide Frischling-T0-Tests): `test result: ok. 7 passed; 0 failed` (10,19 s). Die Review-Fixes beruehren nur neue Codepfade (Eskalation, Paten-Annahme), keine bestehenden Tests; der volle dl-community-Lauf mit 458 gruenen lief auf dem Stand vor diesen zwei Fix-Commits.


## Fixer-Runde (Review-Maengel A bis F, 2026-09-09)

Toolchain und Testweg unveraendert (rustc/cargo 1.97.1, central_test_db.sh, --features testing, SQLX_OFFLINE=1).

### Neue Regressionstests und Rot-Gegenproben

- reaktiver_wunsch_nach_24h_schliessung_erzeugt_neue_anfrage (Mangel B, REQ-3): vor dem Fix rot.
  Rot-Lauf (ohne Reset von pate_requested bei 24h-Schliessung): test result FAILED, 0 passed, 1 failed (3,03 s),
  assertion left == right failed, left []  right [1524083665838276860] (keine neue Karte, weil der reaktive Wunsch am pate_requested=TRUE-Guard haengen bleibt).
  Nach dem Fix (24h-Schliessung setzt in derselben Transaktion pate_requested=FALSE, pate_request_uncertain=FALSE): gruen.
- kadenz_t2_entfaellt_wenn_pate_schon_angeboten_wurde (Sollte 6): Rot-Gegenprobe per Sabotage.
  Ohne "&& !profile.pate_offered" in cadence_due: test result FAILED, 0 passed, 1 failed (Panik bei !actions.contains(T2)), T7 bleibt erhalten.
  Mit der Bedingung: gruen. Bestehender Test kadenz_t2_nur_bei_null_aktivitaet_und_max_drei_kontakte weiter gruen (Default pate_offered=false).

### Gruene Laeufe

- Eskalations- und Reaktivpfad zusammen (reaktiver_wunsch pate_anfrage pate_eskalation uebernehmen_nach wiederholter_patenwunsch): test result ok, 6 passed, 0 failed (10,77 s).
- Kadenz (kadenz gratulation optout_stoppt): test result ok, 12 passed, 0 failed (12,98 s).
- Voller dl-community --features testing: test result FAILED, 461 passed, 2 failed (236,17 s). Die zwei roten sind exakt die vorbestehende Baseline (globaler_optout_antwortet_stateless_ohne_neue_daten_oder_cooldown, reaction_roles::tests::scrim_pool_upsert_fehler_bleibt_fail_open); der fruehere coaching-Flake trat diesmal nicht auf. Vorher 458/459 gruen, jetzt 461 gruen (die zwei neuen Tests).
- clippy -D warnings fuer dl-community (--features testing) und dl-bot je --all-targets: RC=0, keine Warnungen.

TESTNACHWEIS[TW-1]: 461 passed, 0 ignored | Rot-Gegenprobe: B 1 failed statt 0 (ohne Reset), D 1 failed statt 0 (ohne pate_offered-Gate)

## Fixer-Runde 2 (Merge-Kritiker-Hinweise 1 bis 4, 2026-09-10)

Toolchain und Testweg unveraendert (rustc/cargo 1.97.1, central_test_db.sh, --features testing, SQLX_OFFLINE=1, CARGO_INCREMENTAL=0).

Umsetzung je Hinweis:
- Hinweis 1 (team_applications.rs): ModalSpec traegt nur Text-Eingabefelder (ModalField), kein type:10-Display; das Modalformat des Repos kann den Intro-Text nicht rendern. Deshalb Vorschalt-Nachricht (ephemere Components V2 mit Knopf "Weiter zum Formular", custom_id team_apply:form:pate); der Formular-Zweig oeffnet danach das bestehende Modal. Andere Bereiche unveraendert (open liefert weiter direkt das Modal).
- Hinweise 2 und 3 (concierge.rs claim_pate): Uebernahme nur noch atomar ueber die Nachrichten-ID der geklickten Karte (claim_open_pate_request_by_message_tx: UPDATE ... WHERE message_id = $2 AND user_id = $1 AND status = 'open' RETURNING id) und zwar vor der Kanalanlage; null Zeilen bricht mit Rollback und PATE_REQUEST_CLOSED_TEXT ab (keine Patenschaft, kein Kanal). Der Vor-Transaktions-Check latest_pate_request_status wurde entfernt (samt Store-Methode).
- Hinweis 4 (ensure_pate_leitfaden): Fingerprint wird erst nach erfolgreichem Pin gespeichert (message_id vorher, damit kein Doppelpost); beim naechsten Start ohne Fingerprint wird die vorhandene Nachricht editiert und erneut gepinnt.

Bestandsanpassung: valid_pate_claim_interaction traegt jetzt eine Karten-Nachrichten-ID (PATE_CLAIM_TEST_MESSAGE_ID = 900); die neun Erfolgspfad-Claim-Tests seeden dazu eine offene Anfrage (seed_open_pate_request). Die REQ-9-Tests bleiben unveraendert.

Befund: Der partielle Unique-Index concierge_pate_requests_one_open_per_user erlaubt nur eine offene Anfrage je Nutzer, ein zweiter offener Datensatz ist nicht seedbar. Der rein sequentielle Fall "Anfrage vor dem Claim geschlossen" faengt der alte Vor-Transaktions-Check bereits ab (gruen), ist also nicht rot-vor-dem-Fix. Der Hinweis-3-Test klickt darum eine Karte, deren Anfrage bereits als 'claimed' vergeben ist (ein Status, den der alte Check nicht abfing), und weist so die geerbte 0-Zeilen-Toleranz nach.

Neue Regressionstests:
- pate_vorschalt_zeigt_intro_und_knopf_zum_formular (Hinweis 1, reiner Unit-Test)
- uebernehmen_uebernimmt_nur_die_angeklickte_offene_anfrage (Hinweis 2)
- claim_gegen_direkt_uebernommene_anfrage_erzeugt_keine_zweite_patenschaft (Hinweis 3)
- leitfaden_pin_fehler_wird_beim_naechsten_start_nachgeholt (Hinweis 4)

Rot-Gegenproben (Sabotage der jeweiligen Produktionsstelle, danach wiederhergestellt):
- Hinweise 2 und 3: claim_open_pate_request_by_message_tx auf altes Verhalten (WHERE user_id ... status='open', Ok(true) statt RETURNING) -> test result FAILED, 0 passed, 2 failed; beide neuen Claim-Tests bekommen reply None (Claim laeuft durch) statt PATE_REQUEST_CLOSED_TEXT.
- Hinweis 1: Intro-Text aus pate_intro_reply entfernt -> pate_vorschalt_zeigt_intro_und_knopf_zum_formular FAILED (raw.contains(intro) false).
- Hinweis 4: Fingerprint vor dem Pin gespeichert -> leitfaden_pin_fehler_wird_beim_naechsten_start_nachgeholt FAILED (stored_fingerprint.is_none() false, kein Re-Pin beim zweiten Start).

Gruene Laeufe:
- Vier neue Tests einzeln: test result ok, 4 passed, 0 failed (3,40 s).
- Voller dl-community --features testing: test result FAILED, 465 passed, 2 failed (167,75 s). Die zwei roten sind exakt die vorbestehende Baseline (globaler_optout_antwortet_stateless_ohne_neue_daten_oder_cooldown, reaction_roles::tests::scrim_pool_upsert_fehler_bleibt_fail_open); 461 + 4 neue = 465.
- dl-bot (central_test_db.sh): test result ok, 269 passed, 0 failed (56,30 s); der seed_scrim-Bin hat 0 Tests und ist ok (ein zwischenzeitliches "No such file or directory" auf target/debug/deps war ein Umgebungs-Flake beim Wegraeumen der Testbinaries, kein Testfehler, im Einzellauf ok).
- rustfmt --edition 2021 --check fuer concierge.rs und team_applications.rs sauber (cargo fmt --all ist per Guardrail gesperrt).
- clippy -D warnings: dl-community --features testing --all-targets und dl-bot --all-targets je RC=0, keine Warnungen.
