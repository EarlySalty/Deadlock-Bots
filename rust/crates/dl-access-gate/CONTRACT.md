# Access gate contract (v1)

The access gate runs inside dl-bot on the loopback MCP listener (`127.0.0.1:8890` by default). It answers who a Discord user is right now and whether a connector may run a capability for that user. The wire format is plain JSON over HTTP; `dl-access-gate` holds the same types and a client.

## Rule for every caller

Anything other than an explicit allow is a refusal. An answer counts as an allow only with a 2xx status (the gate sends 200), `allowed = true` and `reason = "allowed"`. A transport error, a timeout, any non-2xx status, an unparsable body (including a missing `allowed` or `reason`), an unknown reason or `allowed = false` is a refusal. An allow then splits by its flags: with `shadow = true` the caller only logs what it would do and does not act; otherwise with `confirm_required = true` it acts only after the user confirms; otherwise it acts. In every acting case an internal result is shown only where `place_ok = true`. `GateClient::verdict` and `AuthorizeResponse::verdict` return exactly this as `Verdict::Refused`, `Verdict::Shadow`, `Verdict::NeedsConfirmation { place_ok }` or `Verdict::Act { place_ok }`, so none of these flags can be dropped. The client never uses an environment proxy.

## Authentication

Every route needs `Authorization: Bearer <token>`. The token is the Infisical secret `DISCORD_ACCESS_GATE_TOKEN`, loaded through dl-bot's private snapshot like the other MCP tokens. It must differ from the internal and the public MCP tokens; a reused token stops dl-bot at start. The gate is on only when the secret is set and all six `runtime.start.access_gate_*` keys (see Operating values) are present. A missing secret or a missing key makes dl-bot log a warning and start with the gate off: every route answers `401`. A present key with a value outside its range is not a gate setting but a config error, see Operating values. A missing or wrong token answers `401`; a malformed body answers `400`.

## Operating values

All timings live in `[runtime.start]` of dl-bot's `bot.toml`. dl-core's config validation is the only place that checks ranges. A present value outside its range, or a decision timeout below the lookup timeout, rejects the whole `bot.toml`: every binary that loads that file refuses to start, not only dl-bot (today dl-bot, dl-web, dl-community-points-sync, dl-knowledge, dl-verbinder, dl-central-migrate, dl-mcp and dl-config-check). Only a missing key switches a function off, as the last column says.

| Key | Range | Shipped value in `config/bot.toml` | Missing key |
|---|---|---|---|
| `access_gate_cache_ttl_ms` | 1000 to 3600000 | 60000 | gate off |
| `access_gate_lookup_timeout_ms` | 100 to 60000 | 5000 | gate off |
| `access_gate_decision_timeout_ms` | 100 to 180000, at least the lookup timeout | 15000 | gate off |
| `access_gate_reload_ms` | 500 to 3600000 | 5000 | gate off |
| `access_gate_purge_every_secs` | 60 to 604800 | 21600 | gate off, no purge |
| `access_gate_retention_days` | 1 to 3650 | 180 | gate off, no purge |
| `mcp_public_facts_cache_secs` | 5 to 3600 | 60 | `public_server_facts` is not listed and answers an error |
| `mcp_discord_timeout_ms` | 1000 to 120000 | 60000 | every MCP Discord call of dl-bot and dl-mcp fails before any request leaves the process, so gate lookups answer `lookup_failed` |

Rollout order, because `StartOptions` rejects unknown keys in every binary that reads the shared `bot.toml`: deploy all of those binaries first, then add the eight keys to the live `bot.toml` and install `access-gate.toml` next to it, then restart dl-bot. A dl-bot started without `mcp_discord_timeout_ms` refuses every internal MCP Discord tool that used to run with a fixed 60 s timeout; without `access-gate.toml` every decision is `access_file_unavailable`.

## Routes

### `POST /gate/v1/access`

Request:

| Field | Type | Notes |
|---|---|---|
| `user_id` | string | Discord user id, required |
| `channel_id` | string | optional guild channel id |

