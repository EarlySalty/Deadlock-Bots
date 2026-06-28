# Bot-Cutover: dl-bot übernimmt vom Python-Bot

Stand: 2026-06-10 (nach #114). Jeder Schritt ist user-gated — nichts hiervon
passiert ohne Nanis Freigabe.

> **Update 2026-06-28:** Die Turnier-Domäne (`dl-tournament`, Turnier-Web `:8767`,
> `/turnier`-Admin, `!balance`) wurde entfernt — das Turniersystem läuft im separaten
> Repo **Deadlock-Turniere**. Turnier-Zeilen unten sind historischer Plan-Stand und
> kein Cutover-Ziel mehr. KEEP: OAuth-/Broker-/Tierlist-Bridge zu Deadlock-Turniere.

## Was dl-bot beim Flip übernimmt

| Bereich | Module | Python-Cogs (Blocklist) |
|---|---|---|
| Broker :8770 + Changelog :8899 (inkl. /alert) | dl-broker, dl-changelog | master_broker, changelog_publisher |
| Steam-Bridge (14 Slash-Commands, Panels, Modal) | dl-bridges::steam | steam_bridge |
| Twitch-Live-Buttons + Klick-Tracking | dl-bridges::twitch | twitch/live_bridge |
| Streamer-Link-Matcher (6h + AI-Scoring) | dl-bridges::matcher + dl-ai | twitch/streamer_link_matcher |
| Voice-Session-Tracking | dl-voice::tracker | voice_activity_tracker |
| TempVoice komplett (Panel 100 %, Tag-Filter, Lurker, Min-Rang, Mode-Switch) | dl-voice::tempvoice | tempvoice/core+interface+util |
| Lane-Router + Adaptive Lanes (NewPlayer/Duo/Sortierung) | dl-voice::router + ::adaptive | tempvoice/router*+duo_lanes+new_player_lanes+lane_sorting |
| Feedback-DMs (Erst-/Zweit-Session) | dl-voice::feedback | (Teil von voice_activity_tracker) |
| Website-Invites (Lifecycle) | dl-community::invites | website_invite_cog |
| LiveMatch-Suffixe | dl-voice::status | deadlock_voice_status |
| Rang-Anker auf Comp-Lanes | dl-voice::rank | rank_voice_manager |
| Steam-Link-Nudge | dl-voice::nudge | steam_link_voice_nudge |
| Aktivitätsmuster + Co-Spieler | dl-activity::analyzer | user_activity_analyzer (Kern) |
| AI-Moderation (Scan-Kanal) | dl-moderation (Kern) | ai_moderator (Kern) |
| SecurityGuard (Takeover/Burst/Keyword) | dl-moderation::guard | security_guard (Kern) |
| Tag-System | dl-community::tags | tags/* |
| Coaching-Brücke (Roster + Termin-DMs) | dl-community::coaching | coaching_platform_sync |
| Onboarding-Buttons (Regeln→Rolle, Steam, DM-Hinweise) | bin: onboardglue | welcome_dm (Buttons) |
| Leave-Survey (Exit-DM, Bucket A/B/C) | dl-community::leave_survey | leave_survey |
| Clip-Einsendungen (Interface, Wochenfenster, Dump) | dl-community::clips | clip_submission |
| FAQ-Chat + Ticket-Auto-Helfer | dl-community::faq + dl-ai | faq_chat |
| Onboarding-Wizard (10 Schritte im Thread) | dl-community::onboarding | rules_channel + onboarding (Kern) |
| Coaching-Anfragen (Panel→AI→Rotation→Claim) | dl-community::coaching_requests | coaching_panel + coaching_request |
| LFG-Lobby-Finder (durchgängig) | dl-activity::lfg | lfg |
| Nachrichten-Zähler + Beitritts-Protokoll | dl-activity::analyzer | (Teile von user_activity_analyzer) |

dl-web übernimmt zusätzlich: Stats :8768, Tierlist :8771 — Blocklist:
public_stats_cog, tierlist_public_cog. (Turnier-Web :8767 / turnier_public_cog
und die Turnier-Anmeldung wurden 2026-06-28 entfernt.)

## Flip-Reihenfolge (ein Wartungsfenster)

1. Release-Build: `cargo build --release -p dl-bot -p dl-web`.
2. systemd-Units anlegen (WorkingDirectory = Repo-Root, Env via Infisical
   wie der Python-Bot; dl-bot braucht zusätzlich DL_BOT_GATEWAY=1 und
   einmalig DL_BOT_COMMAND_SYNC=1 + DL_BOT_COMMAND_GUILD_ID).
3. Python: alle Blocklist-Cogs (Tabelle oben) in cog_blocklist.json
   eintragen, Bot stoppen.
4. dl-bot + dl-web starten (Original-Ports). Gateway-Doppelbesitz
   vermeiden: NIE beide gleichzeitig mit aktiven Events.
5. Smoke: /health auf 8768/8771; Broker-Auth-Fehlpfad auf 8770;
   Steam-Panel-Klick; TempVoice-Join-to-create; ein Lane-Panel-Button.
6. Rollback: dl-Services stoppen, Blocklist-Einträge entfernen,
   Python-Bot starten. Kein Schema wurde geändert — gefahrlos.

## Bekannte Lücken beim Flip (bewusst, dokumentiert)

Funktional kleiner als das Original — Nutzer merken ggf.:
- **vstats-/vleaderboard-Text-Commands** des Trackers (Anzeige-Only).
- **Onboarding-Kanal-Flow**: Schritt-Navigation antwortet mit
  Umbau-Hinweis; Regelbestätigung/Rolle funktioniert.
- **AI-Moderation**: Kontext-Backfill, Bild-Checks, Tone-Tag-Schwellen
  (konservativer: Vorschlag statt Auto-Aktion).
- **LFG-Antwort-Flow** (Erkennung+Scores portiert, Routing-Antworten
  fehlen) und **Player-Finder** (per Flag aus — gewollt).
- **step_streamer-Formular** (Streamer-Setup im Onboarding) — der
  Wizard-Schritt informiert und verweist; das Formular bleibt Python.
- **Coaching-Nebenpfade** (role_manager 48h-Ablauf, survey,
  Website-Mirror) — Python weiterlaufen lassen; Kern ist in Rust.
- **Turnier-Admin-Flow** (Zeiträume/Teams/Panel-Post) + customgames-
  Team-VCs — Python weiterlaufen lassen; User-Flow/Web/Store in Rust.
- **Dashboard 8766** (Phase 9, nicht begonnen) — Python behält es.

**WICHTIG — user_activity_analyzer MUSS geblocklistet werden:** Sein
10-min-Co-Spieler-Tracker und der Rust-Tracker schreiben beide
inkrementell in `user_co_players` — parallel laufen = Doppelzählung.
Rust übernimmt Patterns + Co-Spieler + member_events-Basis (join/leave);
Interim-Lücken bis zum Analyzer-Rest-Port: Invite-Attribution der Joins
(zählen als „Unbekannt"), Text-Sessions, Retention-Hooks.

→ Empfehlung: Teil-Cutover. Blocklist nur für die Tabelle oben; die
Lücken-Cogs laufen im Python-Bot weiter (er bleibt ohne Gateway-Konflikt
lauffähig, solange seine verbleibenden Cogs keine Voice/Message-Events
der portierten Domänen anfassen — Voice-Events braucht KEINER der
verbleibenden Cogs außer customgames/Router: vor dem Flip prüfen).

**Nicht portiert (bewusst):** `bug_reporter` — das ist keine reine
Ticket-Annahme, sondern ein Codex-Selbstreparatur-System (Auto-Restart,
Cog-Reload via lokalem Codex-CLI mit gpt-4o-mini/gemini). Es ist ans
Python-Runtime-Modell gebunden (Cog-Reload existiert in Rust nicht),
OpenAI ist auf diesem Host nicht konfiguriert und die Codex-Auth des
gpt-Users ist abgelaufen — faktisch inaktiv. Vor einem Port braucht das
ein Redesign (z. B. MiniMax + systemd-Restart statt Cog-Reload).

**Nicht portiert (ersetzt):** `rename_manager` (DB-Rename-Queue mit
1s-Worker) — die Rust-Module rennen Kanäle direkt um und tragen ihre
Rename-Disziplin selbst (Voice-Status 360/600s-Cooldown, Rank-Manager
60s, TempVoice Create-Fenster). Der Python-rename_manager kann beim
Teil-Cutover weiterlaufen (seine Queue-Füller sind geblocklistet).

## Vor dem Flip noch bauen

Keine Blocker mehr — alle drei ursprünglichen Voraussetzungen (Router-
Lanes, Feedback-DMs, LFG-Routing) sind gebaut. Optional vor dem Flip:
LFG-Antwort-Embed (sonst bleibt lfg.py in Python aktiv — Voice-frei,
kein Konflikt).
