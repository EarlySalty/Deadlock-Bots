status: aktiv 2026-08-27

# Research: Discord Verify-Gate

Belege in EVIDENCE.md. Hier nur die Schlussfolgerungen.

## Vorhanden, wiederverwendbar
- Rollen vergeben/entfernen: `DiscordAdapter::add_role/remove_role` (adapter.rs:610/656).
- Join-Signal inkl. Kontoalter und Invite-Quelle: `MemberEvent::Join` (dispatcher.rs:135), `classify_join_source` (invites.rs:33). Join-Quelle liegt im `metadata`-JSON.
- DM senden: `send_dm_embed` (modglue.rs:3189). DM empfangen: `MessageEvent` mit `guild_id == None`, `subscribe_messages()`. Button/Component: Router in `interactions.rs`, Registrier-Vorbild `onboardglue.rs`.
- LLM: `dl_ai::generate_text` (lib.rs:120), Default DeepSeek v4 Flash (lib.rs:42). Judge-Muster: `moderation_system.rs` (zweistufig, JSON-Verdict, fail-Logik).
- PG: sqlx-`query!`-Muster (invites.rs:171); Migrations-Ordner mit Datumsschema.
- Scheduler: `spawn()` plus `loop { sleep }`, Vorbild `retention.rs:1023`, verdrahtet in `main.rs`.
- Heronamen: `hero_catalog(pool)` aus `tierlist.deadlock_heroes` (data.rs:40), `slugify_hero` (data.rs:14) fuer normalisierten Vergleich.

## Luecken (neu zu bauen)
1. Quarantaene-Rolle ("Ban ohne Ban") existiert nicht. Rolle plus View-Channel-Deny muss eingerichtet werden.
2. Guild-Kick fehlt im `DiscordAdapter`, muss ergaenzt werden (`http.kick_member`).
3. Persistenz zu duenn: `bot.onboarding_pending_verify` hat kein guild_id/attempts/deadline/state. Neue Tabellen noetig.
4. Der Verify-Judge (Prompt, JSON-Verdict, fail-open) ist noch nicht gebaut; `moderation_system.rs` ist die Vorlage.
5. Der gesamte DM-Dialog (Start-Button, Frage, Antwortbewertung, Versuchszaehlung) und der Frist-Kick-Scheduler fehlen.

## Design-Festlegungen aus dem Bestand
- Neues Modul `dl-community/src/verify_gate.rs` fuer die Gate-Logik, verdrahtet in `dl-bot/src/main.rs` per `spawn()`, analog retention/leave_survey.
- Zwei neue PG-Tabellen: `bot.verify_gate_pending` (Zustand pro pending Nutzer) und `bot.verify_gate_kicked` (Rejoin-frei nach Kick).
- Quarantaene-Rolle-ID, Schwelle (30 Tage), Frist (24 h), Versuche (3) und ein Betriebs-Schalter kommen in die Config-Datei (nicht ENV, nicht hart verdrahtet).
