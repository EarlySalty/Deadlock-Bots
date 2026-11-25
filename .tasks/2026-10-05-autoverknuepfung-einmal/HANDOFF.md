# Übergabe an frischen Matcher-Fixer

Stand: `779e701421aee7b4c7cb5e92bdc48c7412d798f5`, sauberer Worktree `/home/nathanael/.worktrees/Deadlock-Bots-autoverknuepfung-einmal-20261005`, Branch `fix/autoverknuepfung-einmal-20261005`. Eigene Vorgängeränderung `558b91514bbada8a509a6f152c21b82701f851df` ist erhalten. Kein Merge, Push oder Deploy erfolgt. Nach dem letzten Gate-Urteil wurde kein Code mehr geändert.

Der neue Fixer übernimmt Matcher und unmittelbaren Notifier-Port einschließlich `rust/crates/dl-bridges/src/glue.rs`. Die bisherige Worker-Sitzung bleibt für Integration und Deploy zuständig. Du bist nicht allein im Codebestand. Fremde Änderungen im Hauptcheckout niemals zurücksetzen, staschen, übernehmen oder ausliefern. Alle Texte natürlich deutsch mit echten Umlauten, humanizer und no-em-dashes anwenden.

## Verbindlicher offener Fund

Eigene Gate-Nachprüfung gegen `779e7014`, Basis `dd58c0af`, Exit 1:

- `rust/crates/dl-bridges/src/matcher.rs:969`: Review wird vor dem Discord-Aufruf als `delivered=true` gespeichert. Ein Prozessabbruch nach Speichern und vor Versand lässt einen geschlossenen Review ohne Nachricht zurück. `deliver_saved_prompts()` überspringt diesen Datensatz anschließend.
- Unabhängiger Rust-/Intent-Kritiker bestätigt denselben Mechanismus auch in `post_manual_link_prompt` und `deliver_saved_prompts`. Seine vollständige Fundliste kommt von der Hauptsession.
- Das erste Gate blockte zusätzlich bloße Dateinamen, deren leerer Elternpfad das Speichern scheitern ließ, und verlorene Vorschläge bei Schreibfehlern. Der Dateiname-Fund wurde in der Nachprüfung ausdrücklich als FIXED anerkannt. Die Versandstrecke bleibt offen. Der damalige Rescan-Race-Hinweis ist durch denselben Scan-Lock beim Admin-Reset korrigiert.

Ziel bleibt ein automatischer fachlicher Versuch je Twitch-ID, auch nach Neustart und Namensänderung. Technische Fehler dürfen keinen dauerhaften fachlichen Negativentscheid erzeugen. Negative Entscheidungen und bestehende manuelle Eingaben bleiben dauerhaft erhalten. Keine KI-Aufrufe, keine Modellanbindung und keine täglichen Wiederholungen. Mehrdeutige Namen brauchen Bestätigung.

## Möglicher Lösungsweg, noch nicht gebaut

Die Versanddaten sind bereits dauerhaft gespeichert. Eine wiederholbare offene Versandphase könnte vor erneutem Versand über den vorhandenen Notifier die Discord-Kanalhistorie prüfen. Die vorhandenen Buttons tragen schon eindeutige `custom_id`-Werte: Review verwendet den gespeicherten Token, manuelle Eingabe den Login. So ließe sich ein nach dem tatsächlichen Versand abgebrochener Vorgang wiederfinden, während ein vor Versand abgebrochener Vorgang nachgeholt würde. Historienfehler müssen den Versand sperren. Keine neue sichtbare technische Kennung, kein neuer Community-Text und keine zusätzlichen LLM-Kosten. Die genaue Umsetzung entscheidet der frische Fixer anhand der vollständigen Befundliste.

Nur `delivered=true` hinter den Aufruf zu verschieben, lässt nach einem Abbruch zwischen Versand und Speicheraktualisierung eine doppelte Nachricht zu. Der Nutzer verlangt ebenfalls Schutz vor wiederholten Meldungen.

## Bereits gebaut

- Dauerhafte `attempted_ids` im bestehenden JSON nach echter Twitch-ID, mit wiederholbarer Phase `evaluating` und wiederholbarem automatischen Schreibversuch `retry_auto`.
- Übernahme belegter alter `processed`, `pending` und `manual_pending`.
- Speicherprüfung vor dem Scan, Fehler beim Laden sperren Speichern und automatische Entscheidungen, fsync auf Datei und Verzeichnis sowie atomisches Umbenennen.
- Kostenloser vorhandener Ratcliff/Obershelp-Vergleich, automatischer Treffer nur bei eindeutigem exaktem normalisiertem Namen. `dl-bot` baut ausschließlich `NoAi`; andere zentrale KI-Konfiguration blieb unverändert.
- Vollständiger Discord-REST-Mitgliederabruf, Seiten mit höchstens 1000 Einträgen. Fehler, fehlender Fortschritt und leerer Gesamtbestand liefern keine entscheidbare Mitgliederliste.
- Startscan wartet auf Gateway-Guild-Bereitschaft. Der bestehende Loop meldet leere erfolgreiche Läufe nicht und protokolliert ausschließlich Zähler und den Zustandspfad.
- Admin-Neuversuch löst aktuelle Twitch-ID über bestehende Kandidaten-API auf und verwendet den Scan-Lock.

