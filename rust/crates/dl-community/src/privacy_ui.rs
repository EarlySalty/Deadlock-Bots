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
    scout_client: Option<Arc<dl_bridges::twitch::TwitchApiClient>>,
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
                Ok(()) => {
                    let pending: bool = match sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM community.scout_privacy_outbox WHERE discord_id = $1)",
                    ).bind(uid).fetch_one(&self.pool).await {
                        Ok(pending) => pending,
                        Err(_) => return BridgeReply::ephemeral_text(OPTIN_ERROR_TEXT),
                    };
                    if !pending {
                        return BridgeReply::ephemeral_text(OPTIN_SUCCESS_TEXT);
                    }
                    match &self.scout_client {
                        Some(client) if crate::scout_privacy::deliver_user(&self.pool, client, uid).await.is_ok() =>
                            BridgeReply::ephemeral_text(OPTIN_SUCCESS_TEXT),
                        Some(_) => BridgeReply::ephemeral_text("Deine erneute Einwilligung ist lokal gespeichert. Die Twitch-Community-Brücke hat sie noch nicht bestätigt; der gespeicherte Auftrag wird erneut versucht. Verknüpfe dein Twitchkonto selbst erneut, damit neue Zuschaueraktivität erfasst werden kann."),
                        None => BridgeReply::ephemeral_text("Deine erneute Einwilligung ist lokal gespeichert, aber noch nicht an die Twitch-Community-Brücke zugestellt. Die Brücke ist hier nicht eingerichtet; der gespeicherte Auftrag bleibt offen, bis sie wieder verfügbar ist. Bitte dem Team melden. Verknüpfe dein Twitchkonto selbst erneut, damit neue Zuschaueraktivität erfasst werden kann."),
                    }
                }
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
                        let remote_done = match &self.scout_client {
                            Some(client) => crate::scout_privacy::deliver_user(&self.pool, client, uid).await.is_ok(),
                            None => false,
                        };
                        if !remote_done {
                            return BridgeReply::ephemeral_text(if self.scout_client.is_some() {
                                "Deine lokalen Daten wurden gelöscht und der Opt-out ist aktiv. Die Löschung der Twitch-Community-Vorschlagskopien ist noch nicht bestätigt. Der gespeicherte Löschauftrag wird erneut versucht; die vollständige Löschung ist noch offen."
                            } else {
                                "Deine lokalen Daten wurden gelöscht und der Opt-out ist aktiv. Die Twitch-Community-Brücke ist hier nicht eingerichtet; ihr gespeicherter Löschauftrag bleibt offen, bis sie wieder verfügbar ist. Die vollständige Löschung ist noch offen. Bitte dem Team melden."
                            });
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
                    let remote = match &self.scout_client {
                        Some(client) => crate::scout_privacy::export(client, uid).await,
                        None => Err("Twitch-Community-Brücke nicht verfügbar".into()),
                    };
                    export_reply(value, remote)
                }
                Err(_) => BridgeReply::ephemeral_text(
                    "⚠️ Konnte den Datenexport nicht erstellen. Bitte später erneut versuchen.",
                ),
            },
        }
    }
}

