# Direkte Build-Erstellung: Discord-Abnahme

Stand: 24. September 2026. Branch `feat/brain-direct-build-publish-20260924`. Nutzerauftrag: Brain soll auf ausdrückliche Nachfrage Builds im Spiel erstellen. Kein Produktiv-Deploy.

## Änderungen

Der bestehende Brain-Verbraucher löst Veröffentlichungen anhand einer ausdrücklichen Erstellungsbitte aus, nicht anhand des bloßen Wortes Build. Erklärungen, Bewertungen, Verneinungen und Entwürfe bleiben ohne automatische Veröffentlichung. Der reguläre Modus nutzt den geprüften neuen CLI-Befehl `publish-build-query`; der bestehende offene Testmodus nutzt weiterhin das gekennzeichnete experimentelle `review-build`. Keine Änderung an Modellen oder Testmodus-Konfiguration.

Erfolg erfordert einen abgeschlossenen Steam-Auftrag und eine positive Build-ID. Wartende Aufträge werden als eingereiht bezeichnet. Fehlende, ungültige oder unbestätigte IDs werden nicht als nutzbare Builds ausgegeben. Die Frage bleibt ein einzelnes Subprozessargument nach `--`.

## Ausgeführte Tests

Cargo/Rust 1.98.0, `--locked -j 2` im eigenen Worktree.

- `cargo test -p dl-brain --lib`: 9 bestanden, 0 Fehler, 0 ignoriert. Darunter ausdrückliche Erstellungsbitten, Verneinungen einschließlich „veröffentliche ihn nicht“, Entwurfswünsche, reine Erklärungen und bestätigte IDs.
- `cargo test -p dl-bot --bin dl-bot brain`: 15 bestanden, 0 Fehler, 0 ignoriert. Bestehende Brain-Kanalgrenzen, DM-Sperre, Slash-Command und Retrieval-Verhalten bleiben geprüft.
- `cargo test -p dl-bot --bin dl-bot requested_build_cli`: 1 bestanden. Mock-CLI belegt regulären und Review-Befehl sowie den Argumentseparator. Keine Shell-Interpolation und kein echter Steam-Auftrag.
- `cargo test -p dl-bot --bin dl-bot review_build_receipt`: 1 bestanden. Erfolg ausschließlich bei bestätigter ID; wartende und fehlgeschlagene Zustände geben diese ID nicht als veröffentlicht aus.

26 ausgewählte Tests bestanden. Keine vollständige Workspace-, GitHub- oder Live-Abnahme behauptet. Ein erster kombinierter Shell-Aufruf nutzte für den zweiten Befehl versehentlich das alte Cargo; die betroffenen Tests wurden danach mit dem expliziten Rustup-Cargo erfolgreich ausgeführt.

## Abhängigkeiten und offene Abnahme

Der Brain-CLI-Branch `feat/direct-build-publish-20260924` muss vor dem Discord-Verbraucher ausgeliefert sein. Die beiden PRs gehören fachlich zusammen. Bestehende Schutz- und Review-Gates bleiben aktiv. Ein unabhängiger externer Review und der echte In-Game-Funktionsbeweis stehen noch aus. GitHub-Actions werden am offenen PR geprüft; beim zugehörigen Steam-PR war bereits ein Abrechnungs-/Ausgabenlimit-Blocker bestätigt.

Der aktuelle PR-first-Testbetrieb untersagt Merge, main-Push, Auto-Merge, Produktions-Uploads, Neustart und Cleanup vor Auslieferung.

MERGEPROTOKOLL[MS-1]: kein Merge: PR-first-Testbetrieb
