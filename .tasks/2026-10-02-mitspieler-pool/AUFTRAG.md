# Auftrag: mitspieler-pool

status: aktiv (2026-10-02)

## Ziel

In #mitspieler-suche schreibt jeder rein und niemand liest, was andere geschrieben haben.
Stattdessen bekommt jedes Mitglied ein Spielerprofil: Beim Onboarding fragt der Bot in einem
kurzen Interview die Vorlieben ab (wann, was, mit wem, wie viele), Steam wird ohne Steam-Bot
verknüpft, aus der Deadlock-API entsteht automatisch ein Spielmuster (wann gezockt, Rang,
Stunden, Spiele, mit wem zuletzt gespielt). Auf einer Website sieht man den Pool, findet
Mitspieler und drückt "Zusammen spielen". Dann öffnet der Bot einen Kanal auf dem Server,
nicht in den DMs. Nach der Session gibt es auf der Website ein Feedback per Checkboxen.

Zuerst wird alles gebaut und getestet (Datenbank, Mapping, Seite, Kanal). Antworten des Bots
im Textkanal #mitspieler-suche gehören ausdrücklich NICHT zu diesem Auftrag.

## Festgelegte Entscheidungen des Nutzers

- Interview über den Bot (Concierge mit Brain-Wissen), beim Onboarding, Speicherung in der
  zentralen DB je Discord-User-ID. Faule Nutzer: möglichst Auswahlmenüs und Knöpfe statt
  Freitext, das Gespräch darf aber natürlich klingen.
- Steam-Verknüpfung ohne Steam-Bot-Freundschaft. Rang, Stunden, Spiele und Spielzeiten kommen
  aus der Deadlock-API, nicht aus Selbstauskunft.
- Website zeigt Pool, Profile, Spielzeiten-Muster und "recently played with".
- "Zusammen spielen": Bot legt proaktiv einen Kanal für die Beteiligten an und pingt sie dort.
  Kanal bleibt 1 h; kommt niemand, wird er gelöscht. DMs dazu nur per Opt-in (Schalter im
  Profil, Default aus).
- Feedback nach der Session nur auf der Website, Checkboxen, keine Benachrichtigung.

## Defaults (vom Intent-Agent gesetzt, im Bericht nennen)

- Pfad der Seite: `/spielerpool` auf deutsche-deadlock-community.de, weil `/mitspieler/` schon
  die Voice-Tower-Seite in `Website/dl-landing` ist.
- Sichtbarkeit: nur eingeloggte Discord-Mitglieder der Guild 1289721245281292288 sehen den
  Pool. "Recently played with" zeigt nur Personen, die selbst im Pool sind.
- Testbetrieb: Interview beim Onboarding zunächst nur für `DL_CONCIERGE_TEST_USER_ALLOWLIST`
  plus Slash-Befehl `/spielerprofil`, damit der Nutzer selbst testen kann. Scharfschalten für
  alle entscheidet der Nutzer nach dem Test.
- Profil löschen: Knopf auf der Website und im Datenschutz-Löschpfad
  (`rust/crates/dl-community/src/privacy.rs`).

## Pakete

Schnitt und Reihenfolge in `PAKETE.md`. Je Paket ein Worker-Thread, eigener Worktree unter
`~/.worktrees/<repo>-pool-<paket>`, eigener Branch `feat/spielerpool-<paket>`.

## Fundstellen (aus dem Vorcheck)

- `rust/crates/dl-community/src/concierge.rs:3971-4013`: Onboarding-Handling und Begrüßungs-DM.
- `rust/crates/dl-community/src/concierge.rs:1676-1706`: `bot.concierge_profiles`,
  `bot.concierge_conversations`, keine strukturierten Spielpräferenzen.
- `rust/docs/central-db/architecture.md:43-44`: `core.steam_links`.
- `Deadlock-Twitch-Bot/rust/crates/tb-dashboard-api/src/auth/steam_openid.rs:7-38,61-124`:
  Steam-OpenID-Prüfung (an Twitch-Login gebunden, Logik übertragbar).
