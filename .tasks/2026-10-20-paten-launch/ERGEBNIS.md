# Paten-Launch: result 2026-10-20

- Merge: 4badadd1 on origin/main (feature 7abfc55b, gate fix 0861c683). Branch, remote branch and worktrees removed.
- Gate: round 1 BLOCK (claim marker not removed when a Patenschaft ends), fixed; round 2 ALLOW (claude-opus-5-5).
- Tests: concierge DB tests 263 passed, 0 failed, 0 ignored. dl-core: 2 FD3 snapshot tests are red, same as the origin/main baseline (43/2).
- Migration 20261020120000_concierge_paten_lifecycle applied via dl-central-migrate behind the FD3 launcher.
- Operating TOML `[runtime.community]`: concierge_enabled=true, concierge_pate_panel_channel_id=1562083323507900517, concierge_pate_help_channel_id=1426220702054355077; proactive unchanged (false). Backup: bot.toml.bak-paten-20261020.
- Release: /opt/deadlock/bots/current -> releases/4badadd1, dl-bot SHA256 67d90187..., PID 4075684, NRestarts 0, error journal empty, Gateway READY.
- Live: panel message 1562215908641087742 in 1562083323507900517 with the button „Ich brauche einen Deadlock-Paten“ (concierge:pate:panel). Team application panel 1544026617733710006 has the Pate button.
- Panel text: the release folder has no assets/paten_panel.toml, so the embedded draft (same text) is used.

## Open

- Live click test (request card in 1524083665838276860 without owner ping, withdraw) needs a throwaway Discord account; none is on file. The flow is covered by the handler-level DB test `patenanfrage_zurueckziehen_schliesst_karte_ohne_knopf`.

## Update 2026-10-25

- Section „🌱 ─ NEUE SPIELER ─“ (1465839366634209361) made visible: @everyone and Coach view deny removed on the category; 📌so-läufts-hier readable but not writable for @everyone; 🤝paten-zentrale stays Paten-only.
- Bug found: the 🆕Neue Spieler Lane anchor was moved into NEUE SPIELER on 2026-10-20, but `dl-voice/src/adaptive.rs` still searched it in the Voice category, so beginner routing and lane expansion were dead ("Adaptive: Anker nicht gefunden"). Fixed in ad0d7ad8 (gate ALLOW claude-opus-5-5), released to /opt/deadlock/bots/releases/ad0d7ad8…, dl-bot SHA256 ce63897f….
- Still open: 💬frag-die-community stays in COMMUNITY (user decision 2026-10-25), onboarding option text, Paten call in #ankündigungen (draft in TEXTE-ENTWURF.md, needs explicit go), first real request/claim.
- DB 2026-10-25: 0 rows in bot.concierge_pate_requests and bot.concierge_patenschaften.
