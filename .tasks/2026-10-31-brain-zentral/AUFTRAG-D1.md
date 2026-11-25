# Package D1: Discord bot answers only via the Brain (Deadlock-Bots)

Read first: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-31-brain-zentral/RULES.md`.

Worktree: `/home/nathanael/.worktrees/bots-brain-zentral-d1`, branch `feat/brain-zentral-d1`, based on origin/main b933716f.

## User goal

Every user-facing answer of the Discord bot comes from the Deadlock Brain. The old concierge, its local LLM, the local answer engine and every fallback wording are removed. The Brain takes over the concierge's manner; the bot only executes mechanics (receive, permissions, send, cooldowns). A member asking in a DM may have the Brain read the channels that member can read.

## Current state (verified 2026-10-31)

- `rust/bin/dl-bot/src/main.rs`: `concierge_ai` is built with `LlmProviderConfig::from_env(...).build_provider_for_env(LlmUseCase::BotPate)` (around L1066-1086) and handed to `Concierge::brain_only(…)`; `FaqChat::brain_only(…)`; passive help uses `GroundedHelpBackend{provider, brain}` (around L1826, channel `DL_BRAIN_HELP_CHANNEL_ID`); invite lounge and voice hint wiring near L1820-1880; `chat_text_generator(use_case, json_mode)` at L169.
- `rust/crates/dl-community/src/concierge.rs`: `answers: Option<Arc<dl_answer::AnswerEngine>>`, `with_brain`, `with_answers`, `local_decision` (L5703), `answer_decision` (L5737), `answer_bound_decision` (L5618), persona `GAP_GUIDANCE` (L250), `KNOWLEDGE_UNAVAILABLE_TEXT` (L53), test `brain_only_keeps_smalltalk_on_the_existing_provider`.
- `rust/crates/dl-community/src/faq.rs`: `answers`, `knowledge_url` (`DEFAULT_KNOWLEDGE_URL = "http://127.0.0.1:9"`), `ask_knowledge_at`, `answer_fixture`.
- `rust/crates/dl-community/src/passive_help.rs`: own provider plus brain. `voice_change_hint.rs`: `OpenAiVoiceHintClassifier` (LlmUseCase::VoiceHint) plus fixed hint text. `invite_lounge.rs`: `INVITE_LOUNGE_HINT_TEXT`.
- `rust/crates/dl-brain`: `AiAnswerer`, `BrainApiAnswerer::answer_query(question, user_id, allow_discord_reads, …)`, `DiscordAnswerTask`; `brain-client` pinned by git rev `7dcab7832323a3a19e5ea79773b0bd2c454a726a` in `rust/crates/dl-brain/Cargo.toml`.
- DM read gap: concierge DM answers go through `/v1/task` with reads hardcoded off, and the Brain rejects reads on tasks. A partial local attempt (adds `allow_discord_reads` to `AnswerOptions`/`AnswerBinding` and a `answer_discord_task_with_history_and_read_access` method) is saved as `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-31-brain-zentral/dm-read-access-partial.patch`. Use it as reference only; the final design follows the new contract.

## Dependency on package K (Deadlock-Brain)

Package K extends `brain-contracts` with capabilities `Concierge` (incl. smalltalk), `Faq`, `PassiveHelp`, `VoiceHint`, `InviteLounge`, a `readable_channel_ids` field for DM tasks and an `Answered | Silent` response. The K worker publishes the contract commit on branch `feat/brain-zentral-k` in `https://github.com/`-origin of `/home/nathanael/repos/Deadlock-Brain` within the first hour. Until then start with the removals that need no contract. Pin `brain-client` to the K branch rev to build, then switch the pin to the main rev once K has merged (the orchestrator posts both revs in your thread). Do not merge to main with a pin to a non-main rev.

## Deliverables

1. Remove the local answer paths: `concierge_ai` and the `LlmUseCase::BotPate` provider, `AnswerEngine` option and `with_answers`, `local_decision`/`answer_decision`/`answer_bound_decision` local branches, `GAP_GUIDANCE` and `KNOWLEDGE_UNAVAILABLE_TEXT` wording, FAQ `knowledge_url`/`ask_knowledge_at`/`answer_fixture`, passive help own provider, `OpenAiVoiceHintClassifier` and the fixed voice hint text, fixed `INVITE_LOUNGE_HINT_TEXT`. Delete the tests that assert the old behaviour (for example `brain_only_keeps_smalltalk_on_the_existing_provider`) and keep the test that asserts no `AnswerEngine::new(` in main.rs.
2. Route each of these through the Brain task capabilities: concierge (incl. smalltalk, DMs), FAQ, passive help, voice hint, invite lounge. The bot sends structured context only, never prompt text. Only `Answered` is posted; `Silent` means no message, no prefix, no fallback text.
3. DM reads: for a direct message from a guild member, compute the channel IDs the member can read from the guild cache and send them as `readable_channel_ids`. Public-channel behaviour unchanged.
4. Keep mechanics in the bot: cooldowns, permission checks, thread handling, message splitting, opt-out (`core.dm_optout`).
5. Do not touch detection or analysis paths (dl-moderation, lfg_freetext, coaching_requests analysis, scrim lagebild, turnierglue). Do not delete the `dl-answer`/`dl-knowledge` crates yet; that is package D2 after your merge.

## Acceptance (you verify and report)

- `cargo check`, `cargo clippy`, `cargo fmt --check`, existing tests green (DB tests on ionos).
- No remaining `LlmUseCase::BotPate`, `LlmUseCase::VoiceHint` (unless still used by a non-answer path), `dl_answer` or `knowledge_url` references in `dl-bot`, `dl-community` answer paths. Show the grep result.
- A built `dl-bot` started against the K branch Brain (local or the staging path you find in the repo) answers a DM smalltalk question and a DM channel-summary question through the Brain; evidence from logs or `answer_trace` without secrets.
- Diff stays under about 150 KB; if it grows beyond that, split and report.
- gate-review ALLOW, merged to main with `brain-client` pinned to a main rev of Deadlock-Brain, SHA reported.
