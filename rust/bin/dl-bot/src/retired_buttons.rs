use std::sync::Arc;

use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};

const RETIRED_PREFIXES: &[&str] = &["voice_pair:"];
const RETIRED_HINT: &str = "Diese Anfrage gibt es nicht mehr.";

struct RetiredButtonHandler;

#[async_trait::async_trait]
impl InteractionHandler for RetiredButtonHandler {
    async fn handle(&self, _interaction: BridgeInteraction) -> BridgeReply {
        BridgeReply::ephemeral_text(RETIRED_HINT)
    }
}

pub fn register(router: &mut InteractionRouter) {
    let handler: Arc<dyn InteractionHandler> = Arc::new(RetiredButtonHandler);
    for prefix in RETIRED_PREFIXES {
        router.on_prefix(*prefix, handler.clone());
    }
}
