# Concierge: gemergter Fix, Abschluss blockiert

Stand: 2026-10-08. Auftraggeber: d3a1741e-82bc-4a48-865b-2845c663dca7.

## Aktueller Stand am 8. Oktober

Die ursprünglichen Codefixes sind in den tatsächlich laufenden Versionen enthalten: Brain b7289d115c0b64016fcfe6cbfe3c797cf9fa76e2 und Bots 0fb873c6887c6ec8df6ce50d15c8ded9781fbadf. Das ist ein beobachteter Zustand, kein von diesem Worker behaupteter neuer Rollout. Brain lief nach unserem Import mit tatsächlichem PID 573910 statt 3178539, Start 04:21:06 CEST. Die exe zeigt auf den genannten Brain-Release und nicht auf eine gelöschte Datei. Das Fehlerjournal seit diesem Start ist leer. Der tatsächliche Bots-Kindprozess bleibt PID 2766584; der Launcher-PID 2766542 ist kein eigener Bot-Prozessnachweis.

Die echte Pocket-Nachricht 1557551797613240342 vom 8. Oktober 00:34:46 UTC erhielt Antwort 1557551799479832590 mit dem privaten Hilfehinweis. Der laufende modglue-Code kehrt bei nichtöffentlichen Kanälen vor dem Brain-Aufruf zurück. Der bereits gemergte Eingangsfix 8e1b8f03 sendet dagegen die Frage an Brain und hält Discord-Leserechte bei solchen Kanälen geschlossen. Keine Öffnung privater Discord-Daten wurde vorgenommen.

Für diesen aktuellen Bots-main wurde dl-bot aus einem eigenen sauberen Worktree gebaut: bestätigter Exit 0, Dauer 8m 55s, Datei /home/nathanael/.local/state/concierge-bots-8e1b8f03/target/release/dl-bot, SHA256 85409aac1eaf49e9a91916da19eb3b3fe4bfbee293e3c5487a993c9de3ef66e6. Ein frischer Fetch bestätigte HEAD und origin/main auf 8e1b8f03cf0d84f4754eedc8fe848ef0fceffce8. Das ist ein gebautes dl-bot-Artefakt, kein installiertes vollständiges Release-Bundle.

Alle fünf korrigierten Seiten wurden aus Docs-SHA defcd1ba6ea75ea9ec0dcac559fac07ab6d78992 aktiviert. Der Import bjwycgxhk bestätigte Exit 0 und aktivierten Endstand maintenance-d44f71917b146df590f6965633015f5d4eeead6b0f30cdfca71680fc0156d4a0. Die beiden neuen Docs-Bindungen dm-concierge und paten sind jetzt vorhanden. Das ursprüngliche geprüfte Fünf-Dokumente-Paket blieb unverändert. Sein SHA256 ist 3c33bc8340674d0fa6fdeadfd0a565c352b6a5274774ad4c3f2970e81f7b5132. Das neue, unveränderte Drei-Dokumente-Teilpaket hat SHA256 1a2bb1c57e4c518753efd39357b55e7094561edb3c0c2986ccf921e00e4e644b und liegt zusätzlich im bestehenden inhaltsadressierten Artefaktspeicher. Seine drei Inhalte und der vorhandene Prüfverweis wurden bewahrt.

Beim anschließenden Entfernen der alten Besitzerbindungen blieben deren zwei öffentliche Dokumente im aktiven Wissensstand festgehalten. Der Reader sperrt sie dann, während die Bereitschaftsprüfung noch öffentliche, gültige Dokumentköpfe für die festgehaltenen Revisionen verlangt. /readyz antwortete deshalb mit 503 und not_ready. Die vorherigen beiden öffentlichen Regeln wurden exakt aus dem unveränderten geprüften Paket wiederhergestellt, über den vorhandenen Rust-Operator mit Exit 0. /readyz bestätigt wieder ready für den Endstand d44f7191; die Bindungs-Prüfsumme ist 04e2ff195d566bbba1ac67150f37ff488d9bf2b774b98ed3464efcc1ab1d4683. Die alten zwei Dokumentbindungen bestanden bis zum anschließend ausgeführten Übergang vorübergehend zusätzlich. Sie sind inzwischen aus dem aktiven Antwortbestand entfernt; historische gespeicherte Wissensstände wurden nicht gelöscht.

