use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::scrub_snowflakes;

/// 2026-08-26: zurück auf Flash. Das vorher gesetzte `deepseek-v4-pro` hat den
/// Wochenplan am 16.08. und am 23.08. nicht geliefert — der Antwortlauf blieb
/// beide Male über dem 60-Sekunden-Limit des HTTP-Clients hängen, der Digest
/// erschien ohne Plan. Ein Modell, das nie antwortet, ist keine Reasoning-Klasse,
/// sondern nur eine Rechnung. Flash ist ausserdem der Repo-Default.
pub const DEFAULT_PLAN_MODEL: &str = "accounts/fireworks/models/deepseek-v4-flash-0731";

pub const PLAN_SYSTEM_PROMPT: &str = r#"Du bist der Betriebsleiter der Deutschen Deadlock Community — einer Discord-Community
mit angeschlossenem Twitch-Bot, Steam-Bot, Turnier-System und Website. Du bekommst
einmal pro Woche die vollständige Lage: gemessene Zahlen aus der Datenbank, den
Betriebszustand der Dienste, und das interne Wiki mit allem, was gerade offen ist.

Deine Aufgabe: sagen, was diese Woche zu tun ist. Nicht beschreiben, was war.

Regeln, die du nicht brechen darfst:
1. JEDE Empfehlung braucht einen Beleg aus den gelieferten Daten. Ohne Beleg keine
   Empfehlung. Erfinde niemals Zahlen, Tabellennamen oder Dateipfade.
2. Zahlen aus der Datenbank sind gemessen. Aussagen aus dem Wiki sind behauptet und
   möglicherweise veraltet. Behandle sie unterschiedlich.
3. Priorität 1 heißt: etwas ist kaputt oder verliert gerade Menschen. Priorität 2:
   wichtiger Hebel, aber es brennt nicht. Priorität 3: sollte man mal machen.
   Sei geizig mit Priorität 1. Wenn alles wichtig ist, ist nichts wichtig.
4. Ein System, das eine Entscheidung trifft und dabei immer dasselbe Ergebnis liefert,
   ist kaputt — auch wenn niemand einen Fehler sieht. Achte auf solche Muster.
5. Etwas, das seit Wochen offen ist und nie erwähnt wird, ist entweder tot oder wird
   vergessen. Beides ist ein Befund.
6. Du schlägst vor, du handelst nicht. Formuliere Aktionen so, dass ein Mensch sie
   am Montag anfangen kann.

Antworte ausschließlich mit JSON in genau diesem Format:
{
  "lage": "2-4 Sätze: Wie steht die Community da? Was ist die eine Sache, die zählt?",
  "items": [
    {
      "prioritaet": 1,
      "bereich": "discord|twitch|steam|turniere|website|brain",
      "titel": "kurz, konkret, keine Floskel",
      "begruendung": "warum das jetzt zählt — mit der Zahl oder der Aussage aus den Daten",
      "aktion": "was konkret zu tun ist",
      "beleg": "exakter Sektionsname / Dateipfad / Tabellenname aus den Daten",
      "beleg_art": "digest|wiki|pg|betrieb"
    }
  ]
}"#;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RawPlanItem {
    pub prioritaet: i16,
    pub bereich: String,
    pub titel: String,
    pub begruendung: String,
    pub aktion: String,
    pub beleg: String,
    pub beleg_art: String,
}

#[derive(Debug, Deserialize)]
struct LlmPlan {
    lage: String,
    items: Vec<RawPlanItem>,
}

