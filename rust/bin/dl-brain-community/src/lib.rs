use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

pub const DECISION_CLASSES: [Decision; 6] = [
    Decision::Yes,
    Decision::No,
    Decision::Unsure,
    Decision::Timeout,
    Decision::Error,
    Decision::Suppressed,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Yes,
    No,
    Unsure,
    Timeout,
    Error,
    Suppressed,
}

impl Decision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unsure => "unsure",
            Self::Timeout => "timeout",
            Self::Error => "error",
            Self::Suppressed => "suppressed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub user_id: i64,
    pub guild_id: i64,
    pub inactive_days: i32,
}

#[derive(Debug, Default)]
pub struct GateData {
    pub opted_out_users: HashSet<i64>,
    pub budgeted_users: HashSet<i64>,
    pub anchors: HashMap<i64, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LedgerEntry {
    pub source: &'static str,
    pub subject_user_id: Option<i64>,
    pub guild_id: Option<i64>,
    pub input_summary: String,
    pub decision: Decision,
    pub confidence: Option<f32>,
    pub reason: String,
    pub action_taken: &'static str,
    pub payload: Value,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct WeeklyPulse {
    pub guild_id: i64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub voice_wau: i64,
    pub text_wau: i64,
    pub new_members: i64,
    pub open_lfg_watches: i64,
    pub fired_lfg_watches: i64,
    pub voice_minutes: f64,
}

#[derive(Debug)]
pub struct RenderedReport {
    pub kpis: Value,
    pub report_text: String,
}

/// Fail-closed-Ausgang, wenn die Gate-Daten (Opt-out/Budget) nicht ladbar sind:
/// keine Kandidaten-IDs in Report oder Ledger, nur ein anonymer Fehler-Eintrag.
pub fn gate_failure_outcome(reason: &str, candidate_count: usize) -> Vec<LedgerEntry> {
    vec![LedgerEntry {
        source: "brain.activation",
        subject_user_id: None,
        guild_id: None,
        input_summary: format!("load={reason};candidates_dropped={candidate_count}"),
        decision: Decision::Error,
        confidence: None,
        reason: reason.to_string(),
        action_taken: "shadow",
        payload: json!({}),
    }]
}

pub fn decide(candidates: &[Candidate], gates: &GateData) -> Vec<LedgerEntry> {
    candidates
        .iter()
        .map(|candidate| {
            let opted_out = gates.opted_out_users.contains(&candidate.user_id);
            let budgeted = gates.budgeted_users.contains(&candidate.user_id);
            let anchor = gates.anchors.get(&candidate.user_id);

            let (decision, reason) = if budgeted {
                (Decision::Suppressed, "budget_14d".to_string())
            } else if let Some(kind) = anchor {
                (Decision::Yes, format!("anchor:{kind}"))
            } else {
                (Decision::No, "no_anchor".to_string())
            };

            let entry = LedgerEntry {
                source: "brain.activation",
                subject_user_id: Some(candidate.user_id),
                guild_id: Some(candidate.guild_id),
                input_summary: format!(
                    "inactive_days={};opted_out={opted_out};budget_14d={budgeted};anchor={}",
                    candidate.inactive_days,
                    anchor.map_or("none", String::as_str)
                ),
                decision,
                confidence: None,
                reason,
                action_taken: "shadow",
                payload: json!({
                    "inactive_days": candidate.inactive_days,
                    "anchor": anchor,
                }),
            };

            // Opt-out/gelöschte Nutzer: kein nutzerbezogener Write — nur die
            // Zählung bleibt sichtbar.
            if opted_out {
                anonymize_for_privacy(&entry)
            } else {
                entry
            }
        })
        .collect()
}

/// Entfernt alle nutzerbezogenen Daten aus einem Ledger-Eintrag; wird sowohl
/// vom Opt-out-Gate in `decide()` als auch vom Recheck unter dem
/// Privacy-Lock direkt vor dem Insert benutzt.
pub fn anonymize_for_privacy(entry: &LedgerEntry) -> LedgerEntry {
    LedgerEntry {
        source: entry.source,
        subject_user_id: None,
        guild_id: entry.guild_id,
        input_summary: "anonymisiert:opted_out".to_string(),
        decision: Decision::Suppressed,
        confidence: None,
        reason: "opted_out".to_string(),
        action_taken: entry.action_taken,
        payload: json!({}),
    }
}

pub fn render_report(
    pulses: &[WeeklyPulse],
    candidates: &[Candidate],
    decisions: &[LedgerEntry],
) -> RenderedReport {
    let mut by_decision = BTreeMap::new();
    for class in DECISION_CLASSES {
        by_decision.insert(class.as_str(), 0_u64);
    }
    let mut by_reason = BTreeMap::new();
    for entry in decisions {
        if let Some(count) = by_decision.get_mut(entry.decision.as_str()) {
            *count += 1;
        }
        *by_reason.entry(entry.reason.as_str()).or_insert(0_u64) += 1;
    }

    let pulse_json: Vec<Value> = pulses
        .iter()
        .map(|pulse| {
            json!({
                "guild_id": pulse.guild_id,
                "period_start": pulse.period_start.to_rfc3339(),
                "period_end": pulse.period_end.to_rfc3339(),
                "voice_wau": pulse.voice_wau,
                "text_wau": pulse.text_wau,
                "new_members": pulse.new_members,
                "open_lfg_watches": pulse.open_lfg_watches,
                "fired_lfg_watches": pulse.fired_lfg_watches,
                "voice_minutes": pulse.voice_minutes,
                "delta": null,
            })
        })
        .collect();

    let accountability = by_decision
        .iter()
        .map(|(class, count)| format!("{class}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    let reasons = if by_reason.is_empty() {
        "keine".to_string()
    } else {
        by_reason
            .iter()
            .map(|(reason, count)| format!("{reason}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    };

    let pulse_text = if pulses.is_empty() {
        "Keine Puls-Daten für diesen Zeitraum.".to_string()
    } else {
        pulses
            .iter()
            .map(|pulse| {
                format!(
                    "Guild {}: Voice-WAU {}, Text-WAU {}, neue Mitglieder {}, Voice-Minuten {:.0}, LFG offen {} / erfüllt {}",
                    pulse.guild_id,
                    pulse.voice_wau,
                    pulse.text_wau,
                    pulse.new_members,
                    pulse.voice_minutes,
                    pulse.open_lfg_watches,
                    pulse.fired_lfg_watches,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Keine rohen User-IDs im Report (Text oder JSON): der Report wird persistiert
    // und läge damit außerhalb des DSA-Löschpfads. IDs sind löschbar im Ledger
    // (bot.ai_decision_ledger, source 'brain.activation') abfragbar.
    let at_risk_text = if candidates.is_empty() {
        "Keine auffällig inaktiven Mitglieder.".to_string()
    } else {
        format!(
            "{} Mitglieder rutschen gerade ab. IDs stehen im Entscheidungs-Ledger (bot.ai_decision_ledger, source brain.activation, aktueller Lauf).",
            candidates.len()
        )
    };

    let report_text = format!(
        "**Zweitgehirn Wochenreport (Shadow-Modus)**\nEs wurde nichts gesendet, alle Entscheidungen sind nur protokolliert.\n\n**Puls**\n{pulse_text}\n\n**Abwanderungs-Kandidaten**\n{at_risk_text}\n\n**Rechenschaft**\nEntscheidungen: {accountability}\nGründe: {reasons}"
    );

    RenderedReport {
        kpis: json!({
            "pulse": pulse_json,
            "at_risk": {"count": candidates.len()},
            "accountability": {
                "by_decision": by_decision,
                "by_reason": by_reason,
            },
        }),
        report_text,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    fn candidate(user_id: i64) -> Candidate {
        Candidate {
            user_id,
            guild_id: 7,
            inactive_days: 10,
        }
    }

    #[test]
    fn decide_applies_opt_out_then_budget_then_anchor() {
        let candidates = vec![candidate(1), candidate(2), candidate(3), candidate(4)];
        let gates = GateData {
            opted_out_users: HashSet::from([1]),
            budgeted_users: HashSet::from([1, 2]),
            anchors: HashMap::from([
                (1, "event".to_string()),
                (2, "event".to_string()),
                (3, "lfg_slot".to_string()),
            ]),
        };

        let decisions = decide(&candidates, &gates);

        assert_eq!(decisions[0].decision, Decision::Suppressed);
        assert_eq!(decisions[0].reason, "opted_out");
        // Opt-out-Nutzer werden anonymisiert geledgert, nie mit ID.
        assert_eq!(decisions[0].subject_user_id, None);
        assert_eq!(decisions[0].input_summary, "anonymisiert:opted_out");
        assert_eq!(decisions[1].decision, Decision::Suppressed);
        assert_eq!(decisions[1].reason, "budget_14d");
        assert_eq!(decisions[2].decision, Decision::Yes);
        assert_eq!(decisions[2].reason, "anchor:lfg_slot");
        assert_eq!(decisions[3].decision, Decision::No);
        assert_eq!(decisions[3].reason, "no_anchor");
    }

    #[test]
    fn decide_returns_exactly_one_ledger_entry_per_candidate() {
        let candidates = vec![candidate(11), candidate(12), candidate(13)];

        let decisions = decide(&candidates, &GateData::default());

        assert_eq!(decisions.len(), candidates.len());
        assert_eq!(
            decisions
                .iter()
                .map(|entry| entry.subject_user_id)
                .collect::<Vec<_>>(),
            vec![Some(11), Some(12), Some(13)]
        );
    }

    #[test]
    fn anonymize_strips_every_user_reference() {
        let entries = decide(&[candidate(99)], &GateData::default());
        let anonymized = anonymize_for_privacy(&entries[0]);

        assert_eq!(anonymized.subject_user_id, None);
        assert_eq!(anonymized.decision, Decision::Suppressed);
        assert_eq!(anonymized.reason, "opted_out");
        assert_eq!(anonymized.input_summary, "anonymisiert:opted_out");
        assert_eq!(anonymized.payload, json!({}));
    }

    #[test]
    fn report_never_contains_raw_user_ids() {
        let subject = 123_456_789_012_345_678_i64;
        let candidates = vec![Candidate {
            user_id: subject,
            guild_id: 7,
            inactive_days: 10,
        }];
        let decisions = decide(&candidates, &GateData::default());

        let report = render_report(&[], &candidates, &decisions);

        // Der Report wird persistiert und liegt damit außerhalb des
        // DSA-Löschpfads: rohe User-IDs dürfen nie hinein.
        assert!(!report.report_text.contains(&subject.to_string()));
        assert!(!report.kpis.to_string().contains(&subject.to_string()));
        assert_eq!(report.kpis["at_risk"]["count"].as_u64(), Some(1));
    }

    #[test]
    fn gate_failure_is_fail_closed_and_leaks_no_ids() {
        let entries = gate_failure_outcome("load_opted_out", 5);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].decision, Decision::Error);
        assert_eq!(entries[0].subject_user_id, None);
        assert_eq!(entries[0].guild_id, None);

        // Report ohne Kandidaten darf keine einzige User-ID enthalten.
        let report = render_report(&[], &[], &entries);
        assert!(report
            .report_text
            .contains("Keine auffällig inaktiven Mitglieder."));
        assert_eq!(report.kpis["at_risk"]["count"].as_u64(), Some(0));
        assert_eq!(
            report.kpis["accountability"]["by_decision"]["error"].as_u64(),
            Some(1)
        );
    }

    #[test]
    fn report_renderer_always_lists_every_decision_class() {
        let decisions = decide(&[candidate(21)], &GateData::default());

        let report = render_report(&[], &[candidate(21)], &decisions);

        for (class, expected) in [
            ("yes", 0),
            ("no", 1),
            ("unsure", 0),
            ("timeout", 0),
            ("error", 0),
            ("suppressed", 0),
        ] {
            assert!(
                report.report_text.contains(&format!("{class}=")),
                "missing decision class {class}"
            );
            assert_eq!(
                report.kpis["accountability"]["by_decision"][class].as_u64(),
                Some(expected)
            );
        }
    }
}
