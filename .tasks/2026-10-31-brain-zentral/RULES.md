# Common rules for all "Brain zentral" packages (2026-10-31)

- Work only in your own worktree and branch (given in the briefing). Never touch the main checkout, other worktrees or dirty checkouts.
- Rust only. No code comments. Small functions, vertical implementation, no scope widening. No `.unwrap()` and no needless `unsafe` in production paths.
- AI calls only through the Brain (bots) or the Brain's central provider (Brain). No new connectors, no hard-coded model names or timeouts, no model or cost changes.
- End-user text (Discord, Twitch, dashboard) in natural German with real umlauts and ß. Everything else (code, commits, notes, replies) in English. No em dashes anywhere.
- Secrets: never read, print, log or commit secrets. No `cat`/`grep`/`source` on secret files, no `/proc/*/environ`, no `systemctl show Environment`, no unredacted DB output. If a secret is needed for a check, use `eval "$(/home/nathanael/Documents/Infisical/export_claude_secret.py --secret NAME)"` for the single command and `unset NAME` afterwards.
- Builds follow `/home/nathanael/Documents/claude-config/machines/v50671/RUST-BUILDS.md` (shared sccache, `cargo-slot`). DB-backed tests run only on ionos; the local host refuses them.
- Tests: existing relevant tests must pass. Add tests only for a concrete bug or an independently checkable contract. Remove tests that assert the legacy behaviour you delete.
- Browser checks only with Moli (`/home/nathanael/.local/bin/moli`, read `claude-config/wissen/agent-browser.md` first). Never start Brave.
- Commit verified intermediate states on your branch and publish them to origin promptly. No force updates, no bypassed hooks, no broken commits. Do not commit `.tasks/` files.
- Finish: run `gate-review --repo <your worktree> --base origin/main` (skill `rolle-merge-schleuse`). On BLOCK fix the cause and rerun until ALLOW. After ALLOW merge to main yourself (fast-forward, refspec `HEAD:main`), publish, then stop. Do not deploy and do not restart services; the orchestrator integrates and deploys.
- Report back in your thread: what changed, commit SHAs on main, gate result, what you verified and how, open points. No invented success.
- Ownership of the delta: lines you move, rewrite or newly wire are yours and follow every rule above; "it was already there" does not excuse a hard-coded timeout, a comment or a silent discard in a path you rewrote. A final report never lists open points that violate these rules: fix them or report a blocker with the reason. The gate checks comments, `.unwrap()`, timeout literals, model names and new lines over 150 KB mechanically before the critic.
