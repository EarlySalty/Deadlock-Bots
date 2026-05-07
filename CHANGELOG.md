## #14 — Tracking-Invite und Auswertung für Website-Joins

- Bot legt automatisch einen permanenten Tracking-Invite an und merkt sich den Code in der DB
- Neuer Slash-Befehl `/website-invite` zeigt Status, Code und bisherige Nutzungen
- `/website-invite-recreate` erzeugt bei Bedarf einen neuen Code (z.B. wenn jemand den alten löscht)
- `/join-quellen [tage]` aggregiert die Member-Joins der letzten N Tage nach Quelle (Website, Vanity, Twitch-Streamer, persönliche Einladungen)
- Channel-Default ist der Welcome-Channel, kann via `WEBSITE_INVITE_CHANNEL_ID` Env-Var überschrieben werden

## #13 — TempVoice Sweep löscht keine Staging-Channels mehr

- Staging-Channels sind jetzt gegen den automatischen Sweep geschützt
- Hintergrund: Der Bot hatte den alten Chill Lanes Staging-Channel selbst gelöscht, weil dessen Name einem Lane-Muster entsprach

## #12 — Chill Lanes Staging Channel auf neuen Channel aktualisiert

- Der Staging-Channel für Chill Lanes wurde nach dem Löschen des alten Channels auf den neuen Channel umgestellt
- Alle betroffenen Stellen im Code wurden aktualisiert: TempVoice, LFG und User-Retention-Links

## #11 — Steam-Verknüpfung: Link wird jetzt immer frisch beim Klick erstellt

- Der „Via Steam verknüpfen"-Button im Onboarding generiert den Login-Link jetzt erst beim Klicken – nicht mehr beim Laden der Nachricht
- Dadurch können Links nicht mehr ablaufen, bevor jemand draufklickt
- Das Problem „invalid Launch" bei der Steam-Verifizierung ist damit behoben

## #10 — Vollständige Security-Fortress hinzugefügt

- Neuer Security-Scan läuft täglich: prüft Workflow-Integrität, findet Secrets im Code, scannt Python auf Sicherheitslücken und bekannte CVEs in Dependencies
- Alle Workflow-Dateien sind jetzt auf genaue Commit-Hashes gepinnt — kein Supply-Chain-Angriff über gemutete Action-Tags möglich
- JavaScript-Abhängigkeiten werden auf bekannte Schwachstellen geprüft
- Security-Fortress ergänzt den bestehenden Deep-Scan sinnvoll, ohne Laufzeit zu verschwenden

## #9 — Dependabot-PRs werden jetzt automatisch gemerged + CI-Laufzeiten halbiert

- Dependabot-PRs werden ab jetzt automatisch approved und direkt gemerged (nicht mehr blockiert durch Lint oder DAST)
- Lint und DAST-Scans überspringen Dependabot-PRs, da sie nur Dependency-Dateien ändern — kein Sicherheitsverlust
- Security-Scans laufen weiterhin täglich; Container/IaC/Supply-Chain scannen jetzt wöchentlich statt täglich
- Security-Incident-Automation läuft jetzt täglich statt alle 6 Stunden — 75 % weniger Runs
- Dependency-Review hat keinen sinnlosen Tages-Schedule mehr (läuft weiterhin auf jedem PR)

## #8 — CI-Artifacts werden nach 30 Tagen automatisch gelöscht

- Alle automatisch erzeugten CI-Berichte (Security-Scans, Performance-Reports, Logs) werden ab jetzt nach 30 Tagen automatisch von GitHub entfernt
- Verhindert, dass sich der GitHub-Actions-Speicher dauerhaft volläuft

## #7 — Steam-Bot startet wieder und updated Ränge

- Steam-Bot lief seit dem 27. April nicht mehr — FAs annehmen/senden und Rang-Updates funktionierten nicht
- Ursache: Drei kombinierte Bugs verhinderten den Start (falscher Pfad auf Linux, fehlende Env-Variablen für den Node-Prozess, veraltete native Module)
- Der Bot läuft jetzt stabil und verarbeitet wieder Rang-Checks und Freundschaftsanfragen

## #6 — DB-Pfad fest im Code, kein Datenverlust mehr bei Neustarts

- Der Bot nutzt jetzt immer `data/deadlock.sqlite3` direkt im Repo — egal welche Umgebungsvariablen gesetzt sind oder nicht
- Davor: Nach der Token-Rotation fehlte die `DEADLOCK_DB_PATH`-Variable → Bot startete mit einer leeren Fallback-DB → alle User wirkten wie Neulinge (kein Voice-Verlauf, keine Steam-Links)
- Die fehlenden 2.5 Tage Daten (276 Voice-Sessions, Steam-Links, Nudge-Status etc.) wurden in die Haupt-DB zurückgespielt

## #5 — Steam-Nudge-DM geht nicht mehr mehrfach an denselben User

- Wenn die ursprüngliche Nudge-DM gelöscht wurde (z. B. vom User selbst), schickt der Bot keine zweite DM mehr — die Nachricht ist weg, die Benachrichtigung bleibt trotzdem gesetzt
- Fehlschläge beim Speichern des „bereits benachrichtigt"-Flags werden jetzt im Log sichtbar, statt still ignoriert zu werden

## #4 — Tag-System: bessere Sortierung in Voice-Lanes und LFG

- Du kannst dir jetzt selbst zwei Tags setzen: deinen Lieblings-Ton (Banter-OK oder Ragebaiter-Free) und optional eine Altersangabe (25+ oder U25)
- Setzen geht entweder direkt im Onboarding nach dem Server-Join oder jederzeit per `/meine-tags`
- Voice-Lane-Owner können in ihrer Lane einen 🛡️ Tag-Filter setzen, damit nur Leute mit passendem Ton oder Alter joinen können
- Wer wiederholt Ragebait fährt, bekommt automatisch einen Ragebaiter-Mod-Tag (14 Tage), der ihn aus Ragebaiter-Free-Lanes raushält — Mods können den Tag jederzeit anpassen
- LFG-Suche kann jetzt auch nach Tags filtern, damit Mitspieler besser zur eigenen Stimmung passen

## #3 — Tierlist-Backend: WR-Daten alle 8 Stunden automatisch

- Neuer Service liefert die Hero-Tierliste der Website mit Live-Winrates pro Skill-Bucket
- Daten werden alle 8 Stunden automatisch aktualisiert, immer auf Basis des aktuellen Patches
- Drei Skill-Buckets verfügbar: All, Phantom+, Eternus
- Admin-Endpunkte für Beschreibungen, Streamer-Listen und Schwellen — Login über bestehenden Discord-Flow
- Build-Voting (👍 / 👎) mit Rate-Limit pro Browser

## #2 — Voice Feedback geht nicht mehr an bestehende User nach Bot-Neustart

- Nutzer, die beim Neustart bereits im Voice-Call saßen, bekommen kein fälschliches „erstes Mal"-Feedback mehr
- Prüfung erweitert: beide Tabellen (voice_stats und voice_session_log) werden gecheckt, nicht nur eine
- Feedback-Retry bei DMs-deaktiviert läuft jetzt nicht mehr ewig: der ursprüngliche Zeitstempel bleibt erhalten und fällt nach 72 Stunden aus dem Fenster

## #1 — Sicherheitslücke: Bot-API nicht mehr von außen erreichbar

- Der interne Statistik-Server (Port 8768) war versehentlich von außen direkt erreichbar
- Jetzt lauscht er nur noch auf localhost — externer Zugriff ohne Caddy nicht mehr möglich
