# Paket G: Concierge erklärt Community und Streamer

status: gebaut, wartet auf Merge zusammen mit A bis D
datum: 2026-10-01
branch: wip/concierge

## Was gebaut ist

1. **Eigenes Bausteinmodul** `rust/crates/dl-community/src/concierge_community.rs`
   - Alle Texte (Tour-Block, fünf feste Antworten, Link-Antworten), Button-Bausteine,
     die deterministische Fragenerkennung `community_question()` und die Schnittstelle
     `TwitchLinkSource` für den persönlichen Verknüpfungslink.
   - Keine Abhängigkeit auf Concierge-Zustand, Datenbank oder Discord-Port. Nur
     `serde_json`, `url`, `async_trait`. Beim Umzug des Concierge ins Deadlock Brain
     wandert die Datei 1:1 mit, nur der `use`-Pfad in `concierge.rs` ändert sich.
2. **Server-Tour** (`concierge.rs`, `tour_body()`): Die Tour besteht jetzt aus
   Kanälen (`TOUR_TEXT`, Text unverändert ohne den Schlussabsatz), dem Block
   "Community und Streamer" mit eigenem Button "Twitch verknüpfen", dann dem
   bisherigen Schlussabsatz (`TOUR_CLOSING_TEXT`) mit den beiden Steckbrief-Buttons.
   Für Clients ohne Components V2 liefert `tour_fallback_text()` denselben Inhalt als Text.
3. **Button "Twitch verknüpfen"** (`concierge:twitch:link`, registriert in `register()`):
   - Fragt `Concierge::twitch_link_source()` nach dem persönlichen Link.
   - Link vorhanden und echte `https://discord.com/...`-Adresse: Antwort mit Link-Button
     "Jetzt mit Twitch verknüpfen".
   - Kein Link: fester Text, der auf den gleichnamigen Knopf im Verify-Kanal
     `<#1398021105339334666>` verweist (gleicher Weg wie Paket A). Kein TODO, kein Fehlerbild.
   - Log-Zeile nur mit Discord-ID und ob ein Link erzeugt wurde.
4. **Feste Antworten ohne Sprachmodell** (`local_conversational_answer()`): Fragen zu Punkten,
   Twitch-Verknüpfung, Clip-Contest, Streamer-Vorschlag und Streamer-Vorteilen werden vor dem
   Wissensdienst lokal beantwortet. Die Erkennung ist bewusst eng (Frageform, Stichworte,
   Ausschluss von Spielkontext wie Rang, Seelen, MMR und von Streamer-Einrichtungsfragen wie
   Dashboard, Bot, Overlay). Unter den Antworten zu Punkten und Twitch hängt der Button
   "Twitch verknüpfen", aber nur bei persönlichen Aktionen (DM oder eigener Fallback-Kanal)
   und nur, wenn der Text exakt eine der festen Antworten ist, nie unter freien Modelltexten.
5. **Wissensseite** `docs/community-punkte-und-streamer.md` im Format der übrigen
   `docs/*.md` (Worum geht es, Wie nutze ich das, Kosten, Technik kurz, Grenzen), mit den
   Regeln aus PLAN.md in Nutzersprache: Punkte, Deckel, Entdecker-Bonus, Clip-Contest,
   Vorschlag, Streamer-Punkte, nur verknüpfte Mitglieder sichtbar, kein Geldwert,
   keine Belohnung für Follows oder Subs.

## Wissenskorpus: Befund

- `dl-knowledge` (:8896) liest ausschließlich den Snapshot aus Deadlock-Docs
  (`DEFAULT_DOCS_PATH = /home/naniadm/.local/share/dl-knowledge/current/public/`,
  HTML, ENV-Override im Betrieb verboten). Im Repo gibt es keinen Korpus, den dl-knowledge indexiert.
- `docs/*.md` sind laut `docs/SSOT-HINWEIS.txt` die Spiegelkopie fürs FAQ-Grounding;
  SSOT ist Deadlock-Docs. Deshalb liegt die neue Seite dort, und die festen Antworten
  liegen im Bausteinmodul, damit der Concierge auch ohne Wissensdienst richtig antwortet.
- `assets/faq_texts.toml` (FAQ-Panel) ist nicht angefasst: Tests pinnen dort 15 Einträge,
  und das Panel sollte erst erweitert werden, wenn A bis D live sind.

## Anschlusspunkt Paket A (nach dem Merge)

Paket A war beim Bau noch nicht im Repo (Branch `wip/link` ohne Code). Deshalb ist der
Link-Erzeuger hinter einer Schnittstelle gekapselt:

```rust
// rust/crates/dl-community/src/concierge_community.rs
#[async_trait]
pub trait TwitchLinkSource: Send + Sync {
    async fn twitch_link_url(&self, discord_user_id: u64) -> Option<String>;
}
```

Anschließen, genau eine Stelle:

