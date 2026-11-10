use super::access::{
    effective_permissions, fetch_member, guild_source, member_access, parse_roles, Access,
    GuildRoles, MemberLookup, ADMIN,
};
use super::{discord_call, json_response, McpState};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use dl_access_gate::{
    AccessRequest, AccessResponse, ActionClass, AuthorizeRequest, AuthorizeResponse, MemberStatus,
    OutcomeRequest, OutcomeResponse, Reason,
};
use dl_core::runtime_config::StartOptions;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

mod bits;
mod decide;
mod policy;
mod record;

const CACHE_LIMIT: usize = 4096;

pub struct GateSettings {
    pub access_file: PathBuf,
    pub owner_id: Option<u64>,
    pub pool: Option<sqlx::PgPool>,
    pub ttl: Duration,
    pub lookup_timeout: Duration,
    pub decision_timeout: Duration,
    pub reload_interval: Duration,
    pub purge_every: Duration,
    pub retention_days: i32,
}

fn required(value: Option<u64>, key: &str) -> Result<u64> {
    match value {
        Some(0) => bail!("runtime.start.{key} darf nicht 0 sein"),
        Some(value) => Ok(value),
        None => bail!("runtime.start.{key} fehlt in der Betriebskonfiguration"),
    }
}

fn millis(value: Option<u64>, key: &str) -> Result<Duration> {
    required(value, key).map(Duration::from_millis)
}

fn seconds(value: Option<u64>, key: &str) -> Result<Duration> {
    required(value, key).map(Duration::from_secs)
}

impl GateSettings {
    pub fn from_options(
        access_file: PathBuf,
        owner_id: Option<u64>,
        pool: Option<sqlx::PgPool>,
        options: &StartOptions,
    ) -> Result<Self> {
        let lookup = millis(
            options.access_gate_lookup_timeout_ms,
            "access_gate_lookup_timeout_ms",
        )?;
        let decision = millis(
            options.access_gate_decision_timeout_ms,
            "access_gate_decision_timeout_ms",
        )?;
        ensure!(
            decision >= lookup,
            "runtime.start.access_gate_decision_timeout_ms muss mindestens access_gate_lookup_timeout_ms sein"
        );
        let retention_days = required(
            options.access_gate_retention_days,
            "access_gate_retention_days",
        )?;
        Ok(Self {
            access_file,
            owner_id,
            pool,
            ttl: millis(options.access_gate_cache_ttl_ms, "access_gate_cache_ttl_ms")?,
            lookup_timeout: lookup,
            decision_timeout: decision,
            reload_interval: millis(options.access_gate_reload_ms, "access_gate_reload_ms")?,
            purge_every: seconds(
                options.access_gate_purge_every_secs,
                "access_gate_purge_every_secs",
            )?,
            retention_days: i32::try_from(retention_days)
                .context("runtime.start.access_gate_retention_days ist zu groß")?,
        })
    }
}

type Stamped<T> = (Instant, T);

pub struct GateState {
    token: String,
    owner_id: Option<u64>,
    policy: Arc<policy::PolicyStore>,
    recorder: Option<record::Recorder>,
    ttl: Duration,
    lookup_timeout: Duration,
    decision_timeout: Duration,
    roles: Mutex<Option<Stamped<Arc<GuildRoles>>>>,
    members: Mutex<HashMap<u64, Stamped<Option<Arc<Access>>>>>,
    channels: Mutex<HashMap<u64, Stamped<Arc<Value>>>>,
}

struct Verdict {
    reason: Reason,
    confirm_required: bool,
    shadow: bool,
    place_ok: bool,
}

impl GateState {
    pub fn start(token: String, settings: GateSettings) -> Self {
        let retention = record::Retention {
            purge_every: settings.purge_every,
            days: settings.retention_days,
        };
        let store = Arc::new(policy::PolicyStore::new(settings.access_file));
        store.reload();
        policy::spawn_reload(store.clone(), settings.reload_interval);
        Self {
            token,
            owner_id: settings.owner_id,
            policy: store,
            recorder: settings
                .pool
                .map(|pool| record::Recorder::start(pool, retention)),
            ttl: settings.ttl,
            lookup_timeout: settings.lookup_timeout,
            decision_timeout: settings.decision_timeout,
            roles: Mutex::new(None),
            members: Mutex::new(HashMap::new()),
            channels: Mutex::new(HashMap::new()),
        }
    }

    fn is_owner(&self, user: u64) -> bool {
        crate::master::is_owner(user, self.owner_id)
    }

    fn fresh(&self, stamp: Instant) -> bool {
        stamp.elapsed() < self.ttl
    }

