use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const LEGACY_SHORT_VERSIONS: &str = include_str!("legacy_short_migration_versions.txt");
const VERSION_WIDTH: usize = 14;
const FIRST_FULL_WIDTH_VERSION: i64 = 20_260_930_220_000;

fn migration_file_names() -> Vec<String> {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut names = Vec::new();
    for entry in fs::read_dir(&migrations_path).expect("migrations directory must be readable") {
        let path = entry.expect("migration entry must be readable").path();
        let metadata = fs::metadata(&path).unwrap_or_else(|error| {
            panic!("metadata of {} must be readable: {error}", path.display())
        });
        if !metadata.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("migration file name must be UTF-8: {}", path.display()));
        names.push(name.to_owned());
    }
    names.sort();
    names
}

fn frozen_short_versions() -> BTreeSet<&'static str> {
    LEGACY_SHORT_VERSIONS
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn version_of(file_name: &str) -> Result<(&str, i64), &'static str> {
    let Some((version, rest)) = file_name.split_once('_') else {
        return Err("has no `_` after the version");
    };
    if !rest.ends_with(".sql") {
        return Err("does not end in .sql");
    }
    if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("version is not made of ASCII digits only");
    }
    match version.parse::<i64>() {
        Ok(number) => Ok((version, number)),
        Err(_) => Err("version does not fit a signed 64-bit integer"),
    }
}

fn uniqueness_problems(file_names: &[String]) -> Vec<String> {
    let mut file_by_number: BTreeMap<i64, &str> = BTreeMap::new();
    let mut problems = Vec::new();
    for file_name in file_names {
        match version_of(file_name) {
            Err(reason) => problems.push(format!("{file_name}: {reason}")),
            Ok((_, number)) => {
                if let Some(earlier) = file_by_number.insert(number, file_name) {
                    problems.push(format!(
                        "{file_name}: version number {number} is already used by {earlier}"
                    ));
                }
            }
        }
    }
    problems
}

fn width_problems(file_names: &[String], frozen: &BTreeSet<&str>) -> Vec<String> {
    let mut frozen_with_file = BTreeSet::new();
    let mut problems = Vec::new();
    for file_name in file_names {
        let (version, number) = match version_of(file_name) {
            Ok(parsed) => parsed,
            Err(reason) => {
                problems.push(format!("{file_name}: {reason}"));
                continue;
            }
        };
        if let Some(known) = frozen.get(version) {
            frozen_with_file.insert(*known);
        } else if version.len() != VERSION_WIDTH {
            problems.push(format!(
                "{file_name}: version must be exactly {VERSION_WIDTH} ASCII digits"
            ));
        } else if number < FIRST_FULL_WIDTH_VERSION {
            problems.push(format!(
                "{file_name}: version number {number} is below {FIRST_FULL_WIDTH_VERSION}"
            ));
        }
    }
    for version in frozen.difference(&frozen_with_file) {
        problems.push(format!(
            "frozen legacy version {version} has no migration file"
        ));
    }
    problems
}

fn names(file_names: &[&str]) -> Vec<String> {
    file_names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn migrationsversionen_sind_eindeutig() {
    let problems = uniqueness_problems(&migration_file_names());
    assert!(
        problems.is_empty(),
        "migration versions must be unique as numbers: {problems:#?}"
    );
}

#[test]
fn migration_versions_have_fourteen_digits() {
    let problems = width_problems(&migration_file_names(), &frozen_short_versions());
    assert!(
        problems.is_empty(),
        "migration versions outside the frozen legacy list must be {VERSION_WIDTH} ASCII digits \
         and at least {FIRST_FULL_WIDTH_VERSION}; this test does not know which versions are \
         applied, so it cannot tell whether a version sorts after them: {problems:#?}"
    );
}

#[test]
fn version_rules_name_the_offending_file() {
    let frozen = BTreeSet::from(["0001", "2026070210"]);
    let accepted = names(&[
        "0001_a.sql",
        "2026070210_b.sql",
        "20260930220000_c.sql",
        "20261110120000_d.sql",
    ]);
    assert_eq!(uniqueness_problems(&accepted), Vec::<String>::new());
    assert_eq!(width_problems(&accepted, &frozen), Vec::<String>::new());

    let numeric_duplicate = names(&["00000000000001_b.sql", "0001_a.sql"]);
    let duplicates = uniqueness_problems(&numeric_duplicate);
    assert_eq!(duplicates.len(), 1, "{duplicates:?}");
    assert!(duplicates[0].starts_with("0001_a.sql: version number 1 is already used"));

    for rejected in [
        "20261110120000999999_overflow.sql",
        "+0000000000001_sign.sql",
        "-0000000000001_sign.sql",
        "2026111012000a_letter.sql",
        "_empty.sql",
        "20261110120000.sql",
        "20261110120000_note.txt",
        "00002026111001_padded.sql",
        "2026111001_short.sql",
        "202611101200000_long.sql",
        "20260930215959_early.sql",
    ] {
        let listed = names(&["0001_a.sql", "2026070210_b.sql", rejected]);
        let problems = width_problems(&listed, &frozen);
        assert_eq!(problems.len(), 1, "{rejected}: {problems:?}");
        assert!(
            problems[0].starts_with(&format!("{rejected}: ")),
            "{problems:?}"
        );
    }

    for unparsed in [
        "20261110120000999999_overflow.sql",
        "+0000000000001_sign.sql",
        "_empty.sql",
        "20261110120000.sql",
    ] {
        let problems = uniqueness_problems(&names(&[unparsed]));
        assert_eq!(problems.len(), 1, "{unparsed}: {problems:?}");
        assert!(
            problems[0].starts_with(&format!("{unparsed}: ")),
            "{problems:?}"
        );
    }

    let stale = width_problems(&names(&["0001_a.sql"]), &frozen);
    assert_eq!(
        stale,
        vec!["frozen legacy version 2026070210 has no migration file".to_owned()]
    );
}
