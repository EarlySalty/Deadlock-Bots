//! Join-Quellen-Klassifikation (Port von `user_activity_analyzer.
//! _classify_join_source` + der Dashboard-Auswertung) — als reine, testbare
//! Funktion, die sowohl der rückwirkende Reclassify-Lauf als auch die
//! Server-Statistik nutzen.
//!
//! **Fix gegenüber Python:** Es gab nur einen *Website*-Override (ein per
//! Invite einer Website-Unterseite zuordenbarer Join wird nachträglich auf
//! `website` korrigiert), aber KEINEN Twitch-Override. Folge: Ein Join über
//! einen Streamer-Invite, der beim Eintreffen z. B. als `personal` getaggt
//! wurde, blieb für immer `personal` — selbst wenn die Streamer-Zuordnung
//! später bekannt war. Hier korrigiert ein analoger Twitch-Override das:
//! Lässt sich ein Invite einem Streamer zuordnen (und ist es keine
//! Website-Quelle), zählt der Join als `twitch`.

use std::collections::HashMap;

use serde_json::Value;

/// Quellen-Buckets (Reihenfolge wie im Original, plus Vanity als eigener
/// Discord-Insights-Bucket).
pub const BUCKETS: [&str; 7] = [
    "public",
    "vanity",
    "website",
    "twitch",
    "personal",
    "bot_invite",
    "unknown",
];

/// Slug → Anzeigename der Website-Unterseiten (wie `website_subpage_labels`).
pub fn website_subpage_label(slug: &str) -> &'static str {
    match slug {
        "landing" => "Landing",
        "streamer" => "Streamer",
        "mitspieler" => "Mitspieler",
        "coaching" => "Coaching",
        "helden" => "Helden",
        "guides" => "Guides",
        _ => "Website",
    }
}

/// Ergebnis der Klassifikation eines Joins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub bucket: String,
    pub kind: String,
    pub label: String,
    pub twitch_login: Option<String>,
    pub invite_code: Option<String>,
    pub invite_url: Option<String>,
}

fn meta_str(metadata: &Value, key: &str) -> String {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Klassifiziert einen Join aus seinen Metadaten und den Invite-Lookups.
///
/// - `twitch_invite_lookup`: `invite_code` (lowercase) → `streamer_login`
/// - `website_code_lookup`: `invite_code` (lowercase) → Unterseiten-Slug
pub fn classify(
    metadata: &Value,
    twitch_invite_lookup: &HashMap<String, String>,
    website_code_lookup: &HashMap<String, String>,
) -> Classified {
    let bucket_raw = meta_str(metadata, "join_source_bucket").to_lowercase();
    let mut kind = {
        let k = meta_str(metadata, "join_source_kind");
        if k.is_empty() {
            meta_str(metadata, "join_source_type")
        } else {
            k
        }
        .to_lowercase()
    };
    let mut label = meta_str(metadata, "join_source_label");

    let invite_code = {
        let c = meta_str(metadata, "invite_code");
        (!c.is_empty()).then_some(c)
    };
    let invite_url = {
        let u = meta_str(metadata, "invite_url");
        match (u.is_empty(), &invite_code) {
            (false, _) => Some(u),
            (true, Some(code)) => Some(format!("https://discord.gg/{code}")),
            (true, None) => None,
        }
    };

    let code_lc = invite_code.as_deref().map(str::to_lowercase);
    // twitch_login: aus den Metadaten, sonst aus dem Invite-Lookup.
    let twitch_login = {
        let direct = meta_str(metadata, "twitch_streamer_login").to_lowercase();
        if !direct.is_empty() {
            Some(direct)
        } else {
            code_lc
                .as_deref()
                .and_then(|c| twitch_invite_lookup.get(c))
                .cloned()
        }
    };
    let website_slug = code_lc
        .as_deref()
        .and_then(|c| website_code_lookup.get(c))
        .cloned();

    let mut bucket = bucket_raw.clone();

    // Website-Override (wie im Original): eine Website-Quelle gewinnt immer.
    if let Some(slug) = &website_slug {
        if bucket != "website" {
            bucket = "website".to_string();
        }
        kind = "website_cta".to_string();
        label = format!("Website: {}", website_subpage_label(slug));
    } else if let Some(login) = &twitch_login {
        // FIX: Twitch-Override analog zum Website-Override. Ein einem Streamer
        // zuordenbarer Invite zählt IMMER als twitch.
        if bucket != "twitch" {
            bucket = "twitch".to_string();
        }
        kind = "twitch_streamer".to_string();
        if label.is_empty() {
            label = format!("Twitch: {login}");
        }
    }

    // Fallback, wenn der Bucket (noch) keiner der bekannten ist.
    if !BUCKETS.contains(&bucket.as_str()) {
        bucket = if website_slug.is_some() {
            "website"
        } else if twitch_login.is_some() {
            "twitch"
        } else if matches!(
            kind.as_str(),
            "server_discovery" | "discovery" | "public_discovery"
        ) {
            "public"
        } else if matches!(kind.as_str(), "vanity" | "vanity_url" | "public_vanity") {
            "vanity"
        } else if kind == "bot_invite"
            || metadata
                .get("inviter_bot")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            "bot_invite"
        } else if invite_code.is_some()
            || matches!(
                kind.as_str(),
                "invite_link" | "personal" | "personal_invite"
            )
        {
            "personal"
        } else {
            "unknown"
        }
        .to_string();
    }

    if label.is_empty() {
        label = default_label(&bucket, twitch_login.as_deref(), metadata);
    }

    Classified {
        bucket,
        kind,
        label,
        twitch_login,
        invite_code,
        invite_url,
    }
}

