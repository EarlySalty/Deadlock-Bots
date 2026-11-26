# Code review and task flow

This file is the binding review process for this repository. It applies to every
contributor and every coding agent. Contributors work in their own setup and their own
Claude Code; nothing here depends on the maintainer's machine. Every task ends as one
pull request that has already been reviewed once and fixed. `AGENTS.md` and
`CLAUDE.md` point here.

The process is built from two measured audits of our previous review gate (Nov 2026).
That gate reviewed every unit against every input class, split large diffs into dozens
of reviewer sessions, blocked on low findings and restarted full reviews after fixes.
Result over 14 days: 122 BLOCK against 8 ALLOW, 75 % of all agent usage spent on
reviewers, each reviewer starting at about 60k tokens of inherited context. The rules
below exist to stop exactly that: review deep once, then only verify.

## 1. Before the first line of code

- Write down the goal, the boundaries (what is out of scope) and one checkable
  acceptance criterion. Put it in `.tasks/<YYYY-MM-DD>-<slug>/TASK.md`.
- Search for existing code before building anything new. Reuse beats rebuild.
- Bug fixes name four things with evidence before coding: the mechanism (reproduced),
  why it was possible, why nobody noticed, and where else the same pattern lives.
  A fix covers the whole class inside the branch; twins outside the branch go into
  the task notes as follow-ups.

## 2. Deterministic checks first

No LLM review runs while these are red. They are cheap and they catch the mechanical
problems so the reviewer does not have to:

- formatter check, linter with warnings as errors, build
- the existing tests that touch the change (full suite only before merge)
- `git diff --check` and a secret scan (`gitleaks git --redact` or equivalent)

The exact commands for this repository are in section 12.

## 3. Agents and models

- One orchestrator session per task drives it from start to pull request. It plans,
  delegates and decides; it keeps file dumps out of its own context.
- Code changes run as `implementer` subagents (`.claude/agents/implementer.md`).
- The review is Claude Code's built-in `/code-review` skill. No custom review
  agents, no extra review tooling.
- Model: **Opus only**. Effort: **medium** by default, **high** only for auth,
  security, concurrency, migrations or a task the maintainer marks as hard. Never
  xhigh or max, and never `/code-review max` or `/code-review ultra` unless the
  maintainer asks for it in the task. No Sonnet, no Haiku.
- At most two subagents run at the same time.

## 4. The review

Run it once the checks from section 2 are green and the work is committed on the
task branch:

1. `/code-review medium <branch>` (`/code-review high <branch>` for the risky areas
   listed in section 3). Add `--fix` to let it apply its findings directly.
2. Fix every real finding (section 5) and its whole class with an `implementer`
   run where `--fix` was not enough. Rerun section 2.
3. One confirmation run: `/code-review low <branch>`. Only findings on lines
   the fix changed count here.

## 5. Severity: what must be fixed

| Severity | Meaning | Effect |
|---|---|---|
| high | A concrete failure scenario: input or state X leads to wrong output, crash, data loss, security hole or rule violation from section 9 | fix before the pull request |
| medium | Real weakness without a concrete failing scenario today | fix if cheap, else list in the pull request |
| low | Style, naming, nice-to-have | ignore or list, never blocks |

A finding without a concrete scenario and a `file:line` anchor is not high.
A finding you think is wrong is not fixed; it goes into the pull request as
`FALSE` with one line of reason.

## 6. No review loops

- The first review sets the agenda. The confirmation run checks the fixes; it is
  not a new review of the whole branch.
- A new full review happens only when the scope changed after the first review
  (new subsystem, new external interface, auth or security code, a data migration).

## 7. Technical failures

- A review run that fails for technical reasons (tool error, usage limit) does not
  count. Retry it once. If it fails again, note `REVIEW_INCONCLUSIVE` in the pull
  request and move on.

## 8. Hard budget per task

The old setup ran up to 20 gate rounds per branch and burned the usage without making
the code better. That does not come back. Per task:

- 1 review run and 1 confirmation run, nothing more
- then the pull request is opened, also if something is still open. Open items go
  into the pull request description with finding, state and reason. The maintainer
  decides on them.

