# Website-Portale

## Worum geht es?
Die Website ist kein einzelner Monolith, sondern besteht aus mehreren klar getrennten Portalen. Für Nutzer sind vor allem diese Einstiege relevant: die Landing-Website, das Aktivitäts-Portal, das Patch-Notes-Portal, das Tierlist-/Builds-Portal und der Coaching-Bereich (inklusive Scrim-Anmeldung). Jedes Subportal löst einen anderen Teil der Community-Erfahrung ab, vom Einstieg über Statistiken bis zu Meta- und Patch-Inhalten.

## Landing
Die Landing-Seite ist die zentrale Startoberfläche der Community. Sie ist als Multi-Page-Frontend aufgebaut und führt auf mehrere Unterseiten weiter, darunter Start, Mitspieler, Coaching, Streamer, Helden-Übersicht, Anfänger-Guide und Survey.

Aus Nutzersicht bietet die Landing vor allem:

- einen klaren Community-Einstieg mit Discord-CTA
- eine Navigation auf die anderen Portale
- redaktionelle Seiten wie Guide- und Hero-Übersicht
- Community-Signale wie Invite- und Serverdaten

Im Frontend werden dafür Discord-Invite- und Widget-Daten live abgefragt. So können Mitgliederzahlen, Online-Zahlen oder sichtbare Channel-Bereiche eingeblendet werden, ohne dass die Seite manuell gepflegt werden muss.

## Activity
Das Aktivitäts-Portal ist der Statistikbereich. Dort gibt es mehrere Tabs für Voice, Text, Peaks und einen persönlichen Bereich. Ein Teil ist öffentlich sichtbar, der persönliche Bereich hängt am Discord-Login.

Öffentlich zugänglich sind unter anderem:

- Voice-Leaderboards
- Text-Leaderboards
- Rank-Distribution
- Aktivitäts-Timelines über mehrere Tage oder Wochen

Nach Login kommen persönliche Ansichten dazu: eigene Stats, Voice- und Text-Historie, Heatmap und Co-Player-Ansicht.

Die Oberfläche arbeitet stark mit Diagrammen und aggregierten API-Endpunkten. Für Nutzer heißt das: Man bekommt keine Rohdatenbank, sondern bereits aufbereitete Übersichten, die auf Vergleich, Verlauf und Aktivitätsmuster ausgerichtet sind. (Technisch kommen diese Daten vom eigenen Stats-Service des Bots, nicht vom Website-Backend.)

## Patch Notes
Das Patch-Portal ist ein lesbarer Patch-Archiv-Bereich. Die Seite lädt Patch-Daten aus einer öffentlichen API und zeigt sie als interaktive Ansicht: Zeitleiste mit Patch-Balken, Übersicht der betroffenen Helden/Items und ein Event-Feed mit Detailansicht.

Wichtige Funktionen:

- Volltextsuche über Patch-Inhalte
- Kategorien-Filter
- Zeitleisten-Ansicht über alle erfassten Patches
- Detailansicht pro Änderung
- Link zurück zur Originalquelle

Der Nutzer bekommt damit keine rohe Forenansicht, sondern eine aufbereitete, deutsch lesbare Archiv-Seite. Bei Ladeproblemen zeigt das Portal explizit eine Fehlermeldung bzw. einen Fallback statt still leer zu bleiben.

## Tierlist / Builds
Das Tierlist-Portal (unter `/builds/`) ist der Meta-Bereich für Hero-Rankings und Build-Empfehlungen. Im ausgelieferten Public-Frontend sind vor allem drei Dinge sichtbar:

- aktuelle Tierliste
- Build-Voting
- Tierlist-Historie

Die Public-Tierlist unterstützt unterschiedliche Buckets wie `all`, `phantom_plus` und `eternus`. Nutzer können zwischen Grid- und Listenansicht wechseln, nach Heroes suchen und einzelne Heroes aufklappen. Im Detailpanel sieht man Build-Beschreibungen, Kernitems, Ability-Order und kann positiv oder negativ voten.

Zusätzlich gibt es eine History-Seite, auf der Snapshots verschiedener Patches oder Abrufe gegeneinander gestellt werden. Falls das Live-Backend ausfällt, besitzt das Portal einen Static-Fallback mit zuletzt gespeicherten JSON-Daten — die Seite fällt dann nicht komplett aus, sondern zeigt weiterhin einen brauchbaren Stand.

## Coaching & Scrims
Der Coaching-Bereich ist ein eigenes Portal mit Discord-Login: Anfrage-Formular für kostenloses Coaching (`/anfrage`), Scrim-Anmeldung und für Coaches eine eigene Plattform mit Warteschlange, Coachee-Details, Zielen/Notizen und Terminen. Details stehen in der Coaching-Doku.

## Was passiert technisch?
Die Website kombiniert mehrere Vite-Frontends mit klaren Basispfaden. Einige Portale sind rein statisch, andere sprechen Live-APIs an oder brauchen einen Discord-Login. Das `builds`-Backend läuft in Rust und stellt Router für Auth, Heroes, Builds, Tierlists, Patch-Daten, History, Admin, Coaching, Coach-Plattform und Scrims bereit; die Activity-Daten kommen vom separaten Stats-Service. Für Nutzer zeigt sich das vor allem in zwei Dingen: schnelle, getrennt deploybare Teilseiten und unterschiedliche Funktionslevel je nach Login oder Live-API-Verfügbarkeit.
