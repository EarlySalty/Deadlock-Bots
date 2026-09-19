# Register: Streamer-Voice-Beitritt

Auftrag aus der ChatGPT-Session mit Twitch-Chat-Screenshot. Keine separate T3-Thread-ID vorhanden.

| Teil | Arbeitsbaum | Branch | Stand |
| --- | --- | --- | --- |
| Twitch: Erkennung, Antwort, Identität, Broker-Client | `/home/nathanael/.worktrees/tb-streamer-voice-join` | `feat/streamer-voice-join` | Implementiert, Regressionen und Gesamtprüfung laufen |
| Discord: authentifizierter Broker, Kanalprüfung, Invite, Platz | `/home/nathanael/.worktrees/db-streamer-voice-join` | `feat/streamer-voice-join` | Implementiert, 83 Bibliothekstests bestanden |
| Unabhängiges Review | vorhandenes `gate_hook.py --review` | beide Repos | Ausstehend |
| Veröffentlichung | zuerst Discord, dann Twitch | aktueller `origin/main` | Noch nicht erfolgt |

Fremde Änderungen in den Haupt-Checkouts bleiben unangetastet. Kein produktiver Kanal wurde für Tests verändert.

## Grenzen

Der Invite öffnet Discord beim richtigen Sprachkanal, ersetzt aber weder die Anmeldung noch das Bestätigen des Beitritts. Bestehende Zugriffsregeln bleiben bestehen. Rollenbeschränkte und gesperrte Räume werden nicht öffentlich eingeladen. Der eigene Streamer-VC ist live rollenbeschränkt und wird deshalb nicht fälschlich als für neue Mitglieder öffentlich ausgegeben.

Die Einladung ist zehn Minuten gültig. Eine nötige Kapazitätserhöhung bleibt bestehen, bis der Kanal regulär geändert oder gelöscht wird. Es gibt keine automatische Rücksetzung und keine Entfernung von Mitgliedern.
