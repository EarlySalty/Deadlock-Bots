# Scrim-Team-Voice-Recording — Implementierungsplan für Codex

**Scope-Hinweis für den Worker:** Halte dich exakt an diesen Auftrag. Ignoriere andere
Doku/Pläne im Repo (z. B. tempvoice-umbau, turnier-automatik) — die betreffen andere Features.

## Ziel

In genau 4 festen Team-Voice-Kanälen der Kategorie „Scrims" können Team-Mitglieder oder
Coaches manuell eine Sprachaufnahme starten/stoppen. Nach dem Stop postet der Bot die
Aufnahme als MP3 in den zugehörigen Team-Text-Kanal.

## Fixe Konfiguration (Guild `1289721245281292288`)

```rust
// (voice_channel_id, team_role_id, team_text_channel_id)
const TEAM_VOICE_CHANNELS: &[(u64, u64, u64)] = &[
    // Shiv-support-voice
    (1521167264533970954, 1521163175318388847, 1521164348976922795),
    // Team-2
    (1521167309828526183, 1521163233245794465, 1521164426424750171),
    // Return Fire Department Voice
    (1521167476711358505, 1521163300480483498, 1521164444787540110),
    // TetraHex Voice
    (1521939025982652416, 1521163334211338391, 1521164466169970728),
];
const COACH_ROLE_ID: u64 = 1494372744286965941;
```

`After-Scrimchannel` (1528447165926604991) ist **nicht** Teil des Scopes.

## Berechtigung

Ein Nutzer darf `/record start` oder `/record stop` für einen der 4 Kanäle nur ausführen,
wenn beide gelten:
1. Er sitzt **gerade selbst** im jeweiligen Voice-Kanal (Ziel wird aus seinem eigenen
   Voice-State abgeleitet, kein Channel-Parameter im Command).
2. Er hat die zu diesem Kanal gehörige `team_role_id` **oder** `COACH_ROLE_ID` (Coach darf
   in allen 4 Kanälen).

Reine Funktion, voll unit-testbar, keine Discord-Calls nötig:
```rust
fn can_operate(voice_channel_id: u64, member_role_ids: &[u64]) -> Option<TeamChannelConfig>
```

## Slash-Command

`/record start` und `/record stop` (Subcommands, keine Optionen). Registrierung über
`InteractionRouter::on_command("record start", spec, handler)` /
`"record stop"` — gleiches Muster wie `"steam links"` (siehe
`crates/dl-discord/src/interactions.rs`, `CommandSpec`/`on_command`).

Ein `RecordCommandHandler` implementiert `InteractionHandler` (Trait in
`crates/dl-discord/src/interactions.rs:192`) und dispatcht intern nach Subcommand-Namen.

## Ablauf `start`

1. Voice-State des Aufrufers lesen (denselben Lookup-Weg wie `crates/dl-voice/src/tracker.rs`
   für Session-Tracking nutzt — nicht neu bauen).
2. `can_operate` prüfen → sonst ephemere Fehlermeldung (Platzhalter-Text).
3. Prüfen, ob für diesen Kanal schon eine Aufnahme läuft (siehe Registry unten) → sonst
   ephemere Fehlermeldung „läuft schon".
4. Songbird-Voice-Verbindung zum Kanal aufbauen (`songbird::Songbird::join`).
5. Receive-Handler registrieren: pro SSRC dekodiertes PCM wird direkt in eine **WAV-Datei
   auf Platte** geschrieben (NICHT im RAM puffern — bei 48kHz/Stereo/16-bit sind das
   ~11,5 MB/Minute; eine Stunde im RAM wären ~700 MB pro Session).
6. Konsens-Nachricht (Platzhalter-Text) in den zugehörigen `team_text_channel_id` posten.
7. Session in Registry eintragen: `channel_id → { started_at, wav_path, voice_channel_id,
   text_channel_id }`.

## Ablauf `stop` (manuell oder automatisch)

Eine gemeinsame Funktion `stop_and_post(channel_id)`, aufgerufen von:
- `/record stop` (nach `can_operate`-Check),
- Auto-Stop bei leerem Kanal,
- Auto-Stop bei Erreichen des Dauer-Caps.

Schritte:
1. Voice-Verbindung trennen (`Songbird::remove`), WAV-Writer flushen/schließen.
2. `ffmpeg` (Systembinary, per PATH aufrufen — auf dem Dev-Host unter
   `~/.local/bin/ffmpeg` gefunden; **Worker muss auf dem tatsächlichen Deploy-Host
   verifizieren, dass `ffmpeg` im PATH des Service-Users liegt**, nicht nur lokal) WAV → MP3
   transkodieren, `-b:a 96k`.
3. MP3 per Attachment in `team_text_channel_id` posten (Muster: `CreateAttachment` wie in
   `crates/dl-discord/src/dispatch.rs`).
4. WAV- und MP3-Temp-Dateien löschen — **immer**, auch wenn Upload fehlschlägt (Cleanup-Guard,
   z. B. via `Drop` oder `defer`-artiges Pattern).
