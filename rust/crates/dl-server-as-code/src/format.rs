use serde_json::Value;

use crate::diff::{DiffAction, DiffChange, FilterReason, ServerDiff};
use crate::model::ObjectKind;

/// Erzeugt den menschenlesbaren Review-Text für Mods (Diff-Preview vor Apply).
///
/// Sprache: Deutsch, eine Zeile pro Änderung. Struktur-Löschungen werden als
/// manuelle Schritte markiert (Leitregel „archivieren statt löschen").
/// Permission-Overwrite-Deletes werden von Apply dagegen ausgeführt und deshalb
/// nicht als manueller Schritt gelabelt. Gefilterte Einträge (dynamische
/// Namespaces, dokumentierte Ausnahmen) werden separat ausgewiesen, damit das
/// Review sieht, was der Sync bewusst NICHT anfasst.
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
        DiffAction::Delete => match change.object.kind {
            ObjectKind::Category | ObjectKind::Channel | ObjectKind::Role => {
                format!("LÖSCHEN (manueller Schritt, Apply archiviert/löscht nie selbst): {label}")
            }
            ObjectKind::BotMessage => {
                format!("LÖSCHEN (Apply für Bot-Nachrichten noch nicht implementiert): {label}")
            }
            ObjectKind::PermissionOverwrite => format!("LÖSCHEN: {label}"),
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ObjectKind, ObjectRef, TargetKind};

    fn delete_change(kind: ObjectKind) -> DiffChange {
        DiffChange {
            object: ObjectRef {
                kind,
                guild_id: 1,
                object_id: 2,
                channel_id: (kind == ObjectKind::PermissionOverwrite).then_some(2),
                target_kind: (kind == ObjectKind::PermissionOverwrite).then_some(TargetKind::Role),
                target_id: (kind == ObjectKind::PermissionOverwrite).then_some(3),
                message_key: None,
            },
            action: DiffAction::Delete,
            fields: Vec::new(),
            desired: None,
            actual: Some(serde_json::json!({"name": "objekt"})),
        }
    }

    #[test]
    fn delete_label_unterscheidet_struktur_und_overwrites() {
        let category = describe_change(&delete_change(ObjectKind::Category));
        assert!(category.contains("manueller Schritt"));

        let overwrite = describe_change(&delete_change(ObjectKind::PermissionOverwrite));
        assert!(overwrite.starts_with("LÖSCHEN: Rechte-Overwrite"));
        assert!(!overwrite.contains("manueller Schritt"));
    }
}
