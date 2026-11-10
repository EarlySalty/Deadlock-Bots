# Access gate contract (v1)

The access gate runs inside dl-bot on the loopback MCP listener (`127.0.0.1:8890` by default). It answers who a Discord user is right now and whether a connector may run a capability for that user. The wire format is plain JSON over HTTP; `dl-access-gate` holds the same types and a client.

## Rule for every caller

Anything other than an explicit allow is a refusal. A caller may act only when the `authorize` answer has HTTP 200, `allowed = true` and `reason = "allowed"`. A transport error, a timeout, any non-2xx status, an unparsable body, an unknown reason or any other field combination is a refusal. `GateClient::permits` implements exactly this.

## Authentication

Every route needs `Authorization: Bearer <token>`. The token is the Infisical secret `DISCORD_ACCESS_GATE_TOKEN`, loaded through dl-bot's private snapshot like the other MCP tokens. It must differ from the internal and the public MCP tokens. Without the secret the gate is off and every route answers `401`. A missing or wrong token answers `401`; a malformed body answers `400`.

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
| `status` | `member`, `not_member`, `lookup_failed` | `lookup_failed` covers Discord errors and lookups over `runtime.start.access_gate_lookup_timeout_ms` (default 5000) |
| `member` | bool | true only for `member` |
| `owner` | bool | the configured owner id |
| `timed_out` | bool | an active Discord timeout |
| `permissions` | string[] | server-wide permission names; Administrator expands to every name |
| `channel_permissions` | string[] or null | effective names in `channel_id`; null when no channel was given or it could not be resolved in the guild |
| `role_ids` | string[] | the member's role ids |
| `top_role_position` | int | position of the highest role |

For `not_member` and `lookup_failed` every right field is empty or false. Answers are cached per user for at most `runtime.start.access_gate_cache_ttl_ms` (default 60000); a role change is visible after that at the latest. A whole decision is bounded by `runtime.start.access_gate_decision_timeout_ms` (default 15000, never below the lookup timeout) and answers `lookup_failed` when it runs over.

### `POST /gate/v1/authorize`

Request:

| Field | Type | Notes |
|---|---|---|
| `caller` | string | connector name from the access file (`brain`, `discord`, ...) |
| `origin` | `discord_message`, `discord_interaction`, `none` | where `user_id` came from |
| `user_id` | string | optional; ignored unless `origin` is a Discord origin |
| `channel_id` | string | optional; with `dm = true` it is the DM channel with the asker |
| `dm` | bool | default false |
| `capability` | string | `area.verb`, lower case |
| `class` | `read`, `write`, `destructive` | |
| `internal` | bool | default false; the result is internal and needs a safe place |
| `target_user_id` | string | optional member the action targets |
| `target_role_id` | string | optional role the action targets |
| `args_hash` | string | 64 lower-case hex characters (SHA-256 of the tool arguments) |
| `args_redacted` | object | optional; keys that look secret are replaced, more than 4096 bytes are stored as a size marker |

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

`place_ok` is true for a non-internal result. For an internal result it is true in a DM channel with the asker, or in a guild channel bound to the capability's area where neither @everyone nor the verified base role can view and every role and member that can view holds the capability. Otherwise false.

Decision order: access file, caller, capability enabled, identity, owner, membership, timeout, `owner_only`, Administrator, mapping, rights, hierarchy, record.

### `POST /gate/v1/outcome`

Request: `decision_id`, `caller` (the caller of that decision), `status` (`ok` or `error`), optional `detail` (cut at 1000 characters). Answer: `{"queued": bool}`. The outcome is attached once; later ones for the same decision are ignored.

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
| `record_unavailable` | the decision could not be queued for the record, so it is refused |

Callers must treat an unknown reason as a refusal; new reasons may be added.

## Access file

`access-gate.toml` next to dl-bot's operating config (`bot.toml`). Only the gate reads it; it is reloaded within `runtime.start.access_gate_reload_ms` (default 5000) of a change. A malformed file is refused and the last good one stays active. Template: `config/access-gate.toml`.

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

Administrator holds every capability that is not `owner_only`; the owner holds everything. A capability's `shadow` defaults to true.

## Record

Every decision is a row in `bot.access_gate_decisions`, written off the request path through a bounded queue. Rows older than `runtime.start.access_gate_retention_days` (default 180) are purged every `runtime.start.access_gate_purge_every_secs` (default 21600). All six values live in dl-bot's `bot.toml`; dl-bot refuses to start when one is missing or 0, and config validation rejects out-of-range values.

## Planned without a break (G0b, confirm click)

An optional `confirmation_id` request field, a nullable `confirmation_id` column and a pending-action table bound to asker, capability and `args_hash`, single use and expiring. Existing callers keep working because the field is optional.
