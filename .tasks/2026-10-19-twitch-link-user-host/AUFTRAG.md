# Twitch-Verknuepfen auf dem Nutzer-Host

## Ziel

Die Abschlussseite `Twitch verknüpft` erscheint auf `https://deutsche-deadlock-community.de/twitch-verknuepfen/fertig`. `admin.deutsche-deadlock-community.de` ist kein Nutzer-Host.

## Ursache

`twitch_link_done_url` baute `redirect_after` aus `MASTER_DASHBOARD_PUBLIC_URL` / `DEFAULT_PUBLIC_BASE_URL` (Admin-Dashboard). Caddy auf dem öffentlichen Host hatte keinen Handle für `/twitch-verknuepfen` und landete im 404-Fallback. Der Admin-Catch-all proxied alles nach 8766, deshalb funktionierte die Seite nur auf `admin.`.

## Änderung

- Done-URL fest auf `https://deutsche-deadlock-community.de/twitch-verknuepfen/fertig` (`twitch-link-done-user-host`).
- Öffentlicher Caddy-Handle `/twitch-verknuepfen*` nach 8766.
- Admin-Host 302 nach dem Nutzer-Host, damit schon gespeicherte Admin-`redirect_after` dort landen.

## Abnahme

Initiate speichert `redirect_after` mit Nutzer-Host. `curl -I https://admin.deutsche-deadlock-community.de/twitch-verknuepfen/fertig` ist 302 auf den Nutzer-Host. GET auf dem Nutzer-Host liefert die HTML-Seite von dl-web.
