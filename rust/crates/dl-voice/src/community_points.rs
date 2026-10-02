//! Anzeige der Community-Punkte (Community-Streamer-Bruecke, Paket C):
//! `!lb` / `!leaderboard` (gemeinsames Leaderboard), `!punkte` (eigene
//! Aufschluesselung) und `!streamerlb` (Partner-Leaderboard).
//!
//! Die Befehle haengen am selben Subscriber, Rate-Limit und Embed-Muster wie
//! `!vstats` / `!vleaderboard` (siehe [`crate::stats`]). Hier stehen nur die
//! reinen Formatierungen; die Zahlen kommen aus
//! [`dl_central_db::community_points`].

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use dl_central_db::community_points::{MemberPoints, Period, StreamerPoints};
use dl_community::concierge_community::VERIFY_CHANNEL_ID;
use serde_json::{json, Value};

/// Plaetze je Leaderboard-Embed.
pub const BOARD_LIMIT: usize = 10;
pub const COMMUNITY_LB_COLOR: u32 = 0x9146FF;
pub const PUNKTE_COLOR: u32 = 0x3498DB;
pub const STREAMER_LB_COLOR: u32 = 0xF1C40F;

pub const PUNKTE_OPTED_OUT_TEXT: &str =
    "Du hast der Speicherung deiner Daten widersprochen. Darum führen wir für dich keine Community-Punkte.";
pub const COMMUNITY_LB_EMPTY_TEXT: &str = "Noch keine Punkte in diesem Zeitraum.";
pub const STREAMER_LB_EMPTY_TEXT: &str = "Noch keine Partner-Punkte in diesem Zeitraum.";

/// Hinweis fuer Mitglieder ohne Twitch-Verknuepfung.
pub fn twitch_link_hint() -> String {
    format!(
        "Noch nicht mit Twitch verknüpft. Klick in <#{VERIFY_CHANNEL_ID}> auf \"Twitch verknüpfen\", dann zählen auch deine Punkte fürs Zuschauen und Mitchatten bei unseren Partner-Streamern."
    )
}

/// Zeitraum aus dem ersten Argument nach dem Befehl. Standard ist die
/// laufende Season (Kalendermonat).
pub fn parse_period(content: &str) -> Period {
    match content
        .split_whitespace()
        .nth(1)
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("gesamt" | "insgesamt" | "alle" | "all" | "alltime") => Period::Gesamt,
        Some("woche" | "week" | "w") => Period::Woche,
        _ => Period::Season,
    }
}

const MONATE: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

/// Anzeigename eines Zeitraums relativ zum Berliner Tag `today`.
pub fn period_label(period: Period, today: NaiveDate) -> String {
    match period {
        Period::Gesamt => "Gesamt".to_string(),
        Period::Season => {
            let month = MONATE
                .get(today.month0() as usize)
                .copied()
                .unwrap_or_default();
            format!("Season {month} {}", today.year())
        }
        Period::Woche => match period.bounds(today) {
            Some(bounds) => format!("Woche ab {}", bounds.start_day.format("%d.%m.%Y")),
            None => "Woche".to_string(),
        },
    }
}

