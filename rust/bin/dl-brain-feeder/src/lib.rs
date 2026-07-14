//! Reine Render-/Format-Logik des Phase-2-Feeders (Wochen-Digest).
//!
//! Kein LLM, keine Seiteneffekte: Daten-Struct rein, Markdown/Strings raus —
//! damit alles deterministisch testbar ist. DB- und Git-Zugriff liegen in main.rs.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};

/// Die sechs Entscheidungsklassen des KI-Ledgers. Der Digest zeigt IMMER alle,
/// auch mit 0 — Stille darf keine Klasse verschlucken (Judge-Regel).
pub const DECISION_CLASSES: [&str; 6] = ["yes", "no", "unsure", "timeout", "error", "suppressed"];

/// Auszug aus dem jüngsten Sonntags-Report (bot.brain_reports).
#[derive(Clone, Debug)]
pub struct BrainReportExcerpt {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub report_text: String,
}

#[derive(Clone, Debug)]
pub struct PulseRow {
    pub guild_id: i64,
    pub voice_wau: i64,
    pub text_wau: i64,
    pub new_members: i64,
    pub voice_minutes: f64,
    pub open_lfg_watches: i64,
    pub fired_lfg_watches: i64,
}

#[derive(Clone, Debug)]
pub struct LedgerAgg {
    pub source: String,
    pub decision: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct ReasonAgg {
    pub source: String,
    pub reason: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct AuditAgg {
    pub action_type: i32,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct SteamAgg {
    pub event_type: String,
    pub decision: String,
    pub count: i64,
}

#[derive(Clone, Debug, Default)]
pub struct PatchSummary {
    /// Rohe Zahl der Patch-Zeilen der Woche (Query-Input, fließt in `gesehen`).
    pub total_events: i64,
    pub patch_titles: Vec<String>,
    pub top_entities: Vec<(String, i64)>,
}

#[derive(Clone, Debug)]
pub struct SurveyRow {
    pub wave_id: i64,
    pub started_at: DateTime<Utc>,
    pub invited_count: i64,
    pub response_count: i64,
    pub response_rate: f64,
    pub average_satisfaction: Option<f64>,
}

/// Twitch stammt aus einer separaten DB; Ausfälle werden sichtbar gemacht,
/// nie still weggelassen (Judge-Regel).
#[derive(Clone, Debug)]
pub enum TwitchSection {
    Unavailable(String),
    Available {
        active_streamers: i64,
        peak_viewers: i64,
        scam: Vec<TwitchVerdict>,
        /// Crew-Radar darf separat fehlen (Tabelle evtl. nicht in Prod).
        crew: Result<Vec<(String, i64)>, String>,
    },
}

#[derive(Clone, Debug)]
pub struct TwitchVerdict {
    pub verdict: String,
    pub category: String,
    pub count: i64,
}

/// Vollständiger Eingabe-Datensatz für einen Digest-Lauf.
#[derive(Clone, Debug)]
pub struct DigestData {
    pub run_date: NaiveDate,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub brain_report: Option<BrainReportExcerpt>,
    pub pulse: Vec<PulseRow>,
    pub ledger: Vec<LedgerAgg>,
    pub reasons: Vec<ReasonAgg>,
    pub audit: Vec<AuditAgg>,
    pub steam: Vec<SteamAgg>,
    pub patch: PatchSummary,
    pub surveys: Vec<SurveyRow>,
    pub twitch: TwitchSection,
}

#[derive(Clone, Debug)]
pub struct RenderedDigest {
    pub markdown: String,
    pub iso_week: u32,
    /// Summe aller von den Queries gesehenen Roh-Zeilen/Aggregat-Inputs.
    pub gesehen: i32,
    /// Zahl der in den Digest aufgenommenen Signal-Zeilen.
    pub relevant: i32,
    pub kategorien: Vec<String>,
}

/// Ein Abschnitt mit seinen Signal-Zeilen (Bullets) und einem Platzhalter,
/// falls keine Signale vorliegen. Signal-Zeilen beginnen immer mit "- ".
struct Section {
    title: &'static str,
    lines: Vec<String>,
    placeholder: &'static str,
}

impl Section {
    fn has_content(&self) -> bool {
        !self.lines.is_empty()
    }

    fn render(&self) -> String {
        let body = if self.lines.is_empty() {
            self.placeholder.to_string()
        } else {
            self.lines.join("\n")
        };
        format!("## {}\n\n{}\n", self.title, body)
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max).collect();
        format!("{head}…")
    }
}

fn fmt_satisfaction(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_string(), |v| format!("{v:.2}"))
}

fn community_pulse(data: &DigestData) -> Section {
    let mut lines = Vec::new();
    for pulse in &data.pulse {
        lines.push(format!(
            "- Guild {}: Voice-WAU {}, Text-WAU {}, neue Mitglieder {}, Voice-Minuten {:.0}, LFG offen {} / erfüllt {}",
            pulse.guild_id,
            pulse.voice_wau,
            pulse.text_wau,
            pulse.new_members,
            pulse.voice_minutes,
            pulse.open_lfg_watches,
            pulse.fired_lfg_watches,
        ));
    }
    if let Some(report) = &data.brain_report {
        let excerpt = truncate_chars(&report.report_text.replace('\n', " "), 800);
        lines.push(format!(
            "- Report-Auszug ({} – {}): {}",
            report.period_start.date_naive(),
            report.period_end.date_naive(),
            excerpt,
        ));
    }
    Section {
        title: "Community-Puls",
        lines,
        placeholder: "Keine Puls-Daten und kein Report für diesen Zeitraum.",
    }
}

fn ki_accountability(data: &DigestData) -> Section {
    // Gesamt-Zeile über alle Quellen: jede der 6 Klassen erscheint, auch 0.
    let mut totals: BTreeMap<&str, i64> = DECISION_CLASSES.iter().map(|c| (*c, 0)).collect();
    for entry in &data.ledger {
        if let Some(count) = totals.get_mut(entry.decision.as_str()) {
            *count += entry.count;
        }
    }
    let gesamt = DECISION_CLASSES
        .iter()
        .map(|class| format!("{class}={}", totals[class]))
        .collect::<Vec<_>>()
        .join(" ");

    let mut lines = vec![format!("- Gesamt: {gesamt}")];

    // Pro Quelle die volle Klassen-Reihe (0 für fehlende Klassen).
    let mut per_source: BTreeMap<&str, BTreeMap<&str, i64>> = BTreeMap::new();
    for entry in &data.ledger {
        let row = per_source
            .entry(entry.source.as_str())
            .or_insert_with(|| DECISION_CLASSES.iter().map(|c| (*c, 0)).collect());
        if let Some(count) = row.get_mut(entry.decision.as_str()) {
            *count += entry.count;
        }
    }
    for (source, row) in &per_source {
        let classes = DECISION_CLASSES
            .iter()
            .map(|class| format!("{class}={}", row[class]))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("- {source}: {classes}"));
    }