5. Session aus Registry entfernen.

## Auto-Stop-Trigger

1. **Kanal leer:** Hook in den zentralen `voice_state_update`-Dispatcher aus
   `crates/dl-voice/src/lib.rs` (dort sind laut Modul-Doku alle Subsysteme Subscriber des
   EINEN Dispatchers — neues Submodul `scrim_record` genauso einhängen wie `tracker`,
   `tempvoice` etc.). Bei jedem Event: wenn der betroffene Kanal einer der 4 ist, eine aktive
   Session hat und jetzt 0 Nicht-Bot-Mitglieder zählt → `stop_and_post`.
2. **Dauer-Cap:** Hartes Limit **60 Minuten** ab `started_at` (Hintergrund: 96kbps MP3 bei
   50 MB Discord-Boost-Tier-2-Limit ergibt ~69 Minuten Maximaldauer — 60 Minuten lässt
   Sicherheitsmarge). Bei Erreichen: `stop_and_post` + zusätzliche Hinweis-Notiz
   (Platzhalter-Text: „automatisch gestoppt, Maximaldauer erreicht").
   Einfachste Umsetzung: ein periodischer Sweep (z. B. alle 60s über die Registry iterieren,
   analog zum Sweep-Pattern in `crates/dl-voice/src/rename_queue.rs` oder dem
   Substitute-Sweep aus dem Scrim-Onboarding) statt pro-Session-Timer.

## Registry

`Arc<Mutex<HashMap<u64 /* voice_channel_id */, RecordingSession>>>` rein in-memory, **keine
DB-Persistenz**. Überlebt kein Bot-Neustart — akzeptierter Trade-off (ponytail: kein Rebuild
einer Reconnect-Logik für einen Rand-Fall, der bei einem Neustart während einer laufenden
Aufnahme minutenweise Audio verliert). Falls das später stört: erst dann nachrüsten.

## Dependencies

- `rust/Cargo.toml` (Workspace): `serenity` Feature-Liste um `"voice"` erweitern (Zeile ~82).
  Neuer Eintrag `songbird = { version = "0.4", features = ["serenity", "receive"] }`.
- `crates/dl-voice/Cargo.toml`: `songbird.workspace = true` ergänzen (analog zu
  `serenity.workspace = true`).
- `crates/dl-discord/src/gateway.rs:635`: `serenity::Client::builder(...)` Chain um
  `.register_songbird()` erweitern. Keine neuen Gateway-Intents nötig (`GUILD_VOICE_STATES`
  ist bereits gesetzt, Zeile 626).

## Wiring

`bin/dl-bot/src/main.rs`: nach Client-Build `songbird::get(&client).await` holen,
`RecordCommandHandler` konstruieren, bei `InteractionRouter` registrieren
(`"record start"`, `"record stop"`), `scrim_record`-Subscriber genauso einhängen wie die
bestehenden `dl-voice`-Subsysteme (exakte Stelle: dort wo `tracker`/`tempvoice` aktuell
verdrahtet werden — an dieser Stelle spiegeln, nicht danebenbauen).

## Texte (NUR Platzhalter, Claude schreibt final)

Jede dieser Stellen bekommt `// TODO(text): Platzhalter — <Kontext>` + einen Dummy-String,
keine finalen deutschen User-Texte von Codex:
- Konsens-Post bei Start ("🔴 Aufnahme gestartet von …")
- Ephemere Fehlermeldung: falscher Kanal
- Ephemere Fehlermeldung: keine Berechtigung
- Ephemere Fehlermeldung: Aufnahme läuft schon
- Ephemere Fehlermeldung: `/record stop` ohne aktive Aufnahme
- Hinweis-Notiz bei Dauer-Cap-Auto-Stop
- Fallback-Nachricht bei fehlgeschlagenem Upload

## Tests (TDD-Pflicht, siehe CLAUDE.md)

Pure/unit-testbar ohne echte Discord-Voice-Verbindung:
- `can_operate`: Tabellentest — eigene Team-Rolle im eigenen Kanal ok, in fremdem Kanal nicht,
  Coach-Rolle in allen 4 ok, unbeteiligte Rolle überall nein, `After-Scrimchannel` liefert
  `None`.
- `is_team_voice_channel`/Lookup: alle 4 IDs + eine beliebige fremde ID (inkl.
  `After-Scrimchannel`-ID explizit, da das der naheliegende Verwechslungsfehler ist).
- Dauer-Cap-Grenzfall: knapp unter/genau bei/über 60 Minuten.
- Registry-Zustandsmaschine: doppelter Start, Stop ohne aktiven Start, Start nach Stop.

**Explizit außerhalb der automatisierten Tests:** echte Voice-Audio-Aufnahme + `ffmpeg`-Aufruf
(kein CI-Harness für echte Discord-Voice-Verbindungen) — das wird live gegen den Testserver
verifiziert, nicht in der Unit-Suite.

## Branch

`feature/scrim-team-voice-recording` in Deadlock-Bots.
