//! Strukturierter Brain-Vertrag; fremde Prompts und ungeprüfte Aussagen werden verworfen.
use crate::{AnswerError, Evidence, Retrieved, Source};
use serde_json::Value;

/// Liest nur explizite Ground-Truth und verifizierte Creator-Aussagen.
pub fn from_context(value: &Value) -> Result<Retrieved, AnswerError> {
    if value.get("intent").and_then(Value::as_str).is_none() {
        return Err(AnswerError::InvalidEvidence);
    }
    if value["intent"] == "out_of_domain"
        || value
            .pointer("/retrieval_meta/out_of_domain")
            .and_then(Value::as_bool)
            == Some(true)
    {
        return Ok(Retrieved {
            out_of_domain: true,
            ..Retrieved::default()
        });
    }
    let mut result = Retrieved::default();
    if let Some(build) = build_context(value) {
        push(
            &mut result,
            Source::GameData {
                title: "Berechneter Deadlock-Build aus Mechanik- und Patchdaten".into(),
            },
            build.to_string(),
        );
    }
    if let Some(ground) = value.get("ground_truth").and_then(Value::as_object) {
        for key in ["item", "patch_overview", "stats", "timeline", "lineage"] {
            if let Some(mut data) = ground.get(key).and_then(prune) {
                if key == "item" {
                    apply_item_overrides(&mut data);
                }
                let historical = matches!(key, "timeline" | "lineage" | "patch_overview");
                let title = if historical {
                    format!("Historische Deadlock-Patchänderungen: {key}")
                } else {
                    format!("Deadlock-Spieldaten: {key}")
                };
                let data = if historical {
                    serde_json::json!({"temporal_scope":"historical", "facts":data})
                } else {
                    data
                };
                push(&mut result, Source::GameData { title }, data.to_string());
            }
        }
    }
    add_game_wiki(value, &mut result);
    if let Some(claims) = value
        .pointer("/creator_knowledge/verified")
        .and_then(Value::as_array)
    {
        for claim in claims {
            if claim.get("status").and_then(Value::as_str) != Some("accepted") {
                continue;
            }
            let Some(text) = claim
                .get("claim_text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            else {
                continue;
            };
            let video = &claim["source_video"];
            let title = video
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Geprüfte Creator-Aussage");
            let url = video
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| super::safe_game_url(url))
                .map(str::to_owned);
            let text = serde_json::json!({"claim_text": text, "evidence_quote": claim.get("evidence_quote")}).to_string();
            push(
                &mut result,
                Source::CreatorVerified {
                    title: title.into(),
                    url,
                },
                text,
            );
        }
    }
    Ok(result)
}

fn push(result: &mut Retrieved, source: Source, text: String) {
    let current: usize = result
        .evidence
        .iter()
        .map(|item| item.text.encode_utf16().count())
        .sum();
    if text.encode_utf16().count() + current > 12_000 || result.evidence.len() >= 12 {
        result.truncated = true;
        return;
    }
    result.evidence.push(Evidence {
        id: format!("G{}", result.evidence.len() + 1),
        source,
        text,
        observed_at: None,
    });
}

