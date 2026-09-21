# TempVoice-Reconcile und konservativer Voice-Abschluss

Stand: 21.09.2026. Repo: Deadlock-Bots.

Der Bot bekommt einen wiederkehrenden Abgleich für liegen gebliebene Lanes und verwaiste Voice-Sessions. Die Mikro-Session-Zählweise bleibt Gegenstand eines gesonderten Vorschlags.

## Arbeitsstand und Abgrenzung

Arbeitsbranch: `fix/tempvoice-reconcile-history-20260921`.
Basis: `origin/main`, Commit `65889a6ec09b1cbd613be7907315ac3c95d97943`.
Worktree: `/home/nathanael/.worktrees/deadlock-bots-voice-reconcile-20260921`.

Der geteilte Checkout `/home/nathanael/repos/Deadlock-Bots` wurde nicht bearbeitet. Die Reparatur wurde in Wegwerf-Postgres getestet, auf Produktion aber nicht angewendet. Ein eigener Dienstneustart oder Release-Build wurde nicht ausgeführt. Der produktive Bringup bleibt beim bestehenden Review-/Deploy-Prozess.

## A. Lane-Abgleich

Die Engine merkt sich den Beginn einer bestätigten Leerbeobachtung mit `tokio::time::Instant`. Ein auf 60 Sekunden getakteter Lauf gleicht die Mitgliedschaft gegen den freigegebenen Guild-Cache ab. Er entfernt alte Buchhaltungseinträge, ergänzt tatsächlich anwesende Mitglieder und behält Joins bei, die neuer als die Momentaufnahme sind. Eine besetzte Lane verliert ihren Leertimer. Ein nicht verfügbarer Cache unterbricht den Leer-Nachweis.

Nach 300 Sekunden bestätigter Leere wird im folgenden Zyklus ein Delete versucht. Feste Kanäle, Router-Einstieg und Staging bleiben ausgenommen. Bereits verschwundene Kanäle können ihren verwaisten Zustand unmittelbar verlieren. Direkt vor dem REST-Aufruf werden Cache und Buchhaltung erneut geprüft. Die Frist setzt einen vollständigen Cache und eine erreichbare Discord-API voraus; API-Ausfälle führen zu protokollierten Wiederholungen statt zu einem behaupteten Löscherfolg.

Die normale eventgetriebene Bereinigung bleibt bestehen. Beim Start werden bekannte leere Kanäle weiterhin ohne zusätzliche Fünf-Minuten-Frist geprüft. Der Startup-Purge überspringt fehlende Kanal-Cache-Einträge wie bisher. Rehydration und Startup-Purge sind durch ein Signal geordnet.

Discord-Delete, Lane-DB-Delete, History-Abgleich und der LFG-Nachlauf haben Zeitlimits von jeweils zehn Sekunden. Reconcile-Kandidaten werden getrennt bearbeitet, sodass ein hängender Delete eine andere Lane nicht aufhält. Bei Fehlern bleibt ein Wiederholungszustand erhalten, auch für zuvor nicht persistierte Custom-Lanes. Ein Discord-404 gilt beim wiederholten Löschen als bereits erledigt. Fehlgeschlagene Deletes melden WARN mit `channel_id`, erfolgreiche Aktionen und der Reconcile-Abschluss INFO.

### Reconnect

READY, GUILD_CREATE, CacheReady und RESUMED sind an die Reconciliation angebunden. Nach einem frischen READY wird auf den vollständigen Guild-Cache gewartet. RESUMED veröffentlicht einen erneuten CacheReady-Impuls für die geladenen Guilds. Shard-Unterbrechungen und nicht verfügbare Guilds sperren Momentaufnahmen.

Serenity 0.12.5 entfernt beim Verarbeiten eines READY die alten Guild-Cache-Einträge. Bereits vor dem READY-Callback geladene frische Guilds werden deshalb übernommen und erneut signalisiert. Das verhindert einen hängen gebliebenen Bereitschaftszustand bei anders angeordneten Tokio-Callbacks. Referenz: Serenity `src/cache/event.rs:480`, `src/cache/event.rs:94` sowie `rust/crates/dl-discord/src/gateway.rs:259`.

