use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use dl_ai::{GenerateRequest, TextGenerator};
use dl_discord::{
    BridgeInteraction, BridgeReply, DiscordAdapter, InteractionHandler, InteractionRouter,
    ModalField, ModalSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const PROPOSAL_CHANNEL_ID: u64 = 1_474_543_558_793_887_937;
const MOD_ROLE_ID: u64 = 1_337_518_124_647_579_661;
const COMMUNITY_MOD_ROLE_ID: u64 = 1_401_891_955_931_222_110;
const GOLD: u64 = 0xC8A86B;
const COMPONENTS_V2: u64 = 32_768;
const INTERNAL_TOKEN_HEADER: &str = "X-Internal-Token";
const PREFIX: &str = "turnier-proposal:";

#[derive(Debug, Clone, Deserialize)]
struct ProposalEnvelope {
    proposal: Proposal,
    #[serde(default)]
    votes: Vec<ProposalVote>,
    #[serde(default)]
    feedback: Vec<ProposalFeedback>,
    #[serde(default)]
    learning_feedback: Vec<ProposalFeedback>,
    approvals: i64,
    required_approvals: i64,
    #[serde(default)]
    went_live: bool,
    tournament_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct Proposal {
    id: i64,
    config_json: String,
    proposal_message_id: Option<String>,
    channel_id: Option<String>,
    tournament_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProposalVote {
    caster_discord_id: String,
    decision: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProposalFeedback {
    caster_discord_id: String,
    raw_text: String,
}

#[derive(Debug, Serialize)]
struct VoteRequest {
    actor_id: String,
    role_ids: Vec<String>,
    decision: &'static str,
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct RevisionRequest {
    actor_id: String,
    role_ids: Vec<String>,
    feedback: String,
    config_json: String,
}

#[derive(Debug, Serialize)]
struct RenderedRequest {
    config_json: String,
    channel_id: String,
    message_id: String,
}

#[derive(Debug, Deserialize)]
struct PublishRequest {
    proposal_id: i64,
    channel_id: u64,
}

#[derive(Clone)]
struct TurnierClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl TurnierClient {
    fn new(base_url: String, token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        }
    }

    async fn get(&self, proposal_id: i64) -> Result<ProposalEnvelope, String> {
        self.request(self.http.get(format!(
            "{}/internal/turnier/v1/proposals/{proposal_id}",
            self.base_url
        )))
        .await
    }

    async fn vote(
        &self,
        proposal_id: i64,
        request: &VoteRequest,
    ) -> Result<ProposalEnvelope, String> {
        self.request(
            self.http
                .post(format!(
                    "{}/internal/turnier/v1/proposals/{proposal_id}/vote",
                    self.base_url
                ))
                .json(request),
        )
        .await
    }

    async fn revise(
        &self,
        proposal_id: i64,
        request: &RevisionRequest,
    ) -> Result<ProposalEnvelope, String> {
        self.request(
            self.http
                .post(format!(
                    "{}/internal/turnier/v1/proposals/{proposal_id}/revision",
                    self.base_url
                ))
                .json(request),
        )
        .await
    }

    async fn rendered(
        &self,
        proposal_id: i64,
        request: &RenderedRequest,
    ) -> Result<ProposalEnvelope, String> {
        self.request(
            self.http
                .post(format!(
                    "{}/internal/turnier/v1/proposals/{proposal_id}/rendered",
                    self.base_url
                ))
                .json(request),
        )
        .await
    }

    async fn request(&self, request: reqwest::RequestBuilder) -> Result<ProposalEnvelope, String> {
        if self.token.trim().is_empty() {
            return Err("TURNIER_INTERNAL_API_TOKEN fehlt".to_string());
        }
        let response = request
            .header(INTERNAL_TOKEN_HEADER, &self.token)
            .send()
            .await
            .map_err(|error| format!("Turnier-API nicht erreichbar: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Turnier-API-Antwort unlesbar: {error}"))?;
        if !status.is_success() {
            return Err(format!("Turnier-API {status}: {}", short(&body, 300)));
        }
        serde_json::from_str(&body).map_err(|error| format!("Turnier-API-JSON ungültig: {error}"))
    }
}

pub struct TurnierProposalService {
    client: TurnierClient,
    adapter: Arc<DiscordAdapter>,
    generator: Option<Arc<dyn TextGenerator>>,
    model: String,
}

impl TurnierProposalService {
    pub fn from_env(
        adapter: Arc<DiscordAdapter>,
        generator: Option<Arc<dyn TextGenerator>>,
        model: String,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let base_url = lookup("TURNIER_INTERNAL_API_BASE_URL")
            .unwrap_or_else(|| "http://127.0.0.1:8900".to_string());
        let token = lookup("TURNIER_INTERNAL_API_TOKEN")
            .or_else(|| lookup("MASTER_BROKER_TOKEN"))
            .or_else(|| lookup("MAIN_BOT_INTERNAL_TOKEN"))
            .or_else(|| lookup("TWITCH_INTERNAL_API_TOKEN"))
            .unwrap_or_default();
        Self {
            client: TurnierClient::new(base_url, token),
            adapter,
            generator,
            model,
        }
    }

    async fn publish(&self, proposal_id: i64, channel_id: u64) -> Result<u64, String> {
        if channel_id != PROPOSAL_CHANNEL_ID {
            return Err("Turniervorschläge dürfen nur in den Mod-Kanal".to_string());
        }
        let proposal = self.client.get(proposal_id).await?;
        if let Some(message_id) = proposal
            .proposal
            .proposal_message_id
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
        {
            return Ok(message_id);
        }

        let config = self.plan_config(&proposal, None).await?;
        let body = proposal_body(&proposal, &config)?;
        let message_id = self.adapter.send_raw_public(channel_id, &body).await?;
        self.client
            .rendered(
                proposal_id,
                &RenderedRequest {
                    config_json: config.to_string(),
                    channel_id: channel_id.to_string(),
                    message_id: message_id.to_string(),
                },
            )
            .await?;
        Ok(message_id)
    }

    async fn plan_config(
        &self,
        envelope: &ProposalEnvelope,
        requested_change: Option<&str>,
    ) -> Result<Value, String> {
        let current: Value = serde_json::from_str(&envelope.proposal.config_json)
            .map_err(|error| format!("Gespeicherter Turnierplan ungültig: {error}"))?;
        validate_plan(&current)?;
        let Some(generator) = &self.generator else {
            tracing::warn!(
                proposal_id = envelope.proposal.id,
                input = %short(&current.to_string(), 200),
                verdict = "fallback",
                confidence = 0.0,
                reason = "kein TextGenerator konfiguriert",
                "Turnier-KI-Entscheidung"
            );
            return Err("Die KI ist gerade nicht verfügbar; es wurde nichts geändert.".to_string());
        };

        let learned = envelope
            .learning_feedback
            .iter()
            .map(|item| format!("- {}: {}", item.caster_discord_id, item.raw_text))
            .collect::<Vec<_>>()
            .join("\n");
        let change =
            requested_change.unwrap_or("Plane daraus einen guten, realistischen Vorschlag.");
        let prompt = format!(
            "Aktueller Turnierplan als JSON:\n{}\n\nBisheriges Mod-Feedback:\n{}\n\nGewünschte Änderung:\n{}\n\nAntworte ausschließlich mit dem vollständigen geänderten JSON-Objekt. Behalte alle vorhandenen Schlüssel. Zeitpunkte sind RFC3339 in UTC und müssen chronologisch bleiben.",
            current,
            if learned.is_empty() { "Noch keines." } else { &learned },
            change
        );
        let output = generator
            .generate_text(GenerateRequest {
                prompt,
                system_prompt: Some("Du unterstützt vertrauenswürdige Discord-Moderatoren bei der Turnierplanung. Du schlägst nur einen Plan vor und führst keine Aktion aus.".to_string()),
                model: Some(self.model.clone()),
                max_output_tokens: Some(1_500),
                reasoning_effort: None,
                temperature: 0.2,
            })
            .await;
        let result = output
            .as_deref()
            .ok_or_else(|| "Die KI hat keinen Turnierplan geliefert.".to_string())
            .and_then(parse_plan)
            .and_then(|mut plan| {
                let plan_object = plan
                    .as_object()
                    .ok_or_else(|| "Turnierplan muss ein JSON-Objekt sein".to_string())?;
                let current_object = current
                    .as_object()
                    .ok_or_else(|| "Gespeicherter Turnierplan ist kein Objekt".to_string())?;
                if let Some(missing) = current_object
                    .keys()
                    .find(|key| !plan_object.contains_key(key.as_str()))
                {
                    return Err(format!("KI-Plan hat den Schlüssel {missing} entfernt"));
                }
                if requested_change.is_some() {
                    let revision = current.get("revision").and_then(Value::as_i64).unwrap_or(1) + 1;
                    plan["revision"] = json!(revision);
                }
                Ok(plan)
            });
        match result {
            Ok(plan) => {
                tracing::info!(
                    proposal_id = envelope.proposal.id,
                    input = %short(&current.to_string(), 200),
                    verdict = "accepted",
                    confidence = 1.0,
                    reason = "Schema und Zeitfolge gültig",
                    "Turnier-KI-Entscheidung"
                );
                Ok(plan)
            }
            Err(error) => {
                tracing::warn!(
                    proposal_id = envelope.proposal.id,
                    input = %short(output.as_deref().unwrap_or("keine Ausgabe"), 200),
                    verdict = "rejected",
                    confidence = 0.0,
                    reason = %error,
                    "Turnier-KI-Entscheidung"
                );
                Err(error)
            }
        }
    }

    async fn announcement_draft(&self, envelope: &ProposalEnvelope) -> String {
        let Some(generator) = &self.generator else {
            return "KI-Entwurf nicht verfügbar. Bitte die Ankündigung selbst schreiben."
                .to_string();
        };
        let prompt = format!(
            "Erstelle aus diesem freigegebenen Turnierplan eine kurze deutsche Discord-Ankündigung als bearbeitbare Vorlage für Mods. Keine Rollen pingen, keine Veröffentlichung behaupten, nur den fertigen Text liefern:\n{}",
            envelope.proposal.config_json
        );
        match generator
            .generate_text(GenerateRequest {
                prompt,
                system_prompt: Some("Du schreibst eine sachliche Vorlage. Die Moderatoren veröffentlichen sie später manuell.".to_string()),
                model: Some(self.model.clone()),
                max_output_tokens: Some(700),
                reasoning_effort: None,
                temperature: 0.4,
            })
            .await
        {
            Some(text) if !text.trim().is_empty() => {
                tracing::info!(
                    proposal_id = envelope.proposal.id,
                    input = %short(&envelope.proposal.config_json, 200),
                    verdict = "draft_created",
                    confidence = 1.0,
                    reason = "TextGenerator lieferte Inhalt",
                    "Turnier-KI-Entscheidung"
                );
                text.trim().to_string()
            }
            _ => {
                tracing::warn!(
                    proposal_id = envelope.proposal.id,
                    input = %short(&envelope.proposal.config_json, 200),
                    verdict = "draft_failed",
                    confidence = 0.0,
                    reason = "leere KI-Antwort",
                    "Turnier-KI-Entscheidung"
                );
                "KI-Entwurf nicht verfügbar. Bitte die Ankündigung selbst schreiben.".to_string()
            }
        }
    }

    async fn edit_proposal_message(&self, envelope: &ProposalEnvelope) -> Result<(), String> {
        let channel_id = envelope
            .proposal
            .channel_id
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| "Vorschlagskanal fehlt".to_string())?;
        let message_id = envelope
            .proposal
            .proposal_message_id
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| "Vorschlagsnachricht fehlt".to_string())?;
        let config: Value = serde_json::from_str(&envelope.proposal.config_json)
            .map_err(|error| format!("Gespeicherter Turnierplan ungültig: {error}"))?;
        self.adapter
            .edit_raw_public(channel_id, message_id, &proposal_body(envelope, &config)?)
            .await
            .map_err(|error| format!("Vorschlagskarte nicht aktualisierbar: {error}"))
    }
}

struct ProposalHandler {
    service: Arc<TurnierProposalService>,
}

#[async_trait::async_trait]
impl InteractionHandler for ProposalHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if !is_approver(&interaction.role_ids) {
            tracing::warn!(
                actor_id = interaction.user_id,
                action = %interaction.custom_id,
                verdict = "forbidden",
                "Turniervorschlag-Aktion"
            );
            return BridgeReply::ephemeral_text(
                "Nur Mods und Community-Mods dürfen Turniervorschläge bearbeiten.",
            );
        }
        let Some((action, proposal_id)) = parse_custom_id(&interaction.custom_id) else {
            return BridgeReply::ephemeral_text("Ungültige Turnieraktion.");
        };
        match action {
            "n" => modal(
                format!("{PREFIX}n-modal:{proposal_id}"),
                "Einwand / keine Zeit",
                "reason",
                "Warum passt der Vorschlag für dich nicht?",
            ),
            "change" => modal(
                format!("{PREFIX}change-modal:{proposal_id}"),
                "Änderung vorschlagen",
                "feedback",
                "Was soll die KI ändern?",
            ),
            "y" => {
                let request = VoteRequest {
                    actor_id: interaction.user_id.to_string(),
                    role_ids: interaction
                        .role_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    decision: "approve",
                    reason: None,
                };
                match self.service.client.vote(proposal_id, &request).await {
                    Ok(envelope) => {
                        tracing::info!(
                            actor_id = interaction.user_id,
                            proposal_id,
                            decision = "approve",
                            approvals = envelope.approvals,
                            went_live = envelope.went_live,
                            "Turniervorschlag-Aktion"
                        );
                        if let Err(error) = self.service.edit_proposal_message(&envelope).await {
                            return BridgeReply::ephemeral_text(error);
                        }
                        if envelope.went_live {
                            let draft = self.service.announcement_draft(&envelope).await;
                            if let Err(error) = self
                                .service
                                .adapter
                                .send_raw_public(PROPOSAL_CHANNEL_ID, &announcement_body(&draft))
                                .await
                            {
                                tracing::error!(%error, proposal_id, "Ankündigungsentwurf nicht postbar");
                            }
                        }
                        BridgeReply::ephemeral_text(if envelope.went_live {
                            "Zweite Freigabe gespeichert; Turnier angelegt und interne Vorlage erstellt."
                        } else {
                            "Freigabe gespeichert."
                        })
                    }
                    Err(error) => BridgeReply::ephemeral_text(error),
                }
            }
            "n-modal" => {
                let reason = option_text(&interaction, "reason");
                if reason.is_empty() {
                    return BridgeReply::ephemeral_text("Ein N braucht einen Grund.");
                }
                let request = VoteRequest {
                    actor_id: interaction.user_id.to_string(),
                    role_ids: interaction
                        .role_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    decision: "reject",
                    reason: Some(reason),
                };
                match self.service.client.vote(proposal_id, &request).await {
                    Ok(envelope) => {
                        tracing::info!(
                            actor_id = interaction.user_id,
                            proposal_id,
                            decision = "reject",
                            reason = %short(request.reason.as_deref().unwrap_or_default(), 200),
                            approvals = envelope.approvals,
                            "Turniervorschlag-Aktion"
                        );
                        match self.service.edit_proposal_message(&envelope).await {
                            Ok(()) => {
                                BridgeReply::ephemeral_text("N und Grund wurden gespeichert.")
                            }
                            Err(error) => BridgeReply::ephemeral_text(error),
                        }
                    }
                    Err(error) => BridgeReply::ephemeral_text(error),
                }
            }
            "change-modal" => {
                let feedback = option_text(&interaction, "feedback");
                if feedback.is_empty() {
                    return BridgeReply::ephemeral_text("Die gewünschte Änderung fehlt.");
                }
                let current = match self.service.client.get(proposal_id).await {
                    Ok(value) => value,
                    Err(error) => return BridgeReply::ephemeral_text(error),
                };
                let channel_id = current.proposal.channel_id.clone();
                let message_id = current.proposal.proposal_message_id.clone();
                let config = match self.service.plan_config(&current, Some(&feedback)).await {
                    Ok(value) => value,
                    Err(error) => return BridgeReply::ephemeral_text(error),
                };
                let request = RevisionRequest {
                    actor_id: interaction.user_id.to_string(),
                    role_ids: interaction
                        .role_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    feedback,
                    config_json: config.to_string(),
                };
                match self.service.client.revise(proposal_id, &request).await {
                    Ok(envelope) => {
                        tracing::info!(
                            actor_id = interaction.user_id,
                            proposal_id,
                            revised_proposal_id = envelope.proposal.id,
                            change = %short(&request.feedback, 200),
                            verdict = "revised",
                            "Turniervorschlag-Aktion"
                        );
                        let Some((channel_id, message_id)) = channel_id.zip(message_id) else {
                            return BridgeReply::ephemeral_text(
                                "Die ursprüngliche Vorschlagskarte ist nicht gespeichert.",
                            );
                        };
                        let envelope = match self
                            .service
                            .client
                            .rendered(
                                envelope.proposal.id,
                                &RenderedRequest {
                                    config_json: config.to_string(),
                                    channel_id,
                                    message_id,
                                },
                            )
                            .await
                        {
                            Ok(value) => value,
                            Err(error) => return BridgeReply::ephemeral_text(error),
                        };
                        match self.service.edit_proposal_message(&envelope).await {
                            Ok(()) => BridgeReply::ephemeral_text(
                                "Neue KI-Version erstellt; die Freigaben starten wieder bei 0.",
                            ),
                            Err(error) => BridgeReply::ephemeral_text(error),
                        }
                    }
                    Err(error) => BridgeReply::ephemeral_text(error),
                }
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Turnieraktion."),
        }
    }
}

pub fn register(router: &mut InteractionRouter, service: Arc<TurnierProposalService>) {
    router.on_prefix(PREFIX, Arc::new(ProposalHandler { service }));
}

#[derive(Clone)]
struct PublisherState {
    service: Arc<TurnierProposalService>,
    token: String,
}

pub fn publisher_router(service: Arc<TurnierProposalService>, token: String) -> Router {
    Router::new()
        .route(
            "/internal/master/v1/turnier/proposals/publish",
            post(publish_proposal),
        )
        .with_state(PublisherState { service, token })
}

async fn publish_proposal(
    State(state): State<PublisherState>,
    headers: HeaderMap,
    Json(request): Json<PublishRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let supplied = headers
        .get(INTERNAL_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if state.token.is_empty() || supplied != state.token {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Interne Authentifizierung fehlt"})),
        ));
    }
    match state
        .service
        .publish(request.proposal_id, request.channel_id)
        .await
    {
        Ok(message_id) => Ok(Json(json!({
            "ok": true,
            "proposal_id": request.proposal_id,
            "message_id": message_id.to_string(),
        }))),
        Err(error) => Err((StatusCode::BAD_GATEWAY, Json(json!({"error": error})))),
    }
}

