use std::sync::Arc;

use dl_discord::dm_gate::DmOptoutHandler;
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use sqlx::PgPool;

const LEGACY_OPTOUT_IDS: &[&str] = &["voice_pair:never", "voice_mate:never"];
const LEGACY_OPTOUT_PREFIXES: &[&str] = &["solo_lfg:never:"];
const RETIRED_PREFIXES: &[&str] = &["voice_pair:", "solo_lfg:", "voice_mate:"];
const RETIRED_CUSTOM_IDS: &[&str] = &["voice_mate_comment"];
const RETIRED_HINT: &str = "Diese Anfrage gibt es nicht mehr.";

struct RetiredButtonHandler;

#[async_trait::async_trait]
impl InteractionHandler for RetiredButtonHandler {
    async fn handle(&self, _interaction: BridgeInteraction) -> BridgeReply {
        BridgeReply::ephemeral_text(RETIRED_HINT)
    }
}

pub fn register(router: &mut InteractionRouter, pool: PgPool) {
    let optout: Arc<dyn InteractionHandler> = DmOptoutHandler::new(pool);
    for custom_id in LEGACY_OPTOUT_IDS {
        router.on_custom_id(*custom_id, optout.clone());
    }
    for prefix in LEGACY_OPTOUT_PREFIXES {
        router.on_prefix(*prefix, optout.clone());
    }
    let retired: Arc<dyn InteractionHandler> = Arc::new(RetiredButtonHandler);
    for prefix in RETIRED_PREFIXES {
        router.on_prefix(*prefix, retired.clone());
    }
    for custom_id in RETIRED_CUSTOM_IDS {
        router.on_custom_id(*custom_id, retired.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use dl_discord::dm_gate::DM_OPTOUT_FAILED;
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    fn router() -> InteractionRouter {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(200))
            .connect_lazy("postgres://retired@127.0.0.1:1/unreachable")
            .expect("lazy pool");
        let mut router = InteractionRouter::new();
        register(&mut router, pool);
        router
    }

    async fn reply_text(router: &InteractionRouter, custom_id: &str) -> Option<String> {
        let handler = router.resolve_component(custom_id)?;
        let interaction = BridgeInteraction {
            custom_id: custom_id.to_string(),
            user_id: 1,
            ..BridgeInteraction::default()
        };
        handler.handle(interaction).await.content
    }

    #[tokio::test]
    async fn old_never_buttons_reach_the_global_dm_optout() {
        let router = router();
        for custom_id in [
            "voice_pair:never",
            "voice_mate:never",
            "solo_lfg:never:1289721245281292288:1376335502919335936",
        ] {
            assert_eq!(
                reply_text(&router, custom_id).await.as_deref(),
                Some(DM_OPTOUT_FAILED),
                "{custom_id}"
            );
        }
    }

    #[tokio::test]
    async fn other_old_buttons_get_the_retired_hint() {
        let router = router();
        for custom_id in [
            "voice_pair:yes:42",
            "solo_lfg:enter:1:2",
            "solo_lfg:later:1:2",
            "solo_lfg:join:1:2",
            "solo_lfg:modal",
            "voice_mate:rate:again:42:1:600",
            "voice_mate:comment:42",
            "voice_mate_comment",
        ] {
            assert_eq!(
                reply_text(&router, custom_id).await.as_deref(),
                Some(RETIRED_HINT),
                "{custom_id}"
            );
        }
    }
}
