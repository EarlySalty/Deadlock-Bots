# Core / Bootstrap / Dispatch / Owner-Admin Reconcile (2026-06-27)

## Kurz-Summary

Aktueller Stand: Die alten Core-Befunde sind im aktuellen Rust-Code groesstenteils weiterhin offen. Event-Normalisierung, zentraler Dispatcher und Interaction-Routing sind vorhanden und absichtlich anders als Python strukturiert. Die portierbaren Owner-Admin- und Bootstrap-Defaults fehlen aber weiter.

GAP-Zaehler: **1 high / 3 medium / 1 low**.

| Severity | Anzahl |
|---|---:|
| high | 1 |
| medium | 3 |
| low | 1 |

## GAPs

| Severity | Typ | Python-Ref | Rust-Ref | User-sichtbare/Korrektheits-Auswirkung | Aufwand |
|---|---|---|---|---|---|
| high | missing-feature | `bot_core/control.py:33-118` (`!master status`), `bot_core/control.py:342-368` (`!master restart`), `bot_core/control.py:370-437` (`!master sync_commands`) | `ABSENT` (`rust/bin/dl-bot/src/main.rs:385-391`, `443-445`, `488`, `514`, `519`, `605` registrieren andere Prefix-Listener; `rg "MasterControl|master status|master sync|master restart"` ohne Rust-Treffer) | Die portierbaren Owner-Admin-Befehle der `!master`/`!m`-Gruppe sind weiter nicht vorhanden: Status-Embed, sauberer In-Process/Service-Restart-Trigger und kontrollierter App-Command-Sync per Discord. Owner verlieren damit die wichtigsten Runtime-Ops-Funktionen beim Cutover. | M |
| medium | missing-feature | `main_bot.py:120-251`, `main_bot.py:373-378` | `ABSENT` (`rust/bin/dl-bot/src/main.rs:36-43` startet direkt ohne Lock; `rust/bin/dl-bot/src/main.rs:689-698` beendet nur via `ctrl_c`/Task-Abort) | Kein Single-Instance-PID-Lock: Eine zweite Rust-Instanz wird nicht hart verhindert. Bei Fehlstart/manuellem Parallelstart drohen doppelte Gateway-Verarbeitung, Broker-/Changelog-Port-Konflikte und Discord-Session-Races. | S |
| medium | behavioral-diff | `bot_core/presence.py:29-40`, `bot_core/presence.py:94-110` | `ABSENT` (`rust/crates/dl-discord/src/gateway.rs:32-35` READY setzt nur `gateway_ready` und loggt; keine `change_presence`/Activity-API-Treffer im Rust-Baum) | Bot-Presence `Watching N Cogs | !help` fehlt weiterhin. User sehen nach Rust-Cutover keine angepasste Activity im Member-Listing; auch nach statischer Registrierung gibt es keinen Ersatztext. | S |
| medium | behavioral-diff | `bot_core/master_bot.py:386-579`, `bot_core/master_bot.py:635-640` | `rust/bin/dl-bot/src/main.rs:660-667`, `rust/crates/dl-discord/src/dispatch.rs:400-418` | Slash-Command-Sync ist weiter opt-in und faellt ohne `DL_BOT_COMMAND_GUILD_ID` auf globalen Bulk-Sync zurueck. Python synct beim Start default-on mit Default-Scope `guild`, inklusive Hash-/State-Skip und `disabled/hybrid/always`-Modus; Rust synct nur bei `DL_BOT_COMMAND_SYNC=1`, sonst gar nicht. Neue/veraenderte Slash-Commands koennen dadurch beim Cutover fehlen oder global erst verzoegert erscheinen. | M |
| low | missing-feature | `bot_core/lifecycle.py:62-79`, `bot_core/lifecycle.py:155-235` | `ABSENT` (`rust/bin/dl-bot/src/main.rs:683`, `rust/bin/dl-bot/src/main.rs:689-698`) | Kein Python-artiger Lifecycle-Supervisor mit Restart-Event, Crash-Backoff und Login-Failure-Circuit-Breaker. Systemd kann Prozess-Restarts abdecken, ersetzt aber weder `!master restart` noch das Login-Failure-Limit im Prozess. | M |

## PARITY / OK

| Bereich | Python-Ref | Rust-Ref | Ergebnis |
|---|---|---|---|
| Event-Normalisierung Message/Voice | `bot_core/presence.py:58-92` (paralleler Voice-Router) | `rust/crates/dl-discord/src/gateway.rs:50-127`, `rust/crates/dl-discord/src/gateway.rs:222-259`, `rust/crates/dl-discord/src/dispatcher.rs:141-187` | ✅ PARITY / Rust-besser strukturiert: ein zentraler Gateway-Handler normalisiert Events und broadcastet an Subscriber. |
| Interaction-Routing | discord.py Views/App-Commands implizit ueber Cogs | `rust/crates/dl-discord/src/interactions.rs:149-205`, `rust/crates/dl-discord/src/dispatch.rs:31-79` | ✅ PARITY fuer Rust-Architektur: zentrale Registry speist Dispatch und Command-Sync. |
| Guild-vs-global Sync-Faehigkeit | `bot_core/master_bot.py:463-518` | `rust/crates/dl-discord/src/dispatch.rs:400-418` | ✅ Teil vorhanden: Rust kann guild-scoped syncen, wenn `guild_id` gesetzt ist. Der Gap liegt im Default/Gating, nicht in der Sync-Primitive. |

## DELIBERATE / Nicht als GAP gewertet

| Bereich | Quelle | Bewertung |
|---|---|---|
| `!master reload`, `reloadall`, `reloadsteam`, `discover`, `unload`, `unloadtree` | Shared Brief `00-SHARED-BRIEF.md`: Cog-Hot-Reload/Discover/Unload im statischen Rust-Binary konzeptionell nicht abbildbar; nur `status`/`restart`/`sync_commands` zaehlen als Luecke. Python-Anker: `bot_core/control.py:120-340`. | 🔵 DELIBERATE. Nicht als GAP gewertet. |
| Cog-Auto-Discovery und Blocklist | Shared Brief + Rust statische Registrierung in `rust/bin/dl-bot/src/main.rs:74-383`. | 🔵 DELIBERATE. Statisches Binary ersetzt Runtime-Cog-Discovery; keine Hot-Reload-Paritaet erwartet. |
| Gateway default-gated | `rust/bin/dl-bot/src/main.rs:1-7`, `rust/bin/dl-bot/src/main.rs:425-427`, `rust/crates/dl-discord/src/lib.rs:10-11` | 🔵 DELIBERATE waehrend Strangler-Phase: Python haelt bis Cutover die Gateway-Session. |

## Top-5 Luecken

1. **high:** Portierbare `!master`-Funktionen `status`, `restart`, `sync_commands` fehlen komplett.
2. **medium:** Kein PID-/Single-Instance-Lock im Rust-Bootstrap.
3. **medium:** Bot-Presence `N Cogs | !help` wird in Rust nicht gesetzt.
4. **medium:** Slash-Command-Sync bleibt opt-in und defaultet ohne Guild-ID auf global.
5. **low:** Lifecycle-Supervisor mit Backoff/Login-Failure-Limit fehlt.

