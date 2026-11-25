# Register: Concierge, Coaching und Spiel-Fallback

Stand: 8. Oktober 2026. Abschluss am Bots-Deploy blockiert.

## Zuordnung

- Intent-Thread: d3a1741e-82bc-4a48-865b-2845c663dca7.
- Ausführung: bestehender Blatt-Worker; CLI-Sitzung 382bbcc8-4264-42be-a848-27a769132381. Keine neuen T3-Threads.
- Ursprüngliche Aufgabenbranches und Entwicklungsworktrees: nach bestätigter Integration entfernt.

## Laufender Stand

Die ursprünglichen Fixes sind auf main veröffentlicht und in den beobachteten laufenden Versionen enthalten: Brain b7289d115c0b64016fcfe6cbfe3c797cf9fa76e2 und Bots 0fb873c6887c6ec8df6ce50d15c8ded9781fbadf. Der zusätzliche Discord-Eingangsfix 8e1b8f03 ist gebaut, aber nicht installiert.

Die Corpus-Umstellung ist abgeschlossen. Fünf korrigierte Docs-Seiten aus defcd1ba6ea75ea9ec0dcac559fac07ab6d78992 bleiben im aktiven Wissensstand. Zwei alte Bots-Quellen sind daraus entfernt und ihre alten Regeln zurückgenommen. Aktiver Stand: maintenance-retire-588d128199fec8a7c988e0ff6bf68f13d33cb8f1a2fcbbc7c2584439fd878cfb. Inhaltlich geprüftes /readyz: application/json, ready, richtiger Wissensstand und richtige Bindungs-Prüfsumme. Brain-PID wechselte beim eigenen Übergang von 573910 zu 1992533; exe ohne deleted, Fehlerjournal seit 06:24:54 CEST leer, NRestarts 0.

## Quellbäume und Artefakte

Die eigenen zuletzt verwendeten Quellbäume Bots 8e1b8f03, Brain ef2f1f95 und die unveränderte Crypto-Abhängigkeit aus Twitch 3341098f wurden nach Prüfung einschließlich ignorierter Dateien entfernt. Zuvor waren alle eigenen Compiler- und Werkzeugprozesse beendet. Die eigene Abhängigkeitsverknüpfung und das leere Layout wurden ebenfalls entfernt; Git-Registrierungen wurden bereinigt. Keine verbliebenen eigenen Aufgabenbranches oder Worktrees.

Erhaltene Artefakte:

- dl-bot: /home/nathanael/.local/state/concierge-bots-8e1b8f03/target/release/dl-bot. SHA256 85409aac1eaf49e9a91916da19eb3b3fe4bfbee293e3c5487a993c9de3ef66e6. Build 8m 55s, Exit 0. Kein vollständiges installiertes Release-Bundle.
- Rust-Einmalwerkzeug: /home/nathanael/.local/state/concierge-corpus-transition/target/debug/concierge-corpus-retire-once. Quelle und Lock-Datei als historische Aufgabenartefakte unter corpus-retire-rust; ihre absoluten Quellabhängigkeiten wurden nach Gebrauch entfernt.
- Unverändertes Drei-Dokumente-Teilpaket: corpus-public-rest.json; SHA256 1a2bb1c57e4c518753efd39357b55e7094561edb3c0c2986ccf921e00e4e644b. Inhalt und vorhandener Prüfverweis bewahrt; Original-Fünf-Dokumente-Paket unverändert.

## Belege

- Öffentlicher Import bjwycgxhk: Exit 0; /tmp/concierge-20261008-public-import.log.
- Bots-Build bkmx5dnk8: Exit 0; /tmp/concierge-20261008-bots-release.log.
- Werkzeug-Build baliwvn18: Exit 101 wegen fehlendem Nachbarpfad für Cargo-Metadaten. Nach Bereitstellung eines eigenen unveränderten Crypto-Worktrees bestätigte Cargo metadata Exit 0; Build bbxugxcja bestätigte Exit 0.
- Vorprüfung br4zx53mu und Aktivierung b33gcpigo: Exit 0; /tmp/concierge-20261008-corpus-tool-plan.log und corpus-tool-apply.log mit gleichem Präfix.
- Abschließende Entfernung alter Regeln b352xwovo: Exit 0, isolated_errors: []; /tmp/concierge-20261008-corpus-retire-final.log. Bereitschaftsnachweis: /tmp/concierge-20261008-ready-final.json und .headers.

Das vorzeitige Entfernen alter Regeln hatte zuvor readiness 503 ausgelöst. Die Regeln wurden exakt wiederhergestellt, danach der sichere Übergang ausgeführt: neue Dokumentbindungen zuerst behalten, alte Dokumentbindungen aus dem Wissensstand entfernen, aktivieren, erst danach die alten Regeln entfernen. Der abschließende Bereitschaftsnachweis ist nach diesem letzten Schritt erfolgt.

## Abschlussgrenze

Ein freigegebener allgemeiner Bots-Installer wurde nicht gefunden. Root-eigene Release-Verzeichnisse wurden nicht über generisches sudo verändert. Pocket, Haze und Coaching bleiben ohne vollständigen Live-Nachweis nach Installation des neuen Bots-Eingangsfixes. Kein settle --selbst. Details in WORKER-ERGEBNIS.md.
