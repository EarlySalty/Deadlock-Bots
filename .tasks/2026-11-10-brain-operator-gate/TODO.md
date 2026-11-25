# G0 access gate: state and next steps

## State (2026-11-12 03:00 Berlin)

- Worktree /home/nathanael/.worktrees/Deadlock-Bots-access-gate-20261110, branch feat/access-gate-20261110, clean, rebased on origin/main 92d9654c. HEAD e5a617a2, 6 commits, nothing pushed:
  4d31d558 feat, 7c81186f timings, ad2edc2a fail-closed, 3782fc4c record/contract/privacy, 34e3b12c errors/single sources/contract tests (round-4 fix), e5a617a2 migration-number test.
- Tests green on 34e3b12c+e5a617a2 before the rebase (dl-bot 399, dl-community 314, dl-core 66, dl-access-gate 4); the rebase only pulled a dl-voice change.
- Gate rounds with a verdict: 1 BLOCK, 2 BLOCK, 3 BLOCK, 4 BLOCK (2026-11-12 02:56). Runs lost to quota or rejected JSON are not rounds.
- Round 4 verdict saved in full: gate-r4-verdict.txt in this folder (280 BLOCKING lines, many are hook-generated per-unit input-class lines). Headline findings from the reviewer:
  1. Shipped config/bot.toml decision timeout is 3x the lookup bound, but authorize can make 4 plus up to 25 sequential lookups, so the decision budget cannot cover the worst case.
  2. Shipped config/access-gate.toml grants two write capabilities to anyone holding any one of several distinct rights (too broad any-of grant).
  3. CONTRACT.md lists a binary that does not read bot.toml.
  4. Twins named but not covered by the Ursache block: dl-bot mcp.rs:792, dl-mcp main.rs:137, 379, 448, gate/tests.rs:448, gate/record.rs:89, gate.rs:465, dl-server-as-code diff.rs:790, CONTRACT.md:58.
  5. Unit input-class misses: with_public_token, with_gate, ensure_distinct_tokens (empty), discord_call / wait_fits / discord_call_within (retry, max_overflow), DiscordHttpError (null_missing, error_path), access.rs effective_permissions (null_missing), from_config (max_overflow), and the new tests at mcp.rs:1514, 1958, 2027.
- Old T3 threads mcp:launch:2d9c0cd5... and mcp:launch:ab8015ca... are settled. Client thread for the final report: f958016c-7085-4ee6-aad4-195324030b77.

## Next step (after the 06:20 Berlin quota reset), per ABLAUF.md "Gate rounds and the round-four review"

Round 4 blocked, so do not go to the user yet:
1. Load `gate-learning`, write a ledger line per finding class.
2. Write GATE-RETRO.md in this folder: why four rounds, each open finding from gate-r4-verdict.txt triaged REAL vs TRIVIA with a reason, fix plan per REAL class including twins.
3. One Fable ask_for_help second opinion on the retro (compact question plus evidence).
4. One fresh native fixer pass for all REAL findings (Ursache block format 1.-4., tests via cargo-slot and test-db-run deadlock_test, fmt, clippy); drop real trivia.
5. Fetch origin/main, rebase if moved, run the final gate round: `gate-review --repo /home/nathanael/.worktrees/Deadlock-Bots-access-gate-20261110 --base origin/main` (no timeout wrapper).
6. ALLOW: publish HEAD to main as its own Bash call (`git -C <worktree> push origin HEAD:main`), `git merge-base --is-ancestor feat/access-gate-20261110 origin/main`, remove worktree, local and remote branch, final report (max 6 lines, brief format in AUFTRAG-G0.md) via t3_thread_send to f958016c-7085-4ee6-aad4-195324030b77, then settle this thread.
   BLOCK on the final round: stop and report remaining findings, attempted fixes and the needed decision to the user.
