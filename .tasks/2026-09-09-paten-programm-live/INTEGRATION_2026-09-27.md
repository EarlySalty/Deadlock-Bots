# Paten-Integration auf aktuellem Bot-Main

Der Paten-Code ist auf den aktuellen Main-Stand portiert. Der gemessene
produktive Startpfad ist weiterhin das hostlokale
`scripts/run_dl_bot_service.sh`; es startet `dl-bot` mit
`--config ~/.config/deadlock-bots/bot.toml`. Das Startskript ist hostlokal und
nicht Teil dieses Repositories. Die produktive Betriebskonfiguration wird als
TOML geladen; die mitgelieferte `config/bot.toml` schaltet Concierge und proaktive DMs unter
`[runtime.community]` ein. Sie ist laut Kopfkommentar nur eine Vorlage; beim
Release müssen dieselben Werte in der tatsächlichen Betriebs-TOML stehen.
Der Bot-Release allein belegt noch keine automatische Begrüßung.

Nach unabhängigem Review zuerst alle vier additiven Migrationen
`2026090918_concierge_pate_requests.sql` und
`2026090919_team_applications_pate_kind.sql` sowie die additive
`2026090920_concierge_pate_dm_retry.sql` und
`2026100101_concierge_pate_optout_close.sql` in dieser Reihenfolge mit dem
zentralen Migrator anwenden. Keine historische Migration überspringen oder
ersetzen. Erst nachdem alle vier Migrationen erfolgreich angewandt und
validiert sind, den Bot aus dem geprüften Main bauen. Für eine kontrollierte Probe
in der tatsächlichen Betriebs-TOML unter `[runtime.community]`
`concierge_enabled = true`, `concierge_proactive = true` und eine ausdrücklich
festgelegte `concierge_test_users`-Liste setzen. Der Wert liegt in der TOML,
nicht in einer Environment-Variable. Vor einem offenen Start muss die
Test-Allowlist bewusst aufgehoben und die Wirkung für echte Neuzugänge geprüft
werden. Die Rollen- und Kanal-IDs bleiben die vorhandenen Betriebswerte.
Schon bei aktiviertem Concierge pflegt der Bot beim Start eine angepinnte
Leitfaden-Nachricht im ausschließlich für Paten zugänglichen Kanal
`1524083665838276860`. Vor dem Bot-Restart muss dieses Ziel geprüft werden.

Die Docs-Seite beschreibt das proaktive Angebot nur bedingt und verspricht
keine Zustellung der 24-Stunden-DM: Opt-out oder gesperrte Discord-DMs können
sie verhindern. Hier werden keine Community-DMs oder -Posts für den Test
ausgelöst.

Die 2-Stunden-Karte gilt erst nach erfolgreicher Discord-Bearbeitung als
eskaliert. Nach einem fehlgeschlagenen Karten-Update versucht der Scheduler es
erneut. Ein Edit mit neuer Mention löst bei Discord keinen verlässlichen Ping
aus. Darum postet der Bot zusätzlich eine Antwort auf die Karte mit dem
Owner-Ping; der Rückgabewert der neuen Nachricht wird gespeichert. Die Antwort
nutzt eine stabile Discord-Nonce gegen unmittelbare Duplikate. Das weicht von
der ursprünglichen Formulierung „in derselben Nachricht“ technisch begründet
ab, bleibt aber an genau dieser Anfrage verknüpft. Vor einem erneuten Senden
sucht der Bot die Nonce oder die eigene Antwort auf genau diese Karte im
Discord-Verlauf seit der Anfragekarte. Der Abgleich hat ein Zeitlimit statt
einer festen Nachrichtenobergrenze. Kann er den Verlauf nicht vollständig
prüfen, sendet er keinen zweiten Ping ins Blaue und versucht den Abgleich beim
nächsten Lauf erneut.
Bei der 24-Stunden-Stufe
wird die Anfrage einmal geschlossen. Eine
gesperrte DM oder ein Opt-out beendet den DM-Versuch; ein Transportfehler
bleibt zum erneuten Versuch offen. Das Karten-Update bleibt ebenfalls bis zum
Erfolg offen, auch nach Bot-Neustart; die geschlossene Karte enthält keinen
Übernehmen-Knopf. Bei einem Transport-Timeout ist die Zustellung technisch
unsicher: Ein erneuter Versuch kann im Grenzfall eine zweite DM erzeugen.
Eine stabile Discord-Nonce mit `enforce_nonce` vermeidet unmittelbare
Doppelzustellungen innerhalb des von Discord angebotenen kurzen Fensters.

Validierung des Bot-Stands 661cf86b: alle vier Migrationen in der vollständigen
Reihenfolge auf einer Wegwerf-DB erfolgreich; Paten-Suite 57 grün, Opt-out-
Suite 59 grün und TOML-Snapshot-Test grün. Die Wiederaufnahme der Bot-ID nach
READY ist im Folgecommit mit einem gezielten Concierge-Test und einem
Adaptertest belegt.
