use std::{collections::BTreeMap, fs, path::Path};

const FROZEN_MENTIONS: &str = include_str!("deadlock_mentions_outside_role_guard.txt");
const ROLE: &str = "deadlock";
const GUARD: &str = "if exists (select 1 from pg_roles where rolname = 'deadlock') then";
const GUARD_ROLE_LITERAL: &str = "'deadlock'";
const REGION_ENDS: [&str; 4] = ["end if", "else", "elsif", "elseif"];
const RULE: &str = "A migration may contain the word `deadlock` only between the exact guard \
    `IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN` (exactly these words \
    and spaces, keywords in any case, a run of whitespace counts as one space, the role literal \
    in lowercase) and the next END IF, ELSE or ELSIF, so put every statement that names the \
    role inside that guard and do not use the role name or the word anywhere else in the file, \
    comments and string literals included.";

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn find_whole(text: &str, needle: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut start = from;
    while let Some(offset) = text.get(start..)?.find(needle) {
        let at = start + offset;
        let end = at + needle.len();
        let free_before = at == 0 || !is_identifier_byte(bytes[at - 1]);
        let free_after = end == bytes.len() || !is_identifier_byte(bytes[end]);
        if free_before && free_after {
            return Some(at);
        }
        start = at + 1;
    }
    None
}

fn find_guard(collapsed: &str, lowered: &str, from: usize) -> Option<usize> {
    let mut start = from;
    while let Some(at) = find_whole(lowered, GUARD, start) {
        if collapsed[at..at + GUARD.len()].contains(GUARD_ROLE_LITERAL) {
            return Some(at);
        }
        start = at + 1;
    }
    None
}

fn guarded_region_end(lowered: &str, from: usize) -> usize {
    REGION_ENDS
        .iter()
        .filter_map(|end| find_whole(lowered, end, from))
        .min()
        .unwrap_or(lowered.len())
}

fn mentions_outside_guard(sql: &str) -> usize {
    let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let lowered = collapsed.to_ascii_lowercase();
    let mut count = 0;
    let mut at = 0;
    loop {
        let guard = find_guard(&collapsed, &lowered, at);
        let open_until = guard.unwrap_or(lowered.len());
        let mut mention = find_whole(&lowered, ROLE, at);
        while let Some(found) = mention.filter(|found| *found < open_until) {
            count += 1;
            mention = find_whole(&lowered, ROLE, found + ROLE.len());
        }
        let Some(guard) = guard else {
            return count;
        };
        at = guarded_region_end(&lowered, guard + GUARD.len());
    }
}

fn mentions_by_file() -> BTreeMap<String, usize> {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut found = BTreeMap::new();
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
            .unwrap_or_else(|| panic!("migration file name must be UTF-8: {}", path.display()))
            .to_owned();
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{name} must be readable as UTF-8: {error}"));
        found.insert(name, mentions_outside_guard(&sql));
    }
    found
}

fn frozen_counts(list: &str) -> Result<BTreeMap<String, usize>, String> {
    let mut frozen = BTreeMap::new();
    for line in list.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let entry = line
            .rsplit_once(':')
            .filter(|(file, count)| {
                !file.is_empty()
                    && !count.is_empty()
                    && count.bytes().all(|byte| byte.is_ascii_digit())
            })
            .and_then(|(file, count)| Some((file, count.parse::<usize>().ok()?)))
            .filter(|(_, count)| *count > 0);
        let Some((file, count)) = entry else {
            return Err(format!(
                "frozen line {line:?} is not `file name:count` with a count above 0"
            ));
        };
        if frozen.insert(file.to_owned(), count).is_some() {
            return Err(format!("frozen file {file} is listed twice"));
        }
    }
    Ok(frozen)
}

fn count_problems(
    found: &BTreeMap<String, usize>,
    frozen: &BTreeMap<String, usize>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (file, count) in found {
        let expected = frozen.get(file).copied().unwrap_or(0);
        if *count != expected {
            problems.push(format!("{file}: expected {expected}, found {count}"));
        }
    }
    for (file, expected) in frozen {
        if !found.contains_key(file) {
            problems.push(format!("{file}: expected {expected}, file is missing"));
        }
    }
    problems
}

