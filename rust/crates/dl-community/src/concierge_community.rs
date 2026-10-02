//! Community-und-Streamer-Bausteine des Concierge (Paket G der Community-Streamer-Brücke).
//!
//! Bewusst eigenständig: Das Modul kennt weder den Concierge-Zustand noch die
//! Datenbank, nur Texte, Buttons, die deterministische Fragenerkennung und die
//! Schnittstelle zum Twitch-Verknüpfungslink. Damit wandert es beim Umzug des
//! Concierge ins Deadlock Brain unverändert mit. Die Regeln stammen aus
//! `.tasks/2026-10-01-community-streamer-bruecke/PLAN.md`; die Langfassung für
//! Menschen steht in `docs/community-punkte-und-streamer.md`.

use async_trait::async_trait;
use serde_json::{json, Value};

/// Custom-ID des Concierge-Buttons "Twitch verknüpfen".
pub const TWITCH_LINK_CUSTOM_ID: &str = "concierge:twitch:link";
pub const TWITCH_LINK_BUTTON_LABEL: &str = "Twitch verknüpfen";
pub const TWITCH_LINK_OPEN_LABEL: &str = "Jetzt mit Twitch verknüpfen";
/// Kanal mit dem Verify-Panel, in dem Paket A den Knopf "Twitch verknüpfen" anbietet.
pub const VERIFY_CHANNEL_ID: u64 = 1_398_021_105_339_334_666;
/// Kanal, in dem die Partner-Streams angekündigt werden (bereits Teil der Tour).
pub const PARTNER_STREAMS_CHANNEL_ID: u64 = 1_304_169_815_505_637_458;
/// Server- und Bot-Fragen an Menschen.
pub const SERVER_FRAGEN_CHANNEL_ID: u64 = 1_491_953_161_747_955_853;
/// Kanal mit dem Knopf "Streamer vorschlagen" (Clip-Panel, Paket F,
/// `streamer_suggest::PANEL_CHANNEL_ID`; ein Test hält beide gleich).
pub const STREAMER_SUGGEST_CHANNEL_ID: u64 = 1_425_215_762_460_835_931;

/// Block für die Server-Tour. Kurz gehalten, die Details liefern die Antworten unten.
pub const TOUR_COMMUNITY_BLOCK: &str = "**Community und Streamer**\nVerknüpf dein Discord mit Twitch, dann sammelst du Punkte, wenn du bei unseren Partner-Streamern zuschaust und mitchattest. Die Punkte landen zusammen mit deinen Voice-Punkten in einem gemeinsamen Leaderboard. Jede Woche gibt es einen Clip-Contest, die Community stimmt ab und die Top 3 laufen im Stream auf dach_lock. Und wenn du einen Streamer kennst, der zu uns passt, kannst du ihn für unser Streamer-Programm vorschlagen. Fragen dazu? Schreib mir einfach, zum Beispiel \"Wie bekomme ich Punkte?\".";

/// Antwort nach Klick auf "Twitch verknüpfen", wenn ein persönlicher Link erzeugt wurde.
pub const TWITCH_LINK_READY_TEXT: &str = "Hier ist dein Link. Discord fragt dich kurz, ob wir deine verknüpften Konten sehen dürfen, danach bist du fertig. Wenn Twitch in deinen Discord-Einstellungen unter Verbindungen noch fehlt, füg es dort zuerst hinzu und klick dann nochmal.";

/// Antwort nach Klick auf "Twitch verknüpfen", wenn gerade kein Link erzeugt werden kann.
pub const TWITCH_LINK_FALLBACK_TEXT: &str = "Den Link kann ich dir hier gerade nicht direkt geben. Geh in <#1398021105339334666> und klick dort auf \"Twitch verknüpfen\", das ist derselbe Weg. Falls Twitch in deinen Discord-Einstellungen unter Verbindungen noch fehlt, füg es dort zuerst hinzu.";

