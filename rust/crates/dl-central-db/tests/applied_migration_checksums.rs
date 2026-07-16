use sha2::{Digest, Sha384};

// Dieser Wert stammt aus der erfolgreich angewendeten Produktionsmigration.
// Die historische SQL-Datei muss bytegleich bleiben; Vertragsaenderungen
// gehoeren in eine neue Migration.
const SCRIM_TEAM_DEFAULT_TIME_SHA384: &str =
    "423022abe243dbe00f81ac78fa7b159d842b5257cd38d67abf1fc4b432c00a471645a5fcb60d9fcfd301c74d625ddd78";

#[test]
fn angewendete_scrim_stammzeit_migration_bleibt_bytegleich() {
    let migration = include_bytes!("../migrations/2026071602_scrim_team_default_time.sql");
    let checksum = hex::encode(Sha384::digest(migration));

    assert_eq!(checksum, SCRIM_TEAM_DEFAULT_TIME_SHA384);
}
