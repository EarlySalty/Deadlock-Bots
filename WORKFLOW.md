# W3.4b W3 - Live-Kopplung Post-Lane + Join + Auto-Close (2026-07-03)

## Fortschritt
- Pflichtkontext W3.4b Phase 1 sowie W2/W2-Rework inkl. stale-`creating`-Hinweis gelesen.
- Neue additive Migration `2026070335_lfg_post_ids.sql`: `voice.lfg_posts.id` als stabile Post-ID fuer `lfg:open_lane:<id>`, `lfg:join:<id>` und race-sichere `UPDATE ... WHERE id = $1`.
- Weg A umgesetzt: Formular-Erfolg antwortet ephemer mit `lfg:open_lane:<post_id>`; Klick nutzt Router-Spawn-Semantik ueber `LaneRouter`, verknuepft `lane_id` nur bei `lane_id IS NULL` und queued ein Render-Update.
- Weg B umgesetzt: TempVoice-Panel registriert `lfg:publish_lane`; Owner-Guard bleibt `owned_lane_of`, Modus wird aus Lane-Kategorie/TempVoice-DB abgeleitet, Modal fragt nur Rangbereich/Platzanzahl, Post wird direkt mit `lane_id` reserviert.
- LFG-Starter-Message bekommt `lfg:join:<post_id>`; Join prueft Mapping, Lane-Existenz/Belegung, Ranked-Rolle, Voice-Connection, eigener Post und moved targeted ueber Port.
- Live-Render umgesetzt: freie Plaetze aus Cache-Belegung/User-Limit, `user_limit=0` mit Modus-Default, Street-Brawl 4; Render-Hash verhindert No-op-Edits.
- Edit-Queue umgesetzt: prozesslokal last-wins, ein Worker, Mindestabstand pro Post, 429-Fallback-Backoff; nutzt `last_render_hash`/`last_post_edit_at`.
- Auto-Close umgesetzt: TempVoice-`cleanup_lane` informiert LFG per Weak-Sink; Reconcile-Ticker schliesst tote Lane-Posts, expired lane-lose Posts und loescht stale `creating` aelter 5 Minuten. Worker/Subscriptions laufen nur bei `lfg_cutover_active`.
- Platzhalter-Texte bleiben bewusst Platzhalter: LFG-Buttons/Modal/Replies/Render verwenden `Platzhalter` bzw. `Platzhalter-voll/offen`.

## Verifikation aktuell
- Gruen: `SQLX_OFFLINE=true cargo check --workspace --all-targets`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice lfg_panel -- --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo build --workspace`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-community`
- Gruen: `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`
- Gruen: `cargo fmt --all -- --check`

## Rest-Risiken
- Ranked-TempVoice-Panel hat bereits das Discord-Limit von 5 Action-Rows; `lfg:publish_lane` belegt dort einen bestehenden Button-Slot. Bestehender `tv_preset_load`-Handler bleibt registriert, hat im Ranked-Panel aber keinen sichtbaren Button mehr.

# W3.4b W3 Kritiker (2026-07-03)

Scope: adversarialer Review des uncommitted W3-Diffs auf Branch `feat/welle34b-lfg`. Keine Source-Aenderungen ausser diesem Report, kein Commit/Push.

## Befunde

### DEPLOY-BREAKER
1. Flag-aus-Invariante ist verletzt: TempVoice zeigt und verdrahtet LFG-Publish trotz `lfg_cutover_active=false`.
   - Datei/Zeilen: `rust/crates/dl-voice/src/tempvoice/interface.rs:262-274`, `:334-397`, `:353-363`, `:387-395`, `:1091-1099`; Wiring `rust/bin/dl-bot/src/main.rs:507`, `:511-516`, Ready-Refresh `:1251`; Lane-Sink `rust/crates/dl-voice/src/tempvoice/engine.rs:1659-1662`; Sink-Ziel ohne Cutover-Guard `rust/crates/dl-voice/src/lfg_panel.rs:1757-1767`.
   - Szenario: `DL_LFG_FORUM_CUTOVER=false`, aber TempVoice-Panels werden beim Gateway-Ready weiter refreshed. `main_view_components()` hat keinen Cutover-Parameter und rendert statisch `lfg:publish_lane`; im Ranked-Panel ist dadurch der alte sichtbare `tv_preset_load`-Button weg. Klicks laufen durch den neuen TempVoice-Handler bis Owner-Check/LFG-Placeholder. Zusaetzlich ist `tempvoice.set_lfg_panel(...)` immer gesetzt und `cleanup_lane()` ruft `on_lane_deleted()` ohne Cutover-Guard auf, also macht ein normaler Lane-Cleanup unter Flag-aus neue LFG-DB/Discord-Close-Arbeit.
   - Fix-Skizze: Cutover-Flag in `TempVoiceInterface`/Panel-Rendering durchreichen; bei Flag-aus exakt alte Components rendern und `lfg:publish_lane` nicht registrieren/anzeigen. `set_lfg_panel` nur bei aktivem Cutover setzen oder `on_lane_deleted()` hart auf `cutover_active` gaten. Regressionstest: Flag-aus-Panel enthaelt `tv_preset_load` und kein `lfg:publish_lane`.

### HIGH
2. Ranked-Panel-Slot-Verdraengung bleibt unter Cutover ein Feature-Verlust und braucht Owner-Entscheid.
   - Datei/Zeilen: `rust/crates/dl-voice/src/tempvoice/interface.rs:337-365`, Handler fuer verdrängtes Feature `:865-896`, Registrierung weiter vorhanden `:1307-1310`; Diff zeigt Ersatz von `button("📂 Preset laden", ..., "tv_preset_load")` durch `LFG_PUBLISH_LANE_CUSTOM_ID`.
   - Szenario: Ranked hat 5 Action-Rows: Row 1 = 5 Buttons, Row 2 = 5 Buttons, Row 3 = Select, Row 4 = 5 Buttons, Row 5 = Select. Discord erlaubt 5 Rows und Selects nicht gemischt mit Buttons; damit gibt es im Ranked-Panel keinen freien Button-Slot. Non-Ranked hat freie Plaetze (`interface.rs:367-396`) und kann LFG ohne Verlust aufnehmen, Ranked nicht. Aktuell wird Preset-Laden still entfernt.
   - Fix-Skizze: Nicht als akzeptiertes Risiko mergen. Owner muss entscheiden: eigene Unteransicht/zweite Panel-Message, bestehende Ranked-Funktion bewusst entfernen, oder UI neu gruppieren. Bis dahin Ranked bei Cutover nicht mit LFG-Button rendern oder Preset-Laden priorisieren.

3. `lfg:open_lane:<post_id>` raeumt nach erfolgreicher Lane-Erstellung den Fehlerpfad nicht auf.
   - Datei/Zeilen: `rust/crates/dl-voice/src/lfg_panel.rs:1512-1536`, race-sicheres DB-Update `:1318-1333`; Router-Spawn erstellt und moved vorher `rust/crates/dl-voice/src/router.rs:840-847`, TempVoice erstellt Kanal/DB vor Rueckgabe `rust/crates/dl-voice/src/tempvoice/engine.rs:796-849`.
   - Szenario: In einem Prozess blockt `TempVoiceEngine` schnelle Doppel-Erstellung weitgehend ueber `state.creating` (`engine.rs:743-752`). Trotzdem: wenn `spawn_lane_from_current_voice()` `Created { lane_id }` liefert und danach `link_post_lane()` wegen DB-Ausfall, bereits geschlossenem/gelinktem Post oder anderer Race `Err`/`false` ergibt, bleibt die neue Voice-Lane bestehen, aber der LFG-Post bleibt lane-los. Der Handler antwortet nur ephemeral `Platzhalter`; kein `cleanup_lane()`, keine alternative Verknuepfung, keine Owner-Anleitung.
   - Fix-Skizze: Nach `Created` bei `link_post_lane=false/Err` die frisch erstellte Lane kontrolliert loeschen oder eine DB-Reservation/Compare-and-swap vor der Lane-Erstellung einfuehren. Test fuer `link_post_lane` rows_affected=0 und DB-Err mit Cleanup-Mock.

4. Close/Reconcile kann Posts dauerhaft `open` lassen, wenn Thread-Archive/Lock scheitert.
   - Datei/Zeilen: `rust/crates/dl-voice/src/lfg_panel.rs:1731-1754`, Aufrufer `:1757-1764`, `:1786-1823`.
   - Szenario: `close_post()` ruft erst `archive_and_lock_thread(thread_id).await?` auf und schreibt den DB-Status erst danach. Bei Discord 404 (Thread manuell geloescht), 403 (fehlende Rechte) oder transientem Fehler bleibt `status='open'`. Wegen `lfg_posts_owner_active_uidx` blockiert das den Owner weiter; Reconcile/Lane-Delete wiederholen denselben Fehler endlos.
   - Fix-Skizze: Discord-Close als best-effort behandeln: 404 als geschlossen werten, DB-Status in jedem Fall kontrolliert auf `closed/expired` setzen und Fehler separat loggen/metricen. Fuer 403 Owner-Alarm statt Owner dauerhaft zu blockieren.

### MITTEL
5. LFG-Edit-Queue ist nur prozesslokal und wird nach Restart nicht neu aufgebaut.
   - Datei/Zeilen: Queue-State `rust/crates/dl-voice/src/lfg_panel.rs:256-260`, Enqueue/Worker `:1594-1697`, Spawn `:1878-1905`.
   - Szenario: Join/Leave/Move enqueue'n nur in einem RAM-`HashSet`. Restart zwischen Move und Edit verliert Pending-Updates; beim Start werden `status='open' AND lane_id IS NOT NULL` nicht initial in die Queue gelegt. Der Post bleibt mit alter Slot-Anzeige, bis ein neues VoiceEvent/Join/Reconcile ihn zufaellig wieder beruehrt.
   - Fix-Skizze: Beim Worker-Start alle offenen lane-gekoppelten Posts enqueuen oder Reconcile fuer lebende Lane-Posts ebenfalls rendern lassen. Optional persistente Queue analog `rename_queue`. 429-Backoff nutzt aktuell nur festen Fallback aus `glue.rs:2093-2101`; echten Retry-After nutzen, falls Serenity ihn liefert.

6. Join-Validierung ist nicht vollstaendig und Fehlertexte sind live nicht verstaendlich.
   - Datei/Zeilen: `rust/crates/dl-voice/src/lfg_panel.rs:1550-1591`, Placeholder-Konstante `:31`.
   - Szenario: `lfg:join` prueft Lane-Mapping, Own-Post, Occupancy, Ranked-Rolle, irgendeine Voice-Connection und faengt Move-Fehler. Es prueft aber nicht `member_voice_channel == lane_id`; ein User in der Ziel-Lane loest einen redundanten Move auf denselben Channel aus. Bei parallel klickenden Usern wird die Kapazitaet vor dem Move aus Cache gelesen; ein gerade joinender User ist bis zum Cache-Event nicht gezaehlt. Alle Fehlerfaelle antworten zwar ephemeral, aber nur mit `Platzhalter`, also nicht verstaendlich fuer Live-User.
   - Fix-Skizze: Vor Move `current_channel == lane_id` als eigenen Erfolg/Fehler behandeln; fuer knappe Slots pro Lane kurz serialisieren oder nach Move validieren/rendern. Konkrete ephemere Texte fuer Lane tot, voll, Rank fehlt, nicht in Voice, eigener Post, schon drin, Move fehlgeschlagen.

7. Testabdeckung gruenschaetzt kritische W3-Pfade.
   - Datei/Zeilen: LFG-Tests `rust/crates/dl-voice/src/lfg_panel.rs:2148-2174`, `:2613-2651`, `:2654-2693`, `:2807-2876`; TempVoice-Panel-Test `rust/crates/dl-voice/src/tempvoice/interface.rs:1398-1450`.
   - Szenario: Die gezielten `lfg_panel`-Tests laufen gruen, decken aber nur schmale Happy-/Einzelfehlerpfade. Es fehlt ein Flag-aus-TempVoice-Panel-Test, der `tv_preset_load` erhaelt und `lfg:publish_lane` ausschliesst. `lfg_open_lane_verknuepft_created_lane_race_sicher` testet nur einen einzelnen erfolgreichen Klick, nicht Doppelklick/`rows_affected=0`/Cleanup. `lfg_join_moved_targeted...` testet nur Happy Path, nicht volle Lane, schon in Ziel-Lane, nicht in Voice, eigener Post, Ranked ohne Rolle oder Move-403/404. Reconcile-Test deckt nicht Archive-Fehler, Ticker-vs-Sink-Doppelclose oder 5-Minuten-Grenzrace ab.
   - Fix-Skizze: Tests vor Merge nachziehen; insbesondere Regressionstests fuer die beiden Deploy-Breaker und Cleanup-Fehlerpfade.

