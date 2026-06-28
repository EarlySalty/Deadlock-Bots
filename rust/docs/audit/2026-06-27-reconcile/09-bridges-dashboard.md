# Reconcile 09 - Bridges / Dashboard (2026-06-27)

Scope: Steam-/Twitch-Bridges, Build-Publisher, Changelog-Publisher, Dashboard und Broker-Bridge-Infrastruktur.

Basis: `00-SHARED-BRIEF.md`, Alt-Audit `2026-06-21-py-rust-discord-parity-audit.md`, Python-Referenzen in `cogs/`, `service/`, Rust-Referenzen in `rust/crates/dl-bridges`, `rust/crates/dl-changelog`, `rust/crates/dl-dashboard`, `rust/crates/dl-broker`, plus `rust/bin/dl-bot`.

## Kurzbefund

- Insgesamt 15 offene Abweichungen: 3 High, 8 Medium, 4 Low.
- Behauptete Fixes verifiziert: Build-Publisher ist gateway-gated; Dashboard Auth-Misconfig fail-closed und Security-Header sind vorhanden; `role/create`, `guild-stats`, `resolve-names` und `scam_revoke` sind im Rust-Pfad vorhanden.
- Gegenbefund zu `channel-info`: Commit `3c52050` portiert nur Python/tests; im Rust-Broker fehlt der Endpoint weiter.
- Kein GAP fuer `cogs/steam_verified_role.py`: der Python-Cog ist im Loader explizit ausgeschlossen, weil Rust `friend_sync` Single-Owner ist.

## Verifizierte Paritaet / erledigte Alt-Punkte

