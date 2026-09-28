use chrono::{Duration, Utc};
use dl_central_db::steam_web_api_ledger::{
    observe, reserve, CallerClass, DenialReason, LedgerError, Reservation,
};
use dl_central_db::test_pool;
use sqlx::PgPool;

#[tokio::test]
#[ignore = "requires central_test_db.sh"]
async fn rolling_caps_are_atomic_under_competing_callers() {
    let db = test_pool().await.expect("isolated migrated database");
    sqlx::query(
        "INSERT INTO steam.web_api_reservations (caller, caller_class)
         SELECT 'patchnotes', 'optional_patch' FROM generate_series(1, 89999)",
    )
    .execute(db.pool())
    .await
    .expect("seed optional usage");
    sqlx::query(
        "INSERT INTO steam.web_api_reservations (caller, caller_class)
         SELECT 'brain', 'standard' FROM generate_series(1, 10000)",
    )
    .execute(db.pool())
    .await
    .expect("seed standard usage");

    let tasks: Vec<_> = (0..40)
        .map(|index| {
            let pool = db.pool().clone();
            tokio::spawn(async move {
                let class = if index % 2 == 0 {
                    CallerClass::OptionalPatch
                } else {
                    CallerClass::Standard
                };
                reserve(&pool, "race", class).await.expect("reservation")
            })
        })
        .collect();
    let mut granted = 0;
    for task in tasks {
        match task.await.expect("join") {
            Reservation::Granted { .. } => granted += 1,
            Reservation::Denied { reason, .. } => assert!(matches!(
                reason,
                DenialReason::GlobalCap | DenialReason::OptionalCap
            )),
        }
    }
    assert_eq!(granted, 1);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steam.web_api_reservations
          WHERE reserved_at > clock_timestamp() - interval '24 hours'",
    )
    .fetch_one(db.pool())
    .await
    .expect("count");
    assert_eq!(count, 100_000);

    sqlx::query("TRUNCATE steam.web_api_reservations RESTART IDENTITY")
        .execute(db.pool())
        .await
        .expect("clear");
    sqlx::query(
        "INSERT INTO steam.web_api_reservations (caller, caller_class)
         SELECT 'patchnotes', 'optional_patch' FROM generate_series(1, 90000)",
    )
    .execute(db.pool())
    .await
    .expect("seed optional cap");
    assert!(matches!(
        reserve(db.pool(), "patchnotes", CallerClass::OptionalPatch)
            .await
            .expect("optional reservation"),
        Reservation::Denied {
            reason: DenialReason::OptionalCap,
            ..
        }
    ));
    assert!(matches!(
        reserve(db.pool(), "brain", CallerClass::Standard)
            .await
            .expect("protected headroom"),
        Reservation::Granted { .. }
    ));

    sqlx::query("TRUNCATE steam.web_api_reservations RESTART IDENTITY")
        .execute(db.pool())
        .await
        .expect("clear");
    sqlx::query(
        "INSERT INTO steam.web_api_reservations (caller, caller_class, reserved_at)
         VALUES ('patchnotes', 'optional_patch', clock_timestamp() - interval '25 hours')",
    )
    .execute(db.pool())
    .await
    .expect("seed expired usage");
    assert!(matches!(
        reserve(db.pool(), "patchnotes", CallerClass::OptionalPatch)
            .await
            .expect("rolling window"),
        Reservation::Granted { .. }
    ));
    let pending_id: i64 = sqlx::query_scalar(
        "INSERT INTO steam.web_api_reservations (caller, caller_class, reserved_at)
         VALUES ('retry', 'standard', clock_timestamp() - interval '72 hours') RETURNING id",
    )
    .fetch_one(db.pool())
    .await
    .expect("seed unreported attempt");
    let completed_id: i64 = sqlx::query_scalar(
        "INSERT INTO steam.web_api_reservations
            (caller, caller_class, reserved_at, response_at, http_status)
         VALUES ('brain', 'standard', clock_timestamp() - interval '72 hours',
                 clock_timestamp() - interval '72 hours', 200) RETURNING id",
    )
    .fetch_one(db.pool())
    .await
    .expect("seed completed attempt");
    reserve(db.pool(), "brain", CallerClass::Standard)
        .await
        .expect("new reservation after old attempts");
    let pending_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM steam.web_api_reservations WHERE id = $1)")
            .bind(pending_id)
            .fetch_one(db.pool())
            .await
            .expect("pending attempt exists");
    assert!(pending_exists);
    let completed = observe(db.pool(), completed_id, Some(200), None)
        .await
        .expect("retry old completed observation");
    assert!(completed.duplicate);
    assert!(observe(db.pool(), pending_id, Some(429), Some("90"))
        .await
        .is_ok());
}

