# Phase 3: dl-bridges — Steam-Brücke, Twitch-Live-Bridge, Interaction-Dispatch

Status: 3a+3b code-komplett (Commit-Historie #73/#74); 3c offen. Stand: 2026-06-10

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

## Offen (3c)

1. `streamer_link_matcher` (6h-Scan + Approval-Flow + !twitch_link_scan).
2. `steam_link_voice_nudge` + `steam_verified_role` — fachlich Voice/DB-lastig,
   Umsetzung zusammen mit Phase 4 (Voice-Dispatcher) sinnvoller; Entscheidung
   beim 3c-Schnitt.
3. Cutover-Checkliste Bot-Seite (Ports 8770/8899, Gateway-Übergabe,
   Command-Sync, Blocklist-Einträge steam_bridge/twitch/changelog_publisher).
