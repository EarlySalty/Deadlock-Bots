//! Discord-Oberfläche `/datenschutz` (Port von `cogs/privacy_controls.py`).
//!
//! v1: Slash `/datenschutz` (Ein-Klick-Bestätigung über einen Danger-Button
//! statt des 2-Schritt-Edit-Flows) + `/datenschutz-optin`. Der Daten-Download
//! („Daten herunterladen") liefert den Export als JSON-Datei-Anhang
//! (`BridgeReply::attachments` → serenity-Multipart). Löschung (DSGVO-Löschrecht),
//! Export (Auskunftsrecht) und Opt-in funktionieren. Der globale Opt-out-Vermerk
//! wird von den angebundenen Schreibpfaden fail-closed geprüft.

use std::sync::Arc;

use dl_discord::{
    BridgeAttachment, BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler,
    InteractionRouter,
};
use serde_json::json;
use sqlx::PgPool;

use crate::db::u64_to_i64;
use crate::privacy::{delete_user_data, export_user_data, set_opt_in};

const PRIVACY_INTRO_TEXT: &str = "Mit dem Button unten löschst du die personenbezogenen Daten, die über diesen Ablauf erfasst wurden. Zusätzlich setzen wir einen globalen Vermerk, dass du keine Speicherung mehr willst. Systeme, die diesen Vermerk auswerten, speichern dann nichts Neues mehr zu dir. Wenn du wissen willst, was das konkret abdeckt, melde dich beim Datenschutz-Support, dort antwortet dir ein Mensch.";
const OPTIN_SUCCESS_TEXT: &str = "Dein Vermerk ist aufgehoben, Systeme, die ihn beachten, können wieder Daten zu dir speichern. Mit /datenschutz setzt du ihn jederzeit erneut.";
const OPTIN_ERROR_TEXT: &str = "Beim `/datenschutz-optin` gab es einen Datenbankfehler, deshalb können wir gerade nicht zuverlässig sagen, ob deine Änderung gespeichert wurde oder nicht. Führ den Befehl einfach nochmal aus, ein zweiter Durchlauf ändert nichts am Ergebnis, und wenn der Fehler bleibt, melde dich beim Support.";
const DELETE_SUCCESS_TAIL: &str = "Dein globaler Vermerk ist aktiv, Systeme, die ihn beachten, speichern nichts Neues mehr zu dir. Mit /datenschutz-optin hebst du ihn wieder auf, für den genauen Umfang wende dich an den Datenschutz-Support, eine absolute Garantie über alle Systeme hinweg gibt es nicht.";

pub type RuntimePrivacyCleanup = Arc<dyn Fn(u64) + Send + Sync>;

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
    clear_runtime_state: RuntimePrivacyCleanup,
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
                content: Some(PRIVACY_INTRO_TEXT.to_string()),
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
            Action::OptIn => match set_opt_in(&self.pool, uid, now).await {
                Ok(()) => BridgeReply::ephemeral_text(OPTIN_SUCCESS_TEXT),
                Err(err) => {
                    tracing::warn!(%err, user_id = uid, "Privacy-Opt-in konnte nicht gespeichert werden");
                    BridgeReply::ephemeral_text(OPTIN_ERROR_TEXT)
                }
            },
            Action::Confirm => {
                match delete_user_data(&self.pool, uid, "slash_datenschutz".to_string(), now).await {
                    Ok(s) => {
                        (self.clear_runtime_state)(interaction.user_id);
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
                            "✅ Die von diesem Ablauf erfassten gespeicherten Daten wurden gelöscht und ein Opt-out ist aktiv.\n\
                             - Voice: {voice} Einträge entfernt\n\
                             - Steam: {steam} Verknüpfung(en) gelöst\n\
                             {DELETE_SUCCESS_TAIL}"
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
                            "📄 Hier sind die von diesem Ablauf erfassten Daten als JSON-Datei. \
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
pub fn register(
    router: &mut InteractionRouter,
    pool: PgPool,
    clear_runtime_state: RuntimePrivacyCleanup,
) {
    router.on_command(
        "datenschutz",
        command_spec(
            "datenschutz",
            "Exportiere oder lösche deine erfassten Daten; nur die Löschung setzt zusätzlich den Opt-out.",
        ),
        Arc::new(PrivacyHandler {
            pool: pool.clone(),
            action: Action::Datenschutz,
            clear_runtime_state: clear_runtime_state.clone(),
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
            clear_runtime_state: clear_runtime_state.clone(),
        }),
    );
    router.on_custom_id(
        "privacy:confirm",
        Arc::new(PrivacyHandler {
            pool: pool.clone(),
            action: Action::Confirm,
            clear_runtime_state: clear_runtime_state.clone(),
        }),
    );
    router.on_custom_id(
        "privacy:export",
        Arc::new(PrivacyHandler {
            pool,
            action: Action::Export,
            clear_runtime_state,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    #[cfg(feature = "testing")]
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn unavailable_pool() -> PgPool {
        let options =
            sqlx::postgres::PgConnectOptions::from_str("postgres://postgres@127.0.0.1:1/test")
                .expect("connect options");
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_millis(10))
            .connect_lazy_with(options)
    }

    #[tokio::test]
    async fn optin_db_fehler_bestaetigt_keinen_erfolg() {
        let handler = PrivacyHandler {
            pool: unavailable_pool(),
            action: Action::OptIn,
            clear_runtime_state: Arc::new(|_| {}),
        };

        let reply = handler
            .handle(BridgeInteraction {
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;
        let content = reply.content.expect("sichtbare Antwort");

        assert!(content.contains("nicht zuverlässig"));
        assert!(!content.contains("wieder eingewilligt"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn optin_bestaetigt_erst_nach_erfolgreichem_schreiben() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test pool");
        sqlx::query(
            "INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
             VALUES(42, TRUE, now())",
        )
        .execute(db.pool())
        .await
        .expect("privacy tombstone");
        let handler = PrivacyHandler {
            pool: db.pool().clone(),
            action: Action::OptIn,
            clear_runtime_state: Arc::new(|_| {}),
        };

        let reply = handler
            .handle(BridgeInteraction {
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(reply.content.as_deref(), Some(OPTIN_SUCCESS_TEXT));
        assert!(!crate::privacy::is_opted_out(db.pool(), 42).await);
    }

    #[tokio::test]
    async fn datenschutz_db_fehler_leert_runtime_zustand_nicht() {
        let calls = Arc::new(AtomicUsize::new(0));
        let handler = PrivacyHandler {
            pool: unavailable_pool(),
            action: Action::Confirm,
            clear_runtime_state: {
                let calls = calls.clone();
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                })
            },
        };

        let reply = handler
            .handle(BridgeInteraction {
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(reply
            .content
            .expect("sichtbare Antwort")
            .contains("nicht abschließen"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn datenschutz_loeschung_leert_runtime_zustand_nach_db_commit() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test pool");
        let calls = Arc::new(AtomicUsize::new(0));
        let cleared_user = Arc::new(AtomicU64::new(0));
        let handler = PrivacyHandler {
            pool: db.pool().clone(),
            action: Action::Confirm,
            clear_runtime_state: {
                let calls = calls.clone();
                let cleared_user = cleared_user.clone();
                Arc::new(move |user_id| {
                    cleared_user.store(user_id, Ordering::SeqCst);
                    calls.fetch_add(1, Ordering::SeqCst);
                })
            },
        };

        let reply = handler
            .handle(BridgeInteraction {
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(cleared_user.load(Ordering::SeqCst), 42);
        assert!(reply
            .content
            .expect("sichtbare Antwort")
            .contains("wurden gelöscht"));
    }
}