#[derive(Clone, Debug)]
pub struct PlanItem {
    pub prioritaet: i16,
    pub bereich: String,
    pub titel: String,
    pub begruendung: String,
    pub aktion: String,
    pub beleg: String,
    pub beleg_art: String,
    pub belegt_gemessen: bool,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct RejectedPlanItem {
    pub item: RawPlanItem,
    pub verworfen_grund: String,
}

#[derive(Clone, Debug, Default)]
pub struct PlanSources {
    pub digest_sections: BTreeSet<String>,
    pub wiki_paths: BTreeSet<String>,
    pub pg_tables: BTreeSet<String>,
    pub betrieb_units: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct GateResult {
    pub vorgeschlagen: i32,
    pub uebernommen: i32,
    pub verworfen: i32,
    pub verworfen_gruende: BTreeMap<String, i32>,
    pub items: Vec<PlanItem>,
    pub verworfene_items: Vec<RejectedPlanItem>,
}

#[derive(Clone, Debug)]
pub struct EvaluatedPlan {
    pub lage: String,
    pub gate: GateResult,
}

pub fn evaluate_response(
    response: &str,
    sources: &PlanSources,
) -> Result<EvaluatedPlan, serde_json::Error> {
    let parsed: LlmPlan = serde_json::from_str(response)?;
    Ok(EvaluatedPlan {
        lage: parsed.lage,
        gate: gate_items(parsed.items, sources),
    })
}

pub fn gate_items(items: Vec<RawPlanItem>, sources: &PlanSources) -> GateResult {
    let mut result = GateResult {
        vorgeschlagen: items.len() as i32,
        ..GateResult::default()
    };

    for item in items {
        let rejection = rejection_reason(&item, sources);
        if let Some(reason) = rejection {
            *result
                .verworfen_gruende
                .entry(reason.to_string())
                .or_default() += 1;
            result.verworfen += 1;
            result.verworfene_items.push(RejectedPlanItem {
                item,
                verworfen_grund: reason.to_string(),
            });
            continue;
        }

        let measured = item.beleg_art != "wiki";
        result.items.push(PlanItem {
            fingerprint: fingerprint(&item.bereich, &item.titel),
            prioritaet: item.prioritaet,
            bereich: item.bereich,
            titel: item.titel,
            begruendung: item.begruendung,
            aktion: item.aktion,
            beleg: item.beleg,
            beleg_art: item.beleg_art,
            belegt_gemessen: measured,
        });
        result.uebernommen += 1;
    }

    debug_assert_eq!(result.vorgeschlagen, result.uebernommen + result.verworfen);
    result
}

fn rejection_reason(item: &RawPlanItem, sources: &PlanSources) -> Option<&'static str> {
    const BEREICHE: [&str; 6] = ["discord", "twitch", "steam", "turniere", "website", "brain"];
    if item.beleg.trim().is_empty() {
        return Some("beleg_fehlt");
    }
    if !BEREICHE.contains(&item.bereich.as_str()) {
        return Some("bereich_ausserhalb_scope");
    }
    if !(1..=3).contains(&item.prioritaet) {
        return Some("prioritaet_ungueltig");
    }
    if [
        item.titel.as_str(),
        item.begruendung.as_str(),
        item.aktion.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Some("inhalt_fehlt");
    }

    let resolved = match item.beleg_art.as_str() {
        "digest" => sources.digest_sections.contains(&item.beleg),
        "wiki" => sources.wiki_paths.contains(&item.beleg),
        "pg" => sources.pg_tables.contains(&item.beleg),
        "betrieb" => sources.betrieb_units.contains(&item.beleg),
        _ => return Some("beleg_art_ungueltig"),
    };
    (!resolved).then_some("beleg_unaufloesbar")
}

pub fn fingerprint(bereich: &str, titel: &str) -> String {
    let normalized: String = titel
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect();
    let digest = Sha256::digest(format!("{bereich}:{normalized}").as_bytes());
    hex::encode(digest)[..16].to_string()
}

#[derive(Clone, Debug)]
pub struct ServiceErrors {
    pub unit: String,
    pub errors: u64,
}

#[derive(Clone, Debug)]
pub enum OperatingHealth {
    Available {
        failed_units: Vec<String>,
        checked_units: usize,
        ok_units: usize,
        error_units: Vec<ServiceErrors>,
        all_units: BTreeSet<String>,
    },
    Unavailable(String),
}

impl OperatingHealth {
    pub fn quellen_fehlend(&self) -> Vec<String> {
        match self {
            Self::Available { .. } => Vec::new(),
            Self::Unavailable(_) => vec!["betrieb".to_string()],
        }
    }

