# Coaching

## Worum geht es?
Das Server-Coaching ist ein kostenloses Community-Angebot. Du schilderst kurz deinen Rang, deine Ziele und deine Baustellen, und danach meldet sich ein echter Coach aus dem Server bei dir. Es geht hier um Human-Coaching, nicht um ein AI-Coaching.

## Wie nutze ich das?
Die Anfrage läuft über die **Website**: Im Kanal <#1494373349944459355> bringt dich der Button zur Coaching-Seite, und auch `/coaching-anfrage` gibt dir den Website-Link. Dort meldest du dich mit Discord an und füllst das Anfrage-Formular aus (Rang, Ziele, Verfügbarkeit und was dich gerade aufhält).

Deine Anfrage landet danach automatisch beim Coach-Team im Discord: Sie wird dort mit Claim-Buttons gespiegelt, ein Coach übernimmt sie und meldet sich bei dir. Die Abstimmung läuft im Coaching-Chat auf dem Server — nicht per DM und nicht über Freundschaftsanfragen. Wenn ihr gemeinsam in einer Coaching-Voice seid und die Session endet, bekommst du eine Feedback-Nachricht und für ein paar Tage Zugriff auf den Feedback-Kanal.

Du kannst mehr als einmal Coaching anfragen. Mit `/coaching-status` prüfst du jederzeit, ob deine letzte Anfrage noch offen ist, auf einen Coach wartet oder bereits läuft.

Dazu gehört auch der **Scrim-Bereich**: Über die Coaching-Website kannst du dich für Scrims (Übungsspiele im Team) anmelden — Teams üben zusammen und machen sich gegenseitig besser.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Das Website-Formular speichert die Anfrage; eine Spiegelung postet sie als Embed mit Claim-/Freigabe-/Abbruch-Buttons ins Coach-Team. Ab dem Claim läuft für dich eine aktive Coaching-Phase mit zeitlich begrenzter Rolle. Wenn die gemeinsame Voice-Session endet, entfernt der Bot die aktive Rolle, vergibt kurzzeitig die Feedback-Berechtigung (5 Tage) und schickt dir den Feedback-Hinweis per DM. Für Coaches gibt es auf der Website eine eigene Plattform mit Warteschlange, Coachee-Details, Zielen/Notizen und Terminen.

## Grenzen & häufige Fragen
- Coaching läuft nur im Coaching-Chat auf dem Server. Bitte keine DMs und keine Freundschaftsanfragen an Coaches.
- Wenn sich ein Coach meldet, solltest du zeitnah reagieren. Wird ein Coaching wegen Nicht-Melden abgebrochen, kann eine 7-Tage-Sperre für neue Anfragen gesetzt werden.
- Die aktive Coaching-Phase ist zeitlich begrenzt. Wenn in diesem Fenster keine Session zustande kommt, brauchst du danach eine neue Anfrage.
- Nach dem Coaching solltest du das Feedback ehrlich ausfüllen. Kurzes positives Feedback ist okay, konstruktive Kritik aber genauso wichtig.
- Es gibt keine feste Garantie für sofortige Verfügbarkeit. Coaches sind Community-Mitglieder und keine 24/7-Hotline.
- Der sichtbare Ablauf ist bewusst simpel: Website-Formular, Coach meldet sich, Session im Server, danach Feedback.

## Für Devs (knapp)
- Rust live: `dl-community/src/coaching_requests.rs` (Spiegelung, Claim/Release/Abort, Status, Survey, Sperren), `coaching.rs` (Coach-Rollen-Sync + Notification-Polling)
- Website: `builds/backend-rust/src/routes/coaching.rs` + `platform.rs` (Coach-Plattform), Frontend `dl-coaching/` (Anfrage-Seite `/anfrage`, Scrim-Anmeldung)
- Wichtige DB-Tabellen: Coaching-Anfragen/Sessions/Sperren zentral in Postgres; Anfragen entstehen auf der Website, nicht mehr im Discord-Modal
