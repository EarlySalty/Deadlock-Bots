use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use dl_central_etl::{
    conversion_for, reconcile_total, run, snapshot_db, source_snapshot_path, text_to_bigint,
    ConvertError, Converter, EngineError, Ledger, LedgerSet, SourceSqlite, DEADLOCK_SQLITE3_SOURCE,
    TOURNAMENT_SOURCE, WEBSITE_SOURCE,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use tempfile::{NamedTempFile, TempDir};

#[test]
fn text_to_bigint_validates_numeric_text_and_i64_range() {
    assert_eq!(
        text_to_bigint("123456789012345678").expect("valid bigint text"),
        123456789012345678
    );
    assert!(matches!(
        text_to_bigint("12x"),
        Err(ConvertError::InvalidBigintText { value }) if value == "12x"
    ));
    assert!(matches!(
        text_to_bigint(" 12"),
        Err(ConvertError::InvalidBigintText { value }) if value == " 12"
    ));
    assert!(matches!(
        text_to_bigint("9223372036854775808"),
        Err(ConvertError::BigintTextOutOfRange { value }) if value == "9223372036854775808"
    ));
}

#[test]
fn unknown_type_pair_is_a_hard_no_converter_error() {
    let err = conversion_for("BLOB", "jsonb").expect_err("BLOB to jsonb is not covered");

    assert!(matches!(
        err,
        EngineError::NoConverter {
            source_affinity,
            target_type,
            ..
        } if source_affinity == "BLOB" && target_type == "jsonb"
    ));
}

#[test]
fn real_schema_type_pairs_have_explicit_converters() {
    assert_eq!(
        conversion_for("TEXT", "date").expect("TEXT to date is covered"),
        Converter::TextToDate
    );
    assert_eq!(
        conversion_for("NUMERIC", "boolean").expect("NUMERIC to boolean is covered"),
        Converter::NumericToBool
    );
    assert_eq!(
        conversion_for("REAL", "timestamp with time zone").expect("REAL to timestamptz is covered"),
        Converter::RealToTimestamptz
    );
    assert_eq!(
        conversion_for("INTEGER", "text").expect("INTEGER to text is covered"),
        Converter::IntegerToText
    );
}

#[test]
fn snapshot_db_copies_without_changing_source_hash_or_mtime() {
    let source = NamedTempFile::new().expect("create source sqlite");
    let conn = Connection::open(source.path()).expect("open source sqlite");
    conn.execute_batch(
        r#"
        CREATE TABLE sample (id INTEGER PRIMARY KEY, name TEXT);
        INSERT INTO sample (id, name) VALUES (1, 'alpha'), (2, 'beta');
        "#,
    )
    .expect("seed source sqlite");
    drop(conn);

    let before_hash = sha256(source.path());
    let before_mtime = fs::metadata(source.path())
        .expect("source metadata before")
        .modified()
        .expect("source mtime before");
    let dest_dir = TempDir::new().expect("create snapshot dir");
    let dest = dest_dir.path().join("snapshot.sqlite3");

    snapshot_db(source.path(), &dest).expect("snapshot succeeds");

    let after_hash = sha256(source.path());
    let after_mtime = fs::metadata(source.path())
        .expect("source metadata after")
        .modified()
        .expect("source mtime after");
    assert_eq!(before_hash, after_hash);
    assert_eq!(before_mtime, after_mtime);

    let snapshot = SourceSqlite::open_read_only(&dest).expect("open snapshot read-only");
    let rows = snapshot.read_rows("sample").expect("read snapshot rows");
    assert_eq!(rows.len(), 2);
}

#[test]
#[ignore]
fn known_source_snapshots_preserve_original_hashes_and_mtimes() {
    let dest_dir = TempDir::new().expect("create known snapshot dir");

    for (source_db, source_path) in [
        ("deadlock-sqlite3", DEADLOCK_SQLITE3_SOURCE),
        ("website", WEBSITE_SOURCE),
        ("tournament", TOURNAMENT_SOURCE),
    ] {
        let source = Path::new(source_path);
        assert!(source.exists(), "source path exists: {}", source.display());
        let before_hash = sha256(source);
        let before_mtime = fs::metadata(source)
            .expect("source metadata before")
            .modified()
            .expect("source mtime before");
        let dest = source_snapshot_path(dest_dir.path(), source_db);

        snapshot_db(source, &dest).expect("known source snapshot succeeds");

        let after_hash = sha256(source);
        let after_mtime = fs::metadata(source)
            .expect("source metadata after")
            .modified()
            .expect("source mtime after");
        assert_eq!(before_hash, after_hash, "{source_db} hash");
        assert_eq!(before_mtime, after_mtime, "{source_db} mtime");
        println!("{source_db}: sha256={before_hash} mtime_unveraendert=true");
    }
}

#[tokio::test]
#[ignore]
async fn tracer_bullet_and_engine_teeth_against_central_db() {
    let pool = test_pool().await;
    clean_targets(&pool).await;

    let snapshot_dir = TempDir::new().expect("create snapshot dir");
    seed_tracer_source(snapshot_dir.path(), "deadlock-sqlite3");
    seed_website_tracer_source(snapshot_dir.path(), "website");
    let ledger_set = LedgerSet {
        sources: BTreeMap::from([
            (
                "deadlock-sqlite3".to_string(),
                Ledger::from_toml_str(TRACER_LEDGER).expect("tracer ledger parses"),
            ),
            (
                "website".to_string(),
                Ledger::from_toml_str(WEBSITE_TRACER_LEDGER).expect("website tracer ledger parses"),
            ),
        ]),
    };

    let report = run(&ledger_set, snapshot_dir.path(), &pool)
        .await
        .expect("tracer ETL run succeeds");
    assert_eq!(report.total_source_rows, 7);
    assert_eq!(report.total_loaded, 7);
    assert_table_counts(
        &aggregate_table_counts(&report.tables),
        &[
            ("activity.member_events", (1, 1)),
            ("coaching.requests", (2, 2)),
            ("core.user_data", (2, 2)),
            ("voice.voice_channel_settings", (2, 2)),
        ],
    );
    reconcile_total(&pool, &report.tables)
        .await
        .expect("initial total reconciliation passes");
    let mut bad_reconcile = report.tables.clone();
    bad_reconcile[0].source_rows += 1;
    assert!(matches!(
        reconcile_total(&pool, &bad_reconcile).await,
        Err(dl_central_etl::VerifyError::TotalReconcileMismatch { .. })
    ));
    assert_tracer_round_trips(&pool).await;

    let counts_before = target_counts(&pool).await;
    let sample_before = idempotence_sample(&pool).await;
    let second = run(&ledger_set, snapshot_dir.path(), &pool)
        .await
        .expect("second ETL run succeeds");
    reconcile_total(&pool, &second.tables)
        .await
        .expect("second total reconciliation passes");
    assert_eq!(counts_before, target_counts(&pool).await);
    assert_eq!(sample_before, idempotence_sample(&pool).await);

    let bad_snapshot_dir = TempDir::new().expect("create bad snapshot dir");
    seed_bad_bigint_source(bad_snapshot_dir.path(), "deadlock-sqlite3");
    let bad_ledger_set = LedgerSet {
        sources: BTreeMap::from([(
            "deadlock-sqlite3".to_string(),
            Ledger::from_toml_str(BAD_BIGINT_LEDGER).expect("bad bigint ledger parses"),
        )]),
    };
    let err = run(&bad_ledger_set, bad_snapshot_dir.path(), &pool)
        .await
        .expect_err("non-numeric TEXT to BIGINT fails");
    assert!(matches!(
        err,
        EngineError::Conversion {
            source_db,
            table,
            column,
            source_pk,
            source,
        } if source_db == "deadlock-sqlite3"
            && table == "user_data"
            && column == "user_id"
            && source_pk == "user_id=not-a-number"
            && matches!(
                source.as_ref(),
                ConvertError::InvalidBigintText { value } if value == "not-a-number"
            )
    ));
}

#[tokio::test]
#[ignore]
async fn no_pk_targets_replace_instead_of_append_against_central_db() {
    let pool = test_pool().await;
    sqlx::query("DELETE FROM bot.schema_version")
        .execute(&pool)
        .await
        .expect("clean no-pk target");

    let snapshot_dir = TempDir::new().expect("create snapshot dir");
    seed_sqlite(
        snapshot_dir.path(),
        "deadlock-sqlite3",
        r#"
        CREATE TABLE schema_version (version INTEGER NOT NULL);
        INSERT INTO schema_version VALUES (7);
        "#,
    );
    let ledger_set = LedgerSet {
        sources: BTreeMap::from([(
            "deadlock-sqlite3".to_string(),
            Ledger::from_toml_str(SCHEMA_VERSION_LEDGER).expect("schema version ledger parses"),
        )]),
    };

    let first = run(&ledger_set, snapshot_dir.path(), &pool)
        .await
        .expect("first no-pk ETL run succeeds");
    assert_eq!(first.total_source_rows, 1);
    assert_eq!(first.total_loaded, 1);
    let count_after_first = table_count(&pool, "bot.schema_version").await;
    assert_eq!(count_after_first, 1);

    let second = run(&ledger_set, snapshot_dir.path(), &pool)
        .await
        .expect("second no-pk ETL run succeeds");
    assert_eq!(second.total_source_rows, 1);
    assert_eq!(second.total_loaded, 1);
    let count_after_second = table_count(&pool, "bot.schema_version").await;

    assert_eq!(count_after_second, count_after_first);
}

async fn test_pool() -> PgPool {
    let dsn = env::var("CENTRAL_TEST_DSN").expect("CENTRAL_TEST_DSN is set by central_test_db.sh");
    assert!(
        !dsn.contains("127.0.0.1:5434") && !dsn.contains("localhost:5434"),
        "refuse to run ETL tests against the production central DB port"
    );
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&dsn)
        .await
        .expect("connect central test db")
}

