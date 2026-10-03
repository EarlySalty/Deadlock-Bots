use dl_community::{
    feedback_hub::{FeedbackHub, FeedbackPort},
    privacy::set_opt_in,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestConfig {
    socket: PathBuf,
    port: u16,
    user: String,
    database: String,
}
struct Transport {
    calls: AtomicUsize,
    fail: bool,
}
#[async_trait::async_trait]
impl FeedbackPort for Transport {
    async fn post_rich(&self, _: u64, _: Map<String, Value>) -> Result<u64, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err("Synthetischer Versandfehler".into())
        } else {
            Ok(501)
        }
    }
    async fn edit_rich(&self, _: u64, _: u64, _: Map<String, Value>) -> Result<(), String> {
        unreachable!()
    }
}

#[tokio::test]
#[ignore = "benötigt eigene isolierte Postgresinstanz und tests/guide-pg.local.json"]
async fn wiedereinwilligung_ablauf_und_zustellfehler_im_echten_pg_pfad() {
    let file = std::fs::File::open("tests/guide-pg.local.json").unwrap();
    let config: TestConfig = serde_json::from_reader(file).unwrap();
    assert!(config.socket.is_absolute() && config.socket.ends_with(".core-test-pg"));
    assert!(config.database.starts_with("guide_test_") && config.user == "brain_core_test");
    let options = PgConnectOptions::new_without_pgpass()
        .host(&config.socket.to_string_lossy())
        .port(config.port)
        .username(&config.user)
        .database(&config.database)
        .password("");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE SCHEMA core; CREATE SCHEMA brain; CREATE SCHEMA bot;
        CREATE TABLE core.user_privacy(user_id BIGINT PRIMARY KEY,opted_out BOOLEAN NOT NULL,deleted_at TIMESTAMPTZ,reason TEXT,updated_at TIMESTAMPTZ NOT NULL);
        CREATE TABLE brain.guide_subjects(guild_id TEXT,user_id TEXT,epoch BIGINT,memory_enabled BOOLEAN,contact_enabled BOOLEAN,globally_opted_out BOOLEAN,deleted BOOLEAN,profile_json JSONB,history_json JSONB,min_event_id BIGINT DEFAULT 0,updated_at TIMESTAMPTZ,PRIMARY KEY(guild_id,user_id));
        CREATE TABLE brain.guide_legacy_imports(guild_id TEXT,user_id TEXT,PRIMARY KEY(guild_id,user_id));
        CREATE TABLE brain.guide_turn_claims(user_id TEXT);
        CREATE TABLE brain.guide_conversations(user_id TEXT);
        CREATE TABLE brain.guide_feedback_drafts(user_id TEXT);
        CREATE TABLE brain.guide_feedback_outbox(guild_id TEXT,user_id TEXT,delivery_id TEXT,destination_channel_id TEXT,text TEXT,state TEXT,expires_at TIMESTAMPTZ);
        CREATE TABLE bot.serverguide_feedback_deliveries(delivery_id TEXT PRIMARY KEY,user_id BIGINT,sent_message_id BIGINT);
        INSERT INTO core.user_privacy VALUES(5,true,now(),'delete',now());
        INSERT INTO brain.guide_subjects VALUES('100','5',7,false,false,true,true,'{}','[]',0,now());
        INSERT INTO brain.guide_turn_claims VALUES('5');
        INSERT INTO brain.guide_conversations VALUES('5');
        INSERT INTO brain.guide_feedback_drafts VALUES('5');
        INSERT INTO brain.guide_feedback_outbox VALUES('100','5','old','500','Altes synthetisches Anliegen','pending',now()+interval '10 minutes');
        INSERT INTO bot.serverguide_feedback_deliveries VALUES('old',5,NULL)")
        .execute(&pool).await.unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    set_opt_in(&pool, 5, now).await.unwrap();
    let state:(i64,bool,bool,bool,bool,Value,Value,i64)=sqlx::query_as("SELECT epoch,memory_enabled,contact_enabled,globally_opted_out,deleted,profile_json,history_json,min_event_id FROM brain.guide_subjects WHERE user_id='5'").fetch_one(&pool).await.unwrap();
    assert_eq!(state.0, 8);
    assert!(!state.1 && !state.2 && !state.3 && !state.4);
    assert_eq!(state.5, serde_json::json!({}));
    assert_eq!(state.6, serde_json::json!([]));
    assert!(state.7 > 0);
    let empty: bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM brain.guide_turn_claims) AND NOT EXISTS(SELECT 1 FROM brain.guide_conversations) AND NOT EXISTS(SELECT 1 FROM brain.guide_feedback_outbox) AND NOT EXISTS(SELECT 1 FROM brain.guide_feedback_drafts)").fetch_one(&pool).await.unwrap();
    assert!(empty);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM brain.guide_legacy_imports")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM bot.serverguide_feedback_deliveries")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    set_opt_in(&pool, 5, now).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT epoch FROM brain.guide_subjects")
            .fetch_one(&pool)
            .await
            .unwrap(),
        8
    );
    sqlx::raw_sql("INSERT INTO brain.guide_feedback_outbox VALUES('100','5','expired','500','Synthetisches Anliegen','pending',now()-interval '1 second'),('100','5','valid','500','Synthetisches Anliegen','pending',now()+interval '10 minutes'),('100','5','failure','500','Synthetisches Anliegen','pending',now()+interval '10 minutes'),('100','5','uncertain','500','Synthetisches Anliegen','pending',now()+interval '10 minutes')").execute(&pool).await.unwrap();
    let transport = Arc::new(Transport {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let hub = FeedbackHub {
        port: transport.clone(),
        pool: pool.clone(),
    };
    assert!(hub
        .deliver_guide_feedback("expired", "100", "5", 500, "Synthetisches Anliegen")
        .await
        .is_err());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        hub.deliver_guide_feedback("valid", "100", "5", 500, "Synthetisches Anliegen")
            .await
            .unwrap(),
        501
    );
    assert_eq!(
        hub.deliver_guide_feedback("valid", "100", "5", 500, "Synthetisches Anliegen")
            .await
            .unwrap(),
        501
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    let failing = Arc::new(Transport {
        calls: AtomicUsize::new(0),
        fail: true,
    });
    let failed_hub = FeedbackHub {
        port: failing.clone(),
        pool: pool.clone(),
    };
    assert!(failed_hub
        .deliver_guide_feedback("failure", "100", "5", 500, "Synthetisches Anliegen")
        .await
        .is_err());
    assert!(failed_hub
        .deliver_guide_feedback("failure", "100", "5", 500, "Synthetisches Anliegen")
        .await
        .is_err());
    assert_eq!(failing.calls.load(Ordering::SeqCst), 1);
    sqlx::raw_sql("CREATE FUNCTION bot.reject_ledger_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'Synthetischer Ledgerfehler'; END $$; CREATE TRIGGER reject_update BEFORE UPDATE ON bot.serverguide_feedback_deliveries FOR EACH ROW EXECUTE FUNCTION bot.reject_ledger_update()") .execute(&pool).await.unwrap();
    assert!(hub
        .deliver_guide_feedback("uncertain", "100", "5", 500, "Synthetisches Anliegen")
        .await
        .is_err());
    assert!(hub
        .deliver_guide_feedback("uncertain", "100", "5", 500, "Synthetisches Anliegen")
        .await
        .is_err());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    pool.close().await;
}
