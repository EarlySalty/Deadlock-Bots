status: aktiv | 2026-10-01

# Scrim Orga Autopilot, Gate Runde 1

- Branch: `codex/luna-dispatch/deadlock-bots/feat-scrim-orga-autopilot-20260914-8be45ee2`
- Head: `8be45ee29b7b5e694fb6457daf0d2623ff7232ff`
- Base: `main` at `dbda52b81cf8e68ea2aa7351a0e7ef2df5c81dbc`
- Gate: `gpt-6.1-sol` returned `BLOCK`
- Scope: `rust/bin/dl-bot/src/scrimglue.rs`

## R1: Batch lock-order inversion can deadlock finalization

`rust/bin/dl-bot/src/scrimglue.rs:3152`

The expiry sweep can retain the batch lock for request A before it locks the batch for request B. The delivery batch update at line 3993 and failure batch update at line 4152 participate in the lock order. The same update paths are called from expiry at line 3152, dead-effect handling at line 4279, and uncertain-effect handling at line 4326. A concurrent transaction can hold B while waiting for the sweep's batch lock, producing a deadlock and aborting finalization.

## R2: Aggregate status checks can use stale request states

`rust/bin/dl-bot/src/scrimglue.rs:3993`

The `EXISTS` check at line 3997 and `NOT EXISTS` check at line 4002 can read request states from a stale statement snapshot. With PostgreSQL `READ COMMITTED`, waiting for the batch row does not refresh the request snapshot. Concurrent final deliveries can leave a fully open batch at `posting`. A delivery racing the failure update at line 4152 can overwrite `post_failed` with `posting`.

No code fix or test/build was performed in this review round. The fixer must address both findings, preserve the source-worktree dashboard WIP, and rerun the local gate against the resulting branch SHA. Heavy Cargo checks remain subject to the coordinator's serialized host-resource hold.

## Runde 2, Fix-Stand 2026-10-01

- Lock-Reihenfolge angepasst: Batch wird vor Requests gesperrt, auch in `save_match_request_posts`; der Lease-Sweep sperrt alle betroffenen Batches sortiert.
- Batch-Aggregat wird erst nach dem Batch-Lock und den Request-Updates in einem eigenen Statement neu berechnet.
- Zwei Concurrency-Regressionstests hinzugefügt.
- Verifikation blockiert: Host-Lock war zunächst belegt. Beim späteren Versuch war er frei, aber installiertes Cargo 1.75 kann `Cargo.lock` Version 4 nicht lesen; `rustfmt` ist nicht installiert.
- Zwei Gate-Aufrufe vor dem Fix-Commit prüften den Commit-HEAD `2a00f7cc`, nicht die uncommitteten Änderungen; sie zählen nicht als Patch-Abnahme.
- Zwischen-Gate nach Sicherung des vollständigen Patches: `gpt-6.1-sol ALLOW`; die zwei ursprünglichen Fix-Funde wurden behoben bewertet.
- Folge-Gate fand einen fehlerhaften Failure-Test und unsynchronisierte Warteprüfungen. Der Test aktualisiert nun ein Request-Feld ohne Statuswechsel und wartet explizit auf den PostgreSQL-Lock-Wait.
- Gate auf dem korrigierten Branch-Head: `gpt-6.1-sol ALLOW`; der Test-Fund wurde als behoben bewertet.
- Folge-Gate fand fehlende Batch-Ziele, die den Expiry-Sweep abbrechen konnten. Failure-Bookkeeping überspringt nun fehlende Batches; ein Regressionstest prüft, dass der Sweep weitere Effekte verarbeitet. Gate: `gpt-6.1-sol ALLOW`.
- Weitere Lock-Order-Lücke bei Status-Effekten geschlossen: Lease-Sweep sammelt und sperrt die betroffenen Request-Batches sortiert; direkter Fehlerpfad sperrt vor dem Request. Concurrency-Test ergänzt. Gate: `gpt-6.1-sol ALLOW`.
- Tests bleiben unausgeführt: beim Versuch war der Host-Lock frei, aber Cargo 1.75 kann `Cargo.lock` Version 4 nicht lesen. `rustfmt` ist nicht installiert.
- Migrationsvertrag geprüft: keine DDL-Änderung; benötigte Scrim-Tabellen und Statuswerte liegen bereits in `dl-central-db`-Migrationen.
