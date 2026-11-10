use std::{collections::BTreeMap, fs, path::Path};

const LEGACY_BARE_GRANTS: &str = include_str!("legacy_bare_deadlock_grants.txt");
const ROLE: &str = "deadlock";

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
    Text(String),
    Body(String),
    Symbol(char),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Grants {
    guarded: usize,
    bare: usize,
}

enum Frame {
    If(bool),
    Case,
    Loop,
    Block,
}

fn word(text: &str) -> Token {
    Token::Word(text.to_owned())
}

fn is_word(token: Option<&Token>, text: &str) -> bool {
    matches!(token, Some(Token::Word(found)) if found == text)
}

fn quoted_end(chars: &[char], start: usize, quote: char, backslash: bool) -> (String, usize) {
    let mut text = String::new();
    let mut at = start + 1;
    while at < chars.len() {
        let current = chars[at];
        if backslash && current == '\\' && at + 1 < chars.len() {
            text.push(chars[at + 1]);
            at += 2;
        } else if current == quote && chars.get(at + 1) == Some(&quote) {
            text.push(quote);
            at += 2;
        } else if current == quote {
            return (text, at + 1);
        } else {
            text.push(current);
            at += 1;
        }
    }
    (text, at)
}

fn block_comment_end(chars: &[char], start: usize) -> usize {
    let mut depth = 0usize;
    let mut at = start;
    while at < chars.len() {
        if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
            depth += 1;
            at += 2;
        } else if chars[at] == '*' && chars.get(at + 1) == Some(&'/') {
            depth -= 1;
            at += 2;
            if depth == 0 {
                return at;
            }
        } else {
            at += 1;
        }
    }
    at
}

fn dollar_body(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut tag_end = start + 1;
    while tag_end < chars.len() && (chars[tag_end].is_alphanumeric() || chars[tag_end] == '_') {
        tag_end += 1;
    }
    let starts_with_digit = chars.get(start + 1).is_some_and(char::is_ascii_digit);
    if chars.get(tag_end) != Some(&'$') || starts_with_digit {
        return None;
    }
    let delimiter = &chars[start..=tag_end];
    let body_start = tag_end + 1;
    let mut at = body_start;
    while at + delimiter.len() <= chars.len() {
        if &chars[at..at + delimiter.len()] == delimiter {
            let body = chars[body_start..at].iter().collect();
            return Some((body, at + delimiter.len()));
        }
        at += 1;
    }
    Some((chars[body_start..].iter().collect(), chars.len()))
}

fn tokenize(sql: &str) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let current = chars[at];
        let next = chars.get(at + 1).copied();
        if current.is_whitespace() {
            at += 1;
        } else if current == '-' && next == Some('-') {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
        } else if current == '/' && next == Some('*') {
            at = block_comment_end(&chars, at);
        } else if current == '\'' {
            let escape_prefix =
                is_word(tokens.last(), "e") && at > 0 && matches!(chars[at - 1], 'e' | 'E');
            if escape_prefix {
                tokens.pop();
            }
            let (text, end) = quoted_end(&chars, at, '\'', escape_prefix);
            tokens.push(Token::Text(text));
            at = end;
        } else if current == '"' {
            let (text, end) = quoted_end(&chars, at, '"', false);
            tokens.push(Token::Quoted(text));
            at = end;
        } else if current == '$' {
            match dollar_body(&chars, at) {
                Some((body, end)) => {
                    tokens.push(Token::Body(body));
                    at = end;
                }
                None => {
                    tokens.push(Token::Symbol('$'));
                    at += 1;
                }
            }
        } else if current.is_alphanumeric() || current == '_' {
            let start = at;
            while at < chars.len()
                && (chars[at].is_alphanumeric() || chars[at] == '_' || chars[at] == '$')
            {
                at += 1;
            }
            let text: String = chars[start..at].iter().collect();
            tokens.push(Token::Word(text.to_lowercase()));
        } else {
            tokens.push(Token::Symbol(current));
            at += 1;
        }
    }
    tokens
}

