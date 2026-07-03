use std::{path::PathBuf, sync::Arc};

use dl_central_db::kv;
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;

pub const LFG_GUILD_ID: u64 = crate::router::ROUTER_GUILD_ID;
pub const LFG_PANEL_KV_NS: &str = "lfg_panel";
pub const LFG_PANEL_MESSAGE_KEY: &str = "components_v2_message_id";
pub const LFG_PAYLOAD_FORMAT_KEY: &str = "payload_format";
pub const LFG_PAYLOAD_FORMAT: &str = "components_v2";
pub const LFG_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const LFG_ACCENT_GOLD: u64 = crate::router::ROUTER_ACCENT_GOLD;
pub const LFG_BANNER_DIR: &str = "assets/welcome-banners";
pub const LFG_PANEL_BANNER_FILENAME: &str = "router-hero.png";
pub const LFG_CREATE_START_CUSTOM_ID: &str = "lfg:create:start";
pub const LFG_PLACEHOLDER_TEXT: &str = "Platzhalter";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfgPanelApplyOutput {
    pub guild_id: u64,
    pub channel_id: Option<u64>,
    pub dry_run: bool,
    pub payload_format: String,
    pub stored_payload_format: Option<String>,
    pub stored_message_id: Option<u64>,
    pub action: String,
    pub message_id: Option<u64>,
    pub blocked_reason: Option<String>,
    pub warnings: Vec<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfgPanelAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfgPanelMessage {
    pub message_id: u64,
    pub has_embeds: bool,
    pub has_components: bool,
    pub custom_ids: Vec<String>,
}

#[async_trait::async_trait]
pub trait LfgPanelPort: Send + Sync {
    async fn post_rich(
        &self,
        channel_id: u64,
        body: Map<String, Value>,
        attachments: &[LfgPanelAttachment],
    ) -> Result<u64, String>;

    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
        attachments: &[LfgPanelAttachment],
    ) -> Result<(), String>;

    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<LfgPanelMessage>, String>;
}

pub struct LfgPanelInterface {
    pool: PgPool,
    port: Arc<dyn LfgPanelPort>,
    channel_id: Option<u64>,
    missing_channel_reason: Option<String>,
}

impl LfgPanelInterface {
    pub fn new(pool: PgPool, port: Arc<dyn LfgPanelPort>, channel_id: Option<u64>) -> Arc<Self> {
        Self::new_with_channel_config(
            pool,
            port,
            channel_id,
            channel_id
                .is_none()
                .then(|| "DL_LFG_PANEL_CHANNEL_ID fehlt".to_string()),
        )
    }

