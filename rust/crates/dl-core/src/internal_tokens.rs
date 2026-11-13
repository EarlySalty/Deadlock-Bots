pub const STEAM_BOT_RECEIVER: &[&str] = &[
    "TWITCH_INTERNAL_API_TOKEN",
    "STEAM_INTERNAL_API_TOKEN",
    "INTERNAL_API_TOKEN",
];

pub const MASTER_BROKER_RECEIVER: &[&str] = &[
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
    "TWITCH_INTERNAL_API_TOKEN",
];

pub const DASHBOARD_INTERNAL_RECEIVER: &[&str] = &[
    "TURNIER_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
    "TWITCH_INTERNAL_API_TOKEN",
];

pub const WEBSITE_API_RECEIVER: &[&str] = &[
    "TWITCH_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "COACHING_BOT_TOKEN",
];

pub const TURNIER_API_RECEIVER: &[&str] = &[
    "TURNIER_INTERNAL_API_TOKEN",
    "MASTER_BROKER_TOKEN",
    "MAIN_BOT_INTERNAL_TOKEN",
    "TWITCH_INTERNAL_API_TOKEN",
];

pub const TWITCH_DASHBOARD_RECEIVER: &[&str] = &["TWITCH_INTERNAL_API_TOKEN"];

pub fn turnier_api_token(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    first_token(lookup, TURNIER_API_RECEIVER)
}

pub fn key_names(keys: &[&str]) -> String {
    keys.join("/")
}

pub fn first_token(lookup: impl Fn(&str) -> Option<String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| present(&lookup, key))
}

pub fn all_tokens(lookup: impl Fn(&str) -> Option<String>, keys: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in keys.iter().filter_map(|key| present(&lookup, key)) {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

pub fn present(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_from(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn sender_waehlt_den_ersten_schluessel_den_der_empfaenger_annimmt() {
        let lookup = lookup_from(&[
            ("MASTER_BROKER_TOKEN", "broker"),
            ("STEAM_INTERNAL_API_TOKEN", " steam "),
            ("TWITCH_INTERNAL_API_TOKEN", "  "),
        ]);
        assert_eq!(
            first_token(&lookup, STEAM_BOT_RECEIVER).as_deref(),
            Some("steam")
        );
        assert_eq!(
            first_token(&lookup, MASTER_BROKER_RECEIVER).as_deref(),
            Some("broker")
        );
        assert_eq!(first_token(lookup_from(&[]), STEAM_BOT_RECEIVER), None);
    }

    #[test]
    fn empfaenger_nimmt_jeden_gesetzten_schluessel_einmal() {
        let lookup = lookup_from(&[
            ("TURNIER_INTERNAL_API_TOKEN", "same"),
            ("MASTER_BROKER_TOKEN", "same"),
            ("TWITCH_INTERNAL_API_TOKEN", "twitch"),
        ]);
        assert_eq!(
            all_tokens(&lookup, DASHBOARD_INTERNAL_RECEIVER),
            vec!["same".to_string(), "twitch".to_string()]
        );
    }
}
