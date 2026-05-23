# Build Publisher

## Zweck
Der `BuildPublisher` ist der Python-Worker, der vorbereitete Hero-Build-Klone aus der Datenbank nimmt und ueber die Steam-Bridge in Deadlock veroeffentlicht. Er ist kein User-Feature und keine Admin-Oberflaeche, sondern das letzte Stueck einer automatisierten Publishing-Pipeline.

## Architektur
Das Cog startet zwei Endlosschleifen:

- Publisher-Loop alle 10 Minuten (`cogs/build_publisher.py:59`)
- Monitor-Loop alle 2 Minuten (`cogs/build_publisher.py:72`)

Der Publisher-Loop liest zuerst `standalone_bot_state` fuer den Steam-Runtime-Zustand (`cogs/build_publisher.py:98`). Nur wenn Steam eingeloggt und der Deadlock-GC bereit ist, werden neue Jobs erzeugt. Danach:

1. Pending-Eintraege aus `hero_build_clones` lesen.
2. Mit `hero_build_sources` und `watched_build_authors` joinen.
3. Pro Hero maximal drei Builds gleichzeitig zulassen; Prioritaet und Recency entscheiden (`cogs/build_publisher.py:149`).
4. Fuer jeden Kandidaten einen `BUILD_PUBLISH`-Task in `steam_tasks` anlegen.
5. Clone-Status auf `processing` setzen und Attempts zaehlen.

Der Monitor-Loop sammelt anschliessend fertige `BUILD_PUBLISH`-Tasks ein, markiert Builds als `uploaded` oder `failed` und setzt haengengebliebene `processing`-Eintraege nach 30 Minuten wieder auf `pending` (`cogs/build_publisher.py:305`).

`docs/build-publishing/AUTONOMER_BETRIEB.md` beschreibt denselben Flow auf Runbook-Ebene. Inhaltlich stimmt der Ablauf noch, einzelne Windows-Pfade dort sind fuer diese Linux-Umgebung aber veraltet.

## Konfiguration
Die Runtime-Konfig steckt direkt im Cog (`cogs/build_publisher.py:26`):

- `enabled`
- `interval_seconds = 600`
- `monitor_interval_seconds = 120`
- `max_attempts = 3`
- `batch_size = 5`

Indirekte Voraussetzungen:

- Steam-Bridge muss `standalone_bot_state` aktuell pflegen.
- Queue-Fueller muss `hero_build_sources` und `hero_build_clones` befuellen.
- Autor-Priorisierung kommt aus `watched_build_authors`.

Es gibt in diesem Cog keine eigenen Discord-Slash-Commands.

## Admin-Workflow
1. Sicherstellen, dass das Cog geladen ist; historisch wurde dafuer `!load build_publisher` genutzt, heute meist ueber den normalen Loader-/Service-Start.
2. Bei ausbleibenden Publishes zuerst Steam-Status bzw. GC-Ready pruefen.
3. Logs auf wiederholte `skipped`- oder `failed`-Meldungen pruefen.
4. Falls noetig den Bot- oder Steam-Bridge-Host neu starten; der Publisher zieht die Queue danach automatisch wieder an.
5. Nur im Ausnahmefall manuell DB-Status der Queue pruefen.

## Datenmodell
Wichtige Tabellen:

- `hero_build_sources`: Quell-Builds der beobachteten Autoren.
- `hero_build_clones`: Publishing-Queue mit Status wie `pending`, `processing`, `uploaded`, `failed`, `cancelled`.
- `steam_tasks`: Bridge-Aufgaben; hier werden `BUILD_PUBLISH`-Jobs erzeugt.
- `standalone_bot_state`: Laufzeitstatus der Steam-Bridge.
- `kv_store` Namespace `build_publisher`: letzter Lauf, Trigger und Fehler (`cogs/build_publisher.py:283`).

## Wartung & Troubleshooting
- `Build publisher skipped: Steam not logged in`: Steam-Session/Bridge prüfen.
- `Build publisher skipped: Deadlock GC not ready`: noch kein GC-Handshake; kurz warten.
- Nach sechs Skips in Folge loggt der Cog absichtlich ein Error-Signal fuer ein echtes Betriebsproblem (`cogs/build_publisher.py:132`).
- Wenn Builds "haengen": der Monitor setzt `processing` nach 30 Minuten automatisch zurueck (`cogs/build_publisher.py:311`).
- Wenn zu viele alte Builds offen sind: der 3-Builds-pro-Hero-Filter storniert ueberschuessige Pending-Eintraege aktiv (`cogs/build_publisher.py:254`).

## Code-Referenz
- Haupt-Cog: `cogs/build_publisher.py:23`
- Queue-Erzeugung: `cogs/build_publisher.py:85`
- GC-/Steam-Gating: `cogs/build_publisher.py:98`
- 3-pro-Hero-Filter: `cogs/build_publisher.py:149`
- Task-Monitoring: `cogs/build_publisher.py:305`
- Runbook: `docs/build-publishing/AUTONOMER_BETRIEB.md:1`
