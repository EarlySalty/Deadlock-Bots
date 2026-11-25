status: aktiv (2026-10-02)

# Briefing: Spielerpool D

[Orchestrator] Paket D im Projekt Website: Spielerpool mit Steam-Verknüpfung.
Auftrag: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/AUFTRAG.md`.
WORKER-REGELN.md, PAKETE.md, VERTRAG-A.md und VERTRAG-E.md im selben Ordner vollständig lesen.

- Worktree: `/home/nathanael/.worktrees/Website-pool-d`
- Branch: `feat/spielerpool-d`
- Intent-Thread: `011b713c-266a-46b6-b0ce-1d5faa060234`
- Rolle: `worker_gross`
- Startvoraussetzung: A gemergt und technischer E-Vertrag festgelegt.

Vertrag A konkret: `own_profile` nur mit der eigenen authentifizierten Discord-ID verwenden,
fremde Profile ausschließlich über die öffentliche Projektion lesen. DM-Opt-in
und bevorzugte Spieler sind privat, Feedback hat keinen öffentlichen Leser.
Alle großen IDs als Dezimalstrings serialisieren. Steam-Link erst nach geprüfter
OpenID-Antwort schreiben. Filter-Zeitfenster sind lokale Profilzeiten, die
API-Heatmap hat UTC-Buckets. Zeitbasis sichtbar und korrekt behandeln;
VERTRAG-C.md beachten, sobald C seinen belegten API-Umfang meldet.
Für Feedback die dauerhafte Liste eigener Teilnehmer-Sessions aus dem finalen
A-Vertrag nutzen, keine ausschließlich im Browser gespeicherte Sessionliste.
Die Methode ist `own_sessions(Scope, limit, offset)`: beendete eigene Sessions
mit eigenem Beitritt und eigenem optionalem Feedback, paginiert und nach Endzeit
sortiert. Den aktualisierten Vertrag für den konkreten Rückgabetyp lesen.

Du besitzt die nötigen Rust-Backend-Routen in `Website/builds/backend-rust`,
die Seiten in `Website/builds/frontend` und die Route im Repo caddy-config.
Für caddy-config einen getrennten eigenen Worktree und Branch verwenden;
Repo-Regeln und Gate auch dort einhalten. Kein Bot-Code ändern.

Die bestehende Discord-Session und den OAuth-Weg erweitern. Nur eingeloggte
Mitglieder der Guild 1289721245281292288 erhalten Poolzugang. Mitgliedschaft
serverseitig prüfen, vorhandene Scopes ergänzen, keine zweite Login- oder
Tokenablage. Steam-OpenID an diese Session binden und in core.steam_links
speichern. Bestehende Twitch-Implementierung ausschließlich lesen.
OpenID-Antwort, Rückkehradresse und Einmalzustand prüfen; keine Steam-Bot-Freundschaft.

Unter `/spielerpool` Pool und Profile mit Spielmuster, Rang, Spielen, Stunden,
Heatmap und den zuletzt gemeinsamen Mitspielern darstellen. Nur zulässige
Poolmitglieder anzeigen. Filter für Zeit, Modus und Rang. Fehlende API-Daten
ehrlich zeigen. Vorlieben und DM-Opt-in bearbeiten können, Default aus.
Der Knopf „Zusammen spielen“ nutzt intern den E-Vertrag, bindet den Initiator
an die authentifizierte Discord-ID und zeigt den erstellten Discord-Kanal.
Feedback nach der Session nur hier über Checkboxen speichern; keine Nachricht
verschicken. Teilnahme für Feedback serverseitig prüfen.
Profil-Löschen entfernt alle Pool-Daten entsprechend dem A-Vertrag.

Bestehenden Stil der Website aufnehmen, echte deutsche Umlaute und klare
Bedienung. Frontend im Browser selbst durchklicken und Screenshots prüfen.
Backend-Zugriffsschutz, fremde Profiländerungen und Teilnehmergrenzen angemessen
testen. Website-Deploy nach Repo-Weg, nötige Services neu starten, Caddy-Route
live prüfen. Nutzerkonten und Steam-Login nicht vortäuschen. Am Ende geprüfte
URLs und noch benötigte Nutzeraktionen für die gemeinsame Testanleitung melden.
