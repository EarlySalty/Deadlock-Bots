//! Wöchentlicher Clip-Contest mit Community-Voting (Paket D der
//! Community-Streamer-Brücke).
//!
//! Ablauf je Wochenfenster (siehe `clips.rs`): Nach Fensterende friert der
//! Scheduler den Stimmzettel ein (höchstens 25 gültige Clips, älteste zuerst),
//! veröffentlicht genau einen Voting-Post im Clip-Kanal und hält die
//! Abstimmung 48 Stunden offen. Danach landen die Top 3 in
//! `clips.clip_contest_results`, als Ergebnis-Post im Kanal und per DM beim
//! Kurator. Jeder Schritt ist ein Zustandswechsel in `clips.clip_votings`;
//! nach einem Neustart macht der Scheduler dort weiter, wo die DB steht.
//!
//! Twitch-Clips von Partner-Streamern kommen über
//! `POST /internal/master/v1/clips/submit` (dl-broker) und landen über
//! [`ClipSubmission::submit_twitch`] im aktuellen Wochenfenster.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

use crate::clips::{compute_week_window, is_valid_url, ClipStore, ClipSubmission};
use crate::db::{i64_to_u64, u64_to_i64, unix_from_utc, utc_from_unix};

// ── Konstanten (keine ENV) ─────────────────────────────────────────────────

/// Voting-Post und Ergebnis-Post laufen im Clip-Kanal.
pub const VOTING_CHANNEL_ID: u64 = crate::clips::SUBMIT_CHANNEL_ID;
/// Kurator, der die Top 3 für den dach_lock-Stream bekommt.
pub const CURATOR_USER_ID: u64 = crate::clips::SEND_TO_USER_ID;
pub const VOTING_DURATION_HOURS: i64 = 48;
pub const MAX_BALLOT_ENTRIES: usize = 25;
/// Mindestalter der Mitgliedschaft für eine Stimme (gegen Zweitkonten).
pub const MIN_MEMBERSHIP_DAYS: i64 = 3;
/// Ein Clip braucht mindestens so viele Stimmen für einen Platz.
pub const MIN_VOTES_FOR_PLACE: u32 = 1;
pub const TOP_PLACES: usize = 3;
/// Ein Voting wird nur für Fenster angelegt, die höchstens so lange vorbei sind.
/// Verhindert, dass ein Deploy alte Wochen nachträglich zur Abstimmung stellt.
pub const VOTING_CATCHUP_HOURS: i64 = 36;
/// Beanspruchte, aber nicht abgeschlossene Schritte werden danach neu versucht.
pub const STALE_CLAIM_MINUTES: i64 = 10;
pub const VOTE_SELECT_PREFIX: &str = "clip_vote_select_v1:";
pub const VOTE_MINE_PREFIX: &str = "clip_vote_mine_v1:";
pub const TWITCH_PERMISSION: &str = "twitch_partner_channel";
pub const MAX_CLIP_URL_LEN: usize = 400;

const CREDIT_DISPLAY_MAX: usize = 40;
const EMBED_DESCRIPTION_MAX: usize = 4000;
const EMBED_TOTAL_BUDGET: usize = 5800;
const COLOR_VOTING: u32 = 0x9146FF;
const COLOR_RESULT: u32 = 0xF1C40F;

// ── Pure Logik ─────────────────────────────────────────────────────────────