Die kanonischen /etc-Konfigurationen führen die zwei Ziele bei Docs und nutzen das korrekte Drei-Dokumente-Teilpaket. Der Übergang ist abgeschlossen: Zuerst wurden genau die zwei alten Dokumentbindungen aus einem neuen Wissensstand entfernt, dieser wurde über den vorhandenen ConfigWriter und ActivationPlan aktiviert, danach entfernte der vorhandene Rust-Pflegepfad die alten Regeln. Das Rust-Einmalwerkzeug überprüfte vor der Änderung die fünf korrigierten Dokumenthashes, unabhängige Wissensstände und die auf genau zwei öffentliche Quellen begrenzte Änderung. Vorprüfung br4zx53mu, Aktivierung b33gcpigo und abschließende Entfernung der alten Regeln b352xwovo bestätigten jeweils Exit 0. Der abschließende Pflegebericht enthält isolated_errors: [].

Aktiver Wissensstand: maintenance-retire-588d128199fec8a7c988e0ff6bf68f13d33cb8f1a2fcbbc7c2584439fd878cfb. Alle fünf korrigierten Dokumente bleiben festgehalten, die zwei alten Bots-Quellen fehlen im neuen Antwortbestand. /readyz wurde nach der abschließenden Pflege mit Content-Type application/json, status ready, dem richtigen Wissensstand und Bindungs-Prüfsumme 75676e42faaa8f2c01e8707cf45cba107c53fdaa932dd04f5606713f488b8a73 geprüft. Der eigene Übergang startete Brain um 06:24:54 CEST neu: tatsächlicher PID 573910 zu 1992533. Die exe zeigt weiterhin ohne deleted auf den bestehenden b7289d11-Release. NRestarts ist 0; journal -p err seit diesem Start ist leer. Kontrollanker /v1/answer und Fixanker [[coaching]] wurden mit gemeinsamem bestätigtem Exit 0 in der laufenden Binary geprüft. Das ist eine bewiesene Corpus-Aktivierung, kein neuer Binary-Deploy.

Der erste Werkzeug-Build baliwvn18 scheiterte mit Exit 101, weil Cargo-Metadaten den fehlenden tb-crypto-Nachbarpfad benötigten. Dafür wurde ein eigener unveränderter Twitch-Worktree von origin/main, SHA 3341098f, im isolierten Layout ergänzt. Cargo metadata bestand anschließend. Der zweite Werkzeug-Build bbxugxcja bestätigte Exit 0. Es wurde kein dauerhafter Dienst und kein neuer Bot-Antwortgenerator angelegt. Die vorhandenen Rust-Bausteine übernahmen Veröffentlichung, Konfigurationssperre, Aktivierung und Rückfall.

Der allgemeine freigegebene Bots-Installer fehlt weiterhin. /opt/deadlock/bots und seine Release-Verzeichnisse gehören root und sind nicht vom Worker beschreibbar. Es wurde weder generisches sudo benutzt noch ein auf fremde SHAs festgelegter historischer Installer umgeschrieben. Der ältere Brain-Bundle f6f5cef6 wurde nicht installiert. Der frühere eigene f6-Quellpfad existiert bei der Prüfung nicht mehr.

LIVEBEWEIS[DV-1]: PID 573910->1992533 | exe ohne (deleted) | journal -p err leer | Anker "[[coaching]]" in Binary | Funktion: Corpus-Übergang mit fünf korrigierten Dokumenten und ohne zwei alte Quellen geprüft; Bots-Eingangsfix nicht installiert, echte Pocket-, Haze- und Coaching-Fragen nach diesem Fix offen | Ort: http://127.0.0.1:8788/readyz, API; https://discord.com/channels/1289721245281292288/1374364800817303632, #bot-logs.