fn counts(entries: &[(&str, usize)]) -> BTreeMap<String, usize> {
    entries
        .iter()
        .map(|(file, count)| ((*file).to_owned(), *count))
        .collect()
}

#[test]
fn role_deadlock_is_named_only_inside_the_role_guard() {
    let frozen = frozen_counts(FROZEN_MENTIONS).unwrap_or_else(|error| panic!("{error}"));
    let problems = count_problems(&mentions_by_file(), &frozen);
    assert!(
        problems.is_empty(),
        "{RULE} tests/deadlock_mentions_outside_role_guard.txt freezes the counts applied \
         migrations already hold; a file not listed there must come out at 0. Files whose count \
         outside the guard differs from the expected one: {problems:#?}"
    );
}

#[test]
fn mentions_outside_the_guard_are_counted() {
    let cases: &[(&str, &str, usize)] = &[
        ("empty text", "", 0),
        ("no mention", "CREATE TABLE core.t (id BIGINT PRIMARY KEY);", 0),
        ("bare grant", "GRANT SELECT ON t TO deadlock;", 1),
        (
            "grant inside the exact guard",
            "DO $roles$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $roles$;",
            0,
        ),
        (
            "guard on pg_catalog.pg_roles is not the exact guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'deadlock') \
             THEN GRANT SELECT ON t TO deadlock; END IF; END $$;",
            2,
        ),
        (
            "guard that counts rows is always true",
            "DO $$ BEGIN IF EXISTS (SELECT count(*) FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$;",
            2,
        ),
        (
            "guard with a second condition",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') OR true \
             THEN GRANT SELECT ON t TO deadlock; END IF; END $$;",
            2,
        ),
        (
            "negated guard",
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             CREATE ROLE deadlock; END IF; END $$;",
            2,
        ),
        (
            "elsif exists is not the exact guard",
            "DO $$ BEGIN IF x THEN NULL; ELSIF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = \
             'deadlock') THEN GRANT SELECT ON t TO deadlock; END IF; END $$;",
            2,
        ),
        (
            "guard with an uppercase role literal names another role",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'DEADLOCK') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$;",
            2,
        ),
        (
            "mention after the end of the guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$; GRANT SELECT ON u TO deadlock;",
            1,
        ),
        (
            "mention after a nested if inside the guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             IF x THEN NULL; END IF; GRANT SELECT ON t TO deadlock; END IF; END $$;",
            1,
        ),
        (
            "mention in the else branch of the guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             NULL; ELSE CREATE ROLE deadlock; END IF; END $$;",
            1,
        ),
        (
            "mention in an elsif branch of the guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             NULL; ELSIF x THEN GRANT SELECT ON t TO deadlock; END IF; END $$;",
            1,
        ),
        (
            "mention in an elseif branch of the guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             NULL; ELSEIF x THEN GRANT SELECT ON t TO deadlock; END IF; END $$;",
            1,
        ),
        (
            "mention after a case else inside the guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             PERFORM CASE WHEN x THEN 1 ELSE 2 END; GRANT SELECT ON t TO deadlock; END IF; END $$;",
            1,
        ),
        (
            "longer identifiers are other names",
            "SELECT deadlock_heroes, dl_deadlock, deadlock2, xdeadlock FROM steam.deadlocks;",
            0,
        ),
        (
            "else and end if inside longer words do not end the guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             PERFORM 'the backend if needed', elsewhere, end_if FROM core.models; \
             GRANT SELECT ON core.channels TO deadlock; END IF; END $$;",
            0,
        ),
        (
            "uppercase guard",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM PG_ROLES WHERE ROLNAME = 'deadlock') THEN \
             GRANT SELECT ON T TO DEADLOCK; END IF; END $$;",
            0,
        ),
        (
            "lowercase guard",
            "do $$ begin if exists (select 1 from pg_roles where rolname = 'deadlock') then \
             grant select on t to deadlock; end if; end $$;",
            0,
        ),
        (
            "guard with a line break inside its parentheses is not the exact guard",
            "DO $$\nBEGIN\n\tIF EXISTS (\n\t\tSELECT 1\n\t\tFROM pg_roles\n\t\tWHERE rolname = \
             'deadlock'\n\t)\r\n\tTHEN\n\t\tGRANT SELECT ON t\n\t\tTO deadlock;\n\tEND\n\tIF;\nEND\n$$;",
            2,
        ),
        (
            "guard with line breaks between its words",
            "DO $$\nBEGIN\n\tIF   EXISTS\n(SELECT 1\n\t\tFROM pg_roles\n\t\tWHERE rolname =\n\
             'deadlock')\r\n\tTHEN\n\t\tGRANT SELECT ON t\n\t\tTO deadlock;\n\tEND\n\tIF;\nEND\n$$;",
            0,
        ),
        (
            "revoke outside a guard",
            "REVOKE ALL ON t FROM deadlock;",
            1,
        ),
        (
            "owner outside a guard",
            "ALTER TABLE t OWNER TO deadlock;",
            1,
        ),
        (
            "grantee passed to format outside a guard",
            "DO $$ BEGIN EXECUTE format('GRANT SELECT ON t TO %I', 'deadlock'); END $$;",
            1,
        ),
        (
            "quoted grantee outside a guard",
            "GRANT SELECT ON t TO \"deadlock\";",
            1,
        ),
        (
            "comment line outside a guard",
            "-- the deadlock role reads this table\nCREATE TABLE t (id INT);",
            1,
        ),
        (
            "guard without end if guards the rest of the text",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON t TO deadlock; GRANT SELECT ON u TO deadlock;",
            0,
        ),
        (
            "two guards with a mention between them",
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$; COMMENT ON TABLE t IS 'for deadlock'; \
             DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON u TO deadlock; END IF; END $$;",
            1,
        ),
        (
            "mention next to non-ASCII text",
            "COMMENT ON TABLE t IS 'für deadlock, ädeadlock, deadlockß';",
            3,
        ),
        ("the word alone", "deadlock", 1),
    ];
    for (label, sql, expected) in cases {
        assert_eq!(mentions_outside_guard(sql), *expected, "{label}: {sql}");
    }
}

