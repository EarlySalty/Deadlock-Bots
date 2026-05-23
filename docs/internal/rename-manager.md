# Rename Manager

## Zweck
Der `RenameManagerCog` serialisiert Voice-Channel-Umbenennungen ueber eine SQLite-Queue. Er existiert, weil Discord Channel-Renames stark ratelimitiert und stoeranfaellig sind. Anstatt dass beliebige Cogs direkt `channel.edit(name=...)` feuern, sollen sie Rename-Wuensche in eine Queue stellen.

## Architektur
Beim Laden legt das Cog die Tabellen an, setzt haengengebliebene Jobs von `PROCESSING` auf `PENDING` zurueck und startet dann optional den Queue-Worker (`cogs/rename_manager.py:35` bis `cogs/rename_manager.py:42`).

Die Queue arbeitet so:

1. Neuer Rename-Wunsch wird ueber `_enqueue_request()` in `rename_requests` geschrieben; pro Channel bleibt nur der neueste Pending-Wunsch bestehen (`cogs/rename_manager.py:112`).
2. `_claim_next_request()` reserviert genau einen Pending-Job atomar fuer den aktuellen Worker (`cogs/rename_manager.py:163`).
3. `_process_rename_queue()` prueft erst einen lokalen Throttle von 360 Sekunden pro Channel, dann Discord-Ratelimits und Fehler (`cogs/rename_manager.py:256`).
4. Erfolgreiche Jobs gehen auf `DONE`; 429er und temporaere Fehler werden erneut auf `PENDING` gesetzt; nach fuenf Versuchen endet der Job als `FAILED`.

Zusätzlich gibt es einen Retry-Pfad fuer kurzzeitige SQLite-Locks (`cogs/rename_manager.py:135`).

## Konfiguration
Direkte Konstanten:

- `QUEUE_CHECK_INTERVAL_SECONDS = 1.0`
- `RENAME_THROTTLE_SECONDS = 360`
- `MAX_RETRIES = 5`
- `DB_LOCK_MAX_RETRIES = 5`

Wichtige Env-/Settings-Anbindung:

- `USE_DB_RENAME_WORKER` via `settings.use_db_rename_worker` (`cogs/rename_manager.py:38`, `service/config.py:129`)

Wenn `USE_DB_RENAME_WORKER=1`, laeuft dieses Exemplar nur im Enqueue-Modus. Das ist fuer Multi-Process-Setups gedacht, bei denen ein anderer Worker die Queue abarbeitet.

## Admin-Workflow
Es gibt keine eigenen Discord-Commands. Der operative Workflow ist daher:

1. Sicherstellen, dass das Cog geladen ist.
2. Bei Rename-Stau DB-Status der Tabelle `rename_requests` pruefen.
3. Wenn viele `FAILED`-Jobs auftauchen, Discord-Ratelimit oder fehlerhafte Channel-IDs gegenpruefen.
4. Nach Bot-Neustart pruefen, ob ehemals haengende Jobs wieder auf `PENDING` gesetzt wurden.

## Datenmodell
- `rename_requests`: Queue mit `channel_id`, `new_name`, `reason`, `status`, `retry_count`, `last_error`, `assigned_worker_id`.
- `rename_global_state`: nur ein globaler Datensatz; aktuell vor allem fuer `last_rename_timestamp` vorgesehen (`cogs/rename_manager.py:83`).

Wichtig: `queue_local_rename_request()` ist die vorgesehene API fuer andere Cogs (`cogs/rename_manager.py:355`).

## Wartung & Troubleshooting
- Wenn Renames nicht passieren: pruefen, ob das Cog im Enqueue-only-Modus laeuft.
- Wenn Jobs festhaengen: das Cog recovert `PROCESSING` beim naechsten Load automatisch.
- Wenn viele `HTTP 429` auftreten: das ist genau der Fall, fuer den die Queue gebaut wurde; nicht mit Direkt-Renames "fixen".
- Aktuell gibt es in diesem Repo keinen direkten In-Repo-Caller auf `queue_local_rename_request()`. Das macht das Modul technisch interessant, aber derzeit zumindest teilweise dormant.

## Code-Referenz
- Haupt-Cog: `cogs/rename_manager.py:29`
- Schema/Recovery: `cogs/rename_manager.py:66`, `cogs/rename_manager.py:100`
- Queue-Claiming: `cogs/rename_manager.py:135`, `cogs/rename_manager.py:163`
- Queue-Worker: `cogs/rename_manager.py:256`
- Externe API: `cogs/rename_manager.py:355`