## Prüfstand

19 Matcher-Tests grün. `cargo check -p dl-bot --jobs 3` grün. Eine bestehende dl-brain-Warnung über `fetch_update` bleibt unverändert. Gezieltes `rustfmt --check` für matcher.rs, glue.rs und main.rs grün. `git diff --check` grün.

Moderne Cargo-Binary: `/home/nathanael/.cargo/bin/cargo`. System-Cargo unter `/usr/bin/cargo` versteht die Lockfile-Version 4 nicht. Cargo-Prüfungen mit einem freien Host-Buildslot und `--jobs 3`; Release zusätzlich exklusiv `/tmp/deadlock-cargo-release.lock` und `--jobs 4`. Regeln stehen in Documents/.tasks/2026-10-02-offene-branches/HOSTPROBE.md. Keine globale Formatbereinigung, Baseline hat fremde Formatabweichungen.

## Live-Bestand und späterer Deploy

Live-Unit: `deadlock-bot-rust.service`, WorkingDirectory `/opt/deadlock/bots/current`, derzeit Release `dd58c0afd4f410ae5624b669137d4ef46d859079`. Releasewurzel gehört root, `data/` existiert dort nicht. Matcher-Pfad ist in der normalen Betriebskonfiguration bislang nicht gesetzt. Der relative Standardpfad erklärt die bislang unbemerkt verlorenen Zustände.

Vorhandener dauerhafter Speicher: `/home/nathanael/repos/Deadlock-Bots/data/streamer_link_state.json`, Eigentümer nathanael. 20 abgeschlossene Fälle und sechs offene manuelle Eingaben, keine offenen Reviews. Keine Quelle löschen. Beim Deploy `runtime.bridges.matcher_state_path` in `/home/nathanael/.config/deadlock-bots/bot.toml` genau auf diesen Pfad setzen. Hauptsession hat das freigegeben. Noch nicht geändert.

Origin/main und Live sind `dd58c0af`. Lokales main im schmutzigen fremden Hauptcheckout steht auf `dbda52b8`; es darf nicht umgesetzt werden. Autorisierter Merge-Weg: eigener Integrationsbranch ab aktuellem origin/main, Fixbranch lokal mergen, vollständigen End-SHA prüfen, regulär HEAD:main pushen. Kein Force-Push, keine PR neu anlegen. Danach sauberen Release bauen, Bot neu starten und tatsächlichen Startscan plus zweiten Neustart messen. Beide eigenen Branches und Worktree am Ende löschen. Geheimnisse niemals lesen oder ausgeben, keine ENV-Konfiguration.

## Abschließender Stand der frischen Fixrunde

Aktueller sauberer SHA: `a7418145dd07af2654a30fd21c3d60950f88d832`, auf demselben Worktree und Branch. Nachfolger `bfaf4391` und `a7418145` sind committed; kein Amend, Rebase, Merge, Push oder Deploy durch den Fixer.

76 Bibliothekstests grün, davon 28 Matcher-Tests und ein Glue-Test. `cargo check -p dl-bot --jobs 3` grün, ausschließlich bestehende dl-brain-Warnung. Gezieltes rustfmt und vollständiger Diff-Check gegen `dd58c0af` grün. Einzelheiten in FIX-BERICHT.md.

Die ursprüngliche Versandlücke und der Namenswechsel-Abschluss sind korrigiert. Die erste eigene Nachprüfung auf `bfaf4391` blockte zusätzlich die fehlende Member-Reservierung bei ungesendetem Review sowie die Vererbung fremder Twitch-ID-Daten über einen Login. Die Nachprüfung auf `a7418145` erklärt beide Funde ausdrücklich als FIXED.

Offener bestätigter Gate-Fund: `matcher.rs:967`, ein neuer negativer Versuch unter einem wiederverwendeten Login überschreibt den noch offenen manuellen Auftrag des älteren Accounts. Dieser ältere Account bleibt ohne benutzbaren Auftrag, weil seine Twitch-ID bereits abgeschlossen ist. Die spätere Lösung muss beide ID-bezogenen manuellen Aufträge samt ursprünglichen Buttons erhalten.

Gate-Deckel tatsächlich erreicht: vier BLOCK-Runden. Keine weitere Nachprüfung und keine Gate-State-Manipulation. Die Hauptsession hat zusätzliche Codeänderungen sowie Merge und Deploy ausdrücklich gestoppt. Branch und Worktree nicht löschen. Die Hauptsession nutzt den vom Nutzer erlaubten Live-Rückfall: automatische Suche deaktivieren. Der Integrator liefert dessen eigenen Live-Nachweis. Der Codefix bleibt für einen später konkret freizugebenden Fortgang erhalten und ist nicht mergefähig.
