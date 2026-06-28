# Phase 9: Dashboard (:8766) — Inventar und Schnitt-Plan

Stand: 2026-06-10. `service/dashboard.py` (7615 Zeilen, aiohttp) ist der
letzte nicht portierte Block. Ein 1:1-Port wäre falsch — gut die Hälfte
der Routen steuert die **Python-Runtime**, die nach dem Vollumstieg nicht
mehr existiert. Das Dashboard braucht das geplante Redesign, und dafür
eine Design-Entscheidung von Nani (unten).

## Fortschritt

- **2026-06-13 — Design-Entscheidung getroffen (Nani: „mach alles fertig"):**
  Bot-Steuerung wird **Option (a)** — systemd-Units + Blocklist-Editor +
  Feature-Flags. Kein Cog-Reload-Nachbau.
- **2026-06-13 — Phase 9a AUTH-PROVIDER KOMPLETT** (Crate `dl-dashboard`,
  Changelog #122). Gebaut + getestet (35 Unit-Tests) + Ende-zu-Ende gegen
  den laufenden Prozess bewiesen:
  - OAuth-Client, DB-State-Store (`oauth_states`), In-Memory-Session-Store
    (`master_dash_session`), Zugriffsentscheidung (Owner/Admin/Mod → Full,
    Community-Mod → TurnierOnly), interne Token-Guards, Redirect-Allowlist.
  - Alle 15 Auth-Routen: Admin-Login (`/auth/discord/login` →
    `/callback/discord`), delegiertes Relay (`initiate`/`consume-result`),
    turnier/twitch `authorize-url`+`session`, `steam-link-session`,
    Twitch-SSO `validate-session`/`import-session`, `/api/auth/me`, Logout.
  - NEU am Broker: `GET /internal/master/v1/discord/member-access` (Rollen +
    Admin-Status aus dem Cache; dl-web hat keinen Gateway).
  - Befund: ALLE rollenliefernden Endpunkte gehen über den Bot-Cache
    (Broker), nicht über den OAuth-Scope `guilds.members.read`.
  - In `dl-web` auf :8766 gebunden (Connect-Info für Loopback-Prüfung). SPA
    ist vorerst Platzhalter. dl-web läuft NICHT als Dienst (Python 8766 aktiv).

## Nächste Schritte (offen)

1. **9b Analytics-Reads** — voice-stats, voice-history, user-retention,
   leave-surveys, member-events, message-activity, co-player-network,
   server-stats. Reine DB-Reads, aber teils groß (voice-history ~344 Z.).
   Brauchen ein gemeinsames `/api`-Auth-Gate (Session aus Cookie via
   `session_from_headers` + ggf. Full-Access-Prüfung; CSRF nur bei
   Mutationen). **`message_activity` + Co-Player brauchen Namensauflösung
   per User-ID → Broker-Bulk** (`_resolve_display_names` nutzt den
   Bot-Cache; entweder `GET /discord/members` einmal cachen oder einen
   Bulk-Resolve-Endpunkt ergänzen).
2. **9c** deadlock/config + heroes (klein, kv/Tabellen).
3. **9d** ~~Turnier-Admin~~ — **2026-06-28 entfernt** (Turnier läuft im Repo
   Deadlock-Turniere; kein `dl-tournament`-Port mehr).
4. **9e** Survey-Web (`leave-survey/{token}`) + public guild-stats/patch-notes.
5. **9f** Steuerung neu (systemd-Restart `dl-bot`/`dl-web`,
   `cog_blocklist.json`-Editor, Log-Tail) — Option (a).

## Routen-Inventar (~70 Routen, 7 Gruppen)

| Gruppe | Routen | Rust-Einschätzung |
|---|---|---|
| **Auth-Provider** | Discord-OAuth (login/callback/logout), Sessions, `/internal/v1/discord/initiate`+`consume` | **PFLICHT zuerst** — Stats :8768 und Tierlist :8771 delegieren ihre Auth hierher (dl-webcore::DashboardClient). Solange Python-8766 läuft, funktioniert alles; der Rust-Port muss session-/cookie-kompatibel sein. |
| **Bot-Steuerung** | bot/restart, cogs/reload/load/unload/block/unblock/discover, dashboard/restart | **NICHT 1:1** — Cog-Reload ist ein Python-Konzept. Rust-Äquivalent: systemd-Restart (dl-bot/dl-web), cog_blocklist.json-Editor (steuert den Rest-Python-Bot), Feature-Flags. |
| **Analytics-Reads** | voice-stats, voice-history, user-retention, leave-surveys, member-events, message-activity, co-player-network, server-stats | Sauber portierbar (reine DB-Reads auf bekannte Verträge). |
| **Deadlock-Config** | config GET/POST, heroes GET/POST | Klein, portierbar (kv/Tabellen). |
| ~~**Turnier-Admin**~~ | tournament/* + turnier/* | **2026-06-28 entfernt** — Turnier läuft im Repo Deadlock-Turniere, kein `dl-tournament`-Port. |
| **Host-Steuerung** | logs (Tail bis 5 MB), standalone/{key} start/stop/restart/command | Host-/Prozess-gebunden; portierbar (tokio::process + Datei-Tail), aber sicherheitskritisch — Scope mit Nani klären. |
| **Survey-Web + Public** | leave-survey/{token} GET/POST, public/guild-stats, public/patch-notes | Survey-Web gehört zur portierten Leave-Survey (#106) — Token-Vertrag existiert in Rust schon. |

## Empfohlene Reihenfolge

1. **Auth-Provider + Sessions** (kompatibel zum dl-webcore-Codec, der
   BYTE-identisch verifiziert ist) — danach kann 8766 in Rust die
   Auth-Quelle für die schon portierten Webs sein.
2. **Analytics-Reads + Survey-Web** (reine Store-Arbeit).
3. **Steuerungs-Seite neu denken** (systemd statt Cogs) — NACH Nanis
   Antwort auf die Design-Frage.

## Design-Frage an Nani (blockiert nur Gruppe „Bot-Steuerung")

Das halbe Dashboard steuert Cogs. Nach dem Vollumstieg gibt es keine.
Optionen: (a) Steuerungs-UI auf systemd-Units + Blocklist-Editor
umbauen, (b) Steuerung ganz weglassen (CLI/SSH reicht), (c) Interim:
Rust-Dashboard liefert Auth+Analytics, Python-8766 bleibt parallel auf
anderem Port für die Steuerung, bis der Python-Bot stirbt.
