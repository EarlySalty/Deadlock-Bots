# Code review and task flow

This file is the binding review process for this repository. It applies to every
contributor and every coding agent (Claude Code, Codex, others). `AGENTS.md` and
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

The exact commands for this repository are in section 11.

## 3. Round 1: one deep review with fixed fan-out

- Review the frozen branch SHA against `origin/main`. Reviewers are read-only
  (read, grep, glob, read-only git). They may read outside the diff to follow callers
  and dependencies.
- Fan-out is fixed and does not grow with diff size:
  - small change (one subsystem, roughly under 300 changed lines): 1 reviewer
  - default: 2 reviewers
    - **Correctness and boundaries**: logic, edge values, error paths, API and DB contracts
    - **Security, concurrency and state**: auth, injection, secrets, locks, tasks,
      retries, shared state, migrations
  - at most 3, and only with a stated reason
- Depth is hypothesis driven, not a matrix. Check an input class only where the
  code touches it:
  - concurrency: locks, tasks, channels, shared state
  - retry and idempotency: queues, network calls, webhooks
  - null, missing and malformed data: API payloads, serde, DB rows
  - overflow and limits: sizes, indices, counters, pagination
  - time: time zones, expiry, ordering
- Reviewers run in a minimal runtime: no inherited skills, plugins, hooks, MCP
  servers or personal `CLAUDE.md` (for Claude Code: `claude --bare` or
  `CLAUDE_CODE_SIMPLE=1`). The review prompt carries the policy it needs.

## 4. Severity: only high blocks

| Severity | Meaning | Effect |
|---|---|---|
| high | A concrete failure scenario: input or state X leads to wrong output, crash, data loss, security hole or rule violation from section 8 | blocks merge |
| medium | Real weakness without a concrete failing scenario today | fix if cheap, else note in the task |
| low | Style, naming, nice-to-have | hint only, never blocks |

A finding without a concrete scenario and a `file:line` anchor is not high.
Each finding states: file and line, failure scenario, root cause, class. Findings
with the same root cause are merged into one.

## 5. After round 1 the findings are frozen

- The round-1 findings are the only review agenda from now on.
- The fixer fixes each finding and its whole class, reruns section 2 and reviews
  its own diff before asking for verification.
- Every follow-up round is **one verification reviewer** that reads the open
  findings and the fix diff and returns one state per finding:
  `FIXED`, `STILL`, `FALSE` (finding was wrong) or `NEW_REGRESSION`.
- A new finding is only allowed when the fix itself introduces a defect on a line
  the fix changed.
- A new full review happens only when the scope changes: a new subsystem, a new
  external interface, auth or security code, a data migration, or an explicit
  deep-review request. Never because of the round count.

## 6. Technical failures are not rounds

- Malformed reviewer output gets one format-only repair call without repo tools.
  It never triggers a second code reading.
- After two technical failures the run ends as `REVIEW_INCONCLUSIVE` and goes to
  the maintainer.
- Runs without a verdict (tool failure, usage limit) do not count as rounds.

## 7. Round limit

Aim for ALLOW in one or two rounds. Never rerun a review just to see what is left.
After four rounds with a verdict and no ALLOW: write a short retro, triage real
findings against trivia, get one independent second opinion, do one clean fixer
pass, run one final round. If that still blocks, stop and hand the retro, the open
findings and the decision needed to the maintainer.

## 8. House rules the review checks

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

## 9. Merge, deploy, done

- Work on your own branch (`<name>/<slug>`), never on `main`. Commit verified
  intermediate states and push the branch. No force push to shared branches, no
  bypassed hooks.
- Hand over with a branch and a short report: goal, what changed, check results,
  review rounds and final states, open points.
- Only the maintainer merges to `main`, after the maintainer's local review gate says
  ALLOW, then deploys and checks the live function. ALLOW is not acceptance: the
  result is also judged against the goal and the rules above.
- "Done" means live and verified, or an explicitly agreed handover point.

## 10. Orchestrator prompt

Use this as the start prompt when one agent takes a task and drives it to a finished,
reviewed branch. Replace the placeholders.

```text
You are the orchestrator for this task in <repo>. Read CODE-REVIEW.md and follow it.

Task: <what the maintainer asked for, verbatim>

1. Write .tasks/<date>-<slug>/TASK.md: goal, boundaries, one checkable acceptance
   criterion. For a bug: the four cause layers with evidence.
2. Search the repo for existing code that already solves part of it.
3. Implement on your own branch in small vertical steps. Run the checks from
   section 2 and commit each verified state.
4. Run review round 1 with the fixed fan-out from section 3. Fix every high finding
   and its class. Verify with one verification reviewer until all findings are
   FIXED or FALSE. Respect the round limit.
5. Check the acceptance criterion against real data or the real runtime.
6. Push the branch and report: goal, changes, checks, review rounds, open points.

Decisions:
- Decide everything reversible yourself (naming, structure, libraries already in
  the repo, internal APIs). Do not ask "shall I?".
- For a real product or architecture decision, ask once: the question, 2 to 3
  options, your recommendation with the reason, and the default you will take.
  Keep working on everything the question does not block.
- If nobody answers within 20 minutes, take your recommended option. Record it in
  .tasks/<date>-<slug>/DECISIONS.md (question, options, choice, reason, how to
  revert) and continue. Mention it in the final report.
- The 20-minute default never applies to: deleting data or anything else
  irreversible, secrets and credentials, money, bans, messages to many users,
  anything published in the maintainer's name, production deploys, breaking
  changes to external APIs. Those wait for an explicit answer; park them in
  DECISIONS.md as OPEN and finish the rest.
```

## 11. Checks for this repository

Rust workspace in `rust/`:

```bash
cd rust
cargo fmt --all -- --check
SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings
SQLX_OFFLINE=true cargo build --workspace
SQLX_OFFLINE=true cargo test -p <touched-crate>
```

`rust/scripts/check.sh` runs format, clippy and tests in one go. DB tests use `rust/scripts/central_test_db.sh`; never point tests at a live database.
