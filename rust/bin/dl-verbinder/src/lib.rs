//! Verbinder (R1, Discord-Agenten-Team) — pure Entscheidungs- und Render-Logik.
//!
//! Seiteneffekte (DB, LLM, Discord) leben in main.rs. Alles hier ist ohne
//! Laufzeitumgebung testbar. Ledger-Typen kommen aus dl-brain-community,
//! damit es genau eine Definition der sechs Urteilsklassen gibt.

use std::collections::{BTreeMap, HashSet};

use dl_brain_community::{Decision, LedgerEntry};
use serde_json::{json, Value};

pub const SOURCE_MATCH: &str = "agent.verbinder";
pub const SOURCE_KRITIK: &str = "agent.verbinder.kritik";
pub const SOURCE_AUSWERTUNG: &str = "agent.verbinder.auswertung";

/// Personalakte: Nonce und Prompts sind Laufzeit-Daten, nie einkompiliert.
/// Fehlt ein Teil, ist die Akte ungültig und der Lauf bricht sichtbar ab —
/// ein stiller Fallback wäre eine zweite Wahrheit.
#[derive(Debug, Clone, PartialEq)]
pub struct Akte {
    pub nonce: String,
    pub prompt_match: String,
    pub prompt_kritik: String,
}

impl Akte {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let nonce = raw
            .lines()
            .find_map(|line| line.strip_prefix("nonce:"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or("Akte ohne nonce-Feld im Frontmatter")?;
        let prompt_match = fenced_block(raw, "prompt-match")?;
        let prompt_kritik = fenced_block(raw, "prompt-kritik")?;
        Ok(Self {
            nonce,
            prompt_match,
            prompt_kritik,
        })
    }
}

fn fenced_block(raw: &str, info: &str) -> Result<String, String> {
    let start_marker = format!("```{info}");
    let mut lines = raw.lines();
    lines
        .by_ref()
        .find(|line| line.trim() == start_marker)
        .ok_or_else(|| format!("Akte ohne ```{info}-Block"))?;
    let block: Vec<&str> = lines.by_ref().take_while(|line| line.trim() != "```").collect();
    let text = block.join("\n").trim().to_string();
    if text.is_empty() {
        return Err(format!("```{info}-Block ist leer"));
    }
    Ok(text)
}

pub fn render_prompt(template: &str, platzhalter: &str, wert: &str) -> String {
    template.replace(&format!("{{{{{platzhalter}}}}}"), wert)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kategorie {
    T1SoloDoppel,
    T2ZeitZwilling,
    T3RueckkehrerAnker,
    T4GesuchOhneResonanz,
}

impl Kategorie {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T1SoloDoppel => "t1_solo_doppel",
            Self::T2ZeitZwilling => "t2_zeit_zwilling",
            Self::T3RueckkehrerAnker => "t3_rueckkehrer_anker",
            Self::T4GesuchOhneResonanz => "t4_gesuch_ohne_resonanz",
        }
    }
}

/// Ein deterministisch gefundener Kandidat. `user_b == 0` heißt: kein
/// Partner (T4-Gesuche haben nur den Suchenden).
#[derive(Debug, Clone)]
pub struct Kandidat {
    pub kategorie: Kategorie,
    pub guild_id: i64,
    pub user_a: i64,
    pub user_b: i64,
    pub opted_out: bool,
    pub daten: Value,
}

pub fn pair_key(a: i64, b: i64) -> (i64, i64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

#[derive(Debug, Clone)]
pub struct Caps {
    pub max_llm_per_run: usize,
    pub max_llm_per_day: i64,
    pub pair_cooldown_days: i64,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            max_llm_per_run: 12,
            max_llm_per_day: 150,
            pair_cooldown_days: 7,
        }
    }
}

