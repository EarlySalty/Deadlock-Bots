# Spec: Website-Coaching-Anfragen in Discord spiegeln (+ Claim)

**Datum:** 2026-06-29
**Repos:** Website (FastAPI-Backend) + Deadlock-Bots (Rust dl-bot)
**Status:** Design freigegeben (User), Implementierung an Codex.

## Ziel
Eine Coaching-Anfrage, die auf der **Website** gestellt wird (`POST /api/coaching/requests` → Tabelle `coaching_requests`), soll **automatisch als schickes Embed im bestehenden Discord-Anfrage-Kanal** erscheinen, mit den **vorhandenen Claim/Round-Robin-Buttons**, sodass Coaches sie wie heute zuweisen können. Plus ein Link-Button „Auf der Website öffnen".

Nicht-Ziel (YAGNI): kein neuer Kanal, kein neuer Zuweisungsweg, keine neue Transport-Infra, keine Anfrage-Detailseite im Frontend.

## Architektur — auf dem bestehenden Notification-Poller mitreiten
Der Bot pollt bereits `GET /api/coaching/platform/notifications/due` (alle 60 s) und ackt via `POST .../notifications/ack` (`coaching.rs`). Heute liefert `notifications_due` Termin-Events (`created|reminder|cancelled`) abgeleitet aus `coaching_appointments` (Ack über `notify_*_at`-Spalten), die der Bot als **DM** zustellt (`build_dm_text`).

Wir erweitern denselben Mechanismus um einen **neuen Typ `request_created`**, der aus **`coaching_requests`** stammt — der Bot rendert ihn als **Embed** statt DM.

```
Website: POST /api/coaching/requests  →  INSERT coaching_requests (notify_discord_at = NULL)
Bot-Poll: GET /notifications/due       →  enthält {type:"request_created", ...} für alle requests mit notify_discord_at IS NULL
Bot:      type=request_created          →  Embed + Claim/Release + Link-Button in REQUEST_CHANNEL_ID posten
Bot:      POST /notifications/ack       →  setzt coaching_requests.notify_discord_at = now (genau-einmal)
Claim:    bestehender Flow              →  sync_coaching() spiegelt Zuweisung zurück auf die Plattform
```

## Cross-Repo-Vertrag (verbindlich für beide Worker)
`GET /api/coaching/platform/notifications/due` liefert in `notifications[]` zusätzlich Items der Form:
```json
{
  "type": "request_created",
  "request_id": "<coaching_requests.id, str>",
  "coachee_id": "<coachees.id, str>",
  "discord_user_id": 123456789,
  "discord_username": "name",
  "rank": "Archon",
  "subrank": "3",
  "hero": "Vindicta | null",
  "games_played": "string | null",
  "hours_played": "string | null",
  "availability": "string | null",
  "current_problems": "string | null",
  "preferred_coach_id": "str | null"
}
```
**Ack:** Der Bot ackt request-Items so, dass `coaching_requests.notify_discord_at` gesetzt wird → jede Anfrage wird **genau einmal** gespiegelt. Ack-Format konsistent zum bestehenden `ack`-Endpoint erweitern (Termin-Acks unverändert lassen).

**Link A:** `https://deutsche-deadlock-community.de/coaching/coachees/{coachee_id}` (Coachee-Detailseite; es gibt keine `/requests/:id`-Route). `coachee_id` wird vom Backend per Upsert aus `discord_user_id` ermittelt und mitgeliefert.

## Backend (Website) — Tickets
1. Migration/DDL: Spalte `coaching_requests.notify_discord_at TIMESTAMP NULL` (idempotent, `IF NOT EXISTS`-Stil passend zum Projekt).
2. `notifications_due` um eine **request-Quelle** erweitern: alle `coaching_requests` mit `notify_discord_at IS NULL` (und sinnvollem Status = neue/offene Anfrage) als `request_created`-Items zurückgeben (Felder s. Vertrag; `coachee_id` per Upsert). Termin-UNION unverändert.
3. `notifications_ack` so erweitern, dass request-Acks `notify_discord_at` setzen.
4. Tests: (a) nach `POST /coaching/requests` taucht genau ein `request_created`-Item in `/notifications/due` auf; (b) nach `ack` verschwindet es (idempotent, kein Doppel); (c) Termin-Notifications unberührt.

## Bot (Deadlock-Bots) — Tickets
1. `coaching.rs`-Poller (`process_notifications`): neuer Branch `type == "request_created"` → **nicht** DM, sondern Embed posten in `REQUEST_CHANNEL_ID` über den bestehenden Port (wie ein Discord-Request, inkl. Claim/Release-Buttons + Auto-Round-Robin „gleiche Methodik" wie heute). Danach acken (auch bei Post-Fehler **nicht** acken → Retry beim nächsten Poll).
2. Embed-Renderer: Variante **ohne AI-Analyse-Feld**. `build_request_embed` um ein Flag (`include_ai: bool`) erweitern ODER schlanke `build_request_embed_no_ai`; Felder: Spieler `@mention`(discord_user_id), Rang (`rank`+`subrank`), Hero, Games/Stunden (`games_played`/`hours_played`), Verfügbarkeit/Slot (`availability`), Probleme (`current_problems`). Reuse der bestehenden Feld-Labels/Helfer (`normalize_inline`).
3. Link-Button „Auf der Website öffnen" (Button-Style 5 = Link, `url` = Link A). Deutscher Button-Text steht fest (von Claude): **„Auf der Website öffnen"** — kein Platzhalter nötig.
4. Mapping der Anfrage-Felder aus dem Notification-Item auf den vorhandenen Request-Typ; Wiederverwendung der bestehenden Claim-Logik (`claim_components`, `coach_claim_*`), Round-Robin + `sync_coaching`-Rücksync unverändert.
5. Tests: (a) Renderer ohne AI-Feld + Link-Button korrekt; (b) Handler: `request_created`-Item → Embed-Payload (nicht DM), inkl. Buttons; (c) idempotent (Post-Fehler → kein Ack).

## TABU / deutsche Texte
Einzige neue user-sichtbare deutsche Zeichenkette = der Link-Button-Text **„Auf der Website öffnen"** (oben vorgegeben). Alle anderen Embed-Texte sind Bestandstexte. Falls ein Worker doch neuen deutschen User-Text braucht → `"Platzhalter"` + Datei:Zeile melden, Claude schreibt final.

## Idempotenz / Fehler
- Genau-einmal: `notify_discord_at`-Flag + Ack. Post fehlgeschlagen → nicht acken → nächster Poll-Retry.
- Eine Anfrage = ein Embed. Keine Dedup über Discord-Panel-Requests nötig (separate Quelle, eigener `request_id`-Namespace).

## Deploy (durch Claude, nach Codex + Kritiker + Verifikation)
- Backend: Worktree → main, Migration greift beim Boot/Deploy, `deadlock-website-backend.service` neu starten.
- Bot: Worktree → main (NICHT im welle2/main-Live-Checkout bauen — Time-Bomb), `--release` bauen, `deadlock-bot-rust.service` neu starten.
- Live-Verifikation: Test-Anfrage über Website → Embed erscheint im Kanal → Claim → Plattform zeigt „zugewiesen".
