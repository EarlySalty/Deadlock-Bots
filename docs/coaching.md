# Coaching

## Worum geht es?
Das Server-Coaching ist ein kostenloses Community-Angebot. Du schilderst kurz deinen Rang, deinen Hero und deine Baustellen, und danach meldet sich ein echter Coach aus dem Server bei dir. Es geht hier um Human-Coaching, nicht um ein AI-Coaching.

## Wie nutze ich das?
Gehe in `#ich-brauch-einen-coach` und klicke auf den Coaching-Button oder nutze alternativ `/coaching-anfrage`. Im Formular trägst du Rang, Main-Hero, Verfugbarkeit, Games oder Stunden und deine Ziele oder Probleme ein. Danach wird deine Anfrage im Coaching-Channel gepostet, damit ein Coach sie sehen und claimen kann.

Sobald ein Coach ubernimmt, bekommst du Bescheid und stimmst euch direkt im Coaching-Chat auf dem Server ab. Die Kommunikation lauft nur dort, nicht per DM und nicht uber Freundschaftsanfragen. Wenn ihr gemeinsam in einer Coaching-Voice seid und die Session endet, bekommst du eine Feedback-Nachricht und fur kurze Zeit Zugriff auf den Feedback-Kanal.

Du kannst mehr als einmal Coaching anfragen. Mit `/coaching-status` kannst du jederzeit prufen, ob deine letzte Anfrage noch analysiert wird, schon auf einen Coach wartet oder bereits lauft.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Der Bot speichert deine Anfrage, bereitet sie fur Coaches auf und postet sie als Embed im Coaching-Channel. Coaches konnen dort direkt claimen; ab dann lauft fur dich eine aktive Coaching-Phase mit zeitlich begrenzter Rolle. Wenn die Voice-Session erkannt wurde und endet, entfernt der Bot die aktive Rolle wieder, vergibt kurzzeitig die Feedback-Berechtigung und schickt dir den Feedback-Hinweis per DM.

## Grenzen & häufige Fragen
- Coaching lauft nur im Coaching-Chat auf dem Server. Bitte keine DMs und keine Freundschaftsanfragen an Coaches.
- Wenn sich ein Coach meldet, solltest du zeitnah reagieren. Wenn ein Coaching wegen Nicht-Melden abgebrochen wird, gibt es eine vorubergehende Sperre fur neue Anfragen.
- Die aktive Coaching-Phase ist zeitlich begrenzt. Wenn in diesem Fenster keine Session zustande kommt, brauchst du danach eine neue Anfrage.
- Nach dem Coaching solltest du das Feedback ehrlich ausfullen. Kurzes positives Feedback ist okay, konstruktive Kritik aber genauso wichtig.
- Offensichtlich unseriose oder komplett sinnfreie Anfragen konnen vom System ausgesiebt werden und erscheinen dann nicht fur Coaches.
- Es gibt keine feste Garantie fur sofortige Verfugbarkeit. Coaches sind Community-Mitglieder und keine 24/7-Hotline.
- Wenn du den sichtbaren Ablauf einhaeltst, ist das System bewusst simpel: Button, Formular, Coach meldet sich, Session im Server, danach Feedback.

## Für Devs (knapp)
- Cogs: `cogs/coaching_panel.py`, `cogs/coaching_request.py`, `cogs/coaching_role_manager.py`, `cogs/coaching_survey.py`
- Abhangigkeiten: `AIConnector` nur fur die interne Anfragesortierung, Rollen-/Voice-Erkennung uber Guild-State, Feedback-Link uber den konfigurierten Feedback-Channel
- Wichtige DB-Tabellen: `coaching_requests`, `coaching_sessions`, `coaching_bans`, `kv_store`
