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
    #[test]
    fn brain_runtime_fields_are_not_exposed_or_editable_in_browser() {
        let config = BotConfig::parse(
            "schema_version=1\n[runtime.ai]\nbrain_client_mode='typed'\nbrain_api_endpoint='http://127.0.0.1:8080'\nbrain_api_scopes=['bot.public']\nbrain_api_timeout_ms=2000",
        )
        .expect("gültige Brain-Konfiguration");
        let catalog = catalog(&config);
        for path in [
            "runtime.ai.brain_client_mode",
            "runtime.ai.brain_api_endpoint",
            "runtime.ai.brain_api_scopes",
            "runtime.ai.brain_api_timeout_ms",
        ] {
            let field = catalog
                .fields
                .iter()
                .find(|field| field.path == path)
                .expect("Brain-Feld klassifiziert");
            assert!(!field.writable, "{path}");
            assert!(!catalog.values.contains_key(path), "{path}");
        }
    }
}