    async fn bounded<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::time::timeout(self.lookup_timeout, work)
            .await
            .context("Discord lookup timed out")?
    }

    async fn roles(&self, st: &McpState) -> Result<Stamped<Arc<GuildRoles>>> {
        if let Some((stamp, roles)) = self.roles.lock().await.as_ref() {
            if self.fresh(*stamp) {
                return Ok((*stamp, roles.clone()));
            }
        }
        let stamp = Instant::now();
        let (guild, verified) = guild_source(st)?;
        let raw = self
            .bounded(discord_call(
                st,
                "GET",
                &format!("/guilds/{guild}/roles"),
                &[],
                None,
                None,
            ))
            .await?;
        let roles = Arc::new(parse_roles(guild, verified, &raw)?);
        *self.roles.lock().await = Some((stamp, roles.clone()));
        Ok((stamp, roles))
    }

    async fn member(&self, st: &McpState, user: u64) -> Result<Option<Arc<Access>>> {
        if let Some((stamp, member)) = self.members.lock().await.get(&user) {
            if self.fresh(*stamp) {
                return Ok(member.clone());
            }
        }
        let stamp = Instant::now();
        let (guild, _) = guild_source(st)?;
        let lookup = self.bounded(fetch_member(st, guild, user)).await?;
        let (roles_stamp, roles) = self.roles(st).await?;
        let member = match lookup {
            MemberLookup::Missing => None,
            MemberLookup::Found(raw) => match member_access(&roles, Some(user), Some(&raw)) {
                Ok(access) => Some(Arc::new(access)),
                Err(error) => {
                    *self.roles.lock().await = None;
                    return Err(error);
                }
            },
        };
        let mut members = self.members.lock().await;
        if members.len() >= CACHE_LIMIT {
            members.retain(|_, (stamp, _)| stamp.elapsed() < self.ttl);
            if members.len() >= CACHE_LIMIT {
                members.clear();
            }
        }
        members.insert(user, (stamp.min(roles_stamp), member.clone()));
        Ok(member)
    }

    async fn channel(&self, st: &McpState, id: u64) -> Result<Arc<Value>> {
        if let Some((stamp, channel)) = self.channels.lock().await.get(&id) {
            if self.fresh(*stamp) {
                return Ok(channel.clone());
            }
        }
        let stamp = Instant::now();
        let channel = Arc::new(
            self.bounded(discord_call(
                st,
                "GET",
                &format!("/channels/{id}"),
                &[],
                None,
                None,
            ))
            .await?,
        );
        let mut channels = self.channels.lock().await;
        if channels.len() >= CACHE_LIMIT {
            channels.clear();
        }
        channels.insert(id, (stamp, channel.clone()));
        Ok(channel)
    }
}

fn snowflake(raw: Option<&str>) -> Result<Option<u64>, ()> {
    match raw {
        None => Ok(None),
        Some(raw) => raw
            .parse::<u64>()
            .ok()
            .filter(|id| *id != 0 && i64::try_from(*id).is_ok())
            .map(Some)
            .ok_or(()),
    }
}

fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
}

fn gate_for(st: &McpState, headers: &HeaderMap) -> Option<Arc<GateState>> {
    let gate = st.gate.as_ref()?;
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .is_some_and(|(scheme, token)| {
            scheme.eq_ignore_ascii_case("Bearer")
                && super::constant_time_eq(token.trim(), &gate.token)
        })
        .then(|| gate.clone())
}

fn unauthorized() -> Response {
    json_response(StatusCode::UNAUTHORIZED, json!({"error":"unauthorized"}))
}

fn invalid() -> Response {
    json_response(StatusCode::BAD_REQUEST, json!({"error":"invalid_request"}))
}

fn answer<T: serde::Serialize>(body: &T) -> Response {
    match serde_json::to_value(body) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err(_) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":"internal"}),
        ),
    }
}

fn permission_names(mask: u64) -> Vec<String> {
    if mask & ADMIN != 0 {
        bits::names(bits::all_known())
    } else {
        bits::names(mask)
    }
}

pub async fn access_post(
    State(st): State<Arc<McpState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(gate) = gate_for(&st, &headers) else {
        return unauthorized();
    };
    let Ok(request) = serde_json::from_str::<AccessRequest>(&body) else {
        return invalid();
    };
    let (Ok(Some(user)), Ok(channel)) = (
        snowflake(Some(&request.user_id)),
        snowflake(request.channel_id.as_deref()),
    ) else {
        return invalid();
    };
    answer(&access_answer(&st, &gate, user, channel).await)
}

