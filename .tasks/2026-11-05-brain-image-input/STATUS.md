# Status

Branch `feat/brain-image-input-20261105`. `dl-brain` pins Deadlock-Brain at the head of the Brain
branch of the same name; the Brain merge must keep that commit reachable (no squash), otherwise the
pin has to be bumped to the merged commit.

## Done

- dl-ai `tool_chat`: `ContentBlock::Image` (Anthropic base64 source, redacted Debug). The
  Anthropic API backend passes it through unchanged; `claude_plan` sends image blocks in the
  stream-json user message (live-checked with `claude -p` on a synthetic PNG, answer "Red");
  chat-provider emulation returns `Unavailable` / `image_input_unsupported` instead of dropping.
- dl-brain `images`: Discord CDN attachment URL allowlist, bounded download without redirects or
  proxy, decode with size limits, resize to the configured edge and re-encode as JPEG off the async
  runtime (also strips metadata), per-image budget inside the Brain contract limits. Slash
  uploads under `/ephemeral-attachments/` are accepted too. PNG, JPEG, WebP and GIF (first
  frame) are decoded; other formats the attachment filter lets through (AVIF, BMP, ...) are
  skipped so a text question is still answered, and an image-only turn without a usable image
  fails (`bild` class). Download or decode failures of a supported image fail the turn.
  Image preparation runs inside the Brain admission slot.
- Follow-ups: the last prepared images per (user, channel) are kept in memory for the configured
  TTL and sent as `origin = earlier` when a follow-up turn has history but no new attachment.
  The cache holds at most 512 entries and 64 MiB of base64.
  Passive help never reuses images. `forget_user` clears them; concierge `clear_user_runtime`
  calls it.
- Wiring: modglue DM, mention and `!brain` paths, `/brain` with the new optional `bild` attachment
  option (dispatch resolves attachment options to `{url, content_type}`), concierge, FAQ chat and
  passive help. Image-only messages use `Was sagst du zu meinem Bild?`.
- Config `runtime.ai.brain_image_*` (count, download bytes, edge, JPEG quality, follow-up seconds),
  classified in the settings catalog.

- Reply mention (added scope): a guild mention that replies to another member's message fetches
  that message (same channel only) plus the same author's directly following messages within two
  minutes before the mention. A bare or one-word mention asks that text; longer own text becomes
  the question with the referenced text as labelled context, truncated to the question limit.
  If the asker's own text alone leaves no room for the reference, the own text goes unchanged.
  Images of the referenced message join the asker's images for this turn only; they are not
  kept for follow-ups, so no copy of another member's image sits in the cache. A reply to the bot itself, a failed
  fetch or an empty reference keep the old behaviour. Cooldown, daily limit and the reply target
  stay with the asker's mention message. The existing fetches and the new one share
  `fetch_message` and `fetch_messages`.

## Live check plan (after dl-bot deploy)

1. Member A posts "uhm wieviele punkte kriegt man jetzt für nen win ?" and right after
   "also für nen char ?". Member B replies to the first message with only the bot mention. The
   bot answers B's message about points per win for a character.
2. B replies to a build screenshot of A with only the mention: the answer names the hero and
   items from the image.
3. B replies to A with "@Bot stimmt das?": the answer judges A's statement.
4. B replies to a bot answer with only the mention: old conversation behaviour.
5. B replies with only the mention to a message that was deleted meanwhile: usage reply as
   before.

## Not covered

- FAQ ticket auto-help stays text-only (it posts candidates into the shadow channel).
- A failed image load surfaces as the existing generic Brain error text.

## Checks

- Tests run with plain cargo locally because ionos was unreachable (ssh timeout). dl-bot:
  264 passed, 90 failed, 9 ignored; dl-community: 281 passed, 4 failed. Every failure is a
  scratch-Postgres test stopping at the missing `CENTRAL_TEST_DSN`, none reaches changed code.
  dl-discord 67 + 3, dl-brain 34, dl-core 43 (+2 ignored), dl-ai tool chat 25 passed.
- The new `referenced_message` port method adds one more instance of the pre-existing
  `double_must_use` warning that clippy 1.99 reports for every `async_trait` method.
- Clippy without `-D warnings`: the workspace has pre-existing `double_must_use` and dead-code
  warnings; none in the new or changed lines.

## Deploy order

1. Deadlock-Brain service (accepts the additive `images` fields).
2. dl-ai-gateway with `max_request_bytes` 8 MiB. The tracked `config/dl-ai-gateway.toml` now
   ships 8 MiB, and the release installer copies it to the live path (an image request can reach
   about 2.9 MiB of base64 plus the grounded payload).
3. dl-bot, then re-register slash commands so `/brain` gets the `bild` option.