#[test]
fn frozen_counts_are_compared_in_both_directions() {
    let frozen = counts(&[("a.sql", 2)]);
    let no_problems = Vec::<String>::new();
    assert_eq!(
        count_problems(&counts(&[("a.sql", 2), ("b.sql", 0)]), &frozen),
        no_problems
    );
    assert_eq!(
        count_problems(&counts(&[("a.sql", 3)]), &frozen),
        vec!["a.sql: expected 2, found 3".to_owned()]
    );
    assert_eq!(
        count_problems(&counts(&[("a.sql", 0)]), &frozen),
        vec!["a.sql: expected 2, found 0".to_owned()]
    );
    assert_eq!(
        count_problems(&counts(&[("a.sql", 2), ("b.sql", 1)]), &frozen),
        vec!["b.sql: expected 0, found 1".to_owned()]
    );
    assert_eq!(
        count_problems(&counts(&[("b.sql", 0)]), &frozen),
        vec!["a.sql: expected 2, file is missing".to_owned()]
    );

    assert_eq!(
        frozen_counts("a.sql:2\n\n  b.sql:1  \n"),
        Ok(counts(&[("a.sql", 2), ("b.sql", 1)]))
    );
    assert_eq!(frozen_counts(""), Ok(BTreeMap::new()));
    for malformed in [
        "a.sql", "a.sql:", "a.sql:x", "a.sql:0", "a.sql:+1", "a.sql: 1", ":1",
    ] {
        assert!(frozen_counts(malformed).is_err(), "{malformed}");
    }
    assert_eq!(
        frozen_counts("a.sql:1\na.sql:2"),
        Err("frozen file a.sql is listed twice".to_owned())
    );
}
