# Turniere

## Worum geht es?
Das Turnier-Portal deckt den kompletten Spielerweg ab: Profil und Einwilligung hinterlegen, einem Event beitreten, ein Team gründen oder einem Team anschließen, Check-in erledigen und später Bracket, Gruppen und Leaderboard verfolgen. Sichtbar für Spieler sind vor allem die öffentlichen Turnierseiten, der eigene Status im Event, Team-Einladungen und die globale Rangliste.

## Wie läuft die Anmeldung ab?
Vor der ersten Teilnahme musst du eingeloggt sein und den Consent-Flow abschließen. Ohne aktuelle Einwilligung blockiert das System jede Team- oder Solo-Anmeldung. Im selben Bereich kannst du auch dein Spielerprofil pflegen: Anzeigename, Bio, Avatar und Benachrichtigungsoptionen für Matchstart, Check-in, Team-Einladungen und Turnier-News.

Danach gibt es drei übliche Wege in ein Turnier:

- Du meldest dich solo an. Dann landest du zunächst als freier Spieler im Turnierpool.
- Du gründest ein eigenes Team. Das System setzt dich automatisch als Captain und trägt deine vorhandenen Rank-/Steam-Daten mit ein.
- Du trittst einem bestehenden Team bei oder nimmst eine Einladung an.

Anmeldung und Teamverwaltung sind nur offen, solange das Turnier im Status `registration` oder `checkin` ist. Wenn ein Team schon voll ist, blockiert der Join. Wer bereits in einem Team desselben Turniers steckt, kann nicht parallel noch einem zweiten Team beitreten.

## Team- und Recruit-Flow
Captains können Teams auf drei Arten auffüllen:

- Direkte Einladung eines solo angemeldeten Spielers
- Einladung über einen konkreten Signup-Eintrag
- Bewerbungsmodus eines Teams

Teams haben dafür einen sichtbaren Recruiting-Status:

- `open`: offen für direkte Beitritte
- `application`: Interessenten schicken eine Bewerbung
- `closed`: kein Recruiting

Wenn du Einladungen lieber automatisch annehmen willst, kann dein Profil das speichern. Dann wird eine passende Team-Einladung sofort angenommen, ohne dass du noch einmal manuell klicken musst. Offene Einladungen kannst du außerdem gesammelt abrufen sowie einzeln annehmen oder ablehnen.

## Check-in und Turnierstatus
Für aktive Events gibt es einen separaten Check-in. Das System speichert, ob du bereits eingecheckt bist, und führt eine öffentliche Statusübersicht mit Anzahl registrierter und eingecheckter Spieler. Ohne gültigen Check-in riskierst du, beim Finalisieren des Teilnehmerfelds nicht berücksichtigt zu werden.

Öffentlich sichtbar sind außerdem:

- Turnierliste
- Turnierdetails
- Teams und Mitglieder
- Gruppenphase
- Bracket
- freie Signups

## Welche Modi unterstützt das System?
Der Backend-Code unterscheidet zwei Turniermodi:

- `bracket_only`: direktes K.-o.-Format
- `group_stage`: Gruppenphase plus anschließendes Bracket

Automatisch wird ab 16 Teams auf Gruppenphase geschaltet; darunter bleibt es bei `bracket_only`. Admins können das per Override erzwingen. Zusätzlich gibt es Single- und Double-Elimination-Brackets. Für Spieler heißt das vor allem: Je nach Event spielst du entweder sofort im K.-o.-Baum oder zuerst in Gruppen und danach in den Playoffs.

## Leaderboard und Spielerprofil
Das globale Leaderboard ist öffentlich und sortiert nach Turnierpunkten. Angezeigt werden unter anderem:

- Gesamtpunkte
- gespielte Turniere
- gespielte und gewonnene Matches
- beste Platzierung
- aktueller Rank, falls vorhanden

Zusätzlich gibt es öffentliche Spielerprofile. Dort siehst du Anzeigename, Avatar, Bio, Rank, Statistiken und die bisherige Turnierhistorie mit Teamnamen. Das ist besonders nützlich, wenn du Kandidaten für dein Team oder bekannte Gegner einordnen willst.

## Draft-System aus Spielersicht
Ein Draft-System ist technisch vorhanden, aber aktuell kein Self-Service-Feature für normale Spieler. Die Draft-Session wird von Admin-Seite für ein Match gestartet und dort auch bedient. Für dich als Spieler bedeutet das praktisch:

- Es gibt eine definierte Hero-Liste für Drafts.
- Drafts hängen an konkreten Matches.
- Die Bedienung läuft derzeit nicht über öffentliche Spieler-Endpunkte.

Wenn ein Event Draft-Regeln nutzt, bekommst du die Picks/Bans also nicht über ein eigenes Spieler-Dashboard, sondern über die Turnierleitung oder den Match-Flow mitgeteilt.

## Häufige Grenzen
- Ohne aktuelle Einwilligung (`Consent`) ist keine Anmeldung möglich.
- Captain-Wechsel und Team-Auflösung haben Sonderregeln: Ein Captain kann nicht einfach verschwinden, solange noch andere Teammitglieder im Kader sind.
- Ein Widerruf der Einwilligung ist blockiert, solange du in einem aktiven Turnier angemeldet bist.
- Offene Einladungen und Bewerbungen sind an den Turnierstatus gebunden und funktionieren nicht mehr beliebig spät.

Kurz technisch: Das Portal speichert Profile, Signups, Einladungen, Bewerbungen, Check-ins und Matchbäume serverseitig. Rank- und Steam-Bezüge werden beim Anmelden oder Teamwechsel nachgeladen, damit das Bracket und das öffentliche Profil konsistent bleiben.
