# TODO: D1b and Brain follow-up (state 2026-11-01)

D1b merged as 24e46ba4 (Deadlock-Bots). dl-bot install still open.
Brain fix merged as 8db68fe5 (Deadlock-Brain), NOT live. Live release is 66989ad9.

Blocker for the release: origin/main also carries hero cards with migration v60 (scripts/migrations/2026-11-01-brain-hero-card-v60.sql) and ops/brain-postgres/site-role.sql, not applied live. brain-release only ships current main, so the release ships them too.

Next:
1. Apply v60 and the role change, or release through the owner of hero cards (see Deadlock-Brain .tasks/2026-10-31-brain-patch-knowledge/TODO.md).
2. brain-release build, verify, install from a clean worktree on origin/main.
3. Rebuild /tmp/d1tools/d1_runtime_proof.rs and rerun the 8 questions; questions are passed as command-line arguments. Expect the patch question Answered and the two smalltalk questions short.
4. Install dl-bot.
