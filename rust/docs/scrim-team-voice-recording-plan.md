# Scrim-Team-Voice-Recording — Implementierungsplan für Codex

**Scope-Hinweis für den Worker:** Halte dich exakt an diesen Auftrag. Ignoriere andere
Doku/Pläne im Repo (z. B. tempvoice-umbau, turnier-automatik) — die betreffen andere Features.

## Ziel

In genau 4 festen Team-Voice-Kanälen der Kategorie „Scrims" können Team-Mitglieder oder
Coaches manuell eine Sprachaufnahme starten/stoppen. Nach dem Stop postet der Bot die
Aufnahme als MP3 in den zugehörigen Team-Text-Kanal.

## Fixe Konfiguration (Guild `1289721245281292288`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamChannelConfig {
    pub voice_channel_id: u64,
    pub team_role_id: u64,
    pub text_channel_id: u64,
}

const TEAM_VOICE_CHANNELS: &[TeamChannelConfig] = &[
    // Shiv-support-voice
    TeamChannelConfig { voice_channel_id: 1521167264533970954, team_role_id: 1521163175318388847, text_channel_id: 1521164348976922795 },
    // Team-2
    TeamChannelConfig { voice_channel_id: 1521167309828526183, team_role_id: 1521163233245794465, text_channel_id: 1521164426424750171 },
    // Return Fire Department Voice
    TeamChannelConfig { voice_channel_id: 1521167476711358505, team_role_id: 1521163300480483498, text_channel_id: 1521164444787540110 },
    // TetraHex Voice
    TeamChannelConfig { voice_channel_id: 1521939025982652416, team_role_id: 1521163334211338391, text_channel_id: 1521164466169970728 },
];
const COACH_ROLE_ID: u64 = 1494372744286965941;

/// Einziger Lookup-Ort — jede andere Funktion (`can_operate`, Auto-Stop-Trigger,
/// Cap-Sweep) ruft NUR diese Funktion auf, kein zweiter Lookup-Pfad.
fn team_channel_config(voice_channel_id: u64) -> Option<TeamChannelConfig> {
    TEAM_VOICE_CHANNELS.iter().copied().find(|c| c.voice_channel_id == voice_channel_id)
}
```

`After-Scrimchannel` (1528447165926604991) ist **nicht** Teil des Scopes —
`team_channel_config(1528447165926604991)` muss `None` liefern.

## Berechtigung

Ein Nutzer darf `/record start` oder `/record stop` für einen der 4 Kanäle nur ausführen,
wenn beide gelten:
1. Er sitzt **gerade selbst** im jeweiligen Voice-Kanal (Ziel wird aus seinem eigenen
   Voice-State abgeleitet, kein Channel-Parameter im Command).
2. Er hat die zu diesem Kanal gehörige `team_role_id` **oder** `COACH_ROLE_ID` (Coach darf
   in allen 4 Kanälen).

**Precondition (vom Aufrufer sicherzustellen, `can_operate` prüft das NICHT erneut):**
`voice_channel_id` ist bereits der aktuelle Voice-Kanal des aufrufenden Members, aufgelöst
über den Voice-State-Lookup — `can_operate` bekommt diesen Wert fertig übergeben.

Reine Funktion, voll unit-testbar, keine Discord-Calls nötig. Unterscheidet explizit
zwischen „kein Team-Kanal" und „Kanal ok, aber keine Berechtigung", weil beide Fälle
unterschiedliche Fehlermeldungen auslösen (siehe Texte-Abschnitt):
```rust
pub enum OperateDenied {
    UnknownChannel,
    NotPermitted,
}

pub fn can_operate(
    voice_channel_id: u64,
    member_role_ids: &[u64],
) -> Result<TeamChannelConfig, OperateDenied> {
    let config = team_channel_config(voice_channel_id).ok_or(OperateDenied::UnknownChannel)?;
    let allowed = member_role_ids.contains(&config.team_role_id)
        || member_role_ids.contains(&COACH_ROLE_ID);
    if allowed { Ok(config) } else { Err(OperateDenied::NotPermitted) }
}
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
2. `can_operate` prüfen → bei `Err(UnknownChannel)` bzw. `Err(NotPermitted)` jeweils die
   passende ephemere Fehlermeldung (zwei verschiedene Platzhalter-Texte, siehe Texte-Abschnitt).
3. **Atomarer Claim** (siehe Registry-Abschnitt, schließt die Race aus zwei parallelen
   `/record start`): unter EINEM Lock prüfen „läuft schon?" und, falls nicht, sofort einen
   `SessionState::Starting`-Platzhalter eintragen. Erst danach — außerhalb des Locks — geht es
   weiter. War schon eine Session da: ephemere Fehlermeldung „läuft schon".
4. `started_at = Instant::now()` **jetzt** setzen, bevor der Join versucht wird (konservativ:
   die Verbindungsaufbauzeit zählt mit in den Dauer-Cap, nie dagegen).
