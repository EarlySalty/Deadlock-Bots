# Brain Testkommando

Stand: 22.09.2026

`/brain frage:<Text>` ist im offenen Testmodus in allen Serverkanälen verfügbar. DMs bleiben deaktiviert. Mit `BRAIN_OPEN_TEST_MODE=true` darf die Antwort auch außerhalb von Deadlock liegen; Deadlock Spielwissen aus `deadlock-brain ask-context` wird weiterhin als zusätzlicher Kontext genutzt.

Normale Wissensfragen besitzen keine Aktionsschnittstelle. Eine konkrete Hero Build Frage ist die einzige freigegebene Schreibwirkung im Testmodus: Der Reasoner berechnet den Build ohne Reasoner Persistenz, legt einen eindeutig als `Brain Review` markierten Steam Publish Task an und gibt nach Möglichkeit die In Game Build ID zurück. Der Produktions Publish Gate für regulär freigegebene Meta Builds bleibt davon getrennt. Credentialartige Antworten werden weiterhin verworfen.

Brain Antworten verwenden den Deadlock Entity Emoji Katalog des Patchnotes Bots für bekannte Helden, Items und Fähigkeiten. `BRAIN_CHANNEL_ALLOWLIST` gilt weiterhin für den normalen, geschlossenen Brain Modus. Im offenen Testmodus wird diese Begrenzung nicht verwendet. `BRAIN_OPEN_TEST_MODE` ist standardmäßig deaktiviert.
