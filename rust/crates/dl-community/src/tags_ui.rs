//! `/meine-tags` — Slash-Oberfläche zur Selbstverwaltung der Voice-Tags
//! (Port von `cogs/tags/interface.py` `MeineTagsView`).
//!
//! Unterschied zum Original: das Python-`MeineTagsView` hält den Auswahlstand
//! (`pending_tags`) im Speicher und schreibt erst beim „Speichern". Der Rust-
//! `InteractionRouter` ist zustandslos — deshalb ist die DB hier die einzige
//! Quelle der Wahrheit: jede Auswahl persistiert **sofort** (`set_/clear_user_tag`),
//! liest frisch via `get_user_tags` und rendert die Nachricht neu (Discord
//! `UPDATE_MESSAGE`). Das ist robuster (kein View-Timeout-Verlust) und braucht
//! keinen serverseitigen Sitzungszustand. Die Ansicht ist ephemer, daher ersetzt
//! die Sichtbarkeit den `interaction_check` (nur der Owner sieht/bedient sie).

use std::collections::HashMap;
use std::sync::Arc;

use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::{json, Value};

use crate::tags::TagService;

const UNSET: &str = "__unset__";

#[derive(Clone, Copy)]
enum Action {
    Open,
    SelectAge,
    SelectTone,
    Reset,
}

struct TagsHandler {
    service: Arc<TagService>,
    action: Action,
}

fn age_label(value: Option<&str>) -> &str {
    match value {
        None => "Nicht gesetzt",
        Some("25+") => "25+",
        Some("u25") => "U25",
        Some(other) => other,
    }
}

fn tone_label(value: Option<&str>) -> &str {
    match value {
        None => "Nicht gesetzt",
        Some("banter_ok") => "Banter-OK",
        Some("ragebaiter_free") => "Ragebaiter-Free",
        Some(other) => other,
    }
}

/// Baut ein String-Select (Action-Row) mit dem aktuell gesetzten Wert als Default.
fn select(key: &str, placeholder: &str, opts: &[(&str, &str, Option<&str>)], current: Option<&str>) -> Value {
    let options: Vec<Value> = opts
        .iter()
        .map(|(label, value, desc)| {
            let mut o = json!({
                "label": label,
                "value": value,
                "default": current.unwrap_or(UNSET) == *value,
            });
            if let Some(d) = desc {
                o["description"] = json!(d);
            }
            o
        })
        .collect();
    json!({
        "type": 1,
        "components": [{
            "type": 3,
            "custom_id": format!("tags:{key}"),
            "placeholder": placeholder,
            "min_values": 1,
            "max_values": 1,
            "options": options,
        }],
    })
}

/// Rendert Embed + Komponenten aus dem aktuellen (persistierten) Tag-Stand.
/// `update` = true → die bestehende Nachricht editieren (Komponenten-Antwort).
fn render(tags: &HashMap<String, String>, status: Option<&str>, update: bool) -> BridgeReply {
    let age = tags.get("age").map(String::as_str);
    let tone = tags.get("tone").map(String::as_str);

    let mut desc =
        "Wähle deinen Lieblings-Ton, damit Voice-Lobbies besser zu dir passen.".to_string();
    if let Some(s) = status {
        desc = format!("{desc}\n\n{s}");
    }
    let embed = json!({
        "title": "Meine Tags",
        "description": desc,
        "color": 0x5865F2,
        "fields": [{
            "name": "Aktuell",
            "value": format!("Alter: {}\nTonfall: {}", age_label(age), tone_label(tone)),
            "inline": false,
        }],
    });

    let age_select = select(
        "age",
        "Alter auswählen",
        &[("Nicht gesetzt", UNSET, None), ("25+", "25+", None), ("U25", "u25", None)],
        age,
    );
    let tone_select = select(
        "tone",
        "Tonfall auswählen",
        &[
            ("Nicht gesetzt", UNSET, None),
            ("Banter-OK", "banter_ok", Some("Hier darf es mal ruppiger werden.")),
            ("Ragebaiter-Free", "ragebaiter_free", Some("Hier bitte ohne gezielte Provokationen.")),
        ],
        tone,
    );
    let reset_disabled = age.is_none() && tone.is_none();
    let reset_row = json!({
        "type": 1,
        "components": [{
            "type": 2,
            "style": 2,
            "label": "Reset",
            "custom_id": "tags:reset",
            "disabled": reset_disabled,
        }],
    });

    BridgeReply {
        embeds: vec![embed],
        components: Some(json!([age_select, tone_select, reset_row])),
        // Initiale Slash-Antwort ephemer; Updates behalten die Sichtbarkeit.
        ephemeral: !update,
        update_message: update,
        ..Default::default()
    }
}

