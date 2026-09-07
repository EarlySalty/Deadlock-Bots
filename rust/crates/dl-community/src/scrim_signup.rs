use std::{collections::BTreeMap, sync::Arc};

use dl_central_db::scrim_runtime::{
    require_local_scrim_write_in_transaction, ScrimRuntimeGateError,
};
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter, ModalField,
    ModalSpec,
};
use dl_squads::store::PARTICIPANTS_LOCK;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::db::{advisory_lock, u64_to_i64, CommunityDbError, CommunityDbResult};

pub const SCRIM_SIGNUP_OPEN_CUSTOM_ID: &str = "scrim_signup:open";
pub const SCRIM_SIGNUP_MODAL_CUSTOM_ID: &str = "scrim_signup:submit";
const SCRIM_SIGNUP_COMMAND: &str = "scrim-signup";
const RANK_SOURCE_STEAM: &str = "steam";
const RANK_SOURCE_ROLE: &str = "discord_role";
const RANK_SOURCE_SELF: &str = "self";
const STATUS_NEW: &str = "new";
const SOURCE_DISCORD_MODAL: &str = "discord_modal";

const SCRIM_SIGNUP_COMMAND_DESCRIPTION: &str =
    "Trag dich für die Scrims ein: Rang, Rolle und wann du kannst.";
const SCRIM_SIGNUP_MODAL_TITLE: &str = "Scrim-Anmeldung";
const SCRIM_SIGNUP_RANK_LABEL: &str = "Rang (leer lassen, wenn verifiziert)";
const SCRIM_SIGNUP_RANK_PLACEHOLDER: &str = "z. B. Oracle, Phantom, Ascendant …";
const SCRIM_SIGNUP_ROLE_LABEL: &str = "Bevorzugte Rolle / Lane";
const SCRIM_SIGNUP_ROLE_PLACEHOLDER: &str = "z. B. Support, Frontline, Carry, Mid …";
const SCRIM_SIGNUP_AVAILABILITY_LABEL: &str = "Wann kannst du?";
const SCRIM_SIGNUP_AVAILABILITY_PLACEHOLDER: &str = "z. B. Mo-Fr ab 19 Uhr, Wochenende ganzer Tag";
const SCRIM_SIGNUP_REPLY_SAVED: &str = "✅ Deine Scrim-Anmeldung ist gespeichert. Viel Erfolg!";
const SCRIM_SIGNUP_REPLY_INVALID: &str =
    "⚠️ Das konnte ich nicht lesen. Beispiel für die Zeiten: Mo-Fr ab 19 Uhr — und gib bitte eine Rolle an.";
const SCRIM_SIGNUP_REPLY_FAILED: &str =
    "⚠️ Da ist was schiefgelaufen — bitte gleich nochmal versuchen.";
const SCRIM_SIGNUP_REPLY_RUNTIME_DENIED: &str =
    "Deine Anmeldung wurde nicht gespeichert. Die Scrim-Verwaltung wird gerade umgestellt und nimmt hier keine Einträge an. Bitte versuche es später noch einmal.";

const DAY_KEYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

