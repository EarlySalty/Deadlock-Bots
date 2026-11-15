use std::{collections::BTreeSet, fs, path::Path};

const TIMEOUTS: [&str; 2] = ["lock_timeout", "statement_timeout"];
const APPLIED_LEAKS: [&str; 1] = ["20261021120000_global_dm_optout.sql"];
const RULE: &str = "A migration sets lock_timeout or statement_timeout only with SET LOCAL, or \
    resets it with RESET <name> after its last session-level SET; sqlx runs each migration in \
    its own transaction on a pooled connection, so a session-level SET outlives the migration.";

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Text(String),
    Body(String),
    Symbol(char),
}

#[derive(Clone, Debug, PartialEq)]
enum Event {
    Set(Option<String>),
    Reset(String),
    ResetAll,
}

fn skip_block_comment(chars: &[char], mut at: usize) -> usize {
    let mut depth = 0usize;
    while at < chars.len() {
        if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
            depth += 1;
            at += 2;
        } else if chars[at] == '*' && chars.get(at + 1) == Some(&'/') {
            depth = depth.saturating_sub(1);
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

fn read_quoted(chars: &[char], mut at: usize, quote: char, escapes: bool) -> (String, usize) {
    let mut text = String::new();
    at += 1;
    while at < chars.len() {
        let c = chars[at];
        if escapes && c == '\\' {
            text.extend(chars.get(at + 1));
            at += 2;
        } else if c == quote && chars.get(at + 1) == Some(&quote) {
            text.insert(text.len(), quote);
            at += 2;
        } else if c == quote {
            return (text, at + 1);
        } else {
            text.insert(text.len(), c);
            at += 1;
        }
    }
    (text, at)
}

fn dollar_tag(chars: &[char], at: usize) -> Option<String> {
    let mut end = at + 1;
    while end < chars.len() && (chars[end] == '_' || chars[end].is_ascii_alphanumeric()) {
        end += 1;
    }
    let tag: String = chars[at + 1..end].iter().collect();
    let valid = !tag.starts_with(|c: char| c.is_ascii_digit());
    (valid && chars.get(end) == Some(&'$')).then(|| format!("${tag}$"))
}

fn read_dollar_body(chars: &[char], at: usize, tag: &str) -> (String, usize) {
    let open = tag.chars().count();
    let rest: String = chars[at + open..].iter().collect();
    match rest.find(tag) {
        Some(close) => {
            let body = rest[..close].to_owned();
            let end = at + open * 2 + body.chars().count();
            (body, end)
        }
        None => (rest, chars.len()),
    }
}

fn is_word_char(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

fn line_end(chars: &[char], at: usize) -> usize {
    chars[at..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(chars.len(), |offset| at + offset)
}

fn word_end(chars: &[char], at: usize) -> usize {
    chars[at..]
        .iter()
        .position(|c| !is_word_char(*c))
        .map_or(chars.len(), |offset| at + offset)
}

fn escape_string_prefix(found: &[Token], chars: &[char], at: usize) -> bool {
    matches!(found.last(), Some(Token::Word(word)) if word == "e")
        && at > 0
        && matches!(chars[at - 1], 'e' | 'E')
}

fn tokens(sql: &str) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        let (token, end) = if c.is_whitespace() {
            (None, at + 1)
        } else if c == '-' && next == Some('-') {
            (None, line_end(&chars, at))
        } else if c == '/' && next == Some('*') {
            (None, skip_block_comment(&chars, at))
        } else if c == '\'' {
            let escapes = escape_string_prefix(&found, &chars, at);
            if escapes {
                found.truncate(found.len() - 1);
            }
            let (text, end) = read_quoted(&chars, at, '\'', escapes);
            (Some(Token::Text(text.to_lowercase())), end)
        } else if c == '"' {
            let (text, end) = read_quoted(&chars, at, '"', false);
            (Some(Token::Word(text.to_lowercase())), end)
        } else if let Some(tag) = (c == '$').then(|| dollar_tag(&chars, at)).flatten() {
            let (body, end) = read_dollar_body(&chars, at, &tag);
            (Some(Token::Body(body)), end)
        } else if is_word_char(c) {
            let end = word_end(&chars, at);
            let text: String = chars[at..end].iter().collect();
            (Some(Token::Word(text.to_lowercase())), end)
        } else {
            (Some(Token::Symbol(c)), at + 1)
        };
        found.extend(token);
        at = end;
    }
    found
}

fn word(token: Option<&Token>) -> Option<&str> {
    match token {
        Some(Token::Word(word)) => Some(word.as_str()),
        _ => None,
    }
}

fn set_statement_event(statement: &[Token]) -> Option<Event> {
    let rest = match word(statement.get(1)) {
        Some("local") => return None,
        Some("session") => statement.get(2..)?,
        _ => statement.get(1..)?,
    };
    let name = word(rest.first())?.to_owned();
    let value = rest.get(2..).unwrap_or_default();
    if matches!(value, [Token::Word(word)] if word == "default") {
        return Some(Event::Reset(name));
    }
    Some(Event::Set(Some(name)))
}

fn statement_event(statement: &[Token]) -> Option<Event> {
    match word(statement.first())? {
        "set" => set_statement_event(statement),
        "reset" => match word(statement.get(1))? {
            "all" => Some(Event::ResetAll),
            name => Some(Event::Reset(name.to_owned())),
        },
        _ => None,
    }
}

fn call_arguments(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut arguments = vec![Vec::new()];
    let mut depth = 0usize;
    for token in tokens {
        match token {
            Token::Symbol('(' | '[') => depth += 1,
            Token::Symbol(')' | ']') if depth == 0 => break,
            Token::Symbol(')' | ']') => depth -= 1,
            Token::Symbol(',') if depth == 0 => {
                arguments.extend([Vec::new()]);
                continue;
            }
            _ => {}
        }
        if let Some(argument) = arguments.last_mut() {
            argument.extend([token.clone()]);
        }
    }
    arguments
}

fn named(argument: &[Token]) -> (Option<&str>, &[Token]) {
    match argument {
        [Token::Word(name), Token::Symbol('='), Token::Symbol('>'), value @ ..]
        | [Token::Word(name), Token::Symbol(':'), Token::Symbol('='), value @ ..] => {
            (Some(name.as_str()), value)
        }
        value => (None, value),
    }
}

fn without_cast(value: &[Token]) -> &[Token] {
    match value {
        [inner @ .., Token::Symbol(':'), Token::Symbol(':'), Token::Word(_)] => inner,
        value => value,
    }
}

fn is_true(value: &[Token]) -> bool {
    match without_cast(value) {
        [Token::Word(word)] => word == "true",
        [Token::Text(text)] => matches!(text.as_str(), "true" | "t" | "on" | "yes" | "y" | "1"),
        _ => false,
    }
}

fn set_config_event(arguments: &[Vec<Token>]) -> Option<Event> {
    let mut name = None;
    let mut local = false;
    for (index, argument) in arguments.iter().enumerate() {
        let (key, value) = named(argument);
        match (key, index) {
            (Some("setting_name"), _) | (None, 0) => name = Some(without_cast(value).to_vec()),
            (Some("is_local"), _) | (None, 2) => local = is_true(value),
            _ => {}
        }
    }
    if local {
        return None;
    }
    match name.as_deref() {
        Some([Token::Text(name)]) => Some(Event::Set(Some(name.clone()))),
        _ => Some(Event::Set(None)),
    }
}

fn set_config_events(statement: &[Token]) -> Vec<Event> {
    statement
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| {
            word(pair.first()) == Some("set_config") && pair[1] == Token::Symbol('(')
        })
        .filter_map(|(at, _)| set_config_event(&call_arguments(&statement[at + 2..])))
        .collect()
}

fn body_events(text: &str) -> Vec<Event> {
    tokens(text)
        .split(|token| *token == Token::Symbol(';'))
        .flat_map(|statement| {
            let mut found = nested_sets(statement);
            found.extend(set_config_events(statement));
            found.extend(
                statement
                    .iter()
                    .enumerate()
                    .filter(|(_, token)| word(Some(token)) == Some("set"))
                    .filter_map(|(at, _)| set_statement_event(&statement[at..])),
            );
            found
        })
        .collect()
}

fn nested_sets(statement: &[Token]) -> Vec<Event> {
    statement
        .iter()
        .filter_map(|token| match token {
            Token::Body(text) | Token::Text(text) => Some(body_events(text)),
            _ => None,
        })
        .flatten()
        .filter(|event| matches!(event, Event::Set(_)))
        .collect()
}

fn events(sql: &str) -> Vec<Event> {
    tokens(sql)
        .split(|token| *token == Token::Symbol(';'))
        .flat_map(|statement| {
            let mut found = nested_sets(statement);
            found.extend(set_config_events(statement));
            found.extend(statement_event(statement));
            found
        })
        .collect()
}

fn resets(event: &Event, name: &str) -> bool {
    match event {
        Event::Reset(reset) => reset == name,
        Event::ResetAll => true,
        Event::Set(_) => false,
    }
}

fn leaks(sql: &str) -> bool {
    let events = events(sql);
    TIMEOUTS.iter().any(|name| {
        let last_set = events.iter().rposition(|event| match event {
            Event::Set(set) => set.as_deref().is_none_or(|set| set == *name),
            _ => false,
        });
        last_set.is_some_and(|at| !events[at..].iter().any(|event| resets(event, name)))
    })
}

fn leaking_files() -> BTreeSet<String> {
    let migrations_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(&migrations_path).expect("migrations directory must be readable") {
        let path = entry.expect("migration entry must be readable").path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("migration file name must be UTF-8: {}", path.display()))
            .to_owned();
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{name} must be readable as UTF-8: {error}"));
        if leaks(&sql) {
            found.insert(name);
        }
    }
    found
}