pub const ANSWER_PUNKTE: &str = "Punkte bekommst du auf drei Wegen, alles landet in einem gemeinsamen Leaderboard.\n\n**Voice:** Wie bisher sammelst du Punkte, wenn du mit anderen im Voice aktiv bist. Mit `!vstats` siehst du deinen Stand.\n\n**Zuschauen bei Partner-Streamern:** Dafür verknüpfst du dein Discord mit Twitch. Dann gibt es 1 Punkt für alle vollen 5 Minuten, die du bei einem Partner zuschaust, während er live ist. Pro Kanal zählen höchstens 72 Punkte am Tag (6 Stunden), über alle Kanäle höchstens 144 Punkte am Tag.\n\n**Mitchatten:** 1 Punkt pro echter Nachricht im Chat eines Partners. Sie zählt, wenn sie mindestens 10 Zeichen hat, kein Befehl mit ! ist, nicht dasselbe wie deine letzte Nachricht ist und seit deiner letzten zählenden Nachricht mindestens eine Minute vergangen ist. Pro Kanal höchstens 30 Punkte am Tag.\n\n**Entdecken:** Schaust du zum ersten Mal bei einem Partner rein, bei dem du noch nie warst, gibt es einmalig 10 Punkte extra, höchstens 3 mal am Tag.\n\nDazu kommen Punkte aus dem Clip-Contest und für erfolgreiche Streamer-Vorschläge. Der Tag zählt nach deutscher Zeit. Follows und Subs bringen keine Punkte, und Punkte sind Anerkennung aus der Community, kein Geld und nichts zum Eintauschen. Twitch-Punkte werden dir erst nach der Verknüpfung zugerechnet. Mit Voice-Punkten, Clip-Contest-Punkten oder Punkten für Streamer-Vorschläge kannst du auch ohne Twitch-Verknüpfung im gemeinsamen Leaderboard erscheinen.";

pub const ANSWER_TWITCH_VERKNUEPFEN: &str = "Das geht in einer Minute. Klick unten auf \"Twitch verknüpfen\" oder in <#1398021105339334666> auf den gleichnamigen Knopf. Discord fragt dich dann, ob wir deine verknüpften Konten sehen dürfen, wir lesen daraus nur dein Twitch-Konto. Wichtig: Twitch muss in deinen Discord-Einstellungen unter Verbindungen hinzugefügt sein, sonst gibt es nichts zu finden. Danach zählen deine Punkte fürs Zuschauen und Mitchatten bei unseren Partner-Streamern für dich, und du tauchst im gemeinsamen Leaderboard auf. Ohne Verknüpfung werden dir keine Twitch-Punkte zugerechnet. Voice-Punkte, Clip-Contest-Punkte und Punkte für Streamer-Vorschläge können dich trotzdem im gemeinsamen Leaderboard zeigen.";

pub const ANSWER_CLIP_CONTEST: &str = "Jede Woche läuft hier ein Clip-Contest. Clips aus den Streams unserer Partner werden eingereicht, und die Community stimmt in Discord ab. Die Top 3 der Woche laufen im Stream auf dach_lock.\n\nPunkte gibt es auch: Platz 1 bringt 100, Platz 2 bringt 60 und Platz 3 bringt 40 Punkte für die Person, die den Clip eingereicht hat. Wer abstimmt, bekommt 2 Punkte, dabei zählt eine Stimme pro Woche. Alles landet im gemeinsamen Leaderboard.";

pub const ANSWER_STREAMER_VORSCHLAGEN: &str = "Du kennst jemanden, der Deadlock streamt und gut zu uns passt? Dann geh in <#1425215762460835931> und drück dort auf den Knopf \"Streamer vorschlagen\". Gib den Twitch-Kanal oder den Link an und schreib kurz, warum. Unser Team schaut sich jeden Vorschlag an und entscheidet selbst, wen es anspricht. Wird der Kanal Partner in unserem Streamer-Programm, bekommt die Person, die ihn als Erste vorgeschlagen hat, 150 Punkte. Du kannst bis zu 3 Kanäle am Tag vorschlagen.";

pub const ANSWER_STREAMER_VORTEILE: &str = "Als Partner-Streamer bekommst du Zuschauer aus einer Community, die gezielt bei Partnern reinschaut. Wer bei dir zum ersten Mal vorbeikommt, bekommt sogar einen Bonus fürs Entdecken.\n\nDu sammelst auch selbst Punkte für das Streamer-Leaderboard: 1 Punkt für alle 30 Minuten, die verknüpfte Community-Mitglieder bei dir zuschauen, 25 Punkte für jeden Raid an einen anderen Partner und 50 Punkte für jede Person, die über deine Einladung auf den Server kommt und hier wirklich aktiv wird. Im wöchentlichen Clip-Contest gibt es 100, 60 oder 40 Punkte für die Plätze 1 bis 3, und die Top 3 laufen im Stream auf dach_lock.\n\nPunkte sind Anerkennung aus der Community, kein Geld. Follows und Subs werden nicht belohnt. Wenn du Partner werden willst, lass dich über \"Streamer vorschlagen\" in <#1425215762460835931> vorschlagen oder frag in <#1491953161747955853>.";

