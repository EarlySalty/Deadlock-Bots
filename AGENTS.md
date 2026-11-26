# Token-Speichergrenze

Für Arbeiten an Authentifizierung und Zugängen gilt `docs/token-storage-db.md`. Bestehende Repositoryregeln bleiben bestehen. Keine Token-Dateien neu anlegen, Konto-Zugänge nur verschlüsselt in der DB und reine Session-/Einmalwerte als Lookup-Hash speichern. Infrastruktur-Schlüssel bleiben im vorhandenen Secret-Manager.

# Code review and task flow

`CODE-REVIEW.md` is binding for every task in this repository. Claude Code loads it
through `CLAUDE.md`; other agents read it before the first step.

| When | What to load or do |
|---|---|
| A new task arrives | Read `CODE-REVIEW.md` sections 1, 3 and 11. Start as orchestrator with the prompt from section 11. Write `.tasks/<date>-<slug>/TASK.md`. |
| Before writing code | Search for existing code. Bug fix: name the four cause layers (section 1). Create branch `<name>/<slug>` from `origin/main`. |
| Delegating code work | Use the `implementer` subagent from `.claude/agents/`. Opus, medium effort, high only for auth, security, concurrency, migrations. The orchestrator spawns as many subagents as the task needs. |
| Before every commit | Run the checks in section 12 of `CODE-REVIEW.md`. No commit while they are red. |
| Code is done and committed | Run `/code-review medium <branch> --fix` (high for risky areas), fix the real findings (section 5), rerun checks, then one `/code-review low <branch>`. Nothing more (section 8). |
| A real product or architecture question | Ask once with options and a recommendation. No answer after 20 minutes: take the recommendation, log it in `DECISIONS.md` (section 11). Never for the excluded cases listed there. |
| Writing user-facing text | Natural German with real umlauts and ß. Links go to `https://deutsche-deadlock-community.de`, never the `admin.` host (section 9). |
| Task finished | Push the branch, open one pull request with the description from section 10, reply with its URL. Never push to `main`; only the maintainer merges. |
