# Brain Testkommando

Stand: 22.09.2026

`/brain frage:<Text>` ist im offenen Testmodus in allen Serverkanälen verfügbar. DMs bleiben deaktiviert. Mit `BRAIN_OPEN_TEST_MODE=true` darf die Antwort auch außerhalb von Deadlock liegen; Deadlock Spielwissen aus `deadlock-brain ask-context` wird weiterhin als zusätzlicher Kontext genutzt.

Der Testpfad besitzt keine Aktionsschnittstelle. Er kann weder Discord Aktionen auslösen noch Dateien, Dienste oder Daten ändern. Der Brain Unterprozess erzwingt für `ask-context` den Postgres Lesemodus beim Verbindungsaufbau, und die Antwortschicht verwirft credentialartige Ausgaben.

`BRAIN_CHANNEL_ALLOWLIST` gilt weiterhin für den normalen, geschlossenen Brain Modus. Im offenen Testmodus wird diese Begrenzung nicht verwendet. `BRAIN_OPEN_TEST_MODE` ist standardmäßig deaktiviert.