    pub fn new_with_channel_config(
        pool: PgPool,
        port: Arc<dyn LfgPanelPort>,
        channel_id: Option<u64>,
        missing_channel_reason: Option<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            channel_id,
            missing_channel_reason,
        })
    }

    pub async fn ensure_panel(&self) {
        if let Err(err) = self.apply_panel(true).await {
            tracing::warn!(%err, "LfgPanelInterface: Panel konnte nicht angewendet werden");
        }
    }

    pub fn target_channel_id(&self) -> Option<u64> {
        self.channel_id
    }

    pub async fn apply_panel(&self, confirm: bool) -> Result<LfgPanelApplyOutput, String> {
        let attachments = lfg_panel_attachments();
        let body = lfg_panel_body_for_attachments(&attachments);
        let stored_message_id = self.panel_message_id().await;
        let stored_payload_format = self.panel_payload_format().await;
        let recent = match self.channel_id {
            Some(channel_id) => match self.port.recent_bot_messages(channel_id, 15).await {
                Ok(messages) => Some(messages),
                Err(err) => {
                    tracing::warn!(%err, "LfgPanelInterface: History-Scan fehlgeschlagen");
                    None
                }
            },
            None => None,
        };
        let history_checked = self.channel_id.is_none() || recent.is_some();
        let history_message_id = recent.as_deref().and_then(find_existing_lfg_v2_panel);
        let planned_message_id = stored_message_id.or(history_message_id);
        let action = match (self.channel_id, planned_message_id) {
            (None, _) => "blocked_missing_channel",
            (Some(_), Some(_)) => "planned_edit",
            (Some(_), None) => "planned_post",
        };
        let mut output = LfgPanelApplyOutput {
            guild_id: LFG_GUILD_ID,
            channel_id: self.channel_id,
            dry_run: !confirm,
            payload_format: LFG_PAYLOAD_FORMAT.to_string(),
            stored_payload_format,
            stored_message_id,
            action: action.to_string(),
            message_id: planned_message_id,
            blocked_reason: self
                .channel_id
                .is_none()
                .then(|| self.missing_channel_reason.clone())
                .flatten(),
            warnings: Vec::new(),
            payload: Value::Object(body.clone()),
        };
        if self.channel_id.is_none() {
            let reason = self
                .missing_channel_reason
                .as_deref()
                .unwrap_or("DL_LFG_PANEL_CHANNEL_ID fehlt");
            output.warnings.push(format!(
                "LFG-Panel: Zielkanal fehlt ({reason}); DL_LFG_PANEL_CHANNEL_ID muss auf das Forum-/Panel-Ziel zeigen."
            ));
            if confirm {
                return Err(format!("LfgPanelInterface: {reason}"));
            }
            return Ok(output);
        }
        if !confirm {
            if self.channel_id.is_some() && !history_checked {
                output.warnings.push(
                    "LFG-Panel: History-Scan fehlgeschlagen; Dry-Run ohne Adoption.".to_string(),
                );
            }
            return Ok(output);
        }

        validate_lfg_panel_attachments(&attachments)?;
        let channel_id = self.channel_id.expect("checked channel_id");
        let mut ignored_history_message_id = None;
        if let Some(message_id) = stored_message_id {
            match self
                .port
                .edit_rich(channel_id, message_id, body.clone(), &attachments)
                .await
            {
                Ok(()) => {
                    self.store_panel_metadata(message_id).await;
                    output.dry_run = false;
                    output.action = "edited".to_string();
                    output.message_id = Some(message_id);
                    return Ok(output);
                }
                Err(err) if is_not_found_error(&err) => {
                    self.delete_panel_message_id().await;
                    ignored_history_message_id = Some(message_id);
                    output.warnings.push(format!(
                        "LFG-Panel: gespeicherte Message-ID {message_id} ist stale (404); KV wurde geloescht."
                    ));
                }
                Err(err) => return Err(err),
            }
        }

        if !history_checked {
            return Err(
                "LfgPanelInterface: ohne erfolgreichen History-Scan wird kein neues Panel gepostet"
                    .to_string(),
            );
        }

        if let Some(message_id) =
            history_message_id.filter(|message_id| Some(*message_id) != ignored_history_message_id)
        {
            self.port
                .edit_rich(channel_id, message_id, body.clone(), &attachments)
                .await
                .map_err(|err| {
                    format!(
                        "LfgPanelInterface: adoptiertes Panel {message_id} konnte nicht editiert werden: {err}"
                    )
                })?;
            self.store_panel_metadata(message_id).await;
            output.dry_run = false;
            output.action = "adopted_edit".to_string();
            output.message_id = Some(message_id);
            return Ok(output);
        }

        let message_id = self.port.post_rich(channel_id, body, &attachments).await?;
        self.store_panel_metadata(message_id).await;
        output.dry_run = false;
        output.action = "posted".to_string();
        output.message_id = Some(message_id);
        Ok(output)
    }

    async fn panel_message_id(&self) -> Option<u64> {
        kv::get(&self.pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
    }

    async fn panel_payload_format(&self) -> Option<String> {
        kv::get(&self.pool, LFG_PANEL_KV_NS, LFG_PAYLOAD_FORMAT_KEY)
            .await
            .ok()
            .flatten()
    }

    async fn store_panel_metadata(&self, message_id: u64) {
        if let Err(err) = kv::set(
            &self.pool,
            LFG_PANEL_KV_NS,
            LFG_PANEL_MESSAGE_KEY,
            &message_id.to_string(),
        )
        .await
        {
            tracing::warn!(%err, "LfgPanelInterface: Message-ID konnte nicht gespeichert werden");
        }
        if let Err(err) = kv::set(
            &self.pool,
            LFG_PANEL_KV_NS,
            LFG_PAYLOAD_FORMAT_KEY,
            LFG_PAYLOAD_FORMAT,
        )
        .await
        {
            tracing::warn!(%err, "LfgPanelInterface: Payload-Format konnte nicht gespeichert werden");
        }
    }

    async fn delete_panel_message_id(&self) {
        if let Err(err) = kv::delete(&self.pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY).await {
            tracing::warn!(%err, "LfgPanelInterface: stale Message-ID konnte nicht geloescht werden");
        }
    }
}

pub fn lfg_repo_root() -> PathBuf {
    crate::router::router_repo_root()
}

pub fn lfg_panel_attachments() -> Vec<LfgPanelAttachment> {
    vec![LfgPanelAttachment {
        id: 0,
        filename: LFG_PANEL_BANNER_FILENAME.to_string(),
        relative_path: format!("{LFG_BANNER_DIR}/{LFG_PANEL_BANNER_FILENAME}"),
    }]
}

pub fn lfg_panel_body() -> Map<String, Value> {
    let attachments = lfg_panel_attachments();
    lfg_panel_body_for_attachments(&attachments)
}

