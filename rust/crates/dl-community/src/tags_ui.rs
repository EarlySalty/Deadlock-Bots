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
            service,
            action: Action::Reset,
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
