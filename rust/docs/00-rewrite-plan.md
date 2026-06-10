# Rust-Rewrite der Deadlock-Bots — Plan und Verträge

Stand: 2026-06-10 · Status: vom Betreiber freigegeben (Scope, Cutover-Strategie, Architektur)

## Auftrag

Das komplette Repo wird phasenweise nach Rust portiert — nicht als blinde 1:1-Übersetzung,
sondern mit Aufräumung: Doppeltes wird zentralisiert, Veraltetes nicht übernommen,
Gott-Dateien werden zerlegt. Das Python-Original bleibt unangetastet liegen und lauffähig;
alles Neue entsteht ausschließlich unter `/rust`.

## Ausgangslage (Inventar 10.6.2026)

- ~71.000 Zeilen Python: ~40 Cogs + Service-Layer in EINEM Prozess
- Der eine Prozess macht sechs Jobs: Discord-Gateway, Dashboard :8766, Turnier-Web :8767,
  Public-Stats :8768, Master-Broker :8770, Tierlist :8771, Changelog-Empfänger :8899,
  dazu ein Subprocess-Supervisor
- `service/dashboard.py`: 7.615 Zeilen, 1 Klasse, 69 Handler, 12 fachfremde Themenblöcke,
  zwei parallele Turnier-APIs, tote Windows/NSSM-Pfade
- 5 Cogs hören unkoordiniert auf `voice_state_update`, 6 auf `message`
- 3 baugleiche Server-Wrapper-Cogs, doppeltes Invite-Tracking, 166 verstreute `os.getenv`
  in 38 Dateien, 934 breite `except Exception`
- DB `data/deadlock.sqlite3`: 114 Tabellen, ~25 leer/verwaist, WAL aktiv, 37 MB
- Bereits ersetzt: `cogs/steam` → Rust-Steam-Bot (eigenes Repo), Twitch-Logik → eigenes Repo

## Entscheidungen

| Frage | Entscheidung |
|---|---|
| Scope | Alles, phasenweise (Bot + Dashboard + Broker + Public-Services) |
| Cutover | Strangler-Fig: Domäne für Domäne, jeder Go-Live nur nach Freigabe des Betreibers |
| Prozessschnitt | 2 Binaries: `dl-bot` (Gateway+Broker+interne APIs), `dl-web` (alle Websites) — ADR 0001 |
| Stack | serenity/poise, axum, rusqlite, tokio, tracing, thiserror — ADR 0001 |
| standalone_manager | Wird nicht portiert (obsolet, systemd übernimmt) — ADR 0002 |
| player_finder | WIRD portiert, bleibt aber per Feature-Flag deaktiviert (Betreiber plant Redesign) |

## Architektur

```
/rust (Cargo-Workspace)
├─ bin/dl-bot       Discord-Gateway (serenity/poise) + Master-Broker :8770
│                   + Changelog-Empfänger :8899 + interne Control-API
├─ bin/dl-web       Public-Stats :8768 + Tierlist :8771 + Turnier :8767 + Dashboard :8766
└─ crates/
   ├─ dl-core       Config (typisiert, EINMAL geladen), Fehler-Basis, Observability
   ├─ dl-db         SQLite-Schicht (rusqlite, 1 Writer + Read-only-Reader), kv_store-Repo
   ├─ dl-discord    Gateway-Glue, Event-Dispatcher, custom_id-Routing, Embed/DM-Helfer
   ├─ dl-ai         Provider-Abstraktion (MiniMax primär, OpenAI/Gemini)
   ├─ dl-broker     Master-Broker-Logik (API-kompatibel: /internal/master/v1)
   ├─ dl-voice      tempvoice + rank_voice + voice_status + tracker + reaction_dm + nudge
   ├─ dl-activity   Aktivitäts-Analytik (zerlegt), retention, EIN Invite-Tracking, lfg, player_finder
   ├─ dl-moderation security_guard + ai_moderator
   ├─ dl-community  tags, faq_chat, server_faq, feedback_hub, clips, rename, leave_survey, rules, bug_reporter
   ├─ dl-onboarding welcome_dm-Flow + onboarding + ai_onboarding
   ├─ dl-coaching   die 5 Coaching-Cogs als EIN Modul mit Session-Lifecycle
   ├─ dl-tournament customgames + team_balancer + Turnier-Web-Logik
   ├─ dl-bridges    steam_bridge (→ Steam-Bot :8783), twitch (→ Twitch-Bot :8776)
   └─ dl-webcore    Session/Auth/CSRF/OAuth-Relay — EINMAL statt 4× kopiert
```