fn lfg_panel_body_for_attachments(attachments: &[LfgPanelAttachment]) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".to_string(), json!(LFG_COMPONENTS_V2_FLAG));
    body.insert(
        "allowed_mentions".to_string(),
        json!({ "parse": Vec::<String>::new() }),
    );
    let mut components = Vec::new();
    if !attachments.is_empty() {
        components.push(lfg_media_gallery(&attachments[0].filename));
    }
    components.push(lfg_text_display(LFG_PLACEHOLDER_TEXT.to_string()));
    components.push(lfg_action_row(vec![lfg_button(
        LFG_PLACEHOLDER_TEXT,
        1,
        LFG_CREATE_START_CUSTOM_ID,
    )]));
    body.insert(
        "components".to_string(),
        json!([{
            "type": 17,
            "accent_color": LFG_ACCENT_GOLD,
            "components": components,
        }]),
    );
    body.insert("attachments".to_string(), json!(attachments));
    body
}

fn lfg_media_gallery(filename: &str) -> Value {
    json!({
        "type": 12,
        "items": [{
            "media": { "url": format!("attachment://{filename}") },
        }],
    })
}

fn lfg_text_display(content: String) -> Value {
    json!({
        "type": 10,
        "content": content,
    })
}

fn lfg_action_row(components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "components": components,
    })
}

fn lfg_button(label: &str, style: u8, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
    })
}

fn validate_lfg_panel_attachments(attachments: &[LfgPanelAttachment]) -> Result<(), String> {
    let repo_root = lfg_repo_root();
    for attachment in attachments {
        let path = repo_root.join(&attachment.relative_path);
        if !path.is_file() {
            return Err(format!(
                "LFG-Banner `{}` fehlt; LFG-Panel wird nicht gepostet/editiert",
                path.display()
            ));
        }
    }
    Ok(())
}

fn find_existing_lfg_v2_panel(messages: &[LfgPanelMessage]) -> Option<u64> {
    messages
        .iter()
        .find(|message| {
            !message.has_embeds
                && message.has_components
                && message
                    .custom_ids
                    .iter()
                    .any(|custom_id| custom_id.starts_with("lfg:create:"))
        })
        .map(|message| message.message_id)
}

fn is_not_found_error(err: &str) -> bool {
    err.contains("HTTP 404") || err.contains("404")
}

struct LfgCreatePanelHandler;

#[async_trait::async_trait]
impl InteractionHandler for LfgCreatePanelHandler {
    async fn handle(&self, _interaction: BridgeInteraction) -> BridgeReply {
        BridgeReply::ephemeral_text(LFG_PLACEHOLDER_TEXT)
    }
}