async fn clean_targets(pool: &PgPool) {
    sqlx::query(
        r#"
        TRUNCATE TABLE
            activity.member_events,
            coaching.requests,
            core.user_data,
            voice.voice_channel_settings
        "#,
    )
    .execute(pool)
    .await
    .expect("truncate tracer targets");
}

fn seed_tracer_source(snapshot_dir: &Path, source_db: &str) {
    seed_sqlite(
        snapshot_dir,
        source_db,
        r#"
        CREATE TABLE voice_channel_settings (
            channel_id INTEGER PRIMARY KEY,
            guild_id INTEGER NOT NULL,
            enabled INTEGER NOT NULL,
            created_at INTEGER,
            updated_at INTEGER
        );
        INSERT INTO voice_channel_settings VALUES
            (111, 10, 1, 1700000000, 1700000060),
            (112, 10, 0, 1700000100, 1700000160);

        CREATE TABLE member_events (
            id INTEGER PRIMARY KEY,
            user_id INTEGER NOT NULL,
            guild_id INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            occurred_at INTEGER,
            metadata TEXT
        );
        INSERT INTO member_events VALUES
            (9001, 42, 10, 'join', 1700000200, '{"mode":"jsonb","ok":true}');

        CREATE TABLE user_data (
            user_id TEXT PRIMARY KEY,
            custom_interval INTEGER,
            paused_until INTEGER,
            created_at INTEGER,
            updated_at INTEGER
        );
        INSERT INTO user_data VALUES
            ('123456789012345678', 15, 1700000300, 1700000000, 1700000400),
            ('223456789012345678', NULL, NULL, 1700000001, 1700000401);

        CREATE TABLE coaching_requests (
            id INTEGER PRIMARY KEY,
            discord_user_id INTEGER NOT NULL,
            rank TEXT NOT NULL,
            subrank TEXT NOT NULL,
            created_at INTEGER
        );
        INSERT INTO coaching_requests VALUES
            (1, 123456789012345678, 'Oracle', '3', 1700000500);
        "#,
    );
}

