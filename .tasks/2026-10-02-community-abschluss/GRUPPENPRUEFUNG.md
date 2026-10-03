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

## Erste gezielte Suite und Nachprüfung

Die vollständige erste Suite lief auf `5f449720f99ef10b666b644bd95eb2c91a2d9285` mit Cargo 1.98.1 und höchstens zwei Jobs. 100 Tests bestanden, ein Test schlug fehl, keiner wurde ignoriert. Der abschließende Compile-Check für `dl-bot`, `dl-web` und `dl-community-points-sync` bestand. Die eigene Timescale-Wegwerf-Datenbank wurde entfernt; beide Hostsperren wurden nachweislich freigegeben. Der Slot wurde ausdrücklich an Challenges für dessen bestätigten kurzen Debugcheck übergeben.

Die Regression `lookupfehler_schreibt_keine_stimme` deckte einen alten Fallback auf: Eine fehlende Verknüpfungstabelle lieferte eine leere Twitch-Zuordnung statt eines Fehlers. Nach dem vollständigen Suiteende wurde dieser Fallback entfernt. Die vollständige Nachsuite auf `81f67f3df016262475211dcc3ede3d13805009b0` bestand mit 15 Cliptests einschließlich der unveränderten Lookupfehlerregression und einem Schema-/Idempotenztest. Debug-Migratorbau und betroffene Clippy-Targets endeten mit Exit 0. Clippy warnt an 70 Stellen, davon stehen 68 Quellzeilen unverändert in der Basis und zwei neue Stellen betreffen Typkomplexität. Ein gesonderter Baseline-Clippy-Lauf fand nicht statt. Manifest und Logs stehen in der eigenen Statusakte.

Privacy-Grabsteine in Streamersync, Stimmen und Ergebnisbildung, die frische Schreibdeadline, der dauerhafte Wochen-Duplikatschlüssel und die erhaltene Groß-/Kleinschreibung von URL-Pfad und Query bestanden ihre gezielten Prüfungen. Der konkurrierende Clip-Ledger-Import und acht Runden paralleler Kontozuordnung bestanden ebenfalls. Logpfade, vollständige Ergebnisse und Ressourcenbelege stehen in der zugeordneten Statusdatei.

Die Sichtprüfung bleibt offen. Sieben HTML-Zustände wurden aus unveränderten Renderer-, SteamHint- und Escaping-Blöcken aus `web.rs` exportiert und mit Quell-, Block- und Ausgabehashes außerhalb des Worktrees gesichert. Der ausdrücklich gewählte In-App-Browser meldet weiterhin fehlende Verfügbarkeit. Ein HTML-Export allein ist kein Browser- oder Live-Nachweis.

## Zusätzlicher Viewer-Privacy-BLOCK

Eine unabhängige Abnahme auf `81f67f3` bestätigt eine weitere Lücke: Viewer-Tageswerte stehen nicht im Export-/Löschvertrag, die Twitch-Zuschauer-ID ist fälschlich als nicht personenbezogen klassifiziert, und der Viewer-Sync prüft keinen Privacy-Grabstein. Linklöschung allein verhindert weder gespeicherte Tagesdaten noch ihren Wiederimport. Die früher bestandene Suite belegt diesen neuen Fix nicht.

Die neue Fixrunde ergänzt Viewer-Tagesdaten in `USER_TABLES` und löscht sie über die bestehende Twitch-Verknüpfung vor deren Entfernung. Eine additive Migration korrigiert die Registry und speichert ausschließlich einen domaingetrennten SHA256-Twitch-ID-Hash als dauerhaften Importschutz. Dieser Hash ist weiterhin ein personenbezogenes Sperrmerkmal; er enthält keine Tagesdaten oder Discordzuordnung und wird nicht als Anonymisierung bezeichnet.

Der vorhandene globale Twitch-Reassignment-Lock ist mit C9 und Twitch abgegrenzt. OAuth-Upsert, Viewer-Sync und Privacy-Erasure verwenden denselben unveränderten Key vor den Nutzerlocks. Erasure setzt den Importschutz in derselben Transaktion. Der Sync prüft sowohl den Grabstein eines noch verknüpften Mitglieds als auch den dauerhaften Hash und darf übersprungene Quellzeilen mit dem Cursor abschließen.

