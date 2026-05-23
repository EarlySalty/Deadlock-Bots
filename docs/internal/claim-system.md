# Claim-System

## Zweck
Das Claim-System ist ein kleiner Spezial-Cog fuer Coaching-Threads. Es nimmt externe Zuweisungsdaten ueber einen lokalen TCP-Socket an, fuegt den zugewiesenen User in den Thread ein und postet dort eine persistente Claim-/Release-View fuer das Staff-Team.

## Architektur
Beim Laden startet das Cog einen lokalen Socket-Server auf `localhost:45680` (`cogs/claim_system.py:194`, `cogs/claim_system.py:211`). Der Payload ist laengenpraefixiertes JSON. Erwartet werden mindestens `thread_id` und optional `assigned_user_id` (`cogs/claim_system.py:241` bis `cogs/claim_system.py:250`).

Flow:

1. Externer Producer sendet JSON an den Socket.
2. `process_notification_data()` prueft, ob der Thread schon verarbeitet wurde (`cogs/claim_system.py:257`).
3. Der zugewiesene User wird in den Thread gezogen.
4. Im Thread wird eine persistente View mit `Coaching übernehmen` und `Freigeben` gepostet.
5. Der Zustand wird in `claimed_threads` persistiert, damit Doppelauslieferungen unterdrueckt werden.

Wenn `assigned_user_id` fehlt, greift ein harter Fallback auf einen Eintrag aus dem statischen `USERS`-Dict (`cogs/claim_system.py:47`, `cogs/claim_system.py:272`). Das ist ein deutlicher Hinweis darauf, dass dieses Modul stark umgebungs- und team-spezifisch ist.

## Konfiguration
Direkte Konstanten in `cogs/claim_system.py`:

- `SOCKET_HOST = "localhost"`
- `SOCKET_PORT = 45680`
- `STAFF_ROLE_ID` fuer Claim-/Release-Rechte

Zusätzlich existieren:

- `claim_data/` und `logs/` als lokale Verzeichnisse
- `USERS` als statische Fallback-Zuordnung

Es gibt keine Slash- oder Prefix-Commands fuer den Regelbetrieb.

## Admin-Workflow
1. Sicherstellen, dass der Producer den lokalen Socket erreicht.
2. Im Coaching-Thread erscheint nach erfolgreicher Zustellung die Claim-View.
3. Nur Staff mit der hinterlegten Rolle kann uebernehmen oder freigeben (`cogs/claim_system.py:87`, `cogs/claim_system.py:125`).
4. Falls ein Thread blockiert ist, Status in `claimed_threads` pruefen; dort steht, ob schon zugewiesen oder bereits geclaimt wurde.

## Datenmodell
Persistiert wird in `claimed_threads` (`cogs/claim_system.py:155`).

Wichtige Felder:

- `thread_id`
- `assigned_user_id`
- `claimed_by_id`
- `created_at`

Im Speicher haelt das Cog zusaetzlich `claimed_threads` als Dictionary fuer schnelle Checks.

## Wartung & Troubleshooting
- Wenn nichts ankommt: lokalen Port 45680, Producer und Bot-Logs pruefen.
- Wenn der Port belegt ist: es gibt exponential backoff und bis zu fuenf Startversuche (`cogs/claim_system.py:212` bis `cogs/claim_system.py:235`).
- Wenn falsche Personen automatisch eingetragen werden: zuerst `assigned_user_id` im eingehenden Payload, dann den statischen `USERS`-Fallback pruefen.
- Der Cog wirkt aktuell relativ isoliert; in diesem Repo gibt es keinen sichtbaren Producer fuer den Socket. Das sollte bei spaeterem Cleanup erneut bewertet werden.

## Code-Referenz
- Haupt-Cog: `cogs/claim_system.py:146`
- Socket-Start: `cogs/claim_system.py:211`
- Client-Handling: `cogs/claim_system.py:241`
- Notification-Flow: `cogs/claim_system.py:257`
- Persistente View: `cogs/claim_system.py:75`
