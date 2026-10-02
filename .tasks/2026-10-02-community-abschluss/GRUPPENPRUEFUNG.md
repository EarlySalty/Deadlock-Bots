# Discord-Community, Prüfung von PR #472

Basis ist Bots-Main `1e6cdf64`. Der Community-Anteil aus `c8ab0799` und der belegte Replay-Fix `e55236c6` wurden selektiv übertragen. Der schmutzige Herkunftsworktree bleibt erhalten. Token-, Paten-, Workflow- und C9-Anteile wurden daraus nicht übernommen.

## Abnahme und Fixrunde

Das historische Gate auf `e55236c6` und das erste frische Gate auf `e2b9656d` melden BLOCK. Die Logs stehen in der zugeordneten Statusakte. Die gemeinsame Fixrunde umfasst:

- Streamersync, Ledger, Stimmen und Ergebnisbildung verwenden die gemeinsamen Privacy-Sperren und beachten Widerspruch beziehungsweise Löschgrabstein.
- Der Clip-Ledger-Import übernimmt nur Discord-Nutzer, deren Privacy-Sperre die laufende Transaktion hält. Während des Wartens neu hinzugekommene Nutzer werden im nächsten Import geprüft. Eine Regression wartet nachweislich auf die Datenbanksperre und prüft beide Importläufe.
- Zusätzliche Producer-Schlüssel eines Wochen-Duplikats werden in einer eigenen additiven Migration dauerhaft und transaktional gespeichert. Gleicher Schlüssel und anderer Clip bleibt ein Konflikt. Metadaten des ursprünglichen Clips bleiben erhalten.
- URL-Host und Schema werden normalisiert. Groß-/Kleinschreibung von Pfad, Query und Twitch-Clip-ID bleibt erhalten.
- Ein fehlgeschlagener Twitch-ID-Lookup verhindert jede Stimme. Die verbindliche Schreibentscheidung prüft die Datenbankzeit nach dem Warten und noch einmal in der INSERT-/UPDATE-Bedingung; die Voting-Sperre serialisiert sie mit der Finalisierung.
- Hilfe und Dokumentation erklären, dass nur Twitch-Punkte eine Twitch-Verknüpfung voraussetzen. Voice- und Ledgerpunkte können auch unverknüpfte Discord-Mitglieder im gemeinsamen Leaderboard zeigen.
- Der neue Punkte-Sync verwendet den vorhandenen privaten FD3-Snapshot. Sein neuer ENV-Wrapper entfällt. Der Timer wird erst nach Gruppenabnahme und ALLOW mit dem tatsächlichen Releasepfad installiert.
- Parallele Twitch-Kontozuordnungen werden serialisiert. Eine Regression prüft zwei gleichzeitig laufende Callbacks und genau einen Besitzer.

Die gezielte Suite muss diese Änderungen gegen eine eigene Wegwerf-Datenbank prüfen. Ein Quellbefund ist kein Testergebnis. Das erneute Gate läuft erst auf dem vollständigen getesteten Fixstand.

## Gemeinsamer Vertrag und Folge

Twitch ist Integrationsverantwortlicher der Gruppe `community-1035-472`. Bots-C9 wird zuerst integriert; danach muss der Community-Branch nachziehen und erneut durch das Gruppen-Gate. Der Bots-Consumer muss vor dem Twitch-Producer ausgeliefert werden. Bot und Webdienst `deadlock-web-rust.service` gehören zum gemeinsamen Bots-Release.

Der Broker liefert `ok/result`. Gleicher Clip mit geändertem Titel, Login oder Einreicher endet mit `duplicate` und `idempotency_metadata_drift`; Twitch speichert dieses Ergebnis und ruft den Broker für denselben Versuch nicht erneut auf. Authentische Twitch-ID, Helix-Clipzuordnung, aktiver Partnerstatus und Berechtigung des Einreichers werden im authentifizierten Producerpfad geprüft. Ein Login allein ist kein Identitätsnachweis.

Streamerpunkte verwenden ausschließlich verknüpfte Zuschauer ohne Privacy-Grabstein aus `twitch_viewer_daily`; das unbeschränkte `viewer_minutes` dient dieser Berechnung nicht. Der vorhandene Roundtrip erwartet bei 1000 Quellminuten genau 120 verknüpfte Minuten und vier Punkte. Exklusive RFC3339-Cursor müssen auch bei Zeitgleichheit ohne Zeilenverlust paginieren. Die zugehörigen Producer-Nachweise für Viewer, Streamer und Scout-Ergebnisse bleiben Bestandteil der gemeinsamen Abnahme.

Der Twitch-Befund zum schreibenden Dashboard-Pool und zur Rolle `twitchdash` gehört ebenfalls zum Gruppen-Gate. Bestehende Migrationschecksums bleiben erhalten; die Korrektur erfolgt bei bereits angewandtem Schema separat.

## Offene Produktkonfiguration

PLAN.md nennt Stufenrollen; PAKET-C.md bezeichnet Stufenrollen und monatlichen Streamer-Spotlight ausdrücklich als nicht gebaut. Belegte Rollen-IDs, Schwellen und ein freigegebener Spotlight-Zielkanal fehlen. Eine konkrete ursprüngliche Nutzerstelle dazu ist bislang nicht nachgewiesen. Diese Funktionen sind kein vorhandenes Merkmal und gehören nicht zum Live-Nachweis der Kernintegration. Der Hauptthread klärt tatsächlich erforderliche Produktwerte. Es werden keine Werte erfunden oder öffentliche Ankündigungen daraus abgeleitet.
