# Reconcile 04 — Voice-Tracking + Nudge + Rename + Reaction-DM

Datum: 2026-06-27  
Python-Referenz: `cogs/voice_activity_tracker.py`, `cogs/steam_link_voice_nudge.py`, `cogs/rename_manager.py`, `cogs/voice_reaction_dm.py`  
Rust-Referenz: `rust/crates/dl-voice/src/{tracker.rs,feedback.rs,stats.rs,nudge.rs,rename_queue.rs}`

## Kurzfazit

Der Kern von Voice-Tracking, Voice-Stats, Feedback-Freitext, Steam-Link-Nudge und Rename-Queue ist im aktuellen Rust-Code deutlich weiter als im Audit vom 2026-06-21. Insbesondere ist der bekannte High-Befund `dm_failed` im Nudge **nicht mehr reproduzierbar**: Rust markiert DM-Fehlschläge nicht mehr als `voice_nudge_done`.

Verbleibende echte GAPs: **0 high / 6 medium / 5 low**.

## GAPs

| Severity | Typ | Python-Ref | Rust-Ref | User-sichtbare/Korrektheits-Auswirkung | Aufwand |
|---|---|---|---|---|---|
| medium | missing-feature | `cogs/voice_reaction_dm.py:56-285` | ABSENT (`rg human_notify_pending_at/VOICE_REACTION_DM rust` ohne Treffer) | Das komplette Voice-Reaction-DM-Cog fehlt. Wenn `VOICE_REACTION_DM_ENABLED` aktiv ist, pollt Python alle 30s `twitch_partner_outreach_conversations`, schickt Owner-DMs fuer Sales-Leads und setzt `human_notify_sent_at`; Rust schickt diese Benachrichtigungen nie. Wegen Feature-Flag default-off medium statt high. | M |
| medium | divergent-logic | `cogs/voice_activity_tracker.py:193-218` | `rust/crates/dl-voice/src/feedback.rs:397-424` | Feedback-Button prueft in Rust weder `status='responded'` noch das 72h-Antwortfenster. Python zeigt bei beantworteten Requests "schon angekommen" und bei Ablauf "Fenster ist abgelaufen"; Rust oeffnet fuer alte `sent/pending` Requests weiter ein Modal bzw. liefert nur generisch "nicht mehr offen". | S |
| medium | missing-feature | `cogs/voice_activity_tracker.py:1536-1743` | ABSENT (`stats.rs:227-240` registriert nur `!vstats`, `!vleaderboard`/Aliasse) | Admin-/Diagnosebefehle `!vtest`, `!vf1`, `!vf4`, `!voice_status`, `!voice_config` fehlen. Admins koennen Feedback-Prompts nicht manuell testen und Voice-Tracker-Konfiguration nicht per Discord zur Laufzeit aendern. | M |
| medium | missing-feature | `cogs/steam_link_voice_nudge.py:354-405`, `cogs/steam_link_voice_nudge.py:467-513` | `rust/crates/dl-voice/src/nudge.rs:314-316`, `rust/bin/dl-bot/src/main.rs:128-135` | Nudge-DM-Restore/Refresh fehlt. Python editiert gespeicherte DMs beim Start und bei aktiven States mit frischer Steam-URL/View-Version; Rust registriert nur den Close-Button. Alte Nudge-DMs behalten dadurch potentiell abgelaufene Steam-Login-Buttons. | M |
| medium | divergent-logic | `cogs/rename_manager.py:311-325` | `rust/crates/dl-voice/src/rename_queue.rs:211-225`, `rust/crates/dl-voice/src/glue.rs:1252-1263` | Rename-Worker behandelt HTTP 429 nicht explizit. Python liest `Retry-After`, aktualisiert den per-Channel-Throttle und schlaeft gezielt; Rust reduziert Discord-Fehler auf `String` und requeued generisch, wodurch Retry-After/last-attempt nicht respektiert werden koennen. Serenity kann Teile intern abfedern, aber die Queue-Semantik ist nicht paritaer. | M |
| medium | divergent-logic | `cogs/steam_link_voice_nudge.py:562-629` | `rust/crates/dl-voice/src/nudge.rs:211-276` | Der normale Nudge-Versand prueft in Rust keinen vorhandenen `steam_nudge_state` und kann bestehende DMs nicht editieren/auffrischen. Python unterscheidet normal vs. `force=True`, fetched vorhandene Messages und sendet im Normalfall nicht erneut; Rust nutzt denselben Direktversand fuer Normal- und Testpfad. Praktisch meist durch Join-Gates abgefangen, aber Race-/Refresh-Verhalten weicht ab. | M |
| low | behavioral-diff | `cogs/voice_activity_tracker.py:757-788`, `cogs/voice_activity_tracker.py:842` | `rust/crates/dl-voice/src/feedback.rs:242-250` | Python loescht vor einem neuen Feedback-Prompt alle alten Requests/Responses und versucht alte DMs zu entfernen. Rust loescht nur `status='pending'`; alte `sent/responded` Requests und klickbare Prompt-DMs bleiben liegen. | S |
| low | behavioral-diff | `cogs/voice_activity_tracker.py:138-171`, `cogs/voice_activity_tracker.py:995-1008` | `rust/crates/dl-voice/src/feedback.rs:472-478` | Modal-Feedback-Forward an den Owner enthaelt in Rust weniger Kontext. Python leitet Request-Typ, Kanal, Dauer und Co-Player mit; Rusts Modalpfad sendet nur User/Request-ID plus Antwort. Freitext-DM-Forward ist dagegen inzwischen kontextreich portiert. | S |
| low | behavioral-diff | `cogs/voice_activity_tracker.py:187-192` | `rust/crates/dl-voice/src/glue.rs:941-949` | Feedback-Button-Label weicht ab: Python `Feedback ausfuellen`, Rust `Feedback geben`. Rein kosmetisch, aber user-sichtbar. | S |
| low | behavioral-diff | `cogs/rename_manager.py:177-185`, `cogs/rename_manager.py:198-240` | `rust/crates/dl-voice/src/rename_queue.rs:254-286`, `rust/crates/dl-voice/src/rename_queue.rs:295-310` | Diagnosefelder werden in Rust schlechter gepflegt: `assigned_worker_id` bleibt beim Claim/Done leer und `last_error` wird bei Throttle-/Retry-Pending nicht gesetzt. Funktional unkritisch im Single-Worker-Prozess, aber Admin-Debugging weicht ab. | S |
| low | behavioral-diff | `cogs/steam_link_voice_nudge.py:721-744` | `rust/crates/dl-voice/src/nudge.rs:337-338`, `rust/crates/dl-voice/src/nudge.rs:596-601` | `!nudgesend` ohne Ziel nutzt in Python optional `NUDGE_TEST_DEFAULT_ID`; Rust nimmt immer den Aufrufer. Das betrifft nur Admin-Testkomfort. | S |

