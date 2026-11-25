# STATE: sweep 2026-11-25, Deadlock-Bots

Orchestrator: Fable, native session in `/home/nathanael/repos/Deadlock-Bots`. Started 2026-11-25 08:17 CET.
Briefs: `/home/nathanael/Documents/claude-config/.tasks/2026-11-25-offene-arbeit-sweep/BRIEF-COMMON.md` and `repos/deadlock-bots/BRIEF.md`.

## A0 environment (08:17 to 08:30)

- Tools exist in `~/.local/bin` (`gate-review`, `cargo-slot`, `graphify`, `moli`, `deploy-queue`, `gitleaks`) and `~/.cargo/bin` (`cargo`, `rustc`, `clippy`, `rustfmt`), but neither directory is on PATH of this session. Every command needs `export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"`. `sccache` not found on PATH or in those two dirs (check before Phase B).
- `gate-review --help` prints usage. That proves nothing for B0; a real verdict run is still untested (sandbox issue, thread `c17c27f9`).
- `git fetch --prune` works. `gh pr list` works.
- Restore: no `tar`, `rsync` or `restic` process, load about 5. No file changed in the last 60 minutes in any worktree (checked to depth 4 only; redo without depth limit before any A4 removal).
- Live writers at 08:20: one `claude` process with cwd in the repo root (this session). None in other worktrees.
- Push hook: a push command must be its own Bash call with no redirection and no pipe, and with an explicit refspec `git -C <repo> push origin <branch>:refs/heads/<name>`.

## Live state

- `deadlock-bot-rust.service` (system unit, MainPID 160357, active since 2026-11-24 12:16) runs `/opt/deadlock/bots/current/dl-bot`.
- `/opt/deadlock/bots/current` -> `releases/59a0ac5fbeac8f6397761cbc2ffdb0a38f3e2398` (link set 2026-11-16 10:38).
- `59a0ac5f` is NOT on `origin/main` (`86679e6c`). It sits on the supporter slim line, 2 commits behind main (`#479`, `#480` are missing live) and differs from main in 36 files. Before this sweep its only carrier was the local-only branch `backup/supporter-p1-slim-pre-reword-r4`; now pushed.
- Consequence: production runs unmerged supporter code and lacks the two newest Brain fixes. Highest priority item.

## A1 inventory

Fresh inventory: 32 worktrees, 73 local branches, 0 stashes, 4 open PRs (#450, #451, #459 draft, #475). No change against the early-morning snapshot except that all tips are now on origin. Raw list: scratchpad `inventory-fresh.txt` (copied into TRIAGE.md once judged).

All five related clones exist (inventory pending, Haiku job H1).

## A2 secured so far (orchestrator, 08:25 to 08:30)

| Local ref | Pushed as | Note |
|---|---|---|
| `backup/supporter-p1-slim-pre-reword-r4` (4209dfc62) | same name | carries live release `59a0ac5f` |
| `backup/supporter-p1-pre-reword-r3` (b2277dd34) | same name | |
| `backup/ticket-scam-shadow-pre-retro` (8b038e447) | same name | |
| `sol/abschluss/7bf0e0375ee34a00` (e3e649ccc) | same name | also covers two detached pruefverbund worktrees |
| `feat/access-gate-20261110` (e5a617a21) | `sicherung/sweep-20261125/feat/access-gate-20261110` | diverged from origin branch of the same name |

Proof: after the pushes no local branch tip and no worktree HEAD is missing on origin (`git branch -r --contains` for every tip).

Open in A2: wip snapshots of 8 dirty worktrees, untracked work product, related clones (Haiku job H1).

## For other repos

(none yet)
