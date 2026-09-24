# Status: Brain Bild-Eingabe

## Umfang und Verhalten

Die vom Nutzer ausdrücklich freigegebene Vorarbeit im Branch `feat/brain-image-input-20260924` wurde übernommen und erweitert. Ausgangsstand ist `ff635f7b354cb09909c01ddd6f773d0682dd89c9` im Repository `EarlySalty/Deadlock-Bots`.

`/brain` erhält ein optionales Feld `bild` für einen Discord-Anhang in PNG, JPEG oder WebP mit 1 Byte bis einschließlich 8 MiB. Frage und Bildbeobachtung bleiben getrennt. Bildfragen liefern eine Beratung und veröffentlichen keinen In-Game-Build. Der bestehende Textpfad bleibt bestehen. Es wurde kein Modell gewechselt; der vorhandene Bildanalyse-Client wird wiederverwendet.

Der Download ist auf Discord-Attachment-Pfade beschränkt, folgt keinen Weiterleitungen, prüft Header und Dateisignatur und begrenzt die Bytes beim Lesen. Fehler werden nicht in bildlose Antworten umgewandelt. Bildkontext und Bildantwort werden im zentralen Inhaltsprotokoll ausgeblendet; Modell, Laufzeit und Erfolg beziehungsweise Fehlerstatus bleiben erhalten.

## Prüfungen

Testumgebung: eigener Worktree, Rust 1.97.1, `--locked -j 2`, leere Produktionszugänge, `SQLX_OFFLINE=true`, lokale HTTP-Fixtures und KI-Ersatzantworten. Kein Produktionsdatenbankzugriff, keine In-Game-Veröffentlichung, keine Discord-Nachricht aus Tests.

Finaler lokaler Lauf Round6 (`/tmp/brain-image-verify-20260924-round6`):

| Prüfung | Ergebnis |
|---|---|
| `cargo +1.97.1 test --locked -j 2 -p dl-ai -p dl-brain -p dl-answer -p dl-discord --lib` | Exit 0; 126 + 8 + 32 + 51 Tests bestanden; 0 fehlgeschlagen/ignoriert |
| `cargo +1.97.1 test --locked -j 2 -p dl-bot modglue::tests::brain_` | Exit 0; 17 bestanden, 0 fehlgeschlagen/ignoriert, 267 andere Tests durch den gezielten Filter nicht ausgeführt |
| `cargo +1.97.1 clippy --locked -j 2 -p dl-ai -p dl-brain -p dl-answer -p dl-discord -p dl-bot --all-targets --no-deps -- -D warnings` | Exit 0 |
| Rustfmt 1.97.1, `--check --edition 2021 --config skip_children=true`, explizit acht geänderte Quelldateien | Exit 0 |

Die Testauswahl umfasst 234 bestandene Tests, keine vollständige Workspace-Abnahme. Neue Test-`unwrap`-Aufrufe wurden durch konkrete `expect`-Meldungen ersetzt. Der bisherige absteigende Namenslängenvergleich im Brain-Emoji-Index verwendet für Clippy jetzt `sort_by_key` mit `Reverse`; die Sortierreihenfolge bleibt erhalten. Keine Lint-Unterdrückung. Verifizierter und gepushter Quellcommit: `ad5859a8e7600ff7df0cfdfbbbe8a6ee4b39a340`. Die anschließende Abnahme-Ablage verändert keine Quelldateien. Der jeweilige vollständige PR-Head und seine aktuellen Checks werden zusätzlich im PR-Bericht festgehalten.

Negative Gegenproben umfassen fremde Hosts, Userinfo, Ports, HTTP, falsche Pfade, nicht unterstützte Dateitypen, Nullgröße, Übergröße mit und ohne Content-Length, Redirects, defekte Signaturen, fehlende Attachment-Auflösung, fehlende Bildanalyse, Bildtext mit Publish-Aufforderung und Inhaltsprotokollierung. Die Build-Gegenprobe verwendet einen absichtlich nicht vorhandenen Prozesspfad, während die Bildberatung erfolgreich antwortet.

