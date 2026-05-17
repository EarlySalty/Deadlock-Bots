## #32 — Channel aus TempVoice-Verwaltung ausgenommen

- Ein bestimmter Voice-Channel wird vom Bot nicht mehr umbenannt, gelöscht oder verschoben
- Der Channel bleibt weiterhin über die Verwaltungsinterfaces anpassbar

## #31 — Austritts-Umfrage: verstehen, warum Leute gehen

- Wenn jemand den Server verlässt, bekommt er automatisch eine freundliche Nachricht mit einer kurzen Umfrage — die Fragen passen sich an, je nachdem ob jemand neu war oder schon länger aktiv dabei
- Nach der ersten Antwort kommt eine gezielte Nachfrage, damit klarer wird, was genau das Problem war
- Wer ausführlicher Feedback geben will (auch mit Bildern), bekommt einen Link zu einer Feedback-Seite auf der Website
- Neue Auswertung im Admin-Dashboard zeigt, aus welchen Gründen Leute gehen und wie oft geantwortet wird
- Gebannte Mitglieder werden von der Umfrage ausgenommen

## #30 — Lobby-Finder treffsicherer und übersichtlicher

- Passt wirklich eine Lobby zu deinem Rang, wird dir gezielt die eine vorgeschlagen statt einer langen Liste
- Rang-Schnitt einer Lobby zeigt jetzt den echten Rang-Namen (z. B. Oracle) statt kryptischer Kürzel
- Aufgeräumteres Layout mit mehr Abstand, klarem Beitritts-Hinweis und Warnung wenn eine Lobby fast voll ist
- Falsche "über deinem Rang"-Warnung bei Lobbys unter deinem Rang behoben
- Erkennt mehr Nachrichten wie "jemand am start?" oder "noch wer wach?" und sortiert auch unverifizierte Rang-Rollen sauber ein

## #29 — Ehrliche Member-Herkunft im Admin-Dashboard

- Kreis-Diagramm "Wo kommen unsere Member her?" zeigt jetzt einen eigenen Website-Bereich statt Website-Joins in "Persönlich" zu verstecken
- Historische Joins ohne klare Quelle werden nicht mehr als "Twitch" geschätzt, sondern ehrlich als "Unbekannt" markiert
- Pro Website-Subseite gibt es jetzt einen eigenen Discord-Invite-Code (Landing, Streamer, Mitspieler, Coaching, Helden, Guides)
- Hover über Twitch- oder Website-Stück zeigt die Aufschlüsselung pro Streamer bzw. Subseite
- `/website-invite-recreate` rotiert jetzt gezielt nur eine ausgewählte Subseite

## #28 — Alle GitHub Code-Scanning-Alerts behoben

- Sicherheitslücke in der Node.js-Abhängigkeit protobufjs durch Paket-Override geschlossen
- XSS-Gefahr in der Aktivitätsstatistik-Seite behoben (HTML-Escaping für API-Daten)
- Integrity-Attribut für externes Chart.js-CDN-Skript hinzugefügt
- YAML-Syntaxfehler im Auto-Merge-Workflow behoben
- Log-Injection-Schwachstelle im Master-Broker gefixt (sanitisierte Log-Werte)
- Über 50 Code-Quality-Hinweise bereinigt (leere Except-Blöcke, ungenutzte Variablen, Import-Stil)
- 23 bestätigte False Positives (URL-Redirection, Cookie-Injection, SQL-Queries) als solche markiert und geschlossen

## #27 — Dependabot Auto-Merge wieder voll funktionsfähig

- Fehlende Hilfsdatei ergänzt, die den Python-Security-Scanner blockiert hat
- Syntaxfehler in der Auto-Merge-Workflow-Datei behoben (Actionlint-Fehler)
- Alle drei Ursachen, die Dependabot-PRs vom automatischen Merge abgehalten haben, sind beseitigt

## #26 — GitHub Actions Minutenverbrauch stark reduziert

- Neun tägliche Workflows auf wöchentliche oder event-basierte Trigger umgestellt
- Dashboard DAST und Auth-Guardrails laufen jetzt wöchentlich statt täglich
- Security-Scans, Secret-Scanning und Incident-Automation brauchen keinen täglichen Lauf mehr
- Semgrep meldet Findings als Artifact statt den Build zu blockieren

## #25 — SecurityGuard: Kein Auto-Ban ohne AI-Bestätigung

- Jeder Burst-Trigger wird jetzt zuerst von der KI geprüft — kein Ban ohne AI-Bestätigung
- Wenn die KI nicht sicher genug ist, landet der Fall nur als Vorschlag im Mod-Channel (inkl. Ban-Button für manuelle Entscheidung)
- Verhindert Fehlbans bei neuen Accounts die zufällig in mehreren Channels aktiv sind
- Bilder werden ebenfalls AI-geprüft wenn Text alleine nicht ausreicht

## #24 — SecurityGuard: Ban-Logs persistent in DB gespeichert + Review-Channel aktiv

- Jeder automatische Ban/Timeout wird jetzt dauerhaft in der Datenbank gespeichert — Fälle bleiben auch nach Bot-Neustart nachvollziehbar
- Die gespeicherten Daten umfassen Grund, betroffene Channels, Nachrichten-Snippets und Anhang-Anzahl
- Auch KI-erkannte Scam-Fälle bei etablierten Accounts (Timeout-Proposals) werden persistiert
- Ein dedizierter Review-Channel zeigt ab sofort alle Incidents als Embed an

## #23 — KI-Moderationskontext deutlich verbessert

