# Abschluss mit autorisierter Abschaltung

status: erledigt mit Rückfalllösung
Datum: 2026-10-05

Der Nutzer hat ausdrücklich erlaubt, die Autoverknüpfung abzuschalten, wenn die wiederholten Kosten nicht sauber vermieden werden. Dieser Rückfall ist produktiv umgesetzt und geprüft. Die automatische Suche ist deaktiviert, vorhandene manuelle Eingaben bleiben erhalten. Normale Betriebskonfiguration enthält matcher_enabled=false und den belegten dauerhaften Zustandspfad. Der Bot ist auf dem bisherigen geprüften Release dd58c0af neu gestartet; Config-Fingerprint, aktiver Dienst und Gateway-Bereitschaft sind live belegt. Ein echter manueller Verknüpfungsvorgang wurde nicht ausgelöst.

Nachweise: LIVE-SOFORTMASSNAHME.md und LIVE-CONFIG-AENDERUNG.json. Keine Zugangsdaten ausgegeben, keine externe Testnachricht gesendet.

Der Einmal-Fix ist nicht ausgeliefert. Code auf a7418145dd07af2654a30fd21c3d60950f88d832 ist sauber gesichert; 76 passende Tests, Bot-Check und gezieltes Format-/Diffprüfen sind grün. Das Gate blockiert die Überschreibung eines älteren offenen manuellen Auftrags bei wiederverwendetem Login. Vier BLOCK-Runden und damit Deckel erreicht. Kein weiterer Review-Aufruf, kein State-Reset, kein Basiswechsel zur Umgehung, kein Merge/Push/Deploy. Unfertigen Branch und Worktree bewusst erhalten; HANDOFF.md enthält die konkrete Fortsetzung. Eine Fortsetzung braucht regulär wieder zulässige Gate-Prüfung, keinen ungeprüften Deploy.

Mit „Jav“ meinte der Nutzer Jev von TypeSafe. Die Hauptsession hat offizielle Anbieterquellen geprüft: Entscheidungsmodell mit 0,042 US-Dollar je Million Eingabetokens, kostenlose Ausgabe. Es wurde kein neues Modell eingebaut. Für die abgeschaltete automatische Suche entstehen keine weiteren Matcher-KI-Aufrufe.