Die drei zuletzt verwendeten eigenen Quellbäume wurden nach Prüfung einschließlich ignorierter Dateien und nach bestätigtem Ende aller eigenen Compiler- und Werkzeugprozesse entfernt. Das waren Bots 8e1b8f03, Brain ef2f1f95 und die unveränderte Crypto-Abhängigkeit aus Twitch 3341098f. Die eigene Abhängigkeitsverknüpfung und das leere Layout wurden entfernt, die drei Git-Registrierungen bereinigt. Das gebaute dl-bot-Artefakt, das Rust-Einmalwerkzeug und die Logs liegen außerhalb der entfernten Worktrees. Keine fremde Arbeit wurde integriert oder gelöscht.

Die folgenden Abschnitte dokumentieren den früheren Stand vom 7. Oktober. Die oben genannten Betriebsbeobachtungen ersetzen ihre inzwischen überholten Aussagen zu Aktivierung, laufenden Versionen und noch aktiven Hintergrundaufgaben. REGISTER.md enthält die aktuellen eigenen Quellbäume und Arbeitszustände. Kein settle --selbst.

## Ergebnis auf origin/main

- Deadlock-Bots: 56571e40. Discord behält zentrale Brain-Texte auch bei InsufficientEvidence. Tatsächlich leere Ergebnisse nutzen den bestehenden Spiel-Fallback statt Nani.
- Deadlock-Brain: f6f5cef6. Zentrale Concierge-Identität in Ich-Form, kein eigenes technisches Innenleben, konstante Coaching-Ziele je Plattform und zentraler Spiel-Fallback. Nani bleibt bei ausdrücklich erkennbaren Twitch-Bot-Fragen.
- Deadlock-Docs: 534eb4c. Fünf öffentliche Seiten korrigiert: faq-bot-selbst, dm-concierge, paten, coaching und module/brain.

Alle drei lokalen Gate-Prüfungen lieferten ALLOW. Keine BLOCK-Runde, kein Fixer nötig. Kein PR, keine GitHub-Actions-Arbeit, keine Änderung am fremden schmutzigen Checkout.

## Belegte Ursache

Die tatsächlichen Nachrichten in #bot-logs wurden gelesen. Pocket um 08:07:57 und die Haze-Frage mit „scalled“ um 08:16:57 bekamen den Nani-Verweis. Die Coaching-Frage um 08:10:27 bekam „Schreib dem Concierge“ und keinen Coaching-Kanal.

Das Journal des Produktivdienstes deadlock-bot-rust.service belegt InsufficientEvidence um 08:07:58 und 08:16:59 sowie Answered um 08:10:30. Die zentrale API auf 127.0.0.1:8788 ist erreichbar. Kein belegter Endpoint-Ausfall und kein belegter Timeout bei diesen beiden Spielfragen. Der Adapter verwarf unzureichende Antworten; modglue ersetzte sie pauschal durch Nani.

Der laufende Brain verwendet release_lexical mit Limit 6. Das Spielwissen bleibt für die beiden Fragen unzureichend. Eine Operator-Abfrage lieferte irrelevante interne Dokumente; sie ist kein exakter Nachbau des öffentlichen Discord-Retrievals. Es wurde kein Umbau der Spielwissensversorgung durchgeführt.

BESTAND[BS-1]: ja | Fundort: rust/bin/dl-bot/src/modglue.rs:718 | Anknüpfung: vorhandener Spiel-Fallback, zentraler Brain-Vertrag und konfigurierte Discord-Consumer.

## Prüfungen

Die gezielte rustfmt-Prüfung aller sechs geänderten Rust-Dateien ist grün. Brain-Clippy für brain-api und brain-contracts mit all-targets und D warnings ist grün. Der serielle Brain-Testlauf ist grün. Der erste parallele Testlauf scheiterte bei unread_keepalive_bodies_close_after_auth_failure_or_timeout an einer HTTP-Wartegrenze; der serielle Wiederholungslauf bestand.

TESTNACHWEIS[TW-1]: 142 passed, 0 ignored | Baseline: Bots-Clippy 3 alt rot, unverändert 3 rot nach Fix.