Echte DB-Regressionen müssen den persönlichen Export, vollständige Löschung, Erhalt fremder Zeilen, Wiederimport auch nach neuer Discord-Zuordnung und beide Rennrichtungen belegen. Die Rennen warten auf beobachtbare Datenbanksperren. Ergebnisse, vollständiger neuer SHA und reguläres R2 werden in der Statusakte festgehalten. Vor positiver gemeinsamer Privacy-Abnahme wird nicht ausgeliefert.

Seit der ausdrücklichen Nutzerentscheidung vom 03.10.2026 gibt es keine Build-Warteschlange oder Nachrichtenübergabe mehr. Beide Hostlocks werden während des gesamten Prüfschritts blockierend gehalten; nach Erwerb folgt eine frische Compilerprobe, bei Belegung mit Wiederholung nach 30 Sekunden. Höchstens zwei Jobs und die fachlichen Integrationsabhängigkeiten bleiben verbindlich.

## Wiedereinwilligung und neue Scoutkopien

Der weitere BLOCK auf `7778b292` ist verbindlich: Ein dauerhafter Nullimport nach `/datenschutz-optin` und erneuter Twitchverknüpfung erfüllt den dokumentierten Nutzerpfad nicht. Der alte Wrapper wurde regulär mit Exit 143 beendet, während er ausschließlich an flock wartete. Auf diesem Kopf startete keine Prüfung und keine Datenbank.

Die neue additive Consentmigration speichert die bewusste Wiedereinwilligung nach OAuthverknüpfung getrennt vom minimalen Hash-Grabstein. OAuth, Optin, Import und Löschung erwerben denselben Identitätslock vor den Nutzerlocks. Kumulative Altzeilen bleiben gesperrt. Der Sync fragt authentifizierte Rohaktivität mit Twitch-ID, Berliner Tag und `activity_since` an. Die Antwort bindet diese Felder ausdrücklich; falsche Bindung oder Remoteausfall lässt den Tagescursor stehen. Der aktuelle Berliner Tag wird unabhängig vom Cursor neu berechnet. Geänderte historische Tage werden vor dem Seitencommit gelesen. Auch ein bestätigter leerer Tag ersetzt den alten Stand. Consent-CAS und `computed_at` verhindern verspätete alte Antworten. Der Streamer-Watchtimepfad liest ausschließlich diese geprüften Viewerwerte.

Der Scout-BLOCK umfasst die zusätzlichen Twitchkopien von Discord-ID und Vorschlagsgrund. Die lokalen Privacytransaktionen speichern geordnete Lösch-/Consentaufträge mit fester UUID und monotoner Epoche. Der vorhandene Bridgeclient und Vorschlags-Retryloop liefern sie in Reihenfolge. Jede Bestätigung wird separat committet. Verlorene Remoteantworten und zentrale Rollbacks behalten UUID und Epoche; `stale` bestätigt keine lokale Operation. Offene Aufträge sperren Vorschlagsweitergabe. Neue Vorschläge tragen ursprüngliches `created_at` als `submitted_at` sowie die gespeicherte Privacyepoche. Nutzerlock, Ursprungsprüfung und Remote-Epochenschutz serialisieren die Weitergabe mit Erasure. Das INSERT setzt `created_at` nach Erwerb des Nutzerlocks ausdrücklich auf `clock_timestamp()`, damit ein vor Optin begonnener und danach fortgesetzter Store keine alte Herkunftszeit bekommt. Die Regression beobachtet beide Lockreihenfolgen und vergleicht die gespeicherte Herkunft mit der tatsächlichen Consentgrenze.

Der öffentliche Datenschutzexport ergänzt die neuen Remote-Communitykopien oder meldet einen erkennbaren Ausfall. Lokale Löschung mit ausstehender Twitchbestätigung wird ausdrücklich als unvollständig gemeldet. Ein neuer allgemeiner Twitch-Löschpfad entsteht daraus nicht. Nach bestätigter Zustellung wird der lokale Auftrag entfernt; die minimale Hash-Epoche schützt verspätete alte Operationen. Die Tests unterscheiden ausdrücklich einen HTTP-Testproducer vom tatsächlichen Twitchcode. Die gemeinsame unabhängige Abnahme muss beide echten Writer prüfen.