/// Deutsche Tausender-Trennung mit Punkt.
pub fn format_de(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    if value < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// "1 Punkt", sonst "n Punkte" (mit Tausender-Trennung).
pub fn punkte_text(value: i64) -> String {
    if value == 1 {
        "1 Punkt".to_string()
    } else {
        format!("{} Punkte", format_de(value))
    }
}

fn medal(place: usize) -> String {
    match place {
        1 => "🥇".to_string(),
        2 => "🥈".to_string(),
        3 => "🥉".to_string(),
        n => format!("{n}."),
    }
}

/// Plaetze mit geteilten Rängen bei Gleichstand (Eingabe sortiert).
fn places<T>(rows: &[T], total: impl Fn(&T) -> i64) -> Vec<usize> {
    let mut out = Vec::with_capacity(rows.len());
    for (idx, row) in rows.iter().enumerate() {
        let place = match (idx.checked_sub(1), out.last()) {
            (Some(prev), Some(&last)) if total(&rows[prev]) == total(row) => last,
            _ => idx + 1,
        };
        out.push(place);
    }
    out
}

/// Embed fuer `!lb`: Top-Liste mit Gesamtpunkten und Herkunft in Kurzform.
pub fn community_board_embed(
    board: &[MemberPoints],
    names: &HashMap<u64, String>,
    label: &str,
    own: Option<(usize, i64)>,
) -> Value {
    let top = &board[..board.len().min(BOARD_LIMIT)];
    let description = if top.is_empty() {
        COMMUNITY_LB_EMPTY_TEXT.to_string()
    } else {
        let mut out = String::new();
        for (row, place) in top.iter().zip(places(top, MemberPoints::total)) {
            let uid = u64::try_from(row.discord_id).unwrap_or_default();
            let name = names
                .get(&uid)
                .cloned()
                .unwrap_or_else(|| format!("User {uid}"));
            let mut parts = Vec::new();
            if row.voice > 0 {
                parts.push(format!("Voice {}", format_de(row.voice)));
            }
            if row.twitch() > 0 {
                parts.push(format!("Twitch {}", format_de(row.twitch())));
            }
            if row.clips > 0 {
                parts.push(format!("Clips {}", format_de(row.clips)));
            }
            if row.suggestions > 0 {
                parts.push(format!("Vorschläge {}", format_de(row.suggestions)));
            }
            out.push_str(&format!(
                "{} **{name}** · {}",
                medal(place),
                punkte_text(row.total())
            ));
            if !parts.is_empty() {
                out.push_str(&format!(" ({})", parts.join(", ")));
            }
            out.push('\n');
        }
        out
    };
    let footer = match own {
        Some((rank, points)) => format!("Du bist auf Platz {rank} · {}", punkte_text(points)),
        None => "Du hast in diesem Zeitraum noch keine Punkte. Mehr dazu mit !punkte".to_string(),
    };
    json!({
        "title": format!("🏆 Community-Leaderboard · {label}"),
        "color": COMMUNITY_LB_COLOR,
        "description": description,
        "footer": { "text": footer }
    })
}

/// Daten fuer `!punkte`.
pub struct PunkteView<'a> {
    pub name: &'a str,
    pub season_label: &'a str,
    pub season: MemberPoints,
    pub season_rank: Option<(usize, i64)>,
    pub total_rank: Option<(usize, i64)>,
    pub week_points: i64,
    pub twitch_linked: bool,
}

fn rank_text(rank: Option<(usize, i64)>) -> String {
    match rank {
        Some((place, points)) => format!("{} · Platz {place}", punkte_text(points)),
        None => "0 Punkte".to_string(),
    }
}

/// Embed fuer `!punkte`: Aufschluesselung der Season plus Gesamt und Woche.
pub fn punkte_embed(view: &PunkteView<'_>) -> Value {
    let s = &view.season;
    let field =
        |name: &str, value: i64| json!({ "name": name, "value": format_de(value), "inline": true });
    let fields = vec![
        field("🎙️ Voice", s.voice),
        field("📺 Zuschauen", s.twitch_watch),
        field("💬 Chat", s.twitch_chat),
        field("🧭 Entdecken", s.twitch_discovery),
        field("🎬 Clip-Contest", s.clips),
        field("💡 Streamer-Vorschläge", s.suggestions),
        json!({ "name": "🏁 Gesamt", "value": rank_text(view.total_rank), "inline": true }),
        json!({ "name": "📅 Diese Woche", "value": punkte_text(view.week_points), "inline": true }),
    ];
    let mut description = format!("**{}:** {}", view.season_label, rank_text(view.season_rank));
    if !view.twitch_linked {
        description.push_str("\n\n");
        description.push_str(&twitch_link_hint());
    }
    json!({
        "title": format!("⭐ Community-Punkte · {}", view.name),
        "color": PUNKTE_COLOR,
        "description": description,
        "fields": fields,
        "footer": { "text": "Ranglisten: !lb (Season), !lb gesamt, !lb woche, !streamerlb" }
    })
}