5. Songbird-Voice-Verbindung zum Kanal aufbauen (`songbird::Songbird::join`).
   - **Fehlschlag:** Claim aus der Registry zurückrollen (Eintrag entfernen), ephemere
     Fehlermeldung posten, Ablauf beenden. Kein hängender `Starting`-Eintrag.
6. Receive-Handler registrieren: pro SSRC dekodiertes PCM wird direkt in eine **WAV-Datei
   auf Platte** geschrieben (NICHT im RAM puffern — bei 48kHz/Stereo/16-bit sind das
   ~11,5 MB/Minute; eine Stunde im RAM wären ~700 MB pro Session).
7. Session-Status auf `SessionState::Recording` setzen (gleicher Registry-Eintrag, kein neuer
   Insert).
8. Konsens-Nachricht (Platzhalter-Text) in den zugehörigen `team_text_channel_id` posten.

## Ablauf `stop` (manuell oder automatisch)

Eine gemeinsame Funktion `stop_and_post(voice_channel_id: u64)` — Parametername bewusst
`voice_channel_id`, weil das **immer der Registry-Key** ist, nie die Text-Channel-ID.
Aufgerufen von drei unabhängigen Triggern, die potenziell gleichzeitig feuern können:
- `/record stop` (nach `can_operate`-Check),
- Auto-Stop bei leerem Kanal,
- Auto-Stop bei Erreichen des Dauer-Caps (Sweep).

Schritte:
1. **Atomarer Claim** (siehe Registry-Abschnitt): unter EINEM Lock nur dann von
   `SessionState::Recording` auf `SessionState::Stopping` wechseln, wenn der Zustand aktuell
   `Recording` ist. Ist er das nicht (schon `Stopping` oder gar nicht vorhanden), sofort
   `return` — genau EIN Trigger gewinnt das Rennen, alle anderen sind No-Ops.
2. Voice-Verbindung trennen (`Songbird::remove`), WAV-Writer flushen/schließen.
3. `ffmpeg` (Systembinary, per PATH aufrufen — auf dem Dev-Host unter
   `~/.local/bin/ffmpeg` gefunden; **Worker muss auf dem tatsächlichen Deploy-Host
   verifizieren, dass `ffmpeg` im PATH des Service-Users liegt**, nicht nur lokal) WAV → MP3
   transkodieren, `-b:a 96k`.
4. MP3 per Attachment in `team_text_channel_id` posten (Muster: `CreateAttachment` wie in
   `crates/dl-discord/src/dispatch.rs`).
   - **Fehlschlag beim Upload:** Fallback-Platzhalter-Nachricht posten. Trotzdem mit Schritt 5
     fortfahren (kein Sonderpfad).
5. WAV- und MP3-Temp-Dateien löschen — **immer**, auch wenn Upload fehlgeschlagen ist
   (Cleanup-Guard, z. B. via `Drop` oder `defer`-artiges Pattern; läuft unabhängig davon, ob
   Schritt 4 erfolgreich war).
6. Session **erst jetzt** vollständig aus der Registry entfernen (nicht schon bei Schritt 1 —
   solange der Eintrag existiert, egal in welchem State, gilt der Kanal als „belegt"; ein
   `/record start` unmittelbar nach einem `/record stop` muss also bis zum Abschluss von
   Schritt 5/6 mit „läuft schon" abgewiesen werden, nicht schon nach dem State-Wechsel in
   Schritt 1).

## Auto-Stop-Trigger

1. **Kanal leer:** Hook in den zentralen `voice_state_update`-Dispatcher aus
   `crates/dl-voice/src/lib.rs` (dort sind laut Modul-Doku alle Subsysteme Subscriber des
   EINEN Dispatchers — neues Submodul `scrim_record` genauso einhängen wie `tracker`,
   `tempvoice` etc.). Bei jedem Event: wenn der betroffene Kanal einer der 4 ist, eine aktive
   Session hat und jetzt 0 Nicht-Bot-Mitglieder zählt → `stop_and_post`.
