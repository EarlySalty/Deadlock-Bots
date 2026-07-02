# Patchnotes-Bot

## Worum geht es?
Der Patchnotes-Bot beobachtet die offiziellen Deadlock-Changelog-Quellen und postet neue Einträge automatisch in den dafür konfigurierten Discord-Bereich. Sein Job ist nicht, das Spiel selbst zu erklären, sondern neue Patches schnell in einem lesbaren Format in die Community zu bringen.

## Was postet der Bot?
Der Bot postet neue Patchnotes als Discord-Nachrichten. Dabei geht es um frisch erkannte Changelog-Einträge, nicht um eine manuell gepflegte Liste. Pro Patch speichert er Titel, Quelle, Datum und den eigentlichen Inhalt serverseitig, damit nichts doppelt ausgespielt wird und spätere Re-Posts möglich bleiben.

Im normalen Betrieb umfasst ein Post:

- eine klare Überschrift mit Patch-Datum
- den aufbereiteten Patchtext in Discord-geeigneten Blöcken
- optional einen Rollen-Ping

Außerdem kann der Bot vorbereitete Sonderposts einmalig aus einer hinterlegten Datei abschicken. Das ist für kuratierte Ankündigungen oder Tests gedacht und kann je nach Konfiguration in den normalen Patchnotes-Channel oder in einen separaten Logs-/Staff-Channel gehen.

## Welche Quelle nutzt er?
Der Bot beobachtet zwei Quellen gleichberechtigt: das offizielle Deadlock-Changelog-Forum und die Steam-News. Er nimmt jeweils den neuesten Eintrag (bei Gleichstand gewinnt das Forum). Zusätzlich reagiert er auf Steam-Signale fast in Echtzeit: Ein erkanntes Steam-Update löst einen mehrminütigen Intensiv-Scan aus, damit der Patch so schnell wie möglich im Channel landet. Wenn ein Forumspost im Kern nur auf die Steam-News verweist, zieht der Bot den eigentlichen Steam-Inhalt nach, damit nicht bloß eine kurze Link-Vorschau gepostet wird.

Wichtig für Nutzer: Der Bot versucht immer die echte Patchquelle zu erwischen, nicht nur irgendeinen Kommentar oder Link-Fetzen. Gleichzeitig merkt er sich bereits verarbeitete Posts, damit identische Inhalte nicht mehrfach im Channel landen.

## In welchen Channel postet er?
Im Regelbetrieb postet der Bot in den konfigurierten Patchnotes-Channel des Discord-Servers. Für vorbereitete Einmal-Posts gibt es optional einen separaten Logs-/Kontroll-Channel. Die konkrete Channel-Zuordnung ist absichtlich Konfiguration und nicht fest im Nutzertext verdrahtet.

## Wie oft prüft der Bot?
Der Bot läuft nicht per klassischem Tages-Cron, sondern als permanenter Hintergrundprozess. Nach dem Start macht er sofort einen ersten Scan und prüft danach standardmäßig alle 35 Sekunden auf neue Einträge. Zusätzlich kann er beim Hochfahren einmalig den neuesten Patch nachholen, falls beim letzten Lauf etwas verpasst wurde.

Damit ist der Bot auf Aktualität getrimmt:

- schneller Erstscan beim Start
- kurze Polling-Intervalle im Dauerbetrieb
- begrenztes Catch-up für verpasste Posts

Sehr alte Einträge werden nicht automatisch erneut gepostet. Standardmäßig greift ein Altersfilter von zwei Tagen für Auto-Posts.

## Nutzt der Bot KI?
Ja, der Bot kann eine KI-gestützte Aufbereitung verwenden. Für Nutzer ist der Effekt vor allem:

- englische oder unhandliche Originaltexte werden in lesbares Deutsch gebracht
- die Ausgabe wird in Discord-taugliche Abschnitte gegliedert
- Helden-, Item- und General-Bereiche bleiben besser lesbar

Die Doku hier erklärt bewusst nicht die interne Prompt- oder API-Mechanik, sondern nur den sichtbaren Nutzen.

Falls eine Übersetzung mal danebenliegt, kann das Team einen Patch direkt im Channel neu übersetzen lassen: `!tpatch` (ohne Ping) bzw. `!ppatch` (mit Ping), mit kurzem Cooldown pro Channel.

## Häufige Grenzen
- Der Bot erklärt keine Patchinhalte aus sich heraus, sondern verarbeitet nur erkannte Changelog-Quellen.
- Bereits bekannte oder identische Inhalte werden blockiert, damit der Channel nicht zugespammt wird.
- Bei Quellfehlern oder API-Problemen kann ein Post ausbleiben, obwohl der Bot weiterläuft.
- Diese Seite dient zum Verständnis des Bots selbst, nicht als Patch-Archiv — fürs Nachschlagen alter Patches gibt es das Patch-Portal auf der Website.

Kurz technisch: Der Bot scannt Forum und Steam, speichert gefundene Patches in einer Datenbank, schützt sich gegen Doppelposts (Inhalts-Signatur, URL- und Patch-ID-Abgleich) und schickt den finalen Text in Discord in mehreren Chunks, wenn eine Nachricht zu lang wäre. Nach dem Speichern stößt er zusätzlich den Wissens-Sync fürs Deadlock-Brain an.
