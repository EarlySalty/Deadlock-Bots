use std::{collections::HashMap, fs, path::Path};

#[test]
fn migrationsversionen_sind_eindeutig() {
    let migrationspfad = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut dateien_nach_version = HashMap::new();

    for eintrag in fs::read_dir(migrationspfad).expect("Migrationsordner muss lesbar sein") {
        let eintrag = eintrag.expect("Migrationseintrag muss lesbar sein");
        if !eintrag
            .file_type()
            .expect("Migrationstyp muss lesbar sein")
            .is_file()
        {
            continue;
        }

        let dateiname = eintrag
            .file_name()
            .into_string()
            .expect("Migrationsdateiname muss UTF-8 sein");
        let Some((version, _)) = dateiname.split_once('_') else {
            continue;
        };

        if let Some(vorherige_datei) =
            dateien_nach_version.insert(version.to_owned(), dateiname.clone())
        {
            panic!("Doppelte Migrationsversion {version}: {vorherige_datei} und {dateiname}");
        }
    }
}

const LEGACY_SHORT_VERSIONS: &str = include_str!("legacy_short_migration_versions.txt");
const VERSION_WIDTH: usize = 14;

#[test]
fn migration_versions_have_fourteen_digits() {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let frozen: std::collections::BTreeSet<&str> = LEGACY_SHORT_VERSIONS
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut frozen_with_file = std::collections::BTreeSet::new();
    let mut offending_files = Vec::new();

    for entry in fs::read_dir(migrations_path).expect("migrations directory must be readable") {
        let entry = entry.expect("migration entry must be readable");
        if !entry
            .file_type()
            .expect("migration file type must be readable")
            .is_file()
        {
            continue;
        }

        let file_name = entry
            .file_name()
            .into_string()
            .expect("migration file name must be UTF-8");
        let Some((version, _)) = file_name.split_once('_') else {
            continue;
        };

        if let Some(known) = frozen.get(version) {
            frozen_with_file.insert(*known);
            continue;
        }

        let fourteen_digits =
            version.len() == VERSION_WIDTH && version.bytes().all(|byte| byte.is_ascii_digit());
        if !fourteen_digits {
            offending_files.push(file_name);
        }
    }

    offending_files.sort();
    let frozen_without_file: Vec<&str> = frozen.difference(&frozen_with_file).copied().collect();

    assert!(
        offending_files.is_empty() && frozen_without_file.is_empty(),
        "migration versions must be exactly {VERSION_WIDTH} ASCII digits (YYYYMMDDHHMMSS); \
         files with another version: {offending_files:?}; \
         frozen legacy versions without a migration file: {frozen_without_file:?}"
    );
}
