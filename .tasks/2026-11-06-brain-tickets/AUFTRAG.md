# Brain on support tickets: diagnosis and fix brief (2026-11-06)

## Live evidence (journal deadlock-bot-rust.service, log channel 1374364800817303632, last 14 days)

7 tickets reached `FaqChat::handle_ticket_message` (rust/crates/dl-community/src/faq.rs:1549). 5 got a shadow candidate, 2 got nothing. None of the 5 candidates would have helped:

| Ticket | What the member wrote | Brain shadow candidate |
|---|---|---|
| 1568234664785612841 | "moin ich würde gerne jemand einen person reporten" (ID and screenshots followed in later messages) | Explains how to report a rule violation via support, lists forbidden things |
| 1567500249134669824 | Scam DM report | "Dafür habe ich leider keine belegten Infos. Ein gehackter Discord-Account ... gehört in den sichtbaren Menschen-Support" |
| 1567507300216737894 | Detailed scam report with username | Long text about the chat moderation bot, "melde das dem Menschen-Support" |
| 1564817340443529287 | Moderation complaint | "Am besten meldest du das dem Moderationsteam oder nutzt den sichtbaren Supportweg" |
| 1563994365804679410 | Greeting / unclear | Generic Concierge self-introduction listing all bots and Paten |
| 1566470519308423349 | "mit einem Freund an den Scrims teilnehmen, nur 2-3 Leute, können wir in ein Team?" | No answer (uncertain). Ticket still has no human reply after 5 days |
| 1563712968661803088 | (not inspected) | No answer |

## Root causes

1. Wrong capability. Tickets call the Brain with `DiscordAnswerCapability::Faq` (faq.rs `lookup_bound_with_history`), the same shape as the private FAQ chat. The Brain therefore answers as a knowledge FAQ and redirects the member to "Menschen-Support" although they are already inside a support ticket. The `_context` argument is ignored.
2. Moderation reports are treated as knowledge questions. 4 of 5 candidates were scam/report tickets; the right first reply is a short intake acknowledgement plus asking for missing facts (who, what, proof), never a policy lecture.
3. Only the first member message is evaluated (KV claim `faq_ticket_shadow_claims`). Members split their problem over several messages within seconds ("ich will jemanden reporten" then ID then screenshots), so the Brain sees a fragment.
4. The ticket-specific style prompt `TICKET_CANDIDATE_SYSTEM_PROMPT` (faq.rs:43) is `#[cfg(test)]` only. In production `ticket_candidate` returns the raw FAQ answer, so none of those rules apply.
5. Knowledge gap: scrim question for duos/trios without a team got no answer.

## Goal

The Brain produces ticket-appropriate first replies in the shadow channel. It stays shadow only; posting into member tickets is a separate product decision and is NOT part of this task.

## Scope

A. Deadlock-Brain (worktree /home/nathanael/.worktrees/brain-ticket-capability-20261106, branch fix/brain-ticket-capability-20261106)
- Add `DiscordAnswerCapability::Ticket` (serde `ticket`) in rust/crates/brain-contracts/src/discord_task.rs and purpose `bot_task:ticket` everywhere `bot_task:faq` is handled (brain-contracts provider_input.rs, brain-kernel lib.rs, brain-api lib.rs, brain-serve discord_live.rs). Same privacy and permission handling as `faq`.
- Ticket answer shape (in the existing answer shape contract where faq/concierge shapes live, look for ANSWER_SHAPE_CONTRACT): the member is already talking to the human team in a ticket. Never send them to support, a ticket, or the moderation team. Moderation, scam, report, ban or account cases: only confirm the team will look at it and ask for exactly the missing facts (user name or ID, what happened, screenshots), no policy recitation, no promises. Real knowledge questions: answer briefly from evidence, like the FAQ. Two to four short sentences, du-form, German.
- Existing tests must keep passing; add a contract test only for the new capability round trip.

B. Deadlock-Bots (worktree /home/nathanael/.worktrees/bots-ticket-capability-20261106, branch fix/brain-ticket-capability-20261106)
- Bump `brain-client` rev in rust/crates/dl-brain/Cargo.toml to the Brain branch head commit once A is committed and pushed.
- faq.rs ticket path uses `DiscordAnswerCapability::Ticket`.
- Collect the opening: after the first member message, wait a quiet window (about 45 s without a new member message, max 3 min) and send all member messages of that window joined as the question. Ignore bot authors (Ticket Tool). Keep the KV claim so each ticket is evaluated once.
- Delete the dead `#[cfg(test)]` ticket generator path (`TICKET_CANDIDATE_SYSTEM_PROMPT`, `ticket_candidate_request`, `fixture_ticket_candidate`, `ticket_generator`) if nothing in production uses it; its rules move into the Brain ticket shape.
- Shadow channel and fail-closed guards stay unchanged.

## Out of scope
- Posting into member tickets, new knowledge pages, any Python code, model or provider changes, deploys and merges to main.

## Acceptance
- Both workspaces build; `cargo fmt --check`, `cargo clippy`, and the existing tests of the touched crates pass (Rust builds per /home/nathanael/Documents/claude-config/machines/v50671/RUST-BUILDS.md, use cargo-slot).
- Replaying the 5 ticket openings above through the Ticket capability (dl-brain example `discord_task_probe` or brain-api test) gives no "Menschen-Support / Ticket öffnen" redirect for any of them, and an intake acknowledgement asking for missing facts for the 4 moderation cases.
- Both branches committed and pushed; report commit SHAs and the probe outputs in WORKER-ERGEBNIS.md next to this file.