    // Top-Gründe je Quelle (max. 5 kommen bereits aus der Query).
    let mut reasons_by_source: BTreeMap<&str, Vec<&ReasonAgg>> = BTreeMap::new();
    for reason in &data.reasons {
        reasons_by_source
            .entry(reason.source.as_str())
            .or_default()
            .push(reason);
    }
    for (source, reasons) in &reasons_by_source {
        let joined = reasons
            .iter()
            .map(|r| format!("{}={}", r.reason, r.count))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("- {source} Top-Gründe: {joined}"));
    }

    Section {
        title: "KI-Rechenschaft",
        lines,
        // Unerreichbar (Gesamt-Zeile steht immer), aber als Vertrag definiert.
        placeholder: "Keine KI-Entscheidungen protokolliert.",
    }
}

fn discord_moderation(data: &DigestData) -> Section {
    let lines = data
        .audit
        .iter()
        .map(|agg| format!("- action_type {}: {}", agg.action_type, agg.count))
        .collect();
    Section {
        title: "Discord-Moderation",
        lines,
        placeholder: "Keine Audit-Log-Einträge in dieser Woche.",
    }
}

fn steam_section(data: &DigestData) -> Section {
    let lines = data
        .steam
        .iter()
        .map(|agg| format!("- {} / {}: {}", agg.event_type, agg.decision, agg.count))
        .collect();
    Section {
        title: "Steam",
        lines,
        placeholder: "Keine Steam-Bot-Ereignisse in dieser Woche.",
    }
}

