# Unabhängige Abnahme des integrierten Endstands

Geprüfter SHA: `bfaf4391630f96268aef4a2f7f4cfffeee929c60`.

Basis: `dd58c0afd4f410ae5624b669137d4ef46d859079`.

Fertig: ja, bezogen auf die Codeänderung. Fix nötig: nein. Keine belegten offenen Fehler im geprüften Auftrag. Merge-Gate und produktiver Nachweis bleiben eigenständige Abschlussbedingungen. Dieses Urteil gilt ausschließlich für den genannten SHA.

## Verbindliche Funde aus der vorherigen Abnahme

B1 ist behoben. `Matcher::deliver_prompt` in `rust/crates/dl-bridges/src/matcher.rs:855` setzt den dauerhaften Versandabschluss erst nach dem Zustellbeleg. Ein fehlgeschlagenes Save stellt den vorherigen offenen Datensatz auch im Arbeitsspeicher wieder her. Die Vorbereitung eines manuellen Auftrags stellt bei Speicherfehlern ebenfalls den vorherigen Zustand wieder her. Review und manuelle Eingabe verwenden beim ersten Versand und bei der Wiederaufnahme denselben Weg.

`AdapterGlue::notify_once` in `rust/crates/dl-bridges/src/glue.rs:131` ermittelt die eigene Bot-ID über REST. Der Abgleich liest die Kanalhistorie bis zum Ende in Seiten zu 100 Nachrichten und prüft Autor sowie exakte Button-Kennung. Fortschrittsprüfung und Fehlerbehandlung verhindern einen Versand bei unvollständigem Abgleich. Der bestehende Adapter verwendet tatsächlich `MessagePagination::Before` und ein Limit von 100. Der Speicherbedarf für die Historie bleibt auf eine Seite beschränkt. Eine bereits gesendete Nachricht mit noch offenem lokalem Auftrag wird über Neustarts hinweg gefunden. Die zusätzliche stabile Nonce deckt kurze API-Verzögerungen ab. Ein Scan-Lock verhindert konkurrierende automatische Versandläufe.

B2 ist behoben. `LinkState::claim` in `rust/crates/dl-bridges/src/matcher.rs:330` speichert einen Loginwechsel bei wiederholbaren Versuchen vor deren Fortsetzung und setzt ihn bei Speicherfehler zurück. Alle automatischen Abschlusswege tragen die Twitch-ID ausdrücklich. `LinkState::mark` schließt bei vorhandener ID genau diesen Versuch. Ein anderer Eintrag mit gleichem Login wird dadurch nicht versehentlich abgeschlossen.

## Geprüfter Nutzer-Intent

Der automatische Scan verwendet ausschließlich den vorhandenen deterministischen Namensvergleich. Er ruft keinen KI-Scorer auf; das Startup erzeugt `NoAi`. Nur ein eindeutiger exakter normalisierter Treffer kann automatisch verknüpft werden. Mehrdeutige und ähnliche Namen bleiben Vorschläge zur manuellen Bestätigung.

Negative Ergebnisse und Vorschläge schließen den automatischen Versuch dauerhaft nach Twitch-ID. Eine offene Zustellung wird später nachgeholt, ohne die fachliche Entscheidung zu wiederholen. Technische Abruf- und Speicherfehler erzeugen kein fachliches Nein. Der vollständige REST-Mitgliederabruf und das Warten auf `CacheReady` sind im integrierten Diff enthalten. Die Gateway-Subscription wird vor dem Start der Gateway-Clients eingerichtet.

Alte `processed`, `pending` und `manual_pending` bleiben erhalten. Alte manuelle Nachrichten ohne neues `delivered`-Feld werden nicht erneut versandt; ihre Button- und Modalpfade bleiben registriert. Die Übernahme vorhandener Fälle schreibt den ersten belegten Zusammenhang mit der Twitch-ID dauerhaft. Bewusste Admin-Neuversuche bleiben möglich und verwenden denselben Scan-Lock.

Die historische Zustandsdatei enthält weiterhin 20 abgeschlossene Fälle, sechs manuelle Eingaben und keine Reviews. Diese alten Einträge enthalten überwiegend keine Twitch-ID; ihre Übernahme verwendet deshalb den belegten alten Login, sobald die Kandidaten-API den Zusammenhang mit der aktuellen ID liefert. Bereits vor der Übernahme erfolgte historische Umbenennungen lassen sich aus dieser Datei allein nicht beweisen. Es werden keine Identitäten erfunden.

## Nachweise und Grenzen

- Den integrierten Diff aller drei geänderten Dateien und die zugehörigen Speicher-, Versand-, Historien-, Button-, Admin- und Startup-Pfade selbst gelesen.
- `/home/nathanael/.cargo/bin/rustfmt --edition 2021 --check` selbst gezielt für `matcher.rs`, `glue.rs` und `dl-bot/src/main.rs` ausgeführt: Exit 0.
- Vollständiges `git diff --check` gegen die genannte Basis selbst ausgeführt: Exit 0. HEAD bestätigt, Worktree sauber; produktive Dateien unverändert.
- Aktuelle Ausführungsnachweise des Fixers gelesen: `cargo test -p dl-bridges --lib --jobs 3`, Exit 0, 71 Tests, davon 23 Matcher-Tests und ein Glue-Test. Die neuen Tests und ihre tatsächlichen Zustandsprüfungen selbst gelesen. Sie decken offene Zustellung, unterbrochenen Versand, Historienfehler, Speicherfehler und umbenannte wiederholbare Versuche ab.
- Aktueller Ausführungsnachweis des Fixers: `/home/nathanael/.cargo/bin/cargo check -p dl-bot --jobs 3`, Exit 0; nur bestehende `dl-brain`-Deprecation zu `fetch_update`. Keine unnötige Wiederholung der Suite oder des Builds durch den Kritiker.

Der Discord-Abgleich wurde am tatsächlichen Adaptercode geprüft. Die Tests simulieren den Notifier; dieser Bericht behauptet keinen echten Discord-Versand und keine Live-Messung der REST-Pagination. Eine vom Betreiber gelöschte Nachricht kann der Historienabgleich nicht wiederfinden.

Vor dem Deploy muss die normale Config `runtime.bridges.matcher_state_path` auf `/home/nathanael/repos/Deadlock-Bots/data/streamer_link_state.json` zeigen. Der vorhandene Altbestand darf dabei nicht ersetzt werden. Startscan, weiterer Scan und Neustart müssen anschließend live belegen, dass alte Fälle erhalten bleiben und keine erneuten negativen Meldungen oder Matcher-KI-Aufrufe entstehen. Der Kritiker hat weder Config noch Services geändert und keinen Merge oder Deploy ausgeführt. Das eigene Bug-/Security-Gate des Fixers läuft zum Berichtszeitpunkt noch.
