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