fn spiel_meta(data: &DigestData) -> Section {
    let mut lines = Vec::new();
    if data.patch.total_events > 0 || !data.patch.patch_titles.is_empty() {
        let titles = if data.patch.patch_titles.is_empty() {
            "(ohne Titel)".to_string()
        } else {
            data.patch.patch_titles.join(", ")
        };
        lines.push(format!(
            "- {} Patch-Zeilen in {} Patch(es): {}",
            data.patch.total_events,
            data.patch.patch_titles.len(),
            titles,
        ));
        for (name, count) in &data.patch.top_entities {
            lines.push(format!("- {name}: {count} Änderungen"));
        }
    }
    Section {
        title: "Spiel-Meta",
        lines,
        placeholder: "Keine Patch-Änderungen in dieser Woche.",
    }
}

fn umfragen(data: &DigestData) -> Section {
    let lines = data
        .surveys
        .iter()
        .map(|wave| {
            format!(
                "- Welle {} ({}): eingeladen {}, Antworten {}, Rate {:.0}%, Ø-Zufriedenheit {}",
                wave.wave_id,
                wave.started_at.date_naive(),
                wave.invited_count,
                wave.response_count,
                wave.response_rate * 100.0,
                fmt_satisfaction(wave.average_satisfaction),
            )
        })
        .collect();
    Section {
        title: "Umfragen",
        lines,
        placeholder: "keine Welle in dieser Woche.",
    }
}

fn twitch(data: &DigestData) -> Section {
    let lines = match &data.twitch {
        TwitchSection::Unavailable(reason) => {
            vec![format!("- Quelle nicht verfügbar ({reason})")]
        }
        TwitchSection::Available {
            active_streamers,
            peak_viewers,
            scam,
            crew,
        } => {
            let mut lines = vec![format!(
                "- Aktive Streamer: {active_streamers}, Peak-Viewer: {peak_viewers}"
            )];
            for verdict in scam {
                lines.push(format!(
                    "- Scam-Verdict {}/{}: {}",
                    verdict.verdict, verdict.category, verdict.count
                ));
            }
            match crew {
                Ok(rows) => {
                    for (verdict, count) in rows {
                        lines.push(format!("- Crew-Radar {verdict}: {count}"));
                    }
                }
                Err(reason) => {
                    lines.push(format!("- Crew-Radar: Quelle nicht verfügbar ({reason})"));
                }
            }
            lines
        }
    };
    Section {
        title: "Twitch",
        lines,
        // Twitch hat immer Inhalt (Aggregate oder sichtbarer Ausfall).
        placeholder: "Twitch: keine Daten.",
    }
}

/// Roh-Zeilen, die die Queries gesehen haben (fließt in den Pflicht-Kopf).
fn count_gesehen(data: &DigestData) -> i32 {
    let twitch_seen = match &data.twitch {
        TwitchSection::Unavailable(_) => 0,
        TwitchSection::Available { scam, crew, .. } => {
            // +1 für die Streamer-/Peak-Kennzahl.
            1 + scam.len() + crew.as_ref().map(Vec::len).unwrap_or(0)
        }
    };
    i32::from(data.brain_report.is_some())
        + data.pulse.len() as i32
        + data.ledger.len() as i32
        + data.audit.len() as i32
        + data.steam.len() as i32
        + data.patch.total_events.max(0) as i32
        + data.surveys.len() as i32
        + twitch_seen as i32
}

/// Ersetzt Läufe von 17–20 Ziffern (Discord-Snowflakes) durch einen Marker.
/// Root-Cause-Guard über den GESAMTEN Digest: egal welcher Abschnitt eine
/// User-ID durchreicht, sie verlässt den Feeder nie (DSA-Löschpfad-Regel).
pub fn scrub_snowflakes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut digits = String::new();
    for ch in input.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            flush_digits(&mut digits, &mut out);
            out.push(ch);
        }
    }
    flush_digits(&mut digits, &mut out);
    out
}

fn flush_digits(digits: &mut String, out: &mut String) {
    if !digits.is_empty() {
        if (17..=20).contains(&digits.len()) {
            out.push_str("[id-entfernt]");
        } else {
            out.push_str(digits);
        }
        digits.clear();
    }
}

