# Gateway output_schema for anthropic_api

## State
- ToolChatRequest has `output_schema: Option<Value>` (struct has no serde derives; the HTTP contract is the gateway WireRequest).
- dl-ai-gateway WireRequest accepts optional top-level `output_schema` (serde default); requests without it work unchanged.
- anthropic_http::wire_request adds `output_config.format = {type: json_schema, schema}` when set. No beta headers.
- claude_plan and chat_backend ignore the field and keep their emulation schema.
- Tests: cargo-slot test -p dl-ai (151 ok), -p dl-ai-gateway (3 ok).

## Open
- Brain client does not send `output_schema` yet; wiring the Brain envelope schema into its gateway call is a separate change.
- No deploy or restart done.
- Gate 4f310e14: ALLOW (claude-opus-5-5 after gpt-6.1-sol usage limit), three NITs:
  - Fallback to claude_plan/chat_backend drops output_schema silently and returns 200 prose; must be solved before the Brain sends the field (brain_answer, brain_bot_task fall back on 429/529/timeout).
  - WireRequest accepts any JSON value as output_schema; a non-object gives 400 on anthropic_api but 200 on emulating providers. Validate as object at the gateway entry.
  - output_config shape on claude-haiku-5-5 not yet proven by a real request; verify with one live call over slot brain_answer.