#[tokio::test]
#[ignore = "requires central_test_db.sh"]
async fn cooldown_and_response_survive_new_client_and_count_failed_attempts() {
    let db = test_pool().await.expect("isolated migrated database");
    let Reservation::Granted { id, .. } =
        reserve(db.pool(), "patchnotes", CallerClass::OptionalPatch)
            .await
            .expect("first reservation")
    else {
        panic!("first reservation must be granted");
    };
    let first = observe(db.pool(), id, Some(429), Some("90"))
        .await
        .expect("observe 429");
    assert!(!first.duplicate);
    assert_eq!(
        first.cooldown_until.expect("cooldown"),
        first.response_at + Duration::seconds(90)
    );
    let again = observe(db.pool(), id, Some(429), Some("90"))
        .await
        .expect("duplicate report");
    assert!(again.duplicate);
    assert_eq!(again.response_at, first.response_at);
    assert!(matches!(
        observe(db.pool(), id, Some(200), None).await,
        Err(LedgerError::ConflictingReport)
    ));
    let alternate_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.pool().connect_options().as_ref().clone())
        .await
        .expect("independent client pool");
    assert!(matches!(
        reserve(&alternate_pool, "brain", CallerClass::Standard)
            .await
            .expect("restart-safe cooldown"),
        Reservation::Denied {
            reason: DenialReason::Cooldown,
            ..
        }
    ));
    let (status, response_at): (Option<i16>, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
        "SELECT http_status, response_at FROM steam.web_api_reservations WHERE id = $1",
    )
    .bind(id)
    .fetch_one(db.pool())
    .await
    .expect("persisted response");
    assert_eq!(status, Some(429));
    assert_eq!(response_at, Some(first.response_at));
    sqlx::query(
        "UPDATE steam.web_api_budget SET cooldown_until = clock_timestamp() - interval '1 second'",
    )
    .execute(db.pool())
    .await
    .expect("expire cooldown");
    let Reservation::Granted { id: second_id, .. } =
        reserve(db.pool(), "brain", CallerClass::Standard)
            .await
            .expect("next request")
    else {
        panic!("cooldown should have expired");
    };
    let result = observe(db.pool(), second_id, None, None)
        .await
        .expect("record transport failure");
    assert!(!result.duplicate);
    let failed_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steam.web_api_reservations WHERE reserved_at > clock_timestamp() - interval '24 hours'",
    )
    .fetch_one(db.pool())
    .await
    .expect("count failures");
    assert_eq!(failed_attempts, 2);
    let Reservation::Granted { id: third_id, .. } =
        reserve(&alternate_pool, "patchnotes", CallerClass::OptionalPatch)
            .await
            .expect("third reservation")
    else {
        panic!("third reservation must be granted");
    };
    let header = "x".repeat(129);
    let fallback = observe(&alternate_pool, third_id, Some(429), Some(&header))
        .await
        .expect("overlong header uses fallback");
    assert_eq!(
        fallback.cooldown_until,
        Some(fallback.response_at + Duration::seconds(30))
    );
    let repeated = observe(&alternate_pool, third_id, Some(429), Some(&header))
        .await
        .expect("overlong header is idempotent");
    assert!(repeated.duplicate);
    alternate_pool.close().await;
}

