# Root cause: supporter role sync stalled on Discord (301cbb0d)

## Layer 1, mechanism (reproduced)

The supporter role tick awaited the guild member snapshot and every grant or revoke inline, without a deadline. One Discord call that never answered blocked the tick and every later cycle, so no supporter got or lost the role until a restart.

Reproduced in `rust/bin/dl-bot/src/supporter/roles.rs` tests against a local HTTP server that accepts and never answers: `hanging_member_snapshot_times_out_and_later_cycles_retry`, `hanging_and_failed_roles_allow_other_members_and_later_cycles`, `rate_limiter_waits_are_bounded_and_later_cycles_proceed`. Each test drives the production tick into a call that never answers and requires the tick and the next cycle to finish; all pass on this branch (ionos, throwaway database, 2026-11-15, log `Documents/.tasks/2026-11-13-supporter-recovery/fix-20261115-p1-tests.log`).

## Layer 2, why it was possible

`DiscordAdapter::new(token)` built `serenity::http::Http::new(token)`. Serenity 0.12.5 builds that on a `reqwest::Client` with no connect and no request timeout. `build_client` and `build_voice_worker_client` used `serenity::Client::builder(token, ...)`, which creates another such `Http`. The configured `runtime.bridges` HTTP limits (`dl_bridges::http::HttpLimits`) never reached any Discord REST call, and nothing in the adapter's signature showed that.

## Layer 3, why nobody noticed

- Every Discord test either mocks the call, which answers at once, or never sends a request. No test stalled a Discord response before 301cbb0d.
- Serenity's default looks safe, because no type or log line says "no timeout".
- The review of 301cbb0d checked only the supporter path that the commit changed. 301cbb0d wrapped single calls in `tokio::time::timeout` and left the client below unbounded.
- The member snapshot shared one per-request budget across all pages.

## Layer 4, where else the pattern lives

All twins named by the gate review use either `adapter.http` or the gateway `Http`. ac6a2daa fixed them all by building both through `bounded_http` from the `runtime.bridges` reqwest client:

| Place | Fixed by |
|---|---|
| `dl-discord/src/adapter.rs` 55, 764, 810, 1074 | ac6a2daa, adapter `Http` on the bounded client |
| `dl-bot/src/modglue.rs` 2648, 3433, 3805, 3824, 3842, 4413, 4659, 4672 | ac6a2daa, same adapter `Http`; both member scans bound each page |
| `dl-voice/src/glue.rs` 1283, 1302 | ac6a2daa, same adapter `Http` |
| `dl-bot/src/serversync.rs` 1188, 1718 | ac6a2daa, per-page bound in `guild_members::fetch_all` |
| `dl-discord/src/gateway.rs` 225, 241, 373, 562 (`ctx.http`) | ac6a2daa, `ClientBuilder::new_with_http(adapter.rest_http(token), ...)` |
| voice worker gateway identity | ac6a2daa, `build_voice_worker_client` takes `adapter.rest_http(&voice_worker_token)` |

Every other `reqwest::Client::builder()` in `rust/` outside tests was read on this branch, and each one sets `.timeout`. That covers dl-dashboard (`server_stats.rs:633`, `public.rs:92`, `visual_brain.rs:436`, `authority.rs:97`, `names.rs:244`), dl-ai (`lib.rs:230,295,534,612`, `chat_provider.rs:1073`, `tool_chat/anthropic_http.rs:45`), `dl-tierlist/lib.rs:46`, `dl-answer/game_http.rs:70`, `dl-webcore/dashboard.rs:43`, dl-bot `scrim_adapter.rs:2544,2610`, dl-mcp `main.rs:69`, `dl-community-points-sync/lib.rs:66`, `dl-verbinder/main.rs:886` and dl-bridges `http.rs`. The bare `reqwest::Client::new()` calls are all in test code. Outside tests, no `Http::new(` or `serenity::Client::builder(` is left in `rust/`; the only `HttpBuilder::new` is `bounded_http`.

Two things stay open, and both are recorded in `claude-config/orchestrierung/OFFENE-URSACHEN.md` on 2026-11-15.

- The serenity rate limiter loops on 429 with `retry_after` sleeps that the client timeout does not bound. Only the supporter paths wrap their calls in an outer deadline.
- Discord attachment uploads now share the configured request limit. That is 15 s by default and adjustable up to 120 s through `runtime.bridges.http_request_timeout_ms`.

The open row from 2026-11-13 still applies to this pattern: per-key locks are held across Discord calls.

## Related fixes on this branch, 2026-11-15

| Commit | Class |
|---|---|
| 6f58ff79 | Supporter migrations used session `SET lock_timeout`. They now use `SET LOCAL`, and a guard test covers every migration. |
| dcba93b3 | Twitch roles had full DML on `core.supporter_sources`. twitchbot now has no grant. twitchdash gets column grants plus the RLS policy `supporter_sources_twitch_rows`. |
| ac6a2daa | Discord REST runs on the configured HTTP limits (this cause). |
| 949b5826 | The Twitch internal API timeout follows the validated range. |
| fd125266 | The `/invite` wait matches the 2 × 90 s Steam task budget. |
| 46f5fb11, a78e8b2a | The Twitch range test now asserts the parsed value. The lock_timeout guard also sees commented and `set_config` forms. |