async fn access_answer(
    st: &McpState,
    gate: &GateState,
    user: u64,
    channel: Option<u64>,
) -> AccessResponse {
    let owner = gate.is_owner(user);
    let member = match gate.member(st, user).await {
        Ok(Some(member)) => member,
        Ok(None) => {
            return AccessResponse {
                owner,
                ..AccessResponse::no_rights(MemberStatus::NotMember)
            }
        }
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "access gate member lookup failed");
            return AccessResponse {
                owner,
                ..AccessResponse::no_rights(MemberStatus::LookupFailed)
            };
        }
    };
    let channel_permissions = match channel {
        Some(channel) => channel_permissions(st, gate, channel, &member).await,
        None => None,
    };
    AccessResponse {
        status: MemberStatus::Member,
        member: true,
        owner,
        timed_out: member.timed_out,
        permissions: permission_names(member.base),
        channel_permissions,
        role_ids: member.roles.clone(),
        top_role_position: member.top_position,
    }
}

async fn channel_permissions(
    st: &McpState,
    gate: &GateState,
    channel: u64,
    member: &Access,
) -> Option<Vec<String>> {
    let raw = gate.channel(st, channel).await.ok()?;
    if raw["guild_id"].as_str() != Some(member.guild.as_str()) {
        return None;
    }
    effective_permissions(&raw, member).map(|mask| {
        if mask == u64::MAX {
            bits::names(bits::all_known())
        } else {
            bits::names(mask)
        }
    })
}

struct Ids {
    user: Option<u64>,
    channel: Option<u64>,
    target_user: Option<u64>,
    target_role: Option<u64>,
}

fn validate(request: &AuthorizeRequest) -> Option<Ids> {
    if !name_ok(&request.caller)
        || !name_ok(&request.capability)
        || !lower_hex(&request.args_hash, 64)
    {
        return None;
    }
    Some(Ids {
        user: snowflake(request.user_id.as_deref()).ok()?,
        channel: snowflake(request.channel_id.as_deref()).ok()?,
        target_user: snowflake(request.target_user_id.as_deref()).ok()?,
        target_role: snowflake(request.target_role_id.as_deref()).ok()?,
    })
}

pub async fn authorize_post(
    State(st): State<Arc<McpState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(gate) = gate_for(&st, &headers) else {
        return unauthorized();
    };
    let Ok(request) = serde_json::from_str::<AuthorizeRequest>(&body) else {
        return invalid();
    };
    let Some(ids) = validate(&request) else {
        return invalid();
    };
    let verdict =
        match tokio::time::timeout(gate.decision_timeout, decide(&st, &gate, &request, &ids)).await
        {
            Ok(verdict) => verdict,
            Err(_) => Verdict {
                reason: Reason::LookupFailed,
                confirm_required: true,
                shadow: true,
                place_ok: false,
            },
        };
    answer(&settle(&gate, request, &ids, verdict))
}

fn as_db(id: Option<u64>) -> Option<i64> {
    id.and_then(|id| i64::try_from(id).ok())
}

fn settle(
    gate: &GateState,
    request: AuthorizeRequest,
    ids: &Ids,
    verdict: Verdict,
) -> AuthorizeResponse {
    let decision_id = hex::encode(rand::random::<[u8; 16]>());
    let mut reason = verdict.reason;
    let decision = record::Decision {
        decision_id: decision_id.clone(),
        request,
        user_id: as_db(ids.user),
        channel_id: as_db(ids.channel),
        target_user_id: as_db(ids.target_user),
        target_role_id: as_db(ids.target_role),
        allowed: reason == Reason::Allowed,
        reason: reason.as_str(),
        shadow: verdict.shadow,
        confirm_required: verdict.confirm_required,
        place_ok: verdict.place_ok,
    };
    let recorded = gate
        .recorder
        .as_ref()
        .is_some_and(|recorder| recorder.decision(decision));
    if !recorded && reason == Reason::Allowed {
        tracing::warn!("access gate decision not recorded, refusing");
        reason = Reason::RecordUnavailable;
    }
    let allowed = reason == Reason::Allowed;
    AuthorizeResponse {
        decision_id,
        allowed,
        reason,
        confirm_required: verdict.confirm_required,
        shadow: verdict.shadow,
        place_ok: allowed && verdict.place_ok,
    }
}