#[async_trait::async_trait]
impl InteractionHandler for TagsHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let uid = interaction.user_id;
        match self.action {
            Action::Open => {
                let tags = self.service.get_user_tags(uid).await;
                render(&tags, None, false)
            }
            Action::SelectAge | Action::SelectTone => {
                let key = if matches!(self.action, Action::SelectAge) {
                    "age"
                } else {
                    "tone"
                };
                let selected = interaction.values.first().map(String::as_str).unwrap_or(UNSET);
                if selected == UNSET {
                    let _ = self.service.clear_user_tag(uid, key).await;
                } else {
                    let _ = self.service.set_user_tag(uid, key, selected).await;
                }
                let tags = self.service.get_user_tags(uid).await;
                render(&tags, Some("Auswahl gespeichert."), true)
            }
            Action::Reset => {
                let _ = self.service.clear_user_tag(uid, "age").await;
                let _ = self.service.clear_user_tag(uid, "tone").await;
                let tags = self.service.get_user_tags(uid).await;
                render(&tags, Some("Deine Tags wurden zurückgesetzt."), true)
            }
        }
    }
}

fn command_spec() -> CommandSpec {
    CommandSpec {
        definition: json!({
            "name": "meine-tags",
            "description": "Verwalte deine sichtbaren Voice-Tags.",
            "type": 1,
            "dm_permission": false,
        }),
    }
}

// ── Mod-Tags (`/mod-tag set|remove|list`) ────────────────────────────────────
// Port von `cogs/tags/mod_commands.py`. Der Mod-Gate läuft serverseitig über
// `default_member_permissions` = MANAGE_MESSAGES (wie andere Mod-Commands);
// der Rollen-Fallback (MOD_ROLE_ID) und der Log-Channel-Post des Originals
// entfallen, weil der Router-`register` weder einen Channel-Sender noch
// Rollen-Kontext bekommt (kein main.rs-Eingriff).

/// MANAGE_MESSAGES (0x2000) als Discord-Permission-Bitstring.
const MANAGE_MESSAGES: &str = "8192";

#[derive(Clone, Copy)]
enum ModAction {
    Set,
    Remove,
    List,
}

struct ModTagHandler {
    service: Arc<TagService>,
    action: ModAction,
}

/// Mention-Form einer User-ID (`<@id>`). Die User-Option kommt aus dem
/// Dispatch als JSON-Number (siehe `option_to_pair`).
fn opt_user_id(interaction: &BridgeInteraction, key: &str) -> Option<u64> {
    interaction.options.get(key).and_then(Value::as_u64)
}

/// Expiry-Anzeige wie Python `_format_expiry`: `unbegrenzt` oder das UTC-Datum.
fn format_expiry(expires_at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match expires_at {
        None => "unbegrenzt".to_string(),
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
    }
}

