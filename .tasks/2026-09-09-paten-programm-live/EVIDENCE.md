# Evidence: Paten-Programm live

status: aktiv
datum: 2026-09-09
contract: CONTRACT.md

Repo-Aufklärung vor dem ersten Edit. Jede Zeile ist eine Fundstelle `pfad:zeile`.
Live-Befunde (Discord, DB, Journal) stehen am Ende getrennt.

## Analoge Implementierungen (wie löst das Repo so etwas schon?)

- rust/crates/dl-community/src/concierge.rs:3359: `handle_native_onboarding_completed`, T0-Einstieg nach nativem Onboarding, gated durch `config.proactive` (Zeile 3370), Claim-once über `CONCIERGE_T0_CLAIM_NS`, DM per `send_dm_v2(user_id, t0_body(has_rank))`.
- rust/crates/dl-community/src/concierge.rs:1222: `t0_body`, T0-Text plus Buttons Tour/Play/Später; hier hängt REQ-1 die Paten-Buttons für Frischlinge an.
- rust/crates/dl-community/src/concierge.rs:1268: `nudge_body` (T2) und 1276 `pate_offer_body` mit Buttons `concierge:pate:yes` / `concierge:pate:no`.
- rust/crates/dl-community/src/concierge.rs:428: `cadence_due`, T2 nur wenn `t0 + 2 Tage`, keine Aktivität, `unsolicited_contact_count < 3`; T7 nach 7 Tagen.
- rust/crates/dl-community/src/concierge.rs:4869: `run_profile_cadence`, sendet T2, setzt danach `after_unsolicited_sent` und Journey `PateOffered`.
- rust/crates/dl-community/src/concierge.rs:5450: `request_pate`, postet `pate_claim_body` in `PATE_REQUEST_CHANNEL_ID`, pingt `PATE_ROLE_ID`, Button `concierge:pate:claim:{user_id}`; Fehlerpfad bei falscher Kanal-ID (5455).
- rust/crates/dl-community/src/concierge.rs:5707: `claim_pate`, nur im Paten-Kanal (5709), nur mit Rolle (5713), Last-Limit 3 (5786), legt `pate-{user_id}` in `pate_category_id` an (5836), schreibt Patenschaft, postet Intro (5950), Match-DM.
- rust/crates/dl-community/src/concierge.rs:7005: Interaktions-Dispatch für `concierge:pate:yes|no|claim:`.
- rust/crates/dl-community/src/concierge.rs:7139: `run_scheduler`, periodischer Lauf, führt Kadenz nur bei `proactive` aus (4851); hier hängen die 2-h- und 24-h-Stufen aus REQ-3 an.
- rust/bin/dl-bot/src/journeyglue.rs:157: `classify_native_onboarding_choice` erkennt Invite-Gast, Frischling, Player über Rollennamen; 305 ermittelt `matched_role_id`; 392 spawnt `handle_native_onboarding_completed(guild_id, user_id)` ohne die Wahl mitzugeben (REQ-1 reicht die Wahl durch).
- rust/crates/dl-community/src/team_applications.rs:25: `ApplicationKind` (Moderation, Coach, Caster, Tournament, Coder, Other), 46 Slug, 55 Label; 73 Status-Enum; 296/302 Buttons "Bewerbung annehmen/ablehnen". Keine Rollenvergabe bei Annahme vorhanden (grep `add_role|role_id` leer).
- rust/crates/dl-community/src/team_applications.rs:21: Texte aus `assets/team_application_texts.toml`, Laufzeitpfad 146, eingebetteter Fallback 167.
- rust/bin/dl-bot/src/main.rs:1051: Verdrahtung Team-Bewerbungen (`TeamApplications::new`, `register`, `ensure_panel`, `run_maintenance_loop`), Muster für den Leitfaden-Poster aus REQ-5.
- rust/bin/dl-bot/src/serversync.rs: Willkommen-Hub wird aus `assets/welcome_texts.toml` gebaut (Treffer für 1522460852114948166); REQ-7 ergänzt dort den Paten-Abschnitt.
- rust/bin/dl-bot/src/aiglue.rs:135: Inventarzeile "Concierge: aus/an"; 192 und 251 Env-Lookup-Defaults; REQ-8 hängt hier die Paten-Zeile an.

## Bestehende Abstraktionen (werden wiederverwendet, nicht nachgebaut)

