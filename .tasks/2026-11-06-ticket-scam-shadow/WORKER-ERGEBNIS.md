# WORKER-ERGEBNIS: Ticket scam shadow (state 2026-11-12)

## Branch
| Repo | Branch | Remote |
|---|---|---|
| Deadlock-Bots | feat/ticket-scam-shadow-20261107 | pushed; published history up to fbd40262 unchanged |

- `0edeeb35` feature commit (shadow triage, escalation reasons, scam screenshot verdict).
- `2ecf3c26`, `38353890`, `65885803`, `4ef7c633`, `d59239d1`, `0c680fc2`: gate round 1 fixes (published).
- `d3642e1e`: skipped images counted (local reword of e086af40 with an Ursache block before the first push).
- Retro fixer pass: `fix(tickets): judge the whole opening and every reported account`, `fix(moderation): one empty-image guard and no screenshot URLs in logs`, `fix(members): a name search with hits but no exact match is unclear`, `chore(config)`, docs. SHAs: `git log origin/main..HEAD`.

## Scope status
1. Collector keeps the opener's images and text; skipped images and cut text are counted in the entry. Done.
2. One shadow entry per opening in 1374364800817303632 with Brain state, candidate, escalation reasons and, for scam reports, screenshot state, reported accounts, policy action and shadow ban decision. Done.
3. No ban, no ping, no member-visible message. Done: all output goes to the log channel through `FaqGlue::send_message`.

## Checks
See GATE-RETRO.md "Fix state" for the commands and results of the retro fixer pass.

## Open for integrator and user
- Final gate round blocked mechanically by five published commits (GATE-RETRO.md, decision a/b/c).
- Live replay with a real scam screenshot needs the screenshot switch on and the user's approval.
- Policy timeout vs "would ban" wording: decide before live bans.