fn parse_custom_id(value: &str) -> Option<(&str, i64)> {
    let rest = value.strip_prefix(PREFIX)?;
    let (action, id) = rest.rsplit_once(':')?;
    Some((action, id.parse().ok()?))
}

fn is_approver(role_ids: &[u64]) -> bool {
    role_ids
        .iter()
        .any(|role| matches!(*role, MOD_ROLE_ID | COMMUNITY_MOD_ROLE_ID))
}

fn option_text(interaction: &BridgeInteraction, key: &str) -> String {
    interaction
        .options
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn modal(custom_id: String, title: &str, field_id: &str, label: &str) -> BridgeReply {
    BridgeReply {
        modal: Some(ModalSpec {
            custom_id,
            title: title.to_string(),
            fields: vec![ModalField {
                custom_id: field_id.to_string(),
                label: label.to_string(),
                placeholder: "Kurz und konkret".to_string(),
                required: true,
                min_length: 2,
                max_length: 1_000,
                paragraph: true,
            }],
        }),
        ..BridgeReply::default()
    }
}

fn proposal_body(
    envelope: &ProposalEnvelope,
    config: &Value,
) -> Result<Map<String, Value>, String> {
    let mut body = Map::new();
    body.insert("flags".to_string(), json!(COMPONENTS_V2));
    body.insert("allowed_mentions".to_string(), json!({"parse": []}));
    body.insert(
        "components".to_string(),
        Value::Array(proposal_components(envelope, config)?),
    );
    Ok(body)
}

fn proposal_components(envelope: &ProposalEnvelope, config: &Value) -> Result<Vec<Value>, String> {
    validate_plan(config)?;
    let name = config
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Turnier");
    let event = config
        .get("event_start")
        .and_then(Value::as_str)
        .unwrap_or("—");
    let registration = config
        .get("registration_end")
        .and_then(Value::as_str)
        .unwrap_or("—");
    let revision = config.get("revision").and_then(Value::as_i64).unwrap_or(1);
    let yes = envelope
        .votes
        .iter()
        .filter(|vote| vote.decision == "approve")
        .map(|vote| format!("<@{}>", vote.caster_discord_id))
        .collect::<Vec<_>>()
        .join(", ");
    let no = envelope
        .votes
        .iter()
        .filter(|vote| vote.decision == "reject")
        .map(|vote| format!("<@{}>", vote.caster_discord_id))
        .collect::<Vec<_>>()
        .join(", ");
    let objections = envelope
        .feedback
        .iter()
        .map(|item| format!("- <@{}>: {}", item.caster_discord_id, item.raw_text))
        .collect::<Vec<_>>()
        .join("\n");
    let live = envelope.proposal.tournament_id.is_some() || envelope.tournament_id.is_some();
    let state = if live {
        "✅ Freigegeben und angelegt"
    } else {
        "🗳️ Wartet auf Freigabe"
    };
    Ok(vec![json!({
        "type": 17,
        "accent_color": GOLD,
        "components": [
            {"type": 10, "content": format!("## 🏆 Turniervorschlag #{} · Version {}\n**{}**", envelope.proposal.id, revision, name)},
            {"type": 14, "divider": true, "spacing": 1},
            {"type": 10, "content": format!("**Status:** {}\n**Turnierstart:** `{}`\n**Anmeldung bis:** `{}`\n**Freigaben:** {}/{}\n**J:** {}\n**N:** {}\n**Einwände / Änderungen:**\n{}", state, event, registration, envelope.approvals, envelope.required_approvals, if yes.is_empty() { "—" } else { &yes }, if no.is_empty() { "—" } else { &no }, if objections.is_empty() { "—" } else { &objections })},
            {"type": 14, "divider": true, "spacing": 1},
            {"type": 1, "components": [
                {"type": 2, "style": 3, "label": "J – Zeit & Freigabe", "custom_id": format!("{PREFIX}y:{}", envelope.proposal.id), "disabled": live},
                {"type": 2, "style": 4, "label": "N – Einwand / keine Zeit", "custom_id": format!("{PREFIX}n:{}", envelope.proposal.id), "disabled": live},
                {"type": 2, "style": 2, "label": "Änderung vorschlagen", "custom_id": format!("{PREFIX}change:{}", envelope.proposal.id), "disabled": live}
            ]}
        ]
    })])
}

fn announcement_body(draft: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".to_string(), json!(COMPONENTS_V2));
    body.insert("allowed_mentions".to_string(), json!({"parse": []}));
    body.insert(
        "components".to_string(),
        json!([{
            "type": 17,
            "accent_color": GOLD,
            "components": [
                {"type": 10, "content": "## 📝 Ankündigungsvorlage\nDas Turnier ist intern freigegeben. Diese Vorlage wird **nicht automatisch veröffentlicht**."},
                {"type": 14, "divider": true, "spacing": 1},
                {"type": 10, "content": draft}
            ]
        }]),
    );
    body
}

fn parse_plan(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    let json_text = if trimmed.starts_with("```") {
        trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .and_then(|value| value.strip_suffix("```"))
            .unwrap_or(trimmed)
            .trim()
    } else {
        trimmed
    };
    let value: Value = serde_json::from_str(json_text)
        .map_err(|error| format!("KI-Plan ist kein JSON-Objekt: {error}"))?;
    validate_plan(&value)?;
    Ok(value)
}

fn validate_plan(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Turnierplan muss ein JSON-Objekt sein".to_string())?;
    for key in [
        "name",
        "registration_start",
        "registration_end",
        "checkin_start",
        "event_start",
        "bracket_start",
    ] {
        if object.get(key).and_then(Value::as_str).is_none() {
            return Err(format!("Turnierplan ohne {key}"));
        }
    }
    let times = [
        "registration_start",
        "registration_end",
        "checkin_start",
        "event_start",
        "bracket_start",
    ]
    .map(|key| {
        DateTime::parse_from_rfc3339(object[key].as_str().unwrap_or_default())
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| format!("Ungültiger Zeitpunkt: {key}"))
    });
    let [registration_start, registration_end, checkin_start, event_start, bracket_start] = times;
    let (registration_start, registration_end, checkin_start, event_start, bracket_start) = (
        registration_start?,
        registration_end?,
        checkin_start?,
        event_start?,
        bracket_start?,
    );
    if !(registration_start < registration_end
        && registration_end <= checkin_start
        && checkin_start <= event_start
        && event_start <= bracket_start)
    {
        return Err("Turnierzeiten sind nicht chronologisch".to_string());
    }
    Ok(())
}

