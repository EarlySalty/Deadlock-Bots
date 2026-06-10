//! Pure TempVoice-Logik (Ränge, Namen, Verwaltungs-Checks) —
//! Referenzwerte aus CPython in den Tests.

pub const RANK_ORDER: [&str; 12] = [
    "unknown",
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
];

pub const SUFFIX_THRESHOLD_RANK: &str = "emissary";
pub const CASUAL_RANK_FALLBACK: &str = "Chill";
pub const DEFAULT_CASUAL_CAP: i64 = 8;
pub const DEFAULT_RANKED_CAP: i64 = 6;
pub const CREATE_RENAME_WINDOW_SEC: i64 = 45;
pub const OWNER_CLAIM_TOP_N: usize = 3;
pub const OWNER_CLAIM_MIN_SECONDS: i64 = 20 * 60;

const RANK_SHORT: [(&str, &str); 11] = [
    ("ini", "initiate"),
    ("see", "seeker"),
    ("alc", "alchemist"),
    ("arc", "arcanist"),
    ("rit", "ritualist"),
    ("emi", "emissary"),
    ("arch", "archon"),
    ("ora", "oracle"),
    ("pha", "phantom"),
    ("asc", "ascendant"),
    ("ete", "eternus"),
];

pub fn rank_index(name: &str) -> usize {
    let n = name.to_lowercase();
    RANK_ORDER.iter().position(|r| *r == n).unwrap_or(0)
}

fn resolve_rank_part(part: &str) -> usize {
    let idx = rank_index(part);
    if idx > 0 {
        return idx;
    }
    RANK_SHORT
        .iter()
        .find(|(short, _)| *short == part)
        .map(|(_, full)| rank_index(full))
        .unwrap_or(0)
}

/// Score = Hauptrangindex·6 (+ Sub-Rang 1-6 bei "Rang N"/"Kurz N").
pub fn rank_score(name: &str) -> usize {
    let n = name.to_lowercase().trim().to_string();
    let base = rank_index(&n);
    if base > 0 {
        return base * 6;
    }
    if let Some((rank_part, sub_part)) = n.rsplit_once(char::is_whitespace) {
        if let Ok(sub) = sub_part.parse::<usize>() {
            if (1..=6).contains(&sub) {
                let base = resolve_rank_part(rank_part.trim());
                if base > 0 {
                    return base * 6 + sub;
                }
            }
        }
    }
    0
}

/// Höchster Rangindex aus Rollennamen (erkennt "Ascendant", "Ascendant 3", "Asc 3").
pub fn member_rank_index(role_names: &[String]) -> usize {
    let mut best = 0;
    for role in role_names {
        let name = role.to_lowercase().trim().to_string();
        let idx = rank_index(&name);
        if idx > best {
            best = idx;
            continue;
        }
        if let Some((rank_part, sub_part)) = name.rsplit_once(' ') {
            if sub_part.len() == 1 && sub_part.chars().all(|c| c.is_ascii_digit()) {
                let sub: usize = sub_part.parse().unwrap_or(0);
                if (1..=6).contains(&sub) {
                    let idx2 = resolve_rank_part(rank_part);
                    if idx2 > best {
                        best = idx2;
                    }
                }
            }
        }
    }
    best
}

pub fn rank_prefix_for(role_names: &[String]) -> Option<String> {
    let idx = member_rank_index(role_names);
    (idx > 0).then(|| capitalize(RANK_ORDER[idx]))
}

/// Durchschnittsrang (unbekannte ignoriert), kaufmännisch gerundet.
pub fn average_rank_prefix(rank_indices: &[usize]) -> Option<String> {
    let known: Vec<usize> = rank_indices.iter().copied().filter(|i| *i > 0).collect();
    if known.is_empty() {
        return None;
    }
    let avg = known.iter().sum::<usize>() as f64 / known.len() as f64;
    let idx = ((avg + 0.5) as usize).clamp(1, RANK_ORDER.len() - 1);
    Some(capitalize(RANK_ORDER[idx]))
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// " • ab X"-Suffix abschneiden (wie _strip_suffixes).
pub fn strip_suffixes(current: &str) -> String {
    match current.find(" • ab ") {
        Some(idx) => current[..idx].to_string(),
        None => current.to_string(),
    }
}

/// LiveMatch-Suffix vom Worker (`" • N/M Im Match|Im Spiel|Lobby/Queue"`).
pub fn has_live_suffix(name: &str) -> bool {
    let lower = name.to_lowercase();
    let Some(bullet) = lower.find('•') else {
        return false;
    };
    let rest = lower[bullet + '•'.len_utf8()..].trim_start();
    let Some((counts, tail)) = rest.split_once(char::is_whitespace) else {
        return false;
    };
    let Some((a, b)) = counts.split_once('/') else {
        return false;
    };
    if a.is_empty()
        || b.is_empty()
        || !a.chars().all(|c| c.is_ascii_digit())
        || !b.chars().all(|c| c.is_ascii_digit())
    {
        return false;
    }
    let tail = tail.trim_start();
    tail.starts_with("im match") || tail.starts_with("im spiel") || tail.starts_with("lobby/queue")
}

/// Trailing-Nummer aus "Lane 3" → "3".
pub fn extract_lane_number(base_name: &str) -> Option<String> {
    let trimmed = base_name.trim_end();
    let digits: String = trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        return None;
    }
    let before = &trimmed[..trimmed.len() - digits.len()];
    before.ends_with(' ').then_some(digits)
}