### Konfiguration

Der neue Abschnitt ist optional. Ohne Abschnitt gilt der Default 300 Sekunden:

```toml
[tempvoice]
empty_lane_grace_seconds = 300
```

Zulässiger Bereich: 1 bis 86400 Sekunden. Beispiel für eine kürzere Frist: 120. Die Konfiguration wird beim regulären Bot-Start übernommen. Eine produktive TOML wurde in dieser Arbeit nicht geändert. Der aktuelle Live-Stand verwendet damit nicht automatisch die neue Implementierung.

## B. Voice-History und Einmal-Reparatur

Es existieren zwei Recorder mit unterschiedlichen Verträgen:

Die Journey führt `activity.voice_open_sessions`. Ihr regulärer Leave schreibt nach `activity.voice_metadata_events`. Der neue Abgleich folgt diesem Vertrag: leere oder verschwundene Kanäle schließen am letzten `updated_at`, begrenzt auf den Beobachtungszeitpunkt beziehungsweise jetzt. `duration_seconds` wird aus diesem Endpunkt und `joined_at` abgeleitet und auf mindestens null begrenzt. Löschen und Leave-Eintrag erfolgen transaktional. User, Guild, Kanal, Join und Aktualisierung werden vor dem Löschen erneut verglichen. Dadurch wird eine inzwischen erneuerte Zeile nicht mit einem älteren Snapshot geschlossen.

Der getrennte `VoiceTracker` besitzt aktive Runtime-Sessions und schreibt Punkte sowie `activity.voice_session_log`. Sein Keepalive prüft jetzt die tatsächliche User-/Kanal-Paarung. Geister schließen am letzten belegten `last_update`, statt durch Keepalives bis zur Kanal-Löschung verlängert zu werden. Der bestehende Writer bleibt zuständig; ein fehlgeschlagener DB-Abschluss lässt die Runtime-Session für einen Retry stehen und rollt die Punktebuchung zurück. Für rückwirkend korrigierte Geister wird kein Feedback oder Survey ausgelöst.

Damit werden aus einer Journey-Zeile keine erfundenen Peak-User-Werte, Punkte oder zusätzlichen Session-Log-Zeilen erzeugt. Abgeschlossene History-Zeilen werden nicht aktualisiert. Der Heartbeat aktualisiert bestätigte User-/Kanal-Paare und überspringt Privacy-Opt-outs. Verwaiste Opt-out-Zeilen werden ohne neue History entfernt. Normale Voice-Updates aktualisieren ebenfalls den belegten Zeitpunkt.

Die gezielte Reparatur liegt in `rust/ops/repair_voice_open_session_20260802.sql`. Sie prüft die konkrete Kombination aus User, Guild, Kanal und Join-Zeit vom 02.08.2026, zusätzlich `updated_at < 2026-09-21 00:00:00+00`. Eine inzwischen erneuerte Session bleibt damit geschützt. Das SQL ist transaktional, wiederholbar, verwendet die bestehende Privacy-Sperre und liefert die Anzahl entfernter offener Sessions sowie angehängter Leave-Ereignisse. Es ist ein Betriebswerkzeug, keine Schema-Migration.

Der bestehende Betriebsprozess kann das SQL nach Review über seinen freigegebenen DB-Zugang ausführen. Die erwartete Wirkung bei unverändert vorhandenem Befund ist eine entfernte offene Zeile und ein Leave-Eintrag. Null Zeilen können eine bereits erledigte oder inzwischen geänderte Session bedeuten und sind kein Beweis für einen Reparaturfehler. Der tatsächliche damalige Leave-Zeitpunkt wird nicht rekonstruiert.

## C. Mikro-Sessions: Befund und Vorschlag

Die Werte 341 Sessions bis fünf Sekunden, davon 273 unter dem genannten Lane-Namen, stammen aus dem Auftrag. Sie wurden nicht gegen die produktive Datenbank neu gezählt.