#[test]
fn session_timeouts_do_not_outlive_their_migration() {
    let applied: BTreeSet<String> = APPLIED_LEAKS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let found = leaking_files();
    assert_eq!(
        found, applied,
        "{RULE} Only the already applied files in APPLIED_LEAKS may keep a leaking SET."
    );
}

#[test]
fn leak_detection_covers_every_form() {
    let cases: &[(&str, &str, bool)] = &[
        ("no timeout", "CREATE TABLE core.t (id BIGINT);", false),
        (
            "session set",
            "SET lock_timeout TO '5s';\nCREATE TABLE core.t (id BIGINT);",
            true,
        ),
        (
            "session keyword",
            "SET SESSION lock_timeout = '5s'; SELECT 1;",
            true,
        ),
        (
            "statement timeout",
            "set statement_timeout to 1000; select 1;",
            true,
        ),
        ("lower case set", "set lock_timeout = '2s'; select 1;", true),
        (
            "set local",
            "SET LOCAL lock_timeout TO '5s'; SELECT 1;",
            false,
        ),
        (
            "set then reset",
            "SET lock_timeout TO '2s'; SELECT 1; RESET lock_timeout;",
            false,
        ),
        (
            "reset before a second set",
            "SET lock_timeout = '2s'; RESET lock_timeout; SET lock_timeout = '3s';",
            true,
        ),
        (
            "other reset",
            "SET lock_timeout = '2s'; RESET statement_timeout;",
            true,
        ),
        (
            "update set column",
            "UPDATE t\nSET lock_timeout_ms = 1;",
            false,
        ),
        ("similar name", "SET lock_timeout_extra TO 1;", false),
        (
            "comment before set",
            "-- keep locks short\nSET lock_timeout TO '5s';\nSELECT 1;",
            true,
        ),
        (
            "block comment before set",
            "/* keep\nlocks short */ SET lock_timeout TO '5s'; SELECT 1;",
            true,
        ),
        (
            "commented out set",
            "-- SET lock_timeout TO '5s';\nSELECT 1;",
            false,
        ),
        (
            "set_config for the session",
            "SELECT set_config('lock_timeout', '5s', false); SELECT 1;",
            true,
        ),
        (
            "set_config for the transaction",
            "SELECT set_config( 'statement_timeout' , '1s' , true ); SELECT 1;",
            false,
        ),
        (
            "set_config then reset",
            "SELECT set_config('lock_timeout', '5s', false); RESET lock_timeout;",
            false,
        ),
        (
            "set_config with a call as value",
            "SELECT set_config('lock_timeout', format('%ss', 5), false); SELECT 1;",
            true,
        ),
        (
            "set_config with named arguments",
            "SELECT set_config(setting_name => 'lock_timeout', new_value => '5s', is_local => false);",
            true,
        ),
        (
            "set_config with named local flag",
            "SELECT set_config(is_local => true, setting_name => 'lock_timeout', new_value => '5s');",
            false,
        ),
        (
            "set_config with a cast flag",
            "SELECT set_config('lock_timeout', '5s', false::boolean);",
            true,
        ),
        (
            "set_config local with a cast",
            "SELECT set_config('lock_timeout', '5s', true::boolean);",
            false,
        ),
        (
            "set_config with a computed name",
            "SELECT set_config(current_setting('x.name'), '5s', false);",
            true,
        ),
        (
            "qualified set_config",
            "SELECT pg_catalog.set_config('lock_timeout', '5s', false);",
            true,
        ),
        (
            "reset inside a string",
            "SET lock_timeout = '2s'; SELECT 'reset lock_timeout';",
            true,
        ),
        (
            "reset inside a function body",
            "SET lock_timeout = '2s'; CREATE FUNCTION f() RETURNS void LANGUAGE sql AS $$ RESET lock_timeout $$;",
            true,
        ),
        (
            "comment marker inside a string",
            "SELECT '--'; SET lock_timeout = '2s'; SELECT 1;",
            true,
        ),
        (
            "block comment marker inside a string",
            "SELECT '/*'; SET lock_timeout = '2s'; SELECT '*/';",
            true,
        ),
        (
            "escaped quote in an escape string",
            "SELECT E'it\\'s -- '; SET lock_timeout = '2s';",
            true,
        ),
        (
            "set inside a do block",
            "DO $body$ BEGIN SET lock_timeout = '2s'; END $body$;",
            true,
        ),
        (
            "set to default",
            "SET lock_timeout = '2s'; SET lock_timeout TO DEFAULT;",
            false,
        ),
        (
            "reset all",
            "SET lock_timeout = '2s'; RESET ALL;",
            false,
        ),
        (
            "nested block comment",
            "/* outer /* inner */ SET lock_timeout = '2s'; */ SELECT 1;",
            false,
        ),
    ];
    for (label, sql, expected) in cases {
        assert_eq!(leaks(sql), *expected, "{label}");
    }
}
