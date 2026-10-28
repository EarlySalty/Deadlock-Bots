# Autonomer Betrieb - Deadlock Build-Publishing

> **Historical (legacy Python implementation).** This page describes the Python cogs that the Rust port replaced. The `cogs/` paths, line numbers and `!` commands below do not exist on `origin/main`. Use it as a behaviour reference only and check `rust/bin/dl-bot/src/` before relying on any detail.


## ✅ System ist KOMPLETT AUTONOM!

Das Build-Publishing-System ist **vollständig für autonomen Betrieb konfiguriert** und benötigt **KEINE manuellen Eingriffe**.

---

## Wie funktioniert der autonome Betrieb?

### 1. Steam-Bridge (Node.js)

**Auto-Start:**
- Wird vom `standalone_manager` automatisch gestartet
- Kommando: `node C:\Users\Nani-Admin\Documents\Deadlock\cogs\steam\steam_presence\index.js`

**Auto-Login:**
```javascript
// index.js Zeile 2546
autoLoginIfPossible();
```
- Prüft beim Start ob `refresh.token` existiert
- Loggt sich automatisch ein wenn Token vorhanden
- **Kein manueller Login nötig!**

**Auto-Reconnect:**
```javascript
// index.js Zeile 968
scheduleReconnect(reason)
```
- Bei Verbindungsverlust: automatischer Reconnect nach 5s
- Verwendet Refresh-Token für Re-Login
- Kein manueller Eingriff nötig

**Crash-Recovery:**
- `standalone_manager` erkennt Crash automatisch
- Restart nach 5 Sekunden
- Auto-Login beim Neustart

### 2. Build Publisher (Python Cog)

**Auto-Start:**
- Wird beim Bot-Start automatisch geladen
- Läuft alle 10 Minuten

**GC-Ready-Check:**
```python
# Prüft vor jedem Run:
if not logged_in or not gc_ready:
    log.warning("Skipped: GC not ready")
    return  # Wartet auf nächsten Run
```
- Wartet automatisch bis Steam eingeloggt
- Wartet automatisch bis GC-Handshake komplett
- Keine manuellen Eingriffe nötig

**Error-Handling:**
- Max 3 Attempts pro Build
- Automatische Retries alle 10 Minuten
- Failed Builds werden markiert, Queue läuft weiter

**Monitoring:**
```python
if consecutive_skips >= 6:  # 60 Minuten
    log.error("GC not ready for 60min!")
```
- Automatisches Alerting bei Problemen
- Logs in `master_bot.log`

### 3. Build Mirror (Python Cog)

**Auto-Sync:**
- Läuft alle 4 Stunden automatisch
- Holt neue Builds von Source-Accounts
- Fügt sie in Queue ein

---

## Workflow (komplett autonom)

```
1. Bot startet
   └─> Standalone Manager startet Steam-Bridge
       └─> Steam-Bridge loggt sich automatisch ein
           └─> GC-Handshake (30-60s)
               └─> GC Ready!

2. Build Mirror (alle 4h)
   └─> Holt Builds von API
       └─> Speichert in hero_build_sources
           └─> Erstellt hero_build_clones (pending)

3. Build Publisher (alle 10min)
   ├─> Prüft: Steam eingeloggt? GC ready?
   │   ├─> NEIN → Skip, warte 10min
   │   └─> JA → Weiter
   └─> Nimmt 5 pending Builds
       └─> Erstellt BUILD_PUBLISH Tasks
           └─> Status: pending → processing

4. Steam-Bridge (Echtzeit)
   └─> Führt BUILD_PUBLISH Task aus
       └─> Sendet via GC (Message 9193)
           ├─> Erfolg → Status: uploaded
           └─> Fehler → Status: processing (Retry in 10min)

5. Build Publisher Monitor
   └─> Prüft abgeschlossene Tasks
       └─> Updated Clone-Status
           ├─> uploaded (mit Build-ID)
           └─> failed (nach 3 Attempts)
```

**Alles läuft automatisch, keine Eingriffe nötig!**

---

## Was passiert bei Problemen?

### Crash der Steam-Bridge

1. `standalone_manager` erkennt Crash
2. Wartet 5 Sekunden
3. Startet Bridge neu
4. Bridge loggt sich automatisch ein
5. GC-Handshake läuft
6. Nach 30-60s: System bereit
7. Build Publisher arbeitet weiter

**Keine manuellen Eingriffe nötig!**

### Steam Session-Timeout

1. Steam-Session läuft ab
2. `error` Event: SessionExpired
3. Auto-Reconnect nach 5s
4. Login mit Refresh-Token
5. GC-Handshake
6. System bereit

**Keine manuellen Eingriffe nötig!**

### GC-Handshake schlägt fehl

1. Build Publisher prüft GC-Status
2. `gc_ready: false` → Skip
3. Log: "GC not ready, waiting"
4. Wartet 10 Minuten
5. Nächster Run: Erneute Prüfung
6. Wenn 60min nicht ready: ERROR-Log

**System wartet automatisch, keine Eingriffe nötig!**

### Build-Publishing schlägt fehl

1. Task Status: FAILED
2. Clone bleibt in "processing"
3. Nach 10min: Erneuter Versuch (Attempt 2)
4. Nach 3 Fehlversuchen: Status "failed"
5. Queue läuft weiter mit anderen Builds

