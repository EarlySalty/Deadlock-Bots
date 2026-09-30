Der Rust-OneShot ersetzt die unversionierte `unit-failure-notify.sh` über die
bestehende `unit-failure-notify@.service`. Vorhandene `50-onfailure.conf`-Drop-ins
bleiben wirksam. Betriebswerte kommen ausschließlich aus einer normalen JSON-Datei
(Vorlage `config/unit-failure-notify.example.json`). Es gibt keine ENV-Config,
keine neue Secretdatei und keinen neuen Token.

Die Template-Unit lädt den vorhandenen `infisical-token-bots` mit systemd
`LoadCredential`. `%d/infisical-token` wird direkt an Rust übergeben, dort als
regulärer FD mit CLOEXEC und NOFOLLOW geöffnet und begrenzt in geschützten RAM
gelesen. Der vorhandene Uplink-Unixsocket-Transport liefert danach den bestehenden
`MASTER_BROKER_TOKEN` aus Infisical, mit derselben bestehenden Fallback-Reihenfolge
`MAIN_BOT_INTERNAL_TOKEN`, `TWITCH_INTERNAL_API_TOKEN` wie der Broker. HTTP bleibt ohne Redirects und Proxys auf der
konfigurierten numerischen Loopback-Adresse.

Installation nach dem üblichen Review, Merge und Release-Build:

1. Binary `dl-unit-failure-notify` installieren und normale JSON-Config anlegen.
2. Das Binary mit `--config /absoluter/pfad/config.json --install
   --unit-directory /absoluter/pfad/systemd/user --binary /absoluter/pfad/dl-unit-failure-notify`
   ausführen. Der kleine Installer ersetzt ausschließlich die Template-Unit.
3. `systemctl --user daemon-reload` ausführen. Die bestehenden OnFailure-Drop-ins
   verwenden ab jetzt die Rust-Unit; kein Bot-Neustart ist für diese Verdrahtung nötig.

`MONITOR_INVOCATION_ID` ist ausschließlich das systemd-Laufzeitmetadatum des
auslösenden OnFailure-Ereignisses. Ohne gültige ID wird keine andere Invocation
als Ersatz gelesen. Journal-Diagnose ist zeitlich und auf 160 Einträge/2 MiB
begrenzt. Ausschließlich feste sichere deutsche Ursachen dürfen den Host
verlassen; JSON-Dumps und untrusted Logtexte werden nie als Rohtext versendet.
Scheitert die begrenzte Diagnose, wird der Ausfall trotzdem mit einer festen
sicheren Meldung zur fehlenden Ursache versendet.

Ein eigenes Modus-0700-Verzeichnis hält je Unit atomaren JSON-Zustand und eine
separate flock-Datei. Die Sperre umfasst auch den Versand. Die letzten 256 gezählten
Invocations bleiben persistent und werden auch bei abwechselnden Wiederholungen
nicht erneut gezählt. Unterdrückte Starts bleiben im Zähler bis
zur nächsten bestätigten Meldung. Sendebudget: höchstens eine erfolgreiche
Meldung je rollierenden 24 Stunden und zwei je rollierenden sieben Tagen.
Uhrsprünge rückwärts eröffnen kein zusätzliches Budget. Beobachtungen und noch
offene sichere Payloads werden persistiert; Sendebudget und Zählerrücksetzung
werden erst nach `ok=true` plus Nachrichten-ID des Brokers fortgeschrieben.

Beim Wechsel wird ein eigenes privates Rust-Verzeichnis verwendet; der vorhandene
Legacy-Ordner und dessen Berechtigungen werden nicht verändert. Falls
`legacy_state_directory` angegeben ist, wird `<unit>.state` einmalig unter
derselben Sperre eingelesen: `LAST_NOTIFY FIRST_FAIL LAST_FAIL SUPPRESSED`.
Die bestätigte Meldezeit und der Wiederholungszähler werden übernommen; ein
vorhandener Rust-Zustand wird niemals überschrieben, ungültige Legacy-Daten
sperren den Versand. Da der Shellzustand keinen Wochenverlauf hat, gilt eine
einmalige Übergangsruhe bis sieben Tage nach der zuletzt tatsächlich bestätigten
Legacy-Meldung. Wiederholte Fehler verschieben diese Ruhe nicht. Danach gilt das
normale Budget. Bei `LAST_NOTIFY=0` gibt es keinen belegten Versand zu übernehmen;
der Legacy-Reset hat ältere Historie gegebenenfalls bereits unwiederbringlich
verworfen. Dafür wird keine vergangene Meldung erfunden.

Config, Binary und Installationsverzeichnis müssen dem Dienstnutzer oder root
gehören und gegen Gruppen-/Welt-Schreibzugriff geschützt sein; Symlinks werden
abgewiesen. Die normale Config wird begrenzt über NOFOLLOW gelesen. Der Installer
erstellt seine temporäre Datei exklusiv mit NOFOLLOW, statt fremden Symlinks zu folgen.

Bei unbestätigtem Versand bleiben Idempotenz-Key und Payload gleich. Der bestehende
Broker dedupliziert diese Wiederholung innerhalb seines Cache-Fensters. Es gibt
keine Änderung an diesem Brokervertrag; eine verlorene Bestätigung über einen
Broker-Neustart bzw. seine Cache-TTL hinaus bietet weiterhin keine garantierte
genau-einmal-Zustellung. Parallele OneShots derselben Unit sind durch flock gesperrt.
