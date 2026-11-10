use super::bits;
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

const SCHEMA_VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    schema_version: u32,
    #[serde(default)]
    connectors: BTreeMap<String, RawConnector>,
    #[serde(default)]
    areas: BTreeMap<String, RawArea>,
    #[serde(default)]
    capabilities: BTreeMap<String, RawCapability>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConnector {
    enabled: bool,
    shadow: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArea {
    channels: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapability {
    #[serde(default)]
    area: Option<String>,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    owner_only: bool,
    #[serde(default)]
    confirm: Option<bool>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_true")]
    shadow: bool,
    #[serde(default)]
    public: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug)]
pub struct Connector {
    pub enabled: bool,
    pub shadow: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Capability {
    pub area: Option<String>,
    pub mask: u64,
    pub roles: BTreeSet<String>,
    pub owner_only: bool,
    pub confirm: Option<bool>,
    pub enabled: bool,
    pub shadow: bool,
    pub public: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub connectors: BTreeMap<String, Connector>,
    pub areas: BTreeMap<String, BTreeSet<String>>,
    pub capabilities: BTreeMap<String, Capability>,
}

impl Policy {
    pub fn area_channels(&self, capability: &Capability) -> Option<&BTreeSet<String>> {
        capability
            .area
            .as_ref()
            .and_then(|area| self.areas.get(area))
    }
}

fn snowflake(raw: &str) -> Result<String> {
    super::snowflake_id(raw)
        .map(|id| id.to_string())
        .with_context(|| format!("id {raw} is not a positive canonical Discord id"))
}

fn capability(
    name: &str,
    raw: RawCapability,
    areas: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Capability> {
    ensure!(super::name_ok(name), "capability name {name} is invalid");
    if let Some(area) = &raw.area {
        ensure!(
            areas.contains_key(area),
            "capability {name} names unknown area {area}"
        );
    }
    ensure!(
        !(raw.public && raw.owner_only),
        "capability {name} is both public and owner_only"
    );
    ensure!(
        !(raw.public && (!raw.permissions.is_empty() || !raw.roles.is_empty())),
        "public capability {name} must not list permissions or roles"
    );
    let mut mask = 0;
    for permission in &raw.permissions {
        let bit = bits::bit(permission)
            .with_context(|| format!("capability {name} names unknown permission {permission}"))?;
        ensure!(
            bit != bits::bit("administrator").unwrap_or_default(),
            "capability {name} must not list administrator"
        );
        mask |= bit;
    }
    let roles = raw
        .roles
        .iter()
        .map(|role| snowflake(role).with_context(|| format!("capability {name} role {role}")))
        .collect::<Result<BTreeSet<_>>>()?;
    Ok(Capability {
        area: raw.area,
        mask,
        roles,
        owner_only: raw.owner_only,
        confirm: raw.confirm,
        enabled: raw.enabled,
        shadow: raw.shadow,
        public: raw.public,
    })
}

pub fn parse(text: &str) -> Result<Policy> {
    let raw: RawPolicy = toml::from_str(text).context("access file is not valid TOML")?;
    if raw.schema_version != SCHEMA_VERSION {
        bail!("access file schema_version must be {SCHEMA_VERSION}");
    }
    let mut connectors = BTreeMap::new();
    for (name, connector) in raw.connectors {
        ensure!(super::name_ok(&name), "connector name {name} is invalid");
        connectors.insert(
            name,
            Connector {
                enabled: connector.enabled,
                shadow: connector.shadow,
            },
        );
    }
    let mut areas = BTreeMap::new();
    for (name, area) in raw.areas {
        ensure!(super::name_ok(&name), "area name {name} is invalid");
        let channels = area
            .channels
            .iter()
            .map(|channel| snowflake(channel).with_context(|| format!("area {name} channel")))
            .collect::<Result<BTreeSet<_>>>()?;
        areas.insert(name, channels);
    }
    let mut capabilities = BTreeMap::new();
    for (name, raw_capability) in raw.capabilities {
        let parsed = capability(&name, raw_capability, &areas)?;
        capabilities.insert(name, parsed);
    }
    Ok(Policy {
        connectors,
        areas,
        capabilities,
    })
}

pub struct PolicyStore {
    path: PathBuf,
    current: RwLock<Option<(Arc<Policy>, [u8; 32])>>,
    last_problem: Mutex<Option<String>>,
}

impl PolicyStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            current: RwLock::new(None),
            last_problem: Mutex::new(None),
        }
    }

    fn first_report(&self, problem: String) -> bool {
        match self.last_problem.lock() {
            Ok(last) if last.as_deref() == Some(problem.as_str()) => false,
            Ok(mut last) => {
                *last = Some(problem);
                true
            }
            Err(_) => true,
        }
    }

    pub fn current(&self) -> Option<Arc<Policy>> {
        self.current
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|(policy, _)| policy.clone()))
    }

    fn known_digest(&self) -> Option<[u8; 32]> {
        self.current
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(|(_, digest)| *digest))
    }

    pub fn reload(&self) {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) => {
                if self.first_report(format!("unreadable {:?}", error.kind())) {
                    tracing::warn!(path = %self.path.display(), %error, "access file unreadable, keeping last good state");
                }
                return;
            }
        };
        let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        if self.known_digest() == Some(digest) {
            return;
        }
        match parse(&text) {
            Ok(policy) => self.install(policy, digest),
            Err(error) => {
                if self.first_report(format!("refused {}", hex::encode(digest))) {
                    tracing::warn!(path = %self.path.display(), error = %format!("{error:#}"), "access file refused, keeping last good state");
                }
            }
        }
    }

    fn install(&self, policy: Policy, digest: [u8; 32]) {
        let capabilities = policy.capabilities.len();
        match self.current.write() {
            Ok(mut guard) => {
                *guard = Some((Arc::new(policy), digest));
                if let Ok(mut last) = self.last_problem.lock() {
                    *last = None;
                }
                tracing::info!(path = %self.path.display(), capabilities, "access file loaded");
            }
            Err(_) => tracing::error!("access file state is poisoned"),
        }
    }
}

pub fn spawn_reload(store: Arc<PolicyStore>, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let store = store.clone();
            if tokio::task::spawn_blocking(move || store.reload())
                .await
                .is_err()
            {
                tracing::error!("access file reload task failed");
            }
        }
    });
}