2. **Dauer-Cap:** **60 Minuten** ab `started_at` — KEIN exakter Timer, sondern ein
   periodischer Sweep alle 60s über die Registry (analog zum Sweep-Pattern in
   `crates/dl-voice/src/rename_queue.rs` bzw. dem Substitute-Sweep aus dem
   Scrim-Onboarding), der jede Session mit `elapsed >= 60min` triggert. Toleranz von bis zu
   ~60s über der Sollgrenze ist bewusst akzeptiert (kein Pro-Session-Timer nötig) — bei 96kbps
   MP3 sind 61 Minuten ~43,9 MB, immer noch klar unter dem 50 MB-Boost-Tier-2-Limit (die
   50MB/96kbps-Rechnung ergibt theoretisch ~72 Minuten Maximaldauer; 60 Minuten +
   Sweep-Toleranz lässt also weiterhin komfortable Marge, keine Bitrate-Anpassung nötig). Bei
   Erreichen: `stop_and_post` + zusätzliche Hinweis-Notiz (Platzhalter-Text: „automatisch
   gestoppt, Maximaldauer erreicht").

## Registry

Rein in-memory, **keine DB-Persistenz**. Überlebt kein Bot-Neustart — akzeptierter
Trade-off (ponytail: kein Rebuild einer Reconnect-Logik für einen Rand-Fall, der bei einem
Neustart während einer laufenden Aufnahme minutenweise Audio verliert). Falls das später
stört: erst dann nachrüsten.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState { Starting, Recording, Stopping }

struct RecordingSession {
    state: SessionState,
    started_at: std::time::Instant, // NICHT chrono/UTC — nur Elapsed-Berechnungen nötig,
                                     // monotone Zeit ist hier das einzig Relevante.
    wav_path: std::path::PathBuf,
    text_channel_id: u64,
}

type Registry = std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<u64, RecordingSession>>>;
// Key = voice_channel_id (siehe `TeamChannelConfig::voice_channel_id`), NIE die Text-Channel-ID.
// tokio::sync::Mutex, weil das Crate dafür bereits Konvention ist (siehe
// crates/dl-voice/src/tracker.rs:151-162). Lock-Disziplin: der Lock wird NIE über einen
// .await-Punkt hinweg gehalten — jede kritische Sektion ist synchron und kurz (State prüfen/
// setzen, Eintrag einfügen/entfernen), alle async-Arbeit (Join, ffmpeg, Upload) passiert mit
// bereits released Lock.
```

**Zustandsübergänge (jeweils die einzigen erlaubten):**
- *(kein Eintrag)* → `Starting`: atomar unter einem Lock-Griff „prüfen ob Key existiert,
  wenn nicht: einfügen" — z. B. `match registry.entry(voice_channel_id) { Entry::Occupied(_) =>
  return AlreadyRecording, Entry::Vacant(v) => { v.insert(Starting-Session); } }`. Schließt die
  Race zwischen zwei parallelen `/record start` aus (Punkt „Start prüft dann trägt ein" ist
  damit EIN Schritt, nicht zwei).
- `Starting` → *(entfernt)`: Rollback bei Join-Fehlschlag.
- `Starting` → `Recording`: nach erfolgreichem Join + Receive-Handler-Registrierung.
- `Recording` → `Stopping`: atomarer Claim, nur wenn aktuell `Recording` — genau ein
  Stop-Trigger (manuell/leerer-Kanal/Cap-Sweep) gewinnt, alle anderen sehen `Stopping` oder
  gar keinen Eintrag mehr und sind No-Ops.
- `Stopping` → *(entfernt)`: erst nach vollständigem Cleanup (Dateien gelöscht), unabhängig
  davon ob der Upload erfolgreich war — ein fehlgeschlagener Upload darf den Kanal nicht
  dauerhaft als „belegt" blockieren.

Kein Zustand darf übersprungen werden; jede Transition läuft über genau eine
Lock-geschützte, synchrone Prüfung+Mutation.

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
- `team_channel_config`: alle 4 IDs liefern die **volle, korrekte** `TeamChannelConfig`
  (Struct-Vergleich, nicht nur "ist Some" — sonst fällt eine vertauschte Rollen-/
  Text-Channel-Zuordnung nicht auf), PLUS zwei getrennte negative Fälle: eine beliebige
  fremde ID liefert `None`, UND explizit `team_channel_config(1528447165926604991)` (=
  `After-Scrimchannel`) liefert `None` (naheliegendster Verwechslungsfehler, eigener
  Testfall statt nur "irgendeine fremde ID").
- `can_operate`: Tabellentest — eigene Team-Rolle im eigenen Kanal → `Ok`, eigene Team-Rolle
  in fremdem Team-Kanal → `Err(NotPermitted)`, Coach-Rolle in allen 4 → `Ok`, unbeteiligte
  Rolle überall → `Err(NotPermitted)`, `After-Scrimchannel`/beliebige fremde ID (auch mit
  Coach-Rolle) → `Err(UnknownChannel)` (die beiden Fehlerarten müssen sich in den Tests
  unterscheiden, nicht nur "kein Ok").
- Dauer-Cap-Grenzfall: `elapsed` knapp unter/genau bei/knapp über 60 Minuten → jeweils
  erwartetes Sweep-Verhalten (nicht triggern / triggern).
- Registry-Zustandsmaschine (synchron testbar, ganz ohne Songbird/Discord): doppelter Start
  auf denselben Kanal (zweiter muss `AlreadyRecording` sehen, weil Insert+Check ein
  atomarer Schritt ist), Stop ohne aktive Session (No-Op), zwei gleichzeitige Stop-Claims auf
  dieselbe `Recording`-Session (nur einer gewinnt den Wechsel zu `Stopping`), Start auf einen
  Kanal dessen letzte Session sich noch im `Stopping`-Zustand befindet (muss `AlreadyRecording`
  liefern, erst nach vollständigem Entfernen wieder frei).

**Explizit außerhalb der automatisierten Tests:** echte Voice-Audio-Aufnahme + `ffmpeg`-Aufruf
(kein CI-Harness für echte Discord-Voice-Verbindungen) — das wird live gegen den Testserver
verifiziert, nicht in der Unit-Suite.

## Branch

`feature/scrim-team-voice-recording` in Deadlock-Bots.
