# invite-sync stale binary (2026-10-24)

State: restored. deadlock-twitch-invite-sync.service failed every 15 min since at least 2026-10-20
("Config-Syntax oder unbekanntes Feld, Byteposition 977"): binary in the live checkout
rust/target/release/ dated 2026-09-30 rejected newer config/bot.toml fields (deny_unknown_fields).
Fresh build from origin/main beec52db installed 2026-10-24 20:51, run OK (107 invites, 199 rows).
Old binary kept as dl-twitch-invite-sync.stale-20260930.

Open:
1. Run from /opt/deadlock/bots/current like dl-community-points-sync (add binary to bots release,
   unit via dl-infisical-env launcher, --config ~/.config/deadlock-bots/bot.toml). Launcher must
   provide TWITCH_INTERNAL_API_TOKEN, currently only DEADLOCK_CENTRAL_DSN.
2. dl-unit-failure-notify: include the last journal error line instead of
   "keine sicher weitergebbare Ursache".
3. Audit other units still exec'ing binaries from the live checkout target/ (dl-repostats etc.).