#[async_trait::async_trait]
impl InteractionHandler for ModTagHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let Some(target) = opt_user_id(&interaction, "user") else {
            return BridgeReply::ephemeral_text("Kein User angegeben.");
        };
        match self.action {
            ModAction::Set => {
                let tag = interaction
                    .options
                    .get("tag")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let reason = interaction
                    .options
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let expires_in_days = interaction.options.get("expires_in_days").and_then(Value::as_i64);
                if let Some(days) = expires_in_days {
                    if days <= 0 {
                        return BridgeReply::ephemeral_text(
                            "`expires_in_days` muss größer als 0 sein.",
                        );
                    }
                }
                // expires_in_days → festes Datum; sonst greift in add_mod_tag die
                // Default-Laufzeit (ragebaiter: 14 Tage).
                let expires_at = expires_in_days
                    .map(|d| chrono::Utc::now() + chrono::Duration::days(d));
                match self
                    .service
                    .add_mod_tag(target, tag, interaction.user_id, reason, expires_at)
                    .await
                {
                    Ok(()) => BridgeReply::ephemeral_text(format!(
                        "Mod-Tag `{tag}` wurde für <@{target}> gesetzt."
                    )),
                    Err(_) => BridgeReply::ephemeral_text(format!(
                        "Unbekanntes Mod-Tag `{tag}`."
                    )),
                }
            }
            ModAction::Remove => {
                let tag = interaction
                    .options
                    .get("tag")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !self.service.has_active_mod_tag(target, tag).await {
                    return BridgeReply::ephemeral_text(format!(
                        "<@{target}> hat kein aktives Mod-Tag `{tag}`."
                    ));
                }
                let _ = self
                    .service
                    .remove_mod_tag(target, tag, interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text(format!(
                    "Mod-Tag `{tag}` wurde von <@{target}> entfernt."
                ))
            }
            ModAction::List => {
                let rows = self.service.active_mod_tags(target).await;
                let title = format!("Mod-Tags für {target}");
                if rows.is_empty() {
                    let embed = json!({
                        "title": title,
                        "description": "Keine aktiven Mod-Tags.",
                        "color": 0xE67E22, // discord.Color.orange()
                    });
                    return BridgeReply {
                        embeds: vec![embed],
                        ephemeral: true,
                        ..Default::default()
                    };
                }
                let fields: Vec<Value> = rows
                    .iter()
                    .map(|row| {
                        json!({
                            "name": row.tag,
                            "value": format!(
                                "Reason: {}\nExpires: {}\nSet by: <@{}>",
                                row.reason.as_deref().unwrap_or("-"),
                                format_expiry(row.expires_at),
                                row.set_by,
                            ),
                            "inline": false,
                        })
                    })
                    .collect();
                let embed = json!({
                    "title": title,
                    "color": 0xE67E22,
                    "fields": fields,
                });
                BridgeReply {
                    embeds: vec![embed],
                    ephemeral: true,
                    ..Default::default()
                }
            }
        }
    }
}

/// Vollständige `mod-tag`-Gruppendefinition (alle Subcommands). Der Router
/// dedupt CommandSpecs nach `name` (letzter gewinnt), darum trägt jeder
/// `on_command`-Aufruf dieselbe komplette Definition.
fn mod_tag_group_spec() -> CommandSpec {
    let tag_option = json!({
        "type": 3, // STRING
        "name": "tag",
        "description": "Mod-Tag",
        "required": true,
        "choices": [{ "name": "Ragebaiter", "value": "ragebaiter" }],
    });
    let user_option = json!({
        "type": 6, // USER
        "name": "user",
        "description": "Betroffener User",
        "required": true,
    });
    CommandSpec {
        definition: json!({
            "name": "mod-tag",
            "description": "Moderationsbefehle für Mod-Tags.",
            "type": 1,
            "dm_permission": false,
            "default_member_permissions": MANAGE_MESSAGES,
            "options": [
                {
                    "type": 1, // SUB_COMMAND
                    "name": "set",
                    "description": "Setzt ein Mod-Tag für einen User.",
                    "options": [
                        user_option,
                        tag_option,
                        {
                            "type": 3, // STRING
                            "name": "reason",
                            "description": "Optionaler Grund",
                            "required": false,
                        },
                        {
                            "type": 4, // INTEGER
                            "name": "expires_in_days",
                            "description": "Ablauf in Tagen",
                            "required": false,
                        },
                    ],
                },
                {
                    "type": 1,
                    "name": "remove",
                    "description": "Entfernt ein Mod-Tag von einem User.",
                    "options": [user_option, tag_option],
                },
                {
                    "type": 1,
                    "name": "list",
                    "description": "Zeigt aktive Mod-Tags eines Users.",
                    "options": [user_option],
                },
            ],
        }),
    }
}

