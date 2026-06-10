# Phase 2: dl-bot-Gerüst — Broker, Changelog-Empfänger, Dispatcher, Gateway

Status: Code-komplett, verifiziert; KEIN Cutover (Bedingungen unten). Stand: 2026-06-10

## Architektur: Ports & Adapter

Die HTTP-Schichten kennen Discord nur über Traits — dadurch ohne Gateway testbar:

- `dl-broker::DiscordPort` — 14 Aktionen des Master-Brokers
- `dl-changelog::ChangelogDiscord` — Senden/Löschen/Datei/History für den Changelog-Dienst
- `dl-discord::DiscordAdapter` — implementiert beide via serenity:
  REST-Aktionen (senden, Rollen, Kanäle, Invites, DMs) funktionieren OHNE Gateway;
  Cache-Pfade (Voice-Member, Rollen-Mitglieder) melden ohne Gateway einen sauberen Fehler.
- `dl-discord::Dispatcher` — DER Event-Verteiler (broadcast): normalisierte
  `VoiceEvent`(Join/Leave/Move)- und `MessageEvent`-Kanäle. Domänen-Crates der
  Phasen 4+ subscriben hier, statt eigene Listener zu registrieren.

## API-Kompatibilität (verifiziert)

- Broker `/internal/master/v1/*`: Envelope `{ok, request_id, idempotency_key,
  cached, result, error}`, alle Fehlertexte, Loopback+Token-Auth, Allowlists
  (gleiche ENV-Namen), Idempotenz (TTL/Max/Inflight-Warten/409-Konflikt/504-Timeout).
  Gegen den LIVE-Python-Broker verglichen: Auth-Fehlerpfade byte-gleich
  (request_id normalisiert); Aktions-Happy-Paths via Mock-Port-Tests.
- Changelog `:8899`: `/changelog`, `/changelog/rich`, `/highlight-clips`,
  `/discord/messages` — Token-/Validierungs-/Fehlerverhalten identisch
  (gegen live verglichen, ohne Discord-Aktion auszulösen).
- Payload-Hash der Idempotenz ist bewusst NICHT byte-gleich zu Pythons
  `json.dumps(ensure_ascii=True)` — der Cache ist in-memory und
  prozess-lokal, nur die Konflikt-Semantik zählt.

## Bewusste Abweichungen

- `view_spec`-Embeds: Tiefenvalidierung des Embeds übernimmt Discord (502)
  statt `discord.Embed.from_dict` (400) — strukturelle Checks bleiben.
- Diagnose-Routen (`/discord/roles`, `/role-members`) brauchen den
  Gateway-Cache; Python chunkt on-demand, Rust liefert den Cache-Stand.

## Kritische Kopplung: Broker-Cutover ⇔ Interaction-Besitz

`view_spec: twitch_live_tracking` baut Buttons mit custom_id
`twitch-live:{login}:{token}`. Die KLICKS verarbeitet der Besitzer der
Gateway-Session (heute: Python, `TwitchLiveBridgeCog` + `bot.add_view`).

**Daraus folgt:** Der Broker-Cutover darf erst erfolgen, wenn auch die
Interaction-Verarbeitung umgezogen ist (Phase 3+), sonst posten Rust-Broker-
Nachrichten Buttons, deren Klicks niemand registriert hat. Gleiches gilt für
`bind_and_register_view`-Semantik (Re-Registrierung nach Restart).

## Gateway (user-gated)

- `DL_BOT_GATEWAY=1` startet die serenity-Session (Intents: Guilds, Members,
  VoiceStates, Messages+Content, Invites, DMs). Default AUS: Der Python-Bot
  hält die Session; zwei aktive Handler = doppelte Event-Verarbeitung.
- `DISCORD_TOKEN` (gleiche ENV wie Python) ist Pflicht für dl-bot;
  Broker-Token-Kette: MASTER_BROKER_TOKEN → MAIN_BOT_INTERNAL_TOKEN →
  TWITCH_INTERNAL_API_TOKEN (wie Original).

## Offen für den Phase-2/3-Cutover

1. Interaction-Routing in Rust (custom_id-Registry) — Phase 3.
2. Slash-Command-Sync-Strategie (Python COMMAND_SYNC_ON_START-Pendant).
3. Members-Chunking beim Gateway-Start (für Diagnose-Routen + Rollen-Sync).
4. Cutover-Checkliste analog 02 (Ports 8770/8899 übernehmen, Python-Broker
   stilllegen, Konsumenten-Smoke: Twitch-Bot health-Check).