1. In dl-bot (dort, wo Paket A den Klick auf "Twitch verknüpfen" im Verify-Panel
   `rust/bin/dl-bot/src/serversync/rang_guide_publish.rs` bzw. `rust/bin/dl-bot/src/verify/`
   beantwortet) die Funktion nehmen, die den persönlichen Discord-Link erzeugt. Das ist der
   Aufruf von `POST /internal/v1/discord/initiate` am dl-dashboard
   (`rust/crates/dl-dashboard/src/web.rs`, `initiate`) mit `scope = "identify connections"`,
   `requesting_service` und `redirect_after`; Rückgabe ist `authorize_url`
   (heute `dl_webcore::dashboard::DashboardClient::discord_initiate`, das den Scope noch fest
   auf `identify` setzt; Paket A erweitert das oder baut eine eigene Variante).
2. Diese Funktion in einen `impl TwitchLinkSource` legen, der `Some(authorize_url)` liefert
   oder `None` bei jedem Fehler.
3. In `rust/bin/dl-bot/src/main.rs` direkt nach `Concierge::with_answers(...)`:
   `concierge.install_twitch_link_source(Arc::new(<PaketA-Quelle>));`

Ohne Schritt 3 funktioniert alles weiter, der Button verweist dann auf den Verify-Kanal.

Erledigt mit Paket F (`PAKET-F-discord.md`): `ConciergeTwitchLink` in
`rust/bin/dl-bot/src/serversync/twitch_link.rs`, installiert in `main.rs`. Der Knopf
"Streamer vorschlagen" sitzt im Clip-Panel `<#1425215762460835931>`, die Antworten verlinken ihn.
Bestätigung nach dem Callback ("Dein Twitch-Konto ... ist verknüpft") und das Speichern
bleiben vollständig bei Paket A; der Concierge reicht nur den Link weiter.

## Umzug ins Deadlock Brain

- `concierge_community.rs` mitnehmen, `use crate::concierge_community as community;` anpassen.
- `docs/community-punkte-und-streamer.md` nach Deadlock-Docs spiegeln
  (`public/discord-server/`, als HTML für den dl-knowledge-Snapshot), damit auch freie
  Fragen außerhalb der festen Muster aus dem Wissensdienst richtig beantwortet werden.
  Das ist in diesem Repo nicht möglich und bewusst nicht gemacht.

## Tests

- `concierge_community::tests` (7): Textregeln (keine Gedankenstriche, keine Fachwörter,
  Discord-Grenzen), Fakten aus PLAN.md in den Antworten, Erkennung der fünf Auftragsfragen
  plus Varianten, Nicht-Erkennung von Steam-, Rang-, Seelen-, Dashboard- und Bot-Fragen und
  Aussagen, Button nur bei Punkten und Twitch, Link-Prüfung nur `https://discord.com`,
  Standardquelle liefert `None`.
- `concierge::tests` (4 neu): Aufbau der Tour (Reihenfolge, Button-IDs, Textbudget 4000),
  lokale Antwort für alle fünf Fragen mit und ohne `free_voice`, Button nur unter festen
  Antworten und nur bei persönlichen Aktionen (auch zusammen mit Tour-Komponenten),
  Button-Klick mit Fallback und mit angeschlossener Quelle (einmalig installierbar).
- Bestehende Concierge-Tests unverändert und grün (DB-Tests über
  `rust/scripts/central_test_db.sh cargo test -p dl-community --features testing --lib concierge`:
  228 grün; 3 scheiterten nur an vollem Datenträger der geteilten Maschine und liefen einzeln grün).
- `cargo test -p dl-community --lib` ohne `testing`: 251 grün, 1 rot
  (`privacy::privacy_contract_tests::alle_migration_user_id_spalten_sind_im_privacy_vertrag`).
  Der Fehler besteht schon auf dem Basis-Commit: die `twitch_invite`-Tabellen aus
  Qualified-Invites fehlen in `privacy.rs`. Nicht Teil von Paket G, nicht angefasst.
- `cargo clippy -p dl-community --features testing --all-targets`: ohne Warnungen.
  `cargo build -p dl-bot -p dl-community`: grün. dl-brain nicht berührt.

## Rest-Risiken

- **Reihenfolge der Freischaltung:** Tour und Antworten beschreiben Funktionen aus A bis D.
  Paket G darf erst live gehen, wenn Verknüpfung, Zuschauerpunkte, Leaderboard und Clip-Contest
  live sind, sonst verspricht der Concierge Dinge, die es noch nicht gibt.
- **Datenschutz-Satz in der Doku** ("Deine Datenschutz-Einstellungen gelten auch für dieses
  Leaderboard") hängt an Paket C (`core.user_privacy` im Leaderboard). Bei Abweichung Doku anpassen.
- **Ort der Knöpfe aus D und F:** Kanäle für Clip-Contest und "Streamer vorschlagen" standen beim
  Bau nicht fest. Die Texte nennen deshalb nur den Knopfnamen und verweisen sonst auf
  `<#1491953161747955853>`. Sobald die Kanäle feststehen, können die Antworten sie verlinken.
- **Freie Modellantworten:** Fragen außerhalb der festen Muster gehen weiter an dl-knowledge;
  bis die Seite in Deadlock-Docs gespiegelt ist, kann dort eine Wissenslücke entstehen
  (Antwort ist dann der Verweis auf den Fragen-Kanal, keine erfundenen Regeln).
