# CONTRACT: Neue-Spieler-Lanes in die Chill-Kategorie umziehen

## Ziel
Die auto-wachsenden "Neue Spieler Lane"-Voice-Kanäle sollen physisch in der
Chill-Kategorie (`1289721245281292290`) leben und dort nachwachsen. Die
Wachstums-Logik bleibt exakt gleich. Der Temp-Voice-Bot und die Chill-Sortierung
dürfen diese Lanes nie umbenennen, verschieben, löschen oder in ihren Rechten
verändern.

## Kontext (aus EVIDENCE.md)
- Wachstum: `dl-voice/src/adaptive.rs`, scannt heute `NP_TARGET_CATEGORY_ID`
  (`1465839366634209361`), Anker `NP_ANCHOR_CHANNEL_ID = 1470126503252721845`.
- LFG-Scanner `dl-bot/src/modglue.rs::scan_lanes` labelt Lanes nur über die
  Kategorie: Chill wird `Casual`, NP-Kategorie wird `NewPlayer`. Ein reiner
  Umzug würde die Lanes als Casual labeln und das Anfänger-Routing plus 6er-Cap
  brechen.
- Temp-Voice-Engine fasst nur DB-getrackte Lanes an (`voice.tempvoice_lanes`) und
  wird nur von den drei Staging-Kanälen ausgelöst. Der NP-Anker ist keiner davon,
  also fasst Temp-Voice die NP-Lanes ohnehin nicht an.
- `create_voice_channel` (glue.rs:2818) kopiert die Anker-Rechte, user_limit und
  bitrate und legt in der übergebenen Kategorie an, gewachsene Lanes behalten die
  Anfänger-Rechte auch in Chill.
- Chill ist bereits in `status::TARGET_CATEGORY_IDS`, solo_watch überwacht die
  Lanes dort weiter.

## REQ
- REQ1: `adaptive.rs` scannt und erzeugt die Neue-Spieler-Lanes in der
  Chill-Kategorie (`1289721245281292290`) statt in `1465839366634209361`.
  `maybe_route_new_player` findet die Kandidaten-Lanes dort.
- REQ2: `modglue.rs::scan_lanes` labelt einen Voice-Kanal als
  `LaneLabel::NewPlayer` (inkl. 6er-Cap), wenn seine ID gleich
  `NP_ANCHOR_CHANNEL_ID` ist ODER sein Name mit `NP_LANE_BASE_NAME` beginnt,
  unabhängig von der Kategorie. Kategorie-basiertes Labeln bleibt sonst wie
  gehabt.
- REQ3: Die Chill-Sortierung (`sort_tempvoice_category`) fasst NP-Anker und
  NP-Lanes nie an: NP-Anker in die Skip-Liste, NP-benannte Kanäle explizit
  überspringen.
- REQ4: NP-Konstanten aus `dl_voice::adaptive` (`NP_ANCHOR_CHANNEL_ID`,
  `NP_LANE_BASE_NAME`) in modglue wiederverwenden, keine zweite Kopie der IDs.

## INV (dürfen sich nicht ändern)
- INV1: Die Wachstums-Regel selbst (`plan_lanes`, Schwelle 6, Anker plus N, Leere
  löschen) bleibt unverändert.
- INV2: Anfänger-Routing-Regel (`ELIGIBLE_STAGING_IDS`, `resolve_new_player_rank`,
  `NP_MAX_RANK_VALUE`) bleibt unverändert.
- INV3: Gewachsene Lanes erben weiter die Rechte des Ankers (glue.rs bleibt
  unangetastet).
- INV4: Der Temp-Voice-Erstellen-Flow und seine Staging-Kanäle bleiben
  unangetastet.

## Nicht-Ziele
- Kein Umbau des Temp-Voice-Bots.
- Die NP-Kategorie (`1465839366634209361`, hält paten-zentrale, ist auch
  concierge `DEFAULT_PATE_CATEGORY_ID`) wird NICHT gelöscht oder umgebaut. Nur
  der Voice-Anker zieht um.
- solo_watch-DM-Label ("New Player" gegen "Casual") ist rein kosmetisch und
  NICHT Teil dieses Auftrags (separater Follow-up, falls gewünscht).
- Der physische Umzug des Anker-Kanals (parent_id nach Chill) passiert per
  Live-API beim Deploy durch die Hauptsession, NICHT im Code.

## Erlaubter Bereich
- `rust/crates/dl-voice/src/adaptive.rs`
- `rust/bin/dl-bot/src/modglue.rs` (nur `scan_lanes` plus nötige `use`)
- Tests in denselben Dateien.

## Rote Baseline
Kein Bug-Fix, kein Regressionstest zuerst. Neue Tests: (a) ein Kanal in Chill mit
Name `🆕Neue Spieler Lane 2` bekommt `NewPlayer`; (b) adaptive-Sync erzeugt in der
Chill-Kategorie.
