//! LFG-Kernlogik — Port der puren Teile von `cogs/lfg.py`.
//!
//! Enthält die Intent-Heuristik (auf 500 echten LFG-Nachrichten kalibriert),
//! die Match-Scores (Rang-Nähe, Zeit-Übereinstimmung) und das
//! Tag-Filter-Parsing. Der Discord-Flow (Routing-Antworten, Empfehlungen,
//! AI-Zweitprüfung via dl-ai) folgt mit Phase 6.

const RANK_TOKENS: [&str; 35] = [
    // RANK_NAME_TO_VALUE-Schlüssel
    "obscurus",
    "initiate",
    "seeker",
    "alchemist",
    "arcanist",
    "ritualist",
    "emissary",
    "archon",
    "oracle",
    "phantom",
    "ascendant",
    "eternus",
    // SHORT_NAME_TO_RANK-Schlüssel
    "ini",
    "see",
    "alc",
    "arc",
    "rit",
    "emi",
    "arch",
    "ora",
    "pha",
    "asc",
    "ete",
    // MESSAGE_RANK_ALIASES-Schlüssel
    "seek",
    "alch",
    "emiss",
    "et",
    "arkanist",
    "ascendent",
    "ethernus",
    // Auffüllung auf Originalumfang (Aliasse mit identischen Kurzformen)
    "ini",
    "arc",
    "rit",
    "emi",
    "arch",
];

fn contains_any(text: &str, words: &[&str]) -> bool {
    words.iter().any(|w| text.contains(w))
}

/// `"suche +2"`, `"lfm+3"`, `"+4"` — SHORT_LFG_COUNT_RE.
fn is_short_lfg_count(text: &str) -> bool {
    let t = text.trim();
    let rest = ["suche", "suchen", "lfm", "lfg"]
        .iter()
        .find_map(|kw| t.strip_prefix(kw))
        .unwrap_or(t);
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('+').unwrap_or(rest).trim_start();
    let mut chars = rest.chars();
    matches!(chars.next(), Some(c) if ('1'..='6').contains(&c)) && chars.as_str().trim().is_empty()
}

/// `"+N"` irgendwo im Text (PLUS_PLAYER_RE: kein weiterer Digit danach).
fn has_plus_player(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'+' {
            continue;
        }
        let rest = text[i + 1..].trim_start();
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            if ('1'..='6').contains(&c) {
                match chars.next() {
                    Some(next) if next.is_ascii_digit() => continue,
                    _ => return true,
                }
            }
        }
    }
    false
}

/// Primäre LFG-Erkennung (Referenzfälle aus CPython im Test).
pub fn keyword_lfg_intent(message: &str) -> bool {
    let text = message.to_lowercase();
    if text.is_empty() {
        return false;
    }
    // Negativ: private Kontaktaufnahme
    if (text.contains("schreib") || text.contains("schreibt") || text.contains("meldet"))
        && (text.contains("priv") || text.contains("privat"))
    {
        return false;
    }
    // Rang + Spielwunsch
    if contains_any(&text, &RANK_TOKENS)
        && (text.contains("bock") || text.contains("lust"))
        && contains_any(
            &text,
            &[
                "runde",
                "runden",
                "ründchen",
                "rundchen",
                "game",
                "games",
                "match",
                "matches",
                "spielen",
                "zocken",
                "grinden",
                "gamen",
            ],
        )
    {
        return true;
    }
    if is_short_lfg_count(&text) {
        return true;
    }
    if text.contains("lfg") || text.contains("lfm") {
        return true;
    }
    if contains_any(&text, &["duo", "trio", "squad", "stack"]) {
        return true;
    }
    if text.contains("bock")
        && contains_any(
            &text,
            &[
                "jemand",
                "jmd",
                "wer",
                "iwer",
                "irgendwer",
                "noch",
                "hat",
                "hätte",
                "hättest",
            ],
        )
    {
        return true;
    }
    if text.contains("lust")
        && contains_any(
            &text,
            &["jemand", "jmd", "wer", "iwer", "irgendwer", "hat", "noch"],
        )
    {
        return true;
    }
    if text.contains("suche") || text.contains("suchen") || text.contains("gesucht") {
        if has_plus_player(&text) {
            return true;
        }
        if contains_any(
            &text,
            &[
                "leute",
                "spieler",
                "mitspieler",
                "team",
                "gruppe",
                "party",
                "wen",
                "anschluss",
                "jemand",
                "noch",
                "nach",
                "mates",
                "mate",
            ],
        ) {
            return true;
        }
    }
    if text.contains("sucht") && text.contains("jemand") {
        return true;
    }
    if contains_any(&text, &["spielen", "zocken", "grinden", "gamen"])
        && contains_any(
            &text,
            &["wer", "jemand", "bock", "jmd", "iwer", "irgendwer"],
        )
    {
        return true;
    }
    if contains_any(&text, &["paar runden", "paar games", "paar rounds"]) {
        return true;
    }
    if text.contains("down") && contains_any(&text, &["jemand", "wer", "iwer", "irgendwer"]) {
        return true;
    }
    if text.contains("mag wer") {
        return true;
    }
    if text.contains("möchte") && (text.contains("jemand") || text.contains("wer")) {
        return true;
    }
    if text.contains("am start") {
        return true;
    }
    if text.contains("wach") && contains_any(&text, &["jemand", "jmd", "wer", "iwer", "irgendwer"])
    {
        return true;
    }
    if contains_any(
        &text,
        &["jemand on", "jmd on", "wer on", "iwer on", "irgendwer on"],
    ) {
        return true;
    }
    if text.contains("neuling")
        && text.contains("platz")
        && (text.contains("jmd") || text.contains("jemand"))
    {
        return true;
    }
    if text.contains("interesse")
        && contains_any(
            &text,
            &[
                "jemand",
                "wer",
                "jmd",
                "iwer",
                "irgendwer",
                "anderer",
                "andere",
                "noch",
                "hat",
                "hätte",
            ],
        )
        && contains_any(
            &text,
            &[
                "spielen",
                "zocken",
                "grinden",
                "gamen",
                "runde",
                "runden",
                "game",
                "games",
                "match",
                "matches",
                "anfänger",
                "anfanger",
                "neuling",
                "neu",
            ],
        )
    {
        return true;
    }
    if text.contains("hmu") {
        return true;
    }
    if text.contains("anyone") && contains_any(&text, &["wanna", "down", "game"]) {
        return true;
    }
    if text.contains("auf der suche") {
        return true;
    }
    false
}

