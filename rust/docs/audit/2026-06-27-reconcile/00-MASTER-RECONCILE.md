# Master — Re-Konziliations-Audit Py→Rust (2026-06-27)

**Methode:** 9 Subsystem-Worker (gpt-5.5/xhigh) vergleichen zeilengenau Python-live (`cogs/`, `bot_core/`, `service/`) ↔ aktueller Rust-Port (`rust/`). Bereits umgesetzte Wellen W1–W7 wurden **am Code + via `cargo test`** verifiziert (nicht am Commit-Text — Stale-Cache-Regel). Bewusste Rust-Änderungen (#17/#18, `bug_reporter`, blocklisted Cogs, Rang-Permission-Auslagerung) sind als DELIBERATE klassifiziert, nicht als Lücke.

**Referenz:** Python = LIVE & Source-of-Truth. Rust-Port = INAKTIV (`deadlock-bot-rust.service` aus). Ziel: volle Parität, damit Rust Python ablösen *kann*. Cutover bleibt separate User-Entscheidung.

## Ergebnis in Zahlen

| Severity | Anzahl |
|---|---|
| 🔴 critical | 0 |
| 🟠 high | 9 |
| 🟡 medium | 66 |
| ⚪ low | 55 |
| **gesamt** | **130** |

Detail je Subsystem in `0N-<subsystem>.md`. Severity-Verteilung: core 1H/4M/2L · moderation 0H/4M/3L · tempvoice 2H/22M/5L · voice-tracking 0H/6M/6L · onboarding-coaching 3H/5M/4L · faq-community 0H/13M/15L · stats-lfg 0H/4M/11L · tournament 1H/2M/6L · bridges 3H/8M/4L.

## 🟠 HIGH-Lücken (Implementierungs-Scope Welle 1)

| # | Subsystem | Python-Ref | Rust-Ref | Lücke | Aufw. | Cluster |
|---|---|---|---|---|---|---|
| H1 | core | `bot_core/control.py:33-118/342-437` | ABSENT (`dl-bot/main.rs`) | `!master status/restart/sync_commands` (Owner-Admin) fehlen komplett. `reload/discover/unload` = DELIBERATE (statisches Binary). | M | C1 |
| H2 | tempvoice | `cogs/tempvoice/interface.py:124-509` | `dl-voice/tempvoice/interface.rs:742-785` (Teil) | TempVoice-Control-Panels werden nicht erzeugt/gespeichert/rehydriert; `!tvpanel` fehlt; Lane-Embeds werden nicht aktualisiert. | M | C2 |
| H3 | tempvoice | `cogs/tempvoice/router_interface.py:137-257` | `dl-voice/router.rs:243-307` (Teil) | RouterInterface postet keine Anleitung/kein Spielmodus-Panel → Router-Einstieg unsichtbar ohne Alt-Message. | S | C2 |
| H4 | onboarding | `cogs/coaching_request.py:129-523` | `dl-community/coaching_requests.rs:1221-1488` (Handler da, Registrierung fehlt) | `coach_claim_*`/`coach_cancel_*`/`coach_`-Prefix-Handler nicht registriert → Coaching-Buttons tot. | M | C3 |
| H5 | onboarding | `cogs/coaching_survey.py:27-230` | `dl-community/coaching_requests.rs:884-1070` (toter Code) | `coaching_requests::spawn()` wird in `main.rs:630` nicht gestartet, `scan_survey_sessions` → `Vec::new()` → Voice-End-Survey-DM + Reward-Rolle feuern nie. | M | C3 |
| H6 | onboarding | `cogs/ai_onboarding.py:218-514` | ABSENT | AI-Onboarding-Flow (`aiob:rules_confirm`, Modal, Start-Button, persistente Views) nicht portiert. | M | C3 |
| H7 | tournament | `cogs/customgames/turnier.py:157-1332` | `dl-tournament/discord_ui.rs:341-454` (nur User-UI); Backend in `store.rs:750-975` unverdrahtet | `/turnier`-Admin-Dialog fehlt: Zeitraum erstellen/beenden, Anmeldungen mit Paging entfernen, Teams erstellen/löschen, alle Anmeldungen löschen. | L | C4 |
| H8 | bridges | `cogs/build_publisher.py:330-400` | `dl-bot/build_publisher.rs:248-270` | Monitor übernimmt keine `steam_tasks` DONE/FAILED → `uploaded_build_id`/`uploaded_version` nie gesetzt, Fehler nie final markiert, Jobs werden wieder pending. | M | C5 |
| H9 | bridges | `cogs/build_publisher.py:98-146` | `dl-bot/build_publisher.rs:74-185` | Readiness-Gate fehlt: Python pausiert bei fehlendem Steam-Login/GC-Ready; Rust queued `BUILD_PUBLISH` sobald Gateway aktiv → hängende/fehlschlagende Publishes. | S/M | C5 |

> Hinweis: H6 (AI-Onboarding) wurde im onboarding-Report als 3. High geführt. Vor Implementierung verifiziert der Worker, ob nicht bereits ein Äquivalent existiert.

**Zusatz-High aus bridges-Report (broker):** `/internal/master/v1/discord/channel-info` (loopback read) fehlt im Rust-Broker (`dl-broker/lib.rs`) → interne Clients bekommen 404 nach Cutover. Commit `3c52050` fügte den Endpoint nur dem **Python**-Broker zu. → Cluster **C5** (mit build-publisher, beide bridges/dl-bot-nah).

## Implementierungs-Wellenplan (sequenziell, ein Feature-Branch)

Worker laufen **sequenziell** im selben Working-Tree (Codex-Worker teilen sich das Arbeitsverzeichnis → paralleles Schreiben würde den Index zerstören). `main.rs` ist der gemeinsame Engpass (C1/C3/C5 verdrahten dort). Reihenfolge nach Wert + Risiko:

- **C1 — Core/Owner-Admin** (H1 + core-Mediums: Presence `N Cogs | !help`, Slash-Sync default-on+guild, Single-Instance-PID-Lock). Dateien: `dl-bot/main.rs`, `dl-discord/gateway.rs`, neues `master`-Modul.
- **C2 — TempVoice-Panels** (H2 + H3). Dateien: `dl-voice/tempvoice/interface.rs`, `dl-voice/router.rs`, ggf. `main.rs`-Command-Registrierung.
- **C3 — Coaching + AI-Onboarding** (H4 + H5 + H6). Dateien: `dl-community/coaching_requests.rs`, `dl-community/onboarding.rs`, `main.rs`-Spawn/Registrierung.
- **C4 — Turnier-Admin-Dialog** (H7). Dateien: `dl-tournament/discord_ui.rs`, `store.rs`.
- **C5 — Build-Publisher + Broker** (H8 + H9 + channel-info). Dateien: `dl-bot/build_publisher.rs`, `dl-broker/lib.rs`/`handlers.rs`.

Pro Cluster: Codex implementiert TDD (Test→Impl→`cargo build`/`clippy`/`test` grün) → Commit → frischer Codex-Kritiker reviewt → Codex-Rework bis sauber → Claude verifiziert externe Signale + merged. **User-sichtbare deutsche Texte** = Codex setzt `"Platzhalter"` + meldet Datei:Zeile, **Claude schreibt final**.

## 🟡 MEDIUM (66) + ⚪ LOW (55) — Backlog Welle 2+

Detail in den Fragment-Dateien (nicht hier dupliziert). Schwerpunkte:

- **tempvoice (22M):** Lane-Purge-Intervall, Lurker-Cleanup hinter `is_managed_lane`, Owner-Backfill-Regeln, Lane-Kategorie-Move, Router-Lane-Namen/Limits, Panel-Mod-Permissions, Duo/Trio-Templates, Preset save/load Port-Bug, MinRank-Select-Checks, Lane-Sortierung Comp/Ranked, Voice-Status-Slots `(X/Y)` + `MATCH_MINUTE_DISPLAY_OFFSET`, `dlvs`-Diagnose.
- **faq-community (13M):** `server_faq_logs` fehlt, `/faq`-Threads + `/faqclose`, Mod-Tag-Permission/Displayname, `website_invite` Zielkanal/Channel-Param, Privacy-Lösch-Zusammenfassung, Retention-Admin-Cmds, `/clips_repost`, Rules-Channel-Onboarding-Trigger.
- **bridges (8M):** live-bridge URL-Normalisierung, broker `resolve-user`/`edit-member`-404-Arme, steam_bridge `discord_name`-Forward + Panel-Channel-Param + Panel-Persistenz, `/changelog post`, dashboard Allowed-Origins-Seed.
- **voice-tracking (6M):** `voice_reaction_dm`-Cog komplett fehlt, Feedback-72h-Fenster, `!vtest`/`!vf1`/`!voice_status`/`!voice_config`, Nudge-DM-Restore, Rename-429-Handling.
- **moderation (4M):** Racism in Rust nicht verworfen, `persistent_ragebait` Mod-Tag, Proposal-Log-Embeds, Case-Persistenz (Attachments/`ai_raw_json`).
- **stats-lfg (4M):** `!smartping`, Join-Source-Retry, LFG-Rangauflösung, Offtopic-Voice-Skip.
- **tournament (2M):** Legacy-TeamBalancer-Panel, `!balance turnier*`-Subcommands.

LOW (55): überwiegend kosmetische Texte, Admin-Diagnose-Komfort, JSON-Typ-Koerzion an Web-Endpoints für fehlerhafte Clients, fehlende Embed-Formatierung. Niedrige Cutover-Priorität.

## Out-of-Scope / DELIBERATE (nicht Lücke)
`bug_reporter`, `player_finder`+`steam` (blocklisted), Coaching-Panel #17/#18, Cog-Hot-Reload, Rang-Permission-Kopplung (bewusst in rank-Port ausgelagert), `steam_verified_role` (Rust `friend_sync` Single-Owner). Details in den Fragment-Reports unter „DELIBERATE".