async fn decide(st: &McpState, gate: &GateState, request: &AuthorizeRequest, ids: &Ids) -> Verdict {
    let confirm_default = request.class != ActionClass::Read;
    let refused = |reason, shadow| Verdict {
        reason,
        confirm_required: confirm_default,
        shadow,
        place_ok: false,
    };
    let Some(policy) = gate.policy.current() else {
        return refused(Reason::AccessFileUnavailable, true);
    };
    let Some(connector) = policy.connectors.get(&request.caller) else {
        return refused(Reason::UnknownCaller, true);
    };
    if !connector.enabled {
        return refused(Reason::ConnectorDisabled, connector.shadow);
    }
    let capability = policy.capabilities.get(&request.capability);
    let shadow = connector.shadow || capability.is_none_or(|capability| capability.shadow);
    let confirm_required = capability
        .and_then(|capability| capability.confirm)
        .unwrap_or(confirm_default);
    let verdict = |reason, place_ok| Verdict {
        reason,
        confirm_required,
        shadow,
        place_ok,
    };
    if capability.is_some_and(|capability| !capability.enabled) {
        return verdict(Reason::CapabilityDisabled, false);
    }
    let user = ids.user.filter(|_| request.origin.is_discord());
    let reason = match (capability.is_some_and(|c| c.public), user) {
        (true, _) => Reason::Allowed,
        (false, None) => Reason::NoIdentity,
        (false, Some(user)) => member_reason(st, gate, request, ids, capability, user).await,
    };
    if reason != Reason::Allowed {
        return verdict(reason, false);
    }
    let place_ok = place(st, gate, &policy, capability, request, ids.channel, user).await;
    verdict(Reason::Allowed, place_ok)
}

async fn member_reason(
    st: &McpState,
    gate: &GateState,
    request: &AuthorizeRequest,
    ids: &Ids,
    capability: Option<&policy::Capability>,
    user: u64,
) -> Reason {
    if gate.is_owner(user) {
        return Reason::Allowed;
    }
    let asker = match gate.member(st, user).await {
        Ok(Some(asker)) => asker,
        Ok(None) => return Reason::NotMember,
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "access gate member lookup failed");
            return Reason::LookupFailed;
        }
    };
    match decide::grant(capability, request.class, &asker) {
        Reason::Allowed => hierarchy(st, gate, &asker, ids).await,
        refused => refused,
    }
}

async fn hierarchy(st: &McpState, gate: &GateState, asker: &Access, ids: &Ids) -> Reason {
    if ids.target_user.is_none() && ids.target_role.is_none() {
        return Reason::Allowed;
    }
    let Ok((_, roles)) = gate.roles(st).await else {
        return Reason::TargetLookupFailed;
    };
    if let Some(target) = ids.target_user {
        if gate.is_owner(target) {
            return Reason::Hierarchy;
        }
        match gate.member(st, target).await {
            Ok(Some(member)) if !decide::outranks(asker, roles.target_rank(&member.roles)) => {
                return Reason::Hierarchy
            }
            Ok(_) => {}
            Err(_) => return Reason::TargetLookupFailed,
        }
    }
    if let Some(role) = ids.target_role {
        let Some(info) = roles.roles.get(&role.to_string()) else {
            return Reason::TargetLookupFailed;
        };
        if !decide::outranks(asker, info.position.unwrap_or(i64::MAX)) {
            return Reason::Hierarchy;
        }
    }
    Reason::Allowed
}

async fn place(
    st: &McpState,
    gate: &GateState,
    policy: &policy::Policy,
    capability: Option<&policy::Capability>,
    request: &AuthorizeRequest,
    channel: Option<u64>,
    user: Option<u64>,
) -> bool {
    if !request.internal {
        return true;
    }
    let (Some(channel), Some(user)) = (channel, user) else {
        return false;
    };
    let Ok(raw) = gate.channel(st, channel).await else {
        return false;
    };
    if request.dm {
        return decide::dm_with(&raw, user);
    }
    let Ok((_, roles)) = gate.roles(st).await else {
        return false;
    };
    let Some(viewers) = decide::guild_place(policy, capability, channel, &raw, &roles) else {
        return false;
    };
    let Some(capability) = capability else {
        return false;
    };
    for viewer in viewers {
        if gate.is_owner(viewer) {
            continue;
        }
        match gate.member(st, viewer).await {
            Ok(None) => {}
            Ok(Some(member)) if decide::holds(capability, &member.roles, member.base) => {}
            _ => return false,
        }
    }
    true
}

pub async fn outcome_post(
    State(st): State<Arc<McpState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(gate) = gate_for(&st, &headers) else {
        return unauthorized();
    };
    let Ok(request) = serde_json::from_str::<OutcomeRequest>(&body) else {
        return invalid();
    };
    if !lower_hex(&request.decision_id, 32) || !name_ok(&request.caller) {
        return invalid();
    }
    let queued = gate
        .recorder
        .as_ref()
        .is_some_and(|recorder| recorder.outcome(request));
    answer(&OutcomeResponse { queued })
}

#[cfg(test)]
mod tests;
