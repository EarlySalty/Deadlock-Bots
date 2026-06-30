use std::{collections::BTreeMap, fs};

use dl_central_etl::{
    check_ledger_set_mapping_completeness, check_mapping_completeness, check_row_counts,
    check_row_counts_with_factor, check_sample_covers_mapped_columns, optional_sqlite_int_to_bool,
    optional_text_json_to_value, optional_unix_seconds_to_datetime, sample_round_trip,
    sqlite_int_to_bool, text_json_to_value, unix_seconds_to_datetime, Ledger, LedgerError,
    LedgerSet, RoundTripFields, RoundTripRows, RowCountFactor, SourceError, SourceSchemas,
    SourceSqlite, SourceTableColumns, VerifyError,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use tempfile::NamedTempFile;

fn source_columns() -> Vec<String> {
    [
        "discord_id",
        "username",
        "created_at",
        "is_verified",
        "raw_json",
        "legacy_note",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn fields(entries: &[(&str, Value)]) -> RoundTripFields {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn table_columns(entries: &[(&str, &[&str])]) -> SourceTableColumns {
    entries
        .iter()
        .map(|(table, columns)| {
            (
                (*table).to_string(),
                columns.iter().map(|column| (*column).to_string()).collect(),
            )
        })
        .collect()
}

#[test]
fn mapping_completeness_rejects_unmapped_source_column() {
    let ledger = Ledger::from_toml_str(include_str!("fixtures/missing-column-ledger.toml"))
        .expect("fixture ledger parses");
    let table = ledger
        .table("fixture_users")
        .expect("fixture table ledger exists");

    let result = check_mapping_completeness(&source_columns(), table);

    assert_eq!(
        result,
        Err(VerifyError::UnmappedColumn {
            column: "legacy_note".to_string()
        })
    );
}

#[test]
fn mapped_columns_must_be_present_in_every_round_trip_sample_row() {
    let ledger = Ledger::from_toml_str(include_str!("fixtures/example-ledger.toml"))
        .expect("fixture ledger parses");
    let table = ledger
        .table("fixture_users")
        .expect("fixture table ledger exists");
    let mut sample = RoundTripRows::new();
    sample.insert(
        "42".to_string(),
        fields(&[
            ("discord_id", json!(42)),
            ("username", json!("alice")),
            ("created_at", json!(1_700_000_000)),
            ("is_verified", json!(true)),
        ]),
    );

    let result = check_sample_covers_mapped_columns(table, &sample);

    assert_eq!(
        result,
        Err(VerifyError::MappedColumnMissingFromSample {
            column: "raw_json".to_string(),
            row_key: "42".to_string()
        })
    );
}

#[test]
fn row_count_mismatch_returns_error() {
    let result = check_row_counts(3, 2);

    assert_eq!(
        result,
        Err(VerifyError::RowCountMismatch {
            expected: 3,
            actual: 2
        })
    );
}

#[test]
fn row_count_factor_rejects_fractional_expected_count_masked_by_integer_division() {
    let result = check_row_counts_with_factor(
        3,
        1,
        RowCountFactor {
            numerator: 1,
            denominator: 2,
        },
    );

    assert_eq!(
        result,
        Err(VerifyError::RowCountMismatch {
            expected: 2,
            actual: 1
        })
    );
}

#[test]
fn row_count_factor_accepts_exact_scaled_count() {
    let result = check_row_counts_with_factor(
        4,
        2,
        RowCountFactor {
            numerator: 1,
            denominator: 2,
        },
    );

    assert_eq!(result, Ok(()));
}

#[test]
fn row_count_factor_rejects_zero_denominator() {
    let result = check_row_counts_with_factor(
        4,
        2,
        RowCountFactor {
            numerator: 1,
            denominator: 0,
        },
    );

    assert_eq!(
        result,
        Err(VerifyError::InvalidRowCountFactor {
            numerator: 1,
            denominator: 0
        })
    );
}

#[test]
fn row_count_factor_rejects_zero_numerator() {
    let result = check_row_counts_with_factor(
        4,
        0,
        RowCountFactor {
            numerator: 0,
            denominator: 2,
        },
    );

    assert_eq!(
        result,
        Err(VerifyError::InvalidRowCountFactor {
            numerator: 0,
            denominator: 2
        })
    );
}

#[test]
fn complete_fixture_passes_mapping_counts_and_round_trip() {
    let ledger = Ledger::from_toml_str(include_str!("fixtures/example-ledger.toml"))
        .expect("fixture ledger parses");
    let table = ledger
        .table("fixture_users")
        .expect("fixture table ledger exists");

    check_mapping_completeness(&source_columns(), table).expect("all source columns covered");
    check_row_counts(2, 2).expect("matching row counts pass");

    let mut source = RoundTripRows::new();
    source.insert(
        "42".to_string(),
        fields(&[
            ("discord_id", json!(42)),
            ("username", json!("alice")),
            ("created_at", json!(1_700_000_000)),
            ("is_verified", json!(true)),
            ("raw_json", json!(r#"{"source":"fixture-a"}"#)),
        ]),
    );
    source.insert(
        "43".to_string(),
        fields(&[
            ("discord_id", json!(43)),
            ("username", json!("bob")),
            ("created_at", json!(1_700_000_100)),
            ("is_verified", json!(false)),
            ("raw_json", json!(r#"{"source":"fixture-b"}"#)),
        ]),
    );

    let target = source.clone();

    check_sample_covers_mapped_columns(table, &source).expect("sample covers all mapped columns");
    sample_round_trip(&source, &target).expect("matching samples pass");
}

#[test]
fn ledger_set_keeps_same_table_names_source_qualified_and_fails_missing_source_column() {
    let dir = tempfile::tempdir().expect("create temp ledger root");
    let deadlock_dir = dir.path().join("deadlock-sqlite3");
    let website_dir = dir.path().join("website");
    fs::create_dir_all(&deadlock_dir).expect("create deadlock ledger dir");
    fs::create_dir_all(&website_dir).expect("create website ledger dir");

    fs::write(
        deadlock_dir.join("coaching.toml"),
        r#"
        [tables.coaching_requests.columns.id]
        status = "mapped"
        to = "coaching.requests.bot_request_id"

        [tables.coaching_requests.columns.discord_user_id]
        status = "mapped"
        to = "coaching.requests.discord_user_id"

        [tables.coaching_requests.columns.message_id]
        status = "mapped"
        to = "coaching.requests.message_id"
        "#,
    )
    .expect("write deadlock ledger");
    fs::write(
        website_dir.join("coaching.toml"),
        r#"
        [tables.coaching_requests.columns.id]
        status = "mapped"
        to = "coaching.requests.website_request_id"

        [tables.coaching_requests.columns.discord_user_id]
        status = "mapped"
        to = "coaching.requests.discord_user_id"

        [tables.coaching_requests.columns.bot_request_id]
        status = "mapped"
        to = "coaching.requests.bot_request_id"
        "#,
    )
    .expect("write website ledger");

    let ledger_set = LedgerSet::from_dir(dir.path()).expect("ledger set loads");
    assert!(ledger_set
        .source("deadlock-sqlite3")
        .expect("deadlock source")
        .table("coaching_requests")
        .is_some());
    assert!(ledger_set
        .source("website")
        .expect("website source")
        .table("coaching_requests")
        .is_some());

    let source_schemas = SourceSchemas::from([
        (
            "deadlock-sqlite3".to_string(),
            table_columns(&[(
                "coaching_requests",
                &["id", "discord_user_id", "message_id"],
            )]),
        ),
        (
            "website".to_string(),
            table_columns(&[(
                "coaching_requests",
                &["id", "discord_user_id", "bot_request_id"],
            )]),
        ),
    ]);

    check_ledger_set_mapping_completeness(&source_schemas, &ledger_set)
        .expect("same table name in two source DBs is checked separately");

    fs::write(
        website_dir.join("coaching.toml"),
        r#"
        [tables.coaching_requests.columns.id]
        status = "mapped"
        to = "coaching.requests.website_request_id"

        [tables.coaching_requests.columns.discord_user_id]
        status = "mapped"
        to = "coaching.requests.discord_user_id"
        "#,
    )
    .expect("remove one website source column from ledger");

    let broken_ledger_set = LedgerSet::from_dir(dir.path()).expect("broken ledger still parses");
    let result = check_ledger_set_mapping_completeness(&source_schemas, &broken_ledger_set);

    assert_eq!(
        result,
        Err(VerifyError::UnmappedSourceColumn {
            source_db: "website".to_string(),
            table: "coaching_requests".to_string(),
            column: "bot_request_id".to_string(),
        })
    );
}

#[test]
fn t1_ledger_fragments_cover_core_source_columns() {
    let ledger_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ledger");
    let ledger_set = LedgerSet::from_dir(ledger_root).expect("T1 ledger directory loads");

    let source_schemas = SourceSchemas::from([
        (
            "deadlock-sqlite3".to_string(),
            table_columns(&[
                (
                    "steam_links",
                    &[
                        "user_id",
                        "steam_id",
                        "name",
                        "verified",
                        "primary_account",
                        "created_at",
                        "updated_at",
                        "legacy_ref",
                        "migrated_at",
                        "deadlock_rank",
                        "deadlock_rank_name",
                        "deadlock_subrank",
                        "deadlock_badge_level",
                        "deadlock_rank_updated_at",
                        "is_steam_friend",
                    ],
                ),
                (
                    "user_privacy",
                    &["user_id", "opted_out", "deleted_at", "reason", "updated_at"],
                ),
            ]),
        ),
        (
            "website".to_string(),
            table_columns(&[(
                "meta_users",
                &[
                    "id",
                    "username",
                    "display_name",
                    "avatar_url",
                    "role",
                    "created_at",
                ],
            )]),
        ),
    ]);

    check_ledger_set_mapping_completeness(&source_schemas, &ledger_set)
        .expect("T1 core ledger fragments cover all source columns");
}

#[test]
fn ledger_rejects_typoed_table_root() {
    let result = Ledger::from_toml_str(
        r#"
        [table.fixture_users.columns.discord_id]
        status = "mapped"
        to = "core.users.discord_id"
        "#,
    );

    assert!(matches!(result, Err(LedgerError::Toml(_))));
}

#[test]
fn ledger_rejects_unknown_table_field() {
    let result = Ledger::from_toml_str(
        r#"
        [tables.fixture_users]
        unknown = true
        "#,
    );

    assert!(matches!(result, Err(LedgerError::Toml(_))));
}

#[test]
fn ledger_rejects_empty_toml() {
    let result = Ledger::from_toml_str("");

    assert!(matches!(result, Err(LedgerError::EmptyLedger)));
}

#[test]
fn ledger_rejects_table_without_columns() {
    let result = Ledger::from_toml_str("[tables.fixture_users]\n");

    assert!(
        matches!(result, Err(LedgerError::EmptyTableColumns { table }) if table == "fixture_users")
    );
}

#[test]
fn sample_round_trip_rejects_missing_target_row() {
    let mut source = RoundTripRows::new();
    source.insert("42".to_string(), fields(&[("username", json!("alice"))]));
    let target = RoundTripRows::new();

    let result = sample_round_trip(&source, &target);

    assert_eq!(
        result,
        Err(VerifyError::RoundTripMismatch {
            key: "42".to_string(),
            field: "<row>".to_string(),
            source_value: Some(json!({"username": "alice"})),
            target_value: None,
        })
    );
}

#[test]
fn sample_round_trip_rejects_extra_target_row() {
    let source = RoundTripRows::new();
    let mut target = RoundTripRows::new();
    target.insert("99".to_string(), fields(&[("username", json!("eve"))]));

    let result = sample_round_trip(&source, &target);

    assert_eq!(
        result,
        Err(VerifyError::RoundTripMismatch {
            key: "99".to_string(),
            field: "<row>".to_string(),
            source_value: None,
            target_value: Some(json!({"username": "eve"})),
        })
    );
}

#[test]
fn sample_round_trip_rejects_missing_target_field() {
    let mut source = RoundTripRows::new();
    source.insert("42".to_string(), fields(&[("username", json!("alice"))]));
    let mut target = RoundTripRows::new();
    target.insert("42".to_string(), fields(&[]));

    let result = sample_round_trip(&source, &target);

    assert_eq!(
        result,
        Err(VerifyError::RoundTripMismatch {
            key: "42".to_string(),
            field: "username".to_string(),
            source_value: Some(json!("alice")),
            target_value: None,
        })
    );
}

#[test]
fn sample_round_trip_rejects_extra_target_field() {
    let mut source = RoundTripRows::new();
    source.insert("42".to_string(), fields(&[("username", json!("alice"))]));
    let mut target = RoundTripRows::new();
    target.insert(
        "42".to_string(),
        fields(&[
            ("raw_json", json!(r#"{"source":"target-only"}"#)),
            ("username", json!("alice")),
        ]),
    );

    let result = sample_round_trip(&source, &target);

    assert_eq!(
        result,
        Err(VerifyError::RoundTripMismatch {
            key: "42".to_string(),
            field: "raw_json".to_string(),
            source_value: None,
            target_value: Some(json!(r#"{"source":"target-only"}"#)),
        })
    );
}

#[test]
fn sample_round_trip_rejects_changed_field_value() {
    let mut source = RoundTripRows::new();
    source.insert("42".to_string(), fields(&[("username", json!("alice"))]));
    let mut target = RoundTripRows::new();
    target.insert("42".to_string(), fields(&[("username", json!("bob"))]));

    let result = sample_round_trip(&source, &target);

    assert_eq!(
        result,
        Err(VerifyError::RoundTripMismatch {
            key: "42".to_string(),
            field: "username".to_string(),
            source_value: Some(json!("alice")),
            target_value: Some(json!("bob")),
        })
    );
}

#[test]
fn converters_preserve_types_and_null_semantics() {
    let timestamp = unix_seconds_to_datetime(1_700_000_000).expect("valid unix timestamp");
    assert_eq!(timestamp.timestamp(), 1_700_000_000);
    assert!(matches!(
        unix_seconds_to_datetime(i64::MAX),
        Err(dl_central_etl::ConvertError::InvalidUnixTimestamp { seconds })
        if seconds == i64::MAX
    ));
    assert_eq!(
        optional_unix_seconds_to_datetime(None).expect("null timestamp is allowed"),
        None
    );

    assert!(!sqlite_int_to_bool(0).expect("0 converts to false"));
    assert!(sqlite_int_to_bool(1).expect("1 converts to true"));
    assert!(matches!(
        sqlite_int_to_bool(2),
        Err(dl_central_etl::ConvertError::InvalidBoolInteger { value }) if value == 2
    ));
    assert_eq!(
        optional_sqlite_int_to_bool(None).expect("null bool is allowed"),
        None
    );

    assert_eq!(
        text_json_to_value(r#"{"roles":["coach"],"active":true}"#).expect("valid json parses"),
        json!({"roles": ["coach"], "active": true})
    );
    assert!(matches!(
        text_json_to_value("{not valid json"),
        Err(dl_central_etl::ConvertError::Json(_))
    ));
    assert_eq!(
        optional_text_json_to_value(None).expect("null json is allowed"),
        None
    );
}

#[test]
fn source_reader_lists_columns_and_reads_json_rows_from_snapshot_file() {
    let file = NamedTempFile::new().expect("create temp sqlite file");
    let conn = Connection::open(file.path()).expect("open temp sqlite for seeding");
    conn.execute_batch(
        r#"
        CREATE TABLE fixture_users (
            discord_id INTEGER NOT NULL,
            username TEXT,
            created_at INTEGER,
            is_verified INTEGER,
            raw_json TEXT,
            legacy_note TEXT
        );

        INSERT INTO fixture_users (
            discord_id, username, created_at, is_verified, raw_json, legacy_note
        ) VALUES (
            42, 'alice', 1700000000, 1, '{"source":"fixture"}', NULL
        );
        "#,
    )
    .expect("seed temp sqlite fixture");
    drop(conn);

    let source = SourceSqlite::open_read_only(file.path()).expect("open sqlite read-only");
    let columns = source
        .table_columns("fixture_users")
        .expect("list source columns");
    assert_eq!(columns, source_columns());

    let rows = source.read_rows("fixture_users").expect("read source rows");
    assert_eq!(rows.len(), 1);

    let expected = BTreeMap::from([
        ("discord_id".to_string(), json!(42)),
        ("username".to_string(), json!("alice")),
        ("created_at".to_string(), json!(1_700_000_000)),
        ("is_verified".to_string(), json!(1)),
        ("raw_json".to_string(), json!(r#"{"source":"fixture"}"#)),
        ("legacy_note".to_string(), Value::Null),
    ]);
    assert_eq!(rows[0].values, expected);
}

#[test]
fn source_reader_keeps_finite_real_values() {
    let file = NamedTempFile::new().expect("create temp sqlite file");
    let conn = Connection::open(file.path()).expect("open temp sqlite for seeding");
    conn.execute_batch(
        r#"
        CREATE TABLE fixture_values (value REAL);
        INSERT INTO fixture_values (value) VALUES (1.25);
        "#,
    )
    .expect("seed temp sqlite fixture");
    drop(conn);

    let source = SourceSqlite::open_read_only(file.path()).expect("open sqlite read-only");
    let rows = source
        .read_rows("fixture_values")
        .expect("finite real values are readable");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].values.get("value"), Some(&json!(1.25)));
}

#[test]
fn source_reader_rejects_non_finite_real_values() {
    let file = NamedTempFile::new().expect("create temp sqlite file");
    let conn = Connection::open(file.path()).expect("open temp sqlite for seeding");
    conn.execute_batch(
        r#"
        CREATE TABLE fixture_values (value REAL);
        INSERT INTO fixture_values (value) VALUES (1e308 * 10);
        "#,
    )
    .expect("seed temp sqlite fixture");
    drop(conn);

    let source = SourceSqlite::open_read_only(file.path()).expect("open sqlite read-only");
    let result = source.read_rows("fixture_values");

    assert!(matches!(result, Err(SourceError::NonFiniteReal { value }) if value.is_infinite()));
}

#[test]
fn source_reader_rejects_invalid_utf8_text() {
    let file = NamedTempFile::new().expect("create temp sqlite file");
    let conn = Connection::open(file.path()).expect("open temp sqlite for seeding");
    conn.execute_batch(
        r#"
        CREATE TABLE fixture_values (value TEXT);
        INSERT INTO fixture_values (value) VALUES (CAST(x'ff' AS TEXT));
        "#,
    )
    .expect("seed temp sqlite fixture");
    drop(conn);

    let source = SourceSqlite::open_read_only(file.path()).expect("open sqlite read-only");
    let result = source.read_rows("fixture_values");

    assert!(matches!(result, Err(SourceError::InvalidUtf8Text)));
}