## Pflicht-Linsen
- 1. Flag-aus-Invariante: nicht sauber. Alter LFG-Responder bleibt bei inaktivem Cutover korrekt aktiv (`main.rs:1158-1169`), LFG-VoiceEvent-Worker/Ticker starten nur bei Cutover (`main.rs:1173-1175`, `lfg_panel.rs:1878-1905`), Ready-Panel-Ensure ist gegated (`main.rs:1253-1254`), `LfgPanelInterface::handle` blockt direkt bei Flag-aus (`lfg_panel.rs:1833-1838`). Aber TempVoice-Panel-Button und Lane-Delete-Sink sind ungated, siehe DEPLOY-BREAKER.
- 2. Panel-Slot-Verdraengung: nicht sauber. Inventar siehe HIGH #2. Non-Ranked hat freie Button-Slots; Ranked ist ohne UX-Entscheid hart voll.
- 3. Race Lane-Verknuepfung: teilweise sauber. Owner-Check vorhanden (`lfg_panel.rs:1513-1517`), `UPDATE ... WHERE lane_id IS NULL` ist DB-seitig race-sicher (`:1318-1333`), `lane_id` unique violation beim Publish wird als `AlreadyOpen` ephemer abgefangen (`:931-935`, `:1452-1471`). Nicht sauber ist Cleanup nach erfolgreicher Lane-Erstellung und fehlgeschlagenem Link, siehe HIGH #3.
- 4. Edit-Queue/Rate-Limits: teilweise sauber. Last-wins pro Post per `HashSet` (`lfg_panel.rs:1594-1603`), Mindestabstand (`:1677-1685`), Render-Hash-No-op (`:1627-1631`) und 429-Requeue (`:1686-1692`) existieren. Restart-Rebuild fehlt, siehe MITTEL #5.
- 5. Join-Validierungen: teilweise sauber. Lane tot/Occupancy None schliesst Post best-effort (`lfg_panel.rs:1560-1563`), voll wird vor Move blockiert (`:1564-1567`), Ranked-Gate (`:1568-1573`), User-in-Voice (`:1574-1581`), eigener Post (`:1554-1556`) und Move-Fehler (`:1582-1589`) sind vorhanden. Schon-in-Ziel-Lane und verstaendliche Fehlertexte fehlen, siehe MITTEL #6.
- 6. Reconcile-Ticker: teilweise sauber. 60s-Tick wird nur im Cutover-Spawn gestartet (`lfg_panel.rs:1878-1905`), stale `creating` nutzt DB-Zeit und 5-Minuten-Schwelle (`:1774-1784`), expired nur lane-los (`:1809-1823`). Nicht sauber: Archive-Fehler verhindern DB-Close, siehe HIGH #4. Ticker-vs-Sink-Doppelclose ist durch `status='open'`-Update weitgehend idempotent, kann aber doppelt archivieren.
- 7. Ported but never wired: im Kern verdrahtet. VoiceEvent-Subscription real in `lfg_panel::spawn` (`lfg_panel.rs:1882-1894`) und Main nur bei Cutover (`main.rs:1173-1175`), Worker/Ticker real gespawnt (`lfg_panel.rs:1896-1904`), Join-Button wird an Starter-Message gebaut (`glue.rs:1756-1763`), Handler registriert (`lfg_panel.rs:1871-1875`), TempVoice-Publish registriert (`interface.rs:1326`). Problem ist Gating, nicht fehlendes Wiring.
- 8. Migration 2026070335: sauber. Neue untracked Migration aendert nur `voice.lfg_posts` additiv (`2026070335_lfg_post_ids.sql:1-36`); keine committete Migration wurde editiert. Fresh-Schema-Vertrag enthaelt `id`/Default (`fresh_migrations_schema.rs:1719-1749`) und der Fresh-Migration-Test ist gruen. Reihenfolge ist live-tauglich, weil `2026070320_lfg_posts.sql` die Tabelle vorher anlegt.
- 9. Serenity-API-Realitaet: sauber. Lokale Serenity 0.12.5 enthaelt `EditThread::archived/locked/audit_log_reason`, `Http::edit_member`, `CreateMessage::components`, `EditMessage`; `SQLX_OFFLINE=true cargo check -p dl-voice -p dl-bot -p dl-central-db` ist gruen.
- 10. Testluecken/Gruen-Waschen: nicht sauber, siehe MITTEL #7. Die behaupteten gruenen Tests beweisen Compile und mehrere Happy Paths, aber nicht die Deploy-Breaker-/Race-/Restart-Faelle.

## Kritiker-Verifikation
- Gruen: `SQLX_OFFLINE=true cargo check -p dl-voice -p dl-bot -p dl-central-db`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice lfg_panel -- --nocapture` (22 passed; TestDb-Pool-Close-Timeouts nur Cleanup-Warnungen)
- Gruen: `git diff --check`

## Gesamturteil
REWORK-NOETIG. Nicht merge-/deployfaehig, weil die Flag-aus-Invariante erneut verletzt ist und Ranked-User schon vor Cutover den sichtbaren `Preset laden`-Button verlieren. Zusaetzlich muessen Lane-Link-Cleanup, Close-Fehlerpfad und die fehlenden Regressionstests vor Live-Deploy nachgezogen werden.

# W3.4b W3 Rework (2026-07-03)

## Fix-Status
- Fix 1 DEPLOY-BREAKER: Behoben. `lfg_cutover_active` wird in `TempVoiceInterface` durchgereicht (`rust/bin/dl-bot/src/main.rs:459-517`, `rust/crates/dl-voice/src/tempvoice/interface.rs:58-90`). Flag-aus nutzt das alte TempVoice-Layout mit `tv_preset_load` und ohne `lfg:publish_lane`; LFG-Publish wird nur bei aktivem Cutover registriert (`interface.rs:369-420`, `:1367-1391`). Lane-Delete-LFG-Sink ist doppelt gegatet: `set_lfg_panel` nur bei Cutover (`main.rs:508-509`) und `on_lane_deleted` returnt bei inaktivem Cutover (`lfg_panel.rs:1882-1885`). Tests: `flag_aus_ranked_panel_behaelt_preset_load_und_ohne_lfg_publish`, `flag_aus_lane_delete_sink_macht_keine_lfg_arbeit`.
- Fix 2 RANKED-PANEL-SLOT: Behoben gemaess Owner-Entscheid. Ranked-Cutover ersetzt `tv_preset_save`/`tv_preset_load` durch `tv_presets` Style 2 und setzt `lfg:publish_lane` in den freien Slot (`interface.rs:369-386`). Das Untermenue antwortet ephemer mit Buttons auf die alten Custom-IDs `tv_preset_save`/`tv_preset_load` (`interface.rs:836-844`). Non-Ranked bekommt LFG im freien letzten Row-Slot (`interface.rs:412-420`). Tests: `cutover_ranked_panel_buendelt_presets_und_zeigt_lfg_publish`, `presets_sammelbutton_oeffnet_save_und_load_untermenue`.
- Fix 3 HIGH open_lane-Fehlerpfad: Behoben. `LfgLaneSpawner` hat `cleanup_created_lane`; `RouterLfgLaneSpawner` delegiert auf `LaneRouter::cleanup_lfg_created_lane` und damit `TempVoiceEngine::cleanup_lane` (`lfg_panel.rs:265-313`, `router.rs:853-862`). `handle_open_lane` bereinigt frisch erstellte Lanes bei `link_post_lane=false` und bei Link-Fehlern (`lfg_panel.rs:1573-1638`). Tests: `lfg_open_lane_cleanup_bei_rows_affected_race`, `lfg_open_lane_cleanup_bei_db_err_nach_spawn`.
- Fix 4 HIGH close_post best-effort: Behoben. `close_post` behandelt Discord-Archive/Lock best-effort: 404 debug/geschlossen, andere Fehler warnen, DB-Status wird trotzdem kontrolliert auf `closed`/`expired` gesetzt (`lfg_panel.rs:1848-1879`). Tests: `close_post_setzt_status_trotz_archive_404_und_500`, `reconcile_archive_fehler_schliesst_db_und_retryt_nicht_endlos`, `reconcile_und_lane_delete_doppelclose_bleibt_idempotent`.
- Fix 5 MITTEL Edit-Queue-Restart: Behoben. Beim Spawn wird ein Initial-Enqueue fuer alle `status='open' AND lane_id IS NOT NULL` gestartet (`lfg_panel.rs:1694-1718`, `:2032-2038`). Der Worker nutzt bereits konkrete `LfgEditError::RateLimited { retry_after_seconds }`; Serenity 0.12.5 reicht im `ErrorResponse` keinen Retry-After-Wert durch, daher bleibt der Glue-Fallback nur dort, wo kein konkreter Wert verfuegbar ist (`glue.rs:2093-2102`). Test: `initial_enqueue_packt_offene_lane_posts_in_render_queue`.
- Fix 6 MITTEL Join-Validierung + Texte: Behoben. Join prueft `member_voice_channel == lane_id` vor `move_member` und antwortet separat; volle Lane, fehlender Rang, nicht in Voice, eigener Post, tote Lane und Move-Fehler nutzen getrennte Konstanten (`lfg_panel.rs:1640-1688`). Die alte Sammelkonstante `LFG_PLACEHOLDER_TEXT` wurde entfernt; `rg LFG_PLACEHOLDER_TEXT rust` liefert keine Treffer. Tests: `lfg_join_blockt_volle_lane_ohne_move`, `lfg_join_blockt_user_der_schon_in_ziel_lane_ist`, `lfg_join_move_403_bleibt_ephemeral_und_rendert_nicht`.
- Fix 7 MITTEL Testluecken: Behoben. Neue Regressionen decken Flag-aus-Panel, Flag-aus-Lane-Delete, Ranked-Presets-Untermenue, `open_lane` rows_affected=0/DB-Err-Cleanup, Join voll/schon drin/Move-403, Archive-404/500, Reconcile-Archive-Fehler, Initial-Queue und Ticker-vs-Sink-Doppelclose ab (`lfg_panel.rs:2796-2842`, `:2965-3048`, `:3242-3436`, `interface.rs:1526-1609`).

## Konstanten-Aufspaltung
- `LFG_PANEL_BODY` -> LFG-Panel-Textdisplay (`lfg_panel.rs:46`, `:599`).
- `LFG_PANEL_BUTTON` -> LFG-Panel-Startbutton (`lfg_panel.rs:47`, `:601`).
- `LFG_MODE_PROMPT` -> ephemere Moduswahl (`lfg_panel.rs:48`, `:989`).
- `LFG_MODE_BUTTON_CASUAL`, `LFG_MODE_BUTTON_RANKED`, `LFG_MODE_BUTTON_STREET_BRAWL` -> Moduswahl-Buttons (`lfg_panel.rs:49-51`, `:682-684`).
- `LFG_MODAL_TITEL`, `LFG_MODAL_FELD_RANG_LABEL`, `LFG_MODAL_FELD_RANG_PLACEHOLDER`, `LFG_MODAL_FELD_PLAETZE_LABEL`, `LFG_MODAL_FELD_PLAETZE_PLACEHOLDER` -> Create-/Publish-Modal (`lfg_panel.rs:52-56`, `:702-717`).
- `LFG_ERR_KEIN_RANKED_RANG`, `LFG_ERR_RANG_UNBEKANNT`, `LFG_ERR_PLAETZE_UNGUELTIG`, `LFG_ERR_SCHON_AKTIVE_SUCHE`, `LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN` -> Formular/Open/Publish-Validierung (`lfg_panel.rs:57-61`, `:1006-1103`, `:1456-1568`, `:1589-1623`).
- `LFG_ERFOLG_POST_ERSTELLT`, `LFG_BTN_LANE_AUFMACHEN` -> Formular-/Open-Erfolg und Lane-Open-Button (`lfg_panel.rs:62-63`, `:734`, `:1125`, `:1594`).
- `LFG_POST_TITEL_SCHEMA`, `LFG_POST_BODY_HEADER`, `LFG_POST_BODY_VON`, `LFG_POST_BODY_MODUS`, `LFG_POST_BODY_RANG`, `LFG_POST_BODY_PLAETZE`, `LFG_POST_STATUS_OFFEN`, `LFG_POST_STATUS_VOLL` -> Forum-Post-Render (`lfg_panel.rs:64-72`, `:867-884`, `:909-912`).
- `LFG_BTN_BEITRETEN` -> Starter-Message-Join-Button (`lfg_panel.rs:72`, `rust/crates/dl-voice/src/glue.rs:1762`).
- `LFG_ERR_JOIN_LANE_TOT`, `LFG_ERR_JOIN_LANE_VOLL`, `LFG_ERR_JOIN_KEIN_RANG`, `LFG_ERR_JOIN_NICHT_IN_VOICE`, `LFG_ERR_JOIN_EIGENER_POST`, `LFG_ERR_JOIN_SCHON_DRIN`, `LFG_ERR_JOIN_MOVE_FEHLGESCHLAGEN` -> Join-Antworten (`lfg_panel.rs:73-79`, `:1642-1683`).
- `LFG_ERR_OPEN_NICHT_DEIN_POST`, `LFG_ERR_OPEN_NICHT_IN_VOICE`, `LFG_ERR_OPEN_LANE_SCHON_VERKNUEPFT` -> Open-Lane-Antworten (`lfg_panel.rs:80-82`, `:1575-1620`).
- `LFG_BTN_PUBLISH_LANE`, `LFG_ERR_PUBLISH_LANE_SCHON_VEROEFFENTLICHT` -> TempVoice-Publish-Button und Publish-Dedupe (`lfg_panel.rs:83-85`, `tempvoice/interface.rs:379-420`, `lfg_panel.rs:1512-1514`).
- `LFG_PRESETS_SUBMENU_TEXT`, `LFG_PRESETS_BTN_SAVE`, `LFG_PRESETS_BTN_LOAD` -> Ranked-Presets-Untermenue (`lfg_panel.rs:86-88`, `tempvoice/interface.rs:836-844`).

## TDD-Beleg
- Rot vor Fix: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice lfg_ -- --nocapture` scheiterte mit fehlender Cutover-Signatur, fehlenden semantischen Konstanten, fehlendem Cleanup-Port und fehlendem Initial-Enqueue-Helfer.
- Gruen nach Fix: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice lfg_ -- --nocapture` (34 passed) und `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice tempvoice::interface::tests -- --nocapture` (9 passed).

## Verifikation
- Gruen: `SQLX_OFFLINE=true cargo check --workspace --all-targets`
- Gruen: `SQLX_OFFLINE=true cargo build --workspace`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-community`
- Gruen: `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`
- Gruen: `cargo fmt --all -- --check`
- Gruen: `git diff --check`

