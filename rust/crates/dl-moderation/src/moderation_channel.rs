pub const DEFAULT_MODERATION_CHANNEL_ID: u64 = 1_315_684_135_175_716_978;
pub const DEFAULT_SCAN_CHANNEL_IDS: [u64; 1] = [1_289_721_245_281_292_291];

pub fn parse_channel_ids(raw: Option<&str>, fallback: &[u64]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(raw) = raw {
        for token in raw.split([',', ';', ' ', '\n', '\t', '\r']) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(id) = token.parse::<u64>() {
                if seen.insert(id) {
                    out.push(id);
                }
            }
        }
    }
    if out.is_empty() {
        fallback.to_vec()
    } else {
        out
    }
}

pub fn moderation_channel_id_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> u64 {
    lookup("MODERATION_CHANNEL_ID")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MODERATION_CHANNEL_ID)
}

pub fn scan_channel_ids_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Vec<u64> {
    let raw = lookup("MOD_SCAN_CHANNEL_IDS").or_else(|| lookup("AI_MODERATOR_SCAN_CHANNEL_IDS"));
    parse_channel_ids(raw.as_deref(), &DEFAULT_SCAN_CHANNEL_IDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scan_channels_with_default_and_deduplication() {
        assert_eq!(parse_channel_ids(None, &[1, 2]), vec![1, 2]);
        assert_eq!(
            parse_channel_ids(Some(" 3, bad;4 3\n5"), &[1]),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn reads_moderation_channel_default() {
        assert_eq!(
            moderation_channel_id_from_lookup(|_| None),
            DEFAULT_MODERATION_CHANNEL_ID
        );
        assert_eq!(
            moderation_channel_id_from_lookup(|key| {
                (key == "MODERATION_CHANNEL_ID").then(|| "42".to_string())
            }),
            42
        );
    }
}