- rust/crates/dl-community/src/concierge.rs:61: `DEFAULT_PATE_CATEGORY_ID = 1465839366634209361`; 66 `PATE_ROLE_ID = 1524047896297738311`; 67 `PATE_REQUEST_CHANNEL_ID = 1524083665838276860`; 63 `FRAG_DIE_COMMUNITY_CHANNEL_ID`.
- rust/crates/dl-community/src/concierge.rs:255: `ConciergeConfig`, 260 `pate_category_id`, 262 `pater_channel_id`, 263 `mod_ping_role_id`, 273 `proactive`; Env-Lesen 278 bis 311 (`DL_CONCIERGE_ENABLED` Default false, `DL_CONCIERGE_PROACTIVE` Default false, `DL_CONCIERGE_PATE_CHANNEL_ID` ohne Default).
- rust/crates/dl-community/src/concierge.rs:400: `ConciergeProfile` mit `pate_offered`, `pate_requested`, `opted_out`, `unsolicited_contact_count`, `t0_sent_at`, `t2_sent_at`.
- rust/crates/dl-community/src/concierge.rs:3012 und 3404: `claim_once_tx` als Einmal-Garantie (Muster für "jede Eskalationsstufe höchstens einmal").
- rust/crates/dl-community/src/concierge.rs:3277: `persist_cadence_uncertain` und 3325 `cadence_uncertain_persisted` (Zustellunsicherheit).
- rust/crates/dl-community/src/concierge.rs:3595: `after_unsolicited_sent` (Kontaktzähler).
- rust/crates/dl-community/src/concierge.rs:3909: `answer_dm_question_inner`, 4415 `knowledge_client::ask(&config.knowledge_url, ...)`, 60 `DEFAULT_KNOWLEDGE_URL = http://127.0.0.1:8896` (Wissenspfad für REQ-6).
- rust/crates/dl-community/src/concierge.rs:4805: `record_journey`; rust/crates/dl-activity/src/journey.rs:45 `JourneyEventType` (enthält `PateOffered`, `PateMatched`).
- rust/crates/dl-community/src/concierge.rs:109 bis 150: Textkonstanten T0, T2, `PATE_YES_TEXT`, `PATE_NO_TEXT`, Fehler- und Unsicherheitstexte, 1312 `pate_match_dm_text`.
- rust/crates/dl-central-db/src/testing.rs:19: `TestDb` für DB-Tests.

## Relevante Tests (laufen vorher, laufen nachher)

- rust/crates/dl-community/src/concierge.rs:14152: `patenschaft_unique_und_journey_pate_matched`.
- rust/crates/dl-community/src/concierge.rs:11750: `patenschaft_respektiert_den_optout_beider_beteiligten`.
- rust/crates/dl-community/src/concierge.rs:9169: `paten_wissensfragen_nutzen_http_wissenspfad_ohne_buttons`.
- rust/crates/dl-community/src/concierge.rs:9220: `ausdruecklicher_eigener_patenwunsch_erhaelt_aktionsbuttons` (INV-1).
- rust/crates/dl-community/src/concierge.rs:9437: `oeffentlicher_patenwunsch_hat_keine_persoenlichen_aktionsbuttons` (REQ-6 Abgrenzung).
- rust/crates/dl-community/src/concierge.rs:10586: `pate_offer_body_haengt_ja_nein_buttons_an`.
- rust/crates/dl-community/src/concierge.rs:11781 bis 13416: `pate_request_*` und `pate_claim_*` Fehler- und Unsicherheitspfade.
- rust/crates/dl-community/src/concierge.rs:8440: Test-Kommentar, `test_config` setzt `DL_CONCIERGE_PROACTIVE` nicht, `proactive = false`; 8448 und 11426 rufen `handle_native_onboarding_completed`.
- rust/crates/dl-central-db/tests/pate_journey_metadata_scrub.rs:8: `migration_entfernt_pate_id_auch_nach_spaeterem_journey_event`.
- rust/crates/dl-community/src/team_applications.rs:1635: Tests lesen dieselbe TOML (Fallback-Pfad).

## Öffentliche Schnittstellen und Verträge (dürfen nicht brechen)

