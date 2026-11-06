# Brain image input (Bots side)

## Trigger

A Discord user posted a Deadlock build screenshot with "macht das Build so noch sinn oder nicht".
The Brain asked which hero, because it never saw the image: the whole answer chain was text-only.

## Goal

Images the asker attached to the message the Brain answers reach the answering model as real
image input. Paths: DM, mention, `!brain`, concierge, FAQ chat, passive help and `/brain`.

## Boundaries

- The bot downloads and validates (Discord CDN allowlist, https, size cap, decode check) and sends
  base64 plus media type. The Brain never fetches URLs.
- No image bytes or URLs in logs. Only the asker's own attachments, plus (added scope by the
  coordinator, 2026-11-06) the attachments of the message the asker explicitly replies to with a
  bot mention, for that turn only.
- Providers that cannot take images fail explicitly, they never drop images.
- Limits live in `runtime.ai` config.
- The build publish path is untouched.

## Acceptance

- dl-ai tool chat carries `image` blocks unchanged to the Anthropic wire and to `claude -p`.
- Chat-provider backends reject image input with `image_input_unsupported`.
- Every Brain answer path forwards the asker's attachments; image-only messages get a default
  question; follow-ups in the same channel can reuse the last images as `earlier`.

## Added scope (coordinator, 2026-11-06)

A guild mention that replies to another member's message answers that message: its text plus
the same author's directly following messages within two minutes form the question; the asker's
own text, if more than one word, becomes the question with the reference as context. Images of
the referenced message flow through the image path. Replies to the bot keep the conversation
history behaviour; a failed fetch or empty reference keeps the usage reply. Reuse the existing
message fetch code.

Brain side: Deadlock-Brain branch `feat/brain-image-input-20261105`.