const RANK_ROLE_IDS: [(u64, &str); 11] = [
    (1331457571118387210, "initiate"),
    (1331457652877955072, "seeker"),
    (1331457699992436829, "acolyte"),
    (1331457724848017539, "sentinel"),
    (1331457879345070110, "mystic"),
    (1331457898781474836, "ritualist"),
    (1331457949654319114, "emissary"),
    (1316966867033653338, "oracle"),
    (1331458016356208680, "phantom"),
    (1331458049637875785, "ascendant"),
    (1331458087349129296, "eternus"),
];

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum AvailabilityStatus {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AvailabilitySlot {
    status: AvailabilityStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RankChoice {
    rank: String,
    source: &'static str,
    verified: bool,
}

#[derive(Debug, thiserror::Error)]
enum ScrimSignupError {
    #[error(transparent)]
    RuntimeGate(#[from] ScrimRuntimeGateError),
    #[error(transparent)]
    Db(#[from] CommunityDbError),
    #[error(transparent)]
    Input(#[from] ScrimSignupInputError),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
enum ScrimSignupInputError {
    #[error("rank invalid")]
    RankInvalid,
    #[error("role missing")]
    RoleMissing,
    #[error("availability invalid")]
    AvailabilityInvalid,
}

pub struct ScrimSignup {
    pool: PgPool,
}

impl ScrimSignup {
    pub fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }

    async fn handle_submit(&self, interaction: BridgeInteraction) -> Result<(), ScrimSignupError> {
        let discord_id = u64_to_i64(interaction.user_id, "interaction.user_id")?;
        let display_name = display_name_from_interaction(&interaction);
        let manual_rank = text_option(&interaction, "rank");
        let roles = text_option(&interaction, "roles").ok_or(ScrimSignupInputError::RoleMissing)?;
        let availability_raw = text_option(&interaction, "availability")
            .ok_or(ScrimSignupInputError::AvailabilityInvalid)?;
        let availability_slots = parse_availability_slots(&availability_raw)?;
        let steam_rank = verified_steam_rank(&self.pool, discord_id).await?;
        let rank = choose_rank(
            steam_rank.as_deref(),
            &interaction.role_ids,
            manual_rank.as_deref(),
        )
        .ok_or(ScrimSignupInputError::RankInvalid)?;

        let mut tx = self.pool.begin().await.map_err(CommunityDbError::from)?;
        require_local_scrim_write_in_transaction(
            &mut tx,
            "dl-community::scrim_signup",
            "scrim.participants self-service upsert",
        )
        .await?;
        upsert_structured_participant(
            &mut tx,
            discord_id,
            &display_name,
            &rank,
            &roles,
            &availability_slots,
        )
        .await?;
        tx.commit().await.map_err(CommunityDbError::from)?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl InteractionHandler for ScrimSignup {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.custom_id == SCRIM_SIGNUP_OPEN_CUSTOM_ID
            || interaction.command == SCRIM_SIGNUP_COMMAND
        {
            return BridgeReply {
                modal: Some(scrim_signup_modal()),
                ..BridgeReply::default()
            };
        }
        if interaction.custom_id != SCRIM_SIGNUP_MODAL_CUSTOM_ID {
            return BridgeReply::ephemeral_text(SCRIM_SIGNUP_REPLY_FAILED);
        }
        match self.handle_submit(interaction).await {
            Ok(()) => BridgeReply::ephemeral_text(SCRIM_SIGNUP_REPLY_SAVED),
            Err(ScrimSignupError::Input(err)) => {
                tracing::info!(%err, "Scrim-Signup: ungueltige Modal-Eingabe");
                BridgeReply::ephemeral_text(SCRIM_SIGNUP_REPLY_INVALID)
            }
            Err(ScrimSignupError::RuntimeGate(err)) => {
                tracing::warn!(%err, "Scrim-Signup: Runtime-Gate hat Speichern abgelehnt");
                BridgeReply::ephemeral_text(SCRIM_SIGNUP_REPLY_RUNTIME_DENIED)
            }
            Err(ScrimSignupError::Db(err)) => {
                tracing::warn!(%err, "Scrim-Signup: Speichern fehlgeschlagen");
                BridgeReply::ephemeral_text(SCRIM_SIGNUP_REPLY_FAILED)
            }
        }
    }
}

pub fn register(router: &mut InteractionRouter, signup: Arc<ScrimSignup>) {
    router.on_command(
        SCRIM_SIGNUP_COMMAND,
        CommandSpec {
            definition: json!({
                "name": SCRIM_SIGNUP_COMMAND,
                "description": SCRIM_SIGNUP_COMMAND_DESCRIPTION,
                "type": 1,
                "dm_permission": false,
            }),
        },
        signup.clone(),
    );
    router.on_custom_id(SCRIM_SIGNUP_OPEN_CUSTOM_ID, signup.clone());
    router.on_custom_id(SCRIM_SIGNUP_MODAL_CUSTOM_ID, signup);
}

fn scrim_signup_modal() -> ModalSpec {
    ModalSpec {
        custom_id: SCRIM_SIGNUP_MODAL_CUSTOM_ID.to_string(),
        title: SCRIM_SIGNUP_MODAL_TITLE.to_string(),
        fields: vec![
            modal_field(
                "rank",
                SCRIM_SIGNUP_RANK_LABEL,
                SCRIM_SIGNUP_RANK_PLACEHOLDER,
                false,
                32,
                false,
            ),
            modal_field(
                "roles",
                SCRIM_SIGNUP_ROLE_LABEL,
                SCRIM_SIGNUP_ROLE_PLACEHOLDER,
                true,
                80,
                false,
            ),
            modal_field(
                "availability",
                SCRIM_SIGNUP_AVAILABILITY_LABEL,
                SCRIM_SIGNUP_AVAILABILITY_PLACEHOLDER,
                true,
                400,
                true,
            ),
        ],
    }
}

fn modal_field(
    id: &str,
    label: &str,
    placeholder: &str,
    required: bool,
    max_length: u16,
    paragraph: bool,
) -> ModalField {
    ModalField {
        custom_id: id.to_string(),
        label: label.to_string(),
        placeholder: placeholder.to_string(),
        value: None,
        required,
        min_length: 0,
        max_length,
        paragraph,
    }
}

fn text_option(interaction: &BridgeInteraction, key: &str) -> Option<String> {
    interaction
        .options
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn display_name_from_interaction(interaction: &BridgeInteraction) -> String {
    [&interaction.author_display_name, &interaction.author_name]
        .into_iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("discord:{}", interaction.user_id))
}

fn normalize_rank(raw: &str) -> Option<String> {
    let rank = raw.trim().to_lowercase();
    dl_activity::lfg::RANK_NAMES
        .iter()
        .find(|(name, _)| *name == rank)
        .map(|(name, _)| (*name).to_string())
}

fn rank_from_role_ids(role_ids: &[u64]) -> Option<String> {
    RANK_ROLE_IDS
        .iter()
        .filter(|(role_id, _)| role_ids.contains(role_id))
        .max_by_key(|(_, rank)| {
            dl_activity::lfg::RANK_NAMES
                .iter()
                .find(|(name, _)| name == rank)
                .map(|(_, value)| *value)
                .unwrap_or_default()
        })
        .map(|(_, rank)| (*rank).to_string())
}

fn choose_rank(
    steam_rank: Option<&str>,
    role_ids: &[u64],
    manual_rank: Option<&str>,
) -> Option<RankChoice> {
    if let Some(rank) = steam_rank.and_then(normalize_rank) {
        return Some(RankChoice {
            rank,
            source: RANK_SOURCE_STEAM,
            verified: true,
        });
    }
    if let Some(rank) = rank_from_role_ids(role_ids) {
        return Some(RankChoice {
            rank,
            source: RANK_SOURCE_ROLE,
            verified: true,
        });
    }
    manual_rank.and_then(normalize_rank).map(|rank| RankChoice {
        rank,
        source: RANK_SOURCE_SELF,
        verified: false,
    })
}

async fn verified_steam_rank(pool: &PgPool, discord_id: i64) -> CommunityDbResult<Option<String>> {
    let Some(row) = sqlx::query(
        r#"
        SELECT deadlock_rank_name
          FROM core.steam_links
         WHERE discord_id = $1
           AND verified = TRUE
           AND deadlock_rank_name IS NOT NULL
         ORDER BY primary_account DESC, deadlock_rank_updated_at DESC NULLS LAST
         LIMIT 1
        "#,
    )
    .bind(discord_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let raw: String = row.try_get("deadlock_rank_name")?;
    Ok(normalize_rank(&raw))
}

fn parse_availability_slots(
    raw: &str,
) -> Result<BTreeMap<String, AvailabilitySlot>, ScrimSignupInputError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ScrimSignupInputError::AvailabilityInvalid);
    }
    if raw.starts_with('{') {
        let slots = serde_json::from_str::<BTreeMap<String, AvailabilitySlot>>(raw)
            .map_err(|_| ScrimSignupInputError::AvailabilityInvalid)?;
        validate_availability_slots(&slots)?;
        return Ok(slots);
    }
    parse_availability_text(raw)
}

fn validate_availability_slots(
    slots: &BTreeMap<String, AvailabilitySlot>,
) -> Result<(), ScrimSignupInputError> {
    if slots.is_empty() {
        return Err(ScrimSignupInputError::AvailabilityInvalid);
    }
    for (day, slot) in slots {
        if !DAY_KEYS.contains(&day.as_str()) {
            return Err(ScrimSignupInputError::AvailabilityInvalid);
        }
        validate_slot(slot)?;
    }
    Ok(())
}

fn validate_slot(slot: &AvailabilitySlot) -> Result<(), ScrimSignupInputError> {
    if slot.from.is_some_and(|value| value > 1440)
        || slot.to.is_some_and(|value| value > 1440)
        || matches!(slot.status, AvailabilityStatus::Available)
            && matches!((slot.from, slot.to), (Some(from), Some(to)) if from >= to)
        || !matches!(slot.status, AvailabilityStatus::Available)
            && (slot.from.is_some() || slot.to.is_some())
    {
        return Err(ScrimSignupInputError::AvailabilityInvalid);
    }
    Ok(())
}

fn parse_availability_text(
    raw: &str,
) -> Result<BTreeMap<String, AvailabilitySlot>, ScrimSignupInputError> {
    let lower = raw.to_lowercase();
    let unavailable = lower.contains("unavailable")
        || lower.contains("nicht")
        || lower.contains("keine zeit")
        || lower.contains("abwesend");
    let times = parse_times(&lower)?;
    let days = parse_days(&lower, !times.is_empty() || unavailable)?;
    let mut slots = unknown_week();
    let slot = AvailabilitySlot {
        status: if unavailable {
            AvailabilityStatus::Unavailable
        } else {
            AvailabilityStatus::Available
        },
        from: times.first().copied(),
        to: times.get(1).copied(),
    };
    validate_slot(&slot)?;
    for day in days {
        slots.insert(DAY_KEYS[day].to_string(), slot.clone());
    }
    Ok(slots)
}

fn parse_times(raw: &str) -> Result<Vec<u16>, ScrimSignupInputError> {
    let re = regex::Regex::new(r"\b([01]?\d|2[0-4])(?::([0-5]\d))?\s*(?:uhr|h)?\b")
        .map_err(|_| ScrimSignupInputError::AvailabilityInvalid)?;
    let mut times = Vec::new();
    for capture in re.captures_iter(raw) {
        let hour = capture
            .get(1)
            .and_then(|m| m.as_str().parse::<u16>().ok())
            .ok_or(ScrimSignupInputError::AvailabilityInvalid)?;
        let minute = capture
            .get(2)
            .map(|m| m.as_str().parse::<u16>())
            .transpose()
            .map_err(|_| ScrimSignupInputError::AvailabilityInvalid)?
            .unwrap_or(0);
        if hour == 24 && minute != 0 {
            return Err(ScrimSignupInputError::AvailabilityInvalid);
        }
        times.push(hour * 60 + minute);
    }
    if times.len() > 2 {
        return Err(ScrimSignupInputError::AvailabilityInvalid);
    }
    Ok(times)
}

fn parse_days(raw: &str, allow_all_fallback: bool) -> Result<Vec<usize>, ScrimSignupInputError> {
    if raw.contains("wochenende") || raw.contains("weekend") {
        return Ok(vec![5, 6]);
    }
    if raw.contains("werktag") || raw.contains("wochentag") || raw.contains("weekday") {
        return Ok((0..=4).collect());
    }
    if raw.contains("täglich") || raw.contains("taeglich") || raw.contains("daily") {
        return Ok((0..=6).collect());
    }

    let day_part = raw
        .find(|ch: char| ch.is_ascii_digit())
        .map(|idx| &raw[..idx])
        .unwrap_or(raw);
    let cleaned = day_part
        .replace(" bis ", "-")
        .replace("und", " ")
        .replace("ab", " ")
        .replace("von", " ")
        .replace("unavailable", " ")
        .replace("nicht", " ")
        .replace("keine zeit", " ")
        .replace([',', ';', '/'], " ");
    let mut days = Vec::new();
    for token in cleaned.split_whitespace() {
        let token = token.trim_matches(|ch: char| !(ch.is_alphanumeric() || ch == '-'));
        if token.is_empty() {
            continue;
        }
        if let Some((from, to)) = token.split_once('-') {
            let from = day_index(from).ok_or(ScrimSignupInputError::AvailabilityInvalid)?;
            let to = day_index(to).ok_or(ScrimSignupInputError::AvailabilityInvalid)?;
            if from <= to {
                days.extend(from..=to);
            } else {
                days.extend(from..=6);
                days.extend(0..=to);
            }
        } else if let Some(day) = day_index(token) {
            days.push(day);
        }
    }
    days.sort_unstable();
    days.dedup();
    if days.is_empty() && allow_all_fallback {
        return Ok((0..=6).collect());
    }
    if days.is_empty() {
        return Err(ScrimSignupInputError::AvailabilityInvalid);
    }
    Ok(days)
}

fn day_index(raw: &str) -> Option<usize> {
    match raw.trim() {
        "mo" | "mon" | "montag" | "monday" => Some(0),
        "di" | "die" | "tue" | "dienstag" | "tuesday" => Some(1),
        "mi" | "wed" | "mittwoch" | "wednesday" => Some(2),
        "do" | "thu" | "donnerstag" | "thursday" => Some(3),
        "fr" | "fri" | "freitag" | "friday" => Some(4),
        "sa" | "sat" | "samstag" | "saturday" => Some(5),
        "so" | "sun" | "sonntag" | "sunday" => Some(6),
        _ => None,
    }
}

fn unknown_week() -> BTreeMap<String, AvailabilitySlot> {
    DAY_KEYS
        .into_iter()
        .map(|day| {
            (
                day.to_string(),
                AvailabilitySlot {
                    status: AvailabilityStatus::Unknown,
                    from: None,
                    to: None,
                },
            )
        })
        .collect()
}

async fn upsert_structured_participant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    discord_id: i64,
    display_name: &str,
    rank: &RankChoice,
    roles: &str,
    availability_slots: &BTreeMap<String, AvailabilitySlot>,
) -> CommunityDbResult<i64> {
    let availability_slots = serde_json::to_string(availability_slots)?;
    let now = chrono::Utc::now();
    advisory_lock(tx, PARTICIPANTS_LOCK).await?;

    if let Some(row) = sqlx::query(
        r#"
        SELECT id
          FROM scrim.participants
         WHERE discord_id = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
    )
    .bind(discord_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        let id: i32 = row.try_get("id")?;
        sqlx::query(
            r#"
            UPDATE scrim.participants
               SET display_name = $2,
                   rank = $3,
                   rank_source = $4,
                   rank_verified = $5,
                   roles = $6,
                   availability_slots = $7::jsonb,
                   updated_at = $8
             WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(display_name)
        .bind(&rank.rank)
        .bind(rank.source)
        .bind(rank.verified)
        .bind(roles)
        .bind(&availability_slots)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        return Ok(i64::from(id));
    }

    if let Some(row) = sqlx::query(
        r#"
        SELECT id
          FROM scrim.participants
         WHERE display_name = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
    )
    .bind(display_name)
    .fetch_optional(&mut **tx)
    .await?
    {
        let id: i32 = row.try_get("id")?;
        sqlx::query(
            r#"
            UPDATE scrim.participants
               SET discord_id = COALESCE(discord_id, $2),
                   rank = $3,
                   rank_source = $4,
                   rank_verified = $5,
                   roles = $6,
                   availability_slots = $7::jsonb,
                   updated_at = $8
             WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(discord_id)
        .bind(&rank.rank)
        .bind(rank.source)
        .bind(rank.verified)
        .bind(roles)
        .bind(&availability_slots)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        return Ok(i64::from(id));
    }

    let id: i32 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(id), 0) + 1
          FROM scrim.participants
        "#,
    )
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.participants(
            id, discord_id, display_name, rank, rank_source, rank_verified,
            roles, availability_slots, status, source, created_at, updated_at
        )
        VALUES($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9, $10, $11, $11)
        "#,
    )
    .bind(id)
    .bind(discord_id)
    .bind(display_name)
    .bind(&rank.rank)
    .bind(rank.source)
    .bind(rank.verified)
    .bind(roles)
    .bind(&availability_slots)
    .bind(STATUS_NEW)
    .bind(SOURCE_DISCORD_MODAL)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(i64::from(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_central_db::testing::{
        set_scrim_runtime_inconsistent, set_scrim_runtime_turniere, test_pool,
    };

    fn signup_interaction(user_id: u64, display_name: &str) -> BridgeInteraction {
        BridgeInteraction {
            custom_id: SCRIM_SIGNUP_MODAL_CUSTOM_ID.to_string(),
            user_id,
            author_name: display_name.to_string(),
            author_display_name: display_name.to_string(),
            options: [
                ("rank".to_string(), json!("initiate")),
                ("roles".to_string(), json!("Support")),
                ("availability".to_string(), json!("Mo-Fr ab 19 Uhr")),
            ]
            .into_iter()
            .collect(),
            ..BridgeInteraction::default()
        }
    }

    async fn participant_count(pool: &PgPool) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM scrim.participants")
            .fetch_one(pool)
            .await
    }

    #[tokio::test]
    async fn self_service_signup_schreibt_nur_im_legacy_runtime(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let legacy = test_pool().await?;
        let reply = ScrimSignup::new(legacy.pool().clone())
            .handle(signup_interaction(1, "Legacy"))
            .await;
        assert_eq!(reply.content.as_deref(), Some(SCRIM_SIGNUP_REPLY_SAVED));
        assert_eq!(participant_count(legacy.pool()).await?, 1);

        let turniere = test_pool().await?;
        set_scrim_runtime_turniere(turniere.pool()).await?;
        let reply = ScrimSignup::new(turniere.pool().clone())
            .handle(signup_interaction(2, "Turniere"))
            .await;
        assert_eq!(
            reply.content.as_deref(),
            Some(SCRIM_SIGNUP_REPLY_RUNTIME_DENIED)
        );
        assert_eq!(participant_count(turniere.pool()).await?, 0);

        let inconsistent = test_pool().await?;
        set_scrim_runtime_inconsistent(inconsistent.pool()).await?;
        let reply = ScrimSignup::new(inconsistent.pool().clone())
            .handle(signup_interaction(3, "Widerspruch"))
            .await;
        assert_eq!(
            reply.content.as_deref(),
            Some(SCRIM_SIGNUP_REPLY_RUNTIME_DENIED)
        );
        assert_eq!(participant_count(inconsistent.pool()).await?, 0);
        Ok(())
    }

    #[test]
    fn availability_slots_roundtrip_documented_json() {
        let raw = r#"{
            "mon":{"status":"available","from":1140,"to":1200},
            "tue":{"status":"unavailable"},
            "wed":{"status":"unknown"},
            "thu":{"status":"unknown"},
            "fri":{"status":"unknown"},
            "sat":{"status":"unknown"},
            "sun":{"status":"unknown"}
        }"#;
        let slots = parse_availability_slots(raw).expect("documented json parses");
        assert_eq!(
            slots.get("mon"),
            Some(&AvailabilitySlot {
                status: AvailabilityStatus::Available,
                from: Some(1140),
                to: Some(1200),
            })
        );
        assert_eq!(
            serde_json::from_str::<BTreeMap<String, AvailabilitySlot>>(
                &serde_json::to_string(&slots).expect("serializes")
            )
            .expect("roundtrip parses"),
            slots
        );
    }

    #[test]
    fn availability_text_mo_fr_ab_19_wird_wochen_json() {
        let slots = parse_availability_slots("Mo-Fr ab 19 Uhr").expect("text parses");
        for day in ["mon", "tue", "wed", "thu", "fri"] {
            assert_eq!(
                slots.get(day),
                Some(&AvailabilitySlot {
                    status: AvailabilityStatus::Available,
                    from: Some(1140),
                    to: None,
                })
            );
        }
        assert_eq!(
            slots.get("sat"),
            Some(&AvailabilitySlot {
                status: AvailabilityStatus::Unknown,
                from: None,
                to: None,
            })
        );
    }

    #[test]
    fn rank_takeover_nimmt_steam_vor_rolle_und_manuell() {
        let choice =
            choose_rank(Some("Oracle"), &[1331458016356208680], Some("initiate")).expect("rank");
        assert_eq!(
            choice,
            RankChoice {
                rank: "oracle".to_string(),
                source: RANK_SOURCE_STEAM,
                verified: true,
            }
        );
    }

    #[test]
    fn rank_takeover_nimmt_rolle_vor_manuell() {
        let choice = choose_rank(None, &[1331458016356208680], Some("initiate")).expect("rank");
        assert_eq!(
            choice,
            RankChoice {
                rank: "phantom".to_string(),
                source: RANK_SOURCE_ROLE,
                verified: true,
            }
        );
    }
}
