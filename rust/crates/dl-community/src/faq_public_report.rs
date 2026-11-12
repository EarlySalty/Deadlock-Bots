use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::{log_ticket_fail_closed, FaqChat, TicketOpening, TICKET_AUTO_HELP_CATEGORY_ID};
use crate::ticket_triage::{self, ShadowSource, TicketImage};

pub(super) const PUBLIC_REPORT_QUIET: Duration = Duration::from_secs(20);
pub(super) const PUBLIC_REPORT_MAX: Duration = Duration::from_secs(90);

type ReportKey = (u64, u64);

pub(super) struct PublicReport {
    guild_id: u64,
    message_id: u64,
    shadow_channel_id: u64,
    opening: TicketOpening,
}

pub(super) type PublicReports = std::sync::Mutex<HashMap<ReportKey, PublicReport>>;

#[derive(Debug, Clone, Copy)]
pub struct PublicMessage<'a> {
    pub guild_id: u64,
    pub channel_id: u64,
    pub message_id: u64,
    pub author_id: u64,
    pub content: &'a str,
    pub images: &'a [TicketImage],
}

impl PublicMessage<'_> {
    fn key(&self) -> ReportKey {
        (self.channel_id, self.author_id)
    }
}

pub struct ReplayMessage {
    pub content: String,
    pub images: Vec<TicketImage>,
}

impl FaqChat {
    fn public_reports(&self) -> std::sync::MutexGuard<'_, HashMap<ReportKey, PublicReport>> {
        self.public_reports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub async fn handle_public_message(self: &Arc<Self>, message: PublicMessage<'_>) {
        if message.guild_id == 0 || self.extend_public_report(&message) {
            return;
        }
        if !ticket_triage::is_public_report_trigger(message.content, message.images.len()) {
            return;
        }
        if self
            .port
            .channel_category(message.guild_id, message.channel_id)
            .await
            == Some(TICKET_AUTO_HELP_CATEGORY_ID)
        {
            return;
        }
        let shadow_channel_id = match self.shadow_target(message.channel_id) {
            Ok(id) => id,
            Err(reason) => {
                log_ticket_fail_closed(
                    reason,
                    message.content.chars().count(),
                    message.channel_id,
                    message.author_id,
                );
                return;
            }
        };
        if !self.open_public_report(&message, shadow_channel_id) {
            return;
        }
        let faq = self.clone();
        let key = message.key();
        tokio::spawn(async move {
            faq.wait_public_report(key).await;
        });
    }

    fn extend_public_report(&self, message: &PublicMessage<'_>) -> bool {
        let mut reports = self.public_reports();
        let Some(report) = reports.get_mut(&message.key()) else {
            return false;
        };
        if report.message_id != message.message_id {
            report.opening.add(
                message.content.trim(),
                message.images,
                tokio::time::Instant::now(),
            );
        }
        true
    }

    fn open_public_report(&self, message: &PublicMessage<'_>, shadow_channel_id: u64) -> bool {
        let now = tokio::time::Instant::now();
        let mut opening = TicketOpening::new(message.author_id, now);
        opening.add(message.content.trim(), message.images, now);
        let mut reports = self.public_reports();
        if reports.contains_key(&message.key()) {
            return false;
        }
        reports.insert(
            message.key(),
            PublicReport {
                guild_id: message.guild_id,
                message_id: message.message_id,
                shadow_channel_id,
                opening,
            },
        );
        true
    }

    async fn wait_public_report(self: Arc<Self>, key: ReportKey) {
        loop {
            tokio::time::sleep(self.ticket_opening_poll).await;
            let ready = {
                let mut reports = self.public_reports();
                let Some(report) = reports.get(&key) else {
                    return;
                };
                if report
                    .opening
                    .is_collecting(self.public_report_quiet, self.public_report_max)
                {
                    None
                } else {
                    reports.remove(&key)
                }
            };
            if let Some(report) = ready {
                self.evaluate_public_report(key.0, report).await;
                return;
            }
        }
    }

    async fn evaluate_public_report(&self, channel_id: u64, report: PublicReport) {
        let source = ShadowSource::PublicReport {
            guild_id: report.guild_id,
            channel_id,
            message_id: report.message_id,
        };
        let author_id = report.opening.author_id;
        let Some(question) = report.opening.question() else {
            return;
        };
        let content = self
            .shadow_report_entry(report.guild_id, source, author_id, &question)
            .await;
        self.send_shadow_entry(source, report.shadow_channel_id, &content)
            .await;
    }

    pub async fn replay_public_report(
        &self,
        source: ShadowSource,
        guild_id: u64,
        author_id: u64,
        messages: &[ReplayMessage],
    ) -> Option<String> {
        let now = tokio::time::Instant::now();
        let mut opening = TicketOpening::new(author_id, now);
        for message in messages {
            opening.add(message.content.trim(), &message.images, now);
        }
        let question = opening.question()?;
        Some(
            self.shadow_report_entry(guild_id, source, author_id, &question)
                .await,
        )
    }
}
