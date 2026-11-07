# TODO: Invite-Lounge Status über Brain und Broker

Stand: 2026-11-07. Brain auf main (5b5de88d). Bots-Branch `invite-lounge-status-20261106` mit Pin auf diesen Stand. Nicht deployt.

## Offen

1. Deploy (nicht Teil dieses Pakets): Brain-Config `/home/nathanael/.config/deadlock-brain/brain-serve.json` braucht `"discord_delivery": {"base_url": "http://127.0.0.1:8770", "token_secret": "<Name des Broker-Tokens, den dl-bot auflöst>", "request_timeout_ms": 5000}`. Ohne diesen Block meldet das Brain `discord_invite_lounge` als nicht verfügbar; der Bot hält die Meldungen in der Ausgangsliste und versucht es mit wachsendem Abstand (höchstens stündlich) erneut. Nach sieben Tagen ohne Zustellung verwirft er eine Meldung mit Warnung im Log. Brain-Config deshalb vor oder zusammen mit dem Bot ausrollen. Ist `MASTER_BROKER_ALLOWED_CHANNEL_IDS` gesetzt, muss sie die Invite-Kanäle enthalten.
2. DB-Tests (`CENTRAL_TEST_DSN`, 4 dl-community-Tests, Brain `pg_*`) auf ionos laufen lassen, sobald ionos erreichbar ist.
3. Restrisiko: Eine DM nach einem dl-bot-Neustart ist nur über den Idempotenzspeicher des Brokers geschützt (im Speicher, 600 s). Kanalantworten gleicht der Bot vor jedem Wiederholungsversuch mit dem Verlauf ab.