## Offene Punkte
- Keine bekannten offenen Rework-Punkte. Serenity 0.12.5 exponiert im verwendeten `ErrorResponse` keinen konkreten Retry-After-Wert; der LFG-Worker verarbeitet konkrete `LfgEditError::RateLimited`-Werte, der Serenity-Glue nutzt daher weiterhin den vorhandenen Fallback, wenn kein Wert verfuegbar ist.

# W3.4b W2 - LFG-Persistenz + Formular-Flow (2026-07-03)

## Fortschritt
- Pflichtkontext gelesen: W3.4b Phase 1, W1 Kritiker und W1 Rework inkl. W2-Cutover-Checkliste.
- Migration `2026070320_lfg_posts.sql` angelegt: `voice.lfg_posts` ohne FK auf `voice.tempvoice_lanes`, mit Unique auf `thread_id` und nullable Unique auf `lane_id`.
- Fresh-Migrations-Vertrag um `2026070320` sowie Spalten-/Unique-Checks fuer `voice.lfg_posts` erweitert.
- Privacy-Vertrag um `voice.lfg_posts.owner_id` erweitert; Contract-Test gezielt gruen.
- LFG-Panel-Flow umgesetzt: `lfg:create:start` -> ephemere Moduswahl -> `lfg:create:modal:<mode>` -> Validierung -> Forum-Post-Port -> Persistenz mit `expires_at = now() + 24 hours`.
- Ranked-Gate nutzt `VERIFIED_RANK_ROLE_IDS`; Casual und Street-Brawl bleiben ungegated.
- Serenity-Port nutzt lokal geprueftes `ChannelId::create_forum_post`; `starter_message_id` wird per kontrolliertem ersten Thread-Message-Fetch gesetzt oder bleibt `NULL`.
- Alter Text-LFG-Responder startet nur noch, wenn `DL_LFG_FORUM_CUTOVER` aus ist. AI-Onboarding und statischer Onboarding-Wizard repointen LFG-Ziele bei Cutover auf `DL_LFG_PANEL_CHANNEL_ID`.
- W3 bewusst nicht umgesetzt: keine Post-Lane-Live-Kopplung, kein Join-/Lane-Open-Button, kein Expiry-Enforcement-Ticker.

## Verifikation
- Gruen: `SQLX_OFFLINE=true cargo build --workspace`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-server-as-code -- --ignored --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`
- Gruen: `cargo test -p dl-community`
- Gruen: `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`
- Gruen: `cargo fmt --all -- --check`

# W3.4b W2 Kritiker (2026-07-03)

Scope: adversarialer Review des uncommitted Diff gegen `729cef06`. Keine Source-Aenderungen ausser diesem Report, kein Commit/Push.

## Befunde

### DEPLOY-BREAKER
1. Flag-aus-Invariante ist verletzt: Das neue LFG-Panel kann vor Cutover sichtbar gepostet werden.
   - Datei/Zeilen: `rust/bin/dl-bot/src/main.rs:483`, `:497`, `:1221-1233`; `rust/bin/dl-bot/src/serversync.rs:2312-2320`, `:4918-4936`.
   - Szenario: `DL_LFG_FORUM_CUTOVER` ist unset/false, aber `DL_LFG_PANEL_CHANNEL_ID` ist fuer den spaeteren Cutover bereits gesetzt. Beim Gateway-Ready postet `lfg_panel_interface_ready.ensure_panel()` trotzdem das Components-V2-LFG-Panel; zusaetzlich kann `/serversync/lfg-panel-apply` mit `confirm=true` das Panel unter Flag-aus posten. User sehen damit neues `lfg:create:*`-Verhalten, waehrend der alte Text-Responder noch laeuft.
   - Fix: `cutover_enabled` in `LfgPanelInterface`/ServerSync durchreichen und `ensure_panel`, `apply_panel(confirm=true)` sowie `lfg:create:*`-Handling bei Flag-aus hart blocken. Dry-Run darf einen `blocked_reason` liefern.

### HIGH
2. Kein Spam-/Doppelpost-Schutz fuer offene LFG-Posts.
   - Datei/Zeilen: `rust/crates/dl-voice/src/lfg_panel.rs:656-690`, `:708-748`; `rust/crates/dl-central-db/migrations/2026070320_lfg_posts.sql:24-28`.
   - Szenario: Ein User submitet das Formular 20-mal oder klickt zweimal schnell. Jeder erfolgreiche Submit erstellt erst einen neuen Discord-Forum-Thread und schreibt danach eine weitere `status='open'`-Row. Die DB hat nur einen normalen `owner_id`-Index, keinen Race-Schutz.
   - Fix: App-seitig vor dem Discord-Post max. 1 offenen Post pro `owner_id` erzwingen und DB-seitig `CREATE UNIQUE INDEX ... ON voice.lfg_posts(owner_id) WHERE status='open'` ergaenzen; bei Treffer ephemer blocken oder bestehenden Post ersetzen/schliessen.

3. Discord-Thread wird vor DB-Persistenz erstellt; Insert-Fehler erzeugt Zombie-Posts.
   - Datei/Zeilen: `rust/crates/dl-voice/src/lfg_panel.rs:656-663`, `:676-690`; Cleanup-Port fehlt in `rust/crates/dl-voice/src/lfg_panel.rs:133-141`.
   - Szenario: `create_forum_post` succeeded, danach schlaegt `INSERT INTO voice.lfg_posts` wegen DB-Ausfall, fehlender Migration oder Constraint-Verletzung fehl. Der User bekommt nur eine ephemere Fehlermeldung, aber im Forum existiert ein sichtbarer Thread ohne DB-Row und damit ohne spaetere Verwaltung/Close/Reconcile-Basis.
   - Fix: Erst eine DB-Reservation (`creating`) mit Race-Constraints schreiben und danach den Thread aktualisieren, oder bei Insert-Fehler den Thread ueber Port `delete/archive/lock` aufraeumen und mit `thread_id`/`owner_id` strukturiert loggen. Ein Reconcile fuer orphan Threads ergaenzen.

4. Rang-Validierung akzeptiert gemischten Muell, sobald ein bekannter Rang vorkommt.
   - Datei/Zeilen: `rust/crates/dl-voice/src/lfg_panel.rs:524-542`; Referenz `rank_index` in `rust/crates/dl-voice/src/tempvoice/logic.rs:41-44`.
   - Szenario: `Phantom bis Kartoffel` wird zu `[Phantom]` gefiltert und als gueltiger Single-Rank-Post persistiert; `Kartoffel bis Ritualist` wird als `Ritualist` akzeptiert. Unbekannte Tokens verschwinden still statt ephemerem Fehler.
   - Fix: Parser als kleine Grammatik bauen: erlaubte Trenner (`bis`, `-`, `to`) separat behandeln, alle nicht-leeren Rang-Tokens muessen bekannt sein; Tests fuer unbekannt, gemischt-unbekannt, vertauscht, gross/klein, Leerzeichen/Umlaute.

### MEDIUM
5. Cutover-true mit fehlender/ungueltiger Panel-ID schaltet altes LFG ab, ohne neuen Einstieg bereitzustellen.
   - Datei/Zeilen: `rust/bin/dl-bot/src/main.rs:483-492`, `:1139-1149`, `:1231-1233`; Fallbacks in `rust/crates/dl-community/src/onboarding.rs:91-105` und `rust/crates/dl-community/src/ai_onboarding.rs:725-739`.
   - Szenario: Operator setzt `DL_LFG_FORUM_CUTOVER=true`, vergisst aber `DL_LFG_PANEL_CHANNEL_ID` oder setzt `0`. Startup warnt nur, der alte Text-Responder wird deaktiviert, kein Panel wird gepostet, und Onboarding faellt auf die alte Kanal-ID zurueck.
   - Fix: Bei Cutover=true eine valide positive Panel-/Forum-ID als Startup-Precondition erzwingen oder den alten Responder aktiv lassen, bis die neue Ziel-ID valide ist.

### LOW
6. AI-Onboarding-Flag-Test laeuft nicht in der dokumentierten Default-Verifikation.
   - Datei/Zeilen: Testmodul-Gate `rust/crates/dl-community/src/ai_onboarding.rs:846`; Test `:986-1008`.
   - Szenario: `cargo test -p dl-community` listet nur den statischen Onboarding-LFG-Test; der AI-Quick-Action-Test ist hinter `feature="testing"`. `SQLX_OFFLINE=true cargo test -p dl-community --features testing ...` scheitert lokal an fehlendem SQLx-Cache fuer einen bestehenden Privacy-Testquery (`privacy.rs:1843`), bevor der reine Flag-Test laufen kann.
   - Fix: Reinen `lfg_target_channel_id_from_lookup`-Test aus dem `testing`-Feature herausziehen oder SQLx-Cache/CI-Kommando fuer `--features testing` nachziehen.

## Sauber-Befunde je Linse
- Flag-aus: Alter Text-Responder bleibt bei `DL_LFG_FORUM_CUTOVER=false` aktiv (`main.rs:97-99`, `:1139-1149`), AI-Onboarding und statischer Wizard fallen bei Flag-aus auf `1376335502919335936` zurueck. Aber Panel-Apply/Auto-Ensure ist nicht gegated, siehe DEPLOY-BREAKER.
- Flow: `dispatch_modal` routet Modal-Submits ueber `router.resolve_component` (`dl-discord/src/dispatch.rs:179-252`), und `router.on_prefix("lfg:create:", ...)` deckt Button- und Modal-IDs ab (`lfg_panel.rs:777-778`). Start-Antworten sind ephemer (`lfg_panel.rs:601-606`).
- Validierung: Slots `0`, `6`, `-1`, `abc`, leer werden app-seitig abgelehnt (`lfg_panel.rs:508-512`); DB hat `requested_slots BETWEEN 1 AND 5` (`2026070320_lfg_posts.sql:11`). Vertauschte Rangbereiche werden abgelehnt (`lfg_panel.rs:537-542`).
- Ranked-Gate: `mode=ranked` wird vor Modal und beim Modal-Submit erneut geprueft (`lfg_panel.rs:610-638`); `casual` und `street_brawl` laufen durch `mode != Ranked` ungegated (`lfg_panel.rs:594-598`).
- Migration/DB/Privacy: Nur neue Migration `2026070320_lfg_posts.sql` liegt untracked vor; keine Alt-Migration geaendert. Fresh-Schema prueft Spalten sowie `thread_id`/`lane_id` Unique. Privacy-Vertrag registriert `voice.lfg_posts.owner_id` und der Contract-Test scannt `owner_id` aus Migrationen, keine Gruenwasch-Allowlist.
- Serenity: `CreateForumPost::new`, `auto_archive_duration`, `audit_log_reason`, `CreateMessage::allowed_mentions` und `ChannelId::create_forum_post` existieren in lokaler `serenity-0.12.5`-Quelle; `SQLX_OFFLINE=true cargo check -p dl-voice -p dl-community -p dl-bot` ist gruen.
- Orphan-Fetch: Fehler beim Starter-Message-Fetch werden geloggt und `starter_message_id` bleibt `NULL` (`lfg_panel.rs:664-673`); der kritischere Insert-nach-Post-Pfad bleibt offen, siehe HIGH.

## Kritiker-Verifikation
- Gruen: `SQLX_OFFLINE=true cargo check -p dl-voice -p dl-community -p dl-bot`
- Gruen: `git diff --check`
- Gruen: `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture`
- Gruen: `./scripts/central_test_db.sh cargo test -p dl-voice lfg_panel -- --nocapture` (11 passed; TestDb-Pool-Close-Timeouts nur Cleanup-Warnungen)
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-community`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot lfg -- --nocapture`
- Rot/Umgebung: `SQLX_OFFLINE=true cargo test -p dl-voice lfg_ -- --nocapture` ohne Test-DB scheitert an fehlender `CENTRAL_TEST_DSN`/`DATABASE_URL`/`DEADLOCK_CENTRAL_DSN`.
- Rot/Tooling: `SQLX_OFFLINE=true cargo test -p dl-community --features testing lfg_quick_action_target -- --nocapture` scheitert vor Testlauf an fehlendem SQLx-Offline-Cache fuer `privacy.rs:1843`.

# W3.4b W2 Rework (2026-07-03)

## Fix-Status
- Fix 1 DEPLOY-BREAKER: Effektives `lfg_cutover_active = DL_LFG_FORUM_CUTOVER && valide DL_LFG_PANEL_CHANNEL_ID` in `main.rs` eingefuehrt und an Panel-Interface, Gateway-Ready, alten LFG-Responder und LFG-Interactions gekoppelt. `apply_panel(confirm=true)` blockt bei inaktivem Cutover; Dry-Run liefert `blocked_reason="cutover_disabled"`.
- Fix 2+3 HIGH: `voice.lfg_posts` auf Reservation-first umgestellt: `creating`-Row vor Discord-Post, partieller Unique-Index auf `owner_id WHERE status IN ('creating','open')`, danach Update auf `open`. Fehler bei Discord-Post/DB-Update loeschen die Reservation; DB-Update-Fehler archivieren/locken den erstellten Thread per Port und loggen strukturiert.
- Fix 4 HIGH: Rangbereich-Parser ist jetzt eine Grammatik fuer leer, Einzelrang, `<rang> bis <rang>` und `<rang>-<rang>`; unbekannte oder vertauschte Tokens werden abgelehnt.
- Fix 5 MEDIUM: `DL_LFG_FORUM_CUTOVER=true` ohne valide Panel-ID ist effektiv inaktiv, warnt laut und laesst den alten Text-Responder sowie Onboarding-Ziele auf dem Legacy-Kanal.
- Fix 6 LOW: Reiner AI-Onboarding-LFG-Ziel-Test liegt in normalem `#[cfg(test)]` und laeuft ohne `feature="testing"`.

