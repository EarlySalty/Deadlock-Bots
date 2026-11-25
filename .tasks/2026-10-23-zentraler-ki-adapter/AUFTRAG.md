# Central AI adapter (dl-ai) for all services

## User goal (2026-10-23)
One central AI adapter for all bots and services. Use the existing `dl-ai` crate of the Discord bot and extend it with plan-based backends (Claude plan, Codex plan) next to the API-key backends (Fireworks/DeepSeek, OpenAI and so on). Every service taps into it and only says which slot or feature it uses; provider, model and settings come from central config.

## Bestand (read-only survey 2026-10-23)
- `rust/crates/dl-ai/`: the `ChatProvider` trait in `chat_provider.rs` has no tools and no multi-turn tool history. `build_provider_for_env` forces Fireworks DeepSeek Flash. The per-feature `LlmUseCase` has 14 features. The Postgres ledger is `public.llm_usage` (`usage.rs`), the transparency log is `transparency*.rs`. There is no streaming. The only tool loop is in `MiniMaxClient` (`lib.rs:1042`) and is unused.
- Other callers:
  - Brain `brain-providers` (repo Deadlock-Brain), with `codex_subscription` fixed to `gpt-6-luna` via the luna proxy on 127.0.0.1:18769. The Brain runs its own tool loop and speaks the Anthropic Messages format.
  - Twitch `tb-llm`, which duplicates the dl-ai ledger.
  - `tb-claude-relay`: the claude CLI on the plan, no tool loop, 2 slots.
  - TradingBot `tb-ai`, with Claude and Codex CLI clients.
  - Patchnotes Perplexity.
- `deadlock-llm-model-resolver.service` has failed (cause unknown).
- The Codex plan quota is shared by the Brain, Claude Code agent sessions (18767) and T3. The Brain is currently down on HTTP 429.

## Target
1. Extend `dl-ai`:
   - a tool-capable request type: messages, tool definitions, tool_use and tool_result blocks, usage;
   - backends:
     - `fireworks` and `openai_compatible` (existing, API key);
     - `claude_plan`: a claude CLI process pool modelled on `tb-claude-relay`. The tool loop is emulated per round through a JSON schema of the form tool_calls or answer;
     - `codex_plan`: through the existing claude-code-proxy or the Codex CLI.
2. Central config per slot: provider, model, timeouts, max tokens, concurrency per provider and an ordered fallback chain. Model names live only in config.
3. A local gateway binary in Deadlock-Bots (`dl-ai-gateway`, a long-running user unit) for services in other repos:
   - an Anthropic Messages-compatible HTTP API with tools, on loopback only;
   - a slot id per request;
   - usage written to the existing ledger.
4. First consumer: the Brain gets a provider kind for the gateway, with the slot `brain_answer` set to Claude plan + `claude-haiku-5-5`. No DeepSeek fallback: the user has no DeepSeek quota at the moment (2026-10-23). The fallback chain stays configurable but is empty for now. The Discord and Twitch bots are migrated later in their own milestones.

## Boundaries
- Rust only, no code comments, no `.unwrap()` in production.
- Postgres only.
- Secrets only via Infisical.
- Do not touch ai-coach.
- Do not hand the Claude Code login token out through a router.
- Approved by the user: Claude plan with Haiku for the Brain. Any other model or cost change for existing bot features needs separate approval; their defaults stay as they are.

## Acceptance
- The gateway runs as a unit. A Brain request on the slot `brain_answer` runs on Claude plan Haiku with tool rounds and is recorded in `llm_usage`.
- When the plan returns 429 or is unavailable, the fallback is used and logged.
- Existing dl-ai users still behave the same: build, tests and clippy are green.
- The gate gives ALLOW, the change is live, and a direct Brain request from the host is answered correctly.