fn grants_to_role(from_grant: &[Token]) -> bool {
    let mut depth = 0usize;
    let mut in_grantees = false;
    for token in &from_grant[1..] {
        match token {
            Token::Symbol(';') => return false,
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.saturating_sub(1),
            Token::Word(found) if !in_grantees && depth == 0 && found == "to" => {
                in_grantees = true;
            }
            Token::Word(found) if in_grantees && (found == "with" || found == "granted") => {
                return false;
            }
            Token::Word(found) | Token::Quoted(found) if in_grantees && found == ROLE => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn then_of(tokens: &[Token], keyword: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, token) in tokens[keyword + 1..].iter().enumerate() {
        match token {
            Token::Symbol(';') if depth == 0 => return None,
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.saturating_sub(1),
            Token::Word(found) if depth == 0 && found == "then" => {
                return Some(keyword + 1 + offset);
            }
            _ => {}
        }
    }
    None
}

fn is_role_guard(condition: &[Token]) -> bool {
    let tail = [
        word("pg_roles"),
        word("where"),
        word("rolname"),
        Token::Symbol('='),
        Token::Text(ROLE.to_owned()),
        Token::Symbol(')'),
    ];
    let head = [word("exists"), Token::Symbol('('), word("select")];
    if !condition.starts_with(&head) || !condition.ends_with(&tail) {
        return false;
    }
    let source = &condition[..condition.len() - tail.len()];
    let from_catalog = [word("from"), word("pg_catalog"), Token::Symbol('.')];
    if !source.ends_with(&[word("from")]) && !source.ends_with(&from_catalog) {
        return false;
    }
    let mut depth = 0usize;
    for token in &condition[1..condition.len() - 1] {
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 {
            return false;
        }
    }
    depth == 1
}

fn close_frame(frames: &mut Vec<Frame>, after_end: Option<&Token>) {
    let closes = |frame: &Frame| match frame {
        Frame::If(_) => is_word(after_end, "if"),
        Frame::Loop => is_word(after_end, "loop"),
        Frame::Case => is_word(after_end, "case"),
        Frame::Block => false,
    };
    let typed = ["if", "loop", "case"]
        .iter()
        .any(|keyword| is_word(after_end, keyword));
    while let Some(frame) = frames.pop() {
        if !typed || closes(&frame) {
            return;
        }
    }
}

fn scan_body(body: &str, inherited_guard: bool, grants: &mut Grants) {
    let tokens = tokenize(body);
    let mut frames: Vec<Frame> = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let after_end = at > 0 && is_word(tokens.get(at - 1), "end");
        let guarded =
            inherited_guard || frames.iter().any(|frame| matches!(frame, Frame::If(true)));
        match &tokens[at] {
            Token::Word(found) => match found.as_str() {
                "if" if !after_end => {
                    if let Some(then) = then_of(&tokens, at) {
                        frames.push(Frame::If(is_role_guard(&tokens[at + 1..then])));
                        at = then;
                    }
                }
                "elsif" | "elseif" => {
                    if let (Some(then), Some(Frame::If(guard))) =
                        (then_of(&tokens, at), frames.last_mut())
                    {
                        *guard = is_role_guard(&tokens[at + 1..then]);
                        at = then;
                    }
                }
                "else" => {
                    if let Some(Frame::If(guard)) = frames.last_mut() {
                        *guard = false;
                    }
                }
                "case" if !after_end => frames.push(Frame::Case),
                "loop" if !after_end => frames.push(Frame::Loop),
                "begin" => frames.push(Frame::Block),
                "end" => close_frame(&mut frames, tokens.get(at + 1)),
                "grant" if grants_to_role(&tokens[at..]) => {
                    if guarded {
                        grants.guarded += 1;
                    } else {
                        grants.bare += 1;
                    }
                }
                _ => {}
            },
            Token::Text(inner) | Token::Body(inner) => scan_body(inner, guarded, grants),
            _ => {}
        }
        at += 1;
    }
}

fn scan_migration(sql: &str) -> Grants {
    let mut grants = Grants::default();
    let tokens = tokenize(sql);
    for statement in tokens.split(|token| *token == Token::Symbol(';')) {
        let is_do = is_word(statement.first(), "do");
        for (at, token) in statement.iter().enumerate() {
            match token {
                Token::Word(found) if found == "grant" && grants_to_role(&statement[at..]) => {
                    grants.bare += 1;
                }
                Token::Body(body) => scan_body(body, false, &mut grants),
                Token::Text(body) if is_do => scan_body(body, false, &mut grants),
                _ => {}
            }
        }
    }
    grants
}

fn scan_migrations() -> BTreeMap<String, Grants> {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut scanned = BTreeMap::new();
    for entry in fs::read_dir(&migrations_path).expect("migrations directory must be readable") {
        let path = entry.expect("migration entry must be readable").path();
        let metadata = fs::metadata(&path).unwrap_or_else(|error| {
            panic!("metadata of {} must be readable: {error}", path.display())
        });
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("migration file name must be UTF-8: {}", path.display()))
            .to_owned();
        if !metadata.is_file() || !name.ends_with(".sql") {
            continue;
        }
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{name} must be readable as UTF-8: {error}"));
        scanned.insert(name, scan_migration(&sql));
    }
    scanned
}