fn export_reply(
    mut value: serde_json::Value,
    remote: Result<serde_json::Value, String>,
) -> BridgeReply {
    let incomplete = remote.is_err();
    match remote {
        Ok(remote) => value["twitch_community_scout"] = remote,
        Err(error) => {
            tracing::warn!(%error, "Twitch-Community-Export nicht vollständig");
            value["twitch_community_scout"] = json!({
                "status": "unavailable", "complete": false,
                "retry": "Export nach Wiederherstellung der Twitch-Community-Brücke erneut anfordern."
            });
        }
    }
    let bytes = match serde_json::to_vec_pretty(&value) {
        Ok(bytes) => bytes,
        Err(_) => {
            return BridgeReply::ephemeral_text(
                "Konnte den Datenexport nicht erstellen. Bitte später erneut versuchen.",
            )
        }
    };
    BridgeReply {
        content: Some(if incomplete {
            "Hier sind deine lokal erfassten Daten als JSON-Datei. Der Export ist unvollständig: Die Twitch-Community-Vorschlagskopien sind gerade nicht verfügbar. Ihr fehlender Teil ist in der Datei gekennzeichnet. Bitte den Export später erneut anfordern. Personenbezogene Fremd-IDs sind geschwärzt."
        } else {
            "Hier sind die von diesem Ablauf erfassten Daten als JSON-Datei. Personenbezogene Fremd-IDs sind geschwärzt."
        }.to_string()),
        ephemeral: true,
        attachments: vec![BridgeAttachment { filename: "deine-daten.json".to_string(), data: bytes }],
        ..Default::default()
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
    register_with_scout(router, pool, clear_runtime_state, None);
}

pub fn register_with_scout(
    router: &mut InteractionRouter,
    pool: PgPool,
    clear_runtime_state: RuntimePrivacyCleanup,
    scout_client: Option<Arc<dl_bridges::twitch::TwitchApiClient>>,
) {
    router.on_command(
        "datenschutz",
        command_spec(
            "datenschutz",
            "Exportiere oder lösche deine erfassten Daten; nur die Löschung setzt zusätzlich den Opt-out.",
        ),
        Arc::new(PrivacyHandler {
            pool: pool.clone(),
            scout_client: scout_client.clone(),
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
            scout_client: scout_client.clone(),
            action: Action::OptIn,
            clear_runtime_state: clear_runtime_state.clone(),
        }),
    );
    router.on_custom_id(
        "privacy:confirm",
        Arc::new(PrivacyHandler {
            pool: pool.clone(),
            scout_client: scout_client.clone(),
            action: Action::Confirm,
            clear_runtime_state: clear_runtime_state.clone(),
        }),
    );
    router.on_custom_id(
        "privacy:export",
        Arc::new(PrivacyHandler {
            pool,
            scout_client: scout_client.clone(),
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

    #[test]
    fn erfolgreicher_export_enthaelt_lokale_und_remote_daten() {
        let reply = export_reply(
            json!({"user_id":42,"tables":{"fixture":[{"value":"lokal"}]}}),
            Ok(json!({"discord_user_id":"42","suggestions":[{"reason":"eigener Grund"}]})),
        );
        assert!(!reply
            .content
            .as_ref()
            .expect("Antwort")
            .contains("unvollständig"));
        let value: serde_json::Value =
            serde_json::from_slice(&reply.attachments[0].data).expect("Exportdatei");
        assert_eq!(value["tables"]["fixture"][0]["value"], "lokal");
        assert_eq!(
            value["twitch_community_scout"]["suggestions"][0]["reason"],
            "eigener Grund"
        );
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn export_behaelt_lokale_datei_bei_fehlender_oder_ausgefallener_scout_bruecke() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("Echte Wegwerf-DB");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Testlistener");
        let address = listener.local_addr().expect("Adresse");
        let router =
            axum::Router::new().fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE });
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("Testserver");
        });
        let client = dl_bridges::twitch::TwitchApiClient::try_new(
            format!("http://{address}"),
            "fixture-token",
            std::time::Duration::from_secs(2),
            false,
        )
        .expect("Client");
        for scout_client in [None, Some(client)] {
            let handler = PrivacyHandler {
                pool: db.pool().clone(),
                action: Action::Export,
                clear_runtime_state: Arc::new(|_| {}),
                scout_client,
            };
            let reply = handler
                .handle(BridgeInteraction {
                    user_id: 42,
                    ..BridgeInteraction::default()
                })
                .await;
            assert!(reply.ephemeral);
            assert!(reply
                .content
                .as_ref()
                .expect("Antwort")
                .contains("unvollständig"));
            assert_eq!(reply.attachments.len(), 1);
            let value: serde_json::Value =
                serde_json::from_slice(&reply.attachments[0].data).expect("Lokale Exportdatei");
            assert_eq!(value["user_id"], 42);
            assert!(value["tables"].is_object());
            assert_eq!(value["twitch_community_scout"]["status"], "unavailable");
            assert_eq!(value["twitch_community_scout"]["complete"], false);
        }
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn optin_db_fehler_bestaetigt_keinen_erfolg() {
        let handler = PrivacyHandler {
            scout_client: None,
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
    async fn optin_ohne_remote_bestaetigung_meldet_ausstehenden_auftrag() {
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
            scout_client: None,
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

        let text = reply.content.expect("Antwort");
        assert!(text.contains("noch nicht an die Twitch-Community-Brücke zugestellt"));
        assert!(text.contains("nicht eingerichtet"));
        assert!(!text.contains("wird wiederholt"));
        assert_ne!(text, OPTIN_SUCCESS_TEXT);
        let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM community.scout_privacy_outbox WHERE discord_id = 42 AND action = 'consent'")
            .fetch_one(db.pool()).await.expect("Gespeicherter Retryauftrag");
        assert_eq!(queued, 1);
        assert!(!crate::privacy::is_opted_out(db.pool(), 42).await);
    }

    #[tokio::test]
    async fn datenschutz_db_fehler_leert_runtime_zustand_nicht() {
        let calls = Arc::new(AtomicUsize::new(0));
        let handler = PrivacyHandler {
            scout_client: None,
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
            scout_client: None,
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
