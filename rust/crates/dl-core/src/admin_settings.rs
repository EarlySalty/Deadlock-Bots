//! Vollständige Klassifikation der derzeitigen BotConfig, einschließlich Overrides.
use crate::{
    bot_config::BotConfig,
    settings_catalog::{self, Catalog, Field},
};
pub const SPEC: &str = include_str!("settings_catalog.tsv");
pub fn fields() -> Vec<Field> {
    settings_catalog::fields(SPEC, &[])
}
pub fn catalog(config: &BotConfig) -> Catalog {
    let value = serde_json::to_value(config).expect("validierte Konfiguration ist serialisierbar");
    settings_catalog::build(&value, fields())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn leaves(value: &serde_json::Value, path: &str, found: &mut BTreeSet<String>) {
        if let Some(object) = value.as_object() {
            for (key, value) in object {
                leaves(
                    value,
                    &if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    },
                    found,
                );
            }
        } else {
            found.insert(path.to_owned());
        }
    }
    #[test]
    fn every_config_leaf_is_explicitly_classified() {
        let mut config = BotConfig::parse("schema_version = 1").expect("Default");
        for field in fields()
            .iter()
            .filter(|field| field.path.starts_with("llm.use_cases."))
        {
            let name = field.path.split('.').nth(2).expect("Anwendungsfall");
            let case: crate::bot_config::UseCase =
                serde_json::from_value(name.into()).expect("echter Anwendungsfall");
            config.llm.use_cases.entry(case).or_default();
        }
        let mut found = BTreeSet::new();
        leaves(
            &serde_json::to_value(&config).expect("JSON"),
            "",
            &mut found,
        );
        let fields = fields();
        let known: BTreeSet<_> = fields.iter().map(|field| field.path.clone()).collect();
        assert!(
            found.is_subset(&known),
            "Neue Config-Felder müssen klassifiziert werden: {:?}",
            found.difference(&known).collect::<Vec<_>>()
        );
        for field in fields
            .iter()
            .filter(|field| field.path.starts_with("llm.use_cases."))
        {
            let name = field.path.split('.').nth(2).expect("Anwendungsfall");
            let _: crate::bot_config::UseCase =
                serde_json::from_value(name.into()).expect("echter KI-Anwendungsfall");
        }
    }
    fn loads_with(path: &str, value: serde_json::Value) -> Result<(), String> {
        let fields = fields();
        let changes = std::collections::BTreeMap::from([(path.to_owned(), value)]);
        let normalized = settings_catalog::normalize(&fields, &changes)
            .map_err(|err| format!("Katalog lehnt ab: {err}"))?;
        let mut document: toml_edit::DocumentMut = "schema_version = 1\n"
            .parse()
            .map_err(|err| format!("{err}"))?;
        settings_catalog::apply(&mut document, &normalized)
            .map_err(|err| format!("Anwenden: {err}"))?;
        BotConfig::parse(&document.to_string())
            .map(|_| ())
            .map_err(|err| format!("Laden lehnt ab: {err}"))
    }

    fn catalog_probes(field: &Field) -> Vec<serde_json::Value> {
        let bounds = [&field.min, &field.max].into_iter().flatten();
        if field.kind == settings_catalog::Kind::Integer {
            return bounds
                .map(|bound| serde_json::Value::String(bound.clone()))
                .collect();
        }
        if field.kind == settings_catalog::Kind::IntegerList {
            return bounds
                .map(|bound| {
                    serde_json::Value::Array(vec![
                        serde_json::Value::String(bound.clone());
                        field.length.unwrap_or(1)
                    ])
                })
                .collect();
        }
        let bound = |raw: &Option<String>, open: f64| {
            raw.as_deref()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(open)
        };
        vec![
            serde_json::json!(bound(&field.min, -1e15)),
            serde_json::json!(bound(&field.max, 1e15)),
        ]
    }

    #[test]
    fn jeder_wert_im_katalogbereich_wird_auch_geladen() {
        let mut rejected = Vec::new();
        for field in fields().iter().filter(|field| {
            field.writable
                && matches!(
                    field.kind,
                    settings_catalog::Kind::Integer
                        | settings_catalog::Kind::Number
                        | settings_catalog::Kind::IntegerList
                )
        }) {
            for probe in catalog_probes(field) {
                if let Err(reason) = loads_with(&field.path, probe.clone()) {
                    rejected.extend([format!("{} = {probe}: {reason}", field.path)]);
                }
            }
        }
        assert!(rejected.is_empty(), "{rejected:#?}");
    }

    #[test]
    fn optional_flags_stay_unset_and_protected_values_do_not_leave_server() {
        let config =
            BotConfig::parse("schema_version=1\n[runtime.start]\nowner_id=12345").expect("Config");
        let catalog = catalog(&config);
        assert_eq!(
            catalog.values["runtime.community.concierge_proactive"],
            serde_json::Value::Null
        );
        assert!(!catalog.values.contains_key("runtime.start.owner_id"));
        assert!(catalog
            .fields
            .iter()
            .any(|field| field.path == "runtime.start.owner_id" && !field.writable));
    }
}
