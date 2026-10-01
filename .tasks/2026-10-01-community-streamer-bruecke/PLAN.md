# Community-Streamer-Brücke

status: aktiv
datum: 2026-10-01
repos: Deadlock-Bots + Deadlock-Twitch-Bot

## Ziel

Community und Partner-Streamer zusammenbringen. Wer Discord und Twitch verknüpft, sammelt Punkte für Watchtime und echtes Chat-Engagement bei Partner-Streamern. Die Punkte landen zusammen mit den Voice-Punkten in einem gemeinsamen Leaderboard. Der Concierge erklärt das direkt beim Onboarding. Die Community kann Streamer für das Partnerprogramm vorschlagen. Jede Woche läuft ein Clip-Contest mit Community-Voting, die Top 3 laufen im dach_lock-Stream. Partner-Streamer werden sichtbar belohnt.

## Bausteine und Verantwortung

| Paket | Repo | Inhalt | Welle |
| --- | --- | --- | --- |
| A Verknüpfung | Deadlock-Bots | Vertrag `.tasks/2026-09-07-twitch-connection-discord-oauth/CONTRACT.md` umsetzen: `core.discord_platform_connections`, Button "Twitch verknüpfen", Broker-Endpunkt `twitch-links` | 1 |
| B Zuschauer-Punkte | Deadlock-Twitch-Bot | Watchtime und Chat-Punkte je Zuschauer, Partnerkanal und Tag; Streamer-Tageswerte; interne Lese-Endpunkte | 1 |
| D Clip-Contest | Deadlock-Bots | Wöchentliches Voting in Discord, Top 3, interner Einreich-Endpunkt für Twitch-Clips | 1 |
| C Leaderboard + Sync | Deadlock-Bots | Sync-Bin Twitch-Punkte in die zentrale DB, Punkte-Ledger, gemeinsames Leaderboard, Streamer-Leaderboard, Stufen-Rollen | 2 |
| E Clips aus Twitch | Deadlock-Twitch-Bot | Streamer reichen Clips aus Twitch ein (Chat-Befehl im eigenen Kanal, Dashboard), Weitergabe an Endpunkt aus D | 2 |
| F Streamer-Scouting | beide | Community schlägt Streamer vor, landet in `tb-scout`, Punkte bei erfolgreicher Partnerschaft | 2 |
| G Concierge | Deadlock-Bots | Onboarding erklärt Verknüpfung, Punkte, Clip-Contest, Streamer-Vorschlag; Wissenstexte | 2 |

## Gemeinsame Verträge

### Identität

- Discord-ID und Twitch-User-ID sind die einzigen Schlüssel. Logins und Namen sind nur Anzeige.
- Harte Verknüpfung kommt ausschließlich aus `core.discord_platform_connections` (Paket A). Keine Namensähnlichkeit für Punkte.
- Twitch-Punkte werden für alle Zuschauer gezählt (Twitch-User-ID), aber erst nach Verknüpfung einer Discord-ID zugerechnet und im Leaderboard gezeigt. Wer nicht verknüpft ist, taucht nirgends namentlich auf.

### Punkteregeln (Konstanten im Code, keine ENV)

Zuschauer, nur in Kanälen aktiver Partner während einer Live-Session, Tag nach Europe/Berlin:

- Watchtime: 1 Punkt je volle 5 Minuten Anwesenheit (Chatters-Liste oder Chat-Aktivität). Deckel 72 Punkte je Kanal und Tag, 144 Punkte je Zuschauer und Tag über alle Kanäle.
- Chat: 1 Punkt je zählender Nachricht. Zählend heißt: kein Befehl (`!`), mindestens 10 Zeichen nach Trimmen, nicht identisch mit der vorherigen Nachricht der Person im Kanal, frühestens 60 Sekunden nach der letzten zählenden Nachricht der Person im Kanal. Deckel 30 je Kanal und Tag.
- Entdecker-Bonus: 10 Punkte beim ersten Auftauchen in einem Partnerkanal, in dem die Person noch nie war. Höchstens 3 Boni je Tag.
- Ausgeschlossen: Broadcaster im eigenen Kanal, bekannte Bot-Konten (`WHITELISTED_BOTS` und vorhandene Bot-Erkennung), gebannte Konten.

Streamer (Partner), je Tag:

- Community-Watchtime im eigenen Kanal (Summe Zuschauerminuten), Anzahl verschiedener Zuschauer, erfolgreiche Raids an andere Partner.
- Punkte für Streamer berechnet Paket C: 1 Punkt je 30 Zuschauerminuten verknüpfter Community-Mitglieder, 25 je Raid an einen Partner, 50 je qualifiziertem Discord-Beitritt über die eigenen Einladungen, 100/60/40 für Clip-Contest-Platz 1/2/3.