fn apply_item_overrides(item: &mut Value) {
    let updates = item
        .pointer("/current_patch_overrides/updates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(properties) = item.get_mut("properties").and_then(Value::as_array_mut) {
        for property in properties {
            let Some(label) = property.get("label").and_then(Value::as_str) else {
                continue;
            };
            if let Some(update) = updates.iter().find(|update| {
                update["stat_name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(label))
            }) {
                if let Some(new) = update.get("new_value") {
                    property["value"] = new.clone();
                }
            }
        }
    }
}

// Explizite Fachfelder: unbekannte Herkunfts-/DB-/Debugfelder werden nicht übernommen.
fn prune(value: &Value) -> Option<Value> {
    match value {
        Value::Null => None,
        Value::String(text) if text.trim().is_empty() => None,
        Value::Object(object) => {
            if object.get("available") == Some(&Value::Bool(false)) {
                return None;
            }
            let clean: serde_json::Map<String, Value> = object
                .iter()
                .filter(|(key, _)| {
                    matches!(
                        key.as_str(),
                        "name"
                            | "title"
                            | "description"
                            | "cost"
                            | "tier"
                            | "slot"
                            | "activation"
                            | "is_active"
                            | "damage"
                            | "properties"
                            | "label"
                            | "value"
                            | "prefix"
                            | "postfix"
                            | "scales_with"
                            | "tooltip_section"
                            | "current_patch_overrides"
                            | "priority"
                            | "updates"
                            | "source"
                            | "patch_title"
                            | "posted_at"
                            | "line"
                            | "raw_line"
                            | "change_type"
                            | "confidence"
                            | "new_value"
                            | "old_value"
                            | "stat_name"
                            | "latest_patch"
                            | "recent_events"
                            | "stat_changes"
                            | "ability_mentions"
                            | "change_type_counts"
                            | "key"
                            | "count"
                            | "event_count"
                            | "summary"
                            | "hints"
                            | "stat_key"
                            | "stat_label"
                            | "numeric_value"
                            | "hero_name"
                            | "relations"
                            | "relation_type"
                            | "source_name"
                            | "target_name"
                            | "metadata"
                            | "related_names"
                            | "relation_counts"
                            | "rework"
                            | "upgrade"
                            | "BoundAbilities"
                            | "HeroKey"
                            | "HeroName"
                            | "Props"
                            | "Scale"
                            | "Move"
                            | "Name"
                            | "Description"
                            | "Activation"
                            | "Cost"
                            | "Tier"
                            | "Slot"
                            | "Info1"
                            | "Info2"
                            | "Info3"
                            | "Info4"
                            | "Abilities"
                            | "Stats"
                            | "Weapon"
                            | "Key"
                            | "Type"
                            | "Value"
                            | "Main"
                            | "Alt"
                            | "Cooldown"
                            | "ChargeUp"
                            | "Desc"
                            | "DescKey"
                            | "Ability"
                            | "Hero"
                            | "Health"
                            | "Damage"
                            | "AbilityCooldown"
                            | "AbilityCastDelay"
                            | "Duration"
                            | "Radius"
                            | "SlowPercent"
                            | "DebuffDuration"
                            | "ExplosionRadius"
                    )
                })
                .filter_map(|(key, value)| {
                    let projected = if key == "BoundAbilities" {
                        value.as_object().map(|slots| {
                            Value::Array(
                                slots
                                    .iter()
                                    .filter(|(slot, _)| slot.parse::<u8>().is_ok())
                                    .filter_map(|(slot, ability)| {
                                        ability.as_object()?;
                                        prune(ability).map(|mut ability| {
                                            ability["slot"] = Value::String(slot.clone());
                                            ability
                                        })
                                    })
                                    .collect(),
                            )
                        })
                    } else {
                        prune(value)
                    };
                    projected.map(|value| (key.clone(), value))
                })
                .collect();
            (!clean.is_empty()).then_some(Value::Object(clean))
        }
        Value::Array(items) => {
            let mut seen = std::collections::HashSet::new();
            let clean: Vec<_> = items
                .iter()
                .filter(|item| {
                    !["name", "Key"].iter().any(|key| {
                        item.get(*key)
                            .and_then(Value::as_str)
                            .is_some_and(|name| name.ends_with("TooltipOnly"))
                    })
                })
                .filter(|item| {
                    !matches!(
                        item.get("stat_key").and_then(Value::as_str),
                        Some("id" | "code_name")
                    )
                })
                .filter_map(prune)
                .filter(|item| seen.insert(item.to_string()))
                .collect();
            (!clean.is_empty()).then_some(Value::Array(clean))
        }
        _ => Some(value.clone()),
    }
}

fn wiki_payload(content: &str) -> Option<String> {
    for fence in ["````json", "```json"] {
        if let Some((_, rest)) = content.split_once(fence) {
            let closing = fence.trim_end_matches("json");
            let raw = rest.split_once(closing)?.0;
            let data: Value = serde_json::from_str(raw.trim()).ok()?;
            if let Some(pages) = data.pointer("/query/pages").and_then(Value::as_object) {
                let pages: Vec<_> = pages.values().filter_map(|page| {
                    let extract = page.get("extract")?.as_str()?;
                    let mut length = 0;
                    let mut omitted = false;
                    let paragraphs: Vec<_> = extract.split("\n\n").take_while(|paragraph| {
                        let size = paragraph.encode_utf16().count() + 2;
                        if length + size <= 8000 { length += size; true } else { omitted = true; false }
                    }).collect();
                    if paragraphs.is_empty() { return None; }
                    Some(serde_json::json!({"title":page.get("title"),"extract":paragraphs.join("\n\n"),"excerpt":omitted}))
                }).collect();
                return (!pages.is_empty()).then(|| Value::Array(pages).to_string());
            }
            return prune(&data).map(|data| data.to_string());
        }
    }
    // Der bekannte Gamewiki-Vertrag liefert importierte strukturierte Payloads.
    // Unbekannte freie Markdownformate werden nicht als geprüfte Spieldaten ausgegeben.
    None
}

/// Reasoner-Ausgabe wird auf nachweisbare Mechanik-/Patchbelege reduziert.
fn build_context(value: &Value) -> Option<Value> {
    if value["build_context_schema"] != "reasoner_build_v1"
        || value.pointer("/retrieval_meta/route")?.as_str()? != "build_reasoner"
    {
        return None;
    }
    if !value
        .pointer("/validation/violations")?
        .as_array()?
        .is_empty()
    {
        return None;
    }
    let build = value.get("build_context")?;
    let raw_core = build.get("core")?.as_array()?;
    let item_budget = 10_000usize.checked_div(raw_core.len())?.saturating_sub(200);
    let core = raw_core
        .iter()
        .enumerate()
        .map(|(index, item)| {
            build_item(item, item_budget).map(|mut item| {
                item["purchase_step"] = serde_json::json!(index + 1);
                item
            })
        })
        .collect::<Option<Vec<_>>>()?;
    if core.is_empty() {
        return None;
    }
    let situations: Vec<_> = build.get("situations").and_then(Value::as_array).into_iter().flatten().filter_map(|block| {
        let items = block.get("items")?.as_array()?.iter().map(|item| build_item(item, item_budget)).collect::<Option<Vec<_>>>()?;
        Some(serde_json::json!({"label": block.get("label"), "optional": block.get("optional"), "items": items}))
    }).collect();
    let mut result = serde_json::json!({"hero_name": build.get("hero_name"), "patch_tag": build.get("patch_tag"), "core": core, "purchase_order_is_binding": true, "situations": [], "evidence_condensed": true});
    for situation in situations {
        let mut candidate = result.clone();
        candidate["situations"].as_array_mut()?.push(situation);
        if candidate.to_string().encode_utf16().count() <= 11_500 {
            result = candidate;
        } else {
            result["situations_omitted"] = Value::Bool(true);
        }
    }
    Some(result)
}

fn build_item(item: &Value, budget: usize) -> Option<Value> {
    if !matches!(item.get("confidence")?.as_str()?, "High" | "Medium") {
        return None;
    }
    let raw = item.get("sources")?.as_array()?;
    // Aktuelle berechnete Mechanik vor der ungekürzten historischen Patchsammlung.
    let kind = if raw.iter().any(|source| source["kind"] == "Mechanic") {
        "Mechanic"
    } else {
        "Patch"
    };
    let mut candidates: Vec<_> = raw
        .iter()
        .filter(|source| source["kind"] == kind)
        .filter_map(|source| source.get("detail").and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())
        .collect();
    candidates.sort_by_key(|text| text.encode_utf16().count());
    candidates.dedup();
    let mut remaining = budget;
    let sources: Vec<_> = candidates
        .into_iter()
        .filter_map(|detail| {
            let source = serde_json::json!({"kind": kind, "detail": detail});
            let size = source.to_string().encode_utf16().count();
            if size <= remaining {
                remaining -= size;
                Some(source)
            } else {
                None
            }
        })
        .collect();
    if sources.is_empty() {
        return None;
    }
    Some(
        serde_json::json!({"item_id": item.get("item_id"), "name": item.get("name"), "sources": sources}),
    )
}

fn add_game_wiki(value: &Value, result: &mut Retrieved) {
    let Some(wiki) = value.pointer("/ground_truth/game_knowledge") else {
        return;
    };
    if wiki.get("available") != Some(&Value::Bool(true)) {
        return;
    }
    if let Some(context) = wiki.get("hero_context").and_then(Value::as_object) {
        if let (Some(hero), Some(complete)) = (
            context.get("hero").and_then(Value::as_str),
            context.get("complete").and_then(Value::as_bool),
        ) {
            let missing: Vec<_> = context
                .get("missing_abilities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|ability| ability.get("name").and_then(Value::as_str))
                .collect();
            let coverage = serde_json::json!({"hero":hero,"complete":complete,"bound_abilities":context.get("bound_abilities").and_then(Value::as_u64),"omitted_abilities":context.get("omitted_abilities").and_then(Value::as_u64),"missing_ability_descriptions":missing});
            push(
                result,
                Source::GameData {
                    title: "Abdeckung der Heldenfähigkeiten".into(),
                },
                coverage.to_string(),
            );
        }
    }
    for page in wiki
        .get("matches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(content) = page
            .get("content")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        else {
            continue;
        };
        let Some(content) = wiki_payload(content) else {
            continue;
        };
        let title = page
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Deadlock-Spielwissen");
        push(
            result,
            Source::GameData {
                title: title.into(),
            },
            content,
        );
    }
}

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;
const BRAIN_SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(20);

pub struct CliRetriever {
    pub bin: PathBuf,
}

#[async_trait::async_trait]
impl crate::Retriever for CliRetriever {
    async fn retrieve(&self, frage: &str) -> Result<crate::Retrieved, crate::AnswerError> {
        let output = run_brain_cli(&self.bin, frage, BRAIN_SUBPROCESS_TIMEOUT).await?;

        if !output.status.success() {
            tracing::warn!(
                status = ?output.status.code(),
                "Brain-CLI lieferte Fehlerstatus"
            );
            return Err(crate::AnswerError::Retrieval);
        }
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            tracing::warn!("Brain-CLI lieferte leeres stdout");
            return Err(crate::AnswerError::InvalidEvidence);
        }

        let value: Value = serde_json::from_slice(&output.stdout).map_err(|err| {
            tracing::warn!(%err, "Brain-CLI JSON konnte nicht geparst werden");
            crate::AnswerError::InvalidEvidence
        })?;
        from_context(&value)
    }
}

async fn run_brain_cli(
    bin: &Path,
    question: &str,
    duration: Duration,
) -> Result<Output, AnswerError> {
    let mut child = Command::new(bin)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("ask-context")
        .arg("--")
        .arg(question)
        .spawn()
        .map_err(|_| AnswerError::Retrieval)?;
    let stdout = child.stdout.take().ok_or(AnswerError::Retrieval)?;
    let stderr = child.stderr.take().ok_or(AnswerError::Retrieval)?;
    let result = timeout(duration, async {
        let (status, stdout, stderr) =
            tokio::try_join!(child.wait(), read_pipe(stdout), read_pipe(stderr))?;
        Ok::<_, std::io::Error>(Output {
            status,
            stdout,
            stderr,
        })
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        other => {
            if let Err(error) = child.start_kill() {
                tracing::warn!(kind = ?error.kind(), "Brain-Prozess konnte nicht beendet werden");
            }
            if let Err(error) = child.wait().await {
                tracing::warn!(kind = ?error.kind(), "Brain-Prozess konnte nicht aufgeräumt werden");
            }
            Err(if other.is_err() {
                AnswerError::Timeout
            } else {
                AnswerError::Retrieval
            })
        }
    }
}

async fn read_pipe<R: AsyncRead + Unpin>(pipe: R) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    pipe.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut buffer)
        .await?;
    if buffer.len() > 2 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Brain-Ausgabe überschreitet das Limit",
        ));
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn tooltip_hilfswerte_sind_keine_mechanik_und_timeline_bleibt_historisch() {
        let result = from_context(&json!({"intent":"item_question","ground_truth":{"item":{"properties":[{"name":"NonHeroAbilityLifestealTooltipOnly","value":"3"},{"name":"AbilityLifestealPercentHero","value":"13"}]},"timeline":{"recent_events":[{"raw_line":"Historische globale Änderung","posted_at":"2024-01-01"}]}}})).expect("fixture");
        assert!(!result.evidence[0].text.contains("TooltipOnly"));
        assert!(result.evidence[0].text.contains("13"));
        assert_eq!(
            serde_json::from_str::<Value>(&result.evidence[1].text).expect("json")
                ["temporal_scope"],
            "historical"
        );
    }

    #[test]
    fn mediawiki_extract_bleibt_ohne_revisions_und_seitenids() {
        let raw = json!({"query":{"pages":{"1446":{"title":"Ability","extract":"Fähigkeiten wirken mit Spirit.\n\nSchilde schützen.","revisions":[{"private":"INTERNAL_MARKER"}],"pageid":1446}}}});
        let projected =
            wiki_payload(&format!("````json\n{raw}\n````")).expect("gültige Testfixture");
        assert!(projected.contains("Fähigkeiten wirken"));
        assert!(!projected.contains("INTERNAL_MARKER"));
        assert!(!projected.contains("1446"));
        assert!(wiki_payload("Beliebiger unbekannter Markdowntext").is_none());
    }

    #[test]
    fn import_payload_haelt_fachfelder_ohne_provenienz_und_itemupgrade_bonus() {
        let raw = json!({"Name":"Held", "HeroName":"Held", "BoundAbilities":{"1":{"Name":"Fähigkeit", "Key":"ability_key"},"2":42}, "Info1":{"Main":{"Props":[{"Name":"Damage","Value":42,"Scale":{"Type":"Spirit","Value":1.2}}]}}, "Upgrades":{"Damage":14}, "_deadlock_data":{"file_path":"PRIVATE_MARKER"}});
        let markdown = format!(
            "<!-- game-wiki-entry -->\nSource Raw Path PRIVATE_MARKER\n````json\n{raw}\n````"
        );
        let projected = wiki_payload(&markdown).expect("gültige Testfixture");
        assert!(!projected.contains("PRIVATE_MARKER"));
        assert!(!projected.contains("Upgrades"));
        assert!(projected.contains("Fähigkeit"));
        assert!(projected.contains("42"));
        assert!(projected.contains("1.2"));
    }

    #[test]
    fn grosse_patchsammlung_verliert_nicht_den_gesamten_build() {
        let core: Vec<_> = (0..15).map(|i| {
            let mut sources = vec![json!({"kind":"Mechanic","detail":"Aktuelles Item: 800 Seelen und +8% Waffenschaden."})];
            sources.extend((0..45).map(|_| json!({"kind":"Patch","detail":"Historische Änderung mit eigenem vorher/nachher-Wert.".repeat(4)})));
            json!({"item_id":i,"name":format!("Item {i}"),"buy_phase":"Early","confidence":"High","sources":sources})
        }).collect();
        let result = from_context(&json!({"intent":"build_recommendation", "build_context_schema":"reasoner_build_v1","retrieval_meta":{"route":"build_reasoner"},"validation":{"violations":[]},"build_context":{"hero_name":"Held","core":core}})).expect("gültige Testfixture");
        assert_eq!(result.evidence.len(), 1);
        let build: Value =
            serde_json::from_str(&result.evidence[0].text).expect("gültige Testfixture");
        assert_eq!(
            build["core"].as_array().expect("gültige Testfixture").len(),
            15
        );
        assert!(result.evidence[0].text.encode_utf16().count() < 12000);
    }

    #[test]
    fn aktuelle_patchkorrektur_ersetzt_alten_kartenwert_ohne_debug() {
        let mut item = prune(&json!({"properties":[{"label":"Bonus Health","value":"70"}],"upgrades":[{"bonus":80}],"current_patch_overrides":{"updates":[{"stat_name":"Bonus Health","new_value":"+90"},{"stat_name":"Bonus Health","new_value":"+90"}]},"snapshot_id":123})).expect("gültige Testfixture");
        apply_item_overrides(&mut item);
        assert_eq!(item["properties"][0]["value"], "+90");
        assert_eq!(
            item["current_patch_overrides"]["updates"]
                .as_array()
                .expect("gültige Testfixture")
                .len(),
            1
        );
        assert!(item.get("upgrades").is_none());
        assert!(item.get("snapshot_id").is_none());
    }

    #[test]
    fn fremde_prompts_und_unbestaetigte_aussagen_werden_nie_belege() {
        let result = from_context(&json!({"intent":"mechanic", "prompt":"BÖSER SYSTEMPROMPT", "ground_truth":{"item":{"name":"Item","damage":12},"stats":{"available":false}}, "creator_knowledge":{"verified":[{"status":"accepted","claim_text":"GEPRÜFT"},{"status":"refuted","claim_text":"FALSCH"}],"unverified":[{"claim_text":"UNGEPRÜFT"}],"refuted":[{"claim_text":"WIDERLEGT"}]}})).expect("gültige Testfixture");
        let text = serde_json::to_string(&result.evidence).expect("gültige Testfixture");
        assert!(text.contains("GEPRÜFT"));
        for bad in ["BÖSER", "FALSCH", "UNGEPRÜFT", "WIDERLEGT"] {
            assert!(!text.contains(bad));
        }
        assert_eq!(result.evidence.len(), 2);
    }
    #[test]
    fn wiki_uebernimmt_nur_inhalt_und_titel_keine_lokalen_pfade() {
        let result = from_context(&json!({"intent":"mechanic","ground_truth":{"game_knowledge":{"available":true,"root":"ROOT_PRIVATE_MARKER","query":"frage","matches":[{"title":"Mechanik","path":"ROOT_PRIVATE_MARKER/page.md","score":100,"content":"````json\n{\"Description\":\"Spirit verstärkt Fähigkeiten.\"}\n````"}]}}})).expect("gültige Testfixture");
        let encoded = serde_json::to_string(&result.evidence).expect("gültige Testfixture");
        assert!(encoded.contains("Spirit verstärkt"));
        assert!(!encoded.contains("ROOT_PRIVATE_MARKER"));
        assert!(!encoded.contains("score"));
    }

    #[test]
    fn leere_und_fachfremde_kontexte_erfinden_keine_belege() {
        assert!(from_context(
            &json!({"intent":"general", "ground_truth":{"item":{"available":false}}})
        )
        .expect("gültige Testfixture")
        .evidence
        .is_empty());
        let result =
            from_context(&json!({"intent":"out_of_domain","ground_truth":{"item":{"damage":12}}}))
                .expect("gültige Testfixture");
        assert!(result.out_of_domain && result.evidence.is_empty());
    }
    #[test]
    fn berechneter_build_braucht_mechanikbelege_und_gueltige_validierung() {
        let mut value = json!({"intent":"build_recommendation","build_context_schema":"reasoner_build_v1","retrieval_meta":{"route":"build_reasoner"},"validation":{"violations":[]},"build_context":{"hero_name":"Held","patch_tag":"aktuell","core":[{"item_id":12,"name":"Item","buy_phase":"Lane","confidence":"High","sources":[{"kind":"Mechanic","detail":"Spirit erhöht den Schaden."}]}],"situations":[]},"prompt":"NICHT VERWENDEN"});
        let result = from_context(&value).expect("gültige Testfixture");
        assert_eq!(result.evidence.len(), 1);
        assert!(!result.evidence[0].text.contains("NICHT VERWENDEN"));
        value["build_context"]["core"][0]["sources"][0]["kind"] = json!("Claim");
        assert!(from_context(&value)
            .expect("gültige Testfixture")
            .evidence
            .is_empty());
    }
}
