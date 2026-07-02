use serde_json::Value;

use crate::diff::{DiffAction, DiffChange, FilterReason, ServerDiff};
use crate::model::ObjectKind;

/// Erzeugt den menschenlesbaren Review-Text für Mods (Diff-Preview vor Apply).
///
/// Sprache: Deutsch, eine Zeile pro Änderung. Löschungen werden explizit als
/// manuelle Schritte markiert (Leitregel „archivieren statt löschen" — Apply
/// führt sie nie selbst aus). Gefilterte Einträge (dynamische Namespaces,
/// dokumentierte Ausnahmen) werden separat ausgewiesen, damit das Review
/// sieht, was der Sync bewusst NICHT anfasst.
pub fn human_summary(diff: &ServerDiff) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Server-Diff für Guild {} — {} Änderung(en), {} bewusst ausgefiltert.\n",
        diff.guild_id,
        diff.changes.len(),
        diff.filtered.len()
    ));

    if diff.changes.is_empty() {
        out.push_str("\nKeine Abweichungen: Ist-Zustand entspricht dem Soll-Modell.\n");
    } else {
        out.push_str("\nÄnderungen (werden erst nach Bestätigung angewendet):\n");
        for change in &diff.changes {
            out.push_str(&format!("- {}\n", describe_change(change)));
        }
    }

    if !diff.filtered.is_empty() {
        out.push_str("\nAusgefiltert (kein Sync-Eingriff):\n");
        for filtered in &diff.filtered {
            let reason = match &filtered.reason {
                FilterReason::DynamicNamespace { system_name, .. } => {
                    format!("dynamischer Namespace „{system_name}“")
                }
                FilterReason::DocumentedException { exception_key, .. } => {
                    format!("dokumentierte Ausnahme „{exception_key}“")
                }
            };
            out.push_str(&format!(
                "- {} — {}\n",
                object_label(&filtered.change),
                reason
            ));
        }
    }

    out
}

/// Eine Zeile pro Änderung: Aktion, Objekt, betroffene Felder.
fn describe_change(change: &DiffChange) -> String {
    let label = object_label(change);
    match change.action {
        DiffAction::Create => format!("ANLEGEN: {label}"),
        DiffAction::Update => {
            let fields: Vec<&str> = change.fields.iter().map(|f| f.field.as_str()).collect();
            if fields.is_empty() {
                format!("ÄNDERN: {label}")
            } else {
                format!("ÄNDERN: {label} — Felder: {}", fields.join(", "))
            }
        }
        DiffAction::Delete => {
            format!("LÖSCHEN (manueller Schritt, Apply archiviert/löscht nie selbst): {label}")
        }
    }
}

/// Objekt-Beschriftung: Art + Name (falls im Soll/Ist-JSON vorhanden) + ID.
fn object_label(change: &DiffChange) -> String {
    let kind = match change.object.kind {
        ObjectKind::Category => "Kategorie",
        ObjectKind::Channel => "Kanal",
        ObjectKind::Role => "Rolle",
        ObjectKind::PermissionOverwrite => "Rechte-Overwrite",
        ObjectKind::BotMessage => "Bot-Nachricht",
    };
    let name = change
        .desired
        .as_ref()
        .and_then(extract_name)
        .or_else(|| change.actual.as_ref().and_then(extract_name));
    match name {
        Some(name) => format!("{kind} „{name}“ ({})", change.object.object_id),
        None => format!("{kind} {}", change.object.object_id),
    }
}

fn extract_name(value: &Value) -> Option<String> {
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
}
