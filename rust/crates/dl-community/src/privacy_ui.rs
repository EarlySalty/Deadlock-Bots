//! Discord-Oberfläche `/datenschutz` (Port von `cogs/privacy_controls.py`).
//!
//! v1: Slash `/datenschutz` (Ein-Klick-Bestätigung über einen Danger-Button
//! statt des 2-Schritt-Edit-Flows) + `/datenschutz-optin`. Der Daten-Download
//! („Daten herunterladen") braucht Datei-Anhänge in der Interaction-Antwort —
//! das unterstützt `BridgeReply` noch nicht und folgt separat. Die **Löschung**
//! (DSGVO-Löschrecht) und das Opt-in funktionieren. Das in-memory-State-Clearing
//! (`_clear_runtime_state`) entfällt: der gesetzte Opt-out gated künftige
//! Schreibzugriffe ohnehin (Tracker prüfen `is_opted_out`).

use std::sync::Arc;

use dl_db::Db;
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::json;

use crate::privacy::{delete_user_data, set_opt_in};

#[derive(Clone, Copy)]
enum Action {
    Datenschutz,
    OptIn,
    Confirm,
}

struct PrivacyHandler {
    db: Db,
    action: Action,
}

#[async_trait::async_trait]
impl InteractionHandler for PrivacyHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let now = chrono::Utc::now().timestamp();
        let uid = interaction.user_id as i64;
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
                    "components": [{
                        "type": 2,
                        "style": 4,
                        "label": "Endgültig löschen",
                        "custom_id": "privacy:confirm"
                    }]
                }])),
                ephemeral: true,
                ..Default::default()
            },
            Action::OptIn => {
                let _ = set_opt_in(&self.db, uid, now).await;
                BridgeReply::ephemeral_text(
                    "Du hast wieder eingewilligt. Ab jetzt dürfen Features wieder Daten speichern.",
                )
            }
            Action::Confirm => {
                match delete_user_data(&self.db, uid, "slash_datenschutz".to_string(), now).await {
                    Ok(s) => {
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
pub fn register(router: &mut InteractionRouter, db: Db) {
    router.on_command(
        "datenschutz",
        command_spec(
            "datenschutz",
            "Löscht deine gespeicherten Daten und deaktiviert zukünftige Speicherung.",
        ),
        Arc::new(PrivacyHandler {
            db: db.clone(),
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
            db: db.clone(),
            action: Action::OptIn,
        }),
    );
    router.on_custom_id(
        "privacy:confirm",
        Arc::new(PrivacyHandler {
            db,
            action: Action::Confirm,
        }),
    );
}