Prüfvertrag ist Twitch `fd41c9185ab7cb50a1dd02c39090cad4059e95e5` auf `84a07dd48ee7e03d3f901fe9b500fa56526074c2`. Dessen `CONTRACT.md` hat SHA256 `ac1320878144a9b2f8132a42d618e85d440bafbfd006b22ae619accbcbb0f763`. Der Consumer-Prüfplan umfasst echte DB-Fälle für Export, Erasure, Fremdzeilen, Altwerteabwehr, Neuaktivität, beide Optin-/Link-/Import- und Import-/Erasure-Reihenfolgen sowie Scout-Schreiben gegen Erasure. HTTP-/DB-Fälle prüfen Nulltag, Antwortbindung, Cursor-Retry, verlorene erfolgreiche Remoteantwort, Bestätigungsrollback, identische Wiederholung und späte alte Erasure. Ergebnisse werden erst nach dem tatsächlichen Lauf eingetragen.

Die Migrationsnummern `20261003021000` und `20261003022000` wurden gegen die vorhandenen Bots-Peerworktrees und die aktuelle Remote-Basis geprüft. Twitch bestätigt keine Kollision mit seinen eigenen additiven Migrationen. Die letzte Remote-Botsprobe steht auf `1e6cdf648429e50014e548b0b4204c8ac48cb4ec`. Vor endgültigem Gruppen-Gate und Integration erfolgt ein erneuter Abgleich unter dem Repo-Lock. Der Community-Consumerkopf enthält noch keine freigegebenen neuen C9-/Guide-Hunks und wird nicht als kombinierte Bots-Endquelle ausgegeben.

## Gemeinsamer Vertrag und Folge

Twitch ist Integrationsverantwortlicher der Gruppe `community-1035-472`. Bots-C9 wird zuerst integriert; danach muss der Community-Branch nachziehen und erneut durch das Gruppen-Gate. Der Bots-Consumer muss vor dem Twitch-Producer ausgeliefert werden. Der Punkte-Timer wird erst nach beiden Live-Auslieferungen und erfolgreichem manuellem Sync aktiviert. Bot und Webdienst `deadlock-web-rust.service` gehören zum gemeinsamen Bots-Release.

Der Broker liefert `ok/result`. Gleicher Clip mit geändertem Titel, Login oder Einreicher endet mit `duplicate` und `idempotency_metadata_drift`; Twitch speichert dieses Ergebnis und ruft den Broker für denselben Versuch nicht erneut auf. Authentische Twitch-ID, Helix-Clipzuordnung, aktiver Partnerstatus und Berechtigung des Einreichers werden im authentifizierten Producerpfad geprüft. Ein Login allein ist kein Identitätsnachweis.

Streamerpunkte verwenden ausschließlich verknüpfte Zuschauer ohne Privacy-Grabstein aus `twitch_viewer_daily`; das unbeschränkte `viewer_minutes` dient dieser Berechnung nicht. Der vorhandene Roundtrip erwartet bei 1000 Quellminuten genau 120 verknüpfte Minuten und vier Punkte. Exklusive RFC3339-Cursor müssen auch bei Zeitgleichheit ohne Zeilenverlust paginieren. Die zugehörigen Producer-Nachweise für Viewer, Streamer und Scout-Ergebnisse bleiben Bestandteil der gemeinsamen Abnahme.

Der Twitch-Befund zum schreibenden Dashboard-Pool und zur Rolle `twitchdash` gehört ebenfalls zum Gruppen-Gate. Bestehende Migrationschecksums bleiben erhalten; die Korrektur erfolgt bei bereits angewandtem Schema separat.

## Offene Produktkonfiguration

PLAN.md nennt Stufenrollen; PAKET-C.md bezeichnet Stufenrollen und monatlichen Streamer-Spotlight ausdrücklich als nicht gebaut. Belegte Rollen-IDs, Schwellen und ein freigegebener Spotlight-Zielkanal fehlen. Eine konkrete ursprüngliche Nutzerstelle dazu ist bislang nicht nachgewiesen. Diese Funktionen sind kein vorhandenes Merkmal und gehören nicht zum Live-Nachweis der Kernintegration. Der Hauptthread klärt tatsächlich erforderliche Produktwerte. Es werden keine Werte erfunden oder öffentliche Ankündigungen daraus abgeleitet.
