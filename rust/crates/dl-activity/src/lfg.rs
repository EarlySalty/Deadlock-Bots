//! LFG-Kernlogik — Port der puren Teile von `cogs/lfg.py`.
//!
//! Enthält die Intent-Heuristik (auf 500 echten LFG-Nachrichten kalibriert),
//! die Match-Scores (Rang-Nähe, Zeit-Übereinstimmung) und das
//! Tag-Filter-Parsing. Der Discord-Flow (Routing-Antworten, Empfehlungen,
//! AI-Zweitprüfung via dl-ai) folgt mit Phase 6.

use serde_json::json;

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

fn parse_user_target(token: &str) -> Option<u64> {
    if let Some(inner) = token.strip_prefix("<@").and_then(|s| s.strip_suffix('>')) {
        return inner.strip_prefix('!').unwrap_or(inner).parse::<u64>().ok();
    }
    token.parse::<u64>().ok()
}

fn first_target(content: &str) -> Option<u64> {
    content
        .split_whitespace()
        .skip(1)
        .find_map(parse_user_target)
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
pub struct DecisionLogLine {
    prefix: DecisionLogPrefix,
    detail: String,
}

impl DecisionLogLine {
    fn new(prefix: DecisionLogPrefix, detail: impl Into<String>) -> Self {
        Self {
            prefix,
            detail: detail.into(),
        }
    }

    fn plain_text(&self) -> String {
        format!("{}: {}", self.prefix.label(), self.detail)
    }

    fn icon_text(&self) -> String {
        format!("{} {}", self.prefix.icon(), self.plain_text())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionLogPrefix {
    Mode,
    Intent,
    Scan,
    RankFilter,
    CoPlayer,
    Decision,
    Duration,
}

impl DecisionLogPrefix {
    fn label(self) -> &'static str {
        match self {
            Self::Mode => LFG_DLOG_PREFIX_MODE,
            Self::Intent => LFG_DLOG_PREFIX_INTENT,
            Self::Scan => LFG_DLOG_PREFIX_SCAN,
            Self::RankFilter => LFG_DLOG_PREFIX_RANK_FILTER,
            Self::CoPlayer => LFG_DLOG_PREFIX_COPLAYER,
            Self::Decision => LFG_DLOG_PREFIX_DECISION,
            Self::Duration => LFG_DLOG_PREFIX_DURATION,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Mode => LFG_DLOG_ICON_MODE,
            Self::Intent => LFG_DLOG_ICON_INTENT,
            Self::Scan => LFG_DLOG_ICON_SCAN,
            Self::RankFilter => LFG_DLOG_ICON_RANK_FILTER,
            Self::CoPlayer => LFG_DLOG_ICON_COPLAYER,
            Self::Decision => LFG_DLOG_ICON_DECISION,
            Self::Duration => LFG_DLOG_ICON_DURATION,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteResult {
    pub mode: RouteMode,
    pub target_channel_id: Option<u64>,
    pub suggested_label: Option<LaneLabel>,
    pub decision_log: Vec<DecisionLogLine>,
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
    let started = std::time::Instant::now();
    let (ranked_intent, sb_intent) = detect_intent(content_lower, rank_value);
    let intent_label = if sb_intent {
        "Street Brawl"
    } else if ranked_intent {
        "Ranked"
    } else {
        "Casual"
    };
    let active_lanes: Vec<&LaneInfo> = lanes.iter().filter(|lane| lane.member_count > 0).collect();
    let mut decision_log = vec![
        DecisionLogLine::new(
            DecisionLogPrefix::Intent,
            format!(
                "{intent_label}; {rank_value}; {}",
                rank_sub
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string())
            ),
        ),
        DecisionLogLine::new(
            DecisionLogPrefix::Scan,
            format!(
                "{}; {}; {}",
                lanes.len(),
                active_lanes.len(),
                lanes.len().saturating_sub(active_lanes.len())
            ),
        ),
    ];
    let finish = |mode: RouteMode,
                  target_channel_id: Option<u64>,
                  suggested_label: Option<LaneLabel>,
                  mut decision_log: Vec<DecisionLogLine>| {
        decision_log.push(DecisionLogLine::new(
            DecisionLogPrefix::Duration,
            format!("{}ms", started.elapsed().as_millis()),
        ));
        RouteResult {
            mode,
            target_channel_id,
            suggested_label,
            decision_log,
        }
    };
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
    decision_log.push(DecisionLogLine::new(
        DecisionLogPrefix::RankFilter,
        format!("{} Lanes passen", eligible.len()),
    ));

    // Co-Spieler-Lane gewinnt (meiste Co-Spieler, dann meiste Mitglieder)
    let co_lanes: Vec<&LaneInfo> = eligible
        .iter()
        .copied()
        .filter(|l| l.co_players_present > 0)
        .collect();
    for lane in &co_lanes {
        let co_players = if lane.co_player_names.is_empty() {
            lane.co_players_present.to_string()
        } else {
            lane.co_player_names.join(", ")
        };
        decision_log.push(DecisionLogLine::new(
            DecisionLogPrefix::CoPlayer,
            format!("{co_players} in '{}'", lane.name),
        ));
    }
    if let Some(best) = co_lanes
        .iter()
        .max_by_key(|l| (l.co_players_present, l.member_count))
    {
        decision_log.push(DecisionLogLine::new(
            DecisionLogPrefix::Decision,
            format!(
                "{:?}; <#{}>; {}",
                RouteMode::CoPlayerLane,
                best.channel_id,
                best.member_count
            ),
        ));
        return finish(
            RouteMode::CoPlayerLane,
            Some(best.channel_id),
            None,
            decision_log,
        );
    }
    let occupied: Vec<&&LaneInfo> = eligible.iter().filter(|l| l.member_count > 0).collect();
    if let Some(best) = occupied.iter().max_by_key(|l| l.member_count) {
        decision_log.push(DecisionLogLine::new(
            DecisionLogPrefix::Decision,
            format!(
                "{:?}; <#{}>; {}",
                RouteMode::JoinExisting,
                best.channel_id,
                best.member_count
            ),
        ));
        return finish(
            RouteMode::JoinExisting,
            Some(best.channel_id),
            None,
            decision_log,
        );
    }
    if let Some(first) = eligible.first() {
        decision_log.push(DecisionLogLine::new(
            DecisionLogPrefix::Decision,
            format!(
                "{:?}; <#{}>; {}",
                RouteMode::CreateNew,
                first.channel_id,
                lane_label_name(&first.label)
            ),
        ));
        return finish(
            RouteMode::CreateNew,
            Some(first.channel_id),
            None,
            decision_log,
        );
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
    decision_log.push(DecisionLogLine::new(
        DecisionLogPrefix::Decision,
        format!("{:?}; -; {}", RouteMode::CreateNew, lane_label_name(&label)),
    ));
    finish(RouteMode::CreateNew, None, Some(label), decision_log)
}

// ── Antwort-Schicht (wie _handle_lfg_request; Embed als JSON) ──────────────

pub const LFG_CHANNEL_ID: u64 = 1376335502919335936;
pub const OUTPUT_CHANNEL_ID: u64 = 1376335502919335936;
pub const LFG_LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const LFG_DECISION_LOG_TITLE_PLACEHOLDER: &str = "LFG Decision Log";
pub const LFG_DLOG_PREFIX_MODE: &str = "Mode";
pub const LFG_DLOG_PREFIX_INTENT: &str = "Intent";
pub const LFG_DLOG_PREFIX_SCAN: &str = "Scan";
pub const LFG_DLOG_PREFIX_RANK_FILTER: &str = "Rank-Filter";
pub const LFG_DLOG_PREFIX_COPLAYER: &str = "Co-Player";
pub const LFG_DLOG_PREFIX_DECISION: &str = "Entscheidung";
pub const LFG_DLOG_PREFIX_DURATION: &str = "Dauer";
pub const LFGTEST_OUTPUT_PLACEHOLDER: &str = "Lane-Übersicht";
pub const LFGROUTE_TITLE_PLACEHOLDER: &str = "LFG-Routing (Simulation)";
pub const LFG_STAGING_FIELD_NAME_PLACEHOLDER: &str = "Empfohlene Lane";
pub const LFG_STAGING_FIELD_VALUE_PLACEHOLDER: &str = "Hier ist noch Platz";
pub const NEW_PLAYER_LANE_ID: u64 = 1470126503252721845;
pub const COACH_REQUEST_CHANNEL_ID: u64 = 1494373349944459355;
pub const STAGING_CASUAL_ID: u64 = 1501089974093873232;
pub const STAGING_RANKED_ID: u64 = 1412804671432818890;
pub const STAGING_STREET_BRAWL_ID: u64 = 1357422958544420944;
pub const MAX_JOIN_LOBBIES_SHOWN: usize = 3;
pub const RANK_WARNING_DIFF: f64 = 1.5;
pub const LOBBY_MAYBE_FULL_THRESHOLD: usize = 6;
const LFG_DLOG_ICON_MODE: &str = "🧭";
const LFG_DLOG_ICON_INTENT: &str = "⚙️";
const LFG_DLOG_ICON_SCAN: &str = "📡";
const LFG_DLOG_ICON_RANK_FILTER: &str = "🔎";
const LFG_DLOG_ICON_COPLAYER: &str = "👥";
const LFG_DLOG_ICON_DECISION: &str = "🎯";
const LFG_DLOG_ICON_DURATION: &str = "⏱️";

fn render_decision_log_lines(lines: &[DecisionLogLine]) -> String {
    lines
        .iter()
        .map(DecisionLogLine::icon_text)
        .collect::<Vec<_>>()
        .join("\n")
}

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

fn lane_label_name(label: &LaneLabel) -> &'static str {
    match label {
        LaneLabel::Casual => "Casual",
        LaneLabel::Ranked => "Ranked",
        LaneLabel::StreetBrawl => "Street Brawl",
        LaneLabel::NewPlayer => "New Player",
    }
}

fn fixed_staging_id(preferred_label: &str) -> u64 {
    match preferred_label {
        "Ranked" => STAGING_RANKED_ID,
        "Street Brawl" => STAGING_STREET_BRAWL_ID,
        "New Player" => NEW_PLAYER_LANE_ID,
        _ => STAGING_CASUAL_ID,
    }
}

fn build_staging_field(lane: &LaneInfo) -> (String, String) {
    (
        format!(
            "{}: {} ({})",
            LFG_STAGING_FIELD_NAME_PLACEHOLDER,
            lane.name,
            lane_label_name(&lane.label)
        ),
        format!(
            "{}: <#{}> {} {}/{}",
            LFG_STAGING_FIELD_VALUE_PLACEHOLDER,
            lane.channel_id,
            lane_label_name(&lane.label),
            lane.member_count,
            lane.user_limit
        ),
    )
}

/// Staging-Kanal je Ziel-Modus (wie _resolve_staging_channel): feste IDs nur,
/// wenn sie im aktuellen Lane-Scan existieren; sonst echte Lanes aus dem Scan.
pub fn resolve_staging_channel(preferred_label: &str, lanes: &[LaneInfo]) -> Option<u64> {
    if preferred_label == "Street Brawl" || preferred_label == "New Player" {
        if let Some(lane) = lanes.iter().find(|lane| {
            lane_label_name(&lane.label) == preferred_label
                && !lane.is_staging
                && lane.member_count < LOBBY_MAYBE_FULL_THRESHOLD
        }) {
            return Some(lane.channel_id);
        }
    }
    let fixed = fixed_staging_id(preferred_label);
    if lanes.iter().any(|lane| lane.channel_id == fixed) {
        return Some(fixed);
    }
    if preferred_label == "New Player" {
        return lanes
            .iter()
            .find(|lane| lane.label == LaneLabel::NewPlayer)
            .map(|lane| lane.channel_id);
    }
    None
}

pub fn build_staging_suggestion_fields(
    preferred_label: &str,
    lanes: &[LaneInfo],
) -> Vec<(String, String)> {
    let mut ordered: Vec<&LaneInfo> = lanes
        .iter()
        .filter(|lane| lane.is_staging && lane_label_name(&lane.label) == preferred_label)
        .collect();
    ordered.extend(
        lanes
            .iter()
            .filter(|lane| lane.is_staging && lane_label_name(&lane.label) != preferred_label),
    );
    let mut fields: Vec<(String, String)> = ordered
        .into_iter()
        .take(2)
        .map(build_staging_field)
        .collect();
    if fields.is_empty() {
        fields.extend(
            lanes
                .iter()
                .filter(|lane| lane.member_count == 0 && !lane.is_staging)
                .take(1)
                .map(build_staging_field),
        );
    }
    fields
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
        if let Some(staging_id) = resolve_staging_channel(preferred_label, lanes) {
            fields.push(json!({
                "name": "Oder eigene Lobby aufmachen?",
                "value": format!(
                    "Wenn nichts passt, mach in <#{staging_id}> eine **{preferred_label}**-Lane auf — erfahrungsgemäß kommen schnell Leute dazu."
                ),
                "inline": false,
            }));
        } else {
            for (name, value) in build_staging_suggestion_fields(preferred_label, lanes) {
                fields.push(json!({
                    "name": name,
                    "value": value,
                    "inline": false,
                }));
            }
        }
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

// ── Flow-Anschluss (wie on_message + _handle_lfg_request) ─────────────────

/// Rang-Namen → Wert (Initiate=1 … Eternus=11), für Text-Parsing.
pub const RANK_NAMES: [(&str, i64); 11] = [
    ("initiate", 1),
    ("seeker", 2),
    ("alchemist", 3),
    ("arcanist", 4),
    ("ritualist", 5),
    ("emissary", 6),
    ("archon", 7),
    ("oracle", 8),
    ("phantom", 9),
    ("ascendant", 10),
    ("eternus", 11),
];

/// Rang aus dem Nachrichtentext ("Oracle 3", "emi II") — wie
/// `_parse_rank_from_message` inkl. Kurz-Aliasse und römischer Subränge.
pub fn parse_rank_from_message(content_lower: &str) -> (String, i64, Option<i64>) {
    const ALIASES: [(&str, &str); 13] = [
        ("ini", "initiate"),
        ("seek", "seeker"),
        ("alch", "alchemist"),
        ("arc", "arcanist"),
        ("rit", "ritualist"),
        ("emi", "emissary"),
        ("emiss", "emissary"),
        ("arch", "archon"),
        ("asc", "ascendant"),
        ("et", "eternus"),
        ("arkanist", "arcanist"),
        ("ascendent", "ascendant"),
        ("ethernus", "eternus"),
    ];
    let parse_sub = |token: &str| -> Option<i64> {
        let token = token.trim().trim_end_matches('+').to_lowercase();
        if let Ok(value) = token.parse::<i64>() {
            return (1..=6).contains(&value).then_some(value);
        }
        match token.as_str() {
            "i" => Some(1),
            "ii" => Some(2),
            "iii" => Some(3),
            "iv" => Some(4),
            "v" => Some(5),
            "vi" => Some(6),
            _ => None,
        }
    };
    let tokens: Vec<&str> = content_lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '+')
        .filter(|t| !t.is_empty())
        .collect();
    let mut best: (String, i64, Option<i64>) = (String::new(), 0, None);
    for (index, token) in tokens.iter().enumerate() {
        let full_name = RANK_NAMES
            .iter()
            .find(|(rank, _)| rank == token)
            .map(|(rank, _)| *rank)
            .or_else(|| {
                ALIASES
                    .iter()
                    .find(|(alias, _)| alias == token)
                    .map(|(_, full)| *full)
            });
        let Some(full_name) = full_name else { continue };
        let rank_value = RANK_NAMES
            .iter()
            .find(|(rank, _)| *rank == full_name)
            .map(|(_, value)| *value)
            .unwrap_or(0);
        if rank_value == 0 {
            continue;
        }
        let sub = tokens.get(index + 1).and_then(|next| parse_sub(next));
        if rank_value > best.1 || (rank_value == best.1 && sub.is_some()) {
            let mut display = full_name.to_string();
            if let Some(first) = display.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            best = (display, rank_value, sub);
        }
    }
    best
}

/// Discord-Seite des Flows (Cache-Scan + Posten; Tests mocken sie).
#[async_trait::async_trait]
pub trait LfgPort: Send + Sync {
    /// Alle Lanes der vier Kategorien als fertige LaneInfo
    /// (Rang-Durchschnitt aus Rollen, Co-Spieler-Markierung des Suchenden).
    async fn scan_lanes(&self, guild_id: u64, co_player_ids: &[u64]) -> Vec<LaneInfo>;
    /// (rank_name, rank_value, rank_sub) aus den Rollen des Users.
    async fn member_rank(&self, guild_id: u64, user_id: u64) -> (String, i64, Option<i64>);
    async fn member_in_voice(&self, guild_id: u64, user_id: u64) -> bool;
    async fn post_embed(&self, channel_id: u64, embed: serde_json::Value);
    async fn post_text(&self, channel_id: u64, content: &str);
}

pub struct LfgResponder {
    pub db: dl_db::Db,
    pub port: std::sync::Arc<dyn LfgPort>,
    cooldown: tokio::sync::Mutex<std::collections::HashMap<u64, std::time::Instant>>,
}

impl LfgResponder {
    pub fn new(db: dl_db::Db, port: std::sync::Arc<dyn LfgPort>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            db,
            port,
            cooldown: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Bekannte Mitspieler (≥ 2 gemeinsame Sessions, wie das Original).
    async fn co_player_ids(&self, user_id: u64) -> Vec<u64> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT co_player_id FROM user_co_players
                      WHERE user_id = ?1 AND sessions_together >= 2",
                )?;
                let rows = stmt.query_map([user_id], |row| row.get(0))?;
                rows.collect()
            })
            .await
            .unwrap_or_default()
    }

    async fn handle_lfgtest(&self, guild_id: u64, channel_id: u64) {
        let lanes = self.port.scan_lanes(guild_id, &[]).await;
        let lines: Vec<String> = lanes
            .iter()
            .map(|lane| {
                format!(
                    "- **{}**: {} (<#{}>, {}/{}, {}, {})",
                    lane_label_name(&lane.label),
                    lane.name,
                    lane.channel_id,
                    lane.member_count,
                    lane.user_limit,
                    lane.avg_rank_label,
                    lane.has_space()
                )
            })
            .collect();
        let content = if lines.is_empty() {
            LFGTEST_OUTPUT_PLACEHOLDER.to_string()
        } else {
            format!(
                "{} ({})\n{}",
                LFGTEST_OUTPUT_PLACEHOLDER,
                lanes.len(),
                lines.join("\n")
            )
        };
        self.port.post_text(channel_id, &content).await;
    }

    async fn handle_lfgroute(&self, guild_id: u64, channel_id: u64, author_id: u64, content: &str) {
        let target = first_target(content).unwrap_or(author_id);
        let (rank_name, rank_value, rank_sub) = self.port.member_rank(guild_id, target).await;
        let co_player_ids = self.co_player_ids(target).await;
        let lanes = self.port.scan_lanes(guild_id, &co_player_ids).await;
        let route = route_to_lane("", rank_value, rank_sub, &lanes);
        let rank_display = if rank_value > 0 {
            match rank_sub {
                Some(sub) => format!("{rank_name} {sub}"),
                None => rank_name,
            }
        } else {
            "Unbekannt".to_string()
        };
        self.port
            .post_embed(
                channel_id,
                json!({
                    "title": LFGROUTE_TITLE_PLACEHOLDER,
                    "color": 0xE67E22,
                    "fields": [
                        {
                            "name": LFG_DLOG_PREFIX_DECISION,
                            "value": Self::route_debug_value(target, &rank_display, &route, &lanes),
                            "inline": false
                        }
                    ],
                }),
            )
            .await;
    }

    fn route_target_display(route: &RouteResult) -> String {
        route
            .target_channel_id
            .map(|id| format!("<#{id}>"))
            .unwrap_or_else(|| "-".to_string())
    }

    fn route_suggested_display(route: &RouteResult) -> &'static str {
        route
            .suggested_label
            .as_ref()
            .map(lane_label_name)
            .unwrap_or("-")
    }

    fn route_debug_value(
        target_id: u64,
        rank_display: &str,
        route: &RouteResult,
        lanes: &[LaneInfo],
    ) -> String {
        let mut lines = vec![
            DecisionLogLine::new(DecisionLogPrefix::Intent, format!("<@{target_id}>")),
            DecisionLogLine::new(DecisionLogPrefix::RankFilter, rank_display.to_string()),
            DecisionLogLine::new(DecisionLogPrefix::Mode, format!("{:?}", route.mode)),
            DecisionLogLine::new(
                DecisionLogPrefix::Decision,
                Self::route_target_display(route),
            ),
            DecisionLogLine::new(
                DecisionLogPrefix::Decision,
                Self::route_suggested_display(route),
            ),
            DecisionLogLine::new(DecisionLogPrefix::Scan, lanes.len().to_string()),
        ];
        lines.extend(route.decision_log.iter().cloned());
        render_decision_log_lines(&lines)
    }

    fn build_decision_log_embed(
        author_id: u64,
        rank_display: &str,
        route: &RouteResult,
        suggestion_count: usize,
    ) -> serde_json::Value {
        let mut lines = vec![
            DecisionLogLine::new(DecisionLogPrefix::Mode, "lobby"),
            DecisionLogLine::new(DecisionLogPrefix::Intent, author_id.to_string()),
            DecisionLogLine::new(DecisionLogPrefix::Intent, format!("<@{author_id}>")),
            DecisionLogLine::new(DecisionLogPrefix::RankFilter, rank_display.to_string()),
            DecisionLogLine::new(DecisionLogPrefix::Mode, format!("{:?}", route.mode)),
            DecisionLogLine::new(
                DecisionLogPrefix::Decision,
                Self::route_target_display(route),
            ),
            DecisionLogLine::new(
                DecisionLogPrefix::Decision,
                Self::route_suggested_display(route),
            ),
            DecisionLogLine::new(DecisionLogPrefix::Scan, suggestion_count.to_string()),
        ];
        lines.extend(route.decision_log.iter().cloned());
        json!({
            "title": LFG_DECISION_LOG_TITLE_PLACEHOLDER,
            "color": 0x99AAB5,
            "description": render_decision_log_lines(&lines),
        })
    }

    /// Nachricht aus dem LFG-Kanal verarbeiten (wie on_message).
    pub async fn handle_message(
        self: &std::sync::Arc<Self>,
        guild_id: u64,
        channel_id: u64,
        author_id: u64,
        content: &str,
        is_admin: bool,
    ) {
        let root = content.split_whitespace().next().unwrap_or_default();
        if matches!(root, "!lfgtest" | "!lfgroute") {
            if !is_admin {
                return;
            }
            if root == "!lfgtest" {
                self.handle_lfgtest(guild_id, channel_id).await;
            } else {
                self.handle_lfgroute(guild_id, channel_id, author_id, content)
                    .await;
            }
            return;
        }
        if channel_id != LFG_CHANNEL_ID {
            return;
        }
        // Wer schon in einer Lane sitzt, sucht eine LOBBY — das übernimmt
        // der (deaktivierte) Player-Finder, nicht der Lobby-Finder.
        if self.port.member_in_voice(guild_id, author_id).await {
            return;
        }
        let content_lower = content.to_lowercase();
        if !keyword_lfg_intent(&content_lower) {
            return;
        }
        {
            let mut cooldown = self.cooldown.lock().await;
            let now = std::time::Instant::now();
            if let Some(last) = cooldown.get(&author_id) {
                if now.duration_since(*last) < std::time::Duration::from_secs(60) {
                    return;
                }
            }
            cooldown.insert(author_id, now);
        }

        // Rang: Rollen zuerst, sonst aus dem Nachrichtentext
        let (mut rank_name, mut rank_value, mut rank_sub) =
            self.port.member_rank(guild_id, author_id).await;
        let has_rank_role = rank_value > 0;
        let mut has_explicit_rank = has_rank_role;
        if rank_value == 0 {
            let (msg_name, msg_value, msg_sub) = parse_rank_from_message(&content_lower);
            if msg_value > 0 {
                rank_name = msg_name;
                rank_value = msg_value;
                rank_sub = msg_sub;
                has_explicit_rank = true;
            }
        }
        let rank_display = if rank_value > 0 {
            match rank_sub {
                Some(sub) => format!("{rank_name} {sub}"),
                None => rank_name.clone(),
            }
        } else {
            "Unbekannt".to_string()
        };
        let is_new_player = is_new_player_request(&content_lower, rank_value, has_rank_role);
        // Anfänger ohne Rang routen wie ein Alchemist 1 (Original-Fallback)
        let (routing_value, routing_sub) = if is_new_player && !has_rank_role && rank_value == 0 {
            has_explicit_rank = true;
            (3, Some(1))
        } else {
            (rank_value, rank_sub)
        };

        let co_player_ids = self.co_player_ids(author_id).await;
        let lanes = self.port.scan_lanes(guild_id, &co_player_ids).await;
        let route = route_to_lane(&content_lower, routing_value, routing_sub, &lanes);
        let suggestions = select_lobby_suggestions(
            &lanes,
            &route,
            routing_value,
            routing_sub,
            is_new_player,
            has_explicit_rank,
        );
        let best_label = route
            .target_channel_id
            .and_then(|id| lanes.iter().find(|l| l.channel_id == id))
            .map(|l| l.label.clone());
        let preferred_label = resolve_mode_label(
            &route,
            best_label.as_ref(),
            is_new_player,
            !suggestions.is_empty(),
            has_explicit_rank,
            routing_value,
        );
        let embed = build_lfg_reply(
            &format!("<@{author_id}>"),
            &rank_display,
            routing_value,
            routing_sub,
            is_new_player,
            &lanes,
            &route,
            &suggestions,
            preferred_label,
        );
        self.port.post_embed(OUTPUT_CHANNEL_ID, embed).await;
        self.port
            .post_embed(
                LFG_LOG_CHANNEL_ID,
                Self::build_decision_log_embed(author_id, &rank_display, &route, suggestions.len()),
            )
            .await;
        tracing::info!(
            author_id,
            rank = %rank_display,
            mode = ?route.mode,
            suggestions = suggestions.len(),
            "LFG-Antwort gepostet"
        );
    }
}

