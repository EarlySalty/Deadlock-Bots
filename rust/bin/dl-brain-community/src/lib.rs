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
            let (decision, reason) = if opted_out {
                (Decision::Suppressed, "opted_out".to_string())
            } else if budgeted {
                (Decision::Suppressed, "budget_14d".to_string())
            } else if let Some(kind) = anchor {
                (Decision::Yes, format!("anchor:{kind}"))
            } else {
                (Decision::No, "no_anchor".to_string())
            };

            LedgerEntry {
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
            }
        })
        .collect()
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

    let mut user_ids: Vec<i64> = candidates
        .iter()
        .map(|candidate| candidate.user_id)
        .collect();
    user_ids.sort_unstable();

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

    // Discord-Nachrichtenlimit: ID-Liste im Text kappen, vollständige Liste steht im KPI-JSON.
    const MAX_IDS_IN_TEXT: usize = 30;
    let at_risk_text = if user_ids.is_empty() {
        "Keine auffällig inaktiven Mitglieder.".to_string()
    } else {
        let shown = user_ids
            .iter()
            .take(MAX_IDS_IN_TEXT)
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let rest = user_ids.len().saturating_sub(MAX_IDS_IN_TEXT);
        if rest > 0 {
            format!(
                "{} Mitglieder rutschen gerade ab. IDs: {shown} und {rest} weitere (volle Liste im Report-JSON).",
                user_ids.len()
            )
        } else {
            format!(
                "{} Mitglieder rutschen gerade ab. IDs: {shown}",
                user_ids.len()
            )
        }
    };

    let report_text = format!(
        "**Zweitgehirn Wochenreport (Shadow-Modus)**\nEs wurde nichts gesendet, alle Entscheidungen sind nur protokolliert.\n\n**Puls**\n{pulse_text}\n\n**Abwanderungs-Kandidaten**\n{at_risk_text}\n\n**Rechenschaft**\nEntscheidungen: {accountability}\nGründe: {reasons}"
    );

    RenderedReport {
        kpis: json!({
            "pulse": pulse_json,
            "at_risk": {"count": user_ids.len(), "user_ids": user_ids},
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
