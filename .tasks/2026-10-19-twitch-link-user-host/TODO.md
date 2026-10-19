# TODO twitch-link-user-host 2026-10-19

## Live now
- Caddy on `/etc/caddy/Caddyfile` already 302s `admin.../twitch-verknuepfen*` to `https://deutsche-deadlock-community.de{uri}`.
- Public host `/twitch-verknuepfen*` reverse_proxies 127.0.0.1:8766.
- Proof: admin GET probe is HTTP/2 302 Location public host; public GET probe is 400 HTML `Link abgelaufen`.

## Not live
- dl-web still `/opt/deadlock/bots/releases/8c071bec...`. Initiate still stores admin `redirect_after`. Marker `twitch-link-done-user-host` is not in the live binary.

## Commits (unpushed)
- Bots `/home/nathanael/.worktrees/Deadlock-Bots-twitch-link-user-host-20261019` `0966d7a3` ahead 1 of origin/main.
- Caddy `/home/nathanael/.worktrees/caddy-twitch-link-user-host-20261019` `2d48501` ahead 1 of origin/master.

## After 14:00 Europe/Berlin
1. `python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review --repo /home/nathanael/.worktrees/Deadlock-Bots-twitch-link-user-host-20261019 --base origin/main --head HEAD`
2. Same for Caddy worktree with `--base origin/master`.
3. On ALLOW: `git push origin HEAD:main` (bots), `git push origin HEAD:master` (Caddy). One git step per bash. Literal paths.
4. Deploy `dl-web` only via deploy-restart-selbstdienst. Keep launcher/companions. Do not reuse the rang-guide worktree.
5. LIVEBEWEIS: initiate `redirect_after` is `https://deutsche-deadlock-community.de/twitch-verknuepfen/fertig`; binary contains `twitch-link-done-user-host`.
6. Delete `~/Caddyfile.neu` after Caddy is on origin.

Tests already green: twitch_link 8 passed, cargo-slot log `call-7059eac4-e485-4d17-b50b-5a24b850d66c-122`.