/// Message-Subscriber (Bot-Nachrichten filtert das Gateway).
pub fn spawn_responder(
    responder: std::sync::Arc<LfgResponder>,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    responder
                        .handle_message(
                            event.guild_id.unwrap_or_default(),
                            event.channel_id,
                            event.author_id,
                            &event.content,
                            event.author_is_admin,
                        )
                        .await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
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
    fn decision_log_prefixe_wie_python() {
        let lanes = vec![lane(1, LaneLabel::Casual, 3, 8, 5.0, 0)];
        let route = route_to_lane("wer bock", 6, Some(3), &lanes);
        let prefixes: Vec<DecisionLogPrefix> =
            route.decision_log.iter().map(|line| line.prefix).collect();
        assert_eq!(
            prefixes,
            vec![
                DecisionLogPrefix::Intent,
                DecisionLogPrefix::Scan,
                DecisionLogPrefix::RankFilter,
                DecisionLogPrefix::Decision,
                DecisionLogPrefix::Duration,
            ]
        );
        let rendered = render_decision_log_lines(&route.decision_log);
        assert!(rendered.contains(&format!("{LFG_DLOG_ICON_INTENT} {LFG_DLOG_PREFIX_INTENT}:")));
        assert!(rendered.contains(&format!("{LFG_DLOG_ICON_SCAN} {LFG_DLOG_PREFIX_SCAN}:")));
        assert!(rendered.contains(&format!(
            "{LFG_DLOG_ICON_RANK_FILTER} {LFG_DLOG_PREFIX_RANK_FILTER}:"
        )));
        assert!(rendered.contains(&format!(
            "{LFG_DLOG_ICON_DECISION} {LFG_DLOG_PREFIX_DECISION}:"
        )));
        assert!(rendered.contains(&format!(
            "{LFG_DLOG_ICON_DURATION} {LFG_DLOG_PREFIX_DURATION}:"
        )));

        let co_route = route_to_lane(
            "wer bock",
            6,
            Some(3),
            &[lane(2, LaneLabel::Casual, 2, 8, 5.0, 2)],
        );
        assert!(co_route
            .decision_log
            .iter()
            .any(|line| line.prefix == DecisionLogPrefix::CoPlayer));
        let co_rendered = render_decision_log_lines(&co_route.decision_log);
        assert!(co_rendered.contains(&format!(
            "{LFG_DLOG_ICON_COPLAYER} {LFG_DLOG_PREFIX_COPLAYER}:"
        )));
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
    fn rang_aus_nachricht() {
        assert_eq!(
            parse_rank_from_message("suche leute, bin oracle 3"),
            ("Oracle".to_string(), 8, Some(3))
        );
        assert_eq!(
            parse_rank_from_message("emi ii lobby?"),
            ("Emissary".to_string(), 6, Some(2))
        );
        assert_eq!(
            parse_rank_from_message("wer bock auf et"),
            ("Eternus".to_string(), 11, None)
        );
        assert_eq!(
            parse_rank_from_message("wer bock auf ethernus"),
            ("Eternus".to_string(), 11, None)
        );
        assert_eq!(
            parse_rank_from_message("einfach zocken"),
            (String::new(), 0, None)
        );
        // höchster Rang gewinnt
        assert_eq!(
            parse_rank_from_message("von seeker bis phantom"),
            ("Phantom".to_string(), 9, None)
        );
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
        let mut ranked_staging = lane(STAGING_RANKED_ID, LaneLabel::Ranked, 0, 6, 0.0, 0);
        ranked_staging.is_staging = true;
        let mut casual_staging = lane(STAGING_CASUAL_ID, LaneLabel::Casual, 0, 8, 0.0, 0);
        casual_staging.is_staging = true;
        assert_eq!(
            resolve_staging_channel("Ranked", &[ranked_staging.clone()]),
            Some(STAGING_RANKED_ID)
        );
        assert_eq!(
            resolve_staging_channel("Casual", &[casual_staging.clone()]),
            Some(STAGING_CASUAL_ID)
        );
        assert_eq!(resolve_staging_channel("New Player", &[]), None);

        // Kompletter Embed
        let mut reply_lanes = lanes.clone();
        reply_lanes.push(casual_staging);
        let reply = build_lfg_reply(
            "@u",
            "Phantom",
            9,
            Some(2),
            false,
            &reply_lanes,
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

    #[test]
    fn staging_fallback_prueft_existenz_und_bietet_echte_lanes_an() {
        let empty_casual = lane(501, LaneLabel::Casual, 0, 8, 0.0, 0);
        assert_eq!(
            resolve_staging_channel("Casual", &[empty_casual.clone()]),
            None
        );
        let fields = build_staging_suggestion_fields("Casual", &[empty_casual]);
        assert_eq!(fields.len(), 1);
        let rendered = format!("{} {}", fields[0].0, fields[0].1);
        assert!(rendered.contains(LFG_STAGING_FIELD_NAME_PLACEHOLDER));
        assert!(rendered.contains(LFG_STAGING_FIELD_VALUE_PLACEHOLDER));
        assert!(rendered.contains("Lane 501"));
        assert!(rendered.contains("Casual"));
        assert!(rendered.contains("<#501>"));

        let new_player = lane(NEW_PLAYER_LANE_ID + 1, LaneLabel::NewPlayer, 2, 6, 2.0, 0);
        assert_eq!(
            resolve_staging_channel("New Player", &[new_player.clone()]),
            Some(new_player.channel_id)
        );
    }

    #[tokio::test]
    async fn lfg_postet_decision_log_embed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dl_db::Db::open_creating(dir.path().join("lfg.sqlite3")).expect("db");
        db.write(|conn| {
            conn.execute(
                "CREATE TABLE user_co_players(user_id INTEGER, co_player_id INTEGER, sessions_together INTEGER)",
                [],
            )
            .map(|_| ())
        })
        .await
        .expect("ddl");
        let port = std::sync::Arc::new(MockLfgPort::new(vec![lane(
            1,
            LaneLabel::Casual,
            1,
            8,
            4.0,
            0,
        )]));
        let responder = LfgResponder::new(db, port.clone());

        responder
            .handle_message(1, LFG_CHANNEL_ID, 42, "lfg", false)
            .await;

        let embeds = port.embeds.lock().expect("embeds");
        let (_, embed) = embeds
            .iter()
            .find(|(channel_id, embed)| {
                *channel_id == LFG_LOG_CHANNEL_ID
                    && embed["title"] == LFG_DECISION_LOG_TITLE_PLACEHOLDER
            })
            .expect("decision log");
        let rendered = embed.to_string();
        assert!(rendered.contains("42"));
        assert!(rendered.contains("Alchemist 3"));
        assert!(rendered.contains("JoinExisting"));
        assert!(rendered.contains("<#1>"));
        assert!(rendered.contains("1"));
        let description = embed["description"].as_str().expect("description");
        assert!(description.contains(&format!("{LFG_DLOG_ICON_MODE} {LFG_DLOG_PREFIX_MODE}:")));
        assert!(description.contains(&format!("{LFG_DLOG_ICON_INTENT} {LFG_DLOG_PREFIX_INTENT}:")));
        assert!(description.contains(&format!("{LFG_DLOG_ICON_SCAN} {LFG_DLOG_PREFIX_SCAN}:")));
        assert!(description.contains(&format!(
            "{LFG_DLOG_ICON_RANK_FILTER} {LFG_DLOG_PREFIX_RANK_FILTER}:"
        )));
        assert!(description.contains(&format!(
            "{LFG_DLOG_ICON_DECISION} {LFG_DLOG_PREFIX_DECISION}:"
        )));
        assert!(description.contains(&format!(
            "{LFG_DLOG_ICON_DURATION} {LFG_DLOG_PREFIX_DURATION}:"
        )));
    }

    #[tokio::test]
    async fn lfg_debug_commands_sind_admin_only_und_rendern_daten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dl_db::Db::open_creating(dir.path().join("lfg-debug.sqlite3")).expect("db");
        db.write(|conn| {
            conn.execute(
                "CREATE TABLE user_co_players(user_id INTEGER, co_player_id INTEGER, sessions_together INTEGER)",
                [],
            )
            .map(|_| ())
        })
        .await
        .expect("ddl");
        let port = std::sync::Arc::new(MockLfgPort::new(vec![lane(
            987_654_321,
            LaneLabel::Casual,
            2,
            8,
            3.5,
            0,
        )]));
        let responder = LfgResponder::new(db, port.clone());

        responder
            .handle_message(1, 555, 42, "!lfgtest", false)
            .await;
        assert!(port.texts.lock().expect("texts").is_empty());

        responder.handle_message(1, 555, 42, "!lfgtest", true).await;
        responder
            .handle_message(1, 555, 42, "!lfgroute", true)
            .await;
        let texts = port.texts.lock().expect("texts");
        let overview = texts
            .iter()
            .find(|(_, content)| content.contains(LFGTEST_OUTPUT_PLACEHOLDER))
            .expect("lfgtest output");
        assert!(overview.1.contains("Lane 987654321"));
        assert!(overview.1.contains("Casual"));
        assert!(overview.1.contains("<#987654321>"));
        assert!(overview.1.contains("2/8"));
        drop(texts);
        let embeds = port.embeds.lock().expect("embeds");
        let route = embeds
            .iter()
            .find(|(_, embed)| embed["title"] == LFGROUTE_TITLE_PLACEHOLDER)
            .expect("route debug embed");
        let rendered = route.1.to_string();
        assert!(rendered.contains("42"));
        assert!(rendered.contains("Alchemist 3"));
        assert!(rendered.contains("JoinExisting"));
        assert!(rendered.contains("<#987654321>"));
        assert!(rendered.contains("1"));
        let value = route.1["fields"][0]["value"].as_str().expect("route value");
        assert!(value.contains(&format!("{LFG_DLOG_ICON_MODE} {LFG_DLOG_PREFIX_MODE}:")));
        assert!(value.contains(&format!("{LFG_DLOG_ICON_INTENT} {LFG_DLOG_PREFIX_INTENT}:")));
        assert!(value.contains(&format!("{LFG_DLOG_ICON_SCAN} {LFG_DLOG_PREFIX_SCAN}:")));
        assert!(value.contains(&format!(
            "{LFG_DLOG_ICON_RANK_FILTER} {LFG_DLOG_PREFIX_RANK_FILTER}:"
        )));
        assert!(value.contains(&format!(
            "{LFG_DLOG_ICON_DECISION} {LFG_DLOG_PREFIX_DECISION}:"
        )));
    }

    struct MockLfgPort {
        lanes: Vec<LaneInfo>,
        embeds: std::sync::Mutex<Vec<(u64, serde_json::Value)>>,
        texts: std::sync::Mutex<Vec<(u64, String)>>,
    }

    impl MockLfgPort {
        fn new(lanes: Vec<LaneInfo>) -> Self {
            Self {
                lanes,
                embeds: std::sync::Mutex::new(Vec::new()),
                texts: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl LfgPort for MockLfgPort {
        async fn scan_lanes(&self, _guild_id: u64, _co_player_ids: &[u64]) -> Vec<LaneInfo> {
            self.lanes.clone()
        }

        async fn member_rank(&self, _guild_id: u64, _user_id: u64) -> (String, i64, Option<i64>) {
            ("Alchemist".to_string(), 3, Some(3))
        }

        async fn member_in_voice(&self, _guild_id: u64, _user_id: u64) -> bool {
            false
        }

        async fn post_embed(&self, channel_id: u64, embed: serde_json::Value) {
            self.embeds
                .lock()
                .expect("embeds")
                .push((channel_id, embed));
        }

        async fn post_text(&self, channel_id: u64, content: &str) {
            self.texts
                .lock()
                .expect("texts")
                .push((channel_id, content.to_string()));
        }
    }
}