fn frozen_bare_grants(list: &str) -> Result<BTreeMap<String, usize>, String> {
    let mut frozen = BTreeMap::new();
    for line in list.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let entry = line
            .rsplit_once(':')
            .and_then(|(file, count)| Some((file, count.parse::<usize>().ok()?)))
            .filter(|(file, count)| !file.is_empty() && *count > 0);
        let Some((file, count)) = entry else {
            return Err(format!(
                "frozen line {line:?} is not `file name:statement count`"
            ));
        };
        if frozen.insert(file.to_owned(), count).is_some() {
            return Err(format!("frozen file {file} is listed twice"));
        }
    }
    Ok(frozen)
}

fn bare_grant_problems(
    scanned: &BTreeMap<String, Grants>,
    frozen: &BTreeMap<String, usize>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (file, grants) in scanned.iter().filter(|(_, grants)| grants.bare > 0) {
        match frozen.get(file) {
            None => problems.push(format!(
                "{file}: {} GRANT statement(s) to role {ROLE} outside a DO block that checks \
                 pg_roles for rolname = '{ROLE}' first",
                grants.bare
            )),
            Some(count) if *count != grants.bare => problems.push(format!(
                "{file}: {} bare GRANT statement(s) to role {ROLE}, the frozen list records {count}",
                grants.bare
            )),
            Some(_) => {}
        }
    }
    for (file, count) in frozen {
        if scanned.get(file).is_none_or(|grants| grants.bare == 0) {
            problems.push(format!(
                "{file}: the frozen list records {count} bare GRANT statement(s) to role {ROLE}, \
                 the migrations hold none"
            ));
        }
    }
    problems
}

#[test]
fn grants_to_role_deadlock_are_guarded() {
    let scanned = scan_migrations();
    let frozen = frozen_bare_grants(LEGACY_BARE_GRANTS).unwrap_or_else(|error| panic!("{error}"));
    let guarded: Vec<&Grants> = scanned
        .values()
        .filter(|grants| grants.guarded > 0)
        .collect();
    let bare: Vec<&Grants> = scanned.values().filter(|grants| grants.bare > 0).collect();
    println!(
        "grants to role {ROLE} in {} migration files: {} guarded statements in {} files, \
         {} bare statements in {} files",
        scanned.len(),
        guarded.iter().map(|grants| grants.guarded).sum::<usize>(),
        guarded.len(),
        bare.iter().map(|grants| grants.bare).sum::<usize>(),
        bare.len(),
    );
    let problems = bare_grant_problems(&scanned, &frozen);
    assert!(
        problems.is_empty(),
        "every GRANT to role {ROLE} must sit inside `DO $roles$ BEGIN IF EXISTS (SELECT 1 FROM \
         pg_roles WHERE rolname = '{ROLE}') THEN ... END IF; END $roles$;` because the role does \
         not exist in every database: {problems:#?}"
    );
}

