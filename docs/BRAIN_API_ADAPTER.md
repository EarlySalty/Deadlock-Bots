# Typisierter Brain-Consumer

`runtime.ai.brain_client_mode` wählt `legacy`, `typed` oder `shadow`. Der Standard ist `legacy`. `typed` verwendet den öffentlichen Brain-Client; `shadow` liefert die bestehende Antwort aus und prüft den typisierten Weg getrennt mit begrenzter Laufzeit. Der vorhandene Brain-Hilfepfad bleibt erhalten.

Die normale Betriebsdatei enthält `brain_api_endpoint`, `brain_api_scopes` und `brain_api_timeout_ms`. Der Endpunkt muss lokal sein, der Scope ist genau `bot.public`, die Frist liegt zwischen 1 und 60000 Millisekunden. Das Bearer kommt aus dem vorhandenen Infisical-Eintrag `TWITCH_INTERNAL_API_TOKEN`. Die bisherige serverseitige Identität `twitch-bot/twitch` und ihre Rechte bleiben erhalten. Es gibt keine Token- oder Scope-Substitution.

Die normale Konfiguration und diese vier Felder sind gegen Änderungen über Admin-Settings geschützt. Queries enthalten weder Principal noch Releasewahl. Jede Adapterinstanz erzeugt einen zufälligen Namensraum und jede Anfrage ein eigenes Gespräch. Aufnahme und Transport haben gemeinsame Fristen und höchstens vier gleichzeitige Anfragen.

`typed` zusammen mit `brain_open_test_mode=true` blockiert den Start ausdrücklich. Der vorhandene Test- und Reviewpfad bleibt damit wirksam. Für `Answered` und `BuildRejected` gelten dieselben URL- und Längengrenzen. Nicht verfügbare oder unzulässige Antworten werden als Fehler behandelt; fehlende Belege ergeben `NoAnswer`.

Der Adapter protokolliert vor dem Transport das Ereignis `brain_consumer_request` mit Route `/v1/answer` und `request_id=client-sha256:<SHA256 der vollständigen Request-ID>`. Das spätere `authenticated_request`-Ereignis von `brain-serve` muss denselben Hash und die bestehende Consumerkennung `twitch-bot` tragen. Der Client-Eintrag allein belegt weder eine erfolgreiche Antwort noch einen Anbieteraufruf. Frage, rohe Request-ID, Gesprächskennung und Credential werden dabei nicht protokolliert.

`bash rust/scripts/check-brain-consumer.sh` prüft den Adapter, die Verdrahtung, Shadow-Abbruch und geschützte Konfiguration offline. Das Script erwirbt zuerst `host-checks.lock`, danach `/tmp/deadlock-cargo-release.lock`, jeweils blockierend ohne Wartefrist. Vor jedem Compilerstart erfolgt die Hostprobe. Es prüft auch die bestehenden Launcher-Tests und vollständiges betroffenes Clippy mit `-D warnings`; Belege liegen SHA-gebunden unter `.consumer-ci-reports/`. Die gemeinsame C9-Abnahme umfasst den heutigen öffentlichen und internen Producervertrag sowie die getrennten Docs- und 2nd-Brain-Releases.
