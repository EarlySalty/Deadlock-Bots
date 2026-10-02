# Pakete: mitspieler-pool

status: aktiv (2026-10-02)

Delegator: siehe REGISTER.md. Pakete sind disjunkt nach Dateien: jede Datei liegt in genau
einem Paket, kein Paket fasst Dateien eines anderen an. Paket A ist Voraussetzung für alle
anderen; B, C, E laufen danach parallel; D startet, sobald A gemergt und der Vertrag von E
(unten) feststeht.

| Paket | Repo | Inhalt (Dateien) | Worker-Thread | Modell | Status |
|---|---|---|---|---|---|
| A Schema + Datenschicht | Deadlock-Bots | neue Migration `dl-central-db/migrations/<nr>_spielerpool.sql`, neues Crate `rust/crates/dl-pool/` (Typen, Queries), Workspace-`Cargo.toml`, `.sqlx`, Löschpfad in `dl-community/src/privacy.rs` | d99950e0-3654-4d2d-aa68-4834c84988b3 | gpt-6.1-sol, worker_mittel | gestartet, Grok-Versuch gesettelt |
| B Interview | Deadlock-Bots | neues Modul `dl-community/src/pool_interview.rs`, minimaler Einhänger in `concierge.rs` (Onboarding) und Slash-Befehl `/spielerprofil` in der Befehlsregistrierung | | | geplant |
| C Deadlock-API-Ingest | Deadlock-Bots | neues Modul `dl-activity/src/pool_ingest.rs` (Matchhistorie, Rang, Stunden, Heatmap Wochentag x Stunde, Mitspieler-Paare), Start in `rust/bin/dl-bot/src/main.rs` | | | geplant |
| E Session-Kanal | Deadlock-Bots | neues Modul `dl-voice/src/pool_session.rs` (Kanal anlegen, Ping, 1 h Haltezeit, Löschen ohne Beitritt, Opt-in-DM), interner Loopback-Endpunkt im dl-bot | | | geplant |
| D Website | Website | Steam-OpenID-Verknüpfung mit Discord-Session, Backend-Routen Pool/Profil/Session/Feedback/Löschen in `builds/backend-rust`, Seiten `/spielerpool` in `builds/frontend`, Caddy-Route | | | geplant |

Dateikonflikt `rust/bin/dl-bot/src/main.rs`: C und E hängen dort je eine Startzeile ein.
Der Delegator lässt E zuerst mergen, C rebased danach.

## Vertrag Session-Kanal (E liefert, D ruft)

`POST http://127.0.0.1:<dl-bot-Port>/internal/pool/session`, Auth mit dem bestehenden
internen Token des Bots (wiederverwenden, kein neues Secret).
Body: `{"initiator_discord_id": "...", "target_discord_ids": ["..."], "mode": "casual|ranked|street_brawl"}`.
Antwort: `{"session_id": "...", "channel_id": "...", "channel_url": "https://discord.com/channels/..."}`.
Die Session-Zeile (Teilnehmer, Kanal, Zeitpunkte) liegt in der Tabelle aus Paket A und ist
Grundlage für das Feedback auf der Website.

Bei Bump-up oder Kontextverlust hier den Stand nachziehen: erledigte Pakete auf "fertig" plus
Commit, laufende auf den aktuellen Stand. Jeder Bump-up bleibt auf dem bestehenden Worktree-Stand.