## W3-Reconcile-Hinweis
- W3-Reconcile soll stale `voice.lfg_posts` mit `status='creating'` und `created_at < now() - interval '5 minutes'` aufraeumen, damit abgebrochene Reservations nicht dauerhaft Owner blockieren.

## Rework-Verifikation
- Gruen: `SQLX_OFFLINE=true cargo check -p dl-bot -p dl-community -p dl-voice -p dl-central-db`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice lfg_panel -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-community lfg_quick_action_target -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot lfg -- --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo build --workspace`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-community`
- Gruen: `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`
- Gruen: `cargo fmt --all -- --check`

# W3.4b W1 Kritiker (2026-07-03)

## Report
Scope: uncommitted Diff im Worktree `Deadlock-Bots-w34-router`. Keine Source-Aenderungen ausser diesem Report, kein Commit/Push.

### DEPLOY-BREAKER
1. Befund #0, von Claude bestaetigt und im Diff verifiziert: Eine bereits live applizierte Migration wurde inline geaendert.
   - Datei/Zeilen: `rust/crates/dl-central-db/migrations/2026070210_server_config_schema.sql:14-27`, `:104-119`; Runner `rust/bin/dl-central-migrate/src/main.rs:12`.
   - Szenario: Prod hat `2026070210_server_config_schema.sql` bereits in `_sqlx_migrations`. Naechster Deploy startet `dl-central-migrate`, `sqlx::migrate!` vergleicht die Checksum und bricht mit sinngemaess "previously applied but has been modified" ab.
   - Fix: Die beiden Inline-Spalten aus `2026070210` komplett zuruecknehmen. Die neue Migration `rust/crates/dl-central-db/migrations/2026070310_server_config_forum_metadata.sql:1-5` reicht, weil sie lexikografisch danach laeuft und `ADD COLUMN IF NOT EXISTS` nutzt.
   - Zusatzcheck: Code/Tests haengen nicht sinnvoll von der Inline-Aenderung ab; die betroffenen Queries in `dl-server-as-code/src/db.rs` sind dynamische `sqlx::query`/`Row`-Zugriffe, und `SQLX_OFFLINE=true cargo build --workspace` ist gruen.

### HIGH
2. Fresh-Migrations-Contract ist rot.
   - Datei/Zeilen: Erwartung ohne neue Spalten in `rust/crates/dl-central-db/tests/fresh_migrations_schema.rs:104-120` und `:205-220`; Assertion bei `:1107-1112`.
   - Szenario: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture` baut eine frische DB mit der neuen Spalte, der Test erwartet aber die alte Spaltenliste und faellt auf `server_config.desired_channels columns` durch.
   - Fix: `server_config_table_contracts()` fuer `desired_channels` und `live_snapshot_channels` um `default_auto_archive_duration` erweitern. Danach Fresh-Test erneut laufen lassen.

3. ServerGuide wird nach dem LFG-Forum-Cutover blockiert.
   - Datei/Zeilen: `rust/bin/dl-bot/src/serversync.rs:2581-2584` resolved `mitspieler-suche`; `:2633-2636` baut daraus weiter eine `SERVER_GUIDE_ACTION_TYPE_CHAT`-Action; `:2711-2738` blockiert CHAT-Actions ohne @everyone `SEND_MESSAGES`; `:3309-3312` definiert die Pruefung.
   - Gegenlaeufiger W3.4b-Zustand: Das LFG-Forum verweigert @everyone `SEND_MESSAGES` absichtlich in `rust/crates/dl-server-as-code/src/rules.rs:1396-1404`.
   - Szenario: Nach ServerSync-Apply existiert `🎯mitspieler-suche` als Forum mit Deny fuer freie Posts. `serversync serverguide-preview` / HTTP `/serversync/serverguide-preview` resolved den Forum-Kanal und blockiert mit "Action `Such dir Mitspieler`: Kanal `<id>` ist nicht @everyone-sendbar".
   - Fix: ServerGuide-Aktion fuer LFG auf `SERVER_GUIDE_ACTION_TYPE_VIEW` umstellen oder auf einen sendbaren Einstieg verweisen, der das Formular/Panel oeffnet. Einen Regressionstest mit Forum + LFG-Overwrite ergaenzen.

### MEDIUM
4. Runtime-Referenzen zeigen weiter auf die alte Textkanal-ID, die W3.4b archiviert/versteckt.
   - Datei/Zeilen: Alter Textkanal wird umbenannt/versteckt in `rust/crates/dl-server-as-code/src/rules.rs:1285-1325`; neues Forum bekommt eine neue synthetische ID in `:1329-1369`. Statische Alt-ID bleibt in `rust/crates/dl-community/src/ai_onboarding.rs:35` und Quick-Action `:697-702`; alter LFG-Responder hoert weiter auf `rust/crates/dl-activity/src/lfg.rs:712-713` und wird in `rust/bin/dl-bot/src/main.rs:1097-1104` gestartet; `rust/crates/dl-community/src/onboarding_steps.json:33-34` nennt ebenfalls die alte ID.
   - Szenario: Nach Apply fuehrt AI-Onboarding "Spieler-Suche" auf `1376335502919335936`, also den archivierten/hidden Textkanal, nicht auf das neue Forum. Der alte LFG-Responder verarbeitet nur Nachrichten im alten Kanal; Forum-Threads/Formular-Interaktionen erreichen ihn nicht.
   - Fix: Alt-ID-Referenzen konfigurieren/auflosen statt hart codieren, nach Forum-Create die neue Forum-ID setzen, oder W1-Struktur hinter einem Cutover-Flag lassen, bis W2 den vollstaendigen LFG-Flow uebernimmt.

5. W1 ist live nicht als alleiniger Cutover nutzbar: alter Schreibkanal weg, neuer Button nur Platzhalter.
   - Datei/Zeilen: Altes LFG wird archiviert/versteckt `rules.rs:1285-1325`; Forum blockt freie Posts `rules.rs:1396-1404`; Button-Handler antwortet nur `Platzhalter` in `rust/crates/dl-voice/src/lfg_panel.rs:274-284`.
   - Szenario: Owner applied W1 vor W2. Normale User koennen im Forum keinen Post erstellen, der Panel-Button erzeugt keinen LFG-Post, und der bisherige Textkanal ist nicht mehr sichtbar/sendbar. Ergebnis: LFG ist fuer User praktisch aus.
   - Fix: Struktur-Delta erst mit funktionsfaehigem Formular/Post-Service aktivieren, oder altes Text-LFG sichtbar/sendbar lassen, bis der neue Flow produktionsbereit ist.

6. LFG-Panel-KV-Idempotenz hat keinen Stale-Message-Recovery-Pfad.
   - Datei/Zeilen: `rust/crates/dl-voice/src/lfg_panel.rs:89-135` entscheidet nur anhand der KV-Message-ID zwischen Edit/Post; es gibt keinen History-Scan und keinen 404-Repost-Fallback. Der robustere Router-Pfad scannt/adoptiert History in `rust/crates/dl-voice/src/router.rs:443-558`.
   - Szenario: KV enthaelt `components_v2_message_id`, die Discord-Message wurde manuell geloescht. Naechster Restart/Apply versucht nur `PATCH /messages/<alte_id>`, scheitert, und postet kein neues Panel. User sehen keinen LFG-Einstieg.
   - Fix: Wie Router History scannen und vorhandenes V2-Panel adoptieren; bei edit-404 KV loeschen und nach erfolgreichem Scan neu posten.

### LOW
7. `DL_LFG_PANEL_CHANNEL_ID` validiert `0`/Muell nicht hart.
   - Datei/Zeilen: `rust/bin/dl-bot/src/main.rs:446-447`; Missing-Channel-Handling in `rust/crates/dl-voice/src/lfg_panel.rs:108-116`.
   - Szenario: Unset oder nicht-numerisch wird still zu `None`; Dry-Run meldet `blocked_missing_channel`, Confirm liefert Fehler. `0` wird als Some(0) akzeptiert und fuehrt erst beim Discord-REST-Call zu einem Fehler auf `/channels/0/messages`.
   - Fix: `NonZeroU64` parsen, ungueltige Werte warnen/blocken und in Dry-Run/Startup eindeutig ausgeben.

### Geprueft und sauber befunden
- Struktur-Delta: Code archiviert alte Nicht-Forum-Kanaele per Alias (`rules.rs:1285-1325`) und erstellt/findet das Forum separat (`rules.rs:1329-1369`). Durch `matching_key` (`rules.rs:2270-2275`) matched auch ein Live-Altkanal namens `🎯mitspieler-suche`; zweiter Apply ist logisch idempotent, weil `archiv-mitspieler-suche` nicht mehr als LFG-Alias matched und das existierende Forum gefunden wird. Der vorhandene Test deckt nur `spieler-suche`, nicht diese Kollisionsvariante.
- Rechte: Desired-Modell enthaelt @everyone Deny fuer `SEND_MESSAGES`, `CREATE_PUBLIC_THREADS`, `CREATE_PRIVATE_THREADS` plus Allow fuer `SEND_MESSAGES_IN_THREADS` (`rules.rs:1396-1404`). Bot-/Teamrollen bekommen Send/Create/Manage-Threads (`rules.rs:1384-1428`). Apply remappt synthetische IDs und schreibt Overwrites generisch korrekt (`rust/crates/dl-server-as-code/src/apply.rs:526-600`).
- DB-Persistenz: `dl-server-as-code/src/db.rs` liest/schreibt `default_auto_archive_duration` symmetrisch in Snapshot und Desired; die neue ALTER-Migration ist nullable/idempotent. SQLx-Offline-Cache musste dafuer nicht aktualisiert werden; Build ist gruen.
- Panel-Wiring: `dl_voice::lfg_panel::register` ist in `main.rs:439-440` verdrahtet, ServerSync setzt das Interface `main.rs:448-455`, Startup applyt bei gesetztem Channel `main.rs:1175-1187`, HTTP `/serversync/lfg-panel-apply` nutzt dieselbe Auth wie die anderen Endpoints `serversync.rs:4915-4935`, und Multipart geht ueber reqwest `files[{id}]` in `dl-voice/src/glue.rs:234-267`.
- Gruenwasch-Check: `welcome_publish.rs`, `db_workflow.rs` und `diff_engine.rs` wurden nur mechanisch um `default_auto_archive_duration: None` erweitert; keine erkennbare Fixture-Aenderung versteckt das LFG-Verhalten.

### Verifikation
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code welle34b_lfg -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot lfg_panel_apply_http_ist_dry_run_default_und_liefert_shell_payload -- --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice lfg_panel -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serverguide_builder_blockt_nur_bei_nicht_sendbarem_chat -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo build --workspace`
- Gruen: `git diff --check`
- Rot: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture` wegen fehlender neuer Spalte im Testvertrag.

# W3.4b W1 Rework (2026-07-03)

## Fix-Status
- Fix 1 DEPLOY-BREAKER: `2026070210_server_config_schema.sql` per `git checkout --` auf HEAD zurueckgenommen; `2026070310_server_config_forum_metadata.sql` bleibt alleinige neue Spaltenquelle.
- Fix 2 HIGH: Fresh-Migrations-Contract fuer `desired_channels` und `live_snapshot_channels` um `default_auto_archive_duration` in realer ALTER-Reihenfolge erweitert. Gezielter Fresh-Test danach gruen.
- Fix 3 HIGH: ServerGuide-Aktion `Such dir Mitspieler` auf `SERVER_GUIDE_ACTION_TYPE_VIEW` umgestellt; Regressionstest mit Forum/@everyone-SEND_MESSAGES-Deny gruen.
- Fix 4+5 MEDIUM: `DL_LFG_FORUM_CUTOVER` als Default-aus-Flag im Desired-Modell eingefuehrt. Cutover aus erzeugt kein LFG-Strukturdelta; Cutover an erzeugt Forum + Archiv, ohne `kind`-Update. Exakte Namenskollision `🎯mitspieler-suche` getestet.
- Fix 6 MEDIUM: LFG-Panel-Publisher bekommt Router-artige Recovery: History-Scan, Adoption nur von Components-V2-Messages ohne Embeds mit `lfg:create:`-Custom-ID, KV-Edit-404 loescht stale Message-ID und repostet/adoptiert.
- Fix 7 LOW: `DL_LFG_PANEL_CHANNEL_ID` wird als positive NonZero-ID geparst; unset/leer/0/ungueltig ergibt `None`, Startup-Warnung und `blocked_reason` im Dry-Run.

## W2-Cutover-Checkliste
- AI-Onboarding-Alt-ID `1376335502919335936` in `rust/crates/dl-community/src/ai_onboarding.rs` fuer W2 auf die neue LFG-Forum-/Panel-Ziel-ID repointen.
- `rust/crates/dl-community/src/onboarding_steps.json` fuer W2 auf die neue LFG-Forum-/Panel-Ziel-ID repointen.
- Alten LFG-Responder aus `rust/bin/dl-bot/src/main.rs` fuer W2 abschalten oder eindeutig auf den neuen Formular/Post-Service umhaengen.

## Rework-Verifikation
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serverguide_builder_blockt_nur_bei_nicht_sendbarem_chat -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code welle34b_lfg -- --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice lfg_panel -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot lfg_panel_apply_http_ist_dry_run_default_und_liefert_shell_payload -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot lfg_panel_channel_id_parst_nur_positive_nonzero_ids -- --nocapture`
- Gruen: `SQLX_OFFLINE=true cargo build --workspace`
- Gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored --nocapture`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice`
- Gruen: `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`
- Gruen: `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`
- Gruen: `cargo fmt --all -- --check`

