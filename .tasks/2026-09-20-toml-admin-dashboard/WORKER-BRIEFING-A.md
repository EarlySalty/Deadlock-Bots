# Briefing: twitch-toml-admin-live

[Orchestrator] Paket A. Auftrag: /home/nathanael/Documents/.tasks/2026-09-20-toml-admin-dashboard/AUFTRAG.md
(vollständig lesen, dann nur Paket A bauen). Worktree und Branch stehen unten.

- Worktree: /home/nathanael/.worktrees/twitch-toml-admin-live-20260920
- Branch: feat/twitch-toml-admin-live-20260920
- Intent-Thread: 9d6f2b8b-9869-4128-b506-ba8956529a73

## Pflichtteile

1. Referenz: origin/main `rust/scripts/run_tb_bot_service.sh` Zeile 7
   `OPERATING_CONFIG='/var/lib/deadlock-twitch/config/bot.toml'`.
   Editor `bot/admin_dashboard/src/pages/config/OperatingConfig.tsx`.
   Sidebar `/config/operating`. Live Release `818e2152` ohne diesen Stand.
   Belegte Nichtsecret-Datei: `/home/nathanael/.config/deadlock-twitch/bot.toml`.
   Deploy: `deploy-twitch-release <sha>`, Restart: `bot-restart twitch-bot twitch-dashboard`.
2. Scope-Zaun: exakt Paket A, kein Discord, kein Patchnotes, kein Refactoring, kein fmt.
   Andere Doku im Repo ignorieren.
3. Branch feat/twitch-toml-admin-live-20260920 auf origin/main. Nur diesen Branch
   pushen, nie main. Uncommitteter Zustand im Worktree vorher prüfen.
4. Beweis: Seite
   https://admin.deutsche-deadlock-community.de/twitch/admin/config/operating
   sichtbar. Speichern schreibt `/var/lib/deadlock-twitch/config/bot.toml`.
   Bot startet mit `--config` dieser Datei. PID-Wechsel, exe ohne (deleted),
   journal -p err leer. Keine Secretwerte in Logs.
5. Intent-Thread 9d6f2b8b-9869-4128-b506-ba8956529a73. Bump-up:
   `[Bump-up] Paket A: Grund: ... Erledigt: ... Worktree: ... Offen: ...`
   danach stoppen.

## Regeln

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder
  Unter-Agenten spawnen.
- Keine Code-Kommentare schreiben.
- Nutzersichtbare Texte auf Deutsch mit echten Umlauten, keine Gedankenstriche.
- Keine Beispielwerte als Produktivdatei. Keine Secrets lesen.
- Helix-Session 65d5c809 nicht anfassen.

## Fertigmeldung

Branch, Commits (SHA), geänderte Dateien, Live-SHA, PID alt/neu, Ort im UI,
was offen ist. Danach stoppen.