Davon 126 Brain-Tests, 6 API-Projektionsfixtures und 10 bestehende Discord-Konversationstests. Die Konversationstests haben einen bestätigten Gesamtexit 0. Der zuvor vom Harness gestoppte API-Lauf lief in seinem eigenen Cargo-Kindprozess zu Ende; das vollständige Log meldet 6 bestandene Tests und keine fehlgeschlagenen Tests, der Gesamtexit dieses Kindprozesses wurde nicht separat erfasst.

Bots-Clippy reproduziert im sauberen Ausgangsworktree e18f5222 dieselben drei dl-brain-Fehler: deprecated fetch_update und zwei double_must_use. Der erste einschließlich Abhängigkeiten ausgeführte Clippy-Lauf scheiterte zusätzlich an explicit_auto_deref in dl-central-db. Kein Bots-Clippy-Lauf wird als grün gemeldet.

Der vorhandene central_test_db.sh-Wrapper scheiterte vor dem Teststart an einer fehlenden Migrator-Konfiguration. Der separate Bots-API-Fixture-Lauf wurde vom Harness nach seiner Hintergrundzeitgrenze gestoppt; sein eigener Cargo-Kindprozess beendete später die Tests. Der zweite Lauf für die vorhandenen gespraech_-Tests beendete sich mit Exit 0. Beide Logs enthalten die tatsächlich gelaufenen Testzahlen.

Alle fünf geänderten öffentlichen HTML-Seiten haben 0 Validatorverstöße. Der Gesamtkorpus hat 13 Verstöße; ein separater unveränderter Ausgangsworktree reproduziert dieselben 13. Der Gesamtkorpus wird nicht als grün gemeldet.

TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 5 belegt | Senke: fünf öffentliche Docs-Seiten und zentrale Antworttexte.

WIRKUNGSPRUEFUNG[WP-1]: 3 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 2/2 geprüft.

Die drei Befunde sind der verworfene zentrale Wissenslücken-Text, die pauschale Nani-Ersatzantwort und die fehlende Ich-Form samt Coaching-Ziel. Der Twitch-Zwilling in tb-knowledge verwirft weiterhin InsufficientEvidence; er wurde nicht geändert. Beantwortete Coaching-Verweise werden zentral plattformgerecht ersetzt. Eine vollständige Twitch-Fallback-Reparatur wird nicht behauptet.

MERGEPROTOKOLL[MS-1]: 16 gezählte native Git-Schritte in eigenen Concierge-Worktrees einzeln | Anläufe: alle drei Repos veröffentlicht, Docs-Push nach erneutem Laden der Merge-Regel | Gate: [gpt-6.1-sol] ALLOW in allen drei Repos.

## Corpus-Aktivierung

Der bestehende Rust-Importer import-reviewed --public-docs lief zunächst erfolgreich. Coaching, faq-bot-selbst und module/brain sind mit Docs-SHA 534eb4c aktiviert. Der damalige Endstand war maintenance-d7ae0a7d7751f7490dd9ad4e814d038028b615e458df9dbd6fefcf589a94635c.

Dm-concierge und paten sind nicht unter den Docs-doc_targets registriert. Ihre bisherigen Imports gehören zu Deadlock-Bots und tragen SHA 19f9292c. Der Versuch, ausschließlich diese beiden öffentlichen Ziele nach Docs zu verschieben, scheiterte beim Reimport mit maintenance_command_failed, auch nach erfolgreichem register-config. Die genaue tiefere Fehlerursache ist nicht belegt. Sämtliche eigenen Änderungen an /etc/deadlock-brain/maintenance.json und maintenance-runtime.json wurden zurückgenommen; die ursprüngliche Konfiguration wurde erfolgreich erneut registriert. Es verbleibt keine absichtlich geänderte Betriebskonfiguration aus diesem Versuch.

## Tatsächliche Blocker und offene Arbeit