| Bereich | Status | Referenzen |
|---|---:|---|
| Steam Rankcheck Button Timeout | OK | Python `cogs/steam_bridge.py:128-137,552-559`; Rust `rust/crates/dl-bridges/src/steam.rs:18-21,39-45,300-305` |
| Steam Verified Role | Deliberate single-owner | Python-Cog existiert, wird aber nicht geladen: `bot_core/cog_loader.py:217-223`; Rust-Single-Owner via Steam-Bot/Friend-Sync |
| Twitch Streamer-Link AI Budget | OK | Python `cogs/twitch/streamer_link_matcher.py:643-654,843-852`; Rust `rust/crates/dl-bridges/src/matcher.rs:361-383,551-563,983-994` |
| Build-Publisher Gateway-Gate (#23 / `7cc2996`) | OK, aber nicht voll parity | Rust `rust/bin/dl-bot/src/main.rs:425-427,606-609`; queue tests vorhanden |
| Dashboard fail-closed Auth / Headers (#24/#25 / `24b9bb7`) | OK | Rust `rust/crates/dl-dashboard/src/config.rs:145-160`; `rust/crates/dl-dashboard/src/web.rs:154-168,378-412` |
| Broker `role/create` | OK | Rust `rust/crates/dl-broker/src/lib.rs:193-199`; `rust/crates/dl-broker/src/handlers.rs:1046-1137`; `rust/crates/dl-discord/src/adapter.rs:387-413` |
| Broker `guild-stats` / `resolve-names` | OK | Rust `rust/crates/dl-broker/src/lib.rs:141-149`; `rust/crates/dl-broker/src/handlers.rs:237-266,334-363`; `rust/crates/dl-discord/src/adapter.rs:705-735` |
| Broker `scam_revoke` ViewSpec | OK | Rust `rust/crates/dl-broker/src/payload.rs:171-227`; `rust/crates/dl-discord/src/adapter.rs:134-142` |
| Website Invite Twitch-Partner Klassifikation | OK | Python `cogs/website_invite_cog.py:47-54,430-438`; Rust `rust/crates/dl-community/src/invites.rs:20-27,49-58` |

## Offene GAPs

| ID | Severity | Typ | Python-Ref | Rust-Ref | User-sichtbare / Korrektheits-Auswirkung | Aufwand |
|---|---:|---|---|---|---|---:|
| BRD-01 | High | Build-Publisher Monitor | `cogs/build_publisher.py:330-400` | `rust/bin/dl-bot/src/build_publisher.rs:248-270` | Rust resetet nur stale `processing`, uebernimmt aber keine `steam_tasks` DONE/FAILED Resultate. Erfolgreiche Uploads setzen daher `uploaded_build_id`/`uploaded_version` nicht, Fehler werden nicht final markiert, und Jobs koennen wieder pending werden. | M |
| BRD-02 | High | Build-Publisher Readiness Gate | `cogs/build_publisher.py:98-146` | `rust/bin/dl-bot/src/build_publisher.rs:74-185`; Gateway-Gate nur `rust/bin/dl-bot/src/main.rs:606-609` | Python pausiert bei fehlendem Steam-Login oder fehlender Deadlock-GC-Ready-State. Rust queued `BUILD_PUBLISH` trotzdem, sobald der Discord-Gateway aktiv ist. Das kann fehlschlagende oder haengende Publish-Tasks erzeugen. | S/M |
| BRD-03 | High | Broker `channel-info` | `service/master_broker.py:275-278,1323-1359` | `rust/crates/dl-broker/src/lib.rs:116-199` (fehlend) | Der loopback-only Read-Endpoint `/internal/master/v1/discord/channel-info` fehlt im Rust-Broker. Interne Clients bekommen nach Rust-Cutover 404 statt Channel-Metadaten. | S |
| BRD-04 | Medium | Twitch Internal API URL Hardening | `cogs/twitch/live_bridge.py:67-113,186-199` | `rust/crates/dl-bridges/src/twitch.rs:75-124` | Python normalisiert Base-URL, lehnt Credentials ab, strippt den internen Pfad und erlaubt non-loopback nur per Override. Rust akzeptiert rohe Base-URLs und kann Token bei Fehlkonfig an externe Hosts senden oder Pfade doppeln. | M |
| BRD-05 | Medium | Broker `resolve-user` Idempotency | `service/master_broker.py:2887-2916` | `rust/crates/dl-broker/src/handlers.rs:300-329,369-385` | Python behandelt `resolve-user` als normalen autorisierten Read. Rust ruft `begin_action` auf und verlangt dadurch `idempotency_key`; alte Clients ohne Key bekommen 400. | S |
| BRD-06 | Medium | Broker `move_voice` Fehlersemantik | `service/master_broker.py:2470-2532` | `rust/crates/dl-broker/src/handlers.rs:1204-1230`; `rust/crates/dl-discord/src/adapter.rs:433-450` | Python prueft Member/Channel/Guild vorab und liefert 404 fuer fehlende Member/Channel. Rust macht nur `edit_member`; die Handler-404-Arme sind praktisch nicht erreichbar, Discord-Fehler werden 502. | M |
| BRD-07 | Medium | Steam Rankcheck Payload | `cogs/steam_bridge.py:532-546` | `rust/crates/dl-bridges/src/steam.rs:126-137` | Rust forwarded keine `data.discord_name`. Der Steam-Bot verliert den Display-Namen fuer Rankcheck-/Supporter-Kontext. | S |
| BRD-08 | Medium | Steam BetaInvite Panel Zielkanal | `cogs/steam_bridge.py:687-760` | `rust/crates/dl-bridges/src/steam.rs:391-427,564-577` | Python erlaubt `/publish_betainvite_panel channel:<...>`. Rust postet nur in den aktuellen Channel; Admins koennen nicht direkt in Zielkanaele publishen. | S |
| BRD-09 | Medium | Steam Panel Restore/Edit | `cogs/steam_bridge.py:29-66,588-662,931-992` | `rust/crates/dl-bridges/src/steam.rs:391-427,616-628` | Python persistiert Panel-Message-IDs und kann bestehende Panels editieren oder nach Restart wiederherstellen. Rust kennt weder `message_id` noch KV-Restore; Wiederholung erzeugt eher Duplikate. | M |
| BRD-10 | Medium | Changelog Slash Command | `cogs/changelog_publisher.py:370-408` | `rust/crates/dl-changelog/src/lib.rs:85-92`; `rust/bin/dl-bot/src/main.rs:412-423` | HTTP-Changelog ist portiert, aber `/changelog post` fehlt im Rust-Interaction-Pfad. Admins verlieren die Discord-native Publish-Aktion. | S |
| BRD-11 | Medium | Dashboard Origin Allowlist | `service/dashboard.py:938-970` | `rust/crates/dl-dashboard/src/config.rs:124-126`; `rust/crates/dl-dashboard/src/web.rs:210-222` | Python seedet Allowed Origins aus Public-/Listen-Base-URLs plus Env. Rust nutzt nur Env; wenn leer, akzeptiert `allowed_origin` jede Origin. CSRF-Token bleibt aktiv, aber die Origin-Haertung ist schwaecher. | S |
| BRD-12 | Low | Twitch Review Status Mention | `cogs/twitch/streamer_link_matcher.py:775-801` | `rust/crates/dl-bridges/src/matcher.rs:721-768,922-949,1009-1031` | Python schreibt Moderator als `interaction.user.mention`; Rust nutzt `author_name`. Statusmeldungen sind weniger eindeutig und nicht anklickbar. | S |
| BRD-13 | Low | Twitch Live Referral URL Validierung | `cogs/twitch/live_bridge.py:123-140,355-374,491-510` | `rust/crates/dl-bridges/src/twitch.rs:220-247,408-417` | Python validiert Referral-URLs und ueberspringt defekte Tracking-Views. Rust reicht `referral_url` aus dem internen API ungeprueft in die Button-Response weiter. | S |
| BRD-14 | Low | Dashboard `turnier_only` Leserechte | `service/dashboard.py:3934-3953,6592-6640` | `rust/crates/dl-dashboard/src/repo_activity.rs:27-59`; `rust/crates/dl-dashboard/src/survey.rs:331-338` | Python erlaubt Repo-Activity und Leave-Survey-Images jeder gueltigen Session. Rust verlangt `guard_full`; Turnier-only Sessions bekommen dort 403. | S |
| BRD-15 | Low | Twitch AI Provider Config | `cogs/twitch/streamer_link_matcher.py:447-449,651-654` | `rust/crates/dl-bridges/src/matcher.rs:330-367`; `rust/bin/dl-bot/src/main.rs:95-99` | Python liest `STREAMER_LINK_AI_PROVIDER`. Rust baut nur den MiniMax-Client aus Env oder `NoAi`, die Provider-Auswahl ist nicht portiert. | S |

## Top-5 nach Risiko

1. BRD-01: Build-Publisher uebernimmt DONE/FAILED Resultate nicht.
2. BRD-02: Build-Publisher ignoriert Steam-Login/GC-Readiness.
3. BRD-03: Rust-Broker hat keinen `channel-info` Endpoint.
4. BRD-04: Twitch Internal API URL-Hardening fehlt.
5. BRD-05: `resolve-user` verlangt faelschlich Idempotency.

## Verifikation

- `cargo test -p dl-bridges -p dl-broker -p dl-dashboard -p dl-changelog --lib`
  - Ergebnis: OK, 27 + 11 + 60 + 6 Tests bestanden.
- `cargo test -p dl-bot build_publisher`
  - Ergebnis: OK, 4 Tests bestanden.

## Hinweise

- `3c52050` ist kein Rust-Fix fuer `channel-info`; der Commit betrifft Python `service/master_broker.py` und Tests.
- `7cc2996` belegt den Rust-Build-Publisher und das Gateway-Gating, nicht aber die Python-Readiness- und DONE/FAILED-Monitor-Paritaet.
- Die stale Alt-Audit-Notiz "Twitch AI Budget fehlt" ist ueberholt; Budget und Summary-Zaehler sind in Rust vorhanden.
