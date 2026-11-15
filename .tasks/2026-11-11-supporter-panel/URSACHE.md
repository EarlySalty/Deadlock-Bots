# Root cause: supporter role sync stalled on Discord

Commit subjects below name the commits, because SHAs change when commit messages are reworded.

## Layer 1, mechanism (reproduced)

The supporter role tick awaited the guild member snapshot and every grant or revoke inline, without a deadline. One Discord call that never answered blocked the tick and every later cycle, so no supporter got or lost the role until a restart.

Reproduced in the tests of `rust/bin/dl-bot/src/supporter/roles.rs`, which call the production functions `fetch_members` and `apply_roles` directly, not the whole tick:

- `hanging_member_snapshot_times_out_and_later_cycles_retry`: a local server never answers the first two member page requests; `fetch_members` must return an error within the limit both times and succeed on the third call.
- `hanging_and_failed_roles_allow_other_members_and_later_cycles`: one role change hangs, one fails, the rest answer `204`; `apply_roles` must finish and still change the other members.
- `rate_limiter_waits_are_bounded_and_later_cycles_proceed`: the server answers `204` at once, but the test holds the serenity rate limiter's route lock, so the call waits inside serenity; `apply_roles` must still finish within the limit, and a later call proceeds once the lock is released.

All three pass on this branch (ionos, throwaway database, 2026-11-15, log `Documents/.tasks/2026-11-13-supporter-recovery/fix-20261115-p1-tests.log`).

## Layer 2, why it was possible

`DiscordAdapter::new(token)` built `serenity::http::Http::new(token)`. Serenity 0.12.5 builds that on a `reqwest::Client` with no connect and no request timeout. `build_client` and `build_voice_worker_client` used `serenity::Client::builder(token, ...)`, which creates another such `Http`. The configured `runtime.bridges` HTTP limits (`dl_bridges::http::HttpLimits`) never reached any Discord REST call, and nothing in the adapter's signature showed that.

## Layer 3, why nobody noticed

- Every Discord test either mocked the call, which answers at once, or never sent a request. No test stalled a Discord response before "fix(supporter): bound Discord snapshots and role changes".
- Serenity's default looks safe, because no type or log line says "no timeout".
- The review of that first fix checked only the supporter path it changed. It wrapped single calls in `tokio::time::timeout` and left the client below unbounded.
- The member snapshot shared one per-request budget across all pages.

## Layer 4, where else the pattern lives

All Discord twins named by the gate review use either `adapter.http` or the gateway `Http`. "fix(discord): every Discord REST call runs on the configured HTTP limits" builds both through `bounded_http` from the `runtime.bridges` reqwest client:

| Place | Fixed by |
|---|---|
| `dl-discord/src/adapter.rs` `DiscordAdapter::new`, `rest_http` | adapter `Http` on the bounded client |
| `dl-bot/src/modglue.rs` member scans, role and message calls | same adapter `Http`; both member scans bound each page through `guild_members::fetch_all` |
| `dl-voice/src/glue.rs` Discord calls | same adapter `Http` |
| `dl-discord/src/gateway.rs` `ctx.http` | `ClientBuilder::new_with_http(adapter.rest_http(token), ...)` |
| voice worker gateway identity | `build_voice_worker_client` takes `adapter.rest_http(&voice_worker_token)` |

Outer deadlines, because the client timeout does not cut serenity rate limiter waits:

- Member lists: `dl_discord::member_pages::fetch_all` bounds every page with the request limit. Callers: serversync member reads, both modglue member scans, the supporter snapshot (`supporter/roles.rs`) and the broker live scan `DiscordAdapter::live_role_members`, which also bounds its role read.
- Guild model reads: `dl_server_as_code::import::fetch_live_guild_model` bounds the channel read and the role read separately and names the read that ran out (`ServerAsCodeError::DiscordDeadline`). The ten serversync callers and the snapshot import pass the request limit; none wraps both reads in one budget any more.
- Supporter grant and revoke calls in `supporter/roles.rs`.

