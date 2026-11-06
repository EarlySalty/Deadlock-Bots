use async_trait::async_trait;
use serde_json::{json, Value};

pub const TWITCH_LINK_CUSTOM_ID: &str = "concierge:twitch:link";
pub const TWITCH_LINK_BUTTON_LABEL: &str = "Twitch verknüpfen";
pub const TWITCH_LINK_OPEN_LABEL: &str = "Jetzt mit Twitch verknüpfen";
pub const VERIFY_CHANNEL_ID: u64 = 1_398_021_105_339_334_666;
pub const STEAM_LINK_CUSTOM_ID: &str = "steam:openid";
pub const STEAM_LINK_BUTTON_LABEL: &str = "Steam verknüpfen";

pub const TOUR_COMMUNITY_BLOCK: &str = "**Community und Streamer**\nVerknüpf dein Discord mit Twitch, dann sammelst du Punkte, wenn du bei unseren Partner-Streamern zuschaust und mitchattest. Die Punkte landen zusammen mit deinen Voice-Punkten in einem gemeinsamen Leaderboard. Jede Woche gibt es einen Clip-Contest, die Community stimmt ab und die Links und Credits der Top 3 werden dach_lock zur kuratierten Auswahl bereitgestellt. Und wenn du einen Streamer kennst, der zu uns passt, kannst du ihn für unser Streamer-Programm vorschlagen. Fragen dazu? Schreib mir einfach, zum Beispiel \"Wie bekomme ich Punkte?\".";

pub const TWITCH_LINK_READY_TEXT: &str = "Hier ist dein Link. Discord fragt dich kurz, ob wir deine verknüpften Konten sehen dürfen, danach bist du fertig. Wenn Twitch in deinen Discord-Einstellungen unter Verbindungen noch fehlt, füg es dort zuerst hinzu und klick dann nochmal.";

pub const TWITCH_LINK_FALLBACK_TEXT: &str = "Den Link kann ich dir hier gerade nicht direkt geben. Geh in <#1398021105339334666> und klick dort auf \"Twitch verknüpfen\", das ist derselbe Weg. Falls Twitch in deinen Discord-Einstellungen unter Verbindungen noch fehlt, füg es dort zuerst hinzu.";

pub fn twitch_link_button() -> Value {
    json!({
        "type": 2,
        "style": 2,
        "label": TWITCH_LINK_BUTTON_LABEL,
        "custom_id": TWITCH_LINK_CUSTOM_ID,
    })
}

pub fn steam_link_button() -> Value {
    json!({
        "type": 2,
        "style": 2,
        "label": STEAM_LINK_BUTTON_LABEL,
        "custom_id": STEAM_LINK_CUSTOM_ID,
    })
}

pub fn twitch_link_url_button(url: &str) -> Value {
    json!({ "type": 2, "style": 5, "label": TWITCH_LINK_OPEN_LABEL, "url": url })
}

#[async_trait]
pub trait TwitchLinkSource: Send + Sync {
    async fn twitch_link_url(&self, discord_user_id: u64) -> Option<String>;
}

pub struct VerifyChannelOnly;

#[async_trait]
impl TwitchLinkSource for VerifyChannelOnly {
    async fn twitch_link_url(&self, _discord_user_id: u64) -> Option<String> {
        None
    }
}

pub fn usable_link(url: &str) -> Option<&str> {
    let url = url.trim();
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let discord_host = host == "discord.com" || host.ends_with(".discord.com");
    (parsed.scheme() == "https" && discord_host).then_some(url)
}

pub fn twitch_link_reply(url: Option<&str>) -> (&'static str, Vec<Value>) {
    match url.and_then(usable_link) {
        Some(url) => (TWITCH_LINK_READY_TEXT, vec![twitch_link_url_button(url)]),
        None => (TWITCH_LINK_FALLBACK_TEXT, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TEXTS: [&str; 3] = [
        TOUR_COMMUNITY_BLOCK,
        TWITCH_LINK_READY_TEXT,
        TWITCH_LINK_FALLBACK_TEXT,
    ];

    #[test]
    fn texte_ohne_gedankenstriche_mit_umlauten_und_in_discord_grenzen() {
        for text in ALL_TEXTS {
            assert!(!text.contains('–') && !text.contains('—'), "{text}");
            assert!(!text.contains(" - "), "{text}");
            assert!(text.encode_utf16().count() <= 2000, "{text}");
            for fachwort in ["OAuth", "Scope", "Connection", "Register", "Ledger"] {
                assert!(!text.contains(fachwort), "{fachwort} in {text}");
            }
        }
        assert!(TOUR_COMMUNITY_BLOCK.contains("Verknüpf"));
        assert!(TWITCH_LINK_FALLBACK_TEXT.contains(&format!("<#{VERIFY_CHANNEL_ID}>")));
    }

    #[test]
    fn verknuepfungsknoepfe_tragen_ihre_custom_ids() {
        let button = twitch_link_button();
        assert_eq!(button["custom_id"], TWITCH_LINK_CUSTOM_ID);
        assert_eq!(button["label"], TWITCH_LINK_BUTTON_LABEL);
        let button = steam_link_button();
        assert_eq!(button["custom_id"], STEAM_LINK_CUSTOM_ID);
        assert_eq!(button["label"], STEAM_LINK_BUTTON_LABEL);
    }
    #[test]
    fn link_antwort_nimmt_nur_discord_links_sonst_fallback() {
        let (text, buttons) = twitch_link_reply(Some(
            "https://discord.com/oauth2/authorize?client_id=1&state=x",
        ));
        assert_eq!(text, TWITCH_LINK_READY_TEXT);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0]["style"], 5);
        assert!(buttons[0]["url"]
            .as_str()
            .unwrap_or_default()
            .starts_with("https://discord.com/oauth2/authorize"));
        for bad in [
            None,
            Some(""),
            Some("http://discord.com/oauth2/authorize"),
            Some("https://evil.example/discord.com"),
            Some("javascript:alert(1)"),
        ] {
            let (text, buttons) = twitch_link_reply(bad);
            assert_eq!(text, TWITCH_LINK_FALLBACK_TEXT, "{bad:?}");
            assert!(buttons.is_empty());
        }
    }

    #[tokio::test]
    async fn standardquelle_liefert_keinen_link() {
        assert_eq!(VerifyChannelOnly.twitch_link_url(42).await, None);
    }
}