Sind alter und neuer Kanal bekannt, veröffentlicht der Gateway-Handler ein `VoiceEvent::Move`, nicht zusätzlich ein künstliches Join-/Leave-Paar. Fehlt der alte Voice-State im Cache, kann derselbe Eingang dagegen als Join normalisiert werden (`rust/crates/dl-discord/src/gateway.rs:614`, Methode `voice_state_update`). Der Tracker beendet bei einem Move das bisherige Kanal-Segment und prüft anschließend Quell- und Zielkanal auf aktive Sessions (`rust/crates/dl-voice/src/tracker.rs:316`). Ein kurzer tatsächlicher Zwischenaufenthalt kann so ein eigenes kurzes Segment erzeugen. Die Standardbedingung für den Session-Start sind zwei aktive Mitglieder (`tracker.rs:47`, `tracker.rs:449`). Ein allein betretenes Router-VC erzeugt mit diesem Default deshalb nicht automatisch einen Session-Log-Eintrag.

Im Router sind zwei Wege zu unterscheiden. `LaneRouter::handle_event` erstellt beim Eintritt unmittelbar die gespeicherte Standard-Lane; ohne Standard folgt unmittelbar eine Casual-Fallback-Lane (`router.rs:993`, `router.rs:1039`). Daneben existiert der verzögerte Auto-Move mit Default 60 Sekunden (`rust/bin/dl-bot/src/main.rs:794`). Dieser prüft nach dem Timer, ob der Nutzer noch im Router und dort allein ist, und prüft den Zustand vor dem Move erneut (`router.rs:1062`, `router.rs:1153`).

Für den im Auftrag genannten Nutzer wurde in den vorhandenen Dienstlogs am **21.09.2026 um 08:44:59.837297 UTC** ein Eintrag `Router-Auto-Move: Entscheidung`, `decision="skipped"`, `reason="left_or_rejoined"` gefunden. Dieser Timer-Eintrag belegt einen übersprungenen Fallback, keinen ausgelösten Move. Die zugängliche Log-Stelle beweist daher nicht, dass der verzögerte Fallback die 341 Blips verursacht hat.

Die kurze Segmentierung passt technisch zu unmittelbaren Router-/Staging-Moves oder schnellen weiteren Kanalwechseln. Die Zuordnung der 273 Zeilen muss über Kanal-IDs und Zeitpunkte erfolgen; ein Lane-Name allein identifiziert weder den zentralen Router-VC `1513468587195633674` noch den Initiator eines Moves. Für eine vollständige Attribution der gezählten Fälle fehlen hier die entsprechenden Zeilen und ihre Korrelation mit Move-Entscheidungen.

**Vorschlag zur Freigabe:** Bot-initiierte Moves vor dem REST-Aufruf mit Quellkanal, Zielkanal, Nutzer, Ursache und kurzer Gültigkeit markieren und mit dem bestätigten Gateway-Move verbinden. Die Roh-History bleibt dabei erhalten. Eine gesondert freizugebende Auswertungsregel kann bestätigte Router-Transit-Segmente aus Gesprächs-/Session-Zählungen ausnehmen oder mit dem angrenzenden Aufenthalt verknüpfen. Ein pauschaler Fünf-Sekunden-Filter würde auch echte Kurzaufenthalte verschlucken. Diese Statistikänderung ist nicht implementiert.

## Quellstellen der Änderungen