/// Kosten- und Schleifenwächter. Läuft VOR jedem LLM-Aufruf und ist die
/// einzige Stelle, die Kandidaten aussortiert — jeder Aussortierte wird als
/// suppressed-Eintrag sichtbar, nichts verschwindet still.
///
/// Reihenfolge: Opt-out → Paar-Cooldown (Zirkel) → Tagesdeckel → Lauf-Deckel.
pub fn waechter_filter(
    kandidaten: Vec<Kandidat>,
    recent_pairs: &HashSet<(i64, i64)>,
    llm_calls_today: i64,
    caps: &Caps,
) -> (Vec<Kandidat>, Vec<LedgerEntry>) {
    let mut durchgelassen = Vec::new();
    let mut unterdrueckt = Vec::new();
    let mut budget_rest = if llm_calls_today >= caps.max_llm_per_day {
        0_usize
    } else {
        usize::try_from(caps.max_llm_per_day - llm_calls_today).unwrap_or(usize::MAX)
    };
    // Ein yes kostet zwei Aufrufe (Match + Kritik); konservativ pro Kandidat 2.
    budget_rest /= 2;

    for kandidat in kandidaten {
        if kandidat.opted_out {
            unterdrueckt.push(suppressed_entry(&kandidat, "opted_out", true));
            continue;
        }
        if recent_pairs.contains(&pair_key(kandidat.user_a, kandidat.user_b)) {
            unterdrueckt.push(suppressed_entry(&kandidat, "pair_cooldown", false));
            continue;
        }
        if budget_rest == 0 {
            unterdrueckt.push(suppressed_entry(&kandidat, "day_cap", false));
            continue;
        }
        if durchgelassen.len() >= caps.max_llm_per_run {
            unterdrueckt.push(suppressed_entry(&kandidat, "run_cap", false));
            continue;
        }
        budget_rest -= 1;
        durchgelassen.push(kandidat);
    }
    (durchgelassen, unterdrueckt)
}

