# Deutsche Deadlock Community Bots

Rust workspace for the Discord bot, the web services and the supporting daemons of the Deutsche Deadlock Community. The workspace lives in `rust/`. The former Python bot (`main_bot.py`, cogs, SQLite) is gone from production; remaining Python files in `scripts/` are helpers and legacy references only.

## What runs where

Units are user units of the host user (`systemctl --user`, unit files under `~/.config/systemd/user/`). This repo has no system-level units of its own. Verify with `systemctl --user status <unit>` before relying on this table.

| Unit | Binary | Role |
|---|---|---|
| `deadlock-bot-rust.service` | `dl-bot` | Discord bot, started through the `dl-infisical-env` launcher from `/opt/deadlock/bots/current` |
| `deadlock-web-rust.service` | `dl-web` | Web services (dashboard, public stats, tierlist, changelog, master broker) |
| `dl-ai-gateway.service` | `dl-ai-gateway` | Central AI provider gateway for the bots |
| `dl-community-points-sync.timer` | `dl-community-points-sync` | Community points sync |
| `dl-insights-sync.timer` | `dl-insights-sync` | Discord insights sync |
| `dl-repostats.timer` | `dl-repostats` | Repository statistics |
| `dl-verbinder.timer`, `dl-verbinder-summary.timer`, `dl-verbinder-auswertung.timer` | `dl-verbinder` | Verbinder reporting |
| `dl-brain-feeder.timer` | `dl-brain-feeder` | Second-brain weekly digest feeder |

Local listeners on `127.0.0.1` are set in `~/.config/deadlock-bots/web.toml` (keys `dashboard_port`, `public_stats_port`, `master_broker_port`, `tierlist_public_port`, `changelog_port`) and the MCP connector port in the bot config. Caddy on the public host proxies the public routes to these ports; the routes are defined in `~/repos/Caddy/hosts/v50671/Caddyfile`, not in this repo.

Configuration lives in `~/.config/deadlock-bots/*.toml` (`bot.toml`, `web.toml`, `dl-ai-gateway.toml`). Example files are in `config/`. Secrets come from Infisical through the launcher; there are no `.env` files.

## Build

```bash
cd rust
cargo build --release --bin dl-bot --bin dl-web --bin dl-ai-gateway
```

Follow `/home/nathanael/Documents/claude-config/machines/v50671/RUST-BUILDS.md` for the shared sccache, slot and artifact rules. Do not start competing release builds.

## Deploy

Releases live in `/opt/deadlock/bots/releases/<sha>`. `/opt/deadlock/bots/current` is a symlink to the active release, and `/opt/deadlock/bots/rollback` keeps the previous one. The release installer is `rust/bin/dl-bot/examples/install_discord_release.rs`; it flips the `current` symlink and rebuilds `dl-bot` and `dl-ai-gateway`. After a switch, restart the affected units:

```bash
systemctl --user restart deadlock-bot-rust.service deadlock-web-rust.service dl-ai-gateway.service
```

## Check that it is live

```bash
readlink -f /opt/deadlock/bots/current
systemctl --user is-active deadlock-bot-rust.service deadlock-web-rust.service dl-ai-gateway.service
ss -ltn | grep -E ':(8766|8768|8770|8771|8899)\b'
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8768/
```

The live release path must equal the deployed `origin/main` SHA.

## Features and operations (October 2026)

- Global DM opt-out: the `core.dm_optout` table in the central Postgres DB holds opted-out users. Only DMs with the `DmIntent::Proactive` intent are checked against it; replies, moderation, security, cleanup and internal DMs bypass the gate. Without a database pool, proactive DMs are refused (`GateUnavailable`). Code: `rust/crates/dl-discord/src/dm_gate.rs`, `rust/crates/dl-central-db/src/dm_optout.rs`.
- Paten program: the onboarding offer and the Paten panel. Channel IDs are set in `runtime.community` (`concierge_pate_panel_channel_id`, `concierge_pate_help_channel_id`).
- Rank panel (`deadlock-rang`): published by `rust/bin/dl-bot/src/serversync/rang_guide_publish.rs` through `POST /serversync/rang-guide-apply` (requires `confirm`). Restore after an overwrite: clear the stored `rang_guide_payload_hash` serversync value, then call the apply endpoint again.
- Scrim lobby automation runs on the Steam-Bot side in its own repository; this repo does not contain it.

## Documentation

- `AGENTS.md`: token storage boundary, read first for any auth or access work.
- `docs/token-storage-db.md`: token and credential storage rules.
- `docs/` (not `docs/_archive/`): feature and operations notes. Files with a `historical` header describe past states only.
- `rust/docs/`: rewrite plan, central DB architecture and ADRs. Audits and plans are dated and marked historical where superseded.

<!-- DDC-LINK-START -->

## Deutsche Deadlock Community

Dieser Bot ist Teil der **[Deutschen Deadlock Community](https://deutsche-deadlock-community.de/)** — Master-Bot der Deutschen Deadlock Community.

- 🌐 Website: [deutsche-deadlock-community.de](https://deutsche-deadlock-community.de/)
- 💬 Discord: [discord.gg/PhkP3WgY7w](https://discord.gg/PhkP3WgY7w)
- 🎮 [Helden-Übersicht auf Deutsch](https://deutsche-deadlock-community.de/helden/) · [Anfänger-Guide](https://deutsche-deadlock-community.de/guides/anfaenger/)

<!-- DDC-LINK-END -->
