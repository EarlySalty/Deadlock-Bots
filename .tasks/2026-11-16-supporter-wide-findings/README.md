# Supporter P1 wide-branch findings backlog

Source: merge gate round 4 on `feat/supporter-p1-20261111` at c6e0b6c9 (2026-11-16), 566 BLOCKING lines. The supporter package was rebuilt slim on `feat/supporter-p1-slim-20261116`; the timeout and config rework stays on `backup/supporter-p1-wide-c6e0b6c9`. These findings are kept here so they can be fixed later as their own packages.

## Files

- `P1-SECOND-OPINION.md`: triage by Fable 5.2. Use section 3:
  - REAL-A in supporter scope: fixed in the slim rebuild, recheck there.
  - REAL-A caused by the rework: only relevant if the rework comes back from the backup branch.
  - Pre-existing on main: open bugs on main today, fix as own packages.
  - REAL-B and TRIVIA.
- `p1-r4-findings.txt`: all raw gate lines (file:line | severity | finding).

## Open on main today (start here)

| Where | Defect |
|---|---|
| rust/bin/dl-bot/src/serversync.rs:4672, 4369, 4914, 5058, 6194 | Static panel reposts write message ids and metadata in separate statements; a crash between them leaves a stale id (five panels share the path). |
| rust/bin/dl-mcp/src/main.rs:371-395 | Discord 429 wait is unbounded (`inf` or huge retry_after saturates the sleep). |
| rust/crates/dl-central-db/migrations/2026072411_scrim_result_refs_contract.sql:193 | Scrim result trigger takes an advisory lock after row locks; lock-order deadlock risk. New migration needed. |
| migration 20261021120000 | Session-level lock_timeout leaks past the migration. New migration needed. |
| rust/bin/dl-bot/src/serversync/twitch_link.rs:79 | Authorize URL not checked. |
| rust/bin/dl-bot/src/vanity.rs:374 area | Vanity directory does not honour the privacy opt-out for existing rows. |
| repo-wide | About 61 hard-coded timeout literals; hand-copied cursor pagers in invite_lounge and reaction_roles. |

Each fix follows ABLAUF.md "Ursache statt Symptom" and goes through the merge gate.
