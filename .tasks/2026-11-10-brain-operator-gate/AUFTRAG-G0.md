[Orchestrator] Package G0: the access gate in dl-bot (live Discord rights as the one authority for every Brain action and every internal read)

Role: leaf implementation worker. You do exactly this package and start no further orchestrators or T3 threads.
Client: main orchestrator, T3 thread f958016c-7085-4ee6-aad4-195324030b77. Send the final report or a real blocker with the T3 tool `t3_thread_send` to that thread. No interim messages.
Worktree: this thread starts in the shared checkout /home/nathanael/repos/Deadlock-Bots, which may be stale and must not be edited. First step: `git -C /home/nathanael/repos/Deadlock-Bots fetch origin`, then `git -C /home/nathanael/repos/Deadlock-Bots worktree add -b feat/access-gate-20261110 /home/nathanael/.worktrees/Deadlock-Bots-access-gate-20261110 origin/main`. Work only in /home/nathanael/.worktrees/Deadlock-Bots-access-gate-20261110.

Before the first code line read `AGENTS.md` or `CLAUDE.md` in the worktree, `/home/nathanael/.claude/CLAUDE.md` (binding rules), the Deadlock-Bots rows in `/home/nathanael/Documents/claude-config/orchestrierung/OFFENE-URSACHEN.md`, the memory note `/home/nathanael/.claude/projects/-home-nathanael-Documents/memory/dl-bot-build-und-deploy.md`, and the concept this package implements the first step of: `/home/nathanael/repos/Deadlock-Brain/.tasks/2026-11-10-brain-operator/KONZEPT.md`.

## What the user wants

The Deadlock Brain becomes the central operator: it can query and change what every bot can do, each bot part behind its own MCP connector. The Brain reaches a connector only through one gate that checks the asking user. Rights are what the user holds in Discord at that moment: a role with Administrator may do everything, Manage Events may handle events, the tournament roles may handle tournaments, a normal user can only ask. The model never decides a right.

This package builds that gate. It is the base for every connector and for the Brain side; both are later packages.

## Existing stock (reuse, do not rebuild)

- `rust/bin/dl-bot/src/mcp/public.rs`: `access(st, requested_user)` and `effective_permissions(...)` fetch member and guild roles live and compute channel permissions with overwrites and timeout. This is the only live rights check in the whole stack and the core you extend. Move it to where both the public MCP and the gate use the same code; the public MCP must behave as before.
- `rust/bin/dl-bot/src/mcp.rs`: the HTTP server on 127.0.0.1:8890 with `/mcp` and `/mcp/public`, bearer tokens. The gate routes live on this server.
- `master::is_owner` (used by `/serversync`): the owner check.
- dl-broker `member-access` route: the dashboards use it for the Administrator bit.
- Three places decide "who is admin" today from a snapshot taken at login, with hard-coded role lists that disagree: `rust/crates/dl-dashboard/src/authority.rs` `decide_access` and `config.rs` (`DASHBOARD_ACCESS_ROLE_IDS`), the Twitch bot `discord_admin_login.rs`, Turniere `turnier-auth/src/roles.rs`. You do not change them here; name them in the report as the next consumers of the gate.
- Use `graphify query` and `graphify affected` before grep.

## Goal

1. Access lookup. `POST /gate/v1/access` on the dl-bot server, loopback only, own bearer secret from Infisical (loaded the way the existing MCP tokens are, never printed, fail closed without it). Input: Discord user id, optional channel id. Output: whether the user is a known guild member, owner flag, timeout flag, server-wide permission bits by name, channel permission bits by name when a channel is given, role ids, position of the highest role. Unknown user, failed Discord lookup or timeout of the lookup: an explicit "no rights" answer, never an error the caller could read as allowed. Cache at most 60 seconds per user; a role change must be visible after that at the latest.
2. Access file. One file in dl-bot's config area (follow the existing operating-config conventions, no `.env`), read by the gate only; no MCP tool, no Brain tool and no model prompt may contain or return it. It holds:
   - capabilities, each with: permission bits that grant it (any of), role ids that grant it (any of), `owner_only`, `confirm` (default true for anything that is not a read), `enabled`, `shadow`;
   - areas with the channel ids bound to them (for internal results);
   - Administrator grants every capability that is not `owner_only`; the owner holds everything; a capability that is not in the file is granted to nobody but Administrator and owner.
   Reload on change without restart, and refuse a malformed file by keeping the last good one and logging it. Ship a first file with the areas of the concept table and the live role ids for Turnier Organisation, Turnier Moderation, Scrim Match Leiter, Coach, Team Coach, Streamer, Server Dev, Community Moderator (look the ids up through the existing Discord access; ids are not secrets). Every connector starts with `shadow = true`.