1. Ein allgemeiner freigegebener Bots-Deploy-Wrapper wurde nicht gefunden. Die vorhandenen alten aufgabengebundenen Installer sind auf fremde SHAs und Release-Pointer festgelegt. Sie wurden nicht umgeschrieben oder ausgeführt. /usr/local/bin und /usr/local/libexec enthalten nur den Twitch-Deploy-Wrapper, bot-restart und brain-release.
2. bot-restart enthält brain-serve.service nicht. Keine Umgehung über generisches sudo oder einen ungeprüften Installer. Der Corpus-Importer hat seinen bestehenden Brain-Neustart ausgeführt, aber das ist kein Deploy des neuen Codes.
3. Die zwei Paten-Seiten brauchen einen funktionierenden Importpfad, der ihre vorhandene Eigentümerbindung respektiert. Der versuchte Besitzerwechsel wurde zurückgenommen.

Der Brain-Release-Build aus dem sauberen eigenen origin/main-Worktree ist fertig. Bundle: /home/nathanael/.local/state/brain-concierge-f6f5cef6. Die vorgeschriebene frische Reproduzierbarkeitsprüfung läuft noch. Keine Installation des Bundles und kein neuer Bots-Release wurden bestätigt.

LIVEBEWEIS[DV-1]: nicht erbracht | neuer Code nicht als laufender Prozess bewiesen | Funktion: Pocket, Haze und Coaching nach neuem Deploy offen | Ort: https://discord.com/channels/1289721245281292288/1374364800817303632 (#bot-logs).

Letzter beobachteter Brain-PID: 2645590, Start 09:04:28 CEST nach Corpus-Import. Letzter systemd-Haupt-PID des Bots-Launchers: 2848836, Start 01:23:58 CEST. Das ist kein PID-Wechselnachweis für diesen Code-Fix. Eine vom Bot selbst gesendete MCP-Nachricht zählt nicht als menschlicher Eingang; ein solcher Scheinbeweis wurde nicht erzeugt.

## Cleanup nach Stop-Hook

Die drei gemergten Aufgabenbranches wurden nach merge-base --is-ancestor mit bestätigtem Exit 0 entfernt. Ihre drei Entwicklungsworktrees und beide Baseline-Worktrees sind gelöscht; vorher waren die ignorierten Inhalte geprüft und sämtliche Bots-Buildprozesse beendet. Die Testlogs liegen außerhalb der Worktrees. Diese Bereinigung erfolgte in 14 einzelnen Git-Schritten einschließlich prune. Kein fremder Prozess wurde gestoppt und keine Lock-Datei gelöscht.

Der gemeinsame Docs-main enthält 13 fremde, vor dieser Sitzung entstandene automatische Frische-Commits vom 19. September bis 4. Oktober 2026. Das ist nicht offene Branch-Arbeit dieses Auftrags. Sie wurden weder gepusht noch zurückgesetzt. Unser Docs-Fix 534eb4c ist bereits auf origin/main.

Der Stop-Hook meldet außerdem feat/brain-rust-cutover-20260919 aus dem gemeinsamen Brain-Checkout. Seine 21 nicht in origin/main enthaltenen Commits datieren vom 18. bis 24. September 2026. Sie wurden in dieser Sitzung weder erstellt noch verändert. Eine Integration dieser fremden älteren Arbeit würde den Concierge-Auftrag überschreiten; sie wurde nicht vorgenommen. Die Meldung des Stop-Hooks ist dadurch nicht mit einem ungemergten Aufgabenbranch dieses Workers gleichzusetzen.

Nur der saubere detached Release-Worktree /home/nathanael/.worktrees/Deadlock-Brain-concierge-release bleibt für die noch laufende eigene Reproduzierbarkeitsprüfung erhalten. Hintergrundaufgabe b4dfvjg6x, Log /tmp/concierge-brain-release-verify.log. Das fertige Bundle liegt separat unter /home/nathanael/.local/state/brain-concierge-f6f5cef6.

Kein settle --selbst: Deploy und Live-Abschluss sind weiterhin blockiert.

Nächster konkreter Schritt: den Deploy-Blocker mit dem Haupt-Orchestrator anhand dieses Berichts klären. Danach aktuellen origin/main-SHA bauen und deployen, die drei echten Discord-Fragen prüfen und erst anschließend den verbleibenden Release-Worktree entfernen.