fn short(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Value {
        json!({
            "name": "Testcup",
            "registration_start": "2026-07-12T18:00:00Z",
            "registration_end": "2026-07-13T18:00:00Z",
            "checkin_start": "2026-07-13T18:30:00Z",
            "event_start": "2026-07-13T19:00:00Z",
            "bracket_start": "2026-07-13T19:10:00Z"
        })
    }

    fn envelope() -> ProposalEnvelope {
        ProposalEnvelope {
            proposal: Proposal {
                id: 42,
                config_json: config().to_string(),
                proposal_message_id: None,
                channel_id: None,
                tournament_id: None,
            },
            votes: Vec::new(),
            feedback: Vec::new(),
            learning_feedback: Vec::new(),
            approvals: 0,
            required_approvals: 2,
            went_live: false,
            tournament_id: None,
        }
    }

    #[test]
    fn proposal_card_is_gold_components_v2_with_three_actions() {
        let body = proposal_body(&envelope(), &config()).expect("valid card");
        assert_eq!(body.get("flags"), Some(&json!(COMPONENTS_V2)));
        let container = &body["components"][0];
        assert_eq!(container["accent_color"], json!(GOLD));
        assert_eq!(
            container["components"][4]["components"]
                .as_array()
                .map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn only_two_mod_roles_are_approvers() {
        assert!(is_approver(&[MOD_ROLE_ID]));
        assert!(is_approver(&[COMMUNITY_MOD_ROLE_ID]));
        assert!(!is_approver(&[1, 2, 3]));
    }

    #[test]
    fn n_and_change_open_required_comment_modals() {
        for (action, field) in [("n", "reason"), ("change", "feedback")] {
            let reply = modal(format!("{PREFIX}{action}:42"), "Titel", field, "Label");
            let modal = reply.modal.expect("modal");
            assert!(modal.fields[0].required);
            assert_eq!(modal.fields[0].custom_id, field);
        }
    }

    #[test]
    fn ai_plan_must_be_an_object_with_chronological_times() {
        assert!(parse_plan(&config().to_string()).is_ok());
        let mut invalid = config();
        invalid["event_start"] = json!("2026-07-12T17:00:00Z");
        assert_eq!(
            parse_plan(&invalid.to_string()).expect_err("invalid chronology"),
            "Turnierzeiten sind nicht chronologisch"
        );
        assert!(parse_plan("[]").is_err());
    }

    #[test]
    fn custom_ids_keep_revision_proposal_id() {
        assert_eq!(parse_custom_id("turnier-proposal:y:42"), Some(("y", 42)));
        assert_eq!(
            parse_custom_id("turnier-proposal:change-modal:99"),
            Some(("change-modal", 99))
        );
    }

    #[tokio::test]
    async fn publisher_route_requires_internal_token_before_side_effects() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let service = Arc::new(TurnierProposalService::from_env(
            DiscordAdapter::new("unused"),
            None,
            dl_ai::DEFAULT_OPENAI_MODEL.to_string(),
            |_| None,
        ));
        let response = publisher_router(service, "expected".to_string())
            .oneshot(
                Request::post("/internal/master/v1/turnier/proposals/publish")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"proposal_id": 1, "channel_id": PROPOSAL_CHANNEL_ID}).to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