| Änderung | Pfad und Einstieg |
| --- | --- |
| Leerfrist, Retry-Buchhaltung, geordneter Start und 60-Sekunden-Loop | `rust/crates/dl-voice/src/tempvoice/engine.rs:221`, `:320`, `:2238` |
| Reconcile, History-Reihenfolge, Startup-Vertrag, Zeitlimits und Delete-Retry | `rust/crates/dl-voice/src/tempvoice/engine/reconcile.rs:8`, `:38`, `:83`, `:125`, `:234` |
| Reconnect-Adapter und Cache-Bereitschaft | `rust/crates/dl-discord/src/voice_cache.rs:1`, `rust/crates/dl-discord/src/gateway.rs:259`, `:291`, `:305`, `:316` |
| Export und Zugriff auf Momentaufnahmen | `rust/crates/dl-discord/src/lib.rs:21`, `rust/crates/dl-discord/src/adapter.rs:86` |
| Voice-/Lane-Port und idempotenter Discord-Delete | `rust/crates/dl-voice/src/glue.rs:662`, `:785`, `:894` |
| TOML-Default, Validierung und Test | `rust/crates/dl-core/src/bot_config.rs:135`, `:351`, `:464` |
| Übernahme der Frist und Anbindung des Recorders | `rust/bin/dl-bot/src/main.rs:759`, `:1430` |
| Konservativer Runtime-Abschluss und Keepalive | `rust/crates/dl-voice/src/tracker/reconcile.rs:9`, `:52`, `rust/crates/dl-voice/src/tracker.rs:604` |
| Journey-Abschluss, Heartbeat und SQL-Tests | `rust/crates/dl-activity/src/journey/voice_reconcile.rs:9`, `:87`, `:110`; Export und normale Update-Zeit in `rust/crates/dl-activity/src/journey.rs:7`, `:1226` |
| Einmal-Reparatur | `rust/ops/repair_voice_open_session_20260802.sql:1` |
| Lane-/Reconnect-/Timeout-Tests | `rust/crates/dl-voice/src/tempvoice/engine/reconcile_tests.rs:29`, `:58`, `:85`, `:111`, `:158`, `:181`, `:206`; Delete-Gegenprobe in `engine.rs:3122` |
| Session-Abschluss und DB-Rollback-Tests | `rust/crates/dl-voice/src/tracker.rs:857`, `:889` |
| Bestehende Config-Test-Fixtures um den Default ergänzt | `rust/crates/dl-voice/src/tempvoice/interface.rs:2489`, `:2831`, `:2861`, `:3335`; `rust/crates/dl-voice/src/router.rs:2675` |

## Testnachweise

Der unveränderte Ausgangsstand bestand `cargo test -p dl-voice -j 2` mit **450 bestandenen und vier ignorierten Tests**. Zwei vor der produktiven Korrektur hinzugefügte Gegenproben schlugen gezielt fehl: Zustandsverlust nach Delete-Fehler und Keepalive-Verlängerung von Geistern. Das Log enthält zwölf bestandene und diese zwei fehlgeschlagenen Tests.

Der anschließende vollständige Lauf

```text
cargo test -p dl-voice -p dl-activity -p dl-discord -p dl-core -j 2
```

bestand mit **597 bestandenen Tests und vier unverändert ignorierten Tests**: 460 Voice, 42 Activity, 48 Discord sowie 47 Core-Unit-/Integrationstests. Die Ausführung erfolgte über `rust/scripts/central_test_db.sh` in Wegwerf-Postgres.

Clippy bestand mit `--all-targets -j 2 -- -D warnings` sowohl für `dl-voice` als auch für `dl-bot`, `dl-core`, `dl-discord` und `dl-activity`. Damit wurde auch die Bot-Verdrahtung geprüft.

Die zusätzliche Voice-/Gateway-Wiederholung mit `--test-threads=4` bestand ebenfalls: 460 Voice- und 48 Discord-Tests, vier unverändert ignorierte Voice-Tests. Nach der zusätzlichen Privacy-Absicherung des Heartbeats bestanden die vier gezielten Journey-SQL-Tests erneut, einschließlich der Kontrolle, dass ein aktiver Opt-out keinen Heartbeat erhält.

Nachweise auf dem Arbeitsrechner: `/home/nathanael/buildlogs/voice-reconcile-20260921/{baseline,red,green2,full-tests,final-tests}.log`. Die ergänzenden Clippy- und SQL-Läufe liegen in den zugehörigen Codex-MCP-Joblogs. Produktive History wurde für diese Tests nicht verändert.

TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: .tasks/2026-09-21-tempvoice-reconcile/REPORT.md
