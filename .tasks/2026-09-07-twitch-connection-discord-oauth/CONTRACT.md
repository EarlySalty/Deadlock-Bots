# Contract: Twitch-Verknüpfung aus dem Discord-OAuth speichern und bereitstellen

status: aktiv
datum: 2026-09-07
klasse: hoch
repo: Deadlock-Bots

Dieser Contract ist der Maßstab für Implementierung und Merge-Kritiker. Nach dem
Anlegen ist er unveränderlich: der Hook lässt nur noch die `status:`-Zeile und
Anhänge unter `## Amendments` zu. Gegenstück im Twitch-Bot:
`Deadlock-Twitch-Bot/.tasks/2026-09-07-register-harte-twitch-links/CONTRACT.md`.

## Ziel

Discord-Mitglieder können ihre Twitch-Verknüpfung aus ihrem Discord-Profil freigeben. Der Verbund speichert Discord-ID und Twitch-User-ID zentral und stellt sie dem Twitch-Bot bereit, damit das Zuschauer-Register echte Mitglieder sicher erkennt (Wahrscheinlichkeit 1,0) statt über Namensähnlichkeit zu raten. Bestandsaufnahme: `EVIDENCE.md` in diesem Ordner.

## Anforderungen (user-sichtbares Verhalten)

- REQ-01 Tabelle: Neue Tabelle `core.discord_platform_connections` (discord_id, platform, platform_user_id, platform_login, verified, updated_at, Primärschlüssel discord_id plus platform) als reguläre Migration in `dl-central-db/migrations`, additiv, ohne Änderung an `core.steam_links`.
- REQ-02 Lesepfad: Der delegierte Broker-Flow (`dl-dashboard/src/web.rs`, consume) extrahiert bei Scope mit `connections` zusätzlich zur Steam-Connection die Twitch-Connection (Typ `twitch`: `id` als Twitch-User-ID, `name` als Login, `verified`) und schreibt sie per Upsert nach REQ-01. Das Steam-Verhalten bleibt unverändert. Ohne Twitch-Connection wird nichts geschrieben und der Aufrufer bekommt das als Ergebnis zurück.
- REQ-03 Einstieg für Mitglieder: Das bestehende Verify-Panel im Discord (Buttons in `dl-bot/src/serversync/rang_guide_publish.rs`) bekommt einen Button "Twitch verknüpfen". Klick liefert eine nur für den Klickenden sichtbare Antwort mit dem Discord-OAuth-Link (Scope `identify connections`, Flow-Typ delegiert, Callback der bestehende `/callback/discord`). Nach dem Callback erhält das Mitglied eine Bestätigung in Nutzersprache ("Dein Twitch-Konto <login> ist verknüpft") oder, wenn Discord keine Twitch-Connection liefert, einen kurzen Hinweis, wie man Twitch in den Discord-Einstellungen unter Verbindungen hinzufügt und es dann erneut versucht. Keine Fachwörter (Scope, OAuth, Register, Connection).
- REQ-04 Hinweis nach Verify: Die Erfolgsantwort des Steam-Verify nennt in einem Satz den Twitch-Knopf als optionalen nächsten Schritt ("Wenn du auf Twitch zuschaust, verknüpf dein Twitch-Konto, dann erkennt dich der Bot in Partner-Streams"). Keine DM, kein Timer, keine Wiederholung.
- REQ-05 Bereitstellung: Neuer loopback-Endpunkt `GET /internal/master/v1/discord/twitch-links` im Broker (`dl-broker/src/handlers.rs`) nach dem Muster von `members`, Antwort als JSON-Liste mit `discord_id`, `twitch_user_id`, `twitch_login`, `verified`, `updated_at`; nur Zeilen mit platform `twitch`. Kein weiteres Auth-Modell als bei `members`.
- REQ-06 Sichtbarkeit: Jede erfolgreiche Verknüpfung und jeder Fehlversuch (keine Twitch-Connection) erscheint als Log-Zeile ohne Personendaten außer den IDs; keine Discord-Nachricht an Betreiber.

## Invarianten (darf sich nicht ändern)

- INV-01: Alle bestehenden OAuth-Flows mit Scope `identify` (Dashboard-Login, Website-Login, dl-web Broker) bleiben unverändert; kein zusätzlicher Consent für Bestandsnutzer außerhalb des neuen Buttons.
- INV-02: Es werden keine Access- oder Refresh-Tokens für den neuen Flow dauerhaft gespeichert; gespeichert wird nur das Ergebnis nach REQ-01.
- INV-03: Steam-Verify-Logik, Rollenvergabe und `core.steam_links` bleiben unverändert.
- INV-04: Keine ENV-Config, kein neues Secret; die Discord-App-Credentials kommen wie bisher aus Infisical.
- INV-05: Migrationen nur additiv; GRANTs operativ wie bei den bestehenden `core`-Tabellen (Leserechte für den Broker-Dienstnutzer, Schreibrechte für dl-dashboard).
- INV-06: Bestehende Tests nicht löschen oder abschwächen; neue Tests für Extraktion (Twitch und Steam gleichzeitig, nur Steam, keine Connection), Upsert und Endpunkt.
- INV-07: Nutzersichtbare Texte deutsch, echte Umlaute, keine Gedankenstriche, Nutzersprache.

## Nicht-Ziele

- Kein Website-Button und keine Änderung am Website-Linked-Role-Flow.
- Kein Backfill (historische Connections sind nicht abrufbar).
- Keine Änderung am Twitch-Bot (eigener Contract).
- Kein Onboarding-DM-Flow.

## Erlaubter Änderungsbereich

- rust/crates/dl-dashboard/src/oauth.rs
- rust/crates/dl-dashboard/src/web.rs
- rust/crates/dl-dashboard/src/lib.rs
- rust/crates/dl-dashboard/tests/
- rust/crates/dl-broker/src/handlers.rs
- rust/crates/dl-broker/src/lib.rs
- rust/crates/dl-broker/src/router.rs
- rust/crates/dl-broker/tests/
- rust/bin/dl-bot/src/serversync/rang_guide_publish.rs
- rust/bin/dl-bot/src/serversync/
- rust/bin/dl-bot/src/verify/
- rust/crates/dl-activity/src/lfg_freetext.rs
- rust/crates/dl-central-db/src/
- dl-central-db/migrations/
- rust/.sqlx/
- rust/Cargo.lock
- docs/
- .tasks/2026-09-07-twitch-connection-discord-oauth/

## Verbotene Änderungen

- rust/crates/dl-bridges/
- rust/bin/dl-web/
- bestehende Migrationen
- Lint- und CI-Konfiguration
- Steam-OpenID-Verify-Pfad

## Offene Produktfragen

- Ob zusätzlich der Website-Login den Scope bekommt, entscheidet der Nutzer später; hier bewusst nicht.

## Amendments