fn suppressed_entry(kandidat: &Kandidat, grund: &str, anonym: bool) -> LedgerEntry {
    let basis = LedgerEntry {
        source: SOURCE_MATCH,
        subject_user_id: Some(kandidat.user_a),
        guild_id: Some(kandidat.guild_id),
        input_summary: format!(
            "trigger={};paar=({},{})",
            kandidat.kategorie.as_str(),
            kandidat.user_a,
            kandidat.user_b
        ),
        decision: Decision::Suppressed,
        confidence: None,
        reason: grund.to_string(),
        action_taken: "shadow",
        payload: json!({
            "trigger": kandidat.kategorie.as_str(),
            "kandidaten": [kandidat.user_a, kandidat.user_b],
        }),
    };
    if anonym {
        dl_brain_community::anonymize_for_privacy(&basis)
    } else {
        basis
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchUrteil {
    pub decision: Decision,
    pub confidence: Option<f32>,
    pub begruendung: String,
    pub vorschlagstext: String,
    pub kanal: String,
}

/// Parst die Match-Antwort des Modells. Erlaubt sind nur yes, no, unsure —
/// alles andere ist ein Fehler des Modells, kein neuer Urteilstyp.
pub fn parse_match_antwort(raw: &str) -> Result<MatchUrteil, String> {
    let value = json_kern(raw)?;
    let decision = match value["decision"].as_str() {
        Some("yes") => Decision::Yes,
        Some("no") => Decision::No,
        Some("unsure") => Decision::Unsure,
        other => return Err(format!("unbekannte decision: {other:?}")),
    };
    Ok(MatchUrteil {
        decision,
        confidence: value["confidence"].as_f64().map(|v| v as f32),
        begruendung: value["begruendung"].as_str().unwrap_or_default().to_string(),
        vorschlagstext: value["vorschlagstext"].as_str().unwrap_or_default().to_string(),
        kanal: value["kanal"].as_str().unwrap_or_default().to_string(),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct KritikUrteil {
    pub ok: bool,
    pub achsen: Value,
    pub begruendung: String,
}

pub fn parse_kritik_antwort(raw: &str) -> Result<KritikUrteil, String> {
    let value = json_kern(raw)?;
    let ok = match value["urteil"].as_str() {
        Some("ok") => true,
        Some("beanstandet") => false,
        other => return Err(format!("unbekanntes urteil: {other:?}")),
    };
    Ok(KritikUrteil {
        ok,
        achsen: value["achsen"].clone(),
        begruendung: value["begruendung"].as_str().unwrap_or_default().to_string(),
    })
}

/// Schneidet Markdown-Zäune und Vor-/Nachlauf weg: erstes `{` bis letztes `}`.
fn json_kern(raw: &str) -> Result<Value, String> {
    let start = raw.find('{').ok_or("kein JSON-Objekt in der Antwort")?;
    let end = raw.rfind('}').ok_or("kein JSON-Objekt in der Antwort")?;
    if end < start {
        return Err("kein JSON-Objekt in der Antwort".into());
    }
    serde_json::from_str(&raw[start..=end]).map_err(|e| format!("JSON unlesbar: {e}"))
}

/// Deterministische Nachprüfung des Vorschlagstexts. Prompt-Verbote sind
/// keine Garantie — diese Checks laufen NACH dem Modell und ihr Befund steht
/// im Ledger und im Staff-Post.
pub fn text_verstoesse(text: &str) -> Vec<&'static str> {
    let mut verstoesse = Vec::new();
    let hat_id = text
        .split(|c: char| !c.is_ascii_digit())
        .any(|ziffern| ziffern.len() >= 17);
    if hat_id {
        verstoesse.push("discord_id_im_text");
    }
    if text.contains('@') {
        verstoesse.push("ping_im_text");
    }
    if text.chars().count() > 200 {
        verstoesse.push("text_zu_lang");
    }
    if text.trim().is_empty() {
        verstoesse.push("text_leer");
    }
    verstoesse
}

/// Ledger-Eintrag für ein geparstes Match-Urteil. Das Feld `llm: true` im
/// Payload zählt für den Tagesdeckel — jeder dieser Einträge war ein
/// Modellaufruf.
pub fn ledger_entry_fuer_match_urteil(
    kandidat: &Kandidat,
    urteil: &MatchUrteil,
    verstoesse: &[&str],
) -> LedgerEntry {
    LedgerEntry {
        source: SOURCE_MATCH,
        subject_user_id: Some(kandidat.user_a),
        guild_id: Some(kandidat.guild_id),
        input_summary: format!(
            "trigger={};paar=({},{});daten={}",
            kandidat.kategorie.as_str(),
            kandidat.user_a,
            kandidat.user_b,
            kandidat.daten
        ),
        decision: urteil.decision,
        confidence: urteil.confidence,
        reason: format!("llm:{}", urteil.begruendung),
        action_taken: "shadow",
        payload: json!({
            "trigger": kandidat.kategorie.as_str(),
            "kandidaten": [kandidat.user_a, kandidat.user_b],
            "score": urteil.confidence,
            "geplanter_kanal": urteil.kanal,
            "geplanter_text": urteil.vorschlagstext,
            "text_verstoesse": verstoesse,
            "llm": true,
        }),
    }
}

/// Ledger-Eintrag für einen fehlgeschlagenen Modellaufruf (Timeout, Fehler,
/// unlesbare Antwort). Zählt ebenfalls gegen den Tagesdeckel.
pub fn ledger_entry_fuer_match_fehler(
    kandidat: &Kandidat,
    decision: Decision,
    reason: String,
) -> LedgerEntry {
    LedgerEntry {
        source: SOURCE_MATCH,
        subject_user_id: Some(kandidat.user_a),
        guild_id: Some(kandidat.guild_id),
        input_summary: format!(
            "trigger={};paar=({},{})",
            kandidat.kategorie.as_str(),
            kandidat.user_a,
            kandidat.user_b
        ),
        decision,
        confidence: None,
        reason,
        action_taken: "shadow",
        payload: json!({
            "trigger": kandidat.kategorie.as_str(),
            "kandidaten": [kandidat.user_a, kandidat.user_b],
            "llm": true,
        }),
    }
}

pub fn ledger_entry_fuer_kritik(
    kandidat: &Kandidat,
    match_ledger_id: i64,
    decision: Decision,
    reason: String,
    achsen: Value,
    begruendung: &str,
    same_model: bool,
) -> LedgerEntry {
    LedgerEntry {
        source: SOURCE_KRITIK,
        subject_user_id: Some(kandidat.user_a),
        guild_id: Some(kandidat.guild_id),
        input_summary: format!("match_ledger_id={match_ledger_id}"),
        decision,
        confidence: None,
        reason,
        action_taken: "shadow",
        payload: json!({
            "match_ledger_id": match_ledger_id,
            "achsen": achsen,
            "begruendung": begruendung,
            "same_model": same_model,
            "llm": true,
        }),
    }
}

pub fn zaehle_klassen(entries: &[LedgerEntry]) -> BTreeMap<&'static str, u64> {
    let mut zaehler: BTreeMap<&'static str, u64> = dl_brain_community::DECISION_CLASSES
        .iter()
        .map(|klasse| (klasse.as_str(), 0_u64))
        .collect();
    for entry in entries {
        if entry.source == SOURCE_MATCH {
            *zaehler.entry(entry.decision.as_str()).or_insert(0) += 1;
        }
    }
    zaehler
}

pub fn pflichtzeile(nonce: &str, kandidaten: usize, zaehler: &BTreeMap<&'static str, u64>) -> String {
    format!(
        "VERBINDER[{nonce}]: kandidaten={kandidaten} | yes={} no={} unsure={} timeout={} error={} suppressed={}",
        zaehler.get("yes").unwrap_or(&0),
        zaehler.get("no").unwrap_or(&0),
        zaehler.get("unsure").unwrap_or(&0),
        zaehler.get("timeout").unwrap_or(&0),
        zaehler.get("error").unwrap_or(&0),
        zaehler.get("suppressed").unwrap_or(&0),
    )
}

/// Staff-Post eines Laufs. `None` heißt: nichts zu posten (kein yes, kein
/// error) — der Lauf bleibt trotzdem vollständig im Ledger.
/// Der Post nennt Vorschläge nur über Trigger und Kanal, nie über User-IDs:
/// er wird in Discord persistiert und läge sonst außerhalb des Löschpfads.
pub fn render_staff_post(
    nonce: &str,
    kandidaten: usize,
    entries: &[LedgerEntry],
    kritiken: &[(i64, bool, String)],
) -> Option<String> {
    let zaehler = zaehle_klassen(entries);
    let yes = *zaehler.get("yes").unwrap_or(&0);
    let errors = *zaehler.get("error").unwrap_or(&0) + *zaehler.get("timeout").unwrap_or(&0);
    if yes == 0 && errors == 0 {
        return None;
    }

    let mut zeilen = vec![
        "**Verbinder (Shadow)** — es wurde nichts gesendet, alles nur protokolliert.".to_string(),
        pflichtzeile(nonce, kandidaten, &zaehler),
    ];
    for entry in entries.iter().filter(|e| e.decision == Decision::Yes) {
        let kanal = entry.payload["geplanter_kanal"].as_str().unwrap_or("?");
        let text = entry.payload["geplanter_text"].as_str().unwrap_or("");
        let verstoesse = entry.payload["text_verstoesse"]
            .as_array()
            .map(|liste| liste.len())
            .unwrap_or(0);
        let kritik_vermerk = kritiken
            .iter()
            .find(|(id, _, _)| entry.payload["ledger_id"].as_i64() == Some(*id))
            .map(|(_, ok, begruendung)| {
                if *ok {
                    " | Kritiker: ok".to_string()
                } else {
                    format!(" | Kritiker: beanstandet ({begruendung})")
                }
            })
            .unwrap_or_default();
        let verstoss_vermerk = if verstoesse > 0 {
            format!(" | Textprüfung: {verstoesse} Verstoß/Verstöße")
        } else {
            String::new()
        };
        zeilen.push(format!(
            "- Vorschlag [{}] über {kanal}: „{text}“{verstoss_vermerk}{kritik_vermerk}",
            entry.payload["trigger"].as_str().unwrap_or("?"),
        ));
    }
    if errors > 0 {
        zeilen.push(format!("- {errors} Lauf-Fehler/Timeouts, Details im Ledger."));
    }
    Some(zeilen.join("\n"))
}

pub fn render_tages_summary(
    nonce: &str,
    zaehler: &BTreeMap<String, i64>,
    llm_calls: i64,
    caps: &Caps,
) -> String {
    let get = |klasse: &str| zaehler.get(klasse).copied().unwrap_or(0);
    let kandidaten: i64 = zaehler.values().sum();
    format!(
        "**Verbinder Tages-Summary (Shadow)**\nVERBINDER[{nonce}]: kandidaten={kandidaten} | yes={} no={} unsure={} timeout={} error={} suppressed={}\nLLM-Aufrufe heute: {llm_calls} von {} (Tagesdeckel)",
        get("yes"),
        get("no"),
        get("unsure"),
        get("timeout"),
        get("error"),
        get("suppressed"),
        caps.max_llm_per_day,
    )
}

/// Agreement je Kategorie aus (kategorie, outcome)-Zeilen aufgelöster
/// yes-Urteile. outcome ist `met` oder `not_met`.
pub fn agreement_je_kategorie(rows: &[(String, String)]) -> BTreeMap<String, (u64, u64)> {
    let mut map: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (kategorie, outcome) in rows {
        let eintrag = map.entry(kategorie.clone()).or_insert((0, 0));
        match outcome.as_str() {
            "met" => eintrag.0 += 1,
            _ => eintrag.1 += 1,
        }
    }
    map
}

pub const AGREEMENT_GATE_PCT: f64 = 85.0;

pub struct WochenberichtInput {
    pub agreement: BTreeMap<String, (u64, u64)>,
    pub offene_yes: u64,
    pub stichprobe: Vec<String>,
    pub klassen_7d: BTreeMap<String, i64>,
}

/// Wochenbericht mit Agreement je Kategorie gegen das 85-Prozent-Gate
/// (Ramp-Muster: kategorieweise Freischaltung) plus Defekt-Heuristik:
/// eine Quelle, die nur eine Urteilsklasse produziert, ist bis zum
/// Gegenbeweis defekt.
pub fn render_wochenbericht(nonce: &str, input: &WochenberichtInput) -> String {
    let mut zeilen = vec![
        "**Verbinder Wochenauswertung (Shadow)**".to_string(),
        format!("VERBINDER[{nonce}]: Wochenbericht"),
        String::new(),
        "**Agreement je Kategorie (Grundwahrheit: trafen sie sich binnen 24 h?)**".to_string(),
    ];
    if input.agreement.is_empty() {
        zeilen.push(format!(
            "Noch keine aufgelösten Vorschläge ({} yes warten auf Auflösung).",
            input.offene_yes
        ));
    } else {
        for (kategorie, (met, not_met)) in &input.agreement {
            let gesamt = met + not_met;
            let pct = 100.0 * (*met as f64) / (gesamt as f64);
            let gate = if pct >= AGREEMENT_GATE_PCT {
                "Gate erreicht"
            } else {
                "unter Gate"
            };
            zeilen.push(format!(
                "- {kategorie}: {met}/{gesamt} eingetreten = {pct:.1} Prozent ({gate}, Schwelle {AGREEMENT_GATE_PCT} Prozent)"
            ));
        }
        if input.offene_yes > 0 {
            zeilen.push(format!("- dazu {} yes noch unaufgelöst", input.offene_yes));
        }
    }

    zeilen.push(String::new());
    zeilen.push("**Urteilsklassen der Woche**".to_string());
    let aktive_klassen = input
        .klassen_7d
        .iter()
        .filter(|(_, anzahl)| **anzahl > 0)
        .count();
    let gesamt: i64 = input.klassen_7d.values().sum();
    zeilen.push(
        input
            .klassen_7d
            .iter()
            .map(|(klasse, anzahl)| format!("{klasse}={anzahl}"))
            .collect::<Vec<_>>()
            .join(" "),
    );
    if aktive_klassen == 1 && gesamt >= 20 {
        zeilen.push(
            "⚠ Nur eine Urteilsklasse über die ganze Woche — bis zum Gegenbeweis defekt (Lehre aus no_anchor).".to_string(),
        );
    }

    zeilen.push(String::new());
    zeilen.push("**Stichprobe (5 zufällige Fälle für den Owner)**".to_string());
    if input.stichprobe.is_empty() {
        zeilen.push("Keine Fälle in dieser Woche.".to_string());
    } else {
        zeilen.extend(input.stichprobe.iter().cloned());
    }
    zeilen.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const AKTE_MINIMAL: &str = "---\nname: agent-verbinder\nnonce: VB-1\n---\n\n```prompt-match\nBewerte {{kandidat}}.\n```\n\n```prompt-kritik\nPrüfe {{vorschlag}}.\n```\n";

    fn kandidat(a: i64, b: i64) -> Kandidat {
        Kandidat {
            kategorie: Kategorie::T1SoloDoppel,
            guild_id: 7,
            user_a: a,
            user_b: b,
            opted_out: false,
            daten: json!({}),
        }
    }

    #[test]
    fn akte_parse_liefert_nonce_und_beide_prompts() {
        let akte = Akte::parse(AKTE_MINIMAL).expect("akte");
        assert_eq!(akte.nonce, "VB-1");
        assert_eq!(akte.prompt_match, "Bewerte {{kandidat}}.");
        assert_eq!(akte.prompt_kritik, "Prüfe {{vorschlag}}.");
    }

    #[test]
    fn akte_ohne_nonce_oder_prompt_ist_fehler() {
        assert!(Akte::parse("---\nname: x\n---\n```prompt-match\na\n```\n```prompt-kritik\nb\n```").is_err());
        assert!(Akte::parse("---\nnonce: VB-1\n---\n```prompt-match\na\n```").is_err());
    }

    #[test]
    fn echte_akte_im_repo_ist_parsebar_und_traegt_vb1() {
        let raw = include_str!("../akte.md");
        let akte = Akte::parse(raw).expect("echte Akte parsebar");
        assert_eq!(akte.nonce, "VB-1");
        assert!(akte.prompt_match.contains("{{kandidat}}"));
        assert!(akte.prompt_kritik.contains("{{vorschlag}}"));
        // Doku-Regel: der Prompt selbst darf den verbotenen Satzbau nicht
        // vorleben — kein Gedankenstrich in den Prompt-Daten.
        assert!(!akte.prompt_match.contains('—'));
        assert!(!akte.prompt_kritik.contains('—'));
    }

    #[test]
    fn waechter_unterdrueckt_opted_out_anonym() {
        let (durch, unterdrueckt) = waechter_filter(
            vec![Kandidat {
                opted_out: true,
                ..kandidat(1, 2)
            }],
            &HashSet::new(),
            0,
            &Caps::default(),
        );
        assert!(durch.is_empty());
        assert_eq!(unterdrueckt.len(), 1);
        assert_eq!(unterdrueckt[0].decision, Decision::Suppressed);
        assert_eq!(unterdrueckt[0].subject_user_id, None, "opted_out nie mit ID");
    }

    #[test]
    fn waechter_zirkel_pair_cooldown_greift_in_beiden_richtungen() {
        let mut recent = HashSet::new();
        recent.insert(pair_key(2, 1));
        let (durch, unterdrueckt) =
            waechter_filter(vec![kandidat(1, 2)], &recent, 0, &Caps::default());
        assert!(durch.is_empty());
        assert_eq!(unterdrueckt[0].reason, "pair_cooldown");
    }

    #[test]
    fn waechter_run_cap_und_day_cap_werden_sichtbar() {
        let caps = Caps {
            max_llm_per_run: 1,
            max_llm_per_day: 150,
            pair_cooldown_days: 7,
        };
        let (durch, unterdrueckt) = waechter_filter(
            vec![kandidat(1, 2), kandidat(3, 4)],
            &HashSet::new(),
            0,
            &caps,
        );
        assert_eq!(durch.len(), 1);
        assert_eq!(unterdrueckt[0].reason, "run_cap");

        let (durch, unterdrueckt) = waechter_filter(
            vec![kandidat(1, 2)],
            &HashSet::new(),
            150,
            &Caps::default(),
        );
        assert!(durch.is_empty());
        assert_eq!(unterdrueckt[0].reason, "day_cap");
    }

    #[test]
    fn day_cap_rechnet_kritik_aufrufe_mit_ein() {
        // 149 von 150 verbraucht: 1 Restaufruf reicht nicht für Match+Kritik.
        let (durch, unterdrueckt) = waechter_filter(
            vec![kandidat(1, 2)],
            &HashSet::new(),
            149,
            &Caps::default(),
        );
        assert!(durch.is_empty());
        assert_eq!(unterdrueckt[0].reason, "day_cap");
    }

    #[test]
    fn parse_match_akzeptiert_nur_die_drei_urteile() {
        let ok = parse_match_antwort(
            "```json\n{\"decision\":\"yes\",\"confidence\":0.8,\"begruendung\":\"b\",\"vorschlagstext\":\"t\",\"kanal\":\"lfg_watch\"}\n```",
        )
        .expect("parse");
        assert_eq!(ok.decision, Decision::Yes);
        assert_eq!(ok.confidence, Some(0.8));
        assert!(parse_match_antwort("{\"decision\":\"suppressed\"}").is_err());
        assert!(parse_match_antwort("kein json").is_err());
    }

    #[test]
    fn parse_kritik_kennt_ok_und_beanstandet() {
        let ok = parse_kritik_antwort("{\"urteil\":\"ok\",\"achsen\":{},\"begruendung\":\"x\"}")
            .expect("parse");
        assert!(ok.ok);
        let schlecht = parse_kritik_antwort(
            "{\"urteil\":\"beanstandet\",\"achsen\":{\"vertraulichkeit\":\"verstoss\"},\"begruendung\":\"x\"}",
        )
        .expect("parse");
        assert!(!schlecht.ok);
    }

    #[test]
    fn text_verstoesse_findet_id_ping_laenge() {
        assert_eq!(
            text_verstoesse("Schreib 364796363709349912 an"),
            vec!["discord_id_im_text"]
        );
        assert_eq!(text_verstoesse("Hey @alle"), vec!["ping_im_text"]);
        assert_eq!(text_verstoesse(&"x".repeat(201)), vec!["text_zu_lang"]);
        assert_eq!(text_verstoesse(""), vec!["text_leer"]);
        assert!(text_verstoesse("Lust auf eine Runde zusammen?").is_empty());
    }

    #[test]
    fn staff_post_nur_bei_yes_oder_fehler_und_ohne_user_ids() {
        let nichts = render_staff_post("VB-1", 3, &[], &[]);
        assert!(nichts.is_none());

        let kand = kandidat(364796363709349912, 706215545044729888);
        let entry = ledger_entry_fuer_match(
            &kand,
            Decision::Yes,
            Some(0.9),
            "llm".into(),
            Some("Lust auf eine Runde?"),
            Some("lfg_watch"),
            &[],
            true,
        );
        let post = render_staff_post("VB-1", 3, &[entry], &[]).expect("post");
        assert!(post.contains("VERBINDER[VB-1]: kandidaten=3 | yes=1"));
        assert!(post.contains("t1_solo_doppel"));
        assert!(
            !post.contains("364796363709349912"),
            "Staff-Post darf keine User-IDs tragen"
        );
    }

    #[test]
    fn pflichtzeile_traegt_alle_sechs_klassen() {
        let zeile = pflichtzeile("VB-1", 0, &zaehle_klassen(&[]));
        for klasse in ["yes=", "no=", "unsure=", "timeout=", "error=", "suppressed="] {
            assert!(zeile.contains(klasse), "fehlende Klasse {klasse}");
        }
    }

    #[test]
    fn agreement_und_gate_rechnen_kategorieweise() {
        let rows = vec![
            ("t1_solo_doppel".to_string(), "met".to_string()),
            ("t1_solo_doppel".to_string(), "met".to_string()),
            ("t1_solo_doppel".to_string(), "not_met".to_string()),
            ("t3_rueckkehrer_anker".to_string(), "met".to_string()),
        ];
        let agreement = agreement_je_kategorie(&rows);
        assert_eq!(agreement["t1_solo_doppel"], (2, 1));
        assert_eq!(agreement["t3_rueckkehrer_anker"], (1, 0));

        let bericht = render_wochenbericht(
            "VB-1",
            &WochenberichtInput {
                agreement,
                offene_yes: 2,
                stichprobe: vec![],
                klassen_7d: BTreeMap::from([("no".to_string(), 30_i64)]),
            },
        );
        assert!(bericht.contains("t1_solo_doppel: 2/3 eingetreten = 66.7 Prozent (unter Gate"));
        assert!(bericht.contains("t3_rueckkehrer_anker: 1/1 eingetreten = 100.0 Prozent (Gate erreicht"));
        assert!(bericht.contains("Nur eine Urteilsklasse"));
        assert!(bericht.contains("2 yes noch unaufgelöst"));
    }

    #[test]
    fn wochenbericht_ohne_aufloesung_nennt_offene_statt_leere() {
        let bericht = render_wochenbericht(
            "VB-1",
            &WochenberichtInput {
                agreement: BTreeMap::new(),
                offene_yes: 4,
                stichprobe: vec![],
                klassen_7d: BTreeMap::new(),
            },
        );
        assert!(bericht.contains("Noch keine aufgelösten Vorschläge (4 yes warten"));
    }
}