/// Quellen-Label, wenn keines gesetzt ist (wie der `source_label`-Zweig).
fn default_label(bucket: &str, twitch_login: Option<&str>, metadata: &Value) -> String {
    match bucket {
        "public" => "Public".to_string(),
        "vanity" => "Vanity-Link".to_string(),
        "website" => "Website".to_string(),
        "twitch" => match twitch_login {
            Some(login) => format!("Twitch: {login}"),
            None => "Twitch".to_string(),
        },
        "personal" => "Persoenlich: Invite-Link".to_string(),
        "bot_invite" => {
            let name = meta_str(metadata, "inviter_name");
            if name.is_empty() {
                "Bot Invite".to_string()
            } else {
                format!("Bot Invite: {name}")
            }
        }
        _ => "Unbekannt".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lookups() -> (HashMap<String, String>, HashMap<String, String>) {
        let mut tw = HashMap::new();
        tw.insert("abc123".to_string(), "coolstreamer".to_string());
        let mut web = HashMap::new();
        web.insert("web999".to_string(), "coaching".to_string());
        (tw, web)
    }

    #[test]
    fn twitch_override_korrigiert_personal() {
        // DER FIX: Join wurde beim Eintreffen als 'personal' getaggt, aber der
        // Invite gehört einem Streamer → muss twitch werden (Python: blieb personal).
        let (tw, web) = lookups();
        let meta = json!({
            "join_source_bucket": "personal",
            "join_source_kind": "invite_link",
            "invite_code": "ABC123",
            "inviter_name": "earlysalty",
        });
        let c = classify(&meta, &tw, &web);
        assert_eq!(c.bucket, "twitch");
        assert_eq!(c.twitch_login.as_deref(), Some("coolstreamer"));
        assert_eq!(c.label, "Twitch: coolstreamer");
    }

    #[test]
    fn twitch_login_direkt_aus_metadaten() {
        let (tw, web) = lookups();
        let meta = json!({ "join_source_bucket": "unknown", "twitch_streamer_login": "DirectGuy" });
        let c = classify(&meta, &tw, &web);
        assert_eq!(c.bucket, "twitch");
        assert_eq!(c.twitch_login.as_deref(), Some("directguy"));
    }

    #[test]
    fn website_override_gewinnt_vor_twitch() {
        let (tw, web) = lookups();
        // Code ist sowohl website als auch (hypothetisch) — website gewinnt.
        let meta = json!({ "join_source_bucket": "personal", "invite_code": "WEB999" });
        let c = classify(&meta, &tw, &web);
        assert_eq!(c.bucket, "website");
        assert_eq!(c.label, "Website: Coaching");
    }

    #[test]
    fn bereits_twitch_bleibt_twitch() {
        let (tw, web) = lookups();
        let meta = json!({ "join_source_bucket": "twitch", "twitch_streamer_login": "x" });
        assert_eq!(classify(&meta, &tw, &web).bucket, "twitch");
    }

    #[test]
    fn fallbacks_ohne_hinweise() {
        let (tw, web) = lookups();
        // discovery → public
        let m = json!({ "join_source_kind": "server_discovery" });
        assert_eq!(classify(&m, &tw, &web).bucket, "public");
        let m = json!({ "join_source_kind": "vanity" });
        assert_eq!(classify(&m, &tw, &web).bucket, "vanity");
        // invite ohne Zuordnung → personal
        let m = json!({ "invite_code": "zzz", "join_source_kind": "invite_link" });
        assert_eq!(classify(&m, &tw, &web).bucket, "personal");
        // bot
        let m = json!({ "inviter_bot": true });
        assert_eq!(classify(&m, &tw, &web).bucket, "bot_invite");
        // nichts → unknown
        let m = json!({});
        assert_eq!(classify(&m, &tw, &web).bucket, "unknown");
    }
}
