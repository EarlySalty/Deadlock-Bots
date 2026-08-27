status: aktiv 2026-08-27

# Plan: Discord Verify-Gate

Ziel und Anforderungen: siehe CONTRACT.md. Belege: EVIDENCE.md. Klasse hoch, User reviewt diesen Plan vor dem Bau.

## Zielort im Code
- Gate-Logik: neues Modul `rust/crates/dl-community/src/verify_gate.rs`.
- Store: `rust/crates/dl-central-db` (sqlx-Funktionen) plus Migration.
- Kick: `rust/crates/dl-discord/src/adapter.rs`.
- Verdrahtung, Config, Interaction-Route: `rust/bin/dl-bot/src/`.

## Milestones

### M1 Persistenz
- Migration `2026082701_verify_gate.sql`: `bot.verify_gate_pending (guild_id, user_id, dm_channel_id, attempts INT DEFAULT 0, state TEXT, created_at, deadline_at)`, PK (guild_id, user_id); `bot.verify_gate_kicked (guild_id, user_id, kicked_at)`, PK (guild_id, user_id).
- Store-Funktionen: `upsert_pending`, `get_pending`, `incr_attempt`, `set_state`, `delete_pending`, `list_expired(now)`, `mark_kicked`, `was_kicked`.
- Validierung: `cargo sqlx prepare` gruen, `cargo build -p dl-central-db`, Unit-Test Insert/Select/Expiry.
- Stop-Regel: Migration bricht Fresh-Schema-Test -> Migration fixen, nicht weiter.

### M2 Guild-Kick im Adapter
- `DiscordAdapter::kick(guild_id, user_id, reason)` via `http.kick_member`, Unknown-Member idempotent auf Ok mappen.
- Validierung: `cargo build -p dl-discord`, Kompiliertest.

### M3 Heroliste-Abgleich
- Funktion `answer_matches_hero(pool, text) -> bool`: `hero_catalog(pool)`, normalisierter Vergleich (slugify, Teilstring auf Wortgrenze).
- Validierung: Unit-Test "Nenne Abrams" besteht, "banana" nicht.

### M4 Verify-Judge
- `judge_answer(text) -> Verdict{pass, unsure}` ueber `dl_ai::generate_text`, Prompt: beschreibt der Text plausibel einen Deadlock-Hero UND ist er auf Deutsch. Striktes JSON. Timeout/Fehler/unsure -> fail-open (pass). Modell aus Config-Default (DeepSeek v4 Flash), nie hart.
- Validierung: Unit-Test mit FixedProvider (deutsche Hero-Beschreibung pass, englischer Satz fail, Provider-Fehler pass).

### M5 Quarantaene-Rolle einrichten
- Idempotenter Setup beim Start: Rolle sicherstellen (ID aus Config, sonst `create_role`), auf allen Kategorien der Guild `VIEW_CHANNEL` fuer diese Rolle denyen. Verify-DM-Weg bleibt erreichbar (DM ist kanalunabhaengig).
- Validierung: Live-Check mit Wegwerf-Account: Rolle vergeben -> sieht keine Kanaele.

### M6 Gate-Orchestrierung (verify_gate.rs)
- Join-Loop `subscribe_members()`: Filter is_bot=false, Konto < 30 Tage, Invite-Quelle nicht "Persoenlicher Invite", `was_kicked=false`. Trifft zu -> `add_role`(Quarantaene) + Start-DM mit Button `verify:start`; `upsert_pending(state=awaiting_start, deadline=now+24h)`. DM nicht zustellbar -> sofort `kick` + `mark_kicked`.
- Interaction `verify:start` -> Frage-DM "Nenne einen Hero aus Deadlock", `set_state(awaiting_answer)`.
- DM-Message-Loop (guild_id None) fuer pending awaiting_answer: M3 sonst M4. Pass -> `remove_role` + Freischalt-DM + `delete_pending`. Fail -> `incr_attempt`; bei 3 -> `kick` + `mark_kicked` + `delete_pending`.
- Validierung: Unit-Tests der Entscheidungslogik (pass/fail/3-fail-kick/kein-gate-bei-personal-invite/kein-gate-bei-was_kicked).

### M7 Frist-Kick-Scheduler
- `spawn()` mit `loop { sleep(300s); for p in list_expired(now) { kick + mark_kicked + delete_pending } }`.
- Validierung: Unit-Test: pending mit deadline in Vergangenheit wird als expired gelistet.

### M8 Verdrahtung und Config
- Config-Block `verify_gate`: `guild_id`, `quarantine_role_id`, `max_account_age_days=30`, `deadline_hours=24`, `max_attempts=3`, `enforce=true` (Betriebs-Notaus, Config nicht ENV; enforce=false = Rolle vergeben aber nicht kicken).
- `main.rs`: `spawn(verify_gate)`, Interaction-Route `verify:` registrieren.
- Validierung: `cargo build` gesamt, `cargo test` (bekannte rote Baseline abgleichen), `cargo clippy`.

## DM-Texte (Orchestrator, du-Form, echte Umlaute)
- Start: "Willkommen. Dein Discord-Konto ist noch recht neu, darum eine kurze Sicherheitsfrage, damit wir Bots und Scam-Accounts draussen halten. Klick auf Verifizieren, um zu starten." plus Button "Verifizieren".
- Frage: "Nenne einen Hero aus Deadlock. Antworte einfach hier in einem kurzen deutschen Satz."
- Fail (Versuch offen): "Das hat noch nicht gepasst. Versuch es bitte noch einmal mit einem Hero aus Deadlock, auf Deutsch."
- Pass: "Danke, du bist freigeschaltet. Viel Spass in der Community."

## Abnahme (vor Scharfschaltung)
- Wegwerf-Account joint ueber Vanity/Discovery: bekommt DM, Quarantaene, sieht keine Kanaele; korrekte Antwort schaltet frei; falsche 3x kickt; kein Invite-Weg umgeht das Gate; echter Bestand bleibt unberuehrt.
- Danach Deploy, systemctl Restart, Live-Beweis, Branch/Worktree loeschen.

## Offene Build-Entscheidungen (an User)
1. Freischalt-DM nach bestandener Antwort: ja (Plan) oder wortlos freischalten.
2. Betriebs-Notaus `enforce` in der Config: ja (Plan) oder ganz ohne Schalter scharf.
3. Quarantaene-Rolle: Bot richtet sie samt Kanal-Deny automatisch ein (Plan) oder du legst Rolle plus Rechte manuell an und gibst mir nur die ID.
