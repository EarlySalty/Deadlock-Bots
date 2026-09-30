mod test_database {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-support/peer_database.rs"
    ));
}

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Issuer {
    initial_count: usize,
    created: AtomicUsize,
    revoked: AtomicUsize,
    failure: bool,
    snapshot_available: bool,
    snapshot_error: bool,
}

#[async_trait::async_trait]
impl InviteIssuer for Issuer {
    async fn count(&self, _: u64) -> Result<usize, String> {
        Ok(self.initial_count + self.created.load(Ordering::SeqCst))
    }
    async fn create(&self, _: u64, _: &str) -> Result<InviteInfo, String> {
        if self.failure {
            return Err("capacity".into());
        }
        let number = self.created.fetch_add(1, Ordering::SeqCst);
        Ok(InviteInfo {
            invite_url: format!("https://discord.gg/Viewer{number}"),
            code: format!("Viewer{number}"),
            guild_id: 1,
        })
    }
    async fn revoke(&self, _: &str) {
        self.revoked.fetch_add(1, Ordering::SeqCst);
    }
    async fn ensure_snapshot_current(&self, _: u64) -> Result<bool, String> {
        if self.snapshot_error {
            Err("snapshot store failed".into())
        } else {
            Ok(self.snapshot_available)
        }
    }
}

async fn setup() -> (
    dl_central_db::TestDb,
    InviteDestination,
    TwitchInvitesConfig,
    Issuer,
) {
    let db = test_database::database().await;
    sqlx::query(
        "INSERT INTO bot.twitch_streamer_invites
         (streamer_login, twitch_user_id, guild_id, channel_id, invite_code, invite_url)
         VALUES ('streamer', '42', 1, 2, 'Channel', 'https://discord.gg/Channel')",
    )
    .execute(db.pool())
    .await
    .expect("channel fixture");
    (
        db,
        InviteDestination {
            streamer_login: "streamer".into(),
            streamer_twitch_user_id: "42".into(),
            guild_id: 1,
            channel_id: 2,
            invite_url: "https://discord.gg/Channel".into(),
        },
        TwitchInvitesConfig {
            personal_links_per_channel: 1,
            ..TwitchInvitesConfig::default()
        },
        Issuer {
            initial_count: 1,
            created: AtomicUsize::new(0),
            revoked: AtomicUsize::new(0),
            failure: false,
            snapshot_available: true,
            snapshot_error: false,
        },
    )
}

#[tokio::test]
async fn concurrent_requests_reuse_one_durable_invite_and_cap_others() {
    let (db, destination, mut config, issuer) = setup().await;
    let (first, second) = tokio::join!(
        resolve(db.pool(), &config, &destination, "43", &issuer),
        resolve(db.pool(), &config, &destination, "43", &issuer),
    );
    let first = first.expect("first");
    let second = second.expect("second");
    assert!(first.personal && second.personal);
    assert_eq!(first.invite_url, second.invite_url);
    assert_eq!(issuer.created.load(Ordering::SeqCst), 1);
    let capped = resolve(db.pool(), &config, &destination, "44", &issuer)
        .await
        .expect("cap fallback");
    assert!(!capped.personal);
    assert_eq!(capped.invite_url, destination.invite_url);
    config.personal_links_per_channel = 0;
    assert!(
        resolve(db.pool(), &config, &destination, "43", &issuer)
            .await
            .expect("existing at zero cap")
            .personal
    );
    let broadcaster = resolve(db.pool(), &config, &destination, "42", &issuer)
        .await
        .expect("broadcaster");
    assert!(!broadcaster.personal);
    assert_eq!(issuer.created.load(Ordering::SeqCst), 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bot.discord_invite_codes")
        .fetch_one(db.pool())
        .await
        .expect("tracking row");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn guild_reserve_and_discord_capacity_errors_keep_the_channel_link() {
    let (db, destination, config, mut issuer) = setup().await;
    issuer.initial_count = 950;
    let capped = resolve(db.pool(), &config, &destination, "43", &issuer)
        .await
        .expect("reserve fallback");
    assert!(!capped.personal);
    assert_eq!(issuer.created.load(Ordering::SeqCst), 0);
    issuer.initial_count = 1;
    issuer.failure = true;
    let failed = resolve(db.pool(), &config, &destination, "43", &issuer)
        .await
        .expect("Discord fallback");
    assert_eq!(failed.invite_url, destination.invite_url);
    assert!(!failed.personal);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bot.twitch_personal_invites")
        .fetch_one(db.pool())
        .await
        .expect("no attribution for fallback");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn revoked_invites_keep_their_attribution_and_return_channel_fallback() {
    let (db, destination, config, issuer) = setup().await;
    assert!(
        resolve(db.pool(), &config, &destination, "43", &issuer)
            .await
            .expect("create")
            .personal
    );
    sqlx::query("UPDATE bot.twitch_personal_invites SET revoked_at = now()")
        .execute(db.pool())
        .await
        .expect("revoke");
    assert!(
        !resolve(db.pool(), &config, &destination, "43", &issuer)
            .await
            .expect("revoked fallback")
            .personal
    );
    assert_eq!(issuer.created.load(Ordering::SeqCst), 1);
    let owner: String =
        sqlx::query_scalar("SELECT inviter_twitch_user_id FROM bot.twitch_personal_invites")
            .fetch_one(db.pool())
            .await
            .expect("owner retained");
    assert_eq!(owner, "43");
}

#[tokio::test]
async fn stale_snapshot_blocks_personal_link_until_reconciled() {
    let (db, destination, config, mut issuer) = setup().await;
    assert!(
        resolve(db.pool(), &config, &destination, "43", &issuer)
            .await
            .expect("create")
            .personal
    );

    issuer.snapshot_error = true;
    assert!(resolve(db.pool(), &config, &destination, "43", &issuer)
        .await
        .is_err());

    issuer.snapshot_error = false;
    issuer.snapshot_available = false;
    let stale = resolve(db.pool(), &config, &destination, "43", &issuer)
        .await
        .expect("stale fallback");
    assert!(!stale.personal);
    assert_eq!(stale.invite_url, destination.invite_url);
    assert_eq!(issuer.created.load(Ordering::SeqCst), 1);

    issuer.snapshot_available = true;
    assert!(
        resolve(db.pool(), &config, &destination, "43", &issuer)
            .await
            .expect("reconciled link")
            .personal
    );
}

#[tokio::test]
async fn persistence_failure_revokes_only_the_new_unpublished_invite() {
    let (db, destination, config, issuer) = setup().await;
    sqlx::query("ALTER TABLE bot.discord_invite_codes ADD CONSTRAINT reject_fixture CHECK (invite_code <> 'Viewer0')")
        .execute(db.pool()).await.expect("failure injection");
    assert!(resolve(db.pool(), &config, &destination, "43", &issuer)
        .await
        .is_err());
    assert_eq!(issuer.revoked.load(Ordering::SeqCst), 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bot.twitch_personal_invites")
        .fetch_one(db.pool())
        .await
        .expect("rolled back");
    assert_eq!(count, 0);
}