Plain reqwest clients on the same Discord token (`modglue.rs` coaching send and edit and evidence mirroring, `dl-voice/src/glue.rs` router payloads and raw history) use `DiscordAdapter::rest_client()`, the bounded client the adapter already holds. Evidence image downloads keep their own 5 s bound per send and per body (`MODERATION_EVIDENCE_DOWNLOAD_TIMEOUT`).

Other reqwest clients outside tests, read on this branch:

| Place | State |
|---|---|
| `dl-dashboard` broker lookup, names, guild stats, public links | one `internal_client` from `DashboardConfig.http_limits`, built in `DashboardApp` and passed to `BrokerMemberLookup::new` and `BrokerNameResolver::new` |
| `dl-webcore/src/dashboard.rs`, `dl-tierlist/src/lib.rs` | client passed in by `dl-web` from `runtime.bridges`; the tierlist keeps its 30 s Deadlock API bound as `DEADLOCK_API_REQUEST_TIMEOUT` |
| `dl-dashboard/src/visual_brain.rs` | 500 ms connect, 5 s request on a loopback call |
| `dl-infisical-env/src/main.rs` | builder without timeout, every request sets `config.timeout` (`lib.rs` `fetch_secrets`) |
| `vendor/uplink-infisical-transport` | returns a builder; both callers (`dl-token-secrets`, `dl-unit-failure-notify`) set a timeout |
| `dl-ai` (`lib.rs` three providers, `chat_provider.rs`, `tool_chat/anthropic_http.rs`) | timeout set; on a failed build `unwrap_or_default` calls `Client::new()`, which panics on the same TLS failure instead of running unbounded; recorded as open |
| `dl-brain/src/api_ingest.rs`, `dl-mcp/src/main.rs`, `dl-answer/src/game_http.rs`, `dl-verbinder/src/transparenz.rs`, `dl-community-points-sync/src/lib.rs` | fixed timeout literals or own constants, outside this branch's diff; recorded as open |

`dl-knowledge`'s `reqwest::Client::new()` sits in an ignored live test. Outside tests, no `Http::new(` or `serenity::Client::builder(` is left in `rust/`; the only `HttpBuilder::new` is `bounded_http`.

Open, recorded in `claude-config/orchestrierung/OFFENE-URSACHEN.md`:

- The serenity rate limiter loops on 429 with `retry_after` sleeps that the client timeout does not bound. Single REST calls in `dl-discord/src/adapter.rs` outside the member and role reads (for example `get_member`, `get_guild`, `get_channel`, `get_user`) still have no outer deadline.
- Discord attachment uploads share the configured request limit (15 s by default, up to 120 s through `runtime.bridges.http_request_timeout_ms`): moderation case evidence, coaching cards, router panel files, serversync banners and `send_attachment_public`. A non-idempotent POST that times out after Discord created the message can be posted again by the next poll.

The open row from 2026-11-13 still applies to this pattern: per-key locks are held across Discord calls.

## Related fixes on this branch

| Commit subject | Class |
|---|---|
| fix(migrations): supporter lock timeouts end with their transaction | Supporter migrations used session `SET lock_timeout`. They now use `SET LOCAL`, and a guard test covers every migration. |
| fix(supporter): Twitch dashboard reaches only its own supporter rows | Twitch roles had full DML on `core.supporter_sources`. twitchbot has no grant. twitchdash gets column grants plus the RLS policy `supporter_sources_twitch_rows`. |
| fix(discord): every Discord REST call runs on the configured HTTP limits | This cause. |
| fix(twitch): internal Twitch API timeout follows the validated range | The Twitch internal API timeout follows the validated range. |
| fix(steam): /invite wait matches the real Steam task budget again | The `/invite` wait matches the 2 × 90 s Steam task budget. |
| fix(migrations): index every subject arm of the Stripe event log reads | Six indexes on `steam.plus_billing_events` for the Steam bot's subscription and tip reads. |
