# ADR 0001: Prozessschnitt (2 Binaries) und Technologie-Stack

Status: angenommen · Datum: 2026-06-10 · Entscheider: Nani (Betreiber) + Claude

## Kontext

Das Python-Original betreibt Discord-Gateway, fünf HTTP-Server und einen
Subprocess-Supervisor in EINEM Prozess. Ein Bot-Restart reißt alle Websites mit;
Web-Last konkurriert mit dem Gateway. Zur Wahl standen: 1 Binary (wie heute),
2 Binaries, oder ein Service pro Subsystem.

## Entscheidung

**Zwei Binaries** in einem Cargo-Workspace, Domänenlogik in Library-Crates:

- `dl-bot`: alles, was die Gateway-Verbindung braucht — Discord, Master-Broker :8770
  (Broker führt Discord-Aktionen aus, MUSS beim Gateway wohnen), Changelog-Empfänger :8899.
- `dl-web`: alles HTTP-Serving ohne Gateway-Bedarf — Public-Stats :8768, Tierlist :8771,
  Turnier :8767, Dashboard :8766. Liest die gemeinsame DB; Discord-Aktionen laufen
  über die Broker-API.

Stack: **serenity 0.12 + poise** (Discord), **axum** (HTTP), **rusqlite bundled**
(SQLite), tokio / tracing / thiserror. Edition 2021, eine Version pro Dependency im
Workspace-Root.

## Begründung

- Bot-Restart killt keine Website mehr; trotzdem nur 2 systemd-Services statt 6.
- rusqlite statt sqlx: der Rust-Steam-Bot greift mit rusqlite bereits produktiv auf
  **exakt dieselbe** deadlock.sqlite3 zu — WAL-Koexistenz mit dem Python-Prozess ist
  dort bewiesen. Gekapselt hinter dl-db, ein späterer Wechsel bleibt möglich.
- serenity/poise statt twilight: ausgereiftestes Ökosystem für den Port von ~40
  Cog-artigen Modulen mit Slash-Commands, Views und persistenten custom_ids.
- Konsistenz: Workspace-Aufbau und Konventionen folgen den bestehenden Rewrites
  (Steam-Bot, Twitch-Bot), damit ein Entwickler-Kopf für alle drei reicht.

## Konsequenzen

- Domänen-Crates müssen gateway-frei nutzbar sein, wo dl-web sie braucht
  (z. B. dl-tournament: Store von dl-web, Discord-Posting von dl-bot).
- Der Broker bleibt die einzige Schreib-Brücke von dl-web Richtung Discord.
