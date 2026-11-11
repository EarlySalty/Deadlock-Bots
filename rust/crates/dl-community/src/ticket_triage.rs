use dl_moderation::action_policy::{ModerationAction, PolicyDecision};
use dl_moderation::content_analyzer::ReportedScreenshotFailure;
use dl_moderation::moderation_system::ReportedScreenshotReview;
use dl_moderation::moderation_verdict::ModerationCategory;

pub const TICKET_MAX_IMAGES: usize = 4;
pub const TICKET_MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ACCOUNT_REFS: usize = 3;
const NAME_LABELS: &[&str] = &[
    "user",
    "username",
    "benutzer",
    "benutzername",
    "nutzer",
    "nutzername",
    "name",
    "account",
    "acc",
    "discord",
    "discordname",
    "tag",
];
const MODERATION_EXACT: &[&str] = &["ban", "bann", "banned", "acc", "mute", "muted"];
const MODERATION_PREFIXES: &[&str] = &[
    "scam",
    "betrug",
    "betrüg",
    "phish",
    "hack",
    "gehackt",
    "gebannt",
    "entbann",
    "unban",
    "report",
    "gemeldet",
    "meldung",
    "melden",
    "account",
    "konto",
    "timeout",
    "gemutet",
    "beleidig",
    "beläst",
    "fake",
    "gestohlen",
    "geklaut",
];
const SCAM_PREFIXES: &[&str] = &[
    "scam",
    "betrug",
    "betrüg",
    "phish",
    "fake",
    "gehackt",
    "hacker",
    "gestohlen",
    "geklaut",
    "reportet",
    "ausversehen",
    "versehentlich",
];
const HUMAN_EXACT: &[&str] = &[
    "mod",
    "mods",
    "moderator",
    "moderatoren",
    "moderation",
    "admin",
    "admins",
    "supporter",
    "mensch",
    "menschen",
    "teammitglied",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketImage {
    pub url: String,
    pub size: u64,
}

impl TicketImage {
    pub fn within_limits(&self) -> bool {
        self.size <= TICKET_MAX_IMAGE_BYTES && !self.url.trim().is_empty()
    }
}

#[async_trait::async_trait]
pub trait ScreenshotReviewer: Send + Sync {
    async fn review_reported_screenshots(
        &self,
        image_urls: Vec<String>,
    ) -> Result<ReportedScreenshotReview, ReportedScreenshotFailure>;
}

#[async_trait::async_trait]
impl ScreenshotReviewer for dl_moderation::ModerationSystem {
    async fn review_reported_screenshots(
        &self,
        image_urls: Vec<String>,
    ) -> Result<ReportedScreenshotReview, ReportedScreenshotFailure> {
        dl_moderation::ModerationSystem::review_reported_screenshots(self, image_urls).await
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScreenshotCheck {
    Disabled,
    NoImages,
    Failed(ReportedScreenshotFailure),
    Reviewed(ReportedScreenshotReview),
}

impl ScreenshotCheck {
    pub fn from_result(
        result: Result<ReportedScreenshotReview, ReportedScreenshotFailure>,
    ) -> Self {
        match result {
            Ok(review) => Self::Reviewed(review),
            Err(ReportedScreenshotFailure::NoImages) => Self::NoImages,
            Err(failure) => Self::Failed(failure),
        }
    }

    fn line(&self) -> String {
        match self {
            Self::Disabled => "Bildprüfung: nicht gelaufen (deaktiviert)".to_string(),
            Self::NoImages => "Bildprüfung: keine Bilder".to_string(),
            Self::Failed(failure) => format!("Bildprüfung fehlgeschlagen ({})", failure.label()),
            Self::Reviewed(review) => verdict_line(review),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationReason {
    BrainNoAnswer,
    BrainUnavailable,
    ModerationCase,
    HumanRequested,
}

impl EscalationReason {
    fn label(self) -> &'static str {
        match self {
            Self::BrainNoAnswer => "Brain ohne belegte Antwort",
            Self::BrainUnavailable => "Brain nicht erreichbar",
            Self::ModerationCase => "Moderations-, Scam-, Bann- oder Accountfall",
            Self::HumanRequested => "Mitglied fragt nach einem Menschen",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escalation {
    pub reasons: Vec<EscalationReason>,
}

impl Escalation {
    pub fn needs_mod(&self) -> bool {
        !self.reasons.is_empty()
    }
}

pub fn escalation(brain_answered: bool, brain_reachable: bool, text: &str) -> Escalation {
    let words = words(text);
    let mut reasons = Vec::new();
    if !brain_reachable {
        reasons.push(EscalationReason::BrainUnavailable);
    } else if !brain_answered {
        reasons.push(EscalationReason::BrainNoAnswer);
    }
    if words.iter().any(|word| is_moderation_word(word)) {
        reasons.push(EscalationReason::ModerationCase);
    }
    if words
        .iter()
        .any(|word| HUMAN_EXACT.contains(&word.as_str()))
    {
        reasons.push(EscalationReason::HumanRequested);
    }
    Escalation { reasons }
}

pub fn is_scam_report(text: &str) -> bool {
    words(text)
        .iter()
        .any(|word| SCAM_PREFIXES.iter().any(|prefix| word.starts_with(prefix)))
}

fn is_moderation_word(word: &str) -> bool {
    MODERATION_EXACT.contains(&word)
        || MODERATION_PREFIXES
            .iter()
            .any(|prefix| word.starts_with(prefix))
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountRef {
    Id(u64),
    Name(String),
}

impl AccountRef {
    fn label(&self) -> String {
        match self {
            Self::Id(id) => id.to_string(),
            Self::Name(name) => name.clone(),
        }
    }
}

pub fn reported_account_refs(text: &str, opener_id: u64) -> Vec<AccountRef> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut refs = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        for candidate in token_refs(token, tokens.get(index + 1).copied()) {
            if candidate != AccountRef::Id(opener_id) && !refs.contains(&candidate) {
                refs.push(candidate);
            }
        }
    }
    refs.truncate(MAX_ACCOUNT_REFS);
    refs
}

fn token_refs(token: &str, next: Option<&str>) -> Vec<AccountRef> {
    let mentions = mention_ids(token);
    if !mentions.is_empty() {
        return mentions.into_iter().map(AccountRef::Id).collect();
    }
    if let Some(id) = snowflake(token.trim_matches(|c: char| !c.is_ascii_digit())) {
        return vec![AccountRef::Id(id)];
    }
    if let Some(name) = token.strip_prefix('@').and_then(username) {
        return vec![AccountRef::Name(name)];
    }
    labelled_name(token, next)
        .map(AccountRef::Name)
        .into_iter()
        .collect()
}

fn labelled_name(token: &str, next: Option<&str>) -> Option<String> {
    let (label, rest) = token.split_once(':')?;
    let label = label
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    if !NAME_LABELS.contains(&label.as_str()) {
        return None;
    }
    let value = if rest.trim().is_empty() { next? } else { rest };
    username(value.trim_start_matches('@'))
}

fn mention_ids(token: &str) -> Vec<u64> {
    token
        .split("<@")
        .skip(1)
        .filter_map(|part| {
            let part = part.strip_prefix('!').unwrap_or(part);
            let (digits, _) = part.split_once('>')?;
            snowflake(digits)
        })
        .collect()
}

fn snowflake(digits: &str) -> Option<u64> {
    if !(17..=20).contains(&digits.len()) || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn username(raw: &str) -> Option<String> {
    let name = raw
        .trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .trim_end_matches('.')
        .to_lowercase();
    let valid = (2..=32).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
        && !matches!(name.as_str(), "everyone" | "here");
    valid.then_some(name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberLookup {
    Member(u64),
    NotMember,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAccount {
    pub reference: AccountRef,
    pub lookup: MemberLookup,
}

impl ResolvedAccount {
    fn line(&self) -> String {
        let status = match self.lookup {
            MemberLookup::Member(id) => format!("Mitglied {id}"),
            MemberLookup::NotMember => "kein Mitglied".to_string(),
            MemberLookup::Unknown => "Mitgliedschaft unbekannt".to_string(),
        };
        format!("{} → {status}", self.reference.label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadowAction {
    WouldBan(u64),
    NoBan(&'static str),
}

impl ShadowAction {
    pub fn line(&self) -> String {
        match self {
            Self::WouldBan(id) => format!("would ban {id}"),
            Self::NoBan(reason) => format!("kein Bann ({reason})"),
        }
    }
}

pub fn shadow_action(
    screenshot: &ScreenshotCheck,
    accounts: &[ResolvedAccount],
    opener_id: u64,
) -> ShadowAction {
    let review = match screenshot {
        ScreenshotCheck::Reviewed(review) => review,
        ScreenshotCheck::Disabled => return ShadowAction::NoBan("Bildprüfung deaktiviert"),
        ScreenshotCheck::NoImages => return ShadowAction::NoBan("keine Bilder"),
        ScreenshotCheck::Failed(_) => return ShadowAction::NoBan("Bildprüfung fehlgeschlagen"),
    };
    if !matches!(review.decision, PolicyDecision::AutoExecute { .. })
        || !review.verdict.verification.confirmed
    {
        return ShadowAction::NoBan("Policy würde nicht automatisch handeln");
    }
    if review.verdict.verification.category != ModerationCategory::Scam {
        return ShadowAction::NoBan("Bild zeigt keinen bestätigten Scam");
    }
    let mut members: Vec<u64> = accounts
        .iter()
        .filter_map(|account| match account.lookup {
            MemberLookup::Member(id) if id != opener_id => Some(id),
            _ => None,
        })
        .collect();
    members.sort_unstable();
    members.dedup();
    match members.as_slice() {
        [id] => ShadowAction::WouldBan(*id),
        [] => ShadowAction::NoBan("gemeldeter Account nicht als Mitglied gefunden"),
        _ => ShadowAction::NoBan("mehrere Mitglieder genannt"),
    }
}

fn policy_label(decision: &PolicyDecision) -> String {
    match decision {
        PolicyDecision::Ignore => "ignore".to_string(),
        PolicyDecision::Proposal { .. } => "proposal".to_string(),
        PolicyDecision::AutoExecute { action, .. } => {
            let action = match action {
                ModerationAction::DeleteOnly => "delete",
                ModerationAction::Timeout => "timeout",
                ModerationAction::Ban => "ban",
            };
            format!("auto_execute ({action})")
        }
    }
}

pub struct ScamSection<'a> {
    pub image_count: usize,
    pub screenshot: &'a ScreenshotCheck,
    pub accounts: &'a [ResolvedAccount],
    pub action: &'a ShadowAction,
}

pub struct ShadowEntry<'a> {
    pub ticket_channel_id: u64,
    pub brain_verdict: &'a str,
    pub escalation: &'a Escalation,
    pub scam: Option<ScamSection<'a>>,
    pub candidate: &'a str,
}

pub fn shadow_entry_text(entry: &ShadowEntry<'_>) -> String {
    let mut lines = vec![
        "🧪 **Ticket-Shadow**".to_string(),
        format!("Ticket: <#{}>", entry.ticket_channel_id),
        format!("Urteil: {}", entry.brain_verdict),
        needs_mod_line(entry.escalation),
    ];
    if let Some(scam) = &entry.scam {
        lines.extend(scam_lines(scam));
    }
    lines.push(String::new());
    lines.push(format!("Kandidat:\n{}", entry.candidate));
    lines.join("\n")
}

fn needs_mod_line(escalation: &Escalation) -> String {
    if !escalation.needs_mod() {
        return "needs_mod: nein".to_string();
    }
    let reasons: Vec<&str> = escalation
        .reasons
        .iter()
        .map(|reason| reason.label())
        .collect();
    format!("needs_mod: ja ({})", reasons.join("; "))
}

fn scam_lines(scam: &ScamSection<'_>) -> Vec<String> {
    let mut lines = vec![format!("Scam-Meldung: ja, Bilder: {}", scam.image_count)];
    lines.push(scam.screenshot.line());
    if scam.accounts.is_empty() {
        lines.push("Gemeldeter Account: keiner erkannt".to_string());
    }
    for account in scam.accounts {
        lines.push(format!("Gemeldeter Account: {}", account.line()));
    }
    lines.push(format!("Aktion (Shadow): {}", scam.action.line()));
    lines
}

fn verdict_line(review: &ReportedScreenshotReview) -> String {
    let verification = &review.verdict.verification;
    format!(
        "Bildprüfung: {}, Sicherheit {:.2}, bestätigt {}, Policy {}\nBegründung: {}",
        verification.category.as_label(),
        verification.confidence,
        if verification.confirmed { "ja" } else { "nein" },
        policy_label(&review.decision),
        verification.reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_moderation::moderation_verdict::{
        ContentAnalysis, ModerationVerdict, VerificationDecision,
    };

    const REAL_SCAM_TICKET: &str = "Heyho, leider nun auch bei euch unterwegs: Der \"Ich hab dich ausversehen auf Discord reportet\"-Betrug. Man wird gebeten Kontakt zu einem \"Discord-Support-Mitarbeiter\" aufzunehmen. Leider hab ich ihn schon bei Discord gemeldet und glaube keinen Zugriff mehr auf die Nachrichten. Demnach keinen Beweis mehr. Sollte noch jemand weiteres sich bei euch melden, habt ihr mit mir eine 2. Bestätigung\n\nUsername: franklygab";

    fn review(category: ModerationCategory, confirmed: bool, auto: bool) -> ScreenshotCheck {
        let decision = if auto {
            PolicyDecision::AutoExecute {
                action: ModerationAction::Timeout,
                timeout_minutes: 1440,
            }
        } else {
            PolicyDecision::Ignore
        };
        ScreenshotCheck::Reviewed(ReportedScreenshotReview {
            verdict: ModerationVerdict {
                analysis: ContentAnalysis {
                    category: category.clone(),
                    confidence: 0.95,
                    reason: "Analyse".to_string(),
                    raw_json: String::new(),
                },
                verification: VerificationDecision {
                    confirmed,
                    category,
                    confidence: 0.93,
                    reason: "Fake-Support fordert Kontaktaufnahme".to_string(),
                    raw_json: String::new(),
                },
                trigger: String::new(),
            },
            decision,
        })
    }

    fn member(id: u64) -> ResolvedAccount {
        ResolvedAccount {
            reference: AccountRef::Id(id),
            lookup: MemberLookup::Member(id),
        }
    }

    #[test]
    fn echter_scam_ticket_text_liefert_username_und_scam() {
        assert_eq!(
            reported_account_refs(REAL_SCAM_TICKET, 411895243357683714),
            vec![AccountRef::Name("franklygab".to_string())]
        );
        assert!(is_scam_report(REAL_SCAM_TICKET));
        let escalation = escalation(true, true, REAL_SCAM_TICKET);
        assert_eq!(escalation.reasons, vec![EscalationReason::ModerationCase]);
    }

    #[test]
    fn account_aus_mention_id_und_at_name_ohne_opener() {
        let text = "<@!123456789012345678> und 223456789012345678, auch @Scammer.Max. Ich bin <@411895243357683714>";
        assert_eq!(
            reported_account_refs(text, 411895243357683714),
            vec![
                AccountRef::Id(123456789012345678),
                AccountRef::Id(223456789012345678),
                AccountRef::Name("scammer.max".to_string()),
            ]
        );
        assert_eq!(
            reported_account_refs("Name:\n@foo_bar", 1),
            vec![AccountRef::Name("foo_bar".to_string())]
        );
        assert!(reported_account_refs("@everyone 12345", 1).is_empty());
    }

    #[test]
    fn needs_mod_gruende() {
        assert!(!escalation(true, true, "Wie stelle ich die Sprache um?").needs_mod());
        assert_eq!(
            escalation(false, true, "Frage").reasons,
            vec![EscalationReason::BrainNoAnswer]
        );
        assert_eq!(
            escalation(false, false, "Kann ein Mod bitte schauen?").reasons,
            vec![
                EscalationReason::BrainUnavailable,
                EscalationReason::HumanRequested
            ]
        );
        assert_eq!(
            escalation(true, true, "Warum wurde ich gebannt?").reasons,
            vec![EscalationReason::ModerationCase]
        );
        assert!(!escalation(true, true, "Ich spiele in einer Band").needs_mod());
    }

    #[test]
    fn would_ban_nur_bei_bestaetigtem_scam_und_genau_einem_fremden_mitglied() {
        let scam = review(ModerationCategory::Scam, true, true);
        assert_eq!(
            shadow_action(&scam, &[member(7)], 1),
            ShadowAction::WouldBan(7)
        );
        assert!(matches!(
            shadow_action(&scam, &[member(1)], 1),
            ShadowAction::NoBan(_)
        ));
        assert!(matches!(
            shadow_action(&scam, &[member(7), member(8)], 1),
            ShadowAction::NoBan(_)
        ));
        let not_member = ResolvedAccount {
            reference: AccountRef::Name("franklygab".to_string()),
            lookup: MemberLookup::NotMember,
        };
        assert!(matches!(
            shadow_action(&scam, &[not_member], 1),
            ShadowAction::NoBan(_)
        ));
        for other in [
            review(ModerationCategory::Scam, false, true),
            review(ModerationCategory::Scam, true, false),
            review(ModerationCategory::NsfwExplicit, true, true),
        ] {
            assert!(matches!(
                shadow_action(&other, &[member(7)], 1),
                ShadowAction::NoBan(_)
            ));
        }
        for state in [
            ScreenshotCheck::Disabled,
            ScreenshotCheck::NoImages,
            ScreenshotCheck::Failed(ReportedScreenshotFailure::AnalysisFailed),
        ] {
            assert!(matches!(
                shadow_action(&state, &[member(7)], 1),
                ShadowAction::NoBan(_)
            ));
        }
    }

    #[test]
    fn derselbe_gemeldete_account_ueber_id_und_name_zaehlt_einmal() {
        let scam = review(ModerationCategory::Scam, true, true);
        let not_member = ResolvedAccount {
            reference: AccountRef::Id(8),
            lookup: MemberLookup::NotMember,
        };
        let by_name = ResolvedAccount {
            reference: AccountRef::Name("scammer".to_string()),
            lookup: MemberLookup::Member(7),
        };
        assert_eq!(
            shadow_action(&scam, &[member(7), not_member, by_name.clone()], 1),
            ShadowAction::WouldBan(7)
        );
        let other_member = ResolvedAccount {
            reference: AccountRef::Name("zweiter".to_string()),
            lookup: MemberLookup::Member(8),
        };
        assert!(matches!(
            shadow_action(&scam, &[member(7), other_member, by_name], 1),
            ShadowAction::NoBan("mehrere Mitglieder genannt")
        ));
    }

    #[test]
    fn fehlgeschlagene_bildpruefung_ist_kein_urteil() {
        let failed = ScreenshotCheck::from_result(Err(ReportedScreenshotFailure::AnalysisFailed));
        let action = shadow_action(&failed, &[member(7)], 1);
        let text = scam_lines(&ScamSection {
            image_count: 1,
            screenshot: &failed,
            accounts: &[member(7)],
            action: &action,
        })
        .join("\n");
        assert!(text.contains("Bildprüfung fehlgeschlagen"), "{text}");
        assert!(!text.contains("Policy"), "{text}");
        assert!(!text.contains("Bildprüfung: other"), "{text}");
        assert_eq!(action, ShadowAction::NoBan("Bildprüfung fehlgeschlagen"));
        let disabled = scam_lines(&ScamSection {
            image_count: 1,
            screenshot: &ScreenshotCheck::Disabled,
            accounts: &[],
            action: &ShadowAction::NoBan("Bildprüfung deaktiviert"),
        })
        .join("\n");
        assert!(disabled.contains("Bildprüfung: nicht gelaufen (deaktiviert)"));
    }

    #[test]
    fn bilder_ueber_groessenlimit_fallen_raus() {
        let ok = TicketImage {
            url: "https://cdn/x.png".to_string(),
            size: TICKET_MAX_IMAGE_BYTES,
        };
        let big = TicketImage {
            url: "https://cdn/y.png".to_string(),
            size: TICKET_MAX_IMAGE_BYTES + 1,
        };
        assert!(ok.within_limits());
        assert!(!big.within_limits());
    }
}