Never rerun a review just to see what is left.

## 9. House rules the review checks

- Production and long-running code is Rust. Existing Python is legacy reference
  only; do not extend it. Existing frontend toolchains stay.
- No `.unwrap()` in production paths, no needless `unsafe`, no code comments.
  Small functions with one responsibility and simple interfaces.
- Persistence in Postgres, no new SQLite stores. Never edit an applied migration;
  add a new one.
- No secrets in the repo, in logs, in test output or in commits. No `.env` files.
  If you need a credential, ask the maintainer.
- AI calls go through the existing central provider of the respective service.
  No new connectors, no hard-coded model names or timeouts, no model or cost changes
  without approval.
- Working language for code, commits and internal docs is English. Text that
  community members read (UI, bot messages, website, user-facing errors) is natural
  German with real umlauts and ß.
- User-facing pages and links use `https://deutsche-deadlock-community.de`, never
  the `admin.` host (that host is only for `/admin`, `/twitch/admin`, `/twitch/auth/*`).
- Tests are added only when they guard a concrete bug or a checkable contract.
  A green test you wrote yourself is not proof; check behaviour against real data
  and the real runtime.

## 10. Branch, pull request, done

- One task, one branch (`<name>/<slug>`), one pull request. Never push to `main`.
  Commit verified intermediate states. No force push to shared branches, no
  bypassed hooks.
- Open the pull request with `gh pr create` after the confirmation run. The description holds:
  - goal and acceptance criterion, and whether it was checked against real data
  - what changed
  - check results from section 2
  - review: `/code-review` level used, findings and what was done, open items with reason
  - decisions taken by default (from `DECISIONS.md`) and decisions still open
- Only the maintainer merges. A green review is not acceptance; the result is also
  judged against the goal and the rules above.

## 11. Orchestrator prompt

Start one Claude Code session (Opus, medium effort) per task in a fresh worktree of
this repository and paste this prompt. The maintainer writes task-specific notes
into the task text; they take precedence over this file where they conflict.

```text
You are the orchestrator for this task in <repo>. Read CODE-REVIEW.md and follow it.
You coordinate; `implementer` subagents write code, `/code-review` reviews it.

Task: <the task text from the maintainer, verbatim>

1. Write .tasks/<date>-<slug>/TASK.md: goal, boundaries, one checkable acceptance
   criterion. For a bug: the four cause layers with evidence.
2. Search the repo for existing code that already solves part of it.
3. Create branch <name>/<slug> from origin/main. Implement in small vertical steps
   with `implementer` subagents. Run the checks from section 2 and commit each
   verified state.
4. Review per section 4: `/code-review medium <branch>` (high for risky areas), fix the real
   findings and their class, rerun section 2, then one `/code-review low <branch>` to
   confirm. That is the whole review budget.
5. Check the acceptance criterion against real data or the real runtime.
6. Push the branch and open the pull request with the description from section 10.
   Reply with the pull request URL and one line per open item.

Decisions:
- Decide everything reversible yourself (naming, structure, libraries already in
  the repo, internal APIs). Do not ask "shall I?".
- For a real product or architecture decision, ask once: the question, 2 to 3
  options, your recommendation with the reason, and the default you will take.
  Keep working on everything the question does not block.
- If nobody answers within 20 minutes, take your recommended option. Record it in
  .tasks/<date>-<slug>/DECISIONS.md (question, options, choice, reason, how to
  revert) and continue. List it in the pull request.
- The 20-minute default never applies to: deleting data or anything else
  irreversible, secrets and credentials, money, bans, messages to many users,
  anything published in the maintainer's name, production deploys, breaking
  changes to external APIs. Those wait for an explicit answer; park them in
  DECISIONS.md as OPEN and finish the rest.
```

## 12. Checks for this repository

Rust workspace in `rust/`:

```bash
cd rust
cargo fmt --all -- --check
SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings
SQLX_OFFLINE=true cargo build --workspace
SQLX_OFFLINE=true cargo test -p <touched-crate>
```

`rust/scripts/check.sh` runs format, clippy and tests in one go. DB tests use `rust/scripts/central_test_db.sh`; never point tests at a live database.