Answer (always HTTP 200 once authenticated and well formed):

| Field | Type | Notes |
|---|---|---|
| `status` | `member`, `not_member`, `lookup_failed` | `lookup_failed` covers Discord errors, a single lookup over `access_gate_lookup_timeout_ms` and a whole answer over `access_gate_decision_timeout_ms` |
| `member` | bool | true only for `member` |
| `owner` | bool | the configured owner id; also true for `not_member` and `lookup_failed` when the id is the owner's |
| `timed_out` | bool | an active Discord timeout |
| `permissions` | string[] | server-wide permission names; Administrator expands to every name |
| `channel_permissions` | string[] or null | effective names in `channel_id`; null when no channel was given or it could not be resolved in the guild |
| `role_ids` | string[] | the member's role ids |
| `top_role_position` | int | position of the highest role |

For `not_member` and `lookup_failed` every right field except `owner` is empty or false. One answer may take up to three Discord lookups (member, roles, channel), each bounded by `access_gate_lookup_timeout_ms`; the whole answer is bounded by `access_gate_decision_timeout_ms` and answers `lookup_failed` when it runs over. Members, roles and channels are cached for at most `access_gate_cache_ttl_ms`; a role change is visible after that at the latest.

`channel_permissions` applies Discord's overwrite order and its implicit denies: without `view_channel` the list is empty, and without `send_messages` it also drops `send_tts_messages`, `embed_links`, `attach_files` and `mention_everyone`. Administrator expands to every name.

### `POST /gate/v1/authorize`

Request:

| Field | Type | Notes |
|---|---|---|
| `caller` | string | connector name from the access file (`brain`, `discord`, ...) |
| `origin` | `discord_message`, `discord_interaction`, `none` | where `user_id` came from |
| `user_id` | string | optional; ignored for the decision unless `origin` is a Discord origin, but always validated and stored in the record |
| `channel_id` | string | optional; with `dm = true` it is the DM channel with the asker |
| `dm` | bool | default false |
| `capability` | string | `area.verb`, lower case |
| `class` | `read`, `write`, `destructive` | |
| `internal` | bool | default false; the result is internal and needs a safe place |
| `target_user_id` | string | optional member the action targets |
| `target_role_id` | string | optional role the action targets |
| `args_hash` | string | 64 lower-case hex characters (SHA-256 of the tool arguments) |
| `args_redacted` | object | optional; must be a JSON object, anything else answers `400`; keys that look secret are replaced, NUL characters are removed from keys and strings, more than 4096 bytes are stored as a size marker |

Unknown fields are rejected with `400`.

Answer:

| Field | Type | Notes |
|---|---|---|
| `decision_id` | string | 32 lower-case hex characters |
| `allowed` | bool | |
| `reason` | string | see below |
| `confirm_required` | bool | the capability's `confirm`, default true for any class but `read` |
| `shadow` | bool | true when the connector or the capability is in shadow mode; the caller logs instead of acting for real |
| `place_ok` | bool | the result may be shown where it was asked; false whenever `allowed` is false |

`place_ok` is true for a non-internal result. For an internal result it is true in a DM channel with the asker, or in a guild channel bound to the capability's area where neither @everyone nor the verified base role can view and every role and member that can view holds the capability. Members are checked through their view overwrites; a channel with more than 25 member overwrites that allow viewing is never a safe place, and a user with such an overwrite who is no longer a guild member is skipped. A channel with a role overwrite for a role missing from the cached guild roles is never a safe place, and the role cache is refreshed on the next lookup. Otherwise false.

Decision order: access file, caller, capability enabled, identity, owner, membership, timeout, `owner_only`, Administrator, mapping, rights, hierarchy, place, record. Every step shares one deadline of `access_gate_decision_timeout_ms`. When a step before the record runs over, the answer is `lookup_failed` with `confirm_required = true`, `shadow = true` and `place_ok = false`. When the record write of an allow fails or runs over, the answer is `record_unavailable` with `place_ok = false` and the `confirm_required` and `shadow` the decision had (see Record).

