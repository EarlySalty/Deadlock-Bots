# Wissensrouter: dauerhafte Secret-Anbindung

Der zentrale Loader in `dl-ai` liest normale Konfiguration aus
`/etc/deadlock-bots/knowledge-router.json`. Vorlage:
`service/config/knowledge-router.example.json`. Die Datei enthält keine Secrets.

Das bestehende systemd-`LoadCredential=infisical-token` bleibt die Bootstrapquelle.
`deadlock-bot-rust.service.d/95-knowledge-router.conf` öffnet dieses Credential über
systemds `%d` als exklusiv reservierten FD9. Der zentrale Loader übernimmt und
schließt FD9 am Prozessanfang, bevor Subprozesse gestartet werden. Er holt den
bestehenden OpenRouter-Key über die bestehende geschützte Infisical-Unixbridge
direkt in den Prozessspeicher. Kein neuer Secret-ENV-Pfad und keine Keydatei.

Der Integrator installiert Konfiguration und Drop-in gemeinsam mit der neuen
Bot-Binary, führt `systemctl --user daemon-reload` und den normalen Neustart aus.
Nach dem Start muss genau die Routeraktivierung im Journal nachweisbar sein;
Fallbackmeldungen sind kein Aktivierungsnachweis. Zusätzlich einen echten
Concierge-/AnswerEngine-Test auf Standardantwort und freien Fallback ausführen.
Ein zweiter regulärer Neustart muss dieselbe Aktivierung ergeben: ein manuell
geerbter Test-FD reicht nicht als Betriebsnachweis.

Fehlende/ungültige Konfiguration, Credentials oder Infisicalzugriff werden ohne
Antwortkörper oder Schlüsselwerte gemeldet. Runtime-Antwortfehler bleiben beim
Router und führen zum bestehenden Antwortpfad.

Der übrige bestehende Bot-Launcher enthält Legacy-ENV-Pfade. Deren vollständiger
Umbau ist nicht Teil dieses Jev-Anschlusses und wird hier nicht als erledigt behauptet.
