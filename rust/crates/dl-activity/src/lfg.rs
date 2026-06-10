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

// ── Lane-Routing (wie _route_to_lane; Referenz-Entscheidungen im Test) ─────

pub const LANE_RANK_TOLERANCE_RANKED: f64 = 2.0;
pub const LANE_RANK_TOLERANCE_CASUAL: f64 = 3.0;
pub const NEW_PLAYER_MAX_RANK: i64 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneLabel {
    Casual,
    Ranked,
    StreetBrawl,
    NewPlayer,
}

#[derive(Debug, Clone)]
pub struct LaneInfo {
    pub channel_id: u64,
    pub label: LaneLabel,
    pub member_count: usize,
    pub user_limit: usize,
    pub avg_rank_value: f64,
    /// Anzahl bekannter Mitspieler des Suchenden in dieser Lane.
    pub co_players_present: usize,
    /// Anzeige-Felder für den Antwort-Embed.
    pub name: String,
    pub avg_rank_label: String,
    pub category_id: u64,
    pub position: i64,
    pub is_staging: bool,
    /// Namen der anwesenden bekannten Mitspieler (max. 3 genutzt).
    pub co_player_names: Vec<String>,
}

impl LaneInfo {
    fn has_space(&self) -> bool {
        self.member_count < self.user_limit
    }
    fn slots_free(&self) -> usize {
        self.user_limit.saturating_sub(self.member_count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteMode {
    /// Lane mit bekannten Mitspielern.
    CoPlayerLane,
    /// Vollste passende besetzte Lane.
    JoinExisting,
    /// Leere passende Lane bzw. neue Lane im vorgeschlagenen Modus.
    CreateNew,
}

#[derive(Debug, Clone)]
pub struct RouteResult {
    pub mode: RouteMode,
    pub target_channel_id: Option<u64>,
    pub suggested_label: Option<LaneLabel>,
}

/// Intent aus Keywords (ranked nur ab Emissary/Rang 6 — wie das Original).
pub fn detect_intent(content_lower: &str, rank_value: i64) -> (bool, bool) {
    let ranked = ["ranked", "grind", "comp", "competitive", "tryhard"]
        .iter()
        .any(|kw| content_lower.contains(kw));
    let street_brawl = ["street brawl", "streetbrawl", "brawl"]
        .iter()
        .any(|kw| content_lower.contains(kw));
    let ranked = ranked && rank_value >= 6;
    (ranked, street_brawl)
}

/// Rang-Fit je Lane-Typ (wie _rank_fits_lane; Subrank-Default 5).
pub fn rank_fits_lane(requester_rank: i64, requester_sub: Option<i64>, lane: &LaneInfo) -> bool {
    if lane.member_count == 0 {
        return true;
    }
    let req_val = requester_rank as f64 + requester_sub.unwrap_or(5) as f64 / 10.0;
    match lane.label {
        LaneLabel::Ranked => (req_val - lane.avg_rank_value).abs() <= LANE_RANK_TOLERANCE_RANKED,
        LaneLabel::Casual | LaneLabel::StreetBrawl => {
            (req_val - lane.avg_rank_value).abs() <= LANE_RANK_TOLERANCE_CASUAL
        }
        LaneLabel::NewPlayer => requester_rank <= NEW_PLAYER_MAX_RANK || requester_rank == 0,
    }
}

/// Routing-Entscheidung wie `_route_to_lane`.
pub fn route_to_lane(
    content_lower: &str,
    rank_value: i64,
    rank_sub: Option<i64>,
    lanes: &[LaneInfo],
) -> RouteResult {
    let (ranked_intent, sb_intent) = detect_intent(content_lower, rank_value);
    let eligible: Vec<&LaneInfo> = if sb_intent {
        lanes
            .iter()
            .filter(|l| l.label == LaneLabel::StreetBrawl && l.has_space())
            .collect()
    } else if ranked_intent {
        lanes
            .iter()
            .filter(|l| {
                l.label == LaneLabel::Ranked
                    && l.has_space()
                    && rank_fits_lane(rank_value, rank_sub, l)
            })
            .collect()
    } else if rank_value > 0 && rank_value <= NEW_PLAYER_MAX_RANK {
        // Anfänger: primär New-Player-Lanes, Fallback Casual+NP
        let np: Vec<&LaneInfo> = lanes
            .iter()
            .filter(|l| {
                l.label == LaneLabel::NewPlayer
                    && l.has_space()
                    && rank_fits_lane(rank_value, rank_sub, l)
            })
            .collect();
        if np.is_empty() {
            lanes
                .iter()
                .filter(|l| {
                    matches!(l.label, LaneLabel::Casual | LaneLabel::NewPlayer)
                        && l.has_space()
                        && rank_fits_lane(rank_value, rank_sub, l)
                })
                .collect()
        } else {
            np
        }
    } else {
        lanes
            .iter()
            .filter(|l| {
                matches!(l.label, LaneLabel::Casual | LaneLabel::NewPlayer)
                    && l.has_space()
                    && rank_fits_lane(rank_value, rank_sub, l)
            })
            .collect()
    };

    // Co-Spieler-Lane gewinnt (meiste Co-Spieler, dann meiste Mitglieder)
    let co_lanes: Vec<&&LaneInfo> = eligible
        .iter()
        .filter(|l| l.co_players_present > 0)
        .collect();
    if let Some(best) = co_lanes
        .iter()
        .max_by_key(|l| (l.co_players_present, l.member_count))
    {
        return RouteResult {
            mode: RouteMode::CoPlayerLane,
            target_channel_id: Some(best.channel_id),
            suggested_label: None,
        };
    }
    let occupied: Vec<&&LaneInfo> = eligible.iter().filter(|l| l.member_count > 0).collect();
    if let Some(best) = occupied.iter().max_by_key(|l| l.member_count) {
        return RouteResult {
            mode: RouteMode::JoinExisting,
            target_channel_id: Some(best.channel_id),
            suggested_label: None,
        };
    }
    if let Some(first) = eligible.first() {
        return RouteResult {
            mode: RouteMode::CreateNew,
            target_channel_id: Some(first.channel_id),
            suggested_label: None,
        };
    }
    // Nichts passt → neue Lane im vorgeschlagenen Modus
    let label = if sb_intent {
        LaneLabel::StreetBrawl
    } else if ranked_intent {
        LaneLabel::Ranked
    } else if rank_value > 0 && rank_value <= NEW_PLAYER_MAX_RANK {
        LaneLabel::NewPlayer
    } else {
        LaneLabel::Casual
    };
    RouteResult {
        mode: RouteMode::CreateNew,
        target_channel_id: None,
        suggested_label: Some(label),
    }
}

// ── Antwort-Schicht (wie _handle_lfg_request; Embed als JSON) ──────────────

pub const LFG_CHANNEL_ID: u64 = 1376335502919335936;
pub const OUTPUT_CHANNEL_ID: u64 = 1376335502919335936;
pub const LFG_LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const NEW_PLAYER_LANE_ID: u64 = 1470126503252721845;
pub const COACH_REQUEST_CHANNEL_ID: u64 = 1494373349944459355;
pub const STAGING_CASUAL_ID: u64 = 1501089974093873232;
pub const STAGING_RANKED_ID: u64 = 1412804671432818890;
pub const STAGING_STREET_BRAWL_ID: u64 = 1357422958544420944;
pub const MAX_JOIN_LOBBIES_SHOWN: usize = 3;
pub const RANK_WARNING_DIFF: f64 = 1.5;
pub const LOBBY_MAYBE_FULL_THRESHOLD: usize = 6;

/// Anfänger-Anfrage (wie _is_new_player_request + _detect_new_player_text).
pub fn is_new_player_request(content_lower: &str, rank_value: i64, has_rank_role: bool) -> bool {
    if rank_value > 0 && rank_value <= NEW_PLAYER_MAX_RANK {
        return true;
    }
    if rank_value > NEW_PLAYER_MAX_RANK || has_rank_role {
        return false;
    }
    let normalized: String = content_lower
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    [
        "neuling",
        "neuer spieler",
        "bin neu",
        "neu im spiel",
        "anfänger",
        "anfanger",
        "noch nicht so gut",
        "mit einem neuling",
        "mit nem neuling",
        "mit 'nem neuling",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase))
}

/// Lobby-Bewertung für die Top-3 (wie _score_lobby_suggestion).
pub fn score_lobby_suggestion(
    lane: &LaneInfo,
    route: &RouteResult,
    rank_value: i64,
    rank_sub: Option<i64>,
    is_new_player: bool,
    has_explicit_rank: bool,
) -> f64 {
    let mut score = 0.0;
    if route.target_channel_id == Some(lane.channel_id) {
        score += 1000.0;
    }
    if lane.co_players_present > 0 {
        score += 250.0 + lane.co_players_present as f64 * 25.0;
    }
    if lane.member_count > 0 {
        score += 200.0 + lane.member_count as f64 * 15.0;
    }
    if is_new_player && lane.label == LaneLabel::NewPlayer {
        score += 1200.0;
    }
    if rank_value > 0 && lane.avg_rank_value > 0.0 {
        let rank_diff =
            (lane.avg_rank_value - (rank_value as f64 + rank_sub.unwrap_or(5) as f64 / 10.0)).abs();
        if rank_diff > 3.0 {
            score -= 500.0;
        } else if has_explicit_rank {
            score += (140.0 - rank_diff * 35.0).max(0.0);
        } else {
            score += (80.0 - rank_diff * 20.0).max(0.0);
        }
    }
    if has_explicit_rank && lane.label == LaneLabel::Ranked {
        score += 50.0;
    }
    score
}

/// Bis zu drei Lobby-Vorschläge (wie _select_lobby_suggestions): Kandidaten
/// filtern, nach Score sortieren; mit Rang gewinnt EINE eng passende Lane
/// (Toleranz 1,5 mit Subrang, sonst 2,0).
pub fn select_lobby_suggestions(
    lanes: &[LaneInfo],
    route: &RouteResult,
    rank_value: i64,
    rank_sub: Option<i64>,
    is_new_player: bool,
    has_explicit_rank: bool,
) -> Vec<LaneInfo> {
    let use_rank_filtering = has_explicit_rank || (is_new_player && rank_value > 0);
    let mut candidates: Vec<&LaneInfo> = Vec::new();
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for lane in lanes {
        if !lane.has_space() || lane.is_staging || !seen.insert(lane.channel_id) {
            continue;
        }
        if !use_rank_filtering {
            if lane.member_count > 0 {
                candidates.push(lane);
            }
            continue;
        }
        if lane.member_count == 0 {
            continue;
        }
        match lane.label {
            LaneLabel::NewPlayer => {
                if is_new_player {
                    candidates.push(lane);
                }
            }
            LaneLabel::StreetBrawl => {
                let wants_brawl = route.target_channel_id.is_some()
                    && lanes.iter().any(|l| {
                        Some(l.channel_id) == route.target_channel_id
                            && l.label == LaneLabel::StreetBrawl
                    });
                if wants_brawl && rank_fits_lane(rank_value, rank_sub, lane) {
                    candidates.push(lane);
                }
            }
            LaneLabel::Casual | LaneLabel::Ranked => {
                if rank_value > 0 && !rank_fits_lane(rank_value, rank_sub, lane) {
                    continue;
                }
                if rank_value > 0
                    && lane.avg_rank_value > 0.0
                    && (rank_value as f64 - lane.avg_rank_value).abs() > 3.0
                {
                    continue;
                }
                candidates.push(lane);
            }
        }
    }
    let mut ranked: Vec<&LaneInfo> = candidates;
    ranked.sort_by(|a, b| {
        let key = |lane: &LaneInfo| {
            (
                score_lobby_suggestion(
                    lane,
                    route,
                    rank_value,
                    rank_sub,
                    is_new_player,
                    use_rank_filtering,
                ),
                lane.member_count as f64,
                lane.slots_free() as f64,
                -(lane.position as f64),
            )
        };
        let (sa, ma, fa, pa) = key(a);
        let (sb, mb, fb, pb) = key(b);
        (sb, mb, fb, pb)
            .partial_cmp(&(sa, ma, fa, pa))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if rank_value > 0 {
        let tolerance = if rank_sub.is_some() { 1.5 } else { 2.0 };
        let requester = rank_value as f64 + rank_sub.unwrap_or(5) as f64 / 10.0;
        if let Some(best_fit) = ranked.iter().find(|lane| {
            lane.avg_rank_value > 0.0 && (lane.avg_rank_value - requester).abs() <= tolerance
        }) {
            return vec![(*best_fit).clone()];
        }
    }
    ranked
        .into_iter()
        .take(MAX_JOIN_LOBBIES_SHOWN)
        .cloned()
        .collect()
}

/// Anzeige-Modus (wie _resolve_mode_label).
pub fn resolve_mode_label(
    route: &RouteResult,
    best_label: Option<&LaneLabel>,
    is_new_player: bool,
    has_active: bool,
    has_explicit_rank: bool,
    rank_value: i64,
) -> &'static str {
    let label_name = |label: &LaneLabel| match label {
        LaneLabel::Casual => "Casual",
        LaneLabel::Ranked => "Ranked",
        LaneLabel::StreetBrawl => "Street Brawl",
        LaneLabel::NewPlayer => "New Player",
    };
    if is_new_player {
        return "New Player";
    }
    if has_active {
        if let Some(label) = best_label {
            return label_name(label);
        }
    }
    if let Some(label) = &route.suggested_label {
        if *label == LaneLabel::Casual && has_explicit_rank && rank_value >= 6 {
            return "Ranked";
        }
        return label_name(label);
    }
    if has_explicit_rank && rank_value >= 6 {
        return "Ranked";
    }
    "Casual"
}

/// Intro-Text (wie _compose_intro_text — alle sechs Zweige wortgleich).
pub fn compose_intro_text(
    user_mention: &str,
    rank_display: &str,
    is_new_player: bool,
    has_active: bool,
    new_player_lane_occupied: bool,
    lobby_count: usize,
) -> String {
    let rank_part = if !rank_display.is_empty() && rank_display != "Unbekannt" {
        format!(" ({rank_display})")
    } else {
        String::new()
    };
    let np_lane = format!("<#{NEW_PLAYER_LANE_ID}>");
    let coach_hint = format!(
        "\n\n💡 Allgemeiner Tipp: Movement ist in Deadlock mega wichtig — übe ruhig Dash, Slide und Air-Dash. Wenn du gezielt besser werden willst, meld dich gerne in <#{COACH_REQUEST_CHANNEL_ID}>."
    );
    if is_new_player {
        if has_active && new_player_lane_occupied {
            return format!(
                "Hey {user_mention}!{rank_part}\nWillkommen! In der {np_lane} sind schon Leute unterwegs — spring rein und spiel mit! Dort triffst du andere, die auch gerade anfangen oder entspannt spielen wollen:{coach_hint}"
            );
        }
        if has_active {
            return format!(
                "Hey {user_mention}!{rank_part}\nWillkommen! Ich hab Lobbys gefunden, die gut zu dir passen. Schau am besten auch mal in die {np_lane} — da sind alle super nett und helfen gerne weiter:{coach_hint}"
            );
        }
        return format!(
            "Hey {user_mention}!{rank_part}\nWillkommen! Mach einfach in {np_lane} eine Lobby auf — sobald du drin bist, sehen andere dass jemand da ist und es kommen erfahrungsgemäß schnell Leute dazu. Trau dich ruhig, hier sind alle freundlich! 👋{coach_hint}"
        );
    }
    if has_active {
        let lobby_text = if lobby_count == 1 {
            "Ich hab eine passende Lobby für dich gefunden"
        } else {
            "Ich hab passende Lobbys für dich gefunden"
        };
        return format!("Hey {user_mention}!{rank_part}\n{lobby_text} — schau rein und spiel mit:");
    }
    format!(
        "Hey {user_mention}!{rank_part}\nGerade ist noch niemand in einer Lobby, aber das heißt nicht dass keiner Bock hat!\nMach einfach eine Lane auf — erfahrungsgemäß kommen schnell Leute dazu."
    )
}

/// Feldtext einer vorgeschlagenen Lobby (wie _build_lobby_field_value).
pub fn build_lobby_field_value(lane: &LaneInfo, warning_line: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    if lane.member_count == 0 {
        lines.push("Noch leer — eröffne sie doch".to_string());
    } else {
        lines.push(format!("{} im Voice", lane.member_count));
    }
    lines.push(format!("Ø-Rang: {}", lane.avg_rank_label));
    if !warning_line.is_empty() {
        lines.push(warning_line.to_string());
    }
    let co_names: Vec<&String> = lane.co_player_names.iter().take(3).collect();
    if !co_names.is_empty() {
        let verb = if co_names.len() > 1 { "sind" } else { "ist" };
        lines.push(format!(
            "👥 {} {verb} auch da",
            co_names
                .iter()
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if lane.member_count >= LOBBY_MAYBE_FULL_THRESHOLD {
        lines.push(
            "⚠️ Könnte schon voll sein — schau kurz rein, sonst eigene Lobby aufmachen."
                .to_string(),
        );
    }
    lines.push(format!(
        "\nHier klicken zum Beitreten 👉 <#{}>",
        lane.channel_id
    ));
    lines.join("\n")
}

/// Staging-Kanal je Ziel-Modus (wie _resolve_staging_channel; die
/// Kategorie-Feinsuche für SB/NP übernimmt der Aufrufer über die Lanes).
pub fn resolve_staging_channel(preferred_label: &str, lanes: &[LaneInfo]) -> u64 {
    match preferred_label {
        "Ranked" => STAGING_RANKED_ID,
        "Street Brawl" => lanes
            .iter()
            .find(|lane| {
                lane.label == LaneLabel::StreetBrawl
                    && !lane.is_staging
                    && lane.member_count < LOBBY_MAYBE_FULL_THRESHOLD
            })
            .map(|lane| lane.channel_id)
            .unwrap_or(STAGING_STREET_BRAWL_ID),
        "New Player" => lanes
            .iter()
            .find(|lane| {
                lane.label == LaneLabel::NewPlayer && lane.member_count < LOBBY_MAYBE_FULL_THRESHOLD
            })
            .map(|lane| lane.channel_id)
            .unwrap_or(NEW_PLAYER_LANE_ID),
        _ => STAGING_CASUAL_ID,
    }
}

/// Kompletter Antwort-Embed (wie der Embed-Teil von _handle_lfg_request).
#[allow(clippy::too_many_arguments)]
pub fn build_lfg_reply(
    user_mention: &str,
    rank_display: &str,
    rank_value: i64,
    rank_sub: Option<i64>,
    is_new_player: bool,
    lanes: &[LaneInfo],
    route: &RouteResult,
    suggestions: &[LaneInfo],
    preferred_label: &str,
) -> serde_json::Value {
    use serde_json::json;
    let has_active = !suggestions.is_empty();
    let new_player_lane_occupied = lanes
        .iter()
        .any(|lane| lane.label == LaneLabel::NewPlayer && lane.member_count > 0);
    let mut fields: Vec<serde_json::Value> = Vec::new();
    let requester_float = rank_value as f64 + rank_sub.unwrap_or(5) as f64 / 10.0;
    if has_active {
        let shown = &suggestions[..suggestions.len().min(MAX_JOIN_LOBBIES_SHOWN)];
        for (index, lane) in shown.iter().enumerate() {
            let status = if lane.slots_free() <= 2 {
                "🟡"
            } else {
                "🟢"
            };
            let warning = if lane.member_count > 0
                && rank_value > 0
                && lane.avg_rank_value - requester_float > RANK_WARNING_DIFF
            {
                "⚠️ etwas über deinem Rang"
            } else {
                ""
            };
            fields.push(json!({
                "name": format!("{status} {}", lane.name),
                "value": build_lobby_field_value(lane, warning),
                "inline": false,
            }));
            if index < shown.len() - 1 {
                fields.push(json!({ "name": "\u{200b}", "value": "\u{200b}", "inline": false }));
            }
        }
    }
    if !is_new_player || has_active {
        let staging_id = resolve_staging_channel(preferred_label, lanes);
        fields.push(json!({
            "name": "Oder eigene Lobby aufmachen?",
            "value": format!(
                "Wenn nichts passt, mach in <#{staging_id}> eine **{preferred_label}**-Lane auf — erfahrungsgemäß kommen schnell Leute dazu."
            ),
            "inline": false,
        }));
    }
    let _ = route;
    json!({
        "title": "🎮 Lobby-Finder",
        "description": compose_intro_text(
            user_mention,
            rank_display,
            is_new_player,
            has_active,
            new_player_lane_occupied,
            suggestions.len(),
        ),
        "color": 0xE67E22,
        "fields": fields,
    })
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

    fn lane(
        id: u64,
        label: LaneLabel,
        members: usize,
        limit: usize,
        avg: f64,
        co: usize,
    ) -> LaneInfo {
        LaneInfo {
            channel_id: id,
            label,
            member_count: members,
            user_limit: limit,
            avg_rank_value: avg,
            co_players_present: co,
            name: format!("Lane {id}"),
            avg_rank_label: if avg > 0.0 {
                format!("{avg:.0}")
            } else {
                "Leer".to_string()
            },
            category_id: 0,
            position: id as i64,
            is_staging: false,
            co_player_names: (0..co).map(|i| format!("Co{i}")).collect(),
        }
    }

    /// Referenz-Entscheidungen aus dem laufenden CPython-Original.
    #[test]
    fn routing_wie_python() {
        let base = vec![
            lane(1, LaneLabel::Casual, 3, 8, 5.0, 0),
            lane(2, LaneLabel::Casual, 1, 8, 5.5, 0),
            lane(3, LaneLabel::Ranked, 2, 6, 9.0, 0),
            lane(4, LaneLabel::StreetBrawl, 1, 4, 3.0, 0),
            lane(5, LaneLabel::Casual, 0, 8, 0.0, 0),
        ];
        // casual phantom → join_existing (vollste Casual = 1)
        let r = route_to_lane("wer bock", 6, Some(3), &base);
        assert_eq!(r.mode, RouteMode::JoinExisting);
        assert_eq!(r.target_channel_id, Some(1));
        // ranked phantom → join Ranked
        let r = route_to_lane("ranked grind", 9, Some(2), &base);
        assert_eq!(r.mode, RouteMode::JoinExisting);
        assert_eq!(r.target_channel_id, Some(3));
        // ranked-Intent unter Rang 6 fällt auf den Anfänger/Casual-Pfad
        let r = route_to_lane("ranked pls", 4, Some(1), &base);
        assert_eq!(r.mode, RouteMode::JoinExisting);
        // street brawl
        let r = route_to_lane("street brawl anyone", 7, None, &base);
        assert_eq!(r.mode, RouteMode::JoinExisting);
        assert_eq!(r.target_channel_id, Some(4));
        // Anfänger ohne NP-Lane → Casual-Fallback
        let r = route_to_lane("wer bock", 2, Some(1), &base);
        assert_eq!(r.mode, RouteMode::JoinExisting);
        // Co-Spieler-Lane gewinnt trotz weniger Mitgliedern
        let co = vec![
            lane(1, LaneLabel::Casual, 3, 8, 5.0, 0),
            lane(6, LaneLabel::Casual, 2, 8, 5.0, 2),
        ];
        let r = route_to_lane("wer bock", 6, Some(3), &co);
        assert_eq!(r.mode, RouteMode::CoPlayerLane);
        assert_eq!(r.target_channel_id, Some(6));
        // alles voll → create_new mit Casual-Vorschlag
        let full = vec![lane(1, LaneLabel::Casual, 8, 8, 6.0, 0)];
        let r = route_to_lane("wer bock", 6, Some(3), &full);
        assert_eq!(r.mode, RouteMode::CreateNew);
        assert_eq!(r.suggested_label, Some(LaneLabel::Casual));
    }

    #[test]
    fn intent_und_rank_fit() {
        assert_eq!(detect_intent("ranked grind", 9), (true, false));
        assert_eq!(detect_intent("ranked grind", 4), (false, false)); // zu niedrig
        assert_eq!(detect_intent("streetbrawl!", 2), (false, true));
        // Ranked ±2, Casual ±3, NP nur bis Rang 4, leere Lane passt immer
        let ranked = lane(1, LaneLabel::Ranked, 2, 6, 9.0, 0);
        assert!(rank_fits_lane(7, Some(5), &ranked)); // 7.5 vs 9.0
        assert!(!rank_fits_lane(6, Some(1), &ranked)); // 6.1 vs 9.0
        let np = lane(2, LaneLabel::NewPlayer, 2, 6, 2.0, 0);
        assert!(rank_fits_lane(4, None, &np));
        assert!(!rank_fits_lane(5, None, &np));
        assert!(rank_fits_lane(
            11,
            None,
            &lane(3, LaneLabel::Ranked, 0, 6, 0.0, 0)
        ));
    }

    #[test]
    fn antwort_schicht() {
        // Anfänger-Erkennung
        assert!(is_new_player_request("bin neu hier", 0, false));
        assert!(is_new_player_request("egal", 3, true)); // Rang ≤4 reicht
        assert!(!is_new_player_request("bin neu", 0, true)); // Rolle ohne Rang-Wert? has_rank_role blockt Text-Pfad
        assert!(!is_new_player_request("wer bock", 9, false));

        // Auswahl: enge Rang-Passung gewinnt allein (Toleranz 2.0 ohne Subrang)
        let lanes = vec![
            lane(1, LaneLabel::Casual, 3, 8, 9.5, 0),
            lane(2, LaneLabel::Casual, 2, 8, 6.2, 0),
        ];
        let route = route_to_lane("wer bock", 6, None, &lanes);
        let picks = select_lobby_suggestions(&lanes, &route, 6, None, false, true);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].channel_id, 2); // 6.5 vs 6.2 passt eng

        // Intro-Texte: Zweige + Mehrzahl
        let intro = compose_intro_text("@u", "Phantom 2", false, true, false, 2);
        assert!(intro.contains("passende Lobbys"));
        assert!(intro.contains("(Phantom 2)"));
        let intro = compose_intro_text("@u", "Unbekannt", true, false, false, 0);
        assert!(intro.contains("Willkommen! Mach einfach in"));
        assert!(intro.contains("Allgemeiner Tipp: Movement"));

        // Feldtext: Warnung, Co-Spieler, Voll-Hinweis
        let mut full = lane(5, LaneLabel::Casual, 6, 8, 7.0, 2);
        full.co_player_names = vec!["A".into(), "B".into()];
        let value = build_lobby_field_value(&full, "⚠️ etwas über deinem Rang");
        assert!(value.contains("6 im Voice"));
        assert!(value.contains("⚠️ etwas über deinem Rang"));
        assert!(value.contains("👥 A, B sind auch da"));
        assert!(value.contains("Könnte schon voll sein"));
        assert!(value.contains("<#5>"));

        // Staging-Auflösung
        assert_eq!(resolve_staging_channel("Ranked", &[]), STAGING_RANKED_ID);
        assert_eq!(resolve_staging_channel("Casual", &[]), STAGING_CASUAL_ID);
        assert_eq!(
            resolve_staging_channel("New Player", &[]),
            NEW_PLAYER_LANE_ID
        );

        // Kompletter Embed
        let reply = build_lfg_reply(
            "@u",
            "Phantom",
            9,
            Some(2),
            false,
            &lanes,
            &route,
            &picks,
            "Casual",
        );
        assert_eq!(reply["title"], "🎮 Lobby-Finder");
        let fields = reply["fields"].as_array().expect("fields");
        assert!(fields.len() >= 2); // Lobby + eigene-Lobby-Hinweis
        assert!(fields.last().expect("last")["value"]
            .as_str()
            .expect("str")
            .contains("**Casual**-Lane"));
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