### `POST /gate/v1/outcome`

Request: `decision_id`, `caller` (the caller of that decision), `status` (`ok` or `error`), optional `detail` (NUL characters removed, then cut at 1000 characters; empty means none). Answer: `{"queued": bool}`. The outcome is attached once; later ones for the same decision are ignored, and an outcome that matches no stored open decision is dropped with a warning log.

## Reasons

| Reason | Meaning |
|---|---|
| `allowed` | the only allow |
| `access_file_unavailable` | no valid access file has been loaded |
| `unknown_caller` | `caller` is not a connector in the access file |
| `connector_disabled` | the connector is disabled |
| `capability_disabled` | the capability is disabled |
| `no_identity` | no Discord user and the capability is not public |
| `not_member` | the user is not a guild member |
| `lookup_failed` | Discord lookup failed or timed out |
| `timed_out` | the user is timed out and the class is not `read` |
| `owner_only` | only the owner holds the capability |
| `unmapped` | the capability is not in the access file; only Administrator and owner hold it |
| `missing_right` | no granting permission bit or role |
| `hierarchy` | a target member or role is at or above the asker's highest role, or the target is the owner |
| `target_lookup_failed` | the target could not be resolved |
| `record_unavailable` | the allow could not be stored before the answer, so it is refused |

`lookup_failed`, `target_lookup_failed`, `record_unavailable` and `access_file_unavailable` can pass on their own. A caller that retries sends a new request and gets a new `decision_id`; the refused decision stays refused.

Callers must treat an unknown reason as a refusal; new reasons may be added.

## Access file

`access-gate.toml` next to dl-bot's operating config (`bot.toml`). Only the gate reads it; it is reloaded within `access_gate_reload_ms` of a change. A malformed file is refused and the last good one stays active. A file that is deleted or unreadable while dl-bot runs also keeps the last good one active; it does not revoke anything until dl-bot restarts, and after a restart without a file every decision is `access_file_unavailable`. Template: `config/access-gate.toml`.

```toml
schema_version = 1

[connectors.brain]
enabled = true
shadow = true

[areas.moderation]
channels = ["<channel id>"]

[capabilities."discord.moderation.ban"]
area = "moderation"
permissions = ["ban_members"]
roles = ["<role id>"]
owner_only = false
confirm = true
enabled = true
shadow = true
public = false
```

Administrator holds every capability that is not `owner_only`; the owner holds everything. A capability's `shadow` and `enabled` default to true, `owner_only` and `public` to false. Role and channel ids must be canonical positive Discord ids that fit a signed 64-bit integer; names are 1 to 128 characters of `a-z`, `0-9`, `.`, `_` and `-`, the same rule the record table checks.

## Record

Decisions are rows in `bot.access_gate_decisions`. An allow is answered as `allowed` only after its row is committed. The insert runs in its own task and is never cancelled. If it fails or misses the decision deadline, the answer is `record_unavailable`, and as soon as the insert ends the task rewrites a row that did commit to `allowed = false`, `reason = record_unavailable`, `place_ok = false`. A failed insert is treated the same way, because the row may have committed before the error reached dl-bot. The rewrite is retried every `access_gate_decision_timeout_ms`, at most 20 times; when the database stays unreachable for that long, an error log names the decision id whose row may still show an allow. So the table never shows an allow the caller did not receive while the database answers within that window. Refusals are recorded best-effort through a bounded queue off the request path and are dropped with a warning log when the queue is full or the insert fails. The schema checks every column edge the writer guarantees (known reasons, name formats, positive ids, object arguments, bounded detail). dl-bot deletes rows older than `access_gate_retention_days` every `access_gate_purge_every_secs` whenever both keys are present, whether the gate token is set or not. A member's data deletion removes every row where the member is `user_id` or `target_user_id`, and the member's data export lists those rows.

## Planned without a break (G0b, confirm click)

An optional `confirmation_id` request field, a nullable `confirmation_id` column and a pending-action table bound to asker, capability and `args_hash`, single use and expiring. Existing callers keep working because the field is optional.