fn seed_website_tracer_source(snapshot_dir: &Path, source_db: &str) {
    seed_sqlite(
        snapshot_dir,
        source_db,
        r#"
        CREATE TABLE coaching_requests (
            id TEXT PRIMARY KEY,
            discord_user_id INTEGER NOT NULL,
            rank TEXT NOT NULL,
            subrank TEXT NOT NULL,
            created_at TIMESTAMP
        );
        INSERT INTO coaching_requests VALUES
            ('web-1', 223456789012345678, 'Phantom', '5', '2023-11-14 22:25:00');
        "#,
    );
}

fn seed_bad_bigint_source(snapshot_dir: &Path, source_db: &str) {
    seed_sqlite(
        snapshot_dir,
        source_db,
        r#"
        CREATE TABLE user_data (
            user_id TEXT PRIMARY KEY,
            custom_interval INTEGER,
            paused_until INTEGER,
            created_at INTEGER,
            updated_at INTEGER
        );
        INSERT INTO user_data VALUES
            ('not-a-number', 15, NULL, 1700000000, 1700000400);
        "#,
    );
}

fn seed_sqlite(snapshot_dir: &Path, source_db: &str, sql: &str) -> PathBuf {
    fs::create_dir_all(snapshot_dir).expect("create snapshot dir");
    let path = source_snapshot_path(snapshot_dir, source_db);
    let conn = Connection::open(&path).expect("open sqlite fixture");
    conn.execute_batch(sql).expect("seed sqlite fixture");
    path
}

