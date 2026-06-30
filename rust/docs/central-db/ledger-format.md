# Mapping-/Drop-Ledger

Das Ledger hält für jede Spalte einer Alt-SQLite fest, wohin sie in der zentralen Postgres-DB wandert — oder warum sie bewusst nicht übernommen wird. Es ist das reviewbare Artefakt hinter dem Versprechen „kein stiller Datenverlust": Was nicht im Ledger steht, lässt das Verifikations-Gate (`check_mapping_completeness`) die Migration abbrechen, statt die Spalte klammheimlich fallenzulassen. Vor dem ETL-Lauf eines Service wird das Ledger von Hand abgenickt.

## TOML-Struktur

Pro Quell-Tabelle und -Spalte ein Block `[tables.<tabelle>.columns.<spalte>]` mit einem `status`. Zwei Stati sind erlaubt: `mapped` mit `to` (voll qualifiziertes Ziel `schema.tabelle.spalte`) oder `dropped` mit `reason` (Begründung, die im Review trägt). Leere Ziele oder Begründungen, unbekannte Felder (etwa der Tippfehler `table` statt `tables`) und ein komplett leeres Ledger werden schon beim Parsen abgewiesen — ein strukturell falsches Ledger kann die Gates nicht still umgehen.

```toml
[tables.fixture_users.columns.discord_id]
status = "mapped"
to = "core.users.discord_id"

[tables.fixture_users.columns.legacy_note]
status = "dropped"
reason = "SP0 fixture: obsolete note column intentionally dropped after review"
```

## Review-Regeln

- Jede Quell-Spalte braucht genau einen Eintrag — `mapped` oder `dropped`. Eine nicht erfasste Spalte ist ein Stopp, kein stillschweigender Default.
- Ein `dropped` ist eine bewusste Entscheidung mit Begründung, kein Vergessen. Trägt die Spalte Daten, die irgendwo noch gebraucht werden, gehört sie `mapped`.
- Jede `mapped`-Spalte muss im Stichproben-Round-Trip wirklich auftauchen (`check_sample_covers_mapped_columns`). Ein Mapping, das nie am Ziel ankommt, ist so gefährlich wie eine vergessene Spalte.
- Ziel-Pfade werden voll qualifiziert (`schema.tabelle.spalte`) notiert, damit das Review ohne weiteren Kontext nachvollziehbar bleibt.

## Befehle

```bash
cargo test -p dl-central-etl --test verify_teeth
cargo test -p dl-central-etl
cargo clippy -p dl-central-etl --tests -- -D warnings
env -u DATABASE_URL SQLX_OFFLINE=true cargo build --workspace
```

## Fixtures

Zwei Fixtures belegen, dass die Gates Zähne haben: `example-ledger.toml` erfasst alle Spalten vollständig und passiert sämtliche Gates; `missing-column-ledger.toml` lässt bewusst eine Spalte (`legacy_note`) offen und muss `check_mapping_completeness` rot werden lassen. Bleibt der Negativtest grün, ist das Gate kaputt — nicht die Fixture.

- `rust/crates/dl-central-etl/tests/fixtures/example-ledger.toml`
- `rust/crates/dl-central-etl/tests/fixtures/missing-column-ledger.toml`