- Kontext-Nachrichten zeigen jetzt relative Zeitstempel (z.B. "[2min ago]") — die KI erkennt ob der Kontext frisch oder veraltet ist
- Nachrichten vom selben User der bewertet wird, sind mit ">>>" markiert — so sieht die KI das Verhaltensmuster des Users klar
- Reply-Chain: wenn jemand auf eine Nachricht antwortet, bekommt die KI die Original-Nachricht direkt mit — Reaktionen werden im richtigen Kontext bewertet
- System-Prompt erklärt jetzt explizit: Reaktion auf eine Provokation wird milder bewertet als die Provokation selbst

## #22 — KI-Moderationsfilter entschärft: weniger False Positives

- Schwellenwert für Moderationsvorschläge von 55% auf 78% Konfidenz angehoben
- Die KI holt jetzt nur noch 12 statt 25 Nachrichten Kontext — verhindert, dass Kontext-Rauschen aus dem Gaming-Channel die Bewertung verfälscht
- System-Prompt präzisiert: Gaming-Klischees (Nationalitäten), kurze Ein-Wort-Antworten und einmalige Beleidigungen werden nicht mehr gemeldet
- Wichtig: Wer schlechtes Verhalten meldet oder kommentiert, wird nicht mehr selbst geflaggt

## #21 — Bild-Scam-Erkennung: KI analysiert Screenshots in mehreren Channels

- Accounts die nur Bilder senden (z.B. gefälschte X-Posts mit Investitions-Gewinnen) werden jetzt erkannt
- Sobald jemand Bilder in 2+ verschiedenen Channels schickt, prüft die KI automatisch ob es Scam ist
- Junge Accounts (<30 Tage) werden bei bestätigtem Bild-Scam direkt gebannt
- Ältere Accounts bekommen einen 24h-Timeout + DM und Mods sehen den Fall im Mod-Channel
- Message-Verlauf wird jetzt für alle Accounts getrackt, nicht nur für neue

## #20 — Klare Grenze: jung = bis 30 Tage, etabliert = älter als 30 Tage

- Accounts bis 30 Tage alt werden bei erkanntem Scam direkt gebannt
- Accounts älter als 30 Tage (möglicherweise gehackt) bekommen stattdessen einen 24h-Timeout und eine DM
- Der KI-Scam-Check läuft jetzt für alle Accounts, nicht mehr nur für neue
- Mehrkanal-Burst-Erkennung bleibt weiterhin auf Accounts unter 30 Tagen begrenzt

## #19 — Etablierte Scam-Accounts werden sofort getimeoutet und per DM informiert

- Accounts die älter sind und möglicherweise gehackt wurden, werden nicht mehr nur gemeldet, sondern sofort für 24h stummgeschaltet
- Der betroffene User bekommt automatisch eine DM: Grund, Dauer, und der Hinweis sich beim Mod-Team zu melden sobald der Account wieder sicher ist
- Mods sehen im Mod-Channel trotzdem ein Info-Embed mit zwei Buttons: "Ban" (eskalieren) oder "Timeout aufheben" (falls False Positive)
- Kein manuelles Bestätigen mehr nötig — der Bot handelt sofort

## #18 — Scam-Erkennung mit KI und erweiterter Account-Prüfung

- Scam-Nachrichten werden jetzt auch von Accounts erkannt, die bis zu einem Monat alt sind (vorher nur 24 Stunden)
- Einzelne Nachrichten mit verdächtigen Inhalten werden per KI automatisch auf Scam geprüft — kein Multi-Channel-Spam mehr nötig
- Neue Accounts werden bei KI-bestätigtem Scam automatisch gebannt
- Ältere, etablierte Accounts (möglicherweise gehackt) werden nicht automatisch gebannt — stattdessen erscheint ein Mod-Vorschlag mit Ban-Button im Mod-Channel
- Das bisherige Join-Zeitfenster als Bedingung wurde entfernt

## #17 — TempVoice greift nicht mehr in fremde Voice-Kategorien ein

- Channels außerhalb der TempVoice-Kategorien (Chill, Comp, Street Brawl) werden nicht mehr umbenannt oder gelöscht
- Betrifft z.B. Custom Games oder andere manuelle Voice-Channels
- Verhindert, dass Channels mit Namen wie "Lane 1" in falschen Kategorien fälschlicherweise als TempVoice-Lane erkannt werden

## #16 — FAQ-Bot antwortet automatisch in neuen Tickets

- Wenn in der Support-Kategorie ein neues Ticket aufgemacht wird, analysiert der FAQ-Bot die erste Nachricht des Users
- Hat der Bot eine passende Antwort aus der Dokumentation, antwortet er direkt im Ticket
- Kann der Bot das Problem nicht lösen, schreibt er gar nichts – der Mensch übernimmt dann wie gewohnt
- Gilt für alle Channels die mit "ticket-" beginnen in der ❓Support-Kategorie

## #15 — FAQ-Bot erkennt Onboarding- und Invite-Probleme automatisch

- FAQ-Bot gibt jetzt bei "kein Invite" oder "kann nicht herunterladen" sofort eine Schritt-für-Schritt-Anleitung aus
- Checkliste: Onboarding abgeschlossen? Richtige Option gewählt? Rollen gesetzt? /betainvite verwendet? Steam-Kauf vorhanden?
- Bot weist freundlich aber klar darauf hin wenn das Onboarding-Lesen das Problem gelöst hätte
- Neue Dokumentationsdatei mit dem vollständigen Invite-Ablauf für die Bot-Wissensbasis

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
