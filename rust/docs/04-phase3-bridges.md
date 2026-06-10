# Phase 3: dl-bridges — Steam-Brücke, Twitch-Live-Bridge, Interaction-Dispatch, Matcher

Status: KOMPLETT (3a #73, 3b #74, 3c #75). Stand: 2026-06-10

## Was existiert

- **Interaction-Routing** (`dl-discord::interactions`): custom_id-Matcher
  (exakt/Präfix) + Slash-Command-Registry (liefert auch die Sync-Definitionen).
  Handler geben deklarative `BridgeReply` zurück (Text/Embeds/Components/Modal/
  Channel-Post) — dadurch ohne Discord testbar.
- **Gateway-Dispatch** (`dl-discord::dispatch`): interaction_create →
  Router → Raw-JSON-Response (type 4/5/9), 2-Sekunden-Defer-Schwelle wie das
  Original, Followup-Pfad, Panel-Posts (öffentlich + ephemere Bestätigung),
  Subcommand-Abflachung ("steam links"), Slash-Command-Sync
  (DL_BOT_COMMAND_SYNC=1, optional DL_BOT_COMMAND_GUILD_ID).
  **Unbekannte custom_ids werden ignoriert** — im Parallelbetrieb bedient der
  Python-Bot seine eigenen Views weiter.
- **Steam-Brücke** (`dl-bridges::steam`): kompletter Port von steam_bridge.py —
  Wire-Vertrag gegen Mock-Server feldgenau getestet.
- **Twitch-Live-Bridge** (`dl-bridges::twitch`): Klick-Tracking
  (POST /live/link-click, Idempotency-Key `twitch-live-click-{interaction_id}`),
  Start-Rehydrierung mit Backoff (GET /live/active-announcements),
  Registry statt `bot.add_view`. **Robuster als Python:** unbekannte Klicks
  laden die Ankündigungen einmal frisch nach, statt ins Leere zu laufen.

## Phase-2-Kopplung aufgelöst

Der `twitch-live:*`-Präfix-Handler verarbeitet jetzt die Klicks der vom
Broker geposteten Buttons. Damit können Broker (:8770) + Gateway gemeinsam
cutten, sobald Phase 3c fertig ist und die Cutover-Checkliste steht.

## 3c: Streamer-Link-Matcher (`dl-bridges::matcher`)

Port von streamer_link_matcher.py: Namens-Normalisierung (NFKD-Deaccent,
Leetspeak, Affix-Strip) und difflib-`SequenceMatcher.ratio` (Ratcliff/
Obershelp) sind in Rust nachgebaut und mit **CPython-Referenzwerten** als
Vertrags-Tests abgesichert. Scan-Kern hinter Ports (GuildPort/Notifier/
AiScorer) — ohne Discord testbar; Review-Buttons (`slm:link|reject:{token}`)
mit Mod-Guard, JSON-State (`data/streamer_link_state.json`, gleiche Struktur),
6h-Loop (erste Iteration übersprungen), `!twitch_link_scan` +
`!twitch_link_rescan_login`.

**Bewusste Lücke:** AI-Scoring (MiniMax) hängt an dl-ai (Phase 6) — bis dahin
Heuristik-Modus, identisch zum Original ohne AIConnector (nur eindeutige
Exakt-Treffer erreichen Auto-Link ≥ 90).

## Verschoben nach Phase 4/5

`steam_link_voice_nudge` (Voice-Events + DB) und `steam_verified_role`
(DB-Sweeps) sind fachlich Voice-/DB-Domänen — sie kommen mit dem
Voice-Dispatcher (Phase 4) bzw. der DB-Schicht-Erweiterung (Phase 5).

## Offen für den Bot-Cutover (eigene Checkliste vor Phase-4-Ende)

Ports 8770/8899 übernehmen, Gateway-Übergabe (DL_BOT_GATEWAY=1 + Python-
Blocklist), Command-Sync einmalig (DL_BOT_COMMAND_SYNC=1), Konsumenten-Smoke
(Twitch-Bot → Broker-health).
