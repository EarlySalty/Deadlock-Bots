# Gate round 4 findings (two BLOCK answers, both cut off before the end)

Sources: gate run 14:20 (partial) and gate run 20:02 (partial, /tmp/claude-1000/-home-nathanael-repos-Deadlock-Bots/066fafc1-fd1a-4efe-a6f1-47a1da07da4b/scratchpad/gate8.txt). HEAD 7c7a1ef5 on origin/main 11f721bd.

## Classes

1. Off state swallowed as success. `fetch_threads` discards every `discord_call` error, so a missing `mcp_discord_timeout_ms` (DiscordCallsOff) or any Discord failure becomes an empty successful `list_threads`. Places: rust/bin/dl-bot/src/mcp.rs:937 and 965-971, rust/bin/dl-mcp/src/main.rs:590 and 609-616, later pages at rust/bin/dl-bot/src/mcp/public.rs:544-547. The 796d commit layer 4 claims a missing key switches the function off; the contract must hold at every consumer.
2. Duplicated rules with no single source:
   - Bearer parsing three times: rust/bin/dl-bot/src/mcp/gate.rs:254-261, rust/bin/dl-bot/src/mcp.rs:213-219, rust/bin/dl-bot/src/mcp.rs:224-230.
   - Cache freshness rule twice: rust/bin/dl-bot/src/mcp/gate.rs:134 and 188.
   - Range numbers copied as text into validator messages (rust/crates/dl-core/src/runtime_config.rs:206, 211 and the six gate messages) and into rust/crates/dl-access-gate/CONTRACT.md:19-26.
3. Retry timing not bound to the configured timeout: `discord_call` 429/5xx backoff literals at rust/bin/dl-bot/src/mcp.rs:715-724 run inside the configurable lookup timeout without being derived from it or bounded by the remaining deadline.
4. Token reuse rule order dependent and untested: rust/bin/dl-bot/src/mcp.rs:174-177; test `reused_gate_token_stops_the_start` (rust/bin/dl-bot/src/mcp/gate/tests.rs:823-835) never sets a public token.
5. Tests that do not reach the contract:
   - `wire_ids_and_names_follow_the_record_checks` (rust/bin/dl-bot/src/mcp/gate/tests.rs:838-867) never inserts into bot.access_gate_decisions, so drift from the migration CHECKs stays invisible.
   - The missing-timings test proves only `tool_definitions(None)` and never reaches the refusal at rust/bin/dl-bot/src/mcp/public.rs:852-854 or the public listing at rust/bin/dl-bot/src/mcp.rs:256-258.
   - No test drives a tool call (list_threads) with the HTTP client off.
6. "Three contract mismatches" between CONTRACT.md and code: details were in the cut-off part. Audit rust/crates/dl-access-gate/CONTRACT.md against routes, fields, reasons, config keys and behaviour in code and fix every mismatch.