/// Themen, die der Concierge ohne Sprachmodell und ohne Wissensdienst beantwortet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommunityTopic {
    Punkte,
    TwitchVerknuepfen,
    ClipContest,
    StreamerVorschlagen,
    StreamerVorteile,
}

impl CommunityTopic {
    pub const fn answer(self) -> &'static str {
        match self {
            Self::Punkte => ANSWER_PUNKTE,
            Self::TwitchVerknuepfen => ANSWER_TWITCH_VERKNUEPFEN,
            Self::ClipContest => ANSWER_CLIP_CONTEST,
            Self::StreamerVorschlagen => ANSWER_STREAMER_VORSCHLAGEN,
            Self::StreamerVorteile => ANSWER_STREAMER_VORTEILE,
        }
    }

    /// Antworten, unter denen der Concierge den Button "Twitch verknüpfen" anbietet.
    pub const fn offers_twitch_link(self) -> bool {
        matches!(self, Self::Punkte | Self::TwitchVerknuepfen)
    }

    pub const fn log_key(self) -> &'static str {
        match self {
            Self::Punkte => "punkte",
            Self::TwitchVerknuepfen => "twitch_verknuepfen",
            Self::ClipContest => "clip_contest",
            Self::StreamerVorschlagen => "streamer_vorschlagen",
            Self::StreamerVorteile => "streamer_vorteile",
        }
    }
}

const QUESTION_STARTERS: [&str; 24] = [
    "wie", "was", "wo", "wann", "warum", "wieso", "weshalb", "wozu", "wofuer", "kann", "koennte",
    "gibt", "gibts", "bekomme", "bekommt", "krieg", "kriege", "kriegt", "muss", "darf", "lohnt",
    "erklaer", "erklaere", "zeig",
];

/// Wörter, bei denen "Punkte" oder "Rangliste" das Spiel meinen und nicht die Community.
const GAME_CONTEXT_WORDS: [&str; 9] = [
    "rang",
    "rank",
    "ranked",
    "elo",
    "mmr",
    "seelen",
    "souls",
    "matchmaking",
    "item",
];

fn fold(text: &str) -> String {
    text.to_lowercase()
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('ß', "ss")
}

fn words(folded: &str) -> Vec<&str> {
    folded
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect()
}

fn looks_like_question(folded: &str, words: &[&str]) -> bool {
    folded.contains('?')
        || words
            .first()
            .is_some_and(|first| QUESTION_STARTERS.contains(first))
}

fn any_word_starts(words: &[&str], prefixes: &[&str]) -> bool {
    words
        .iter()
        .any(|word| prefixes.iter().any(|prefix| word.starts_with(prefix)))
}

