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

## Amendment 2026-11-12 (user): public scam reports, shadow
Trigger example: general channel 1289721245281292291, message 1570393213364076626: a member pings mod role 1337518124647579661 with a screenshot and "Bannt mal die Person / Scammer".
1. Any guild message that pings role 1337518124647579661 (or says scam/scammer with an image) starts the same review path as a ticket opening (reuse ticket_triage + review_reported_screenshots, no second classifier). Collect the reporter's images from that message and their follow-ups within a short window.
2. Resolve the reported account also from the screenshot (vision extracts display/user name or ID), then member lookup as for tickets. Same "would ban" rule.
3. Brain writes the reply it would post: short, casual German like a human mod ("Danke, hab ich mir angeschaut, ist raus."), no bot formatting, no "Ich habe Ihre Meldung erhalten". Through the central Brain path and its answer-shape rules.
4. Shadow only: log channel 1374364800817303632 gets source link, reply candidate, verdict, resolved account, would-ban. No post, ping or ban. Respect `moderation.ticket_screenshot_review` switch (default false).
Acceptance: replay of message 1570393213364076626 (and 1570396144607957032) gives one shadow entry with a natural reply candidate and a resolved account or a clear German reason why not.
