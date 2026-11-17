# Invite lounge KI intent: handover

State 2026-11-17, gate round 3 fixer.

- Branch `feat/invite-lounge-ki-intent-20261116`, pins Brain 0ba41619. HEAD: see `git log -1`.
- Local baseline reds (need a database, not caused by this branch): 4 dl-community DSN-only tests (reaction_roles runtime_gate, privacy runtime_gate, two scrim_signup) and 1 dl-bot faq test (service_faq_v2_repost_loescht_fremde_message_nicht).

## Done in round 3

- Turn path: every failed read (privacy, context, steam link, invite audit) fails the turn and the catch-up retries it; live order per author; start time repaired before each catch-up; catch-up window equals the request window.
- One home: open phases from `Phase::OPEN`, privacy predicate `PRIVACY_HIDDEN_PREDICATE`, one Discord epoch constant; hidden members leave the tick.

## Open

1. B3, B4, B6, B7, B8, E1, E4 to E8, F1 texts, F2 to F5, G, H, broker MessageNotFound handler test, tour length in characters.
2. Full checks, DSN suite for dl-community and dl-central-db on ionos, publish, report.
3. Integrator: re-pin to the final Brain head, then gate round 3.