**Automatisches Retry-System!**

---

## Konfiguration

### Build Publisher (`cogs/build_publisher.py`)

```python
self.enabled = True                # An/Aus
self.interval_seconds = 10 * 60    # Intervall (10min)
self.max_attempts = 3              # Max Retries
self.batch_size = 5                # Builds pro Run
```

### Build Fetching (Node.js `index.js` - DISCOVER_WATCHED_BUILDS)

Build fetching wird automatisch vom Node.js Steam Bridge durchgeführt.
Konfiguration erfolgt über `watched_build_authors` Tabelle in der Datenbank.

### Steam-Bridge (Umgebungsvariablen)

```bash
# Optional - hat bereits gute Defaults
STEAM_TASK_POLL_MS=2000           # Task-Polling-Intervall
DEADLOCK_GC_READY_TIMEOUT_MS=30000  # GC-Ready-Timeout
```

---

## Monitoring

### Logs prüfen

**Master Bot:**
```bash
tail -f C:\Users\Nani-Admin\Documents\Deadlock\logs\master_bot.log
```

Wichtige Meldungen:
- `Build publisher skipped: GC not ready` - Normal, wartet
- `Build publisher: GC not ready for 60min` - Problem! Steam-Bridge prüfen
- `Created BUILD_PUBLISH task #XXX` - Task wurde erstellt
- `Build XXX published successfully` - Erfolg!

**GC-Kommunikation:**
```bash
tail -f C:\Users\Nani-Admin\Documents\Deadlock\logs\deadlock_gc_messages.log
```

Wichtige Events:
- `gc_ready` - GC ist bereit
- `send_update_hero_build` - Build wird gesendet
- `gc_message msgType 9194` - Response vom GC

### Datenbank-Status

```bash
python check_build_queue.py
python check_steam_status.py
```

### Erwartete Log-Muster

**Normal (alles gut):**
```
2025-12-06 10:00:00 - Build publisher run completed: 3 queued, 0 errors, 3 checked
2025-12-06 10:00:30 - Build 291314 published successfully as #123456
2025-12-06 10:10:00 - Build publisher run completed: 2 queued, 0 errors, 2 checked
```

**GC nicht ready (wartet):**
```
2025-12-06 10:00:00 - Build publisher skipped: Deadlock GC not ready (skip #1, waiting for handshake)
2025-12-06 10:10:00 - Build publisher skipped: Deadlock GC not ready (skip #2, waiting for handshake)
2025-12-06 10:20:00 - Build publisher run completed: 5 queued, 0 errors, 5 checked
```

**Problem (GC zu lange nicht ready):**
```
2025-12-06 11:00:00 - Build publisher: GC not ready for 6 intervals (60 min). Check Steam bridge!
```
→ Steam-Bridge manuell prüfen oder neu starten

---

## Setup für Autonomen Betrieb

### Einmalig: Build Publisher aktivieren

**Option 1: main_bot.py bearbeiten**

Füge hinzu:
```python
initial_extensions = [
    # ... existing ...
    "cogs.build_publisher",  # NEU
]
```

**Option 2: Via Discord**
```
!load build_publisher
```

### Das war's!

System läuft jetzt komplett autonom:
- ✅ Steam-Bridge loggt sich automatisch ein
- ✅ Build Mirror holt Builds alle 4h
- ✅ Build Publisher arbeitet Queue alle 10min ab
- ✅ Crashes werden automatisch recovered
- ✅ Errors werden automatisch retried

**Null manuelle Eingriffe nötig!**

---

## FAQ

### Muss ich den Steam-Bot einloggen?

**NEIN!** Auto-Login ist aktiviert:
- Refresh-Token existiert: `.steam-data/refresh.token`
- Wird beim Start automatisch verwendet
- Bei Crash: Automatischer Re-Login

### Muss ich Tasks erstellen?

**NEIN!** Build Publisher erstellt automatisch Tasks:
- Alle 10 Minuten
- Für alle pending Builds
- Max 5 pro Run

### Was passiert nachts?

**System läuft 24/7:**
- Build Mirror: 00:00, 04:00, 08:00, 12:00, 16:00, 20:00
- Build Publisher: Alle 10 Minuten
- Steam-Bridge: Immer online

### Muss ich die Queue überwachen?

**Nur wenn du willst!**
- System arbeitet autonom
- Bei Problemen: ERROR-Logs
- Optionales Monitoring mit Scripts

### Was wenn der Server neu startet?

1. Master Bot startet
2. Standalone Manager startet Steam-Bridge
3. Steam-Bridge loggt sich automatisch ein
4. Build Publisher läuft nach 30s weiter
5. System normal online

**Alles automatisch!**

---

## Zusammenfassung

✅ **Komplett autonom** - Null manuelle Eingriffe
✅ **Auto-Login** - Refresh-Token-basiert
✅ **Auto-Recovery** - Crashes werden automatisch gehandelt
✅ **Auto-Retry** - Failed Tasks werden wiederholt
✅ **Auto-Monitoring** - ERROR-Logs bei Problemen
✅ **24/7-Betrieb** - Läuft durchgehend

**Einfach `!load build_publisher` ausführen und vergessen!**
