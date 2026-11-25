status: aktiv (2026-10-02), Entwurf vor Bauabschluss

# Nutzertest Spielerpool

Noch nicht als Fertigmeldung verwenden. URLs, sichtbare Bedienelemente und
technische Nachweise nach dem Deploy der Pakete ergänzen. Gesamtauftrag ist
noch nicht live abgenommen.

## Vorbereitung durch die Worker

Vor dem Nutzertest dokumentieren die Worker den gültigen Testzugang, die
deployten main-SHAs, den Ingest-Stand und die tatsächlich geprüften URLs.
Keine Geheimnisse oder Tokens in diese Datei aufnehmen.
Die Freigabe für alle Mitglieder bleibt beim Nutzer.

## Schritte mit den beiden Konten

1. Mit dem freigegebenen Discord-Konto auf dem Server `/spielerprofil` aufrufen. Auswahlfragen beantworten. Erneut aufrufen und prüfen, dass die Angaben erhalten bleiben. Mit dem Zweitkonto wiederholen, sobald dessen Testzugang bestätigt ist.
2. Auf `https://deutsche-deadlock-community.de/spielerpool` mit demselben Discord-Konto anmelden und Steam über den dortigen Knopf verknüpfen. Für das Zweitkonto in einer getrennten Browsersitzung wiederholen. Dafür ist keine Steam-Bot-Freundschaft nötig.
3. Nach dem bestätigten API-Ingest beide Profile im Pool prüfen: Spielmuster, Rang, Spiele und Stunden müssen mit dem belegten Datenstand übereinstimmen. Zeit-, Modus- und Rangfilter benutzen. Gemeinsame Mitspieler dürfen nur erscheinen, wenn sie selbst im Pool sind.
4. Beim anderen Profil „Zusammen spielen“ drücken. Prüfen, dass ein Kanal auf dem Server entsteht und beide dort gepingt werden. DM-Opt-in zunächst ausgeschaltet lassen und prüfen, dass keine Einladungs-DM kommt.
5. Eine Testsession beitreten und gemeinsam nutzen. Anschließend die Feedback-Checkboxen auf der Website speichern und erneut öffnen. Dafür darf keine Feedback-Benachrichtigung eintreffen.
6. Eine weitere Session erstellen, der niemand beitritt. Nach einer Stunde prüfen, dass der Kanal verschwunden ist. Der Worker dokumentiert dazu die gespeicherten Zeitpunkte und den Löschlauf.
7. Beim Testprofil DM-Opt-in einschalten und den Einladungsweg gezielt mit den eigenen Konten prüfen. Danach bei Bedarf wieder ausschalten.
8. Das Zweitkonto-Profil auf der Website löschen. Prüfen, dass es im Pool und unter gemeinsamen Mitspielern verschwindet. Die Worker bestätigen zusätzlich, dass seine Pool-Daten entfernt sind und der laufende Ingest sie nicht wieder anlegt.

## Technische Belege, noch offen

- A: Migration, Datenschicht, Löschpfad, main-SHA, Live-Nachweis.
- B: Interview und gespeicherte Antworten je Discord-ID, main-SHA, Live-Nachweis.
- C: echte API-Quelldaten und geschriebene Aggregate einschließlich Mitspieler-Paaren, main-SHA.
- E: Session-Persistenz, Kanal, Ping, Neustart und Löschung ohne Beitritt, main-SHA.
- D: Steam-Mapping ohne Bot-Freundschaft, Guild-Zugriffsschutz, Filter, Feedback und Löschen, Website- und Caddy-SHAs.
- Aufräumen: Threads gesettelt, Branches und Worktrees nach Merge und Live-Prüfung gelöscht.
