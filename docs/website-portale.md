# Website-Portale

## Worum geht es?
Die Website ist kein einzelner Monolith, sondern besteht aus mehreren klar getrennten Portalen. Für Nutzer sind vor allem vier Einstiege relevant: die Landing-Website, das Aktivitäts-Portal, das Patch-Notes-Portal und das Tierlist-/Builds-Portal. Jedes Subportal löst einen anderen Teil der Community-Erfahrung ab, vom Einstieg über Statistiken bis zu Meta- und Patch-Inhalten.

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

Nach Login kommen persönliche Ansichten dazu:

- eigene Stats
- Voice- und Text-Historie
- Heatmap
- Co-Player-Ansicht

Die Oberfläche arbeitet stark mit Diagrammen und aggregierten API-Endpunkten. Für Nutzer heißt das: Man bekommt keine Rohdatenbank, sondern bereits aufbereitete Übersichten, die auf Vergleich, Verlauf und Aktivitätsmuster ausgerichtet sind.

## Patch Notes
Das Patch-Portal ist ein lesbarer Patch-Archiv-Bereich. Die Seite lädt Patchnotes aus einer öffentlichen API, blendet nur relevante Einträge ab 2025 ein und stellt sie als aufklappbare Karten dar.

Wichtige Funktionen:

- Volltextsuche über Patch-Inhalte
- Kategorien-Filter
- Datumsanzeige pro Patch
- aufklappbare Detailansicht
- Link zurück zur Originalquelle

Der Nutzer bekommt damit keine rohe Forenansicht, sondern eine aufbereitete, deutsch lesbare Archiv-Seite. Bei Ladeproblemen zeigt das Portal explizit eine Fehlermeldung statt still leer zu bleiben.

## Tierlist / Builds
Das Tierlist-Portal ist der Meta-Bereich für Hero-Rankings und Build-Empfehlungen. Im ausgelieferten Public-Frontend sind vor allem drei Dinge sichtbar:

- aktuelle Tierliste
- Build-Voting
- Tierlist-Historie

Die Public-Tierlist unterstützt unterschiedliche Buckets wie `all`, `phantom_plus` und `eternus`. Nutzer können zwischen Grid- und Listenansicht wechseln, nach Heroes suchen und einzelne Heroes aufklappen. Im Detailpanel sieht man Build-Beschreibungen, Kernitems, Ability-Order und kann positiv oder negativ voten.

Zusätzlich gibt es eine History-Seite, auf der Snapshots verschiedener Patches oder Abrufe gegeneinander gestellt werden. So ist schnell erkennbar, welcher Hero gestiegen oder gefallen ist. Falls das Live-Backend ausfällt, besitzt das Portal einen Static-Fallback mit zuletzt gespeicherten JSON-Daten. Das ist für Nutzer wichtig, weil die Seite dann nicht komplett ausfällt, sondern weiterhin einen brauchbaren Stand zeigt.

## Ergänzende Meta-App
Neben den vier Kernportalen existiert im Repo noch eine separate React/FastAPI-Meta-App. Dort sind zusätzliche Seiten für Heroes, Builds, Tierlists, Patchnotes, History, Feedback und Coaching angelegt. Aus Nutzersicht ist das eher ein erweiterter Produktzweig als der primäre Community-Einstieg.

## Was passiert technisch?
Die Website kombiniert mehrere Vite-Frontends mit klaren Basispfaden. Einige Portale sind rein statisch, andere sprechen Live-APIs an oder brauchen einen Discord-Login. Das `builds`-Backend stellt dafür Router für Auth, Heroes, Builds, Tierlists, Patchnotes, History, Admin und Coaching bereit. Für Nutzer zeigt sich das vor allem in zwei Dingen: schnelle, getrennt deploybare Teilseiten und unterschiedliche Funktionslevel je nach Login oder Live-API-Verfügbarkeit.
