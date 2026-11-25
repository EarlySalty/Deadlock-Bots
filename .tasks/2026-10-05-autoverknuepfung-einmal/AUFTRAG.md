# Autoverknüpfung einmal je Twitch-ID

ORCHESTRIERUNG[OR-1]: Stufe mittel | Schritt dispatch | Artefakt: /home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-05-autoverknuepfung-einmal/AUFTRAG.md

BRIEFING[WB-1]: Pflichtteile 5/5 | Timer 20 min | Worktree: durch Bereichsführung anzulegen

[Orchestrator] Du bist Teil-Orchestrator für diesen einen Bereich. Hauptauftraggeber ist die aktive Codex-Hauptsession im Nutzerthread, Kennung root-autoverknuepfung-20261005. Native Subagenten für Recherche, Rust-Bau und unabhängige Abnahme sind ausdrücklich erlaubt. Keine zusätzlichen T3-Threads ohne Rückmeldung.

## Ziel und Vertrag

Der Nutzer zeigt täglich wiederkehrende Discord-Nachrichten „Kein Discord-Match“ mit manueller Eingabe und KI-Transparenz „Streamer-Matcher“. Er fordert automatische Verknüpfung nur einmal je Person, keine täglichen Wiederholungen und keine wiederholten Tokenkosten bei weiterhin unverknüpften Accounts. Er fragt nach „Jav“ für eine einfache Ja/Nein-Entscheidung. Die Hauptsession hat optional gefragt, ob damit Jaro-Winkler gemeint ist; diese Bedeutung ist noch unbestätigt. Prüfe den bereits vorhandenen deterministischen Namensvergleich und ob die Matcher-KI vollständig entfallen kann. Keine neuen oder anderen LLM-Modelle einbauen. Wenn der Nutzer einen anderen Algorithmus meint, gibt die Hauptsession das nach.

Einmaligkeit dauerhaft nach Twitch-Plattform-ID, auch für negative Ergebnisse und über Neustarts hinweg. Vor KI-Aufruf und Meldung atomar/dauerhaft sichern, soweit der vorhandene Vertrag es verlangt. Bereits früher vergeblich geprüfte Accounts sollen nicht wieder massenhaft Meldungen erzeugen. Altbestand mit belegten bestehenden Daten übernehmen, keine Tatsachen erfinden. Technische Ausfälle nicht als fachliches Nein speichern. Manuelle Verknüpfung und bewusster Admin-Eingriff bleiben benutzbar. Mehrdeutige ähnliche Namen dürfen nicht blind als gesicherte Identität verknüpft werden. Grenze eigene Ursachenanalyse auf diesen Matcher und dessen Speicherung, Laufzeit und Live-Pfad ein.

## Bestand und Eigentum

Repo /home/nathanael/repos/Deadlock-Bots, Ausgangs-HEAD dbda52b81cf8e68ea2aa7351a0e7ef2df5c81dbc. Im Hauptcheckout gibt es zahlreiche fremde uncommittierte Änderungen. Du bist nicht allein im Codebestand. Niemals fremde Änderungen zurücksetzen, staschen oder mit ausliefern. Eigenen Worktree und Branch ab aktuellem origin/main anlegen, Eigentum am Matcher rust/crates/dl-bridges/src/matcher.rs und den notwendigen eigenen Speicher-/Migrations-/Wiring-Dateien. Erst Pfade melden, wenn Erweiterungen nötig werden. Keine allgemeinen Refactorings.

Vorcheck: graphify query lief in diesem Repo und zeigt Matcher in rust/crates/dl-bridges/src/matcher.rs, run_scan_inner ungefähr L492. Meldung ungefähr L869. Start-/Anbieterwahl in rust/bin/dl-bot/src/main.rs ungefähr L592. Bestehender Graph liegt in graphify-out, Abfrage war wegen Budget gekürzt. Nutze ihn gezielt und prüfe den wirklichen Code sowie Live-Zustand. Produktiver Code ausschließlich Rust. Falls AGENTS.md fehlt, lokale CLAUDE.md und übergeordnete Anweisungen berücksichtigen.

## Beweis und Abschluss

Besitzt Bau, Datenaufnahme, Integration und Deploy dieses Bereichs. Commit, regulärer Merge und Push sind autorisiert, kein Force-Push. Nutzer hat Merge und Deploy nach gültiger Abnahme und Gate ohne Rückfrage erlaubt. Vor dem Merge unabhängige Intent-Abnahme durch frischen nativen Kritiker und danach Bug-/Security-Gate. Rust-Reviewer für Rust-Änderungen einsetzen. Fixer prüfen eigene Änderungen zusätzlich mit gate_hook.py --review. Vor Deploy Report mit SHA und Urteilen an Hauptsession; bei eindeutiger Erfüllung darfst du sofort vollständig abschließen.

Pflichtbeweise: dieselbe Twitch-ID löst bei zweitem Scan und nach Neustart weder KI-Aufruf noch erneute Negativmeldung aus; erstmaliger negativer Versuch wird gemerkt; manuelle Eingabe funktioniert; Fehlerbehandlung und Mehrdeutigkeit bleiben korrekt. Bestehende passende Prüfungen ausführen, keine künstliche Testpflicht. Schemaänderungen im eigenen Migrator nach Merge und vor Code-Deploy anwenden. Geheimnisse nie lesen/ausgeben/schreiben, keine ENV-Dateien oder Konfiguration über Environment. Infisical-Zugänge nur durch vorhandene sichere Werkzeuge. Nur ein Cargo-Release-Build gleichzeitig auf dem Host. Discord-Bot über tatsächliche systemctl --user Unit neu starten und den Live-Pfad messen. Worktree und Branch nach Merge und Live-Beweis löschen.

Referenz /home/nathanael/Documents/Docs/workspace/gates-und-merge.md. Keine Community-Nachricht veröffentlichen. Alle eigenen Texte, Kommentare, Berichte und Subagentenaufträge mit humanizer und no-em-dashes prüfen, natürliche deutsche Texte mit echten Umlauten und ohne Gedankenstriche.

## Routing

Berichte in diesem Aufgabenordner in BERICHT.md, Start- und Zwischenzustand in STATUS.md. Hauptsession pflegt REGISTER.md. Melde früh Ursache, deterministischen Algorithmus, geplante Dateien, Worktree und Branch. Bei Blockade konkreten Zustand und mögliche Lösung melden. Abschluss nennt Commit, unabhängiges Urteil, Gate, Deploy-Ziel, tatsächliche Live-Nachweise sowie verbleibende Grenzen. Hauptsession wartet aktiv und übernimmt Rückfragen. Eigene Session nicht vor gelesenem Abschluss settlen.
