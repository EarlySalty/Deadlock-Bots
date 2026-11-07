use dl_brain::{InviteLoungeUpdate, InviteProblem, InviteStatus};
use serde::{Deserialize, Serialize};

pub(super) const FOLLOW_UP_SECONDS: i64 = 60;
pub(super) const RETRY_SECONDS: i64 = 5 * 60;
pub(super) const CHECK_IN_SECONDS: i64 = 30 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Notice {
    FriendRequest,
    Invite,
    AlreadyInvited,
    InvalidCode,
    Rejected,
    NotAccepted,
    CheckIn,
    Confirmed,
}

impl Notice {
    pub(super) fn update(self) -> InviteLoungeUpdate {
        match self {
            Self::FriendRequest => status(InviteStatus::FriendshipMissing),
            Self::Invite => status(InviteStatus::Sent),
            Self::AlreadyInvited => status(InviteStatus::AlreadyHasGame),
            Self::InvalidCode => problem(InviteProblem::InvalidFriendCode),
            Self::Rejected => problem(InviteProblem::SteamRejected),
            Self::NotAccepted => problem(InviteProblem::FriendRequestNotAccepted),
            Self::CheckIn => InviteLoungeUpdate::CheckIn,
            Self::Confirmed => InviteLoungeUpdate::UserConfirmed,
        }
    }
}

fn status(status: InviteStatus) -> InviteLoungeUpdate {
    InviteLoungeUpdate::Status { status }
}

fn problem(problem: InviteProblem) -> InviteLoungeUpdate {
    InviteLoungeUpdate::Problem { problem }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum SteamWait {
    Delivery,
    Retry,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RequestStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) channel_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) notices: Vec<Notice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) wait: Option<SteamWait>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) check_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) check_in_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) check_in_message_id: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) invite_now: bool,
}