## Verifizierte Parity / stale Altbefunde

| Befund | Ergebnis | Beleg |
|---|---|---|
| Nudge-DM-Fehlschlag markiert User dauerhaft als genudged | **stale / erledigt** | Python setzt `_mark_nudge_done` nur nach erfolgreichem DM-Versand (`steam_link_voice_nudge.py:614-637`). Aktueller Rust-Code setzt `DONE_NS` nur im `Ok`-Zweig (`nudge.rs:242-266`), im `Err`-Zweig nicht (`nudge.rs:272-275`). Test `dm_fehlschlag_markiert_nicht_done` verifiziert das (`nudge.rs:537-544`). |
| Feedback-DM-Freitextantwort fehlt | **stale / erledigt** | Rust hat `handle_dm_message` mit DM-Filter, 72h-Window, DB-Insert, `status='responded'`, Owner-Forward und Ack (`feedback.rs:288-386`) und registriert den Message-Subscriber in `main.rs:520-525`. |
| Owner-Forward fuer Freitext enthaelt zu wenig Kontext | **stale / erledigt fuer Freitext** | Rust-Freitextforward enthaelt Request-ID, Typ, Autor, Kanal, Dauer und Co-Player (`feedback.rs:365-381`). Nur der Modalpfad bleibt reduziert, siehe Low-GAP oben. |
| Opt-out/Exempt-Recheck nach 30-Minuten-Wait fehlt | **kein Paritaets-GAP** | Python prueft Opt-out/Exempt vor `_count_voice_minutes` und nach dem Wait nur Steam-Link (`steam_link_voice_nudge.py:647-664`). Rust prueft Opt-out/Exempt im Join-Gate und nach dem Wait ebenfalls nur Steam-Link (`nudge.rs:153-208`). |

## Stichproben-Parity

- Voice-Session-Kern: Config aus `voice_cfg`, Opt-out-Skip, Grace, Punkteberechnung, `voice_stats`-Upsert und `voice_session_log`-Insert sind portiert (`tracker.rs:176-207`, `tracker.rs:247-548`).
- Stats-Commands: `!vstats`, `!vleaderboard`, `!vlb`, `!voicetop` existieren mit Rate-Limit und Live-Session-Zuschlag (`stats.rs:227-345`). Zielaufloesung ist enger als Python-MemberConverter (nur Mention/ID), aber niedrige Relevanz.
- Rename-Queue-Kern: Schema, last-wins-Enqueue, FIFO-Claim, 360s-Throttle, Retry bis `MAX_RETRIES` und Crash-Recovery sind vorhanden (`rename_queue.rs:78-133`, `rename_queue.rs:153-227`).
- Nudge-Kernflow: Tag-2-Gate, 30-Minuten-Watch, Opt-out/Exempt/Steam-Link/done/active-Gates, DM-Embed+Buttons, Close-Button und `!nudgesend`/`!t30` sind vorhanden (`nudge.rs:145-208`, `nudge.rs:211-360`).
