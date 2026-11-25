# Fix der offenen Review-Funde

Commit: `a7418145dd07af2654a30fd21c3d60950f88d832`, über `bfaf4391630f96268aef4a2f7f4cfffeee929c60` Nachfolger von `779e701421aee7b4c7cb5e92bdc48c7412d798f5` auf `fix/autoverknuepfung-einmal-20261005`.

Worktree: `/home/nathanael/.worktrees/Deadlock-Bots-autoverknuepfung-einmal-20261005`.

Verändert sind ausschließlich `rust/crates/dl-bridges/src/matcher.rs` und `rust/crates/dl-bridges/src/glue.rs`. Kein Merge, Push oder Deploy durch diesen Fixer.

## B1: Versand bleibt bis zum belegten Abschluss offen

Review und manuelle Eingabe speichern den Auftrag mit `delivered=false`, einschließlich Embed und Komponenten. Der erste Versand und die Wiederaufnahme benutzen denselben Abschlussweg. Erst nach belegter Zustellung wird `delivered=true` mit Kanal- und Nachrichten-ID gespeichert. Scheitert diese Speicherung, wird auch der Zustand im Arbeitsspeicher auf den offenen Auftrag zurückgesetzt. Scheitert die Vorbereitung der manuellen Eingabe, wird der vorherige Datensatz wiederhergestellt.

Der Discord-Port löst die aktuelle eigene Bot-ID über REST auf und liest die Kanalhistorie vollständig mit Seiten zu 100 Nachrichten. Er sucht ausschließlich eigene Nachrichten mit der exakten gespeicherten Button-Kennung. Eine bereits gesendete Nachricht wird dadurch nach einem Abbruch vor der lokalen Speicherung wiedergefunden. Fehler bei Bot-ID, Historienabruf oder Komponentenlesen sowie fehlender Fortschritt beim Blättern sperren den erneuten Versand. Zusätzlich setzt der Versand eine stabile `nonce` mit `enforce_nonce` für kurze API-Verzögerungen. Der dauerhafte Nachweis kommt aus der Historie, nicht aus deren zeitlich begrenzter Nonce-Prüfung.

Es entstehen keine neuen Community-Texte und keine externen Testnachrichten. Der Abgleich setzt voraus, dass die Nachricht im Discord-Kanal weiterhin vorhanden ist. Eine aktiv gelöschte Nachricht kann die Historie nicht belegen.

## B2: Automatische Ergebnisse schließen die Twitch-ID

Alle fachlichen Abschlüsse im automatischen Scan tragen die aufgenommene Twitch-ID ausdrücklich in den Ergebnisdaten. `LinkState::mark` schließt bei vorhandener ID ausschließlich diesen Versuch. Der Login ist dafür kein Identitätsfilter mehr. Wiederaufnahme eines `evaluating`- oder `retry_auto`-Versuchs schreibt einen geänderten Login vorher dauerhaft nach und setzt ihn bei einem Schreibfehler zurück. Alte manuelle Datensätze ohne ID behalten ihre Buttons. `mark` ohne ID schließt keine ID-Versuche mehr. Neue manuelle Aufträge und Buttons tragen die Twitch-ID; veraltete Buttons zu einem wiederverwendeten Login werden abgewiesen. Review und manuelle Eingabe lösen vor der Verknüpfung den aktuellen Login über die gespeicherte Twitch-ID aus dem vorhandenen Kandidaten-Endpunkt auf. So folgen sie einer Umbenennung, ohne einen inzwischen fremden Namen zu verwenden.

## Weitere eigene Gate-Funde auf bfaf4391

Die erste eigene Nachprüfung lieferte Exit 1 und BLOCK mit zwei weiteren konkreten Funden. Beide sind in `a7418145` gemeinsam behoben:

- Persistierte Reviews reservieren den Discord-Member auch dann, wenn der Versand scheitert. Der Vorschlag bleibt ein gültiges fachliches Ergebnis. Alle offenen Reviews reservieren ihren Member auch in späteren Scans und nach Neustarts.
- ID-tragende Datensätze werden nur derselben Twitch-ID vererbt. Ein Login darf keine nachweislich fremde ID übernehmen. Beim Laden werden bekannte IDs aus `processed`, `manual_pending` und `pending` in den ID-Abschlüssen erhalten, damit das spätere Überschreiben eines alten Logins die frühere Identität nicht löscht. ID-lose Altdaten werden nur übernommen, solange der Login noch keiner anderen ID zugeordnet wurde.