async fn assert_tracer_round_trips(pool: &PgPool) {
    let row = sqlx::query(
        r#"
        SELECT enabled, EXTRACT(EPOCH FROM created_at)::bigint AS created_at
        FROM voice.voice_channel_settings
        WHERE channel_id = $1
        "#,
    )
    .bind(111_i64)
    .fetch_one(pool)
    .await
    .expect("voice row exists");
    assert!(row.try_get::<bool, _>("enabled").expect("enabled bool"));
    assert_eq!(
        row.try_get::<i64, _>("created_at")
            .expect("created_at epoch"),
        1_700_000_000
    );

    let row = sqlx::query(
        r#"
        SELECT metadata->>'mode' AS mode, metadata->>'ok' AS ok
        FROM activity.member_events
        WHERE id = $1
        "#,
    )
    .bind(9001_i64)
    .fetch_one(pool)
    .await
    .expect("jsonb row exists");
    assert_eq!(
        row.try_get::<String, _>("mode").expect("json mode"),
        "jsonb"
    );
    assert_eq!(row.try_get::<String, _>("ok").expect("json ok"), "true");

    let row = sqlx::query(
        r#"
        SELECT user_id, custom_interval, EXTRACT(EPOCH FROM paused_until)::bigint AS paused_until
        FROM core.user_data
        WHERE user_id = $1
        "#,
    )
    .bind(123456789012345678_i64)
    .fetch_one(pool)
    .await
    .expect("user_data row exists");
    assert_eq!(
        row.try_get::<i64, _>("user_id").expect("user_id bigint"),
        123456789012345678
    );
    assert_eq!(
        row.try_get::<i32, _>("custom_interval")
            .expect("custom_interval int4"),
        15
    );
    assert_eq!(
        row.try_get::<i64, _>("paused_until")
            .expect("paused_until epoch"),
        1_700_000_300
    );

    let row = sqlx::query(
        r#"
        SELECT request_uid, bot_request_id, discord_user_id, rank, subrank
        FROM coaching.requests
        WHERE request_uid = $1
        "#,
    )
    .bind("deadlock-sqlite3:coaching_requests:1")
    .fetch_one(pool)
    .await
    .expect("bot union target row exists");
    assert_eq!(
        row.try_get::<String, _>("request_uid")
            .expect("request_uid text"),
        "deadlock-sqlite3:coaching_requests:1"
    );
    assert_eq!(
        row.try_get::<i32, _>("bot_request_id")
            .expect("bot_request_id int4"),
        1
    );
    assert_eq!(
        row.try_get::<i64, _>("discord_user_id")
            .expect("discord_user_id int8"),
        123456789012345678
    );
    assert_eq!(
        row.try_get::<String, _>("rank").expect("rank text"),
        "Oracle"
    );
    assert_eq!(
        row.try_get::<String, _>("subrank").expect("subrank text"),
        "3"
    );

    let row = sqlx::query(
        r#"
        SELECT
            request_uid,
            website_request_id,
            discord_user_id,
            rank,
            subrank,
            EXTRACT(EPOCH FROM created_at)::bigint AS created_at
        FROM coaching.requests
        WHERE request_uid = $1
        "#,
    )
    .bind("website:coaching_requests:web-1")
    .fetch_one(pool)
    .await
    .expect("website union target row exists");
    assert_eq!(
        row.try_get::<String, _>("request_uid")
            .expect("request_uid text"),
        "website:coaching_requests:web-1"
    );
    assert_eq!(
        row.try_get::<String, _>("website_request_id")
            .expect("website_request_id text"),
        "web-1"
    );
    assert_eq!(
        row.try_get::<i64, _>("discord_user_id")
            .expect("discord_user_id int8"),
        223456789012345678
    );
    assert_eq!(
        row.try_get::<String, _>("rank").expect("rank text"),
        "Phantom"
    );
    assert_eq!(
        row.try_get::<String, _>("subrank").expect("subrank text"),
        "5"
    );
    assert_eq!(
        row.try_get::<i64, _>("created_at")
            .expect("created_at epoch"),
        1_700_000_700
    );
}