/// Kleinste freie Nummer für "Prefix N" in der Kategorie.
pub fn next_name(existing_names: &[String], prefix: &str) -> String {
    let mut used = std::collections::HashSet::new();
    let needle = format!("{prefix} ");
    for name in existing_names {
        if let Some(rest) = name.strip_prefix(&needle) {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            // Wortgrenze wie das \b im Original
            let boundary_ok = rest[digits.len()..]
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric())
                .unwrap_or(true);
            if !digits.is_empty() && boundary_ok {
                if let Ok(n) = digits.parse::<u64>() {
                    used.insert(n);
                }
            }
        }
    }
    let mut n = 1;
    while used.contains(&n) {
        n += 1;
    }
    format!("{prefix} {n}")
}

/// Verwaltete Lane? (Kategorie + Namens-Präfix, Fixed-IDs ausgenommen.)
pub fn is_managed_lane_name(name: &str) -> bool {
    let n = name.to_lowercase();
    let mut prefixes: Vec<&str> = vec!["lane", "street brawl", "chill"];
    prefixes.extend(RANK_ORDER);
    prefixes
        .iter()
        .any(|p| n == *p || n.starts_with(&format!("{p} ")))
}

/// Lane-Name inkl. optionalem Min-Rang-Suffix (wie _compose_name).
pub fn compose_name(base: &str, min_rank: &str, in_minrank_category: bool) -> String {
    let mut name = base.to_string();
    if in_minrank_category
        && min_rank != "unknown"
        && rank_index(min_rank) >= rank_index(SUFFIX_THRESHOLD_RANK)
    {
        name.push_str(&format!(" • ab {}", capitalize(min_rank)));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_scores_wie_python() {
        // Referenz aus CPython (_rank_score)
        assert_eq!(rank_score("initiate"), 6);
        assert_eq!(rank_score("Initiate 3"), 9);
        assert_eq!(rank_score("Emissary 1"), 37);
        assert_eq!(rank_score("Asc 3"), 63);
        assert_eq!(rank_score("eternus"), 66);
        assert_eq!(rank_score("quatsch"), 0);
        assert_eq!(rank_score("Phantom 6"), 60);
        assert_eq!(rank_score("arch 2"), 44);
    }

    #[test]
    fn member_rank_aus_rollen() {
        let roles = vec![
            "Mitglied".to_string(),
            "Asc 3".to_string(),
            "Seeker".to_string(),
        ];
        assert_eq!(member_rank_index(&roles), 10); // ascendant
        assert_eq!(rank_prefix_for(&roles).as_deref(), Some("Ascendant"));
        assert_eq!(member_rank_index(&["Mod".to_string()]), 0);
    }

    #[test]
    fn durchschnittsrang() {
        // initiate(1) + eternus(11) → avg 6 → emissary
        assert_eq!(average_rank_prefix(&[1, 11]).as_deref(), Some("Emissary"));
        assert_eq!(average_rank_prefix(&[0, 0]), None);
        // unknown wird ignoriert: nur seeker(2)
        assert_eq!(average_rank_prefix(&[0, 2]).as_deref(), Some("Seeker"));
    }

    #[test]
    fn namen_helfer() {
        assert_eq!(strip_suffixes("Lane 3 • ab Phantom"), "Lane 3");
        assert_eq!(strip_suffixes("Lane 3"), "Lane 3");
        assert!(has_live_suffix("Lane 1 • 4/6 Im Match"));
        assert!(has_live_suffix("Chill 2 • 2/8 lobby/queue"));
        assert!(!has_live_suffix("Lane 1 • ab Phantom"));
        assert_eq!(extract_lane_number("Lane 12").as_deref(), Some("12"));
        assert_eq!(extract_lane_number("Phantom"), None);
        assert_eq!(
            next_name(&["Lane 1".into(), "Lane 3".into()], "Lane"),
            "Lane 2"
        );
        assert_eq!(next_name(&[], "Street Brawl"), "Street Brawl 1");
        assert_eq!(
            compose_name("Lane 2", "phantom", true),
            "Lane 2 • ab Phantom"
        );
        assert_eq!(compose_name("Lane 2", "seeker", true), "Lane 2");
        assert_eq!(compose_name("Lane 2", "phantom", false), "Lane 2");
    }

    #[test]
    fn managed_lane_namen() {
        assert!(is_managed_lane_name("Lane 4"));
        assert!(is_managed_lane_name("Phantom 2"));
        assert!(is_managed_lane_name("Street Brawl 1"));
        assert!(is_managed_lane_name("chill 3"));
        assert!(!is_managed_lane_name("AFK"));
    }
}