/// Erkennt eng umrissene Fragen zu Punkten, Twitch-Verknüpfung, Clip-Contest,
/// Streamer-Vorschlag und Streamer-Vorteilen. Bewusst konservativ: Was hier nicht
/// greift, geht wie bisher an den Wissensdienst.
pub fn community_question(text: &str) -> Option<CommunityTopic> {
    let folded = fold(text.trim());
    let words = words(&folded);
    if words.is_empty() || words.len() > 30 || !looks_like_question(&folded, &words) {
        return None;
    }
    let has = |prefixes: &[&str]| any_word_starts(&words, prefixes);
    let mentions_twitch = has(&["twitch"]);
    let mentions_streamer = has(&["streamer", "streamerin", "partnerprogramm"]);
    let mentions_clip = has(&["clip"]);
    // Einrichtungsfragen von Streamern (Dashboard, Bot, Overlay) gehören zum Wissensdienst.
    if has(&["dashboard", "overlay", "obs", "bot"]) {
        return None;
    }

    if mentions_clip
        && has(&[
            "contest",
            "wettbewerb",
            "voting",
            "abstimm",
            "einreich",
            "reich",
            "top",
        ])
    {
        return Some(CommunityTopic::ClipContest);
    }
    if (mentions_streamer || mentions_twitch || has(&["kanal"]))
        && (has(&["vorschlag", "vorschlaeg", "empfehl", "nominier"])
            || (has(&["schlag"]) && words.contains(&"vor")))
    {
        return Some(CommunityTopic::StreamerVorschlagen);
    }
    if mentions_streamer
        && (has(&[
            "bringt",
            "bringen",
            "vorteil",
            "lohnt",
            "nutzen",
            "profitier",
        ]) || folded.contains("partner werden"))
    {
        return Some(CommunityTopic::StreamerVorteile);
    }
    if mentions_twitch && has(&["verknuepf", "verbind", "verlink", "link", "koppel"]) {
        return Some(CommunityTopic::TwitchVerknuepfen);
    }
    let about_points = has(&["punkt", "leaderboard", "rangliste", "bestenliste"]);
    let game_context = words.iter().any(|word| GAME_CONTEXT_WORDS.contains(word));
    if about_points && !game_context {
        return Some(CommunityTopic::Punkte);
    }
    None
}

/// Button "Twitch verknüpfen" im Concierge-Stil (sekundär, eigener Klick nötig).
pub fn twitch_link_button() -> Value {
    json!({
        "type": 2,
        "style": 2,
        "label": TWITCH_LINK_BUTTON_LABEL,
        "custom_id": TWITCH_LINK_CUSTOM_ID,
    })
}

/// Link-Button mit dem persönlichen Verknüpfungslink.
pub fn twitch_link_url_button(url: &str) -> Value {
    json!({ "type": 2, "style": 5, "label": TWITCH_LINK_OPEN_LABEL, "url": url })
}

/// Liefert den persönlichen Discord-Link, über den ein Mitglied Twitch verknüpft
/// (Scope `identify connections`, delegierter Flow, Callback `/callback/discord`).
///
/// Die Implementierung kommt aus Paket A. `None` heißt: gerade kein Link, der
/// Concierge verweist dann auf den Knopf im Verify-Kanal.
#[async_trait]
pub trait TwitchLinkSource: Send + Sync {
    async fn twitch_link_url(&self, discord_user_id: u64) -> Option<String>;
}

/// Standardquelle, solange kein Link-Erzeuger angeschlossen ist.
pub struct VerifyChannelOnly;

#[async_trait]
impl TwitchLinkSource for VerifyChannelOnly {
    async fn twitch_link_url(&self, _discord_user_id: u64) -> Option<String> {
        None
    }
}

/// Nur Links, die wirklich zum Discord-Login führen, landen im Button.
pub fn usable_link(url: &str) -> Option<&str> {
    let url = url.trim();
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let discord_host = host == "discord.com" || host.ends_with(".discord.com");
    (parsed.scheme() == "https" && discord_host).then_some(url)
}