async fn target_counts(pool: &PgPool) -> BTreeMap<String, i64> {
    let mut counts = BTreeMap::new();
    for target in [
        "activity.member_events",
        "coaching.requests",
        "core.user_data",
        "voice.voice_channel_settings",
    ] {
        let sql = format!("SELECT COUNT(*)::bigint AS count FROM {target}");
        let row = sqlx::query(&sql)
            .fetch_one(pool)
            .await
            .expect("count target rows");
        counts.insert(
            target.to_string(),
            row.try_get::<i64, _>("count").expect("count as i64"),
        );
    }
    counts
}

async fn table_count(pool: &PgPool, target: &str) -> i64 {
    let sql = format!("SELECT COUNT(*)::bigint AS count FROM {target}");
    let row = sqlx::query(&sql)
        .fetch_one(pool)
        .await
        .expect("count target rows");
    row.try_get::<i64, _>("count").expect("count as i64")
}

async fn idempotence_sample(pool: &PgPool) -> BTreeMap<String, String> {
    let mut sample = BTreeMap::new();
    for (name, sql) in [
        (
            "voice",
            "SELECT enabled::text || ':' || EXTRACT(EPOCH FROM updated_at)::bigint::text AS sample FROM voice.voice_channel_settings WHERE channel_id = 111",
        ),
        (
            "jsonb",
            "SELECT metadata::text AS sample FROM activity.member_events WHERE id = 9001",
        ),
        (
            "bigint",
            "SELECT user_id::text || ':' || COALESCE(custom_interval::text, 'NULL') AS sample FROM core.user_data WHERE user_id = 123456789012345678",
        ),
        (
            "union",
            "SELECT string_agg(request_uid || ':' || discord_user_id::text || ':' || rank, ',' ORDER BY request_uid) AS sample FROM coaching.requests",
        ),
    ] {
        let row = sqlx::query(sql)
            .fetch_one(pool)
            .await
            .expect("sample row exists");
        sample.insert(
            name.to_string(),
            row.try_get::<String, _>("sample").expect("sample as text"),
        );
    }
    sample
}

fn aggregate_table_counts(tables: &[dl_central_etl::TableResult]) -> BTreeMap<String, (u64, u64)> {
    let mut counts = BTreeMap::<String, (u64, u64)>::new();
    for table in tables {
        let entry = counts.entry(table.target.clone()).or_default();
        entry.0 += table.source_rows;
        entry.1 += table.loaded_rows;
    }
    counts
}