/// Rang-Nähe-Score (Subrang-Punkte auf 10er-Basis, wie das Original).
pub fn rank_score(
    target_rank: i64,
    target_sub: Option<i64>,
    cand_rank: i64,
    cand_sub: Option<i64>,
) -> f64 {
    let t_sub = target_sub.unwrap_or(5);
    let c_sub = cand_sub.unwrap_or(5);
    let diff = ((target_rank * 10 + t_sub) - (cand_rank * 10 + c_sub)).abs();
    if diff <= 5 {
        1.0
    } else if diff <= 10 {
        0.7
    } else if diff <= 20 {
        0.4
    } else {
        0.0
    }
}

/// Zeit-Übereinstimmung mit typischen Stunden/Tagen (±2h, Mitternacht-Wrap).
pub fn time_match_score(
    typical_hours: &[i64],
    typical_days: &[i64],
    now_hour: i64,
    now_weekday: i64,
) -> f64 {
    if typical_hours.is_empty() && typical_days.is_empty() {
        return 0.0;
    }
    let hour_match = typical_hours.iter().any(|h| {
        let diff = (now_hour - h).abs();
        diff <= 2 || diff >= 22
    });
    let day_match = typical_days.is_empty() || typical_days.contains(&now_weekday);
    if hour_match && day_match {
        1.0
    } else if hour_match {
        0.7
    } else if day_match {
        0.4
    } else {
        0.0
    }
}

/// Tag-Filter aus der Nachricht: `25+` und `ragebaiter-free`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagFilters {
    pub min_age_25: bool,
    pub ragebaiter_free: bool,
}

pub fn parse_tag_filters(message: &str) -> TagFilters {
    let lower = message.to_lowercase();
    // (?<!\d)25\+(?!\d)
    let mut min_age_25 = false;
    let bytes = lower.as_bytes();
    let mut i = 0;
    while let Some(offset) = lower[i..].find("25+") {
        let start = i + offset;
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_digit();
        let after = start + 3;
        let after_ok = after >= bytes.len() || !bytes[after].is_ascii_digit();
        if before_ok && after_ok {
            min_age_25 = true;
            break;
        }
        i = start + 1;
    }
    let ragebaiter_free = ["ragebaiter free", "ragebaiter-free", "ragebaiterfree"]
        .iter()
        .any(|p| lower.contains(p));
    TagFilters {
        min_age_25,
        ragebaiter_free,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_wie_python() {
        // Referenzfälle aus dem laufenden CPython-Original
        for positive in [
            "lfg ranked",
            "suche +2",
            "+3",
            "jemand bock auf deadlock?",
            "wer lust auf paar runden",
            "phantom bock auf ründchen",
            "duo abend?",
            "wer on",
            "suche leute zum zocken",
            "mag wer",
            "bin am start",
            "jemand wach?",
        ] {
            assert!(keyword_lfg_intent(positive), "sollte LFG sein: {positive}");
        }
        for negative in [
            "schreibt mir privat",
            "hallo zusammen",
            "gg wp",
            "englisch please",
            "such nach team", // "such" ohne e matcht nicht (wie Python)
            "",
        ] {
            assert!(
                !keyword_lfg_intent(negative),
                "sollte KEIN LFG sein: {negative}"
            );
        }
    }

    #[test]
    fn rank_scores() {
        assert_eq!(rank_score(9, Some(3), 9, Some(5)), 1.0); // 2 Punkte
        assert_eq!(rank_score(9, None, 8, None), 0.7); // 10 Punkte
        assert_eq!(rank_score(9, Some(1), 7, Some(5)), 0.4); // 16 Punkte
        assert_eq!(rank_score(11, Some(6), 1, Some(1)), 0.0);
    }

    #[test]
    fn time_scores() {
        // Stunde passt (22 ↔ 0 über Mitternacht) + Tag passt
        assert_eq!(time_match_score(&[23], &[4], 1, 4), 1.0);
        // nur Stunde
        assert_eq!(time_match_score(&[20], &[0], 21, 5), 0.7);
        // nur Tag
        assert_eq!(time_match_score(&[8], &[5], 14, 5), 0.4);
        // nichts
        assert_eq!(time_match_score(&[8], &[0], 14, 5), 0.0);
        assert_eq!(time_match_score(&[], &[], 14, 5), 0.0);
    }

    #[test]
    fn tag_filter_parsing() {
        assert_eq!(
            parse_tag_filters("suche +2, 25+ ragebaiter-free"),
            TagFilters {
                min_age_25: true,
                ragebaiter_free: true
            }
        );
        // 125+ und 25+7 zählen nicht (Digit-Grenzen)
        assert_eq!(parse_tag_filters("125+ hp build"), TagFilters::default());
        assert_eq!(parse_tag_filters("25+7 = 32"), TagFilters::default());
        assert!(parse_tag_filters("nur 25+!").min_age_25);
    }
}
