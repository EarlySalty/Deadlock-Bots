# Twitch-Clips und Social Media

## Worum geht es?

Der Twitch-Bot kann Clips nicht nur sammeln, sondern zu einer ganzen Social-Media-Pipeline weiterverarbeiten: Clips werden automatisch gefunden, aufbereitet, vor dem Posten geprueft, auf Plattformen verteilt und spaeter wieder aus dem lokalen Speicher entfernt. Aus Streamer-Sicht ist das die Bruecke zwischen Twitch-Momenten und TikTok, YouTube Shorts oder Instagram Reels.

## Was merkt man als Streamer?

Zuerst einmal: Deine Clips tauchen nicht nur manuell auf. Der Bot durchsucht aktive Partnerkanaele automatisch alle `6 Stunden` nach neuen Twitch-Clips. Pro Lauf werden die letzten `7 Tage` betrachtet und pro Streamer bis zu `20` Clips gezogen.

Danach passiert fuer jeden Clip grob dieser Ablauf:

1. Der Clip wird registriert und landet als neuer Kandidat im System.
2. Ein Enrichment-Schritt erzeugt Transkript, korrigiert Begriffe und baut Titel, Beschreibungen und Hashtags pro Plattform.
3. Vor dem Upload gibt es einen Approval-Schritt in Discord.
4. Erst nach Freigabe wird in die Upload-Queue geschoben.
5. Nach dem Upload sammelt das System Performance-Daten.

Wichtig: Das aktuelle Social-Media-Dashboard ist im Frontend noch admin-only. Als normaler Streamer merkst du die Funktion heute daher vor allem daran, dass Clips automatisch vorbereitet und gepostet werden, nicht daran, dass du schon jeden Schritt selbst im UI freischalten kannst.

## Approval: was bedeutet das praktisch?

Clips gehen nicht blind live. Nach dem Enrichment werden sie auf `awaiting approval` gesetzt. Die Freigabe laeuft aktuell ueber Discord-DMs an den internen Freigabe-Flow. Das bedeutet fuer dich:

- ein Clip kann vorbereitet sein, ohne sofort gepostet zu werden
- Plattformen koennen einzeln freigegeben werden
- Clips koennen auch bewusst uebersprungen oder verworfen werden

Fuer Zuschauer ist das sinnvoll, weil dadurch weniger halbfertige oder unpassende Posts nach aussen gehen.

## Enrichment: was wird automatisch verbessert?

Der Bot erzeugt nicht nur rohe Uploads. Er baut pro Clip zusaetzlich:

- Roh- und Korrekturtranskript
- erkannte Deadlock-Begriffe
- plattformspezifische Titel
- plattformspezifische Beschreibungen
- Hashtags fuer YouTube, TikTok und Instagram

Dadurch wirken die Posts nicht wie einfache Twitch-Exports, sondern eher wie Social-Cutdowns mit passender Verpackung. Wenn lokal kein nutzbares Sprach- oder LLM-Setup verfuegbar ist, kann ein Clip auch mit reduziertem Automationsgrad weiterlaufen statt komplett zu verschwinden.

## Upload und Plattformen

Aktuell ist die Pipeline fuer folgende Ziele gebaut:

- TikTok
- YouTube
- Instagram

Vor dem Upload wird der Clip bei Bedarf heruntergeladen, fuer Vertical-Video verarbeitet und dann pro Plattform hochgeladen. Es gibt auch manuelle Uploads: Nicht jeder Clip muss von Twitch selbst stammen. Die Pipeline kennt sowohl Twitch-Quellen als auch manuell hochgeladene Dateien.

## Retention: wie lange bleibt ein Clip im System?

Die lokale Clip-Retention ist konkret: `14 Tage ab Erstellung`. Diese Frist wird beim Speichern gesetzt. Der Retention-Worker raeumt Clips weg, wenn mindestens eine der folgenden Bedingungen passt:

- der Clip wurde auf allen aktiven Plattformen veroeffentlicht
- der Clip wurde bewusst verworfen

Dann wird erst die lokale Datei geloescht und anschliessend der Datenbankeintrag entfernt. Fuer dich heisst das: Das System ist nicht als unendliches Roharchiv gedacht, sondern als produktive Upload-Pipeline mit begrenzter Zwischenhaltung.

## Analytics: was kommt nach dem Upload?

Nach veroeffentlichten Clips sammelt der Bot spaeter Leistungsdaten nach:

- Views
- Likes
- Kommentare
- Shares
- je nach Plattform weitere Kennzahlen wie Watch Time oder CTR

Die Analytics werden nicht nur einmal gelesen, sondern in Buckets fuer `24h`, `7d` und `30d` nachgezogen. Daraus entstehen Wochen- und Monatsreports. Viewer merken davon indirekt, dass erfolgreiche Formate eher wiederholt werden und schwache Formate schneller auffallen.

## Grenzen und wichtige Hinweise

- Der Social-Media-Self-Service ist aktuell noch nicht breit fuer alle Streamer freigegeben.
- Freigabe ist ein echter Stopper: ohne Approval kein Upload.
- Retention nach 14 Tagen betrifft die lokale Zwischenhaltung, nicht automatisch die bereits geposteten Plattform-Inhalte.
- Analytics kommen zeitversetzt, nicht sofort im Moment des Uploads.