Das unabhängige Review hat nach seiner finalen Delta-Prüfung ALLOW gemeldet, einschließlich Datenschutzänderung, korrigierter Test-Fixtures und vollständig grünem Round6. Der Bericht liegt in REVIEW-EXTERN.md; der Review-Thread ist beendet. Der zuerst gestartete Coding-Thread hatte einen abgelaufenen OAuth-Zugang; der erste Review-Thread hatte kein Kontingent. Beide haben keinen Code und kein negatives Urteil erzeugt und wurden beendet. Details in REGISTER.md.

## Offene Abnahmen

Der alte Fehler aus dem Screenshot beim reinen Mo-&-Krill-Buildwunsch ist nicht als behoben nachgewiesen. Der bisherige Text-Buildpfad startet die Brain-CLI und fasst dessen Prozess-/Timeout-/Antwortfehler in einer generischen Meldung zusammen. Eine sichere Laufzeitdiagnose konnte hier nicht erhoben werden; die Meldung allein belegt keine bestimmte Ursache. Der neue Bildpfad ist davon getrennt. Kein Produktionsaufruf wurde zum Nachstellen ausgelöst.

## GitHub-Abnahme

PR: https://github.com/EarlySalty/Deadlock-Bots/pull/450, offen, kein Auto-Merge. Gepushter Quell-Head: `ad5859a8e7600ff7df0cfdfbbbe8a6ee4b39a340`; synthetischer GitHub-Merge-Ref dieses Stands: `f115c813ffa857daea730f2bf26c5f30e005176d`. Dieser Merge-Ref ist kein ausgeführter Test und kein Merge nach main.

| Workflow | Lauf, Versuch 1 | Ergebnis |
|---|---|---|
| Discord config CI | https://github.com/EarlySalty/Deadlock-Bots/actions/runs/35947236066 | failure, Job nicht gestartet |
| dl-knowledge eval CI | https://github.com/EarlySalty/Deadlock-Bots/actions/runs/35947236023 | failure, Job nicht gestartet |
| dl-knowledge hybrid CI | https://github.com/EarlySalty/Deadlock-Bots/actions/runs/35947236073 | failure, Job nicht gestartet |

Die Annotationen der drei tatsächlichen Check-Runs `107467810494`, `107467810206` und `107467810643` wurden einzeln per GitHub-API gelesen. GitHub nennt bei jedem Lauf fehlgeschlagene Kontozahlungen oder ein zu erhöhendes Ausgabenlimit als Startblocker. Damit hat auf GitHub kein funktionaler Test diesen Stand ausgeführt. Die Abrechnung wurde nicht verändert. Abhilfe: Kontoinhaber prüft Billing und Plans beziehungsweise das bestehende Ausgabenlimit; danach dieselben PR-Läufe erneut ausführen und deren Ergebnisse auswerten. Kein Blind-Rerun ohne veränderte Voraussetzung.

Derselbe Startblocker ist bereits für den unveränderten Basiscommit `ff635f7b354cb09909c01ddd6f773d0682dd89c9` im main-Lauf https://github.com/EarlySalty/Deadlock-Bots/actions/runs/35820441759 nachgewiesen (Check-Run `107050931281`). Das ist ein Kontoblocker, kein durch die Bildänderung nachgewiesener Testfehler.

GitGuardian Security Checks meldet für den Quell-Head success. Dieser Einzelcheck ersetzt keine funktionale CI-Abnahme. Im derzeitigen Basisstand existiert außerdem kein `Required PR Gate`; der separate CI-Umbau bleibt im Besitz seiner Repo-Session. Ein fehlender Gate-Check wird nicht als grüne Gesamtabnahme gewertet. Nach der reinen Dokumentationsablage werden die neuen PR-Head-Runs ebenfalls nachgelesen und im PR-Text mit vollständiger SHA-Zuordnung dokumentiert.

MERGEPROTOKOLL[MS-1]: kein Merge: PR-first-Testbetrieb
LIVEBEWEIS[DV-1]: nicht ausgeführt: PR-first-Testbetrieb
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: private Task-Akte und repo-nahe Nutzungsdoku