# W3.4b W1 Struktur-Delta + Panel-Shell (2026-07-03)

## Ziel
`🎯mitspieler-suche` wird als neues Forum im Desired-Modell angelegt; der alte Textkanal bleibt als archivierter History-Kanal erhalten. Freies Posten im Forum wird per Overwrite verhindert, Thread-Antworten bleiben erlaubt. LFG-Panel-Publisher wird als Shell analog Router verdrahtet. Kein Commit/Push.

## Fortschritt
- Pflichtkontext gelesen: W3.4b Phase 1 Zeilen 1-89 und Konzept C (`docs/onboarding-redesign/welle3-modernisierung-konzept.md:56-64`).
- Befund: Existing Apply sendet Channel-`type` nur beim Create; Diff wuerde `kind` als Update-Feld erkennen. W1 braucht daher Tests fuer Create-Forum plus Archiv-Update statt Text->Forum-Update.
- TDD gestartet: Red-Tests fuer Forum-Desired-Modell, Archivierung des Altkanals, LFG-Forum-Rechte und Guard gegen `kind`-Update werden ergaenzt.
- Implementiert: `ChannelSpec.default_auto_archive_duration` inkl. Import/Diff/Apply/DB-Spalte; neues Forum `🎯mitspieler-suche` wird als Create modelliert, alter Textkanal wird zu `archiv-mitspieler-suche` ins Archiv verschoben.
- Implementiert: LFG-Forum-Overwrites blocken fuer `@everyone` freie Posts/Public-/Private-Threads und erlauben Thread-Antworten; Bot-/Teamrollen erhalten Erstellen/Thread-Verwaltung.
- Implementiert: LFG-Panel-Shell in `dl_voice::lfg_panel` mit Components-V2-Payload, Button `lfg:create:start`, Platzhalter-Antwort, KV-idempotentem ServerSync-Publisher und reqwest-Multipart-Glue ueber `files[{id}]`.
- Verifikation gruen aus `rust/`: `SQLX_OFFLINE=true cargo build --workspace`; `SQLX_OFFLINE=true cargo test -p dl-server-as-code`; `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice`; `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check`.
- Hinweis: Im Worktree liegt `central_test_db.sh` unter `rust/scripts/`, nicht unter Root-`scripts/`; Cargo-Kommandos wurden aus `rust/` ausgefuehrt.

# W3.4b Phase 1 - LFG v2 + Leaderboard (2026-07-03)

## Scope
Read-only-Analyse und Implementierungsdesign. Keine Produktionscode-Aenderung, kein Commit/Push. Ergebnis: belastbarer Wellen-DAG fuer LFG v2 mit Live-Kopplung Post <-> Lane und oeffentlichem Rank-Leaderboard.

## Pflichtkontext
- Konzept C ist die verbindliche Zielrichtung: `🎯mitspieler-suche` wird Forum, Posts entstehen nur per Formular, Felder sind Modus/Rang-Bereich/freie Plaetze; zwei Erstellwege und Live-Zustand aus echter Voice-Belegung sind Kernidee (`docs/onboarding-redesign/welle3-modernisierung-konzept.md:56-64`).
- Konzept D koppelt W3.4b an W3.4a: Router ersetzt die Creator, Modus -> Lane, Verwaltung der eigenen Lane ueber ein TempVoice-Button-Panel (`docs/onboarding-redesign/welle3-modernisierung-konzept.md:66-68`).

## 1. Forum-Mechanik
- Ist-Zustand: `mitspieler-suche` ist im Soll-Modell ein normaler Textkanal in `CATEGORY_DEADLOCK` (`rust/crates/dl-server-as-code/src/rules.rs:117`). Neue synthetische Kanaele aus diesem Pfad werden hart als `ChannelKind::Text` angelegt (`rust/crates/dl-server-as-code/src/rules.rs:656-680`); Rename-/Alias-Regeln halten den Namen nur auf `🎯mitspieler-suche` (`rust/crates/dl-server-as-code/src/rules.rs:738`, `rust/crates/dl-server-as-code/src/rules.rs:768-792`).
- Discord-Typwechsel ist kein sauberer Apply-Pfad: `ChannelKind::Forum` ist zwar im Modell vorhanden und mappt auf Discord-Type 15 (`rust/crates/dl-server-as-code/src/model.rs:44-110`), aber `ChannelSpec` hat nur generische Felder und keine Forum-spezifischen Tags/Layout/Default-Archive-Felder (`rust/crates/dl-server-as-code/src/model.rs:152-166`). Die Diff-Engine wuerde `kind` als Feld-Diff erkennen (`rust/crates/dl-server-as-code/src/diff.rs:318-322`), der Apply-Pfad sendet `type` aber nur bei Create, nicht bei Update (`rust/crates/dl-server-as-code/src/apply.rs:127-153`, `rust/crates/dl-server-as-code/src/apply.rs:660-681`). Design daher: Struktur-Delta mit neuem Forum-Kanal, kein Text->Forum-Update.
- ServerSync kann Kanaele anlegen, wenn ein `ChannelSpec` mit `kind=Forum` im Desired-Modell steht, weil `create_channel(... channel_payload(..., true))` den Type sendet (`rust/crates/dl-server-as-code/src/apply.rs:127-141`). Was fehlt: Rules-Pfad wie `ensure_named_forum_channel` oder Channel-Kind-Override fuer `mitspieler-suche`; optional Erweiterung von `ChannelSpec`/Apply fuer Forum-Metadaten, denn Serenity kennt `default_auto_archive_duration`, `available_tags`, `default_sort_order` fuer Forum-Create (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/builder/create_channel.rs:220-254`).
- Rechte-Modell: @everyone-Basis erlaubt aktuell `SEND_MESSAGES`, `CREATE_PUBLIC_THREADS` und `SEND_MESSAGES_IN_THREADS` (`rust/crates/dl-server-as-code/src/rules.rs:207-221`). Das LFG-Forum braucht deshalb explizite Denies fuer freies Posten/Threads. Bot und Team behalten Erstellen/Thread-Verwaltung; User bekommen Lesen und Button-Interaktion, optional Thread-Antworten nur wenn Owner es will.
- BotMessage im generischen Apply ist No-op (`rust/crates/dl-server-as-code/src/apply.rs:75-84`). Sticky/Guidelines/Panel sollte daher ueber einen eigenen Publisher analog Router/Welcome laufen; ServerSync hat bereits einen `router_apply`-Pfad und verdrahtet `RouterInterface` (`rust/bin/dl-bot/src/serversync.rs:610-638`, `rust/bin/dl-bot/src/serversync.rs:2287-2297`, `rust/bin/dl-bot/src/serversync.rs:4567-4593`).
- Posting-Modell: User duerfen nicht frei posten. Der Einstieg ist eine Guidelines-/Panel-Message mit `Suche erstellen`; Bot erstellt den Forum-Post. Serenity 0.12.5 bietet dafuer `CreateForumPost` mit Name, Message, Auto-Archive, Slowmode und Tags (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/builder/create_forum_post.rs:13-24`, `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/builder/create_forum_post.rs:27-95`) sowie `ChannelId::create_forum_post` (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/model/channel/channel_id.rs:953-964`) und `Http::create_forum_post` (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/http/client.rs:456-488`).
- Modal-Grenze: `dl-discord::ModalSpec` kann nur Textfelder (`rust/crates/dl-discord/src/interactions.rs:56-74`), Dispatch liest nur `InputText` aus (`rust/crates/dl-discord/src/dispatch.rs:179-204`) und serialisiert nur Component-Type 4 (`rust/crates/dl-discord/src/dispatch.rs:358-376`). String-Select-Werte existieren fuer Components (`rust/crates/dl-discord/src/dispatch.rs:119-124`). Design: Modus per Select/Button vor dem Modal, danach Modal fuer Rangbereich/freie Plaetze; "Lane gleich aufmachen?" als Ergebnis-Button statt Checkbox, solange `ModalSpec` nicht erweitert wird.
- Custom-ID-Schema: `lfg:create:start` oeffnet den Flow, `lfg:create:mode:<mode>` setzt `casual|ranked|street_brawl`, Modal-ID `lfg:create:modal:<mode>` traegt den Modus, Ergebnis-Button `lfg:open_lane:<lfg_id>` erzeugt optional eine Lane, Post-Button `lfg:join:<lfg_id>` moved targeted in die gemappte Lane, TempVoice-Panel-Button `lfg:publish_lane` publisht die aktuelle eigene Lane. Prefix-Routing ist im Router bereits etabliert (`rust/crates/dl-voice/src/router.rs:1019-1024`).
- Cutover-Risiko: der alte LFG-Responder hoert auf Textnachrichten im alten Channel (`rust/crates/dl-activity/src/lfg.rs:712-714`, `rust/crates/dl-activity/src/lfg.rs:1474-1508`) und wird im Bot gestartet (`rust/bin/dl-bot/src/main.rs:1086-1093`). W3.4b muss ihn fuer den Forum-Cutover deaktivieren oder eindeutig umhaengen.