    pub fn units(&self) -> BTreeSet<String> {
        match self {
            Self::Available { all_units, .. } => all_units.clone(),
            Self::Unavailable(_) => BTreeSet::new(),
        }
    }
}

pub fn render_operating_health(health: &OperatingHealth) -> String {
    match health {
        OperatingHealth::Unavailable(reason) => {
            format!("Quelle nicht verfügbar ({reason}).")
        }
        OperatingHealth::Available {
            failed_units,
            checked_units,
            ok_units,
            error_units,
            ..
        } => {
            let failed = if failed_units.is_empty() {
                "keine".to_string()
            } else {
                failed_units.join(", ")
            };
            let errors = if error_units.is_empty() {
                "keine".to_string()
            } else {
                error_units
                    .iter()
                    .map(|entry| format!("{}={}", entry.unit, entry.errors))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "Fehlgeschlagene Units: {failed}\nGeprüft: {checked_units}, ok: {ok_units}\nUnits mit Journal-Fehlern: {errors}"
            )
        }
    }
}

pub fn render_plan_markdown(run_date: NaiveDate, model: &str, plan: &EvaluatedPlan) -> String {
    let reasons = if plan.gate.verworfen_gruende.is_empty() {
        "keine".to_string()
    } else {
        plan.gate
            .verworfen_gruende
            .iter()
            .map(|(reason, count)| format!("{reason}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut markdown = format!(
        "# Vorschläge — {}\n\nvorgeschlagen {} / übernommen {} / verworfen {} (Gründe: {}) / Modell: {}\n\n## Lage\n\n{}\n\n",
        run_date.format("%Y-%m-%d"),
        plan.gate.vorgeschlagen,
        plan.gate.uebernommen,
        plan.gate.verworfen,
        reasons,
        model,
        plan.lage,
    );
    if plan.gate.items.is_empty() {
        markdown.push_str("## Vorschläge\n\nKeine Vorschläge.\n");
        return scrub_snowflakes(&markdown);
    }

    for priority in 1..=3 {
        let group: Vec<_> = plan
            .gate
            .items
            .iter()
            .filter(|item| item.prioritaet == priority)
            .collect();
        if group.is_empty() {
            continue;
        }
        markdown.push_str(&format!("## P{priority}\n\n"));
        for item in group {
            let evidence = if item.belegt_gemessen {
                "gemessen"
            } else {
                "behauptet"
            };
            markdown.push_str(&format!(
                "### {} — {}\n\n{}\n\nAktion: {}\n\nBeleg ({evidence}, {}): {}\n\n",
                item.bereich, item.titel, item.begruendung, item.aktion, item.beleg_art, item.beleg,
            ));
        }
    }
    scrub_snowflakes(&markdown)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeSet;

    use super::*;

    fn raw_item(bereich: &str, beleg: &str, beleg_art: &str) -> RawPlanItem {
        RawPlanItem {
            prioritaet: 2,
            bereich: bereich.to_string(),
            titel: "Router pruefen".to_string(),
            begruendung: "Der Wochenwert ist auffaellig".to_string(),
            aktion: "Logs pruefen".to_string(),
            beleg: beleg.to_string(),
            beleg_art: beleg_art.to_string(),
        }
    }

    fn sources() -> PlanSources {
        PlanSources {
            digest_sections: BTreeSet::from(["Community-Puls".to_string()]),
            wiki_paths: BTreeSet::from(["projekte/server-beleben.md".to_string()]),
            pg_tables: BTreeSet::from(["bot.ai_decision_ledger".to_string()]),
            betrieb_units: BTreeSet::from(["dl-bot.service".to_string()]),
        }
    }

    #[test]
    fn erfundener_wiki_pfad_wird_verworfen_und_gezaehlt() {
        let result = gate_items(
            vec![raw_item("discord", "projekte/erfunden.md", "wiki")],
            &sources(),
        );

        assert_eq!(result.vorgeschlagen, 1);
        assert_eq!(result.uebernommen, 0);
        assert_eq!(result.verworfen, 1);
        assert_eq!(result.verworfen_gruende["beleg_unaufloesbar"], 1);
    }

    #[test]
    fn existierender_wiki_pfad_wird_uebernommen() {
        let result = gate_items(
            vec![raw_item("discord", "projekte/server-beleben.md", "wiki")],
            &sources(),
        );

        assert_eq!(result.uebernommen, 1);
        assert!(!result.items[0].belegt_gemessen);
    }

    #[test]
    fn tradingbot_wird_vom_scope_gate_verworfen() {
        let result = gate_items(
            vec![raw_item("tradingbot", "projekte/server-beleben.md", "wiki")],
            &sources(),
        );

        assert_eq!(result.verworfen_gruende["bereich_ausserhalb_scope"], 1);
    }

    #[test]
    fn vorgeschlagen_ist_immer_uebernommen_plus_verworfen() {
        let result = gate_items(
            vec![
                raw_item("discord", "Community-Puls", "digest"),
                raw_item("discord", "", "wiki"),
                raw_item("tradingbot", "dl-bot.service", "betrieb"),
            ],
            &sources(),
        );

        assert_eq!(result.vorgeschlagen, result.uebernommen + result.verworfen);
    }

    #[test]
    fn kaputtes_llm_json_ist_ein_fehler() {
        assert!(evaluate_response("{kaputt", &sources()).is_err());
    }

    #[test]
    fn item_ohne_beleg_wird_verworfen_statt_den_lauf_abzubrechen() {
        let response = r#"{
            "lage":"Ein Beleg fehlt.",
            "items":[
                {
                    "prioritaet":2,
                    "bereich":"discord",
                    "titel":"Belegtes Item",
                    "begruendung":"Messwert liegt vor",
                    "aktion":"Prüfen",
                    "beleg":"Community-Puls",
                    "beleg_art":"digest"
                },
                {
                    "prioritaet":2,
                    "bereich":"discord",
                    "titel":"Unbelegtes Item",
                    "begruendung":"Ohne Quelle",
                    "aktion":"Nicht übernehmen",
                    "beleg_art":"digest"
                }
            ]
        }"#;

        let result = evaluate_response(response, &sources()).unwrap();

        assert_eq!(result.gate.vorgeschlagen, 2);
        assert_eq!(result.gate.uebernommen, 1);
        assert_eq!(result.gate.verworfen, 1);
        assert_eq!(result.gate.verworfen_gruende["beleg_fehlt"], 1);
    }

    #[test]
    fn null_items_ist_ok_und_markdown_sagt_keine_vorschlaege() {
        let result =
            evaluate_response(r#"{"lage":"Ruhige Woche.","items":[]}"#, &sources()).unwrap();
        let markdown = render_plan_markdown(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(),
            "deepseek-v4-pro",
            &result,
        );

        assert!(markdown.contains("Keine Vorschläge"));
    }

    #[test]
    fn fingerprint_ist_stabil_und_bereichssensitiv() {
        assert_eq!(
            fingerprint("discord", "Router pruefen!"),
            fingerprint("discord", "Router pruefen!")
        );
        assert_ne!(
            fingerprint("discord", "Router pruefen!"),
            fingerprint("twitch", "Router pruefen!")
        );
    }

    #[test]
    fn betriebsausfall_ist_sichtbar_und_als_quelle_fehlend_markiert() {
        let health = OperatingHealth::Unavailable("User-Bus nicht erreichbar".to_string());

        assert!(render_operating_health(&health).contains("Quelle nicht verfügbar"));
        assert_eq!(health.quellen_fehlend(), vec!["betrieb".to_string()]);
    }
}
