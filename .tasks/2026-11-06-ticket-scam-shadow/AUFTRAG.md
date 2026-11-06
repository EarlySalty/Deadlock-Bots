# Brief: ticket help with mod escalation and scam-report ban (shadow)

## Goal (user, 2026-11-06)
The Brain should help in support tickets and contact a mod only when needed, so the first mod ping is not required. For Steam or Discord scam reports the bot should be able to ban the reported account without a mod, by checking the screenshots the member sends.

## Decisions
- Everything runs in shadow mode first: nothing is posted to members, no ping, no ban. Results go to log channel 1374364800817303632 only. Switching to live is a separate approval.
- Removing the Ticket Tool welcome role ping (<@&1337518124647579661>) is done by the user in the Ticket Tool dashboard, only after the shadow phase.
- Auto-ban target rule after shadow: screenshot clearly shows a scam AND the reported account is identified as a member of the guild.

## Build on
- Ticket path: dl-community faq.rs `TicketOpening`, `wait_ticket_opening`, `evaluate_ticket_opening` (merged with fix/brain-ticket-capability-20261106).
- Scam vision check: dl-moderation `content_analyzer` (`generate_multimodal` through the central dl_ai provider), `ModerationVerdict`, `ActionPolicy` (auto_execute_verified_confidence 0.85), shadow branch `auto_execute_shadow` in moderation_system.rs. Reuse, do not build a second classifier.

## Scope
1. Collector keeps image attachments of the opener (count and size limits, same author only).
2. Per ticket opening, shadow log one entry containing:
   - Brain reply candidate (existing).
   - Escalation decision: `needs_mod` yes/no with reason (Brain Insufficient, moderation or account case, explicit request for a human).
   - For scam reports with images: vision verdict (category, confidence), the reported account resolved from mention, ID or name in text or screenshot, whether it is a guild member, and the action the policy would take ("would ban <id>").
3. No ban, no ping, no member-visible message in this package.

## Acceptance
- Real ticket openings in category 1459628097145147645 produce one shadow log entry each with the three parts above.
- A scam screenshot replay (from an existing closed ticket) yields category scam and a resolved account, logged as "would ban".
- fmt, clippy -D warnings, existing tests green (baseline: 4 DB tests in dl-community need ionos).