/// Baut den kompletten Wochen-Digest (Markdown) plus die Kopf-Kennzahlen.
pub fn render_digest(data: &DigestData) -> RenderedDigest {
    let iso_week = data.run_date.iso_week().week();

    let sections = [
        community_pulse(data),
        ki_accountability(data),
        discord_moderation(data),
        steam_section(data),
        spiel_meta(data),
        umfragen(data),
        twitch(data),
    ];

    let gesehen = count_gesehen(data);
    let relevant: i32 = sections.iter().map(|s| s.lines.len() as i32).sum();
    let kategorien: Vec<String> = sections
        .iter()
        .filter(|s| s.has_content())
        .map(|s| s.title.to_string())
        .collect();
    let kategorien_text = if kategorien.is_empty() {
        "keine".to_string()
    } else {
        kategorien.join(", ")
    };

    let mut body = String::new();
    body.push_str(&format!(
        "# Wochen-Digest KW{iso_week} — {}\n\n",
        data.run_date.format("%Y-%m-%d")
    ));
    body.push_str(&format!(
        "gesehen {gesehen} / relevant {relevant} / Kategorien: {kategorien_text}\n\n"
    ));
    body.push_str(&format!(
        "Zeitraum: {} bis {}\n\n",
        data.period_start.to_rfc3339(),
        data.period_end.to_rfc3339(),
    ));
    for section in &sections {
        body.push_str(&section.render());
        body.push('\n');
    }
    // Quellen als Verweis, keine Datenkopie (Wiki-Regel).
    body.push_str(
        "---\nQuellen (nur Verweise, keine Kopien): bot.brain_reports, \
         bot.ai_decision_ledger, core.discord_audit_log, steam.bot_event_log, \
         brain.patch_events, bot.survey_wave_summary, activity.weekly_pulse, \
         twitch: twitch_stats_tracked / twitch_scam_guard_verdicts / twitch_crew_radar_log.\n",
    );

    let markdown = scrub_snowflakes(&body);

    RenderedDigest {
        markdown,
        iso_week,
        gesehen,
        relevant,
        kategorien,
    }
}

/// Baut die log.md-Zeile im Vertragsformat aus test_wiki.py:
/// `## [JJJJ-MM-TT] feed — Wochen-Digest KW<n>` (Typ 'feed', em-dash).
pub fn log_entry(
    run_date: NaiveDate,
    iso_week: u32,
    gesehen: i32,
    relevant: i32,
    besonderheit: &str,
) -> String {
    format!(
        "## [{}] feed — Wochen-Digest KW{iso_week}\n\ngesehen {gesehen} / relevant {relevant}. {besonderheit}\n",
        run_date.format("%Y-%m-%d"),
    )
}