Discord-Ereignisse (Ledger, Paket C):

- Clip-Contest: Platz 1/2/3 = 100/60/40 an den Einsender, 2 Punkte je abgegebener Stimme (höchstens 1 Stimme je Woche zählt).
- Streamer-Vorschlag, der Partner wird: 150 Punkte.
- Voice-Punkte bleiben unverändert aus `voice.voice_stats`.

### Interne Schnittstelle Twitch-Bot zu Deadlock-Bots (Paket B, gelesen von Paket C)

Bestehender interner Twitch-API-Server, gleiche Auth wie `/internal/twitch/v1/streamer-invites` (`X-Internal-Token`, Loopback).

`GET /internal/twitch/v1/community-points/viewers?updated_since=<RFC3339>&limit=<1..5000>`

```json
{"rows":[{"twitch_user_id":"123","twitch_login":"name","channel_twitch_user_id":"456","day":"2026-10-01","watch_minutes":95,"chat_messages":12,"points_watch":19,"points_chat":12,"points_discovery":10,"updated_at":"2026-10-01T20:00:00Z"}],"next_updated_since":"2026-10-01T20:00:00Z","has_more":false}
```

`GET /internal/twitch/v1/community-points/streamers?updated_since=<RFC3339>&limit=<1..5000>`

```json
{"rows":[{"streamer_twitch_user_id":"456","streamer_login":"name","discord_user_id":"789","day":"2026-10-01","viewer_minutes":4200,"unique_viewers":61,"raids_to_partners":1,"updated_at":"2026-10-01T22:00:00Z"}],"next_updated_since":"...","has_more":false}
```

Zeilen sind Tageswerte und werden beim Lesen idempotent überschrieben. Sortierung nach `updated_at`, dann Schlüssel; Cursor stabil.

### Clip-Einreichung aus Twitch (Paket D baut, Paket E nutzt)

Broker-Endpunkt in Deadlock-Bots nach dem Muster der bestehenden internen Broker-Endpunkte:

`POST /internal/master/v1/clips/submit`

```json
{"source":"twitch","clip_url":"https://clips.twitch.tv/...","streamer_twitch_user_id":"456","streamer_login":"name","submitted_by_twitch_user_id":"456","title":"...","idempotency_key":"twitch-clip-<clip_id>"}
```

Antwort: `{"status":"accepted"|"duplicate"|"rejected","submission_id":123,"reason":null}`. Nur Clips von Partnerkanälen; Duplikate über die Clip-URL je Woche.

### Streamer-Vorschlag (Paket F)

Discord-Button "Streamer vorschlagen" mit Modal (Twitch-Kanal, warum). Speicherung in der zentralen DB, Weitergabe an den Twitch-Bot über `POST /internal/twitch/v1/scout/community-suggestion` (Twitch-Login, vorgeschlagen von Discord-ID). Der Twitch-Bot legt den Kanal als Kandidat in `twitch_scout_candidates` mit Quelle Community an, die bestehende Admin-Freigabe bleibt der einzige Weg in die Outreach-Kette. Wird der Kanal Partner, bekommt der erste Vorschlagende die Punkte.

## Leitplanken (alle Pakete)

- Rust, bestehende Muster wiederverwenden, keine neuen Secrets, keine ENV-Konfiguration. Betriebswerte in TOML nur wo das Repo das schon so macht.
- Migrationen nur additiv, Rückweg unter `rollbacks/` wo das Repo das so führt. Zentrale DB: `rust/crates/dl-central-db/migrations`, Twitch-DB: `rust/migrations`.
- Nutzersichtbare Texte deutsch, echte Umlaute, keine Gedankenstriche, Nutzersprache ohne Fachwörter.
- Datenschutz: Punkte und Namen nur für verknüpfte Mitglieder sichtbar; `core.user_privacy` respektieren, wo es bereits für Leaderboards gilt.
- Kein Kauf von Views, keine Belohnung für Follows oder Subs (Twitch-Richtlinien). Punkte sind Community-Anerkennung, kein Geldwert.
- Bestehende Tests nicht abschwächen. Neue Tests für Regeln, Deckel, Idempotenz, Endpunkte.

## Migrationsnamen (Kollisionen vermeiden)

- A: `2026100101_discord_platform_connections.sql`
- D: `2026100102_clip_contest_voting.sql`
- C: `2026100103_community_points.sql`
- F: `2026100104_streamer_suggestions.sql`
- B (Twitch-DB): `20261001100000_community_points.sql`
- F (Twitch-DB): `20261001110000_scout_community_source.sql`