fn assert_table_counts(actual: &BTreeMap<String, (u64, u64)>, expected: &[(&str, (u64, u64))]) {
    for (target, counts) in expected {
        assert_eq!(actual.get(*target), Some(counts), "{target}");
    }
}

fn sha256(path: &Path) -> String {
    let bytes = fs::read(path).expect("read file for sha256");
    format!("{:x}", Sha256::digest(bytes))
}

const TRACER_LEDGER: &str = r#"
[tables.voice_channel_settings.columns.channel_id]
status = "mapped"
to = "voice.voice_channel_settings.channel_id"

[tables.voice_channel_settings.columns.guild_id]
status = "mapped"
to = "voice.voice_channel_settings.guild_id"

[tables.voice_channel_settings.columns.enabled]
status = "mapped"
to = "voice.voice_channel_settings.enabled"

[tables.voice_channel_settings.columns.created_at]
status = "mapped"
to = "voice.voice_channel_settings.created_at"

[tables.voice_channel_settings.columns.updated_at]
status = "mapped"
to = "voice.voice_channel_settings.updated_at"

[tables.member_events.columns.id]
status = "mapped"
to = "activity.member_events.id"

[tables.member_events.columns.user_id]
status = "mapped"
to = "activity.member_events.user_id"

[tables.member_events.columns.guild_id]
status = "mapped"
to = "activity.member_events.guild_id"

[tables.member_events.columns.event_type]
status = "mapped"
to = "activity.member_events.event_type"

[tables.member_events.columns.occurred_at]
status = "mapped"
to = "activity.member_events.occurred_at"

[tables.member_events.columns.metadata]
status = "mapped"
to = "activity.member_events.metadata"

[tables.user_data.columns.user_id]
status = "mapped"
to = "core.user_data.user_id"

[tables.user_data.columns.custom_interval]
status = "mapped"
to = "core.user_data.custom_interval"

[tables.user_data.columns.paused_until]
status = "mapped"
to = "core.user_data.paused_until"

[tables.user_data.columns.created_at]
status = "mapped"
to = "core.user_data.created_at"

[tables.user_data.columns.updated_at]
status = "mapped"
to = "core.user_data.updated_at"

[tables.coaching_requests.columns.id]
status = "mapped"
to = "coaching.requests.bot_request_id"

[tables.coaching_requests.columns.discord_user_id]
status = "mapped"
to = "coaching.requests.discord_user_id"

[tables.coaching_requests.columns.rank]
status = "mapped"
to = "coaching.requests.rank"

[tables.coaching_requests.columns.subrank]
status = "mapped"
to = "coaching.requests.subrank"

[tables.coaching_requests.columns.created_at]
status = "mapped"
to = "coaching.requests.created_at"
"#;

const WEBSITE_TRACER_LEDGER: &str = r#"
[tables.coaching_requests.columns.id]
status = "mapped"
to = "coaching.requests.website_request_id"

[tables.coaching_requests.columns.discord_user_id]
status = "mapped"
to = "coaching.requests.discord_user_id"

[tables.coaching_requests.columns.rank]
status = "mapped"
to = "coaching.requests.rank"

[tables.coaching_requests.columns.subrank]
status = "mapped"
to = "coaching.requests.subrank"

[tables.coaching_requests.columns.created_at]
status = "mapped"
to = "coaching.requests.created_at"
"#;

const BAD_BIGINT_LEDGER: &str = r#"
[tables.user_data.columns.user_id]
status = "mapped"
to = "core.user_data.user_id"

[tables.user_data.columns.custom_interval]
status = "mapped"
to = "core.user_data.custom_interval"

[tables.user_data.columns.paused_until]
status = "mapped"
to = "core.user_data.paused_until"

[tables.user_data.columns.created_at]
status = "mapped"
to = "core.user_data.created_at"

[tables.user_data.columns.updated_at]
status = "mapped"
to = "core.user_data.updated_at"
"#;

const SCHEMA_VERSION_LEDGER: &str = r#"
[tables.schema_version.columns.version]
status = "mapped"
to = "bot.schema_version.version"
"#;