/// Antworttext und Zusatzbuttons nach Klick auf "Twitch verknüpfen".
pub fn twitch_link_reply(url: Option<&str>) -> (&'static str, Vec<Value>) {
    match url.and_then(usable_link) {
        Some(url) => (TWITCH_LINK_READY_TEXT, vec![twitch_link_url_button(url)]),
        None => (TWITCH_LINK_FALLBACK_TEXT, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TEXTS: [&str; 8] = [
        TOUR_COMMUNITY_BLOCK,
        TWITCH_LINK_READY_TEXT,
        TWITCH_LINK_FALLBACK_TEXT,
        ANSWER_PUNKTE,
        ANSWER_TWITCH_VERKNUEPFEN,
        ANSWER_CLIP_CONTEST,
        ANSWER_STREAMER_VORSCHLAGEN,
        ANSWER_STREAMER_VORTEILE,
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
        assert!(ANSWER_PUNKTE.contains("für"));
    }

    #[test]
    fn punkteantwort_nennt_regeln_und_deckel_aus_dem_plan() {
        for fakt in [
            "vollen 5 Minuten",
            "72 Punkte",
            "144 Punkte",
            "mindestens 10 Zeichen",
            "kein Befehl mit !",
            "mindestens eine Minute",
            "30 Punkte",
            "10 Punkte extra",
            "3 mal am Tag",
            "kein Geld",
            "Follows und Subs bringen keine Punkte",
            "auch ohne Twitch-Verknüpfung",
        ] {
            assert!(ANSWER_PUNKTE.contains(fakt), "fehlt: {fakt}");
        }
        for fakt in [
            "100",
            "60",
            "40",
            "2 Punkte",
            "eine Stimme pro Woche",
            "dach_lock",
        ] {
            assert!(ANSWER_CLIP_CONTEST.contains(fakt), "fehlt: {fakt}");
        }
        assert!(ANSWER_STREAMER_VORSCHLAGEN.contains("150 Punkte"));
        assert!(ANSWER_STREAMER_VORSCHLAGEN.contains(&format!("<#{STREAMER_SUGGEST_CHANNEL_ID}>")));
        assert!(ANSWER_STREAMER_VORSCHLAGEN.contains("3 Kanäle am Tag"));
        assert!(ANSWER_STREAMER_VORTEILE.contains(&format!("<#{STREAMER_SUGGEST_CHANNEL_ID}>")));
        for fakt in ["30 Minuten", "25 Punkte", "50 Punkte", "kein Geld"] {
            assert!(ANSWER_STREAMER_VORTEILE.contains(fakt), "fehlt: {fakt}");
        }
        assert!(ANSWER_TWITCH_VERKNUEPFEN.contains("<#1398021105339334666>"));
        assert!(TWITCH_LINK_FALLBACK_TEXT.contains("<#1398021105339334666>"));
    }

    #[test]
    fn fragen_aus_dem_auftrag_werden_deterministisch_erkannt() {
        let cases = [
            ("Wie bekomme ich Punkte?", CommunityTopic::Punkte),
            ("wie krieg ich punkte", CommunityTopic::Punkte),
            ("Wie funktioniert das Leaderboard?", CommunityTopic::Punkte),
            (
                "Wie verknüpfe ich Twitch?",
                CommunityTopic::TwitchVerknuepfen,
            ),
            (
                "wie verknuepfe ich mein twitch konto",
                CommunityTopic::TwitchVerknuepfen,
            ),
            (
                "Wie funktioniert der Clip-Contest?",
                CommunityTopic::ClipContest,
            ),
            ("Wo reiche ich einen Clip ein?", CommunityTopic::ClipContest),
            (
                "Wie schlage ich einen Streamer vor?",
                CommunityTopic::StreamerVorschlagen,
            ),
            (
                "Kann ich einen Twitch-Kanal vorschlagen?",
                CommunityTopic::StreamerVorschlagen,
            ),
            (
                "Was bringt mir das als Streamer?",
                CommunityTopic::StreamerVorteile,
            ),
            (
                "Wie kann ich Partner werden als Streamer?",
                CommunityTopic::StreamerVorteile,
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(community_question(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn fremde_fragen_und_aussagen_bleiben_beim_wissensdienst() {
        for text in [
            "Wie verknüpfe ich meinen Steam-Account?",
            "Wie viele Punkte brauche ich für den nächsten Rang?",
            "Wie bekomme ich mehr Seelen?",
            "Ich schaue gern Streamer auf Twitch.",
            "Wie funktioniert der Steam Bot?",
            "Wo finde ich Mitspieler?",
            "Wie verknüpfe ich Twitch mit dem Streamer-Dashboard?",
            "Was bringt mir der Twitch-Bot als Streamer?",
            "Ich will direkt spielen",
            "",
        ] {
            assert_eq!(community_question(text), None, "{text}");
        }
    }

    #[test]
    fn nur_twitch_relevante_antworten_bieten_den_button_an() {
        assert!(CommunityTopic::Punkte.offers_twitch_link());
        assert!(CommunityTopic::TwitchVerknuepfen.offers_twitch_link());
        assert!(!CommunityTopic::ClipContest.offers_twitch_link());
        assert!(!CommunityTopic::StreamerVorschlagen.offers_twitch_link());
        assert!(!CommunityTopic::StreamerVorteile.offers_twitch_link());
        let button = twitch_link_button();
        assert_eq!(button["custom_id"], TWITCH_LINK_CUSTOM_ID);
        assert_eq!(button["label"], TWITCH_LINK_BUTTON_LABEL);
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