/// Wer einen Clip eingereicht hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submitter {
    Discord(u64),
    Twitch {
        twitch_user_id: String,
        login: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContestClip {
    pub submission_id: i64,
    pub created_at_ts: i64,
    pub link: String,
    pub credit: String,
    pub submitter: Submitter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwitchClipUrl {
    pub slug: String,
    pub canonical: String,
}

fn valid_twitch_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 100
        && slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn valid_twitch_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 25
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Akzeptiert nur `https://clips.twitch.tv/<slug>` und
/// `https://(www.|m.)twitch.tv/<kanal>/clip/<slug>`; Query und Fragment
/// werden verworfen. Kanonisch ist immer die clips.twitch.tv-Form.
pub fn parse_twitch_clip_url(raw: &str) -> Option<TwitchClipUrl> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_CLIP_URL_LEN || raw.chars().any(char::is_whitespace) {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let segments: Vec<&str> = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
    let slug = match (host.as_str(), segments.as_slice()) {
        ("clips.twitch.tv", [slug]) => *slug,
        ("twitch.tv" | "www.twitch.tv" | "m.twitch.tv", [channel, "clip", slug])
            if valid_twitch_login(channel) =>
        {
            *slug
        }
        _ => return None,
    };
    valid_twitch_slug(slug).then(|| TwitchClipUrl {
        slug: slug.to_string(),
        canonical: format!("https://clips.twitch.tv/{slug}"),
    })
}

/// Schlüssel für die Duplikat-Erkennung je Woche. Twitch-Clips über ihren
/// Slug (beide URL-Formen gleich), sonst die normalisierte URL.
pub fn clip_key(link: &str) -> String {
    if let Some(clip) = parse_twitch_clip_url(link) {
        return format!("twitch:{}", clip.slug);
    }
    let trimmed = link.trim();
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    without_fragment.trim_end_matches('/').to_lowercase()
}

/// Stimmzettel: gültige Clips, Duplikate (gleicher Clip) nur einmal mit der
/// frühesten Einsendung, sortiert nach Einsendezeit, höchstens 25.
pub fn select_ballot(clips: &[ContestClip]) -> Vec<ContestClip> {
    let mut sorted: Vec<&ContestClip> = clips
        .iter()
        .filter(|clip| is_valid_url(clip.link.trim()))
        .collect();
    sorted.sort_by_key(|clip| (clip.created_at_ts, clip.submission_id));
    let mut seen = HashSet::new();
    sorted
        .into_iter()
        .filter(|clip| seen.insert(clip_key(&clip.link)))
        .take(MAX_BALLOT_ENTRIES)
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub place: u8,
    pub submission_id: i64,
    pub votes: u32,
}

/// Top 3 nach Stimmen. Gleichstand: die frühere Einsendung gewinnt (bei
/// gleicher Zeit die kleinere ID). Stimmen für Clips außerhalb des
/// Stimmzettels zählen nicht; ein Platz braucht mindestens
/// [`MIN_VOTES_FOR_PLACE`] Stimmen.
pub fn rank_top3(ballot: &[ContestClip], votes: &[i64]) -> Vec<Placement> {
    let mut counts: HashMap<i64, u32> = HashMap::new();
    for submission_id in votes {
        *counts.entry(*submission_id).or_default() += 1;
    }
    let mut ranked: Vec<(&ContestClip, u32)> = ballot
        .iter()
        .map(|clip| (clip, counts.get(&clip.submission_id).copied().unwrap_or(0)))
        .filter(|(_, votes)| *votes >= MIN_VOTES_FOR_PLACE)
        .collect();
    ranked.sort_by(|(a, va), (b, vb)| {
        vb.cmp(va)
            .then(a.created_at_ts.cmp(&b.created_at_ts))
            .then(a.submission_id.cmp(&b.submission_id))
    });
    ranked
        .into_iter()
        .take(TOP_PLACES)
        .enumerate()
        .map(|(index, (clip, votes))| Placement {
            place: u8::try_from(index + 1).unwrap_or(u8::MAX),
            submission_id: clip.submission_id,
            votes,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteDenial {
    OwnClip,
    MembershipTooNew { allowed_from_ts: i64 },
}

/// Stimmrecht: nicht für den eigenen Clip (Discord-Einsender oder per
/// verknüpftem Twitch-Konto der Streamer), Mitgliedschaft mindestens
/// [`MIN_MEMBERSHIP_DAYS`] Tage. Unbekanntes Beitrittsdatum blockiert nicht.
pub fn check_vote(
    voter_id: u64,
    voter_twitch_ids: &[String],
    clip: &ContestClip,
    joined_at_ts: Option<i64>,
    now_ts: i64,
) -> Result<(), VoteDenial> {
    let own = match &clip.submitter {
        Submitter::Discord(user_id) => *user_id == voter_id,
        Submitter::Twitch { twitch_user_id, .. } => {
            voter_twitch_ids.iter().any(|id| id == twitch_user_id)
        }
    };
    if own {
        return Err(VoteDenial::OwnClip);
    }
    if let Some(joined) = joined_at_ts {
        let allowed_from_ts = joined + MIN_MEMBERSHIP_DAYS * 86_400;
        if now_ts < allowed_from_ts {
            return Err(VoteDenial::MembershipTooNew { allowed_from_ts });
        }
    }
    Ok(())
}

/// Fenster, in das eine Twitch-Einsendung fällt: das laufende, sonst (in der
/// Stunde zwischen Samstag 23:00 und Sonntag 00:00) das nächste.
pub fn twitch_target_window(now: DateTime<Utc>) -> (i64, i64) {
    let (start, end) = compute_week_window(now);
    let now_ts = now.timestamp();
    if start <= now_ts && now_ts <= end {
        return (start, end);
    }
    let after_end = Utc.timestamp_opt(end + 2 * 3600, 0).single().unwrap_or(now);
    compute_week_window(after_end)
}

fn berlin_date(ts: i64) -> String {
    chrono_tz::Europe::Berlin
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%d.%m.%Y").to_string())
        .unwrap_or_default()
}

/// Credit für die Anzeige: einzeilig, ohne Markdown-Steuerzeichen, gekürzt.
pub fn display_credit(clip: &ContestClip) -> String {
    let raw = match &clip.submitter {
        Submitter::Twitch { login, .. } => login.clone(),
        Submitter::Discord(_) => clip.credit.clone(),
    };
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| {
            !matches!(
                c,
                '[' | ']' | '(' | ')' | '*' | '_' | '`' | '~' | '|' | '<' | '>'
            )
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = if cleaned.is_empty() {
        "ohne Namen".to_string()
    } else {
        cleaned
    };
    if cleaned.chars().count() > CREDIT_DISPLAY_MAX {
        let short: String = cleaned.chars().take(CREDIT_DISPLAY_MAX - 1).collect();
        format!("{short}…")
    } else {
        cleaned
    }
}

fn credit_suffix(clip: &ContestClip) -> String {
    match &clip.submitter {
        Submitter::Twitch { .. } => " (Twitch)".to_string(),
        Submitter::Discord(_) => String::new(),
    }
}

pub fn voting_footer(window_id: i64) -> String {
    format!("Clip-Contest · Runde {window_id}")
}

pub fn result_footer(window_id: i64) -> String {
    format!("Clip-Contest · Ergebnis Runde {window_id}")
}

/// Zeilen in Embeds verteilen: je Embed höchstens 4000 Zeichen Beschreibung,
/// alles zusammen (Titel, Beschreibungen, Footer) höchstens 5800. Passt eine
/// Zeile mit Link nicht mehr ins Budget, erscheint sie ohne Link; passt auch
/// das nicht, verweist ein Hinweis auf das Auswahlmenü.
fn ballot_embeds(header: String, lines: Vec<(String, String)>, footer: String) -> Vec<Value> {
    const TITLE: &str = "🎬 Clip-Contest: Stimm für deinen Lieblingsclip";
    const MORE: &str = "Weitere Clips findest du im Auswahlmenü.";
    let reserve = MORE.chars().count() + 1;
    let mut descriptions = vec![header];
    let mut used = descriptions[0].chars().count() + footer.chars().count() + TITLE.chars().count();
    let mut truncated = false;
    for (with_link, without_link) in lines {
        let remaining = EMBED_TOTAL_BUDGET.saturating_sub(used + reserve);
        let line = if with_link.chars().count() < remaining {
            with_link
        } else if without_link.chars().count() < remaining {
            without_link
        } else {
            truncated = true;
            break;
        };
        push_line(&mut descriptions, &mut used, line);
    }
    if truncated {
        push_line(&mut descriptions, &mut used, MORE.to_string());
    }
    let count = descriptions.len();
    descriptions
        .into_iter()
        .enumerate()
        .map(|(index, description)| {
            let mut embed = json!({ "description": description, "color": COLOR_VOTING });
            if index == 0 {
                embed["title"] = json!(TITLE);
            }
            if index + 1 == count {
                embed["footer"] = json!({ "text": footer });
            }
            embed
        })
        .collect()
}

fn push_line(descriptions: &mut Vec<String>, used: &mut usize, line: String) {
    let len = line.chars().count() + 1;
    *used += len;
    match descriptions.last_mut() {
        Some(last) if last.chars().count() + len <= EMBED_DESCRIPTION_MAX => {
            last.push('\n');
            last.push_str(&line);
        }
        _ => descriptions.push(line),
    }
}

fn ballot_lines(ballot: &[ContestClip]) -> Vec<(String, String)> {
    ballot
        .iter()
        .enumerate()
        .map(|(index, clip)| {
            let number = index + 1;
            let credit = display_credit(clip);
            let suffix = credit_suffix(clip);
            (
                format!("**{number}.** [{credit}]({}){suffix}", clip.link.trim()),
                format!("**{number}.** {credit}{suffix} (Link im Wochen-Dump)"),
            )
        })
        .collect()
}

/// Voting-Post (offen) mit Auswahlmenü.
pub fn voting_message(
    window_id: i64,
    week_start_ts: i64,
    week_end_ts: i64,
    voting_end_ts: i64,
    ballot: &[ContestClip],
) -> serde_json::Map<String, Value> {
    let header = format!(
        "Das sind die Clips der Woche vom {} bis {}. Schau sie dir an und wähle unten deinen Favoriten.\n\
         Du hast eine Stimme pro Woche und kannst sie bis zum Ende ändern. Für deinen eigenen Clip kannst du nicht stimmen.\n\
         Die Abstimmung endet <t:{voting_end_ts}:f> (<t:{voting_end_ts}:R>). Die Top 3 laufen im dach_lock-Stream.\n",
        berlin_date(week_start_ts),
        berlin_date(week_end_ts),
    );
    let embeds = ballot_embeds(header, ballot_lines(ballot), voting_footer(window_id));
    let options: Vec<Value> = ballot
        .iter()
        .enumerate()
        .map(|(index, clip)| {
            let label: String = format!("{}. {}", index + 1, display_credit(clip))
                .chars()
                .take(100)
                .collect();
            json!({ "label": label, "value": clip.submission_id.to_string() })
        })
        .collect();
    let mut body = serde_json::Map::new();
    body.insert("embeds".into(), json!(embeds));
    body.insert(
        "components".into(),
        json!([
            { "type": 1, "components": [{
                "type": 3,
                "custom_id": format!("{VOTE_SELECT_PREFIX}{window_id}"),
                "placeholder": "Clip auswählen und abstimmen",
                "min_values": 1,
                "max_values": 1,
                "options": options,
            }]},
            { "type": 1, "components": [{
                "type": 2, "style": 2, "label": "Meine Stimme anzeigen",
                "custom_id": format!("{VOTE_MINE_PREFIX}{window_id}"),
            }]},
        ]),
    );
    body.insert("allowed_mentions".into(), json!({ "parse": [] }));
    body
}

/// Voting-Post nach dem Ende: gleiche Liste, ohne Auswahl.
pub fn closed_voting_message(
    window_id: i64,
    week_start_ts: i64,
    week_end_ts: i64,
    ballot: &[ContestClip],
) -> serde_json::Map<String, Value> {
    let header = format!(
        "Das waren die Clips der Woche vom {} bis {}. Die Abstimmung ist beendet, das Ergebnis steht im Kanal.\n",
        berlin_date(week_start_ts),
        berlin_date(week_end_ts),
    );
    let embeds = ballot_embeds(header, ballot_lines(ballot), voting_footer(window_id));
    let mut body = serde_json::Map::new();
    body.insert("embeds".into(), json!(embeds));
    body.insert("components".into(), json!([]));
    body.insert("allowed_mentions".into(), json!({ "parse": [] }));
    body
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultEntry {
    pub place: u8,
    pub votes: u32,
    pub clip: ContestClip,
}

fn medal(place: u8) -> &'static str {
    match place {
        1 => "🥇",
        2 => "🥈",
        _ => "🥉",
    }
}

fn votes_word(votes: u32) -> String {
    if votes == 1 {
        "1 Stimme".to_string()
    } else {
        format!("{votes} Stimmen")
    }
}

fn submitter_text(clip: &ContestClip) -> String {
    match &clip.submitter {
        Submitter::Discord(user_id) => format!("eingereicht von <@{user_id}>"),
        Submitter::Twitch { login, .. } => format!("aus dem Twitch-Kanal von {login}"),
    }
}

/// Ergebnis-Post im Kanal.
pub fn result_message(
    window_id: i64,
    week_start_ts: i64,
    week_end_ts: i64,
    results: &[ResultEntry],
) -> serde_json::Map<String, Value> {
    let mut lines = vec![format!(
        "Die Community hat abgestimmt. Das sind die besten Clips der Woche vom {} bis {}:\n",
        berlin_date(week_start_ts),
        berlin_date(week_end_ts)
    )];
    if results.is_empty() {
        lines.push(
            "Diese Woche hat leider niemand abgestimmt. Nächste Woche gibt es eine neue Runde."
                .to_string(),
        );
    } else {
        for entry in results {
            lines.push(format!(
                "{} **Platz {}:** [{}]({}) mit {}, {}",
                medal(entry.place),
                entry.place,
                display_credit(&entry.clip),
                entry.clip.link.trim(),
                votes_word(entry.votes),
                submitter_text(&entry.clip),
            ));
        }
        lines.push(
            "\nGlückwunsch! Die Top 3 laufen demnächst im dach_lock-Stream. Danke an alle, die mitgemacht haben."
                .to_string(),
        );
    }
    let description: String = lines
        .join("\n")
        .chars()
        .take(EMBED_DESCRIPTION_MAX)
        .collect();
    let mut body = serde_json::Map::new();
    body.insert(
        "embeds".into(),
        json!([{
            "title": "🏆 Clip-Contest: Die Gewinner der Woche",
            "description": description,
            "color": COLOR_RESULT,
            "footer": { "text": result_footer(window_id) },
        }]),
    );
    // Gewinner werden genannt, aber nicht angepingt.
    body.insert("allowed_mentions".into(), json!({ "parse": [] }));
    body
}

/// DM an den Kurator mit Links und Credits für den Stream.
pub fn curator_text(week_start_ts: i64, week_end_ts: i64, results: &[ResultEntry]) -> String {
    let mut lines = vec![format!(
        "🎬 **Clip-Contest Top 3** (Woche {} bis {}) für den dach_lock-Stream:",
        berlin_date(week_start_ts),
        berlin_date(week_end_ts)
    )];
    if results.is_empty() {
        lines.push("Diese Woche gab es keine Stimmen, also keine Gewinner.".to_string());
    }
    for entry in results {
        let who = match &entry.clip.submitter {
            Submitter::Discord(user_id) => format!("Discord-ID {user_id}"),
            Submitter::Twitch {
                login,
                twitch_user_id,
            } => format!("Twitch {login} ({twitch_user_id})"),
        };
        lines.push(format!(
            "{}. Credit: {} | {} | {} | {}",
            entry.place,
            display_credit(&entry.clip),
            entry.clip.link.trim(),
            votes_word(entry.votes),
            who
        ));
    }
    let text = lines.join("\n");
    text.chars().take(1990).collect()
}

fn denial_text(denial: &VoteDenial) -> String {
    match denial {
        VoteDenial::OwnClip => {
            "🙅 Für deinen eigenen Clip kannst du nicht stimmen. Such dir einen anderen Favoriten aus.".to_string()
        }
        VoteDenial::MembershipTooNew { allowed_from_ts } => format!(
            "⏳ Abstimmen kannst du erst, wenn du mindestens {MIN_MEMBERSHIP_DAYS} Tage auf dem Server bist. Das ist <t:{allowed_from_ts}:R> so weit."
        ),
    }
}

// ── Store ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotingRow {
    pub window_id: i64,
    pub guild_id: u64,
    pub channel_id: u64,
    pub status: String,
    pub message_id: Option<u64>,
    pub week_start_ts: i64,
    pub week_end_ts: i64,
    pub voting_start_ts: Option<i64>,
    pub voting_end_ts: Option<i64>,
    pub result_message_id: Option<u64>,
    pub curator_dm_sent: bool,
    /// Ein früherer Versuch hat den Ergebnis-Post schon beansprucht.
    pub result_post_attempted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteOutcome {
    Saved,
    Changed,
    Unchanged,
    Closed,
    NotOnBallot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwitchClipRequest {
    pub clip_url: String,
    pub streamer_twitch_user_id: String,
    pub streamer_login: String,
    pub submitted_by_twitch_user_id: Option<String>,
    pub title: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwitchSubmitOutcome {
    Accepted(i64),
    Duplicate(i64),
    ReplayMetadataDrift(i64),
    Rejected(&'static str),
}

fn interval_minutes(minutes: i64) -> chrono::Duration {
    chrono::Duration::minutes(minutes)
}

fn submitter_from_row(
    source: &str,
    user_id: Option<i64>,
    streamer_twitch_user_id: Option<String>,
    streamer_login: Option<String>,
) -> Option<Submitter> {
    if source == "twitch" {
        let twitch_user_id = streamer_twitch_user_id?;
        Some(Submitter::Twitch {
            login: streamer_login.unwrap_or_else(|| twitch_user_id.clone()),
            twitch_user_id,
        })
    } else {
        Some(Submitter::Discord(i64_to_u64(user_id?, "user_id")?))
    }
}

const CLIP_COLUMNS: &str = "s.id, s.created_at, s.link, s.credit, s.source, s.user_id, \
                            s.streamer_twitch_user_id, s.streamer_login";

fn clip_from_row(row: &sqlx::postgres::PgRow) -> Option<ContestClip> {
    let source: String = row.try_get("source").ok()?;
    let submitter = submitter_from_row(
        &source,
        row.try_get("user_id").ok()?,
        row.try_get("streamer_twitch_user_id").ok()?,
        row.try_get("streamer_login").ok()?,
    )?;
    let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok()?;
    Some(ContestClip {
        submission_id: row.try_get("id").ok()?,
        created_at_ts: created_at.map(unix_from_utc).unwrap_or_default(),
        link: row.try_get("link").ok()?,
        credit: row.try_get("credit").ok()?,
        submitter,
    })
}

fn voting_from_row(row: &sqlx::postgres::PgRow) -> Option<VotingRow> {
    let opt_ts = |name: &str| -> Option<i64> {
        row.try_get::<Option<DateTime<Utc>>, _>(name)
            .ok()
            .flatten()
            .map(unix_from_utc)
    };
    let opt_id = |name: &str| -> Option<u64> {
        row.try_get::<Option<i64>, _>(name)
            .ok()
            .flatten()
            .and_then(|v| i64_to_u64(v, "message_id"))
    };
    Some(VotingRow {
        window_id: row.try_get("window_id").ok()?,
        guild_id: i64_to_u64(row.try_get("guild_id").ok()?, "guild_id")?,
        channel_id: i64_to_u64(row.try_get("channel_id").ok()?, "channel_id")?,
        status: row.try_get("status").ok()?,
        message_id: opt_id("message_id"),
        week_start_ts: unix_from_utc(row.try_get("start_at").ok()?),
        week_end_ts: unix_from_utc(row.try_get("end_at").ok()?),
        voting_start_ts: opt_ts("voting_start_at"),
        voting_end_ts: opt_ts("voting_end_at"),
        result_message_id: opt_id("result_message_id"),
        curator_dm_sent: opt_ts("curator_dm_sent_at").is_some(),
        result_post_attempted: opt_ts("result_post_claimed_at").is_some(),
    })
}

/// (status, voting_end_at, guild_id, week_start_at, week_end_at)
type FinalizeState = (
    String,
    Option<DateTime<Utc>>,
    i64,
    DateTime<Utc>,
    DateTime<Utc>,
);

const VOTING_SELECT: &str =
    "SELECT v.window_id, v.guild_id, v.channel_id, v.status, v.message_id, \
            w.start_at, w.end_at, v.voting_start_at, v.voting_end_at, v.result_message_id, \
            v.curator_dm_sent_at, v.result_post_claimed_at \
       FROM clips.clip_votings v JOIN clips.clip_windows w ON w.id = v.window_id";

impl ClipStore {
    fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Abgelaufene Fenster ohne Voting (höchstens [`VOTING_CATCHUP_HOURS`] alt).
    pub async fn windows_needing_voting(
        &self,
        guild_id: u64,
        now: DateTime<Utc>,
    ) -> Result<Vec<i64>, sqlx::Error> {
        let guild_id =
            u64_to_i64(guild_id, "guild_id").map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        sqlx::query_scalar(
            "SELECT w.id FROM clips.clip_windows w
              WHERE w.guild_id = $1 AND w.end_at <= $2 AND w.end_at > $3
                AND NOT EXISTS (SELECT 1 FROM clips.clip_votings v WHERE v.window_id = w.id)
              ORDER BY w.end_at",
        )
        .bind(guild_id)
        .bind(now)
        .bind(now - chrono::Duration::hours(VOTING_CATCHUP_HOURS))
        .fetch_all(self.pool())
        .await
    }

    /// Alle dem Fenster zugeordneten Einsendungen (Discord und Twitch).
    pub async fn window_clips(&self, window_id: i64) -> Result<Vec<ContestClip>, sqlx::Error> {
        let rows = sqlx::query(&format!(
            "SELECT {CLIP_COLUMNS}
               FROM clips.clip_window_submissions ws
               JOIN clips.clip_submissions s ON s.id = ws.submission_id
              WHERE ws.window_id = $1
              ORDER BY s.created_at, s.id"
        ))
        .bind(window_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.iter().filter_map(clip_from_row).collect())
    }

    /// Voting samt Stimmzettel genau einmal anlegen. Leerer Stimmzettel → skipped.
    pub async fn create_voting(
        &self,
        window_id: i64,
        guild_id: u64,
        channel_id: u64,
        ballot: &[ContestClip],
    ) -> Result<bool, sqlx::Error> {
        let to_db = |v: u64, f: &'static str| {
            u64_to_i64(v, f).map_err(|e| sqlx::Error::Protocol(e.to_string()))
        };
        let mut tx = self.pool().begin().await?;
        let status = if ballot.is_empty() {
            "skipped"
        } else {
            "pending"
        };
        let inserted = sqlx::query(
            "INSERT INTO clips.clip_votings(window_id, guild_id, channel_id, status, closed_at)
             VALUES ($1, $2, $3, $4, CASE WHEN $4 = 'skipped' THEN now() END)
             ON CONFLICT (window_id) DO NOTHING",
        )
        .bind(window_id)
        .bind(to_db(guild_id, "guild_id")?)
        .bind(to_db(channel_id, "channel_id")?)
        .bind(status)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        for (index, clip) in ballot.iter().take(MAX_BALLOT_ENTRIES).enumerate() {
            let position = i16::try_from(index + 1).unwrap_or(i16::MAX);
            sqlx::query(
                "INSERT INTO clips.clip_voting_entries(window_id, position, submission_id)
                 VALUES ($1, $2, $3)",
            )
            .bind(window_id)
            .bind(position)
            .bind(clip.submission_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn voting(&self, window_id: i64) -> Result<Option<VotingRow>, sqlx::Error> {
        let row = sqlx::query(&format!("{VOTING_SELECT} WHERE v.window_id = $1"))
            .bind(window_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().and_then(voting_from_row))
    }

    /// Votings, an denen der Scheduler noch etwas zu tun hat.
    pub async fn unfinished_votings(&self, guild_id: u64) -> Result<Vec<VotingRow>, sqlx::Error> {
        let guild_id =
            u64_to_i64(guild_id, "guild_id").map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let rows = sqlx::query(&format!(
            "{VOTING_SELECT} WHERE v.guild_id = $1
               AND v.status IN ('pending', 'publishing', 'open', 'closing')
             ORDER BY v.window_id"
        ))
        .bind(guild_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.iter().filter_map(voting_from_row).collect())
    }

    /// Eingefrorener Stimmzettel in Positionsreihenfolge.
    pub async fn ballot(&self, window_id: i64) -> Result<Vec<ContestClip>, sqlx::Error> {
        let rows = sqlx::query(&format!(
            "SELECT {CLIP_COLUMNS}
               FROM clips.clip_voting_entries e
               JOIN clips.clip_submissions s ON s.id = e.submission_id
              WHERE e.window_id = $1
              ORDER BY e.position"
        ))
        .bind(window_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.iter().filter_map(clip_from_row).collect())
    }

    /// Veröffentlichung beanspruchen (pending oder hängengebliebenes publishing).
    pub async fn claim_publish(
        &self,
        window_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let affected = sqlx::query(
            "UPDATE clips.clip_votings
                SET status = 'publishing', publish_claimed_at = $2, updated_at = $2
              WHERE window_id = $1
                AND (status = 'pending'
                     OR (status = 'publishing' AND publish_claimed_at < $3))",
        )
        .bind(window_id)
        .bind(now)
        .bind(now - interval_minutes(STALE_CLAIM_MINUTES))
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(affected == 1)
    }

    /// Geplantes Zeitfenster vor dem Senden festhalten, damit ein Neustart nach
    /// dem Senden dieselben Zeiten wie im Post verwendet.
    pub async fn plan_voting_times(
        &self,
        window_id: i64,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE clips.clip_votings
                SET voting_start_at = $2, voting_end_at = $3, updated_at = now()
              WHERE window_id = $1 AND status = 'publishing'",
        )
        .bind(window_id)
        .bind(start)
        .bind(end)
        .execute(self.pool())
        .await
        .map(|_| ())
    }

    pub async fn mark_open(&self, window_id: i64, message_id: u64) -> Result<bool, sqlx::Error> {
        let message_id = u64_to_i64(message_id, "message_id")
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let affected = sqlx::query(
            "UPDATE clips.clip_votings
                SET status = 'open', message_id = $2, updated_at = now()
              WHERE window_id = $1 AND status = 'publishing'
                AND voting_start_at IS NOT NULL AND voting_end_at IS NOT NULL",
        )
        .bind(window_id)
        .bind(message_id)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(affected == 1)
    }

    pub async fn mark_skipped(&self, window_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE clips.clip_votings SET status = 'skipped', closed_at = now(), updated_at = now()
              WHERE window_id = $1 AND status IN ('pending', 'publishing')",
        )
        .bind(window_id)
        .execute(self.pool())
        .await
        .map(|_| ())
    }

    /// Stimme speichern oder ändern. Sperrt die Voting-Zeile geteilt, damit
    /// keine Stimme nach dem Auszählen (exklusive Sperre) mehr durchrutscht.
    pub async fn cast_vote(
        &self,
        window_id: i64,
        voter_id: u64,
        submission_id: i64,
        now: DateTime<Utc>,
    ) -> Result<VoteOutcome, sqlx::Error> {
        let voter = u64_to_i64(voter_id, "voter_user_id")
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let mut tx = self.pool().begin().await?;
        let state: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT status, voting_end_at FROM clips.clip_votings
              WHERE window_id = $1 FOR SHARE",
        )
        .bind(window_id)
        .fetch_optional(&mut *tx)
        .await?;
        let open = matches!(&state, Some((status, Some(end))) if status == "open" && now < *end);
        if !open {
            tx.rollback().await?;
            return Ok(VoteOutcome::Closed);
        }
        let on_ballot: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM clips.clip_voting_entries
                            WHERE window_id = $1 AND submission_id = $2)",
        )
        .bind(window_id)
        .bind(submission_id)
        .fetch_one(&mut *tx)
        .await?;
        if !on_ballot {
            tx.rollback().await?;
            return Ok(VoteOutcome::NotOnBallot);
        }
        let previous: Option<i64> = sqlx::query_scalar(
            "SELECT submission_id FROM clips.clip_votes
              WHERE window_id = $1 AND voter_user_id = $2 FOR UPDATE",
        )
        .bind(window_id)
        .bind(voter)
        .fetch_optional(&mut *tx)
        .await?;
        let outcome = match previous {
            Some(prev) if prev == submission_id => VoteOutcome::Unchanged,
            Some(_) => {
                sqlx::query(
                    "UPDATE clips.clip_votes SET submission_id = $3, updated_at = $4
                      WHERE window_id = $1 AND voter_user_id = $2",
                )
                .bind(window_id)
                .bind(voter)
                .bind(submission_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                VoteOutcome::Changed
            }
            None => {
                sqlx::query(
                    "INSERT INTO clips.clip_votes(window_id, voter_user_id, submission_id, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $4)
                     ON CONFLICT (window_id, voter_user_id)
                     DO UPDATE SET submission_id = excluded.submission_id, updated_at = excluded.updated_at",
                )
                .bind(window_id)
                .bind(voter)
                .bind(submission_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                VoteOutcome::Saved
            }
        };
        tx.commit().await?;
        Ok(outcome)
    }

    pub async fn my_vote(&self, window_id: i64, voter_id: u64) -> Result<Option<i64>, sqlx::Error> {
        let voter = u64_to_i64(voter_id, "voter_user_id")
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        sqlx::query_scalar(
            "SELECT submission_id FROM clips.clip_votes WHERE window_id = $1 AND voter_user_id = $2",
        )
        .bind(window_id)
        .bind(voter)
        .fetch_optional(self.pool())
        .await
    }

    /// Auszählen und Top 3 genau einmal speichern (open → closing).
    pub async fn finalize_voting(
        &self,
        window_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        let state: Option<FinalizeState> = sqlx::query_as(
            "SELECT v.status, v.voting_end_at, v.guild_id, w.start_at, w.end_at
                   FROM clips.clip_votings v JOIN clips.clip_windows w ON w.id = v.window_id
                  WHERE v.window_id = $1 FOR UPDATE OF v",
        )
        .bind(window_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((status, Some(end), guild_id, week_start, week_end)) = state else {
            tx.rollback().await?;
            return Ok(false);
        };
        if status != "open" || now < end {
            tx.rollback().await?;
            return Ok(false);
        }
        let rows = sqlx::query(&format!(
            "SELECT {CLIP_COLUMNS}
               FROM clips.clip_voting_entries e
               JOIN clips.clip_submissions s ON s.id = e.submission_id
              WHERE e.window_id = $1
              ORDER BY e.position"
        ))
        .bind(window_id)
        .fetch_all(&mut *tx)
        .await?;
        let ballot: Vec<ContestClip> = rows.iter().filter_map(clip_from_row).collect();
        let votes: Vec<i64> =
            sqlx::query_scalar("SELECT submission_id FROM clips.clip_votes WHERE window_id = $1")
                .bind(window_id)
                .fetch_all(&mut *tx)
                .await?;
        for placement in rank_top3(&ballot, &votes) {
            let Some(clip) = ballot
                .iter()
                .find(|clip| clip.submission_id == placement.submission_id)
            else {
                continue;
            };
            let (source, user_id, twitch_id, login) = match &clip.submitter {
                Submitter::Discord(user_id) => (
                    "discord",
                    Some(
                        u64_to_i64(*user_id, "user_id")
                            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?,
                    ),
                    None,
                    None,
                ),
                Submitter::Twitch {
                    twitch_user_id,
                    login,
                } => (
                    "twitch",
                    None,
                    Some(twitch_user_id.clone()),
                    Some(login.clone()),
                ),
            };
            sqlx::query(
                "INSERT INTO clips.clip_contest_results(
                     window_id, place, guild_id, week_start_at, week_end_at, submission_id,
                     source, user_id, streamer_twitch_user_id, streamer_login, votes, decided_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT (window_id, place) DO NOTHING",
            )
            .bind(window_id)
            .bind(i16::from(placement.place))
            .bind(guild_id)
            .bind(week_start)
            .bind(week_end)
            .bind(placement.submission_id)
            .bind(source)
            .bind(user_id)
            .bind(twitch_id)
            .bind(login)
            .bind(i32::try_from(placement.votes).unwrap_or(i32::MAX))
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE clips.clip_votings SET status = 'closing', updated_at = $2 WHERE window_id = $1",
        )
        .bind(window_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Gespeicherte Top 3 mit Clip-Daten (Clip kann per Löschung fehlen).
    pub async fn results(&self, window_id: i64) -> Result<Vec<ResultEntry>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT r.place, r.votes, r.source, r.user_id, r.streamer_twitch_user_id,
                    r.streamer_login, s.id, s.created_at, s.link, s.credit
               FROM clips.clip_contest_results r
               JOIN clips.clip_submissions s ON s.id = r.submission_id
              WHERE r.window_id = $1
              ORDER BY r.place",
        )
        .bind(window_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let place: i16 = row.try_get("place").ok()?;
                let votes: i32 = row.try_get("votes").ok()?;
                let source: String = row.try_get("source").ok()?;
                let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok()?;
                Some(ResultEntry {
                    place: u8::try_from(place).ok()?,
                    votes: u32::try_from(votes).ok()?,
                    clip: ContestClip {
                        submission_id: row.try_get("id").ok()?,
                        created_at_ts: created_at.map(unix_from_utc).unwrap_or_default(),
                        link: row.try_get("link").ok()?,
                        credit: row.try_get("credit").ok()?,
                        submitter: submitter_from_row(
                            &source,
                            row.try_get("user_id").ok()?,
                            row.try_get("streamer_twitch_user_id").ok()?,
                            row.try_get("streamer_login").ok()?,
                        )?,
                    },
                })
            })
            .collect())
    }

    pub async fn claim_result_post(
        &self,
        window_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let affected = sqlx::query(
            "UPDATE clips.clip_votings SET result_post_claimed_at = $2, updated_at = $2
              WHERE window_id = $1 AND status = 'closing' AND result_message_id IS NULL
                AND (result_post_claimed_at IS NULL OR result_post_claimed_at < $3)",
        )
        .bind(window_id)
        .bind(now)
        .bind(now - interval_minutes(STALE_CLAIM_MINUTES))
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(affected == 1)
    }

    pub async fn set_result_message(
        &self,
        window_id: i64,
        message_id: u64,
    ) -> Result<(), sqlx::Error> {
        let message_id = u64_to_i64(message_id, "message_id")
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        sqlx::query(
            "UPDATE clips.clip_votings SET result_message_id = $2, updated_at = now()
              WHERE window_id = $1 AND result_message_id IS NULL",
        )
        .bind(window_id)
        .bind(message_id)
        .execute(self.pool())
        .await
        .map(|_| ())
    }

    pub async fn claim_curator_dm(
        &self,
        window_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let affected = sqlx::query(
            "UPDATE clips.clip_votings SET curator_dm_claimed_at = $2, updated_at = $2
              WHERE window_id = $1 AND status = 'closing' AND curator_dm_sent_at IS NULL
                AND (curator_dm_claimed_at IS NULL OR curator_dm_claimed_at < $3)",
        )
        .bind(window_id)
        .bind(now)
        .bind(now - interval_minutes(STALE_CLAIM_MINUTES))
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(affected == 1)
    }

    pub async fn mark_curator_dm_sent(&self, window_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE clips.clip_votings SET curator_dm_sent_at = now(), updated_at = now()
              WHERE window_id = $1 AND curator_dm_sent_at IS NULL",
        )
        .bind(window_id)
        .execute(self.pool())
        .await
        .map(|_| ())
    }

    /// closing → closed, sobald Ergebnis-Post und Kurator-DM erledigt sind.
    pub async fn close_if_done(&self, window_id: i64) -> Result<bool, sqlx::Error> {
        let affected = sqlx::query(
            "UPDATE clips.clip_votings SET status = 'closed', closed_at = now(), updated_at = now()
              WHERE window_id = $1 AND status = 'closing'
                AND result_message_id IS NOT NULL AND curator_dm_sent_at IS NOT NULL",
        )
        .bind(window_id)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(affected == 1)
    }

    /// Mit Discord verknüpfte Twitch-IDs (Paket A). Fehlt die Tabelle noch,
    /// gibt es keine Verknüpfungen.
    pub async fn linked_twitch_ids(&self, user_id: u64) -> Result<Vec<String>, sqlx::Error> {
        let exists: bool = sqlx::query_scalar(
            "SELECT to_regclass('core.discord_platform_connections') IS NOT NULL",
        )
        .fetch_one(self.pool())
        .await?;
        if !exists {
            return Ok(Vec::new());
        }
        sqlx::query_scalar(
            "SELECT platform_user_id::text FROM core.discord_platform_connections
              WHERE discord_id::text = $1 AND platform = 'twitch'",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await
    }

    /// Partnerkanal: bekannter Streamer in `bot.twitch_streamer_invites`
    /// (über die Twitch-ID, ersatzweise über den Login, solange die ID fehlt).
    pub async fn is_partner_channel(
        &self,
        guild_id: u64,
        streamer_twitch_user_id: &str,
        streamer_login: &str,
    ) -> Result<bool, sqlx::Error> {
        let guild_id =
            u64_to_i64(guild_id, "guild_id").map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM bot.twitch_streamer_invites
                  WHERE (guild_id = $1 OR guild_id IS NULL)
                    AND (twitch_user_id = $2
                         OR (twitch_user_id IS NULL AND lower(streamer_login) = lower($3))))",
        )
        .bind(guild_id)
        .bind(streamer_twitch_user_id)
        .bind(streamer_login)
        .fetch_one(self.pool())
        .await
    }

    /// Twitch-Einsendung idempotent speichern. Die Fensterzeile wird für die
    /// Dauer der Prüfung exklusiv gesperrt, damit parallele Einsendungen
    /// desselben Clips nicht beide durchkommen.
    pub async fn submit_twitch(
        &self,
        guild_id: u64,
        request: &TwitchClipRequest,
        now: DateTime<Utc>,
    ) -> Result<TwitchSubmitOutcome, sqlx::Error> {
        let Some(clip) = parse_twitch_clip_url(&request.clip_url) else {
            return Ok(TwitchSubmitOutcome::Rejected("invalid_clip_url"));
        };
        let key = format!("twitch:{}", clip.slug);
        let guild =
            u64_to_i64(guild_id, "guild_id").map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let (start_ts, end_ts) = twitch_target_window(now);
        let start_at = utc_from_unix(start_ts).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let end_at = utc_from_unix(end_ts).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO clips.clip_windows(guild_id, start_at, end_at, status)
             VALUES ($1, $2, $3, 'running')
             ON CONFLICT (guild_id, start_at, end_at) DO NOTHING",
        )
        .bind(guild)
        .bind(start_at)
        .bind(end_at)
        .execute(&mut *tx)
        .await?;
        let window_id: i64 = sqlx::query_scalar(
            "SELECT id FROM clips.clip_windows
              WHERE guild_id = $1 AND start_at = $2 AND end_at = $3 FOR UPDATE",
        )
        .bind(guild)
        .bind(start_at)
        .bind(end_at)
        .fetch_one(&mut *tx)
        .await?;

        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(ToString::to_string);
        let replay: Option<(
            i64,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, link, streamer_twitch_user_id, streamer_login,
                    submitted_by_twitch_user_id, title
               FROM clips.clip_submissions WHERE idempotency_key = $1",
        )
        .bind(&request.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?;
        // Der Producer-Schlüssel bindet die Clip-ID. Metadaten bleiben beim ersten
        // Submit; Drift wird als Duplicate sichtbar, ohne einen Retry auszulösen.
        if let Some((id, link, streamer_id, streamer_login, submitted_by, stored_title)) = replay {
            tx.rollback().await?;
            return Ok(if clip_key(&link) != key {
                TwitchSubmitOutcome::Rejected("idempotency_conflict")
            } else if streamer_id == request.streamer_twitch_user_id
                && streamer_login == request.streamer_login
                && submitted_by == request.submitted_by_twitch_user_id
                && stored_title == title
            {
                TwitchSubmitOutcome::Accepted(id)
            } else {
                TwitchSubmitOutcome::ReplayMetadataDrift(id)
            });
        }

        let existing: Vec<(i64, String)> = sqlx::query_as(
            "SELECT s.id, s.link FROM clips.clip_window_submissions ws
               JOIN clips.clip_submissions s ON s.id = ws.submission_id
              WHERE ws.window_id = $1
              ORDER BY s.created_at, s.id",
        )
        .bind(window_id)
        .fetch_all(&mut *tx)
        .await?;
        if let Some((id, _)) = existing.iter().find(|(_, link)| clip_key(link) == key) {
            tx.rollback().await?;
            return Ok(TwitchSubmitOutcome::Duplicate(*id));
        }

        let id: i64 = sqlx::query_scalar(
            "INSERT INTO clips.clip_submissions(
                 guild_id, user_id, link, credit, permission, info, created_at, source,
                 streamer_twitch_user_id, streamer_login, submitted_by_twitch_user_id, title,
                 idempotency_key)
             VALUES ($1, NULL, $2, $3, $4, $5, $6, 'twitch', $7, $8, $9, $5, $10)
             RETURNING id",
        )
        .bind(guild)
        .bind(&clip.canonical)
        .bind(&request.streamer_login)
        .bind(TWITCH_PERMISSION)
        .bind(&title)
        .bind(now)
        .bind(&request.streamer_twitch_user_id)
        .bind(&request.streamer_login)
        .bind(&request.submitted_by_twitch_user_id)
        .bind(&request.idempotency_key)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO clips.clip_window_submissions(window_id, submission_id, user_id)
             VALUES ($1, $2, NULL)",
        )
        .bind(window_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(TwitchSubmitOutcome::Accepted(id))
    }
}

// ── Discord-Seite / Scheduler ──────────────────────────────────────────────

impl ClipSubmission {
    /// Twitch-Einsendung aus dem Broker. Prüft Partnerkanal und Clip-URL.
    pub async fn submit_twitch(
        &self,
        guild_id: u64,
        request: &TwitchClipRequest,
    ) -> Result<TwitchSubmitOutcome, sqlx::Error> {
        if parse_twitch_clip_url(&request.clip_url).is_none() {
            return Ok(TwitchSubmitOutcome::Rejected("invalid_clip_url"));
        }
        if !self
            .store
            .is_partner_channel(
                guild_id,
                &request.streamer_twitch_user_id,
                &request.streamer_login,
            )
            .await?
        {
            return Ok(TwitchSubmitOutcome::Rejected("not_partner"));
        }
        self.store
            .submit_twitch(guild_id, request, Utc::now())
            .await
    }

    /// Ein Scheduler-Schritt für den Contest. Jede Phase ist idempotent und
    /// liest ihren Zustand aus der DB.
    pub async fn process_contest(&self, guild_id: u64) {
        let now = Utc::now();
        match self.store.windows_needing_voting(guild_id, now).await {
            Ok(windows) => {
                for window_id in windows {
                    let ballot = match self.store.window_clips(window_id).await {
                        Ok(clips) => select_ballot(&clips),
                        Err(err) => {
                            tracing::warn!(%err, window_id, "Clip-Contest: Einsendungen nicht lesbar");
                            continue;
                        }
                    };
                    if let Err(err) = self
                        .store
                        .create_voting(window_id, guild_id, VOTING_CHANNEL_ID, &ballot)
                        .await
                    {
                        tracing::warn!(%err, window_id, "Clip-Contest: Voting nicht angelegt");
                    }
                }
            }
            Err(err) => tracing::warn!(%err, "Clip-Contest: Fensterabfrage fehlgeschlagen"),
        }
        let votings = match self.store.unfinished_votings(guild_id).await {
            Ok(votings) => votings,
            Err(err) => {
                tracing::warn!(%err, "Clip-Contest: Votings nicht lesbar");
                return;
            }
        };
        for voting in votings {
            match voting.status.as_str() {
                "pending" | "publishing" => self.publish_voting(&voting, now).await,
                "open" => {
                    if voting
                        .voting_end_ts
                        .is_some_and(|end| now.timestamp() >= end)
                    {
                        match self.store.finalize_voting(voting.window_id, now).await {
                            Ok(true) => {
                                if let Ok(Some(updated)) = self.store.voting(voting.window_id).await
                                {
                                    self.finish_voting(&updated, now).await;
                                }
                            }
                            Ok(false) => {}
                            Err(err) => tracing::warn!(
                                %err,
                                window_id = voting.window_id,
                                "Clip-Contest: Auszählung fehlgeschlagen"
                            ),
                        }
                    }
                }
                "closing" => self.finish_voting(&voting, now).await,
                _ => {}
            }
        }
    }

    async fn publish_voting(&self, voting: &VotingRow, now: DateTime<Utc>) {
        match self.store.claim_publish(voting.window_id, now).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(err) => {
                tracing::warn!(%err, window_id = voting.window_id, "Clip-Contest: Claim fehlgeschlagen");
                return;
            }
        }
        let ballot = match self.store.ballot(voting.window_id).await {
            Ok(ballot) => ballot,
            Err(err) => {
                tracing::warn!(%err, window_id = voting.window_id, "Clip-Contest: Stimmzettel nicht lesbar");
                return;
            }
        };
        if ballot.is_empty() {
            let _ = self.store.mark_skipped(voting.window_id).await;
            return;
        }
        // Gab es schon einen Versuch (Zeiten geplant), kann der Post nach einem
        // Abbruch zwischen Senden und Speichern schon im Kanal stehen: erst
        // suchen, dann senden. Schlägt die Suche fehl, wird nicht gesendet; der
        // Claim läuft ab und der nächste Versuch sucht erneut.
        let existing = if voting.voting_start_ts.is_some() {
            let footer = voting_footer(voting.window_id);
            match self
                .port
                .find_message_by_footer(voting.channel_id, &footer)
                .await
            {
                Ok(found) => found,
                Err(err) => {
                    tracing::warn!(%err, window_id = voting.window_id, "Clip-Contest: Kanalsuche fehlgeschlagen");
                    return;
                }
            }
        } else {
            None
        };
        let message_id = match (existing, voting.voting_start_ts, voting.voting_end_ts) {
            (Some(message_id), Some(_), Some(_)) => message_id,
            _ => {
                let start = now;
                let end = now + chrono::Duration::hours(VOTING_DURATION_HOURS);
                if let Err(err) = self
                    .store
                    .plan_voting_times(voting.window_id, start, end)
                    .await
                {
                    tracing::warn!(%err, window_id = voting.window_id, "Clip-Contest: Zeitplan nicht gespeichert");
                    return;
                }
                let body = voting_message(
                    voting.window_id,
                    voting.week_start_ts,
                    voting.week_end_ts,
                    end.timestamp(),
                    &ballot,
                );
                match self
                    .port
                    .post_message(
                        voting.channel_id,
                        body,
                        format!("clipvote{}", voting.window_id),
                    )
                    .await
                {
                    Ok(message_id) => message_id,
                    Err(err) => {
                        tracing::warn!(%err, window_id = voting.window_id, "Clip-Contest: Voting-Post fehlgeschlagen");
                        return;
                    }
                }
            }
        };
        match self.store.mark_open(voting.window_id, message_id).await {
            Ok(true) => tracing::info!(
                window_id = voting.window_id,
                message_id,
                "Clip-Contest: Voting offen"
            ),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(%err, window_id = voting.window_id, "Clip-Contest: Status open nicht gespeichert")
            }
        }
    }

    /// Ergebnis-Post, Voting-Post schließen, Kurator-DM; danach closed.
    async fn finish_voting(&self, voting: &VotingRow, now: DateTime<Utc>) {
        let window_id = voting.window_id;
        let results = match self.store.results(window_id).await {
            Ok(results) => results,
            Err(err) => {
                tracing::warn!(%err, window_id, "Clip-Contest: Ergebnis nicht lesbar");
                return;
            }
        };
        if voting.result_message_id.is_none()
            && matches!(self.store.claim_result_post(window_id, now).await, Ok(true))
        {
            let found = if voting.result_post_attempted {
                let footer = result_footer(window_id);
                match self
                    .port
                    .find_message_by_footer(voting.channel_id, &footer)
                    .await
                {
                    Ok(found) => found,
                    Err(err) => {
                        tracing::warn!(%err, window_id, "Clip-Contest: Kanalsuche fehlgeschlagen");
                        return;
                    }
                }
            } else {
                None
            };
            let posted = match found {
                Some(message_id) => Ok(message_id),
                None => {
                    self.port
                        .post_message(
                            voting.channel_id,
                            result_message(
                                window_id,
                                voting.week_start_ts,
                                voting.week_end_ts,
                                &results,
                            ),
                            format!("clipres{window_id}"),
                        )
                        .await
                }
            };
            match posted {
                Ok(message_id) => {
                    if let Err(err) = self.store.set_result_message(window_id, message_id).await {
                        tracing::warn!(%err, window_id, "Clip-Contest: Ergebnis-Post nicht gespeichert");
                    }
                    // Auswahl am Voting-Post entfernen; rein kosmetisch, Stimmen
                    // sind ab Voting-Ende ohnehin gesperrt.
                    if let (Some(message_id), Ok(ballot)) =
                        (voting.message_id, self.store.ballot(window_id).await)
                    {
                        let body = closed_voting_message(
                            window_id,
                            voting.week_start_ts,
                            voting.week_end_ts,
                            &ballot,
                        );
                        if let Err(err) = self
                            .port
                            .edit_message(voting.channel_id, message_id, body)
                            .await
                        {
                            tracing::warn!(%err, window_id, "Clip-Contest: Voting-Post nicht geschlossen");
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, window_id, "Clip-Contest: Ergebnis-Post fehlgeschlagen")
                }
            }
        }
        if !voting.curator_dm_sent
            && matches!(self.store.claim_curator_dm(window_id, now).await, Ok(true))
        {
            let text = curator_text(voting.week_start_ts, voting.week_end_ts, &results);
            match self
                .port
                .send_curator_text(CURATOR_USER_ID, crate::clips::SUBMIT_CHANNEL_ID, text)
                .await
            {
                Ok(()) => {
                    if let Err(err) = self.store.mark_curator_dm_sent(window_id).await {
                        tracing::warn!(%err, window_id, "Clip-Contest: Kurator-DM nicht vermerkt");
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, window_id, "Clip-Contest: Kurator-DM fehlgeschlagen")
                }
            }
        }
        match self.store.close_if_done(window_id).await {
            Ok(true) => tracing::info!(window_id, "Clip-Contest: Runde abgeschlossen"),
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(%err, window_id, "Clip-Contest: Abschluss nicht gespeichert")
            }
        }
    }

    async fn handle_vote(&self, interaction: &BridgeInteraction, window_id: i64) -> BridgeReply {
        let now = Utc::now();
        let voting = match self.store.voting(window_id).await {
            Ok(Some(voting)) => voting,
            Ok(None) => return BridgeReply::ephemeral_text("Diese Abstimmung gibt es nicht mehr."),
            Err(err) => {
                tracing::warn!(%err, window_id, "Clip-Contest: Voting nicht lesbar");
                return BridgeReply::ephemeral_text(
                    "❌ Das hat gerade nicht geklappt. Versuch es gleich nochmal.",
                );
            }
        };
        let end_ts = voting.voting_end_ts.unwrap_or_default();
        if voting.status != "open" || now.timestamp() >= end_ts {
            return BridgeReply::ephemeral_text(
                "⌛ Die Abstimmung ist schon beendet. Danke fürs Vorbeischauen!",
            );
        }
        let Some(submission_id) = interaction
            .values
            .first()
            .and_then(|value| value.parse::<i64>().ok())
        else {
            return BridgeReply::ephemeral_text("Bitte wähle einen Clip aus der Liste.");
        };
        let ballot = match self.store.ballot(window_id).await {
            Ok(ballot) => ballot,
            Err(err) => {
                tracing::warn!(%err, window_id, "Clip-Contest: Stimmzettel nicht lesbar");
                return BridgeReply::ephemeral_text(
                    "❌ Das hat gerade nicht geklappt. Versuch es gleich nochmal.",
                );
            }
        };
        let Some((index, clip)) = ballot
            .iter()
            .enumerate()
            .find(|(_, clip)| clip.submission_id == submission_id)
        else {
            return BridgeReply::ephemeral_text("Diesen Clip gibt es in dieser Abstimmung nicht.");
        };
        let twitch_ids = self
            .store
            .linked_twitch_ids(interaction.user_id)
            .await
            .unwrap_or_default();
        let joined = self
            .port
            .member_joined_at(interaction.guild_id, interaction.user_id)
            .await;
        if let Err(denial) = check_vote(
            interaction.user_id,
            &twitch_ids,
            clip,
            joined,
            now.timestamp(),
        ) {
            return BridgeReply::ephemeral_text(denial_text(&denial));
        }
        let number = index + 1;
        let credit = display_credit(clip);
        match self
            .store
            .cast_vote(window_id, interaction.user_id, submission_id, now)
            .await
        {
            Ok(VoteOutcome::Saved) => BridgeReply::ephemeral_text(format!(
                "✅ Deine Stimme für Clip {number} ({credit}) ist gespeichert. Du kannst sie bis <t:{end_ts}:f> noch ändern."
            )),
            Ok(VoteOutcome::Changed) => BridgeReply::ephemeral_text(format!(
                "🔁 Deine Stimme geht jetzt an Clip {number} ({credit}). Ändern kannst du sie bis <t:{end_ts}:f>."
            )),
            Ok(VoteOutcome::Unchanged) => BridgeReply::ephemeral_text(format!(
                "👍 Deine Stimme liegt schon bei Clip {number} ({credit})."
            )),
            Ok(VoteOutcome::Closed) => {
                BridgeReply::ephemeral_text("⌛ Die Abstimmung ist schon beendet. Danke fürs Vorbeischauen!")
            }
            Ok(VoteOutcome::NotOnBallot) => {
                BridgeReply::ephemeral_text("Diesen Clip gibt es in dieser Abstimmung nicht.")
            }
            Err(err) => {
                tracing::warn!(%err, window_id, "Clip-Contest: Stimme nicht gespeichert");
                BridgeReply::ephemeral_text("❌ Deine Stimme konnte gerade nicht gespeichert werden. Versuch es gleich nochmal.")
            }
        }
    }

    async fn handle_my_vote(&self, interaction: &BridgeInteraction, window_id: i64) -> BridgeReply {
        let vote = match self.store.my_vote(window_id, interaction.user_id).await {
            Ok(vote) => vote,
            Err(err) => {
                tracing::warn!(%err, window_id, "Clip-Contest: eigene Stimme nicht lesbar");
                return BridgeReply::ephemeral_text(
                    "❌ Das hat gerade nicht geklappt. Versuch es gleich nochmal.",
                );
            }
        };
        let Some(submission_id) = vote else {
            return BridgeReply::ephemeral_text("Du hast in dieser Runde noch nicht abgestimmt.");
        };
        let ballot = self.store.ballot(window_id).await.unwrap_or_default();
        match ballot
            .iter()
            .enumerate()
            .find(|(_, clip)| clip.submission_id == submission_id)
        {
            Some((index, clip)) => BridgeReply::ephemeral_text(format!(
                "🗳️ Deine Stimme liegt bei Clip {} ({}).",
                index + 1,
                display_credit(clip)
            )),
            None => BridgeReply::ephemeral_text("Du hast in dieser Runde noch nicht abgestimmt."),
        }
    }
}

struct VoteHandler {
    clips: Arc<ClipSubmission>,
}

#[async_trait::async_trait]
impl InteractionHandler for VoteHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let custom_id = interaction.custom_id.as_str();
        if let Some(window_id) = custom_id
            .strip_prefix(VOTE_SELECT_PREFIX)
            .and_then(|rest| rest.parse::<i64>().ok())
        {
            return self.clips.handle_vote(&interaction, window_id).await;
        }
        if let Some(window_id) = custom_id
            .strip_prefix(VOTE_MINE_PREFIX)
            .and_then(|rest| rest.parse::<i64>().ok())
        {
            return self.clips.handle_my_vote(&interaction, window_id).await;
        }
        BridgeReply::ephemeral_text("Unbekannte Aktion.")
    }
}

pub fn register(router: &mut InteractionRouter, clips: Arc<ClipSubmission>) {
    let handler = Arc::new(VoteHandler { clips });
    router.on_prefix(VOTE_SELECT_PREFIX, handler.clone());
    router.on_prefix(VOTE_MINE_PREFIX, handler);
}

#[cfg(test)]
#[path = "clip_contest_tests.rs"]
mod tests;
