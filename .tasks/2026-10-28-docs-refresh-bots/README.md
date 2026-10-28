# Docs refresh bots (2026-10-28)

Checked against origin/main 45e65a1f and the live host (systemctl --user, ss, Caddyfile).

Changed:
- README.md: replaced the stale Python-era README with a current overview (user units, ports, build, deploy, live checks, October features).
- Historical headers on dated plans, audits, specs, drafts, dated snapshots, and the legacy Python cog docs in docs/internal (ai-moderator, ai-onboarding-pipeline, security-guard, build-publisher, rename-manager, claim-system, steam-bridge-watchdog, scam-revoke-button, admin-commands) and docs/build-publishing/AUTONOMER_BETRIEB.md.

Open items:
- Not every non-archive doc was line-by-line verified. Top-level docs/*.md pages with feature claims were not re-checked.
- Scrim lobby automation (Steam-Bot repo) and English / multi-server support: no code found in this repo; not documented here.
- Live: user unit dl-community-points-sync.service is failed; dl-brain-feeder.timer is inactive; service/systemd/dl-brain-feeder.service points to an old /home/naniadm path (unit file, out of docs scope).