#[tokio::test]
#[ignore = "requires central_test_db.sh"]
async fn budget_time_is_read_after_waiting_for_the_row_lock() {
    let db = test_pool().await.expect("isolated migrated database");
    let application_name = format!("ledger_clock_probe_{}", std::process::id());
    let probe_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            db.pool()
                .connect_options()
                .as_ref()
                .clone()
                .application_name(&application_name),
        )
        .await
        .expect("independent probe connection");

    sqlx::query(
        "UPDATE steam.web_api_budget
            SET cooldown_until = clock_timestamp() + interval '1 second'
          WHERE id = true",
    )
    .execute(db.pool())
    .await
    .expect("set short cooldown before locking");

    let mut blocker = db.pool().begin().await.expect("budget blocker");
    let _: bool =
        sqlx::query_scalar("SELECT id FROM steam.web_api_budget WHERE id = true FOR UPDATE")
            .fetch_one(&mut *blocker)
            .await
            .expect("lock budget row");

    let reserve_pool = probe_pool.clone();
    let reserve_task =
        tokio::spawn(
            async move { reserve(&reserve_pool, "clock_probe", CallerClass::Standard).await },
        );
    wait_until_probe_is_blocked(db.pool(), &application_name).await;
    sqlx::query("SELECT pg_sleep(1.1)")
        .execute(&mut *blocker)
        .await
        .expect("hold lock past cooldown");
    let released_at: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *blocker)
        .await
        .expect("read lock release time");
    blocker.commit().await.expect("release budget row");
    let reservation = reserve_task
        .await
        .expect("reservation task")
        .expect("reserve");
    let Reservation::Granted {
        id: reserved_id,
        reserved_at,
    } = reservation
    else {
        if let Reservation::Denied { retry_at, .. } = reservation {
            assert!(
                retry_at >= released_at,
                "cooldown denial returned stale retry_at {retry_at} before lock release {released_at}"
            );
        }
        panic!("expired cooldown should grant a reservation");
    };
    assert!(
        reserved_at >= released_at,
        "reservation timestamp {reserved_at} predates lock release {released_at}"
    );
    let persisted_reserved_at: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT reserved_at FROM steam.web_api_reservations WHERE id = $1")
            .bind(reserved_id)
            .fetch_one(db.pool())
            .await
            .expect("persisted reservation slot");
    assert!(persisted_reserved_at >= released_at);

    let Reservation::Granted { id, .. } = reserve(db.pool(), "clock_probe", CallerClass::Standard)
        .await
        .expect("observation reservation")
    else {
        panic!("expired cooldown should allow reservation");
    };
    let mut blocker = db.pool().begin().await.expect("observation blocker");
    let _: bool =
        sqlx::query_scalar("SELECT id FROM steam.web_api_budget WHERE id = true FOR UPDATE")
            .fetch_one(&mut *blocker)
            .await
            .expect("lock budget row for observation");

    let observe_pool = probe_pool.clone();
    let observe_task =
        tokio::spawn(async move { observe(&observe_pool, id, Some(200), None).await });
    wait_until_probe_is_blocked(db.pool(), &application_name).await;
    sqlx::query("SELECT pg_sleep(1.1)")
        .execute(&mut *blocker)
        .await
        .expect("hold observation lock");
    let released_at: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *blocker)
        .await
        .expect("read lock release time");
    blocker.commit().await.expect("release observation row");
    let observation = observe_task
        .await
        .expect("observation task")
        .expect("observe");
    assert!(observation.response_at >= released_at);

    probe_pool.close().await;
}

async fn wait_until_probe_is_blocked(pool: &PgPool, application_name: &str) {
    for _ in 0..1_000 {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1
                  FROM pg_stat_activity
                 WHERE application_name = $1
                   AND wait_event_type = 'Lock'
            )",
        )
        .bind(application_name)
        .fetch_one(pool)
        .await
        .expect("inspect probe lock state");
        if blocked {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("probe did not wait for the budget row lock");
}