- rust/crates/dl-central-db/migrations/2026070712_concierge_patenschaften.sql:1: `bot.concierge_patenschaften` (id, user_id, pate_id, guild_id, channel_id, created_at, released_at); 11 Unique auf `user_id WHERE released_at IS NULL`; 15 Index `pate_id`.
- rust/crates/dl-central-db/migrations/2026070710_onboarding_concierge.sql:44: `concierge_profiles.pate_offered`, 45 `pate_requested`.
- rust/crates/dl-central-db/migrations/2026071123_pate_delivery_uncertainty.sql:4: `pate_request_uncertain`.
- rust/crates/dl-central-db/migrations/2026070712_concierge_patenschaften.sql:47: Journey-Check erweitert um `pate_offered`, `pate_matched`.
- rust/crates/dl-central-db/migrations/2026071112_pate_journey_metadata_scrub.sql:2: Scrub von `pate_id`/`channel_id` aus Journey-Metadaten.
- Custom-IDs `concierge:pate:yes`, `concierge:pate:no`, `concierge:pate:claim:{user_id}`, `concierge:t0` (Claim-Namespace).
- scripts/run_dl_bot_service.sh:78: `DL_CONCIERGE_ENABLED=1`; 81 `DL_CONCIERGE_PATE_CHANNEL_ID=1524083665838276860`; 82 `DL_CONCIERGE_PATE_CATEGORY_ID=1465839366634209361`; `DL_CONCIERGE_PROACTIVE` fehlt.

## Änderungsfläche (welche Dateien voraussichtlich angefasst werden)

- rust/crates/dl-community/src/concierge.rs: Frischling-T0 mit Paten-Buttons, Anfrage-Persistenz, Eskalationsstufen im Scheduler, Mention-Wissensantwort in Paten-Kanälen, Leitfaden-Poster.
- rust/bin/dl-bot/src/journeyglue.rs: Onboarding-Wahl an den Concierge durchreichen.
- rust/crates/dl-central-db/migrations/: neue Tabelle für Paten-Anfragen (Anlage, Übernahme, Eskalation, Abschluss, Nachrichten-ID).
- rust/crates/dl-community/src/team_applications.rs, assets/team_application_texts.toml: Bereich "Pate", Rollenvergabe bei Annahme, Willkommens-DM.
- assets/paten_leitfaden.toml: Leitfaden-Texte.
- rust/bin/dl-bot/src/serversync.rs, assets/welcome_texts.toml: Paten-Abschnitt im Willkommen-Hub.
- rust/bin/dl-bot/src/aiglue.rs: Inventarzeile.
- scripts/run_dl_bot_service.sh: `DL_CONCIERGE_PROACTIVE=1`.
- rust/.sqlx/: Offline-Query-Dateien.

## Live-Befunde (Discord, DB, Journal; Stand 2026-09-09)

- Discord: Rolle "Pate" 1524047896297738311 mit 4 Mitgliedern (u. a. 203567477706194944, 279971744964542464, 426770757989957633); Kanal 🤝paten-zentrale 1524083665838276860 in Kategorie Community, Overwrites: Paten-Rolle allow 68608, @everyone deny 1024, `last_message_id` null (nie eine Nachricht).
- Discord: Kategorie "Neue Spieler" 1465839366634209361 existiert und ist leer (keine `pate-*`-Kanäle).
- Discord-Onboarding (GET /guilds/.../onboarding): Prompt "Wo stehst du gerade?" mit Option 🌱 "Ich bin ganz neu, nehmt mich an die Hand" vergibt Rollen 1304216250649415771 und 1522384961481609236 (Frischling); Prompt "Willst du Starthilfe?" Option 🧭 "Kleine Server-Tour per DM" vergibt 1527047393164529684 (Server-Tour). Rollen-Zähler: Frischling 70, Server-Tour 15, Invite-Gast 38.
- Discord: 🧭willkommen 1522460852114948166 besteht aus fünf Components-V2-Nachrichten des Bots (Willkommen, Bereiche, Team, Außerhalb, Schnellstart); kein Treffer für "Pate".
- Discord: Team-Panel 🤝teil-vom-team-werden 1544026617733710006 vorhanden; Bewerbungsarten laut Code ohne "Pate".
- DB (`bot.concierge_profiles`): 158 Profile, 26 mit `t0_sent_at`, 0 mit `t2_sent_at`, 0 `pate_offered`, 0 `pate_requested`, 0 `opted_out`; `bot.concierge_patenschaften` 0 Zeilen. Journey-Tabellen liegen unter `activity.journey_events`.
- Journal `deadlock-bot-rust` (nur seit 2026-09-08 vorhanden): Startzeile "Concierge: an, Zugang: offen für alle, proaktive DMs: aus"; keine weitere Concierge-Zeile.
- Wissenskorpus live: `/home/naniadm/.local/share/dl-knowledge/current/public/discord-server/` (Snapshot 6f788f11), Paten werden in `dm-concierge.html`, `negativ-wissen.html`, `faq-bot-selbst.html` erwähnt, eine eigene Paten-Seite fehlt; Quelle Repo Deadlock-Docs `public/discord-server/`, Deploy `tools/deploy_corpus.sh <git-ref>`.

## Offene Architekturfrage

- keine
