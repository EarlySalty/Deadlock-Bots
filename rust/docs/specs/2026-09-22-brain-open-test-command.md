# Brain Testkommando

Stand: 22.09.2026

`/brain frage:<Text>` ist im konfigurierten Testkanal verfügbar. Mit `BRAIN_OPEN_TEST_MODE=true` darf die Antwort auch außerhalb von Deadlock liegen; Deadlock Spielwissen aus `deadlock-brain ask-context` wird weiterhin als zusätzlicher Kontext genutzt.

Der Testpfad besitzt keine Aktionsschnittstelle. Er kann weder Discord Aktionen auslösen noch Dateien, Dienste oder Daten ändern. Der Brain Unterprozess erzwingt für `ask-context` den Postgres Lesemodus beim Verbindungsaufbau, und die Antwortschicht verwirft credentialartige Ausgaben.

Die Kanalbegrenzung bleibt über `BRAIN_CHANNEL_ALLOWLIST` aktiv. Der offene Modus ist getrennt über `BRAIN_OPEN_TEST_MODE` schaltbar und ist standardmäßig deaktiviert.
