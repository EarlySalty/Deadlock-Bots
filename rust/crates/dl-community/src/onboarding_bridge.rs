//! Native-Onboarding-Followup-Bruecke.
//!
//! Der DM-Claim nutzt zwei persistierte Zustaende in `bot.kv_store`: `claimed`
//! solange ein Handler noch zwischen Claim und DM-Ausgang sein kann, und
//! `dm_done` sobald die DM gesendet wurde oder Discord 50007 geliefert hat.
//! Restrisiko: Ein Crash im Fenster zwischen entschiedenem DM-Ausgang und dem
//! `dm_done`-Update laesst `claimed` zurueck; spaetere Events behandeln das
//! bewusst als in-flight, damit kein paralleler Handler den Marker vor einer
//! moeglichen DM-Wiederholung entfernt.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

pub const ONBOARDING_BRIDGE_DM_NS: &str = "onboarding_bridge_dm";
pub const RANG_VERKNUEPFUNG_ROLE_NAME: &str = "Rang-Verknüpfung";
pub const FOLLOWUP_DM_BUTTON_LABEL: &str = "Zur Anleitung";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOnboardingCompletedEvent {
    pub guild_id: u64,
    pub user_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRole {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSnapshot {
    pub is_bot: bool,
    pub roles: Vec<MemberRole>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangGuideLink {
    pub channel_id: u64,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeClaimResult {
    Acquired,
    AlreadyClaimedInFlight,
    AlreadyClaimedDmDone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeDmResult {
    Sent,
    CannotSend50007,
    Transient(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeAction {
    ClaimDm,
    SendDm,
    MarkDmDone,
    ReleaseClaim,
    RemoveMarkerRole,
    InfoDmBlocked,
    WarnDmTransient,
    WarnMarkerRemoveFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDecisionInput {
    pub event: NativeOnboardingCompletedEvent,
    pub main_guild_id: u64,
    pub is_bot: bool,
    pub member_roles: Vec<MemberRole>,
    pub claim_result: Option<BridgeClaimResult>,
    pub dm_result: Option<BridgeDmResult>,
    pub marker_remove_result: Option<Result<(), String>>,
}

#[async_trait]
pub trait OnboardingBridgePort: Send + Sync {
    async fn member_snapshot(&self, guild_id: u64, user_id: u64) -> Result<MemberSnapshot, String>;
    async fn claim_dm_once(&self, user_id: u64) -> Result<BridgeClaimResult, String>;
    async fn mark_dm_done(&self, user_id: u64) -> Result<(), String>;
    async fn release_dm_claim(&self, user_id: u64) -> Result<(), String>;
    async fn rang_guide_link(&self, guild_id: u64) -> RangGuideLink;
    async fn send_followup_dm(
        &self,
        user_id: u64,
        content: String,
        components: Value,
    ) -> BridgeDmResult;
    async fn remove_marker_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
    ) -> Result<(), String>;
}

pub struct OnboardingBridge {
    port: Arc<dyn OnboardingBridgePort>,
    main_guild_id: u64,
}

impl OnboardingBridge {
    pub fn new(port: Arc<dyn OnboardingBridgePort>, main_guild_id: u64) -> Self {
        Self {
            port,
            main_guild_id,
        }
    }

    pub async fn handle_native_onboarding_completed(&self, event: NativeOnboardingCompletedEvent) {
        if event.guild_id != self.main_guild_id {
            return;
        }

        let snapshot = match self
            .port
            .member_snapshot(event.guild_id, event.user_id)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(err) => {
                tracing::warn!(
                    %err,
                    guild_id = event.guild_id,
                    user_id = event.user_id,
                    "Onboarding-Bruecke: Member-Snapshot fehlgeschlagen"
                );
                return;
            }
        };
        let Some(marker_role_id) = marker_role_id(&snapshot.roles) else {
            return;
        };
        if snapshot.is_bot {
            return;
        }

        let initial = BridgeDecisionInput {
            event: event.clone(),
            main_guild_id: self.main_guild_id,
            is_bot: snapshot.is_bot,
            member_roles: snapshot.roles.clone(),
            claim_result: None,
            dm_result: None,
            marker_remove_result: None,
        };
        if !decide_onboarding_bridge_actions(&initial).contains(&BridgeAction::ClaimDm) {
            return;
        }

        let claim_result = match self.port.claim_dm_once(event.user_id).await {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(
                    %err,
                    guild_id = event.guild_id,
                    user_id = event.user_id,
                    "Onboarding-Bruecke: KV-Claim fehlgeschlagen"
                );
                return;
            }
        };
        let claimed = BridgeDecisionInput {
            claim_result: Some(claim_result),
            ..initial.clone()
        };
        let claimed_actions = decide_onboarding_bridge_actions(&claimed);
        if claimed_actions.contains(&BridgeAction::RemoveMarkerRole) {
            self.remove_marker_role_and_warn(&event, marker_role_id, claimed)
                .await;
            return;
        }
        if !claimed_actions.contains(&BridgeAction::SendDm) {
            return;
        }

        let guide_link = self.port.rang_guide_link(event.guild_id).await;
        let dm_result = self
            .port
            .send_followup_dm(
                event.user_id,
                followup_dm_content(guide_link.channel_id),
                followup_dm_components(&guide_link.url),
            )
            .await;
        let after_dm = BridgeDecisionInput {
            claim_result: Some(claim_result),
            dm_result: Some(dm_result.clone()),
            ..initial.clone()
        };
        let after_dm_actions = decide_onboarding_bridge_actions(&after_dm);

        if after_dm_actions.contains(&BridgeAction::ReleaseClaim) {
            if let Err(err) = self.port.release_dm_claim(event.user_id).await {
                tracing::warn!(
                    %err,
                    guild_id = event.guild_id,
                    user_id = event.user_id,
                    "Onboarding-Bruecke: KV-Claim-Freigabe fehlgeschlagen"
                );
            }
        }
        if after_dm_actions.contains(&BridgeAction::WarnDmTransient) {
            tracing::warn!(
                guild_id = event.guild_id,
                user_id = event.user_id,
                error = dm_error_label(&dm_result),
                "Onboarding-Bruecke: Followup-DM transient fehlgeschlagen"
            );
        }
        if after_dm_actions.contains(&BridgeAction::InfoDmBlocked) {
            tracing::info!(
                guild_id = event.guild_id,
                user_id = event.user_id,
                "Onboarding-Bruecke: Followup-DM nicht zustellbar (50007), Claim bleibt bestehen"
            );
        }
        if after_dm_actions.contains(&BridgeAction::MarkDmDone) {
            if let Err(err) = self.port.mark_dm_done(event.user_id).await {
                tracing::warn!(
                    %err,
                    guild_id = event.guild_id,
                    user_id = event.user_id,
                    "Onboarding-Bruecke: DM-Done-Markierung fehlgeschlagen; Marker-Cleanup wird trotzdem versucht"
                );
            }
        }
        if !after_dm_actions.contains(&BridgeAction::RemoveMarkerRole) {
            return;
        }

        self.remove_marker_role_and_warn(&event, marker_role_id, after_dm)
            .await;
    }

    async fn remove_marker_role_and_warn(
        &self,
        event: &NativeOnboardingCompletedEvent,
        marker_role_id: u64,
        input: BridgeDecisionInput,
    ) {
        let remove_result = self
            .port
            .remove_marker_role(event.guild_id, event.user_id, marker_role_id)
            .await;
        let after_remove = BridgeDecisionInput {
            marker_remove_result: Some(remove_result.clone()),
            ..input
        };
        if decide_onboarding_bridge_actions(&after_remove)
            .contains(&BridgeAction::WarnMarkerRemoveFailed)
        {
            let err = remove_result
                .err()
                .unwrap_or_else(|| "unbekannter Fehler".to_string());
            tracing::warn!(
                %err,
                guild_id = event.guild_id,
                user_id = event.user_id,
                role_id = marker_role_id,
                "Onboarding-Bruecke: Marker-Rolle konnte nicht entfernt werden"
            );
        }
    }
}

pub fn decide_onboarding_bridge_actions(input: &BridgeDecisionInput) -> Vec<BridgeAction> {
    if input.event.guild_id != input.main_guild_id || input.is_bot {
        return Vec::new();
    }
    if marker_role_id(&input.member_roles).is_none() {
        return Vec::new();
    }

    let Some(claim_result) = input.claim_result else {
        return vec![BridgeAction::ClaimDm];
    };
    match claim_result {
        BridgeClaimResult::AlreadyClaimedInFlight => return Vec::new(),
        BridgeClaimResult::AlreadyClaimedDmDone => {
            let mut actions = vec![BridgeAction::RemoveMarkerRole];
            if matches!(input.marker_remove_result, Some(Err(_))) {
                actions.push(BridgeAction::WarnMarkerRemoveFailed);
            }
            return actions;
        }
        BridgeClaimResult::Acquired => {}
    }

    let mut actions = vec![BridgeAction::ClaimDm];
    let Some(dm_result) = input.dm_result.as_ref() else {
        actions.push(BridgeAction::SendDm);
        return actions;
    };
    actions.push(BridgeAction::SendDm);

    match dm_result {
        BridgeDmResult::Sent => {
            actions.push(BridgeAction::MarkDmDone);
            actions.push(BridgeAction::RemoveMarkerRole);
        }
        BridgeDmResult::CannotSend50007 => {
            actions.push(BridgeAction::InfoDmBlocked);
            actions.push(BridgeAction::MarkDmDone);
            actions.push(BridgeAction::RemoveMarkerRole);
        }
        BridgeDmResult::Transient(_) => {
            actions.push(BridgeAction::ReleaseClaim);
            actions.push(BridgeAction::WarnDmTransient);
            return actions;
        }
    }

    if matches!(input.marker_remove_result, Some(Err(_))) {
        actions.push(BridgeAction::WarnMarkerRemoveFailed);
    }

    actions
}

pub fn marker_role_id(roles: &[MemberRole]) -> Option<u64> {
    let expected = normalized_role_name(RANG_VERKNUEPFUNG_ROLE_NAME);
    roles
        .iter()
        .find(|role| normalized_role_name(&role.name) == expected)
        .map(|role| role.id)
}

pub fn followup_dm_content(channel_id: u64) -> String {
    format!(
        "Hey! Du hast beim Onboarding angekreuzt, dass du deinen Rang verknüpfen willst. 🔗\n\n\
Die Anleitung dafür steht in <#{channel_id}> — drei kurze Schritte, danach zieht sich dein Rang automatisch aus deinen echten Matches und bleibt von selbst aktuell.\n\n\
Falls was hakt: Schreib einfach in den Kanal, wir helfen dir weiter."
    )
}

pub fn followup_dm_components(guide_url: &str) -> Value {
    json!([{ "type": 1, "components": [{
        "type": 2,
        "style": 5,
        "label": FOLLOWUP_DM_BUTTON_LABEL,
        "url": guide_url,
    }]}])
}

fn normalized_role_name(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn dm_error_label(result: &BridgeDmResult) -> &str {
    match result {
        BridgeDmResult::Sent => "sent",
        BridgeDmResult::CannotSend50007 => "50007",
        BridgeDmResult::Transient(err) => err.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const MAIN_GUILD: u64 = 1289721245281292288;
    const USER_ID: u64 = 42;
    const MARKER_ROLE_ID: u64 = 5008;

    fn marker_role() -> MemberRole {
        MemberRole {
            id: MARKER_ROLE_ID,
            name: RANG_VERKNUEPFUNG_ROLE_NAME.to_string(),
        }
    }

    fn decision(
        roles: Vec<MemberRole>,
        claim_result: Option<BridgeClaimResult>,
        dm_result: Option<BridgeDmResult>,
        marker_remove_result: Option<Result<(), String>>,
    ) -> Vec<BridgeAction> {
        decide_onboarding_bridge_actions(&BridgeDecisionInput {
            event: NativeOnboardingCompletedEvent {
                guild_id: MAIN_GUILD,
                user_id: USER_ID,
            },
            main_guild_id: MAIN_GUILD,
            is_bot: false,
            member_roles: roles,
            claim_result,
            dm_result,
            marker_remove_result,
        })
    }

    #[test]
    fn entscheidung_ohne_marker_tut_nichts_und_claimt_nicht() {
        assert!(decision(Vec::new(), None, None, None).is_empty());
    }

    #[test]
    fn entscheidung_marker_aber_claim_in_flight_tut_nichts() {
        assert!(decision(
            vec![marker_role()],
            Some(BridgeClaimResult::AlreadyClaimedInFlight),
            None,
            None
        )
        .is_empty());
    }

    #[test]
    fn entscheidung_dm_done_mit_marker_entfernt_nur_marker() {
        assert_eq!(
            decision(
                vec![marker_role()],
                Some(BridgeClaimResult::AlreadyClaimedDmDone),
                None,
                None,
            ),
            vec![BridgeAction::RemoveMarkerRole]
        );
    }

    #[test]
    fn entscheidung_dm_ok_claimt_sendet_und_entfernt_marker() {
        assert_eq!(
            decision(
                vec![marker_role()],
                Some(BridgeClaimResult::Acquired),
                Some(BridgeDmResult::Sent),
                Some(Ok(())),
            ),
            vec![
                BridgeAction::ClaimDm,
                BridgeAction::SendDm,
                BridgeAction::MarkDmDone,
                BridgeAction::RemoveMarkerRole,
            ]
        );
    }

    #[test]
    fn entscheidung_dm_50007_behaelt_claim_und_entfernt_marker() {
        assert_eq!(
            decision(
                vec![marker_role()],
                Some(BridgeClaimResult::Acquired),
                Some(BridgeDmResult::CannotSend50007),
                Some(Ok(())),
            ),
            vec![
                BridgeAction::ClaimDm,
                BridgeAction::SendDm,
                BridgeAction::InfoDmBlocked,
                BridgeAction::MarkDmDone,
                BridgeAction::RemoveMarkerRole,
            ]
        );
    }

    #[test]
    fn entscheidung_dm_transient_gibt_claim_frei_und_entfernt_marker_nicht() {
        assert_eq!(
            decision(
                vec![marker_role()],
                Some(BridgeClaimResult::Acquired),
                Some(BridgeDmResult::Transient("timeout".to_string())),
                None,
            ),
            vec![
                BridgeAction::ClaimDm,
                BridgeAction::SendDm,
                BridgeAction::ReleaseClaim,
                BridgeAction::WarnDmTransient,
            ]
        );
    }

    #[test]
    fn entscheidung_rollen_entfernung_fehler_loggt_nur_warnung() {
        assert_eq!(
            decision(
                vec![marker_role()],
                Some(BridgeClaimResult::Acquired),
                Some(BridgeDmResult::Sent),
                Some(Err("403".to_string())),
            ),
            vec![
                BridgeAction::ClaimDm,
                BridgeAction::SendDm,
                BridgeAction::MarkDmDone,
                BridgeAction::RemoveMarkerRole,
                BridgeAction::WarnMarkerRemoveFailed,
            ]
        );
    }

    #[test]
    fn followup_dm_text_ist_byte_genau_und_ohne_deutsche_anfuehrungszeichen() {
        let text = followup_dm_content(1398021105339334666);

        assert_eq!(
            text,
            "Hey! Du hast beim Onboarding angekreuzt, dass du deinen Rang verknüpfen willst. 🔗\n\n\
Die Anleitung dafür steht in <#1398021105339334666> — drei kurze Schritte, danach zieht sich dein Rang automatisch aus deinen echten Matches und bleibt von selbst aktuell.\n\n\
Falls was hakt: Schreib einfach in den Kanal, wir helfen dir weiter."
        );
        assert!(!text.contains('\u{201e}'));
        assert!(!text.contains('\u{201c}'));
    }

    struct TestPort {
        calls: Mutex<Vec<&'static str>>,
        claim_results: Mutex<VecDeque<BridgeClaimResult>>,
        dm_result: BridgeDmResult,
    }

    impl TestPort {
        fn new(claim_results: Vec<BridgeClaimResult>, dm_result: BridgeDmResult) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                claim_results: Mutex::new(claim_results.into()),
                dm_result,
            }
        }
    }

    #[async_trait]
    impl OnboardingBridgePort for TestPort {
        async fn member_snapshot(
            &self,
            _guild_id: u64,
            _user_id: u64,
        ) -> Result<MemberSnapshot, String> {
            self.calls.lock().expect("calls").push("member_snapshot");
            Ok(MemberSnapshot {
                is_bot: false,
                roles: vec![marker_role()],
            })
        }

        async fn claim_dm_once(&self, _user_id: u64) -> Result<BridgeClaimResult, String> {
            self.calls.lock().expect("calls").push("claim");
            Ok(self
                .claim_results
                .lock()
                .expect("claim_results")
                .pop_front()
                .unwrap_or(BridgeClaimResult::Acquired))
        }

        async fn mark_dm_done(&self, _user_id: u64) -> Result<(), String> {
            self.calls.lock().expect("calls").push("mark_done");
            Ok(())
        }

        async fn release_dm_claim(&self, _user_id: u64) -> Result<(), String> {
            self.calls.lock().expect("calls").push("release");
            Ok(())
        }

        async fn rang_guide_link(&self, guild_id: u64) -> RangGuideLink {
            self.calls.lock().expect("calls").push("guide_url");
            RangGuideLink {
                channel_id: 1398021105339334666,
                url: format!("https://discord.com/channels/{guild_id}/1398021105339334666/9001"),
            }
        }

        async fn send_followup_dm(
            &self,
            _user_id: u64,
            _content: String,
            _components: Value,
        ) -> BridgeDmResult {
            self.calls.lock().expect("calls").push("send_dm");
            self.dm_result.clone()
        }

        async fn remove_marker_role(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _role_id: u64,
        ) -> Result<(), String> {
            self.calls.lock().expect("calls").push("remove_role");
            Ok(())
        }
    }

    #[tokio::test]
    async fn service_nutzt_port_in_der_erwarteten_reihenfolge() {
        let port = Arc::new(TestPort::new(
            vec![BridgeClaimResult::Acquired],
            BridgeDmResult::Sent,
        ));
        let bridge = OnboardingBridge::new(port.clone(), MAIN_GUILD);

        bridge
            .handle_native_onboarding_completed(NativeOnboardingCompletedEvent {
                guild_id: MAIN_GUILD,
                user_id: USER_ID,
            })
            .await;

        assert_eq!(
            *port.calls.lock().expect("calls"),
            vec![
                "member_snapshot",
                "claim",
                "guide_url",
                "send_dm",
                "mark_done",
                "remove_role"
            ]
        );
    }

    #[tokio::test]
    async fn service_zweites_event_nach_dm_done_entfernt_nur_marker_ohne_dm() {
        let port = Arc::new(TestPort::new(
            vec![
                BridgeClaimResult::Acquired,
                BridgeClaimResult::AlreadyClaimedDmDone,
            ],
            BridgeDmResult::Sent,
        ));
        let bridge = OnboardingBridge::new(port.clone(), MAIN_GUILD);
        let event = NativeOnboardingCompletedEvent {
            guild_id: MAIN_GUILD,
            user_id: USER_ID,
        };

        bridge
            .handle_native_onboarding_completed(event.clone())
            .await;
        port.calls.lock().expect("calls").clear();
        bridge.handle_native_onboarding_completed(event).await;

        assert_eq!(
            *port.calls.lock().expect("calls"),
            vec!["member_snapshot", "claim", "remove_role"]
        );
    }
}
