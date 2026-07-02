//! Discord-Oberfläche `/datenschutz` (Port von `cogs/privacy_controls.py`).
//!
//! v1: Slash `/datenschutz` (Ein-Klick-Bestätigung über einen Danger-Button
//! statt des 2-Schritt-Edit-Flows) + `/datenschutz-optin`. Der Daten-Download
//! („Daten herunterladen") liefert den Export als JSON-Datei-Anhang
//! (`BridgeReply::attachments` → serenity-Multipart). Löschung (DSGVO-Löschrecht),
//! Export (Auskunftsrecht) und Opt-in funktionieren. Das in-memory-State-Clearing
//! (`_clear_runtime_state`) entfällt: der gesetzte Opt-out gated künftige
//! Schreibzugriffe ohnehin (Tracker prüfen `is_opted_out`).

use std::sync::Arc;

use dl_discord::{
    BridgeAttachment, BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler,
    InteractionRouter,
};
use serde_json::json;
use sqlx::PgPool;

use crate::db::u64_to_i64;
use crate::privacy::{delete_user_data, export_user_data, set_opt_in};

#[derive(Clone, Copy)]
enum Action {
    Datenschutz,
    OptIn,
    Confirm,
    Export,
}

struct PrivacyHandler {
    pool: PgPool,
    action: Action,
}

#[async_trait::async_trait]
impl InteractionHandler for PrivacyHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let now = chrono::Utc::now().timestamp();
        let uid = match u64_to_i64(interaction.user_id, "interaction.user_id") {
            Ok(uid) => uid,
            Err(_) => {
                return BridgeReply::ephemeral_text(
                    "Diese Discord-ID kann nicht verarbeitet werden. Bitte dem Team melden.",
                );
            }
        };
        match self.action {
            Action::Datenschutz => BridgeReply {
                content: Some(
                    "Dieser Vorgang entfernt deine gespeicherten Daten (z. B. Voice-Statistiken \
                     und Logs, Steam-Verknüpfungen, TempVoice-, Partner- und KI-Onboarding-Daten) \
                     und setzt ein Opt-out. Standardmäßig wird danach nichts Neues gespeichert.\n\n\
                     Klicke **Endgültig löschen**, um fortzufahren."
                        .to_string(),
                ),
                components: Some(json!([{
                    "type": 1,
                    "components": [
                        {
                            "type": 2,
                            "style": 4,
                            "label": "Endgültig löschen",
                            "custom_id": "privacy:confirm"
                        },
                        {
                            "type": 2,
                            "style": 2,
                            "label": "Daten herunterladen",
                            "custom_id": "privacy:export"
                        }
                    ]
                }])),
                ephemeral: true,
                ..Default::default()
            },
            Action::OptIn => {
                let _ = set_opt_in(&self.pool, uid, now).await;
                BridgeReply::ephemeral_text(
                    "Du hast wieder eingewilligt. Ab jetzt dürfen Features wieder Daten speichern.",
                )
            }
            Action::Confirm => {
                match delete_user_data(&self.pool, uid, "slash_datenschutz".to_string(), now).await {
                    Ok(s) => {
                        if interaction.guild_id > 0 {
                            if let Err(err) = dl_activity::journey::record_privacy_opt_out_aggregate(
                                &self.pool,
                                interaction.guild_id,
                                chrono::Utc::now(),
                            )
                            .await
                            {
                                tracing::warn!(%err, user_id = uid, "Journey opt_out-Aggregat konnte nicht geschrieben werden");
                            }
                        }
                        let voice = s.sum(&["voice_session_log.user_id", "voice_stats.user_id"]);
                        let steam = s.steam_ids.len();
                        BridgeReply::ephemeral_text(format!(
                            "✅ Deine gespeicherten Daten wurden gelöscht und ein Opt-out ist aktiv.\n\
                             - Voice: {voice} Einträge entfernt\n\
                             - Steam: {steam} Verknüpfung(en) gelöst\n\
                             Zukünftige Speicherung ist blockiert, bis du mit /datenschutz-optin wieder zustimmst."
                        ))
                    }
                    Err(_) => BridgeReply::ephemeral_text(
                        "⚠️ Konnte die Datenlöschung nicht abschließen. Bitte später erneut versuchen.",
                    ),
                }
            }
            Action::Export => match export_user_data(&self.pool, uid, now).await {
                Ok(value) => {
                    let bytes =
                        serde_json::to_vec_pretty(&value).unwrap_or_else(|_| b"{}".to_vec());
                    BridgeReply {
                        content: Some(
                            "📄 Hier sind deine gespeicherten Daten als JSON-Datei. \
                             Personenbezogene Fremd-IDs sind dabei geschwärzt."
                                .to_string(),
                        ),
                        ephemeral: true,
                        attachments: vec![BridgeAttachment {
                            filename: "deine-daten.json".to_string(),
                            data: bytes,
                        }],
                        ..Default::default()
                    }
                }
                Err(_) => BridgeReply::ephemeral_text(
                    "⚠️ Konnte den Datenexport nicht erstellen. Bitte später erneut versuchen.",
                ),
            },
        }
    }
}

fn command_spec(name: &str, description: &str) -> CommandSpec {
    CommandSpec {
        definition: json!({
            "name": name,
            "description": description,
            "type": 1,
        }),
    }
}

/// Registriert `/datenschutz`, `/datenschutz-optin` und den Bestätigungs-Button.
pub fn register(router: &mut InteractionRouter, pool: PgPool) {
    router.on_command(
        "datenschutz",
        command_spec(
            "datenschutz",
            "Löscht deine gespeicherten Daten und deaktiviert zukünftige Speicherung.",
        ),
        Arc::new(PrivacyHandler {
            pool: pool.clone(),
            action: Action::Datenschutz,
        }),
    );
    router.on_command(
        "datenschutz-optin",
        command_spec(
            "datenschutz-optin",
            "Reaktiviere Speicherung nach einem Opt-out.",
        ),
        Arc::new(PrivacyHandler {
            pool: pool.clone(),
            action: Action::OptIn,
        }),
    );
    router.on_custom_id(
        "privacy:confirm",
        Arc::new(PrivacyHandler {
            pool: pool.clone(),
            action: Action::Confirm,
        }),
    );
    router.on_custom_id(
        "privacy:export",
        Arc::new(PrivacyHandler {
            pool,
            action: Action::Export,
        }),
    );
}