#[test]
fn scanner_classifies_known_shapes() {
    let guarded = |count| Grants {
        guarded: count,
        bare: 0,
    };
    let bare = |count| Grants {
        guarded: 0,
        bare: count,
    };
    let cases = [
        ("", Grants::default()),
        ("GRANT SELECT ON t TO deadlock;", bare(1)),
        ("grant select on t to deadlock", bare(1)),
        ("GRANT dl_patchnotes_privacy TO deadlock;", bare(1)),
        ("GRANT SELECT ON t TO reader,\n    DeadLock ;", bare(1)),
        ("GRANT SELECT ON t TO \"deadlock\", GROUP reader;", bare(1)),
        ("GRANT UPDATE (a, b)\nON t\nTO reader, deadlock WITH GRANT OPTION;", bare(1)),
        ("ALTER DEFAULT PRIVILEGES IN SCHEMA s GRANT SELECT ON TABLES TO deadlock;", bare(1)),
        ("GRANT SELECT ON t TO reader; GRANT SELECT ON t TO deadlock;", bare(1)),
        ("GRANT SELECT ON deadlock TO reader;", Grants::default()),
        ("GRANT SELECT ON t TO \"Deadlock\", deadlock_reader;", Grants::default()),
        ("GRANT SELECT ON t TO reader WITH GRANT OPTION; SELECT deadlock;", Grants::default()),
        ("REVOKE GRANT OPTION FOR SELECT ON t FROM deadlock;", Grants::default()),
        ("COMMENT ON TABLE t IS 'GRANT SELECT ON t TO deadlock;';", Grants::default()),
        ("-- GRANT SELECT ON t TO deadlock;\n/* GRANT a TO deadlock; */", Grants::default()),
        ("SELECT $note$ it's fine $note$; GRANT SELECT ON t TO deadlock;", bare(1)),
        ("SELECT E'it\\'s'; GRANT SELECT ON t TO deadlock;", bare(1)),
        (
            "DO $roles$\nBEGIN\n    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN\n        \
             GRANT SELECT ON t TO deadlock;\n    END IF;\nEND\n$roles$;",
            guarded(1),
        ),
        (
            "do $$ begin\n if exists(select 1 from pg_catalog.pg_roles where rolname='deadlock') then\n\
             grant select,insert on t\n   to deadlock;\n grant a to deadlock; end if; end $$;",
            guarded(2),
        ),
        (
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             EXECUTE 'GRANT SELECT ON t TO deadlock'; \
             EXECUTE format('GRANT SELECT ON %I TO deadlock', 't'); END IF; END $$;",
            guarded(2),
        ),
        (
            "DO 'BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = ''deadlock'') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END';",
            guarded(1),
        ),
        (
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             CREATE TABLE IF NOT EXISTS t (a int); \
             PERFORM CASE WHEN true THEN 1 ELSE 2 END; \
             FOR i IN 1..2 LOOP NULL; END LOOP; \
             BEGIN NULL; EXCEPTION WHEN others THEN NULL; END; \
             GRANT SELECT ON t TO deadlock; END IF; END $$;",
            guarded(1),
        ),
        ("DO $$ BEGIN GRANT SELECT ON t TO deadlock; END $$;", bare(1)),
        ("DO $$ BEGIN EXECUTE 'GRANT SELECT ON t TO deadlock'; END $$;", bare(1)),
        (
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'twitchbot') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$;",
            bare(1),
        ),
        (
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$;",
            bare(1),
        ),
        (
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') OR true THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$;",
            bare(1),
        ),
        (
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             NULL; ELSE GRANT SELECT ON t TO deadlock; END IF; END $$;",
            bare(1),
        ),
        (
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             NULL; END IF; GRANT SELECT ON t TO deadlock; END $$;",
            bare(1),
        ),
        (
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN \
             GRANT SELECT ON t TO deadlock; END IF; END $$; GRANT SELECT ON u TO deadlock;",
            Grants {
                guarded: 1,
                bare: 1,
            },
        ),
        (
            "CREATE FUNCTION f() RETURNS void LANGUAGE plpgsql AS $$ BEGIN \
             GRANT SELECT ON t TO deadlock; END $$;",
            bare(1),
        ),
    ];
    for (sql, expected) in cases {
        assert_eq!(scan_migration(sql), expected, "{sql}");
    }
}

#[test]
fn frozen_list_cannot_rot() {
    let scanned = BTreeMap::from([
        (
            "a.sql".to_owned(),
            Grants {
                guarded: 0,
                bare: 1,
            },
        ),
        (
            "b.sql".to_owned(),
            Grants {
                guarded: 2,
                bare: 0,
            },
        ),
        (
            "c.sql".to_owned(),
            Grants {
                guarded: 0,
                bare: 2,
            },
        ),
    ]);
    let frozen = |list: &str| frozen_bare_grants(list).expect("well-formed list");

    assert!(bare_grant_problems(&scanned, &frozen("a.sql:1\nc.sql:2\n")).is_empty());

    let unlisted = bare_grant_problems(&scanned, &frozen("a.sql:1\n"));
    assert_eq!(unlisted.len(), 1, "{unlisted:?}");
    assert!(unlisted[0].starts_with("c.sql: 2 GRANT statement(s)"));

    let grown = bare_grant_problems(&scanned, &frozen("a.sql:1\nc.sql:1\n"));
    assert_eq!(grown.len(), 1, "{grown:?}");
    assert!(grown[0].starts_with("c.sql: 2 bare GRANT statement(s)"));

    let stale = bare_grant_problems(&scanned, &frozen("a.sql:1\nb.sql:1\nc.sql:2\ngone.sql:1\n"));
    assert_eq!(stale.len(), 2, "{stale:?}");
    assert!(stale[0].starts_with("b.sql: the frozen list records 1"));
    assert!(stale[1].starts_with("gone.sql: the frozen list records 1"));

    for malformed in ["a.sql", "a.sql:0", "a.sql:x", ":1", "a.sql:1\na.sql:1"] {
        assert!(frozen_bare_grants(malformed).is_err(), "{malformed}");
    }
}