Crates entstehen erst in der Phase, die sie braucht — keine leeren Hüllen auf Vorrat.

## Querschnitts-Redesigns

1. **Ein Voice-Event-Dispatcher** statt 5 unkoordinierter Listener: `dl-discord`
   normalisiert `voice_state_update` zu Session-Events, Domänen subscriben.
   Gleiches Muster für `message`.
2. **Eine Config**: typisiert, beim Start einmal gelesen und validiert, dann immutable.
   ENV-Namen identisch zum Python-Original (gleiche systemd-Umgebung für beide Welten).
3. **Eine Web-Auth** (`dl-webcore`): Dashboard-OAuth, Sessions, CSRF, Internal-Token —
   heute 4× kopiert. Tierlists Direkt-Griff in fremde Session-Dicts entfällt.
4. **Ein Invite-Tracking, eine Turnier-API** (Legacy `/api/tournament/*` entfällt).
5. **Fehler-Typen statt `except Exception`**: thiserror pro Domäne, Behandlung an den Rändern.
6. **user_activity_analyzer zerlegen** in Erfassung / Aggregation / Abfrage-Commands.
7. **Privacy als Vertrag**: jede Domain-Crate deklariert ihre personenbezogenen Tabellen
   selbst; der DSGVO-Export/-Delete sammelt sie ein (kein manuelles Zentralregister).

## Strangler-Fig-Mechanik

- `data/deadlock.sqlite3` ist der gemeinsame Vertrag → `01-db-contract.md`.
- Cutover pro Domäne: Eintrag in `cog_blocklist.json` (bewährt bei `cogs.steam`),
  Rust übernimmt Funktion und Port. Ports bleiben identisch — Twitch-Bot (Broker :8770),
  Caddy und Website merken nichts.
- Python-Code wird NIE gelöscht, nur deaktiviert.
- Pro Cutover: Flip-Checkliste in `rust/docs/`, Go-Live nur nach Freigabe.

## Phasen

| Phase | Inhalt | Cutover-Risiko |
|---|---|---|
| 0 | Workspace, dl-core, dl-db, Docs, Check-Script | — |
| 1 | dl-web: Public-Stats + Tierlist (read-heavy, kein Discord) | niedrig |
| 2 | dl-bot-Gerüst: Gateway, Dispatcher, Broker :8770, Changelog :8899 | mittel |
| 3 | dl-bridges: steam_bridge + steam_*-Cogs + twitch | mittel |
| 4 | dl-voice (Redesign um den Dispatcher) | hoch |
| 5 | dl-activity + lfg + player_finder (Flag aus) | mittel |
| 6 | dl-community + dl-ai + dl-moderation | mittel |
| 7 | dl-onboarding + dl-coaching | mittel |
| 8 | dl-tournament (Web + Cogs) | mittel |
| 9 | Dashboard (sauber geschnitten) + Restpflege | hoch |

## Nicht portiert (bleibt im Python-Original erhalten, wird nicht gelöscht)

| Was | Warum |
|---|---|
| `old/` | Archiv, läuft nirgends |
| `cogs/steam`-Namespace-Brücke | Import-Hack, seit Rust-Steam-Bot per Blocklist deaktiviert |
| Legacy `/api/tournament/*` | Doppelte Turnier-API; nur `/api/turnier/*` wird portiert |
| Windows/NSSM-Pfade im Dashboard | Betrieb ist vollständig Linux/systemd |
| `service/hooks/startup_check.py` | No-Op-Stub |
| `coaching_sessions_legacy` | Migrationsrest; `coaching_sessions` ist aktiv |
| `beta_invite_pending_payments` | 0 Zeilen, totes Feature |
| `steam_rich_presence`, `steam_presence_watchlist` | 0 Zeilen; Presence macht der Rust-Steam-Bot |
| standalone_manager (Supervisor) | ADR 0002 — Tabellen-Vertrag `standalone_bot_state` bleibt |

Funktionierende Features mit bislang leeren Tabellen (z. B. Tierlist-Build-Votes) werden portiert.

## Qualitäts-Gates

- `rust/scripts/check.sh`: `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test`
- Contract-Tests gegen die HTTP-APIs vor jedem Cutover (insbesondere Broker)
- Tests laufen NIE gegen die Produktions-DB, nur gegen Temp-DBs mit Vertrags-DDL
- Doku-Pflicht: Architektur/ADRs/DB-Vertrag in `rust/docs/` laufend mitschreiben
