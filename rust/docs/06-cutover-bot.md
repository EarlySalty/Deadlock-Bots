# Bot-Cutover: dl-bot übernimmt vom Python-Bot

Stand: 2026-06-10 (nach #98). Jeder Schritt ist user-gated — nichts hiervon
passiert ohne Nanis Freigabe.

## Was dl-bot beim Flip übernimmt

| Bereich | Module | Python-Cogs (Blocklist) |
|---|---|---|
| Broker :8770 + Changelog :8899 (inkl. /alert) | dl-broker, dl-changelog | master_broker, changelog_publisher |
| Steam-Bridge (14 Slash-Commands, Panels, Modal) | dl-bridges::steam | steam_bridge |
| Twitch-Live-Buttons + Klick-Tracking | dl-bridges::twitch | twitch/live_bridge |
| Streamer-Link-Matcher (6h + AI-Scoring) | dl-bridges::matcher + dl-ai | twitch/streamer_link_matcher |
| Voice-Session-Tracking | dl-voice::tracker | voice_activity_tracker |
| TempVoice (Join-to-create, Panel, Tag-Filter, Lurker, Min-Rang) | dl-voice::tempvoice | tempvoice/* (außer Router/Duo/NewPlayer, s. Lücken) |
| LiveMatch-Suffixe | dl-voice::status | deadlock_voice_status |
| Rang-Anker auf Comp-Lanes | dl-voice::rank | rank_voice_manager |
| Steam-Link-Nudge | dl-voice::nudge | steam_link_voice_nudge |
| Aktivitätsmuster + Co-Spieler | dl-activity::analyzer | user_activity_analyzer (Kern) |
| AI-Moderation (Scan-Kanal) | dl-moderation (Kern) | ai_moderator (Kern) |
| SecurityGuard (Takeover/Burst/Keyword) | dl-moderation::guard | security_guard (Kern) |
| Tag-System | dl-community::tags | tags/* |
| Coaching-Brücke (Roster + Termin-DMs) | dl-community::coaching | coaching_platform_sync |
| Onboarding-Buttons (Regeln→Rolle, Steam, DM-Hinweise) | bin: onboardglue | welcome_dm (Buttons) |

dl-web übernimmt zusätzlich: Stats :8768, Tierlist :8771,
**Turnier-Web :8767** (live-gedifft) — Blocklist: public_stats_cog,
tierlist_public_cog, turnier_public_cog.

## Flip-Reihenfolge (ein Wartungsfenster)

1. Release-Build: `cargo build --release -p dl-bot -p dl-web`.
2. systemd-Units anlegen (WorkingDirectory = Repo-Root, Env via Infisical
   wie der Python-Bot; dl-bot braucht zusätzlich DL_BOT_GATEWAY=1 und
   einmalig DL_BOT_COMMAND_SYNC=1 + DL_BOT_COMMAND_GUILD_ID).
3. Python: alle Blocklist-Cogs (Tabelle oben) in cog_blocklist.json
   eintragen, Bot stoppen.
4. dl-bot + dl-web starten (Original-Ports). Gateway-Doppelbesitz
   vermeiden: NIE beide gleichzeitig mit aktiven Events.
5. Smoke: /health auf 8767/8768/8771; Broker-Auth-Fehlpfad auf 8770;
   Steam-Panel-Klick; TempVoice-Join-to-create; ein Lane-Panel-Button.
6. Rollback: dl-Services stoppen, Blocklist-Einträge entfernen,
   Python-Bot starten. Kein Schema wurde geändert — gefahrlos.

## Bekannte Lücken beim Flip (bewusst, dokumentiert)

Funktional kleiner als das Original — Nutzer merken ggf.:
- **Router-/Duo-/NewPlayer-Lanes** + Panel `tv_mode_switch_*` (einzige
  offene Panel-Funktion).
- **Feedback-DMs** nach der ersten Voice-Session (+ vstats-Commands).
- **Onboarding-Kanal-Flow**: Schritt-Navigation antwortet mit
  Umbau-Hinweis; Regelbestätigung/Rolle funktioniert.
- **AI-Moderation**: Kontext-Backfill, Bild-Checks, Tone-Tag-Schwellen
  (konservativer: Vorschlag statt Auto-Aktion).
- **LFG-Antwort-Flow** (Erkennung+Scores portiert, Routing-Antworten
  fehlen) und **Player-Finder** (per Flag aus — gewollt).
- **FAQ-Chat, Clips, Leave-Survey, Bug-Reporter, Rules, Rename** —
  Python-Cogs weiterlaufen lassen (kein Konflikt, eigene Domänen).
- **Coaching-Discord-UI** (coaching_request/panel/survey) — Python
  weiterlaufen lassen; die Plattform-Brücke ist in Rust.
- **Turnier-Discord-UI** (turnier.py) + customgames-Flow — Python
  weiterlaufen lassen; Web + Store + Balancer sind in Rust.
- **Dashboard 8766** (Phase 9, nicht begonnen) — Python behält es.

→ Empfehlung: Teil-Cutover. Blocklist nur für die Tabelle oben; die
Lücken-Cogs laufen im Python-Bot weiter (er bleibt ohne Gateway-Konflikt
lauffähig, solange seine verbleibenden Cogs keine Voice/Message-Events
der portierten Domänen anfassen — Voice-Events braucht KEINER der
verbleibenden Cogs außer customgames/Router: vor dem Flip prüfen).

## Vor dem Flip noch bauen (Reihenfolge nach Risiko)

1. Router/Duo/NewPlayer-Lanes (einziger verbleibender Voice-Konsument).
2. Feedback-DM-System (user-sichtbar nach Cutover).
3. LFG-Routing-Antworten (user-sichtbar im LFG-Kanal).