pub fn register(router: &mut InteractionRouter) {
    router.on_prefix("lfg:create:", Arc::new(LfgCreatePanelHandler));
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::{BridgeInteraction, InteractionRouter};
    use serde_json::{json, Map, Value};
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn lfg_panel_body_ist_components_v2_mit_start_button_und_banner() {
        let attachments = lfg_panel_attachments();
        let body = lfg_panel_body_for_attachments(&attachments);

        assert_eq!(body.get("flags"), Some(&json!(LFG_COMPONENTS_V2_FLAG)));
        assert_eq!(body.get("allowed_mentions"), Some(&json!({"parse": []})));
        assert_eq!(
            body.get("attachments"),
            Some(&json!([{"id": 0, "filename": LFG_PANEL_BANNER_FILENAME}]))
        );
        let components = body["components"][0]["components"]
            .as_array()
            .expect("container components");
        assert!(components.iter().any(|component| {
            component["type"] == 12
                && component["items"][0]["media"]["url"]
                    == format!("attachment://{LFG_PANEL_BANNER_FILENAME}")
        }));
        let custom_ids: Vec<&str> = components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert_eq!(custom_ids, vec![LFG_CREATE_START_CUSTOM_ID]);
    }

    #[tokio::test]
    async fn lfg_panel_interface_postet_und_editiert_idempotent_ueber_kv() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let first = interface.apply_panel(true).await.expect("first apply");
        assert_eq!(first.action, "posted");
        assert_eq!(first.channel_id, Some(777));
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9101")
        );

        let second = interface.apply_panel(true).await.expect("second apply");
        assert_eq!(second.action, "edited");
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(port.edits.lock().expect("edits").len(), 1);
    }

    #[tokio::test]
    async fn lfg_panel_apply_postet_neu_wenn_kv_message_geloescht_ist() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        dl_central_db::kv::set(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY, "7777")
            .await
            .expect("kv set");
        let port = Arc::new(MockLfgPanelPort::default());
        port.not_found_edits.lock().expect("not found").push(7777);
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let output = interface.apply_panel(true).await.expect("apply");

        assert_eq!(output.action, "posted");
        assert_eq!(output.message_id, Some(9101));
        assert_eq!(port.posts.lock().expect("posts").len(), 1);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("stale (404)")));
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("9101")
        );
    }

    #[tokio::test]
    async fn lfg_panel_apply_adoptiert_nur_lfg_create_v2_panel_aus_history() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let pool = db.pool().clone();
        let port = Arc::new(MockLfgPanelPort::default());
        *port.recent.lock().expect("recent") = vec![
            LfgPanelMessage {
                message_id: 7001,
                has_embeds: false,
                has_components: true,
                custom_ids: vec!["router_spawn_casual".to_string()],
            },
            LfgPanelMessage {
                message_id: 7002,
                has_embeds: true,
                has_components: true,
                custom_ids: vec![LFG_CREATE_START_CUSTOM_ID.to_string()],
            },
            LfgPanelMessage {
                message_id: 7003,
                has_embeds: false,
                has_components: true,
                custom_ids: vec![LFG_CREATE_START_CUSTOM_ID.to_string()],
            },
        ];
        let interface = LfgPanelInterface::new(pool.clone(), port.clone(), Some(777));

        let output = interface.apply_panel(true).await.expect("apply");

        assert_eq!(output.action, "adopted_edit");
        assert_eq!(output.message_id, Some(7003));
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        let edited_message_id = {
            let edits = port.edits.lock().expect("edits");
            assert_eq!(edits.len(), 1);
            edits[0].1
        };
        assert_eq!(edited_message_id, 7003);
        assert_eq!(
            dl_central_db::kv::get(&pool, LFG_PANEL_KV_NS, LFG_PANEL_MESSAGE_KEY)
                .await
                .expect("kv")
                .as_deref(),
            Some("7003")
        );
    }

    #[tokio::test]
    async fn lfg_panel_dry_run_braucht_keine_channel_config() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(MockLfgPanelPort::default());
        let interface = LfgPanelInterface::new(db.pool().clone(), port.clone(), None);

        let output = interface.apply_panel(false).await.expect("dry run");

        assert!(output.dry_run);
        assert_eq!(output.action, "blocked_missing_channel");
        assert!(output.channel_id.is_none());
        assert_eq!(
            output.blocked_reason.as_deref(),
            Some("DL_LFG_PANEL_CHANNEL_ID fehlt")
        );
        assert_eq!(port.posts.lock().expect("posts").len(), 0);
        assert_eq!(port.edits.lock().expect("edits").len(), 0);
    }

    #[tokio::test]
    async fn lfg_create_start_button_antwortet_w1_mit_platzhalter() {
        let mut router = InteractionRouter::new();
        register(&mut router);

        let handler = router
            .resolve_component(LFG_CREATE_START_CUSTOM_ID)
            .expect("lfg handler");
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: LFG_CREATE_START_CUSTOM_ID.to_string(),
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(LFG_PLACEHOLDER_TEXT));
    }

    type MockPost = (u64, Map<String, Value>, Vec<LfgPanelAttachment>);
    type MockEdit = (u64, u64, Map<String, Value>, Vec<LfgPanelAttachment>);

    #[derive(Default)]
    struct MockLfgPanelPort {
        posts: StdMutex<Vec<MockPost>>,
        edits: StdMutex<Vec<MockEdit>>,
        not_found_edits: StdMutex<Vec<u64>>,
        recent: StdMutex<Vec<LfgPanelMessage>>,
    }

    #[async_trait::async_trait]
    impl LfgPanelPort for MockLfgPanelPort {
        async fn post_rich(
            &self,
            channel_id: u64,
            body: Map<String, Value>,
            attachments: &[LfgPanelAttachment],
        ) -> Result<u64, String> {
            self.posts
                .lock()
                .expect("posts")
                .push((channel_id, body, attachments.to_vec()));
            Ok(9100 + self.posts.lock().expect("posts").len() as u64)
        }

        async fn edit_rich(
            &self,
            channel_id: u64,
            message_id: u64,
            body: Map<String, Value>,
            attachments: &[LfgPanelAttachment],
        ) -> Result<(), String> {
            if self
                .not_found_edits
                .lock()
                .expect("not found")
                .contains(&message_id)
            {
                return Err("Discord PATCH fehlgeschlagen: HTTP 404: Unknown Message".to_string());
            }
            self.edits.lock().expect("edits").push((
                channel_id,
                message_id,
                body,
                attachments.to_vec(),
            ));
            Ok(())
        }

        async fn recent_bot_messages(
            &self,
            _channel_id: u64,
            _limit: u8,
        ) -> Result<Vec<LfgPanelMessage>, String> {
            Ok(self.recent.lock().expect("recent").clone())
        }
    }
}