/// Embed fuer `!streamerlb`: Partner nach Punkten mit Herkunft.
pub fn streamer_board_embed(board: &[StreamerPoints], label: &str) -> Value {
    let top = &board[..board.len().min(BOARD_LIMIT)];
    let description = if top.is_empty() {
        STREAMER_LB_EMPTY_TEXT.to_string()
    } else {
        let mut out = String::new();
        for (row, place) in top.iter().zip(places(top, StreamerPoints::total)) {
            let mut parts = Vec::new();
            if row.community_minutes > 0 {
                let hours = row.community_minutes / 60;
                let minutes = row.community_minutes % 60;
                parts.push(format!("Community-Zuschauzeit {hours}h {minutes}m"));
            }
            if row.raids > 0 {
                parts.push(if row.raids == 1 {
                    "1 Raid".to_string()
                } else {
                    format!("{} Raids", row.raids)
                });
            }
            if row.qualified_joins > 0 {
                parts.push(if row.qualified_joins == 1 {
                    "1 neues Mitglied".to_string()
                } else {
                    format!("{} neue Mitglieder", row.qualified_joins)
                });
            }
            if row.clip_points > 0 {
                parts.push(format!("Clip-Contest {}", format_de(row.clip_points)));
            }
            out.push_str(&format!(
                "{} **{}** · {}",
                medal(place),
                row.streamer_login,
                punkte_text(row.total())
            ));
            if !parts.is_empty() {
                out.push_str(&format!(" ({})", parts.join(", ")));
            }
            out.push('\n');
        }
        out
    };
    json!({
        "title": format!("🎥 Partner-Leaderboard · {label}"),
        "color": STREAMER_LB_COLOR,
        "description": description,
        "footer": { "text": "1 Punkt je 30 Minuten Community-Zuschauzeit, 25 je Raid an Partner, 50 je neuem aktiven Mitglied, 100/60/40 im Clip-Contest" }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("day")
    }

    fn member(id: i64, voice: i64, watch: i64, clips: i64) -> MemberPoints {
        MemberPoints {
            discord_id: id,
            voice,
            twitch_watch: watch,
            clips,
            ..MemberPoints::default()
        }
    }

    fn assert_no_dashes(text: &str) {
        assert!(
            !text.contains('—') && !text.contains('–'),
            "keine Gedankenstriche: {text}"
        );
    }

    #[test]
    fn zeitraum_aus_argument() {
        assert_eq!(parse_period("!lb"), Period::Season);
        assert_eq!(parse_period("!lb season"), Period::Season);
        assert_eq!(parse_period("!lb Gesamt"), Period::Gesamt);
        assert_eq!(parse_period("!leaderboard all"), Period::Gesamt);
        assert_eq!(parse_period("!streamerlb woche"), Period::Woche);
        assert_eq!(parse_period("!lb quatsch"), Period::Season);
    }

    #[test]
    fn zeitraum_beschriftung() {
        assert_eq!(
            period_label(Period::Season, d("2026-10-01")),
            "Season Oktober 2026"
        );
        assert_eq!(
            period_label(Period::Season, d("2026-03-31")),
            "Season März 2026"
        );
        assert_eq!(
            period_label(Period::Woche, d("2026-10-01")),
            "Woche ab 28.09.2026"
        );
        assert_eq!(period_label(Period::Gesamt, d("2026-10-01")), "Gesamt");
    }

    #[test]
    fn community_leaderboard_formatierung() {
        let board = vec![
            member(1, 100, 50, 0),
            member(2, 0, 0, 150),
            member(3, 10, 0, 0),
        ];
        let names = HashMap::from([(1_u64, "Anna".to_string()), (2_u64, "Ben".to_string())]);
        let embed = community_board_embed(&board, &names, "Season Oktober 2026", Some((3, 10)));
        let description = embed["description"].as_str().expect("description");
        assert_eq!(
            description,
            "🥇 **Anna** · 150 Punkte (Voice 100, Twitch 50)\n🥇 **Ben** · 150 Punkte (Clips 150)\n🥉 **User 3** · 10 Punkte (Voice 10)\n"
        );
        assert_eq!(
            embed["title"],
            "🏆 Community-Leaderboard · Season Oktober 2026"
        );
        assert_eq!(embed["footer"]["text"], "Du bist auf Platz 3 · 10 Punkte");
        assert_no_dashes(&embed.to_string());

        let empty = community_board_embed(&[], &HashMap::new(), "Gesamt", None);
        assert_eq!(empty["description"], COMMUNITY_LB_EMPTY_TEXT);
        assert!(empty["footer"]["text"]
            .as_str()
            .expect("footer")
            .contains("!punkte"));
    }

    #[test]
    fn leaderboard_zeigt_hoechstens_zehn_plaetze() {
        let board: Vec<MemberPoints> = (1..=15).map(|i| member(i, 100 - i, 0, 0)).collect();
        let embed = community_board_embed(&board, &HashMap::new(), "Gesamt", None);
        let lines = embed["description"]
            .as_str()
            .expect("description")
            .lines()
            .count();
        assert_eq!(lines, BOARD_LIMIT);
    }

    #[test]
    fn punkte_aufschluesselung_mit_und_ohne_verknuepfung() {
        let season = MemberPoints {
            discord_id: 7,
            voice: 1234,
            twitch_watch: 19,
            twitch_chat: 12,
            twitch_discovery: 10,
            clips: 102,
            suggestions: 150,
        };
        let mut view = PunkteView {
            name: "Anna",
            season_label: "Season Oktober 2026",
            season,
            season_rank: Some((2, 1527)),
            total_rank: Some((5, 9000)),
            week_points: 42,
            twitch_linked: true,
        };
        let embed = punkte_embed(&view);
        let fields = embed["fields"].as_array().expect("fields");
        let pairs: Vec<(String, String)> = fields
            .iter()
            .map(|f| {
                (
                    f["name"].as_str().expect("name").to_string(),
                    f["value"].as_str().expect("value").to_string(),
                )
            })
            .collect();
        assert_eq!(pairs[0], ("🎙️ Voice".into(), "1.234".into()));
        assert_eq!(pairs[1], ("📺 Zuschauen".into(), "19".into()));
        assert_eq!(pairs[2], ("💬 Chat".into(), "12".into()));
        assert_eq!(pairs[3], ("🧭 Entdecken".into(), "10".into()));
        assert_eq!(pairs[4], ("🎬 Clip-Contest".into(), "102".into()));
        assert_eq!(pairs[5], ("💡 Streamer-Vorschläge".into(), "150".into()));
        assert_eq!(
            pairs[6],
            ("🏁 Gesamt".into(), "9.000 Punkte · Platz 5".into())
        );
        assert_eq!(pairs[7], ("📅 Diese Woche".into(), "42 Punkte".into()));
        assert_eq!(
            embed["description"],
            "**Season Oktober 2026:** 1.527 Punkte · Platz 2"
        );
        assert_no_dashes(&embed.to_string());

        view.twitch_linked = false;
        view.season_rank = None;
        let embed = punkte_embed(&view);
        let description = embed["description"].as_str().expect("description");
        assert!(description.starts_with("**Season Oktober 2026:** 0 Punkte"));
        assert!(description.contains("Twitch verknüpfen"));
        assert!(description.contains("<#1398021105339334666>"));
        assert_no_dashes(description);
    }

    #[test]
    fn streamer_leaderboard_formatierung() {
        let board = vec![
            StreamerPoints {
                streamer_twitch_user_id: "456".into(),
                streamer_login: "partner_a".into(),
                community_minutes: 125,
                watch_points: 4,
                raids: 2,
                qualified_joins: 1,
                join_points: 50,
                clip_points: 60,
                ..StreamerPoints::default()
            },
            StreamerPoints {
                streamer_twitch_user_id: "789".into(),
                streamer_login: "partner_b".into(),
                community_minutes: 31,
                watch_points: 1,
                ..StreamerPoints::default()
            },
        ];
        let embed = streamer_board_embed(&board, "Season Oktober 2026");
        assert_eq!(
            embed["description"],
            "🥇 **partner_a** · 164 Punkte (Community-Zuschauzeit 2h 5m, 2 Raids, 1 neues Mitglied, Clip-Contest 60)\n🥈 **partner_b** · 1 Punkt (Community-Zuschauzeit 0h 31m)\n"
        );
        assert_no_dashes(&embed.to_string());
        let empty = streamer_board_embed(&[], "Gesamt");
        assert_eq!(empty["description"], STREAMER_LB_EMPTY_TEXT);
    }

    #[test]
    fn tausender_trennung() {
        assert_eq!(format_de(0), "0");
        assert_eq!(format_de(1234567), "1.234.567");
        assert_eq!(format_de(-1200), "-1.200");
        assert_eq!(punkte_text(1), "1 Punkt");
        assert_eq!(punkte_text(0), "0 Punkte");
        assert_eq!(punkte_text(1500), "1.500 Punkte");
    }
}
