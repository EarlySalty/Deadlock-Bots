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
- No image bytes or URLs in logs. Only the asker's own attachments.
- Providers that cannot take images fail explicitly, they never drop images.
- Limits live in `runtime.ai` config.
- The build publish path is untouched.

## Acceptance

- dl-ai tool chat carries `image` blocks unchanged to the Anthropic wire and to `claude -p`.
- Chat-provider backends reject image input with `image_input_unsupported`.
- Every Brain answer path forwards the asker's attachments; image-only messages get a default
  question; follow-ups in the same channel can reuse the last images as `earlier`.

Brain side: Deadlock-Brain branch `feat/brain-image-input-20261105`.
