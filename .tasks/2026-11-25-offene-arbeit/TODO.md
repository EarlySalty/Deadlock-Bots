# TODO: sweep 2026-11-25, Deadlock-Bots

Resume point for a fresh or resumed orchestrator. Read `STATE.md` first.

## Done

- A0 environment recorded in `STATE.md`.
- A1 inventory of the main repo (32 worktrees, 73 branches, 0 stashes, 4 PRs).
- A2 part 1: all local branch tips and worktree HEADs are on origin (5 pushes, table in `STATE.md`).

## Open, in this order

1. A2 part 2 (Haiku job H1, see `REGISTER.md`): wip snapshots of the 8 dirty worktrees, untracked work product, inventory and securing of the 5 related clones. Result expected in `A2-SECURE.md`.
2. A3 mechanical table (Haiku job H2): landed-by-content per branch, result in `A3-CONTENT-TABLE.md`.
3. A3 judgement (Opus jobs O1 to O3, one at a time): supporter series plus live release; November branches (invite lounge, Brain, ticket scam, access gate, gateway, steam timeout, repostats); September and October branches, PRs, related clones. Results in `TRIAGE-<topic>.md`, merged into `TRIAGE.md`.
4. A4 cleanup of proven LANDED and SUPERSEDED items. Redo the 60-minute writer check without depth limit first.
5. A5 `DECISIONS.md`.
6. B0 check (real gate verdict, build path). Expected: gate still broken, then park with `bin/park-for-gate.sh deadlock-bots`, status file, `claim-next.sh`.

## Known high priority

- Live `dl-bot` runs release `59a0ac5f` from the supporter slim line, not on main, and lacks main commits `#479` and `#480`.

## Rules to remember

- PATH export in every command. Pushes: own Bash call, explicit refspec, no pipe, no redirection.
- Max two subagents, max one Opus, second slot Haiku only. No replacement after a limit stop.