3. Decision. `POST /gate/v1/authorize`. Input: caller name (which connector or the Brain), user id, channel id or DM marker, capability, class (read, write, destructive), internal flag, optional target user id and target role id, a hash of the tool arguments. Output: allowed or not, a machine-readable reason, `confirm_required`, `shadow`, `place_ok`, and a decision id.
   - Discord's hierarchy applies: a target member or role whose position is equal to or above the asker's highest role is refused, the owner excepted.
   - `place_ok` for an internal result: true in a DM with the asker; in a guild channel only if the channel is bound to the capability's area in the access file and nobody outside the holders of that capability can view it, computed from the channel overwrites (if @everyone or the verified base role can view, it is false). False otherwise.
   - No user id, or a user id that did not come from Discord (the caller says so with an explicit origin field: discord_message, discord_interaction, none): only capabilities marked public in the file are allowed.
4. Record. New table through a new migration (never edit applied ones; migration versions are fourteen digits; check origin/main for the next free one right before the commit): one row per decision with time, caller, user id, channel id, capability, class, argument hash, the redacted arguments (bounded size), allowed, reason, shadow. `POST /gate/v1/outcome` lets the connector attach the result to its decision id (ok or error, bounded text). Writes must not delay a decision noticeably: follow how other hot-path audit inserts in this repo are done and never let a failed insert turn a refusal into an allow. A read route for the owner view is a later package; a 180 day purge belongs here.
5. Client crate. A small crate in this workspace with the wire types and one async client (`access`, `authorize`, `outcome`), so the bots that already depend on this repo's crates use the same types. Keep the wire format plain JSON over HTTP so a service without that dependency can call it. Document the contract in one markdown file next to the crate: routes, fields, reasons, and the rule that a caller must treat anything but an explicit allow as a refusal.
6. Out of this package but prepared: the confirm click (a pending action that only the asking user can confirm in Discord, bound to the argument hash, single use, expiring) is package G0b. Design the decision row and the `authorize` input so that a confirmation id can be added without a breaking change, and say in the report what G0b has to add.

## Acceptance (against real data, read only)

- For three real members with different roles (one Moderator, one Community Moderator, one member with only base roles; pick them through the existing member search, name them in the report only by role, not by name or id) the `access` output matches what Discord shows for them, server-wide and in one restricted channel.
- `authorize` answers: base member + `discord.event.write` refused; Streamer role + `discord.event.write` allowed (holds Manage Events); Community Moderator + a ban of a Moderator refused by hierarchy; any member + internal capability in a public channel `place_ok` false; missing token 401; unmapped capability refused for a Moderator without Administrator and allowed for one with it.
- Removing a role in a test fixture changes the answer after the cache time.
- The public MCP still passes its existing tests.

## Rules

- English for code, commits and notes. No code comments. Rust only, without `.unwrap()` in production paths and without needless `unsafe`. Small functions. Persistence in Postgres, new migration files only.
- Never read or print secrets; secrets only through the Infisical loader the repo already uses. No `.env`. No `pgrep -f`.
- No user or community data to any AI service. Discord ids in logs are fine, message content is not part of this package at all.
- Builds and tests only through `cargo-slot` (`/home/nathanael/Documents/claude-config/machines/v50671/RUST-BUILDS.md`, `/home/nathanael/Documents/claude-config/wissen/ionos-builds.md`); DB tests through the `cargo-test-db` path described there, affected crates only. Run fmt and clippy for the touched crates.
- Step discipline: read a file once in full, bundle edits per file, run independent calls in parallel, one chained check command with shortened output, no sleep loops.
- Tests only where they guard a concrete contract: the decision table above, fail closed on lookup failure and on a missing or malformed access file, hierarchy, `place_ok`.
- Merge gate: `gate-review --repo /home/nathanael/.worktrees/Deadlock-Bots-access-gate-20261110 --base origin/main`. One git step per Bash call whenever main is involved, literal paths. On BLOCK fix the class of the finding; start one fresh native subagent per fix round as fixer, otherwise no subagents. After ALLOW: rebase onto current origin/main if needed, gate again if the tree changed, then `git push origin HEAD:main` as its own Bash call. No force push, no bypassed hooks.
- You end at the merge. No release build, no deploy, no restart. Do not commit this file or other task notes.
- A real blocker (rights, secrets, data loss): stop and report the exact failing command and exit code.

## Final report, at most 6 lines

Merged SHA on main, gate verdict, routes and the secret name the integrator must create in Infisical, path of the access file and of the contract document, migration number, what G0b must add, open points. Then `python3 /home/nathanael/Documents/tools/t3-thread.py settle --selbst` as the very last step.