/// Registriert `/meine-tags` + die Select-/Reset-Komponenten-Handler.
pub fn register(router: &mut InteractionRouter, service: Arc<TagService>) {
    router.on_command(
        "meine-tags",
        command_spec(),
        Arc::new(TagsHandler {
            service: service.clone(),
            action: Action::Open,
        }),
    );
    router.on_custom_id(
        "tags:age",
        Arc::new(TagsHandler {
            service: service.clone(),
            action: Action::SelectAge,
        }),
    );
    router.on_custom_id(
        "tags:tone",
        Arc::new(TagsHandler {
            service: service.clone(),
            action: Action::SelectTone,
        }),
    );
    router.on_custom_id(
        "tags:reset",
        Arc::new(TagsHandler {
            service: service.clone(),
            action: Action::Reset,
        }),
    );

    // /mod-tag set|remove|list — qualifizierte Subcommand-Namen; jeder Spec
    // trägt die volle Gruppendefinition (Router dedupt nach name).
    router.on_command(
        "mod-tag set",
        mod_tag_group_spec(),
        Arc::new(ModTagHandler {
            service: service.clone(),
            action: ModAction::Set,
        }),
    );
    router.on_command(
        "mod-tag remove",
        mod_tag_group_spec(),
        Arc::new(ModTagHandler {
            service: service.clone(),
            action: ModAction::Remove,
        }),
    );
    router.on_command(
        "mod-tag list",
        mod_tag_group_spec(),
        Arc::new(ModTagHandler {
            service,
            action: ModAction::List,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_und_defaults() {
        assert_eq!(age_label(None), "Nicht gesetzt");
        assert_eq!(age_label(Some("u25")), "U25");
        assert_eq!(tone_label(Some("banter_ok")), "Banter-OK");
        // Default-Markierung: aktueller Wert ist default, sonst nicht.
        let sel = select("age", "x", &[("Nicht gesetzt", UNSET, None), ("25+", "25+", None)], Some("25+"));
        let opts = sel["components"][0]["options"].as_array().unwrap();
        assert_eq!(opts[0]["default"], json!(false)); // UNSET
        assert_eq!(opts[1]["default"], json!(true)); // 25+
    }

    #[test]
    fn mod_tag_spec_enthaelt_alle_subcommands() {
        let spec = mod_tag_group_spec();
        let def = &spec.definition;
        assert_eq!(def["name"], json!("mod-tag"));
        assert_eq!(def["default_member_permissions"], json!("8192"));
        let subs: Vec<&str> = def["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["name"].as_str().unwrap())
            .collect();
        assert_eq!(subs, vec!["set", "remove", "list"]);
        // set hat user/tag/reason/expires_in_days
        let set_opts = def["options"][0]["options"].as_array().unwrap();
        assert_eq!(set_opts.len(), 4);
        assert_eq!(set_opts[0]["type"], json!(6)); // USER
        assert_eq!(set_opts[1]["choices"][0]["value"], json!("ragebaiter"));
    }

    #[test]
    fn format_expiry_wie_python() {
        assert_eq!(format_expiry(None), "unbegrenzt");
        let dt = chrono::DateTime::parse_from_rfc3339("2026-06-14T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(format_expiry(Some(dt)), "2026-06-14");
    }

    #[test]
    fn render_setzt_update_und_ephemeral() {
        let tags = HashMap::new();
        let open = render(&tags, None, false);
        assert!(open.ephemeral && !open.update_message);
        let upd = render(&tags, Some("x"), true);
        assert!(!upd.ephemeral && upd.update_message);
        // Reset deaktiviert, wenn keine Tags gesetzt.
        let row = &upd.components.as_ref().unwrap()[2]["components"][0];
        assert_eq!(row["disabled"], json!(true));
    }
}
