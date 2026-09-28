# Fortsetzung: technisch korrigiert, lokales Merge-Gate gesperrt

Stand 29.09.2026: Feature-Branch `fix/moderation-negierte-scam-hinweise` ist gepusht. Geprüfter Code liegt in HEAD `2c7f52ac6b903b63021d5c2714dc759f7532399a`. Die unabhängige R4 hat die vollständige verbleibende Mängelliste samt Kontrollen freigegeben. Der Fixer berichtet 91 bestandene Tests sowie grüne Format-/Clippy-Prüfung; der Reviewer hat zehn gezielte Tests und Rust-Gegenproben selbst ausgeführt. Berichte liegen in diesem Ordner.

## Verbindlicher Blocker

Der lokale Merge-Gate hat nach fünf aufeinanderfolgenden BLOCK-Runden den gespeicherten Status `escalated` gesetzt. `gate_hook.py:5535-5543` lehnt damit weitere Versuche ab und verweist die Entscheidung an den Nutzer. Die inhaltliche R4-Freigabe hebt diese Sperre nicht auf. Kein Reset von Gate-Dateien/Zählern, keine neue Branch-Identität, keine Änderung des Vergleichsstands oder sonstige Umgehung. Der Nutzer wurde über die Sperre informiert. Es wurde weder gemergt noch deployt.

## Nach zulässiger Auflösung der Gate-Eskalation

1. Nutzerentscheidung und den regulären Gate-Weg prüfen. Aktuellen origin/main-Stand holen und den eigenen unveränderten Feature-Anteil vergleichen. Fremde Arbeit nicht übernehmen oder verändern. Erneute Prüfungen nur auf zulässigem Weg; bestehende Hooks bleiben wirksam.
2. Erst bei gültigem lokalen Gate-ALLOW nach main mergen und pushen. Keine Abhängigkeit von GitHub Actions.
3. Release aus eigenem Worktree passend zum dann aktuellen origin/main bauen. Zuvor Herkunft des laufenden Binaries prüfen. Bestehenden dl-bot-Deploy-Weg nutzen; Dienst `deadlock-bot-rust.service` läuft als User-Unit. Keine Secrets ausgeben und keinen laufenden fremden Build anfassen.
4. Restart mit PID-/Binary-/Journal-Beleg. Funktion über deterministische lokale Prüffälle bzw. kontrollierte harmlose Prüfung belegen, keine Timeout-/Ban-Probe gegen ein echtes Mitglied und keine Community-Bilder für Tests an externe Modelle schicken.
5. Die alte Warnkarte gehört zur Originalnachricht `1554224525426565242` in Kanal `1289721245281292291`, Guild `1289721245281292288`. Originalnachricht wurde read-only geprüft und besteht weiter. Vorhandenen Fallstatus prüfen; falls noch offen, über den vorhandenen Verwerfen-/deny_case-Dienstweg schließen und die vorhandene Karte in place aktualisieren. Nicht neu senden und nicht direkt in der Datenbank ändern.
6. Erst danach gemergten Branch und Worktree sicher entfernen und verbliebene abgeschlossene Worker-Threads settlen.

Die global ergänzte Ablaufregel verlangt direkte Umsetzung konkreter Fix-Aufträge statt bloßer Planabgabe. Sie hebt keine dieser Schutzgrenzen auf.