- `Website/builds/backend-rust/src/routes/auth.rs:28-45,143-159`: Discord OAuth (`identify`),
  JWT-Session-Cookie.
- `Website/builds/frontend/src/App.tsx:32-66`, `.../context/AuthContext.tsx:21-49`: React-App
  mit `/auth/me`.
- `Website/dl-landing/src/site.js:491-505`: `/mitspieler/` ist belegt.
- `Deadlock-Twitch-Bot/rust/crates/tb-chat/src/rank_lookup.rs:67-75,310-331`: öffentliche
  Deadlock-API für Rang.
- `Deadlock-Steam-Bot/rust/crates/steam-flows/src/deadlock_api.rs:30-87`: Deadlock-API-Rang.
- `rust/crates/dl-activity/src/analyzer.rs:142-221`: `activity.user_activity_patterns` aus
  Voice-Sessions (Zusatzsignal fürs Spielmuster).
- `rust/crates/dl-activity/src/player_finder.rs`: abgeschalteter Finder (Rang ±3, Zeitmuster,
  Steam-Status), Matching-Logik wiederverwenden.
- `rust/crates/dl-voice/src/mate_survey.rs:633-651`: `activity.voice_mate_ratings`.
- `rust/crates/dl-voice/src/lfg_panel.rs:558-613`: `RouterLfgLaneSpawner`.
- `rust/crates/dl-voice/src/tempvoice/engine/reconcile.rs:167-303`: Löschen leerer Lanes.
- `rust/crates/dl-central-db/migrations/2026100101_concierge_pate_optout_close.sql`: letzte
  Migration, vor Vergabe `origin/main` frisch holen.

## Was nicht angefasst wird

- Textkanal #mitspieler-suche, LFG-Forum, LFG-Panel-Verhalten, Steam-Bot (Deadlock-Steam-Bot).
- Twitch-Bot-Repo: nur Logik lesen und übertragen, dort nichts ändern.
- Keine neuen LLM-Clients: jeder Modellaufruf über den zentralen Provider des Bots, kein
  Modellwechsel.
- Schon angewandte Migrationen nie ändern.

## Fertig-Kriterium

Mit dem Konto des Nutzers (Discord-ID aus der Allowlist) und einem Zweitkonto belegt:
1. `/spielerprofil` bzw. Onboarding startet das Interview, Antworten stehen in der DB.
2. Steam per OpenID auf der Website verknüpft, Eintrag in `core.steam_links` ohne Bot-Freundschaft.
3. Ingest hat aus der Deadlock-API Rang, Spiele, Stunden, Spielzeiten-Heatmap und
   Mitspieler-Paare geschrieben.
4. `/spielerpool` zeigt beide Profile mit Muster und "recently played with", Filter nach
   Zeit, Modus und Rang funktionieren.
5. "Zusammen spielen" öffnet einen Kanal, pingt beide, Kanal verschwindet nach 1 h ohne Beitritt.
6. Feedback-Checkboxen speichern, Profil-Löschen entfernt alle Pool-Daten.

## Deploy-Weg

Migration als `postgres` in die zentrale DB, `dl-bot` Release-Build im eigenen Worktree,
`systemctl --user restart deadlock-bot-rust`, Website-Backend und Frontend nach dem
Deploy-Weg des Website-Repos, Caddy-Route für `/spielerpool` im Repo `caddy-config`.

## Rahmen

- Du bist der einzige Thread für dein Paket. Keine Unter-Threads oder Unter-Agenten spawnen.
- Keine Code-Kommentare schreiben, Code erklärt sich selbst.
- Nur den eigenen Branch pushen, nie main ohne Gate-ALLOW.
- Codesuche zuerst über `graphify` (Skill `code-suche`).
- Keine ENV-Dateien, Secrets aus Infisical, interne Tokens wiederverwenden.
- Nutzersichtbare Texte natürlich, echtes Deutsch mit Umlauten, keine Em-Dashes, nicht bottig.
- Auftrag größer als beschrieben: Bump-up-Nachricht an den Intent-Thread, dann stoppen.