/// Relativer Pfad des Digests; hängt bei Kollision -2, -3 … an (immutable-Regel).
/// `exists` prüft die Existenz eines relativen Pfads (repo-relativ).
pub fn pick_digest_relpath(date: NaiveDate, exists: impl Fn(&str) -> bool) -> String {
    let dir = date.format("%Y-%m");
    let base = date.format("%Y-%m-%d");
    let first = format!("raw/{dir}/{base}-wochen-digest.md");
    if !exists(&first) {
        return first;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("raw/{dir}/{base}-wochen-digest-{suffix}.md");
        if !exists(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use chrono::TimeZone;
    use regex::Regex;

    fn ts(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0).unwrap()
    }

    fn sample(twitch: TwitchSection, report_text: &str) -> DigestData {
        DigestData {
            run_date: NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            period_start: ts(7, 19),
            period_end: ts(14, 19),
            brain_report: Some(BrainReportExcerpt {
                period_start: ts(7, 18),
                period_end: ts(14, 18),
                report_text: report_text.to_string(),
            }),
            pulse: vec![PulseRow {
                guild_id: 42,
                voice_wau: 10,
                text_wau: 20,
                new_members: 3,
                voice_minutes: 123.4,
                open_lfg_watches: 2,
                fired_lfg_watches: 1,
            }],
            ledger: vec![
                LedgerAgg {
                    source: "brain.activation".into(),
                    decision: "no".into(),
                    count: 5,
                },
                LedgerAgg {
                    source: "brain.activation".into(),
                    decision: "yes".into(),
                    count: 1,
                },
                LedgerAgg {
                    source: "moderation".into(),
                    decision: "error".into(),
                    count: 2,
                },
            ],
            reasons: vec![ReasonAgg {
                source: "brain.activation".into(),
                reason: "no_anchor".into(),
                count: 5,
            }],
            audit: vec![
                AuditAgg {
                    action_type: 22,
                    count: 4,
                },
                AuditAgg {
                    action_type: 25,
                    count: 1,
                },
            ],
            steam: vec![SteamAgg {
                event_type: "friend_add".into(),
                decision: "ok".into(),
                count: 7,
            }],
            patch: PatchSummary {
                total_events: 4,
                patch_titles: vec!["Update 2026-07-10".into()],
                top_entities: vec![("Abrams".into(), 3)],
            },
            surveys: vec![SurveyRow {
                wave_id: 9,
                started_at: ts(10, 12),
                invited_count: 50,
                response_count: 20,
                response_rate: 0.4,
                average_satisfaction: Some(4.25),
            }],
            twitch,
        }
    }

    fn available_twitch() -> TwitchSection {
        TwitchSection::Available {
            active_streamers: 12,
            peak_viewers: 340,
            scam: vec![
                TwitchVerdict {
                    verdict: "scam".into(),
                    category: "phishing".into(),
                    count: 2,
                },
                TwitchVerdict {
                    verdict: "clean".into(),
                    category: "-".into(),
                    count: 8,
                },
            ],
            crew: Ok(vec![("suspicious".into(), 1)]),
        }
    }

    fn count_bullets(markdown: &str) -> i32 {
        markdown.lines().filter(|l| l.starts_with("- ")).count() as i32
    }

    /// true, wenn irgendein *maximaler* Ziffernlauf eine Länge im Bereich hat.
    fn max_digit_run_in(range: std::ops::RangeInclusive<usize>, text: &str) -> bool {
        let mut run = 0usize;
        for ch in text.chars().chain(std::iter::once(' ')) {
            if ch.is_ascii_digit() {
                run += 1;
            } else {
                if range.contains(&run) {
                    return true;
                }
                run = 0;
            }
        }
        false
    }

    #[test]
    fn header_present_and_counts_are_consistent() {
        let data = sample(available_twitch(), "Puls ok, alles ruhig.");
        let rendered = render_digest(&data);

        // gesehen = report(1)+pulse(1)+ledger(3)+audit(2)+steam(1)
        //         + patch.total(4)+surveys(1)+twitch(streamer 1 + scam 2 + crew 1 = 4) = 17
        assert_eq!(rendered.gesehen, 17, "gesehen falsch berechnet");

        // relevant muss exakt der Zahl der Signal-Bullets entsprechen.
        assert_eq!(
            rendered.relevant,
            count_bullets(&rendered.markdown),
            "relevant weicht von den Bullet-Zeilen ab"
        );

        let header = format!(
            "gesehen {} / relevant {} / Kategorien: ",
            rendered.gesehen, rendered.relevant
        );
        assert!(
            rendered.markdown.contains(&header),
            "Pflicht-Kopf-Zeile fehlt/falsch: {}",
            rendered
                .markdown
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join(" | ")
        );
        assert!(rendered.markdown.contains(&format!(
            "# Wochen-Digest KW{} — 2026-07-14",
            rendered.iso_week
        )));
    }

    #[test]
    fn header_present_even_with_no_signals() {
        // Alles leer außer der immer vorhandenen KI-Gesamt-Zeile.
        let data = DigestData {
            run_date: NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            period_start: ts(7, 19),
            period_end: ts(14, 19),
            brain_report: None,
            pulse: vec![],
            ledger: vec![],
            reasons: vec![],
            audit: vec![],
            steam: vec![],
            patch: PatchSummary::default(),
            surveys: vec![],
            twitch: TwitchSection::Unavailable("DSN nicht gesetzt".into()),
        };
        let rendered = render_digest(&data);
        assert!(rendered.markdown.contains("gesehen 0 / relevant "));
        // KI-Gesamt (1) + Twitch-Ausfall (1) sind die einzigen Signale.
        assert_eq!(rendered.relevant, 2);
        assert_eq!(rendered.relevant, count_bullets(&rendered.markdown));
    }

    #[test]
    fn every_decision_class_appears_even_at_zero() {
        let data = sample(available_twitch(), "x");
        let rendered = render_digest(&data);
        // Gesamt-Zeile enthält alle sechs Klassen mit ihren Werten.
        for class in DECISION_CLASSES {
            assert!(
                rendered.markdown.contains(&format!("{class}=")),
                "Entscheidungsklasse {class} fehlt im Digest"
            );
        }
        // Konkret: unsure/timeout/suppressed haben 0 und stehen trotzdem drin.
        assert!(rendered.markdown.contains("unsure=0"));
        assert!(rendered.markdown.contains("timeout=0"));
        assert!(rendered.markdown.contains("suppressed=0"));
    }

    #[test]
    fn twitch_outage_produces_visible_section() {
        let data = sample(
            TwitchSection::Unavailable("Verbindung abgelehnt".into()),
            "x",
        );
        let rendered = render_digest(&data);
        assert!(rendered.markdown.contains("## Twitch"));
        assert!(rendered
            .markdown
            .contains("Quelle nicht verfügbar (Verbindung abgelehnt)"));
        assert!(rendered.kategorien.iter().any(|k| k == "Twitch"));
    }

    #[test]
    fn crew_radar_outage_tolerated_but_visible() {
        let twitch = TwitchSection::Available {
            active_streamers: 5,
            peak_viewers: 100,
            scam: vec![],
            crew: Err("relation does not exist".into()),
        };
        let rendered = render_digest(&sample(twitch, "x"));
        assert!(rendered
            .markdown
            .contains("Crew-Radar: Quelle nicht verfügbar (relation does not exist)"));
    }

    #[test]
    fn digest_never_leaks_discord_snowflakes() {
        let snowflake = "123456789012345678"; // 18 Stellen
        let report = format!("Nutzer {snowflake} war aktiv");
        let rendered = render_digest(&sample(available_twitch(), &report));
        assert!(
            !rendered.markdown.contains(snowflake),
            "Snowflake-ID im Digest durchgesickert"
        );
        assert!(rendered.markdown.contains("[id-entfernt]"));

        // Kein maximaler 17–20-stelliger Ziffernlauf überlebt irgendwo im Markdown.
        assert!(
            !max_digit_run_in(17..=20, &rendered.markdown),
            "Snowflake-Muster im Digest gefunden"
        );
    }

    #[test]
    fn scrub_keeps_short_and_long_numbers() {
        // 16 Stellen bleiben (z. B. kein Snowflake), 21 bleiben (kein Snowflake).
        assert_eq!(scrub_snowflakes("1234567890123456"), "1234567890123456");
        assert_eq!(
            scrub_snowflakes("123456789012345678901"),
            "123456789012345678901"
        );
        assert_eq!(scrub_snowflakes("id=12345678901234567"), "id=[id-entfernt]");
    }

    #[test]
    fn log_entry_matches_wiki_contract_regex() {
        let entry = log_entry(
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
            29,
            17,
            9,
            "Twitch verfügbar.",
        );
        // Regex aus test_wiki.py nachgebaut.
        let re = Regex::new(r"(?m)^## \[\d{4}-\d{2}-\d{2}\] (?:ingest|query|lint|feed|setup) — ")
            .unwrap();
        assert!(
            re.is_match(&entry),
            "log-Eintrag verletzt Wiki-Vertrag: {entry}"
        );
        assert!(entry.starts_with("## [2026-07-14] feed — Wochen-Digest KW29"));
    }

    #[test]
    fn digest_path_appends_suffix_on_collision() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 14).unwrap();
        let taken = |p: &str| {
            p == "raw/2026-07/2026-07-14-wochen-digest.md"
                || p == "raw/2026-07/2026-07-14-wochen-digest-2.md"
        };
        assert_eq!(
            pick_digest_relpath(date, taken),
            "raw/2026-07/2026-07-14-wochen-digest-3.md"
        );
        assert_eq!(
            pick_digest_relpath(date, |_| false),
            "raw/2026-07/2026-07-14-wochen-digest.md"
        );
    }
}