Der Gate-Zustand wurde vor der zweiten Nachprüfung ausschließlich gelesen: `block_rounds=3`, `escalated=false`, Grenze 4. Die weitere reguläre Nachprüfung war zugelassen. Weder Basis noch Branch noch Gate-State wurden geändert oder gelöscht.

## Prüfungen

- `/home/nathanael/.cargo/bin/cargo test -p dl-bridges --lib --jobs 3`: Exit 0, 76 Tests grün, davon 28 Matcher-Tests und ein Glue-Test. Host-Buildslot eingehalten.
- Neue Beweise: Neustart vor dem Versand, Historienfehler ohne Versand, Abbruch nach tatsächlichem Versand vor lokalem Abschluss, Wiederaufnahme ohne doppelte Nachricht, fehlgeschlagener Versandabschluss mit offenem Zustand, fehlgeschlagene manuelle Vorbereitung, Wiederaufnahme von `evaluating` und `retry_auto` nach Namenswechsel ohne Abschluss einer anderen Twitch-ID. Dazu Reservierungen bei fehlgeschlagenem Versand im selben Lauf und nach Neustart, wiederverwendete Logins bei drei Speicherquellen, einmalige Übernahme ID-loser Altdaten, Erhalt einer alten ID nach Überschreiben des Logins und manuelle Verknüpfung nach Umbenennung mit Abweisung alter Buttons.
- `/home/nathanael/.cargo/bin/cargo check -p dl-bot --jobs 3`: auf dem abschließenden SHA `a7418145` Exit 0, Laufzeit 3 Minuten 13 Sekunden. Ausschließlich die vorhandene Deprecation-Warnung zu `fetch_update` in `dl-brain`. Host-Buildslot eingehalten.
- Gezieltes `rustfmt --edition 2021 --check` für Matcher, Glue und Bot-main: Exit 0.
- `git diff --check`: Exit 0.
- Eigenes `gate_hook.py --review` gegen Basis `dd58c0afd4f410ae5624b669137d4ef46d859079`, HEAD `a7418145`: Exit 1, BLOCK. Die beiden Funde auf `bfaf4391` wurden ausdrücklich als FIXED anerkannt; der nachfolgend genannte Fund bleibt offen.

Die unabhängige Abnahme des neuen SHAs und der spätere Live-Nachweis gehören zur Integration durch die Hauptsession. Dieser Bericht beansprucht keine Merge- oder Deploy-Freigabe.

## Abschließender Blocker und verbindlicher Stopp

Das Gate auf `a7418145` benennt `rust/crates/dl-bridges/src/matcher.rs:967`: Ein neuer Account mit wiederverwendetem Login ersetzt im neuen `no_match` noch `manual_pending[login]`. Der vorherige offene manuelle Auftrag geht damit verloren. Die älteren Buttons werden abgewiesen, während die terminale Twitch-ID des älteren Accounts auch nach einer Umbenennung keine neue automatische Prüfung mit Ersatzprompt zulässt.

Der nächste Fix müsste offene manuelle Aufträge verschiedener Twitch-IDs parallel erhalten und die alten Buttons weiterhin auf ihren ursprünglichen Auftrag auflösen. Das wurde nicht mehr geändert oder geprüft.

Das Gate meldet vier aufeinanderfolgende BLOCK-Runden und den verbindlichen Deckel. Die Hauptsession ordnete deshalb ausdrücklich an: keine zusätzlichen Codeänderungen, keine weitere Gate-Nachprüfung, kein State-Reset, kein Merge und kein Deploy dieses Fixes. Der aktuelle Worktree ist sauber auf `a7418145dd07af2654a30fd21c3d60950f88d832`. Branch und Worktree bleiben für eine später ausdrücklich freigegebene Fortsetzung erhalten.

Laut Hauptsession wurde die automatische Suche als vom Nutzer ausdrücklich erlaubter Rückfall live deaktiviert. Der Integrator führt dessen tatsächlichen Live-Nachweis. Dieser Fixer hat keine produktive Konfiguration geändert und behauptet keinen eigenen Live-Beweis. Der Codefix bleibt gate-blockiert und hat keine Merge-Freigabe.
