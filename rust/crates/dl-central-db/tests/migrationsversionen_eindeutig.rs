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