impl RequestStatus {
    pub(super) fn for_channel(channel_id: u64) -> Self {
        Self {
            channel_id: Some(channel_id),
            ..Self::default()
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub(super) fn claim(&mut self, notice: Option<Notice>) -> Option<Notice> {
        let notice = notice.filter(|notice| !self.notices.contains(notice))?;
        self.notices.push(notice);
        Some(notice)
    }

    pub(super) fn asked(&self) -> bool {
        self.notices.contains(&Notice::CheckIn)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InviteReply {
    FriendRequest,
    Invited,
    AlreadyInvited,
    InvalidCode,
    Rejected,
    Busy,
    Unreachable,
    Unclear,
}

pub(super) fn classify_invite_reply(result: &Result<String, String>) -> InviteReply {
    let Ok(text) = result else {
        return InviteReply::Unreachable;
    };
    let text = text.trim_start();
    if text.starts_with('📨') {
        InviteReply::FriendRequest
    } else if text.starts_with('🎮') {
        InviteReply::Invited
    } else if text.starts_with("ℹ️") || text.starts_with('✅') {
        InviteReply::AlreadyInvited
    } else if text.starts_with('❌') {
        InviteReply::InvalidCode
    } else if text.starts_with("Steam hat die Einladung abgelehnt") {
        InviteReply::Rejected
    } else if text.starts_with('⏳') {
        InviteReply::Busy
    } else if text.starts_with("⚠️") {
        InviteReply::Unreachable
    } else {
        InviteReply::Unclear
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SteamOutcome {
    Sent,
    AlreadyHasGame,
    Rejected,
    Expired,
}

pub(super) fn steam_outcome(decision: &str, code: Option<i64>) -> Option<SteamOutcome> {
    match decision {
        "sent" if code == Some(5) => Some(SteamOutcome::AlreadyHasGame),
        "sent" => Some(SteamOutcome::Sent),
        "rejected" => Some(SteamOutcome::Rejected),
        "timeout" => Some(SteamOutcome::Expired),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SteamProgress {
    pub(super) outcome: Option<SteamOutcome>,
    pub(super) invited: bool,
    pub(super) open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Finish(Option<Notice>),
    Wait {
        wait: SteamWait,
        at: i64,
        notice: Option<Notice>,
    },
    Dispatch,
    Expire,
}

pub(super) fn after_invite(reply: InviteReply, now: i64) -> Step {
    let delivery = |notice| Step::Wait {
        wait: SteamWait::Delivery,
        at: now + FOLLOW_UP_SECONDS,
        notice,
    };
    match reply {
        InviteReply::FriendRequest => delivery(Some(Notice::FriendRequest)),
        InviteReply::Busy => delivery(None),
        InviteReply::Unreachable => Step::Wait {
            wait: SteamWait::Retry,
            at: now + RETRY_SECONDS,
            notice: None,
        },
        InviteReply::Invited => Step::Finish(Some(Notice::Invite)),
        InviteReply::AlreadyInvited => Step::Finish(Some(Notice::AlreadyInvited)),
        InviteReply::InvalidCode => Step::Finish(Some(Notice::InvalidCode)),
        InviteReply::Rejected => Step::Finish(Some(Notice::Rejected)),
        InviteReply::Unclear => Step::Finish(None),
    }
}

pub(super) fn after_progress(wait: SteamWait, progress: SteamProgress, now: i64) -> Step {
    match progress.outcome {
        Some(SteamOutcome::Sent) => Step::Finish(Some(Notice::Invite)),
        Some(SteamOutcome::AlreadyHasGame) => Step::Finish(Some(Notice::AlreadyInvited)),
        Some(SteamOutcome::Rejected) => Step::Finish(Some(Notice::Rejected)),
        Some(SteamOutcome::Expired) => Step::Finish(Some(Notice::NotAccepted)),
        None if progress.open => Step::Wait {
            wait: SteamWait::Delivery,
            at: now + FOLLOW_UP_SECONDS,
            notice: None,
        },
        None if progress.invited => Step::Finish(Some(Notice::Invite)),
        None => match wait {
            SteamWait::Retry => Step::Dispatch,
            SteamWait::Delivery => Step::Finish(None),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CheckIn {
    AlreadyInvited,
    Skip,
    Ask,
}

pub(super) fn check_in_step(invited: bool, answered: bool) -> CheckIn {
    if invited {
        CheckIn::AlreadyInvited
    } else if answered {
        CheckIn::Skip
    } else {
        CheckIn::Ask
    }
}

pub(super) fn check_in_window(age: i64, room_wait: i64) -> bool {
    (CHECK_IN_SECONDS..room_wait).contains(&age)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000;

    fn reply(text: &str) -> InviteReply {
        classify_invite_reply(&Ok(text.to_string()))
    }

    #[test]
    fn steam_texte_ergeben_den_gemeldeten_zustand() {
        assert_eq!(
            reply("📨 Freundschaftsanfrage ist raus. Sag ihm, er soll sie auf Steam annehmen."),
            InviteReply::FriendRequest
        );
        assert_eq!(reply("🎮 Ihr seid schon Steam-Freunde, ich hab die Einladung direkt angestoßen.\nAntwort von Steam: OK"), InviteReply::Invited);
        assert_eq!(
            reply("ℹ️ Dieser Steam-Account wurde schon eingeladen, am <t:1700000000:D>."),
            InviteReply::AlreadyInvited
        );
        assert_eq!(
            reply(
                "✅ Die Einladung war schon raus, mir hat nur der Eintrag in der Historie gefehlt."
            ),
            InviteReply::AlreadyInvited
        );
        assert_eq!(
            reply("❌ Das ist kein gültiger Steam-Freundescode."),
            InviteReply::InvalidCode
        );
        assert_eq!(
            reply("Steam hat die Einladung abgelehnt.\nAntwort von Steam: x"),
            InviteReply::Rejected
        );
        assert_eq!(
            reply("⏳ Steam braucht noch länger für die Einladung."),
            InviteReply::Busy
        );
        assert_eq!(
            reply("⚠️ Ich konnte gerade nicht prüfen, ob ihr Steam-Freunde seid."),
            InviteReply::Unreachable
        );
        assert_eq!(
            classify_invite_reply(&Err("weg".into())),
            InviteReply::Unreachable
        );
        assert_eq!(reply("Irgendwas anderes"), InviteReply::Unclear);
    }

    #[test]
    fn freundschaftsanfrage_meldet_fa_und_wartet_auf_den_bot() {
        assert_eq!(
            after_invite(InviteReply::FriendRequest, NOW),
            Step::Wait {
                wait: SteamWait::Delivery,
                at: NOW + FOLLOW_UP_SECONDS,
                notice: Some(Notice::FriendRequest)
            }
        );
        assert_eq!(
            Notice::FriendRequest.update(),
            InviteLoungeUpdate::Status {
                status: InviteStatus::FriendshipMissing
            }
        );
    }

    #[test]
    fn einladung_raus_und_bereits_eingeladen_beenden_mit_einer_meldung() {
        assert_eq!(
            after_invite(InviteReply::Invited, NOW),
            Step::Finish(Some(Notice::Invite))
        );
        assert_eq!(
            after_invite(InviteReply::AlreadyInvited, NOW),
            Step::Finish(Some(Notice::AlreadyInvited))
        );
        assert_eq!(
            Notice::Invite.update(),
            InviteLoungeUpdate::Status {
                status: InviteStatus::Sent
            }
        );
        assert_eq!(
            Notice::AlreadyInvited.update(),
            InviteLoungeUpdate::Status {
                status: InviteStatus::AlreadyHasGame
            }
        );
    }

    #[test]
    fn langsames_steam_meldet_nichts_und_bleibt_offen() {
        for reply in [InviteReply::Busy, InviteReply::Unreachable] {
            let Step::Wait { notice, at, .. } = after_invite(reply, NOW) else {
                panic!("{reply:?} muss offen bleiben");
            };
            assert_eq!(notice, None);
            assert!(at > NOW);
        }
        let open = SteamProgress {
            open: true,
            ..SteamProgress::default()
        };
        for wait in [SteamWait::Delivery, SteamWait::Retry] {
            assert_eq!(
                after_progress(wait, open, NOW),
                Step::Wait {
                    wait: SteamWait::Delivery,
                    at: NOW + FOLLOW_UP_SECONDS,
                    notice: None
                }
            );
        }
        assert_eq!(
            after_progress(SteamWait::Retry, SteamProgress::default(), NOW),
            Step::Dispatch
        );
    }

    #[test]
    fn bot_schritt_nach_der_wartezeit_ergibt_die_naechste_meldung() {
        let outcome = |outcome| SteamProgress {
            outcome: Some(outcome),
            open: true,
            invited: false,
        };
        assert_eq!(
            after_progress(SteamWait::Delivery, outcome(SteamOutcome::Sent), NOW),
            Step::Finish(Some(Notice::Invite))
        );
        assert_eq!(
            after_progress(
                SteamWait::Delivery,
                outcome(SteamOutcome::AlreadyHasGame),
                NOW
            ),
            Step::Finish(Some(Notice::AlreadyInvited))
        );
        assert_eq!(
            after_progress(SteamWait::Delivery, outcome(SteamOutcome::Rejected), NOW),
            Step::Finish(Some(Notice::Rejected))
        );
        assert_eq!(
            after_progress(SteamWait::Delivery, outcome(SteamOutcome::Expired), NOW),
            Step::Finish(Some(Notice::NotAccepted))
        );
        assert_eq!(steam_outcome("sent", Some(0)), Some(SteamOutcome::Sent));
        assert_eq!(
            steam_outcome("sent", Some(5)),
            Some(SteamOutcome::AlreadyHasGame)
        );
        assert_eq!(
            steam_outcome("rejected", Some(4)),
            Some(SteamOutcome::Rejected)
        );
        assert_eq!(steam_outcome("timeout", None), Some(SteamOutcome::Expired));
        assert_eq!(steam_outcome("failed", None), None);
    }

    #[test]
    fn probleme_werden_konkret_gemeldet() {
        assert_eq!(
            after_invite(InviteReply::InvalidCode, NOW),
            Step::Finish(Some(Notice::InvalidCode))
        );
        assert_eq!(
            after_invite(InviteReply::Rejected, NOW),
            Step::Finish(Some(Notice::Rejected))
        );
        assert_eq!(
            Notice::Rejected.update(),
            InviteLoungeUpdate::Problem {
                problem: InviteProblem::SteamRejected
            }
        );
        assert_eq!(
            Notice::NotAccepted.update(),
            InviteLoungeUpdate::Problem {
                problem: InviteProblem::FriendRequestNotAccepted
            }
        );
    }

    #[test]
    fn jede_meldung_hoechstens_einmal_je_anfrage() {
        let mut status = RequestStatus::for_channel(1);
        assert_eq!(
            status.claim(Some(Notice::FriendRequest)),
            Some(Notice::FriendRequest)
        );
        assert_eq!(status.claim(Some(Notice::FriendRequest)), None);
        assert_eq!(status.claim(None), None);
        assert_eq!(status.notices, vec![Notice::FriendRequest]);
    }

    #[test]
    fn nachfrage_nach_dreissig_minuten_nur_ohne_antwort_und_ohne_einladung() {
        assert!(!check_in_window(CHECK_IN_SECONDS - 1, 3600));
        assert!(check_in_window(CHECK_IN_SECONDS, 3600));
        assert!(!check_in_window(3600, 3600));
        assert_eq!(check_in_step(false, false), CheckIn::Ask);
        assert_eq!(check_in_step(false, true), CheckIn::Skip);
        assert_eq!(check_in_step(true, false), CheckIn::AlreadyInvited);
        assert_eq!(check_in_step(true, true), CheckIn::AlreadyInvited);
        assert_eq!(Notice::CheckIn.update(), InviteLoungeUpdate::CheckIn);
    }

    #[test]
    fn bestaetigung_meldet_den_eingang() {
        assert_eq!(
            Notice::Confirmed.update(),
            InviteLoungeUpdate::UserConfirmed
        );
    }
}
