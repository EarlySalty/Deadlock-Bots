status: aktiv (2026-10-02)

# Laufender Delegator-Stand

Dieser Stand dient der Fortsetzung desselben Auftrags. Kein zweiter Delegator starten.
Aktueller Delegator: 011b713c-266a-46b6-b0ce-1d5faa060234.
Intent: T3-ID 2be5e1b8-b1c3-4d67-85e1-9eafc137121b, Session-Kennung b61e3506.

## Aktuell

Paket A läuft in d99950e0-3654-4d2d-aa68-4834c84988b3 mit gpt-6.1-sol.
Worker hat den Worktree angelegt und origin/main bei 47ea7994 frisch geholt.
Aktueller Befund: core.steam_links hat den Schlüssel (discord_id, steam_id),
steam_id ist Text. Pool-Schreibvorgänge sollen die vorhandenen Datenschutz-Locks
und Opt-out-Grabsteine beachten.
Migration `20261002000000_spielerpool.sql`, Rust-API, Cargo-Verdrahtung und
Datenschutz-Löschpfad sind angelegt. Nach einem Fehler des veralteten
Standard-Cargo prüft A mit aktueller Toolchain und isoliertem PostgreSQL.
Gate-Urteil, Merge und Live-Nachweis stehen noch aus.
VERTRAG-A.md liegt jetzt vor und wurde vom Delegator gelesen. 13 ursprüngliche
DB-Integrationstests bestanden. Auf Delegator-Hinweis hat A `own_sessions`
für dauerhaftes eigenes Website-Feedback ergänzt und im Vertrag dokumentiert.
Jetzt 14 Pool-Tests und drei neue privacy.rs-Tests grün, Pool-Clippy grün.
Privacy-Schemavertrag scheitert an elf fremden Twitch-/Patchnotes-Spalten;
exakt derselbe Fehler auf unverändertem 47ea7994 nachgewiesen. Nicht nebenbei
fixen und Gate nicht umgehen. Community-Lint läuft, danach einziger Gate-Review.

Der erste A-Start mit Grok b7093798 scheiterte vor Bau mit 402 wegen leerem
Guthaben; ist gesettelt und wird nicht wieder aufgenommen. Auch der alte
Delegator 8f38f31a wird nicht wieder aufgenommen.

## Modellwahl

Orchestrator hat ausdrücklich klargestellt: Sol direkt starten, historische
Reset-Sperren nicht als aktuellen Laufzeitbeweis behandeln. Falls `new --rolle`
wegen des alten Log-Signals schon vor Threadanlage scheitert, mit zusätzlichem
`--model sol` starten. Keine Sperren speichern oder verändern.
Nur bei echtem Usage-Limit im laufenden Sol-Thread ist opus55 als Ausnahme
freigegeben. Grok nicht mehr nutzen. `send` nie mit `--model`.

## Nächste Schritte

1. A bis Gate und Merge begleiten. Bei BLOCK frischen Fixer nach `--rolle fixer` starten, Befund und bestehenden Worktree übergeben. Nicht den Implementierer fixen lassen.
2. Nach A-Merge VERTRAG-A.md lesen und vorbereitete Briefings an tatsächliche Vertragsfelder anpassen. B, C und E als eigene Worker in Deadlock-Bots starten. E-Vertrag früh festlegen lassen.
3. D als eigenen Worker im Projekt Website starten, sobald A gemergt und E-Vertrag feststeht. Es besitzt auch die Caddy-Route im getrennten Repo-Worktree.
4. E vor C mergen; C anschließend rebasen und gemeinsame Verdrahtung erneut durch den Gate prüfen. Alle Deploys und Cargo-Release-Builds koordinieren, höchstens ein solcher Build gleichzeitig.
5. Jeder fertige Worker meldet main-SHA, Live-Nachweis, Ort im UI und tatsächliche Bereinigung von Branch und Worktree. Ungeprüfte Nutzeraktionen in der finalen Testanleitung belassen.
6. Register und Pakete bei Änderungen fortschreiben. Fertige oder ausgefallene Threads settlen. Nach Fertigmeldung allerletzter eigener Schritt `settle --selbst`.

## Umfang

Der Delegator bearbeitet ausschließlich Aufgabenartefakte, keinen produktiven Code.
Geteiltes Deadlock-Bots-Checkout enthält fremde Änderungen; nicht anfassen.
Testbetrieb begrenzt, Interview nicht ungefragt für alle Mitglieder aktivieren.
Kein Bot-Verhalten in #mitspieler-suche bauen. Kein Steam-Bot ändern.
Gesamtauftrag noch nicht fertig, gemergt oder live geprüft.