## 2. Live-Kopplung Post <-> Lane
- TempVoice-Lifecycle-Hooks existieren in der Engine, aber nicht als generischer Domain-Event-Bus: `handle_event` verarbeitet Join/Leave/Move (`rust/crates/dl-voice/src/tempvoice/engine.rs:506-532`), `on_join` backfillt/persistiert verwaltete Lanes und refreshed Namen (`rust/crates/dl-voice/src/tempvoice/engine.rs:535-612`), `on_leave` transferiert Owner oder ruft bei leerer Lane `cleanup_lane` (`rust/crates/dl-voice/src/tempvoice/engine.rs:614-679`).
- Router-Lanes sind anschlussfaehig: `create_router_lane` gibt `Option<u64>` Lane-ID zurueck (`rust/crates/dl-voice/src/tempvoice/engine.rs:730-754`), legt Modus-Kategorien und Caps an (`rust/crates/dl-voice/src/tempvoice/engine.rs:756-845`), persistiert in `voice.tempvoice_lanes` (`rust/crates/dl-voice/src/tempvoice/engine.rs:810-824`) und bewegt den User in die Lane (`rust/crates/dl-voice/src/tempvoice/engine.rs:833-840`). `cleanup_lane` entfernt State/DB und loescht den Discord-Kanal (`rust/crates/dl-voice/src/tempvoice/engine.rs:1641-1653`).
- Event-Quelle fuer LFG: TempVoice subscribed bereits Voice/Channel/Gateway-Events und startet einen Purge-Loop (`rust/crates/dl-voice/src/tempvoice/engine.rs:1799-1840`). Empfehlung: eigener LFG-Service subscribed dieselben VoiceEvents fuer Occupancy-Updates plus periodischer Reconcile; fuer sicheres Close-on-delete entweder kleiner Lifecycle-Sink an `cleanup_lane` oder Reconcile, falls Events/Cache fehlen.
- Persistenz fehlt: vorhandene Voice-Tabellen enthalten `voice.router_user_prefs`, `voice.tempvoice_interface` und `voice.tempvoice_lanes` (`rust/crates/dl-central-db/migrations/0005_voice.sql:66-133`), aber keine Lane<->Forum-Post-Mapping-Tabelle. Neue Migration noetig, z. B. `voice.lfg_posts` mit `guild_id`, `forum_channel_id`, `thread_id UNIQUE`, `starter_message_id NULL`, `lane_id UNIQUE NULL`, `owner_id`, `mode`, `rank_min`, `rank_max`, `requested_slots`, `status`, `created_at`, `updated_at`, `expires_at`, `closed_at`, `last_render_hash`, `last_post_edit_at`. Kein hartes FK auf `voice.tempvoice_lanes`, weil `cleanup_lane` den Lane-Datensatz vor dem Discord-Thread-Close loescht (`rust/crates/dl-voice/src/tempvoice/engine.rs:1641-1649`).
- Datenschutz-Vertrag nachziehen: Privacy kennt Rank-History-Visibility (`rust/crates/dl-community/src/privacy.rs:318-336`) und Voice-Owner-/Lane-User-Spalten (`rust/crates/dl-community/src/privacy.rs:536-568`). Neue `owner_id`/eventuelle Join-User-Spalten in LFG muessen dort erfasst oder bewusst vermieden werden.
- Post-Edits duerfen nicht pro VoiceEvent direkt rausgehen. Es gibt ein passendes lokales Muster: `rename_queue` ist last-wins, ein Worker, persistent, mit Mindestabstand/Backoff (`rust/crates/dl-voice/src/rename_queue.rs:1-12`, `rust/crates/dl-voice/src/rename_queue.rs:221-282`). LFG braucht analog eine coalescende Edit-Queue mit Render-Hash, pro Post Debounce und 429-Backoff. Serenity kann Message-Edits (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/http/client.rs:1954-1988`) sowie Thread-Archive/Lock (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/builder/edit_thread.rs:12-31`, `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/model/channel/channel_id.rs:902-912`, `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/http/client.rs:2277-2295`).
- Starter-Message-ID ist eine offene API-Frage: `CreateForumPost` liefert `GuildChannel`; dessen `id` ist sicher der Thread/Post, `last_message_id` ist laut Serenity-Kommentar nur fuer Textkanaele (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serenity-0.12.5/src/model/channel/guild_channel.rs:44-72`). Design: `thread_id` sofort speichern; `starter_message_id` nur speichern, wenn vorhanden oder nach kontrolliertem Fetch.
- "Beitreten"-Button: bestehendes `smart_route` ist nicht direkt passend, weil es irgendeine passende Lane waehlt (`rust/crates/dl-voice/src/router.rs:890-928`). W3.4b braucht targeted join: `lfg_join:<post_id>` -> Mapping laden -> Lane validieren -> Cap/Rang pruefen -> `move_member`. Der Move-Pfad existiert in `RouterPort` (`rust/crates/dl-voice/src/router.rs:385-393`) und Glue nutzt Discord `edit_member(channel_id)` (`rust/crates/dl-voice/src/glue.rs:387-405`). Freie Plaetze kommen aus Cache-Members/User-Limit (`rust/crates/dl-voice/src/glue.rs:658-665`, `rust/crates/dl-voice/src/glue.rs:704-714`) mit Modus-Fallbacks aus TempVoice-Logik (`rust/crates/dl-voice/src/tempvoice/logic.rs:21-23`) bzw. Street-Brawl-Cap 4 (`rust/crates/dl-voice/src/tempvoice/engine.rs:789`).
- Auto-Close: wenn Lane stirbt, Thread per `EditThread::archived(true).locked(true)` schliessen; fuer Posts ohne Lane `expires_at=created_at+24h` und Scheduler/Ticker im LFG-Service. Bestehende Worker-Loop-Muster gibt es in TempVoice-Purge (`rust/crates/dl-voice/src/tempvoice/engine.rs:1799-1810`) und Rename-Queue (`rust/crates/dl-voice/src/rename_queue.rs:221-282`).

## 3. Zwei Erstellwege
- Weg A, Forum-Formular: Panel im LFG-Forum -> Modus-Select/Button -> Text-Modal fuer Rangbereich/freie Plaetze -> Bot validiert -> `CreateForumPost`. Danach ephemerer Ergebnis-Button "Lane gleich aufmachen" ruft den bestehenden Router-Spawn-Pfad, wenn der User gerade in Voice ist. `spawn_lane_from_current_voice` hat genau diese Semantik und blockt `NotInVoice`, `AlreadyOwnLane`, Cooldown und Ranked ohne Rang (`rust/crates/dl-voice/src/router.rs:807-851`).
- Weg B, aus eigener Lane: bestehende Lane-Verwaltung sitzt im TempVoice-Panel. `TempVoiceInterface` kann globale Panel-Messages posten/editen (`rust/crates/dl-voice/src/tempvoice/interface.rs:42-84`), der Handler findet die aktuelle verwaltete Lane (`rust/crates/dl-voice/src/tempvoice/interface.rs:476-490`) und `owned_lane_of` erzwingt Owner oder Manage-Channels (`rust/crates/dl-voice/src/tempvoice/interface.rs:492-501`). Die registrierten `tv_*`-IDs zeigen den bestehenden Panel-Umfang (`rust/crates/dl-voice/src/tempvoice/interface.rs:1260-1302`). Design: neuen Button `lfg_publish_lane` im TempVoice-Panel, Guard ueber `owned_lane_of`, danach Post mit vorhandener `lane_id`.
- Router-Panel zeigt ebenfalls TempVoice-Verwaltungsbuttons im Router-Kanal (`rust/crates/dl-voice/src/router.rs:262-320`). Fuer "aus eigener Lane" ist das technisch nutzbar, aber UX-seitig bleibt der Guard die eigene aktuelle Lane, nicht die Message-Position.

## 4. Ranked-Gate
- Konzept fordert Ranked-Gate nur fuer Ranked; Casual/Street Brawl offen (`docs/onboarding-redesign/welle3-modernisierung-konzept.md:60-64`).
- Wiederverwendungspunkt ist `VERIFIED_RANK_ROLE_IDS` mit 11 Haupt-Rangrollen (`rust/crates/dl-voice/src/router.rs:173-186`). Der Router nutzt dieselbe Pruefung bereits beim Button-Spawn (`rust/crates/dl-voice/src/router.rs:831-839`) und beim Auto-/Smart-Route (`rust/crates/dl-voice/src/router.rs:892-905`). LFG-Formular und `lfg_join` muessen diese Pruefung fuer `mode=ranked` identisch anwenden; Casual und Street Brawl duerfen nicht daran haengen.
- Rang-Parsing fuer Bereichsvalidierung sollte nicht den alten LFG-Code duplizieren, sondern `tempvoice::logic` verwenden: `RANK_ORDER`, `rank_index`, `rank_score`, `member_rank_index` existieren bereits (`rust/crates/dl-voice/src/tempvoice/logic.rs:4-92`).

## 5. Leaderboard oeffentlich
- Website-Repo ist bereits vorbereitet: `/aktivitaet/` enthaelt eine `Rang-Leaderboard`-Card mit Sortierung (`/home/naniadm/Documents/Website/dl-activity/index.html:131-144`). Frontend ruft `/api/public/leaderboard/rank?sort=...&days=30&limit=50` (`/home/naniadm/Documents/Website/dl-activity/src/activity.js:452-466`).
- Opt-in ist vorhanden: die Website bietet `private`, `members`, `public` und weist darauf hin, dass Members/Public ins Rang-Leaderboard fuehren (`/home/naniadm/Documents/Website/dl-activity/index.html:311-335`); Save laeuft ueber `PUT /api/public/me/rank-visibility` (`/home/naniadm/Documents/Website/dl-activity/src/activity.js:999-1018`). Backend-Routen existieren (`rust/crates/dl-stats/src/lib.rs:95-112`), Migration defaultet auf `private` (`rust/crates/dl-central-db/migrations/2026070260_rank_history_visibility.sql:1-14`), und Backend filtert Leaderboard-Users zuerst ueber Sichtbarkeit (`rust/crates/dl-stats/src/rank_history.rs:180-190`, `rust/crates/dl-stats/src/rank_history.rs:402-426`).
- Deadlock-Bots-Doku bestaetigt das Portal als oeffentlichen Stats-/Leaderboard-Bereich (`/home/naniadm/Documents/Deadlock-Bots/docs/website-portale.md:18-30`) und Datenschutzgrenzen fuer oeffentliche Leaderboards (`/home/naniadm/Documents/Deadlock-Bots/docs/stats-und-privacy.md:28-36`). Die Landing verlinkt Mitspieler und Aktivitaet bereits (`/home/naniadm/Documents/Website/deco-elevator-new/index.html:146-156`, `/home/naniadm/Documents/Website/dl-landing/index.html:433`).
- Optionen:
  - Empfehlung: Website-Link im LFG-Forum-Header/Panel auf `/aktivitaet/`, optional spaeter mit Anchor `#rank-leaderboard-card`. Aufwand klein, respektiert vorhandenes Opt-in, keine doppelte Datenhaltung, keine Discord-Rate-Limits.
  - Alternative: Bot publisht regelmaessig ein Rank-Snapshot in Discord. Aufwand mittel: Scheduler, Render/Message-Edit, Sichtbarkeitsfilter erneut beachten, Rate-Limits. Kein Muss fuer W3.4b Phase 1.

## 6. Wellen-DAG
- W1 Struktur-Delta + LFG-Panel-Shell
  - Scope: `rules.rs`/Desired-Modell kann `🎯mitspieler-suche` als neues Forum modellieren; alter Textkanal wird umbenannt/archiviert/versteckt oder explizit aus dem Matching genommen; Forum-Rechte verhindern freie User-Posts; LFG-Panel-Publisher als Dry-Run/Apply-Pfad analog Router.
  - Dependencies: W3.4a Router-Panel/ServerSync-Muster; Owner-Test-Punkt vor Live-Apply.
  - Definition of Done: Preview zeigt neues Forum statt Typ-Update; alter Textkanal bleibt nachvollziehbar; @everyone kann nicht frei posten/Threads erstellen; Panel payload ist testbar.
  - Testbarkeit ohne Live-API: `cargo test -p dl-server-as-code`, Payload-/Diff-Tests fuer Forum-Kind und Rechte, ServerSync-Dry-Run-Snapshot. Owner-Test: Live-Dry-Run prueft explizit, dass kein Text->Forum-Update geplant ist.
- W2 LFG-Persistenz + Formular -> Forum-Post
  - Scope: Migration `voice.lfg_posts`; LFG-Service/Port fuer Forum-Post-Erstellung; Modus-Select + Text-Modal; Ranked-Gate; alte Text-LFG-Responder deaktivieren oder aus Cutover entfernen; Privacy-Vertrag erweitern.
  - Dependencies: W1 Forum vorhanden, InteractionRouter/ModalSpec.
  - Definition of Done: User koennen nicht frei posten; Bot erstellt validierten Forum-Post; Ranked ohne Rang wird ephemer blockiert; `thread_id`/optional `starter_message_id` persistiert; Posts ohne Lane bekommen 24h-Expiry.
  - Testbarkeit ohne Live-API: SQL-Migration/Fresh-Schema, Unit-Tests fuer Formularvalidierung/Rangrange, Mock-Port fuer `CreateForumPost`, Privacy-Vertragstest.
- W3 Live-Kopplung + Beitreten + Auto-Close
  - Scope: Mapping `lane_id` <-> `thread_id`; VoiceEvent/Reconcile-Worker; debounce Edit-Queue; `lfg_join:<post_id>` targeted move; Live-Render freie Plaetze/voll; Thread archivieren/locken bei Lane-Tod; 24h-Fallback-Expiry fuer lane-lose Posts.
  - Dependencies: W2 Persistenz/Post-IDs; RouterPort/TempVoice-Engine; Entscheidung ob Lifecycle-Sink oder Reconcile-only.
  - Definition of Done: Post aktualisiert nur bei Render-Diff; Join moved in exakt die gemappte Lane und prueft Cap/Rank; Lane-Tod schliesst Post; Rate-Limit-Backoff verhindert Edit-Sturm.
  - Testbarkeit ohne Live-API: Fake Dispatcher VoiceEvents, Fake Clock fuer Debounce/Expiry, Mock Discord-Port fuer EditMessage/EditThread/MoveMember, Reconcile-Test fuer geloeschte Lane.
- W4 Leaderboard-Public-Rollout
  - Scope: Link/Copy im LFG-Forum-Panel auf bestehendes Aktivitaetsportal; optional Website-Anchor fuer Rang-Leaderboard. Kein Bot-Snapshot in Phase 1 empfohlen.
  - Dependencies: W1 Panel-Publisher; Website-Owner-Entscheid fuer Anchor.
  - Definition of Done: LFG-Forum verweist sichtbar auf das oeffentliche Rang-Leaderboard und erklaert Opt-in knapp; keine neue Discord-Sync-Quelle fuer Rangdaten.
  - Testbarkeit ohne Live-API: Payload-Snapshot-Test fuer Link; Website nur read-only in dieser Analyse, ggf. separate Website-Welle fuer Anchor.

## Offene Fragen / Owner-Entscheide
- Soll das LFG-Forum Thread-Antworten von Usern erlauben, oder bleiben auch Antworten in Bot-Posts gesperrt und Kommunikation laeuft nur ueber Voice?
- Struktur-Cutover: alter Textkanal archivieren/verstecken mit welchem Namen, und soll History erhalten sichtbar bleiben?
- Forum-Metadaten: Default-Archive-Dauer, Sortierung, Slowmode, Tags bewusst leer? Konzept sagt keine Tag-Pillen, aber Discord-Forum kann trotzdem Defaults brauchen.
- Starter-Message-ID: nach Forum-Post-Create per `last_message_id` versuchen oder explizit erste Message fetchen? Serenity-Kommentar macht das offen.
- "Lane gleich aufmachen": nur erlauben, wenn User aktuell in Voice ist, oder soll der Button zum Router-VC verweisen?
- Freie Plaetze bei `user_limit=0`: als "offen" anzeigen oder Modus-Default nutzen? Empfehlung: Modus-Default fuer LFG, weil Post sonst nicht sinnvoll "voll" werden kann.
- Lifecycle-Integration: minimaler LFG-Reconcile-Worker reicht, oder soll TempVoice einen kleinen expliziten Lifecycle-Sink fuer `lane_created/lane_deleted` bekommen?
- Website: braucht `/aktivitaet/` einen stabilen Anchor zum Rang-Leaderboard fuer den Forum-Link?

## Risiken
- Discord erlaubt keine Text->Forum-Konversion als normales Update; unser Apply sendet `type` nur bei Create. Falsches Matching koennte sonst einen wirkungslosen oder fehlerhaften Update-Diff erzeugen.
- Rechtefehler koennen das zentrale Produktversprechen brechen: @everyone hat heute Thread-/Send-Rechte aus der Basis, daher braucht das Forum explizite Denies und einen Owner-Dry-Run.
- Live-Updates koennen Rate-Limits treffen. Nur coalescende Queue mit Render-Hash, Debounce und 429-Backoff verwenden.
- Cache-Staleness bei Voice-Mitgliedern/User-Limits kann freie Plaetze kurz falsch rendern; Reconcile und Update-Debounce muessen das ausgleichen.
- Old-LFG-Responder kann mit dem neuen No-Free-Post-Modell kollidieren, wenn er nicht deaktiviert wird.
- LFG-Persistenz speichert User-IDs; Privacy-Vertrag und Export/Erase muessen vor Rollout nachgezogen werden.

# W3.4a Router-Flow + TempVoice-Panel (2026-07-03)

## Ziel
Router-Auswahl-Flow im Kanal `deadlock-router` als idempotentes Components-V2-Panel plus TempVoice-Verwaltungs-Panel umsetzen. Keine Strukturänderungen in `rules.rs`, keine Live-Discord-Calls, kein Commit/Push.

## Phase 1 Ist-Analyse
- Konzept C/D gelesen: LFG v2 bleibt Folgebaustein; Router-Flow spawnt Lanes in Modus-Kategorien und soll die Creator erst nach Owner-Test ersetzen.
- TempVoice-Spawn: `TempVoiceEngine` verarbeitet Join-Events auf Staging-IDs Casual `1501089974093873232`, Ranked `1412804671432818890`, Street Brawl `1357422958544420944`; `create_lane_inner` erstellt Voice-Kanal, persistiert `voice.tempvoice_lanes`, moved den User und wendet Owner-Settings an. Zusätzlich existiert `create_router_lane` für Router-Lanes.
- Modi aus bestehender Engine: `casual`/Chill -> Kategorie `1289721245281292290`, `ranked` -> `1412804540994162789`, `street_brawl` -> `1357422957017698478`. Neue-Spieler ist nur Adaptive-Routing-Hook, Custom Game hat keine bestehende TempVoice-Spawn-Regel; deshalb kein neuer Button ohne separate Engine-Erweiterung.
- TempVoice-Verwaltung: bestehende `tv_*`-Handler bieten u. a. Region DE/EU, Owner Claim, Limit, Kick, Ban, Unban, Duo/Trio/Reset, Presets, Rang-Präferenz, Rename, Lurker, Moduswechsel, Min-Rank und Tag-Filter. Guards laufen über `lane_of`/`owned_lane_of`; Fehler ohne Lane/ohne Owner sind ephemer.
- `rules.rs`: Kategorien/DynamicNamespaces enthalten Chill, Deadlock Router und Street Brawl als `tempvoice_*`; alte Creator-/Join-Trigger-Kanäle bleiben im Modell. Auftrag bestätigt: keine `rules.rs`-Strukturänderung.
- `welcome_publish.rs`: Components V2 nutzt `flags=32768`, Container `accent_color=13150315`, `allowed_mentions.parse=[]`, KV-Message-ID-Storage und HTTP-Apply über `/serversync/welcome-apply`. Router übernimmt dieses Muster, aber ohne Welcome-Textloader.

## Fortschritt
- Hotfix Router-Attachment-Multipart gestartet: Ursache in Serenity-`CreateAttachment.id`/`files[0]` bestaetigt; Umsetzung scoped auf `dl-voice`-Transportpfad ohne Router-Payload-Aenderungen.
- Hotfix umgesetzt: `RouterGlue::post_rich/edit_rich` nutzt jetzt reqwest-Multipart mit `payload_json` und `files[{attachment.id}]`; Token kommt direkt aus `serenity::Http::token()` inklusive Scheme. Verifikation gruen: `SQLX_OFFLINE=true cargo build --workspace`; `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`; `./scripts/central_test_db.sh cargo test -p dl-voice`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `git diff --check`.
- Rework nach Kritiker-Review gestartet: Scope strikt auf Router-Spawn-Spam/Lane-Hopping, History-Adoption über `router_spawn_`-IDs und Legacy-Panel-Cleanup. Kein Commit/Push; bestehender W3.4a-Diff bleibt erhalten.
- Rework umgesetzt: `router_spawn_*` blockt Owner in eigener TempVoice-Lane mit neuem `AlreadyOwnLane`-Outcome und setzt pro User einen 30s-In-Memory-Cooldown nur nach `Created`; Cooldown-Klicks liefern `Cooldown`.
- Rework umgesetzt: `RouterPanelMessage` enthält rekursiv extrahierte `custom_ids`; History-Adoption matcht nur noch V2-Messages ohne Embeds mit `router_spawn_`-Custom-ID.
- Rework umgesetzt: `apply_panel(confirm=true)` löscht nach erfolgreichem Apply Legacy-Panel-Messages aus `guide_message_id`/`interface_message_id` und entfernt die KV-Keys; Dry-Run kündigt geplante Löschungen nur als Warnings an.
- Ist-Analyse abgeschlossen; Umsetzung startet scoped in `dl-voice` Router/TempVoice-Engine und `dl-bot` ServerSync-HTTP.
- Implementiert: Router-Panel als eine Components-V2-Message im `deadlock-router`-Kanal mit `flags=32768`, Gold-Container, leerem `allowed_mentions.parse`, KV-Message-ID und Payload-Format-Key. Zweiter Containerabschnitt im selben Panel nutzt bestehende `tv_*`-Verwaltungsaktionen.
- Implementiert: neue `router_spawn_*`-Buttons erstellen per bestehender `TempVoiceEngine::create_router_lane` Lanes fuer Casual/Ranked/Street Brawl aus dem aktuellen Voice-State des klickenden Users; User ohne Voice bzw. Ranked ohne Rang erhalten ephemere Antworten. Alte `router_mode_*`-/Autojoin-Interaktionen bleiben registriert.
- Implementiert: interner HTTP-Trigger `POST /serversync/router-apply` mit Dry-Run-Default analog Welcome-Apply; `ServerSyncService` verdrahtet das bestehende `RouterInterface`.
- Platzhaltertexte liegen in `rust/crates/dl-voice/src/router.rs` als `Platzhalter: ...`-Konstanten; Claude ersetzt final.
- Verifikation bisher: `SQLX_OFFLINE=true cargo build --workspace` gruen; `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot` gruen; `./scripts/central_test_db.sh cargo test -p dl-voice` gruen; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings` gruen; `cargo fmt --all -- --check` gruen; `git diff --check` gruen.
- Rework-Verifikation gruen: `./scripts/central_test_db.sh cargo test -p dl-voice router_ -- --nocapture`; `SQLX_OFFLINE=true cargo build --workspace`; `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`; `./scripts/central_test_db.sh cargo test -p dl-voice`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `git diff --check`.
- Hinweis: direkte DB-Harness-Tests ohne `CENTRAL_TEST_DSN`/`DATABASE_URL` schlagen in bestehenden `dl-bot`/`dl-voice`-Tests fehl; mit dem Projekt-Wrapper laufen sie gruen.
- W3.4a-Nachtrag gestartet: Router-Panel nutzt jetzt die fertigen Banner aus `assets/welcome-banners/` als Components-V2-Media-Galleries und fuehrt Attachment-Metadaten analog Welcome-Payload. Erste Zieltests gruen: `SQLX_OFFLINE=true cargo test -p dl-voice router_panel_body_ist_components_v2_mit_router_und_tv_buttons -- --nocapture`; `./scripts/central_test_db.sh cargo test -p dl-voice router_interface_ -- --nocapture`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot router_apply_http_ist_dry_run_default_und_liefert_v2_payload -- --nocapture`; `SQLX_OFFLINE=true cargo check -p dl-voice`.
- W3.4a-Nachtrag abgeschlossen: Router-Apply validiert Banner vor POST/PATCH, uebergibt dieselben Attachments bei neuem Post und Edit erneut an die Glue-Schicht; Dry-Run zeigt nur Payload/Attachment-Metadaten. Verifikation gruen: `SQLX_OFFLINE=true cargo build --workspace`; `./scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot`; `./scripts/central_test_db.sh cargo test -p dl-voice`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `git diff --check`.

# W3.2e Components-V2-Hub (2026-07-03)

## Ziel
Welcome-Hub von Embeds auf Discord Components V2 umbauen, Team-Dedupe ergaenzen, Bot als Team-Mitglied darstellen und Storage fuer Message-Listen/Repost-Semantik erweitern. Kein Commit/Push; keine Live-Discord-Calls durch diesen Worker ausserhalb bestehender Codepfade/Tests.

## Fortschritt
- Implementierungsworker gestartet im Worktree `Deadlock-Bots-hub-polish`; Pflichtkontext `WORKFLOW.md`, `welcome_publish.rs` und Welcome-Preview-/Apply-Pfad inklusive rohem `payload_json`-Multipart gelesen.
- Befund: Renderer ist noch Embed-basiert; Apply patcht pro Sektion anhand eines einzelnen KV-Keys und nutzt Marker-/Titel-Fallback fuer alte Embeds. REST-Pfad serialisiert aber bereits kontrolliertes JSON und Multipart-Dateien, daher fuer Components V2 geeignet.
- Umsetzungsplan: Payload-Struct auf `flags=32768`, `allowed_mentions.parse=[]`, Components/Attachments ohne `content`/`embeds` umbauen; Navigation an Kategoriegrenzen chunkingfaehig machen; KV-Storage auf `welcome_message_id_<section>_<index>` plus `welcome_payload_format=2`; Format-/Count-Wechsel als Delete+Repost mit Quickstart-Neu-Hero-ID behandeln.
- Implementiert im Zwischenstand: Components-V2-Renderer fuer Hero/Navigation/Team/Socials/Quickstart, Navigation-Chunking, Team-Dedupe, Bot-Team-Block aus `[team]` TOML/Defaults, indexierter KV-Storage und Repost/Patch-Entscheidung nach Format+Messageanzahl.
- Zieltests bisher gruen: `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot welcome_publish`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync`.
- Abschluss-Verifikation gruen: `SQLX_OFFLINE=true cargo build --workspace`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync`; `SQLX_OFFLINE=true cargo clippy -p dl-bot --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `git diff --check`. Kein Commit/Push.

# W3.2c Runtime-Texte TOML (2026-07-03)

## Ziel
Welcome-Hub-Texte und Link-URLs zur Publish-Zeit aus `assets/welcome_texts.toml` laden. Kein Commit/Push; Aenderungen bleiben uncommitted fuer Claude-Review.

## Fortschritt
- Implementierungsworker gestartet; Pflichtkontext `WORKFLOW.md`, `welcome_publish.rs`, Welcome-Preview-/Apply-Pfade und vorhandene Tests gelesen.
- Befund: Payloads nutzen bisher direkt `WELCOME_TEXTS`/`WELCOME_LINK_URLS`; `toml` ist im Workspace vorhanden, aber `dl-bot` referenziert es noch nicht.
- Implementiert: `assets/welcome_texts.toml` als Seed mit heutigen Texten, Runtime-Loader mit Missing-File-Fallback/Warnung, hartem Parse-Fehler und Merge-Semantik; Welcome-Payloads verwenden die geladene Konfiguration.
- Neue Unit-Tests fuer Seed-Parse gegen Defaults, Parse-Fehler, fehlende Datei und Teil-Merge sind gruen (`SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot welcome_texts`).
- Abschluss-Verifikation gruen: `SQLX_OFFLINE=true cargo build --workspace`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync`; `SQLX_OFFLINE=true cargo clippy -p dl-bot --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `git diff --check`. Kein Commit/Push.

# W3.2b Hub-Polish + Struktur-Delta (2026-07-03)

## Ziel
Owner-Feedback fuer Welcome-Hub und Soll-Modell v2 umsetzen. Kein Commit/Push; Aenderungen bleiben uncommitted fuer Claude-Review.

## Fortschritt
- Implementierungsworker gestartet im Worktree `Deadlock-Bots-hub-polish` auf Branch `feat/welle3-hub-polish`. Pflichtkontext gelesen: `WORKFLOW.md`, Welle-3-Konzept, `rules.rs`, `welcome_publish.rs` und Welcome-Apply-Pfad in `serversync.rs`.
- Befund: Soll-Modell nutzt dokumentierte Rename-/Move-Tabellen und Welle2b-Archiv-Deny-Muster; Welcome-Hub setzt sichtbare Marker zentral ueber Embed-Footer und sendet bisher maximal ein Banner-Attachment pro Message.
- Soll-Modell-Deltas umgesetzt: NEWS-Kanaele nach INFORMATION, `deadlock-invite` nach COMMUNITY, `twitch` -> `deadlock-streamer`, leere NEWS-Kategorie entfernt, Legacy-Lane-Hilfskanaele mit Archiv-Deny ins Archiv. `SQLX_OFFLINE=true cargo test -p dl-server-as-code` gruen.
- Welcome-Hub-Deltas umgesetzt: markerlose Embeds, Divider-Attachments, Team-Ausschluss/Mehrfach-Aliase, Quickstart-Jump und Navigation-Filtertests. `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot welcome_publish` gruen.
- Abschluss-Verifikation gruen: `SQLX_OFFLINE=true cargo build --workspace`; `cargo test -p dl-server-as-code`; `cargo test -p dl-bot --bin dl-bot serversync`; `./scripts/central_test_db.sh cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `git diff --check`. Kein Commit/Push.

# Rank-History Backend dl-stats (2026-07-03)

## Ziel
Rank-History-Migration und dl-stats-API fuer `/aktivitaet/api/*` implementieren. Kein Commit/Push und keine Git-Kommandos durch diesen Worker.

## Fortschritt
- Rework nach Kritiker-Pass gestartet: Scope strikt auf `dl-stats` Rank-History, `dl-central-etl` Replay-Mapping, Migration/Fresh-Schema und SQLx-Cache. Keine Git-Kommandos, kein Commit/Push.
- Rework umgesetzt: Membership ist jetzt exakt "neuestes Guild-member_event ist join"; `unban`/alle anderen neuesten Events zaehlen nicht. `days=0` ist dokumentiert und getestet als All-Time, Days werden bis 3650 geklemmt, negative/nicht-numerische Werte liefern 400.
- Rework umgesetzt: Rank-Leaderboard filtert Sichtbarkeit zuerst und holt current/baseline per LATERAL aus SQL; History-Tie-Breaker nutzen konsistent `badge_level DESC` bei gleichem `captured_at`. Migration `2026070260_rank_history_visibility.sql` enthaelt den partial covering Index auf `steam.steam_rank_history`.
- Rework umgesetzt: `dl-central-etl` Steam-Rank-History-Testplan mappt `badge_level` und bildet den Inventory-Typ `captured_at INTEGER -> timestamptz` ab; Ledger hatte `badge_level` bereits gemappt.
- EXPLAIN-Beleg gegen Wegwerf-DB mit 105000 History-Zeilen: Leaderboard-Plan nutzt fuer current, oldest baseline und cutoff baseline jeweils `Index Only Scan using steam_rank_history_user_captured_visible_cover_idx`; kein Full-Read von `steam_rank_history` im sichtbaren Teil.
- Rework-Verifikation gruen: `SQLX_OFFLINE=true cargo test -p dl-stats --features testing`; `./scripts/central_test_db.sh cargo test -p dl-stats --features testing -- --include-ignored`; `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored`; `cargo test -p dl-central-etl`; `cargo fmt -p dl-stats -p dl-central-etl -p dl-central-db -- --check`; `SQLX_OFFLINE=true cargo clippy -p dl-stats -p dl-central-db --all-targets --features testing -- -D warnings`; `SQLX_OFFLINE=true cargo clippy -p dl-central-etl --all-targets -- -D warnings`.
- Pflichtkontext gelesen: `dl-stats/src/lib.rs`, `me.rs`, `public.rs`, `ranks.rs`, zentrale Migrationen und `fresh_migrations_schema.rs`.
- Befund: naechste freie Migration ist `2026070260`; Display-Namen werden wie Voice/Text-Leaderboards ueber `activity.member_events` aufgeloest.
- Membership-Befund: keine dedizierte persistente Current-Member-Tabelle gefunden; `activity.member_events` enthaelt Join/Leave/Backfill-Events und wird als konservative DB-Quelle fuer aktuelle Mitgliedschaft geprueft/weiterverwendet, falls eindeutig genug.
- TDD-Red: `SQLX_OFFLINE=true cargo test -p dl-stats --features testing rank_history` scheitert erwartungsgemaess an fehlender `not_found_response`/Stub-Logik.
- Implementiert: Migration `2026070260_rank_history_visibility.sql`, `dl-stats::rank_history` mit Me-History, Visibility-Upsert, Rank-Leaderboard und zieluserbezogener History. IDs werden in JSON als Strings ausgegeben.
- SQLx-Cache fuer `dl-stats` gegen Wegwerf-Postgres aktualisiert; erster DB-Lauf zeigte stale `sqlx::migrate!`-Artefakte, danach `cargo clean -p dl-central-db -p dl-central-migrate` und erneuter Lauf gruen.
- Zieltests gruen: `SQLX_OFFLINE=true cargo test -p dl-stats --features testing rank_history` (4 passed, 2 ignored) und `./scripts/central_test_db.sh cargo test -p dl-stats --features testing rank_history -- --include-ignored` (6 passed).
- Privacy-Vertrag nachgezogen: `steam.rank_history_visibility` ist in `dl-community::privacy::USER_TABLES` aufgenommen; Vertragstest gruen.
- Abschluss-Verifikation gruen: `cargo fmt -p dl-stats -p dl-central-db -p dl-community -- --check`; `SQLX_OFFLINE=true cargo clippy -p dl-stats -p dl-central-db --all-targets --features testing -- -D warnings`; `SQLX_OFFLINE=true cargo clippy -p dl-community --lib -- -D warnings`; `SQLX_OFFLINE=true cargo test -p dl-stats --features testing`; `SQLX_OFFLINE=true cargo test -p dl-central-db --features testing`; `./scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored`; `./scripts/central_test_db.sh cargo test -p dl-stats --features testing -- --include-ignored`; `SQLX_OFFLINE=true cargo test -p dl-community privacy_contract_tests::alle_migration_user_id_spalten_sind_im_privacy_vertrag`.
- Hinweis: `SQLX_OFFLINE=true cargo clippy -p dl-stats -p dl-central-db -p dl-community --all-targets --features testing -- -D warnings` scheitert an einem bestehenden fehlenden SQLx-Cache fuer `dl-community/src/privacy.rs` Test-Query bei Zeile 1822; nicht durch die Rank-History-Aenderung verursacht.

# Invite-Lounge-Watcher deadlock-invite (2026-07-02)

## Ziel
Neuer Rust-Message-Listener in `dl-community` fuer den offenen Kanal `#deadlock-invite`: bei Invite-Frage ohne Steam-Freundescode einmalig pro User/24h per Text-Reply hinweisen. Kein Commit/Push; vorhandene uncommitted Aenderungen bleiben erhalten.

## Fortschritt
- Workflow gelesen und Worktree-Status geprueft; bestehende fremde Aenderungen u. a. in Docs, `dl-bridges::steam` und `dl-community::faq.rs` werden nicht reverted.
- Kanal-ID-Befund: `cogs/onboarding.py` enthaelt `CH_BETA = 1428745737323155679`; `onboarding_steps.json` referenziert denselben Kanal als Invite-/Zugangskanal; Server-as-Code/Konzept dokumentieren Rename `beta-zugang` -> `deadlock-invite`. Daher feste Konstante statt Env-Var.
- Implementiert: neues Modul `dl_community::invite_lounge` mit Dispatcher-Subscriber, deterministischer Invite-/Freundescode-Heuristik, Text-Reply-Port ohne Embeds/Buttons und KV-Cooldown `invite_lounge:cooldown/user:<id>` fuer 24h.
- Verdrahtet: `rust/bin/dl-bot/src/main.rs` startet den Watcher im gateway-gated Message-Listener-Block.
- Verifikation gruen: `cargo fmt --check -p dl-community -p dl-bot`; `cargo check -p dl-community`; `cargo test -p dl-community` (51 passed); Zusatz wegen `main.rs`-Wiring: `cargo check -p dl-bot`.
# Welle2b W1 Onboarding-Fundament (2026-07-02)

## Welle2b W3 Incident-Hardening §0 (2026-07-03)
- Rework-Implementierungsworker fuer Guard-Befunde 1-5 gestartet. Verbindlich bestaetigt: nur Worktree `Deadlock-Bots-welle2b`, kein Checkout `main`, kein Push, kein Commit durch diesen Worker; `CHANGELOG.md` bleibt unberuehrt.
- TDD-Start: Regressions-Tests werden fuer effektive Rechte/Member-Deny-Deletes, Archiv-Namespace-Materialisierung, vorhandene Archiv-Kinder und DB-basierte Preview-Freshness ergaenzt.
- Rework umgesetzt: Effektiv-Rechte-Guard vergleicht normalisierte Vorher/Nachher-Bitmasken (`kein VIEW_CHANNEL => 0`) und blockt nur Oeffnungen; Member-Deny-Deletes blocken konservativ ohne Member-Rollendaten. Archiv-Kandidaten umgehen den faq-/Scrim-Namespace-Filter, vorhandene Archiv-Kinder materialisieren eigene `@everyone -VIEW`-Denies.
- Freshness-Rework umgesetzt: Apply-, Onboarding-Apply- und Serverguide-Apply-Pfade verwenden DB-berechnete Preview-Ablauf-Flags statt Prozesszeit.
- Rework-Verifikation gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync`; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored`; `./scripts/central_test_db.sh cargo test -p dl-bot --bin dl-bot serversync::tests::onboarding_preview_load_bindet_guild_message_key_und_applied_status -- --ignored --nocapture`; `cargo fmt --all`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test --workspace`; `git diff --check`. Kein Commit/Push.
- Implementierungsworker gestartet im Worktree `Deadlock-Bots-welle2b` auf Branch `feat/welle2b-onboarding`. Verbindlich: kein Checkout von `main`, kein Push, kein Commit durch diesen Worker; `CHANGELOG.md` bleibt unberuehrt.
- Pflichtkontext gelesen: `docs/onboarding-redesign/phase1-rechte-soll-modell.ENTWURF.md` §0 Incident-Lehren Welle 2a. Umsetzung startet TDD-fokussiert in `dl-server-as-code` Diff/Rules/Apply und `dl-bot` Serversync-Freshness.
- Implementiert: Archivkanaele materialisieren eigene `@everyone -VIEW`-Overwrites; Diff hat effektive-Rechte-Guard mit geblockten Preview-Eintraegen; faq-/Coaching-Scrim-Namespaces und Owner-Overrides fuer Sammelpunkt/Coaching-Kategorie sind ergaenzt.
- Implementiert: Apply lehnt Previews aelter als 15 Minuten ab, sortiert Sichtbarkeits-Denies vor Overwrite-Deletes und behandelt 404/Gone als Skip+Report statt Laufabbruch. Onboarding-/Serverguide-Apply pruefen dieselbe Preview-Freshness.
- Verifikation gruen: `cargo fmt --all`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test -p dl-server-as-code --features testing -- --include-ignored`; `./scripts/central_test_db.sh cargo test -p dl-bot --bin dl-bot serversync::tests::onboarding_preview_load_bindet_guild_message_key_und_applied_status -- --ignored --nocapture`; `./scripts/central_test_db.sh cargo test --workspace`; `git diff --check`. Kein Commit/Push.

## Welle2b W2 Kanal-Sanierung + Regelwerk + Server Guide (2026-07-03)
- Implementierungsworker gestartet im Worktree `Deadlock-Bots-welle2b` auf Branch `feat/welle2b-onboarding`. Verbindlich: kein Checkout von `main`, kein Push, kein Commit durch diesen Worker; `CHANGELOG.md` bleibt unberuehrt.
- Rework-Implementierungsworker fuer 3 Kritiker-Befunde gestartet: Server-Guide-DTO `title`/`description`, fester CHAT-action_type-Wert und 429-Retry fuer Regelwerk-REST. Verbindlich: TDD, kein Commit/Push durch diesen Worker.
- Rework umgesetzt im Zwischenstand: Server-Guide-PUT/GET-DTOs nutzen `title` plus `description`, CHAT-Actions nutzen fest `SERVER_GUIDE_ACTION_TYPE_CHAT = 1`, Regelwerk-REST-Calls retryen HTTP 429 mit `retry_after`/`Retry-After` und Mass-Deletes pausieren 350ms. Zieltest `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serversync::tests` gruen.
- Abschluss-Verifikation gruen: `cargo fmt --all`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test --workspace`; `git diff --check`. Kein Commit/Push.
- Pflichtkontext gelesen: Konzept §3 und §4.6, Ist-Zustand-Kanalbefunde, bestehende `dl-server-as-code::rules` und `rust/bin/dl-bot/src/serversync.rs`. Umsetzung erfolgt ueber bestehendes Preview/Hash/Apply-Muster und KV-Flags in `bot.kv_store`.
- Implementiert im Zwischenstand: `DesiredModelOptions` mit gegatetem Welle2b-Archiv, neue Kategorie `📦 Archiv`, Umzug eindeutiger Archivkandidaten, `kreativ-ecke`, `/serversync archive-enable|archive-disable` plus HTTP; `regelwerk-publish` mit Dry-Run, Thread-/Bot-Message-Cleanup und KV-Message-ID; `serverguide-preview|apply` mit GET/PUT `/new-member-welcome`, Hash-Preview und clientseitiger Rechtevalidierung.
- Zieltests bisher gruen: `SQLX_OFFLINE=true cargo test -p dl-server-as-code welle2b_archivregeln`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot regelwerk`; `SQLX_OFFLINE=true cargo test -p dl-bot --bin dl-bot serverguide`; Archiv-HTTP-/Slash-Einzeltests.
- Abschluss-Verifikation gruen: `cargo fmt --all`; `cargo fmt --all -- --check`; `SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings`; `./scripts/central_test_db.sh cargo test --workspace` aus `rust/`; `git diff --check`. Kein Commit/Push.

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

## Slash /betainvite entfernen (2026-07-02)
- Implementierungsworker gestartet: Scope ist nur der user-facing Slash-Command `/betainvite` in `dl-bridges::steam`; Button-/Prefix-Funnel `betainvite:` und Admin-Panel-Command bleiben erhalten. Kein Commit/Push.
- Implementiert: Slash-Command-Registrierung `betainvite` entfernt; Registrierungstest erwartet weiter den `betainvite:`-Prefix und zaehlt eine Top-Level-Command-Definition weniger.
- Sync-Befund: `dl-discord::dispatch::sync_commands` nutzt Bulk-Overwrite via `create_guild_commands`/`create_global_commands` mit `router.command_definitions()`, daher verschwindet ein entfernter Command beim naechsten aktivierten Sync.
- Verifikation gruen: `cargo check -p dl-bridges`; `cargo test -p dl-bridges router_registrierung_vollstaendig`; `cargo fmt --check -p dl-bridges`.

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
