# Briefing: discord-patchnotes-betrieb

[Orchestrator] Paket B. Auftrag: /home/nathanael/Documents/.tasks/2026-09-20-toml-admin-dashboard/AUFTRAG.md
(vollständig lesen, dann nur Paket B bauen). Worktree und Branch stehen unten.

- Worktree Discord: /home/nathanael/.worktrees/discord-patchnotes-betrieb-20260920
- Branch Discord: feat/discord-patchnotes-betrieb-20260920
- Worktree Patchnotes falls nötig: /home/nathanael/.worktrees/patchnotes-betrieb-api-20260920
- Branch Patchnotes: feat/patchnotes-betrieb-api-20260920
- Intent-Thread: 9d6f2b8b-9869-4128-b506-ba8956529a73

## Pflichtteile

1. Referenz: `service/static/operating-config.js` Discord- und Steam-Karten.
   `service/static/dashboard.html` Tab Betriebseinstellungen Zeile 1656.
   `rust/crates/dl-dashboard/src/operating_config.rs` guard_full / guard_mutate.
   Patchnotes-Datei `/home/nathanael/.config/deadlock-bots/patchnotes/bot.toml`.
   Restart-Bestand `/usr/local/bin/bot-restart` Kurznamen patchnotes, dl-bot, web,
   steam-bot. Steam-Cores: bei Katalogwechsel beide zuerst stoppen.
2. Scope-Zaun: exakt Paket B. Keine Twitch-Dateien, kein Refactoring, kein fmt.
   Steam-Editor nicht nachbauen, nur Patchnotes-Karte und Übernahme nach Speichern
   im bestehenden Tab. Andere Doku im Repo ignorieren.
3. Branch wie oben, nur eigenen Branch pushen, nie main.
4. Beweis: im Discord-Admin Tab Betriebseinstellungen Patchnotes-Felder laden
   und speichern. Datei ist dieselbe, die `deadlock-patchnotes.service` liest.
   Nach Speichern aktiver Stand nicht mehr „Neustart ausstehend", PID gewechselt.
   Keine Secrets in der Oberfläche.
5. Intent-Thread 9d6f2b8b-9869-4128-b506-ba8956529a73. Bump-up:
   `[Bump-up] Paket B: Grund: ... Erledigt: ... Worktree: ... Offen: ...`
   danach stoppen.

## Regeln

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder
  Unter-Agenten spawnen.
- Keine Code-Kommentare schreiben.
- Nutzersichtbare Texte auf Deutsch mit echten Umlauten, keine Gedankenstriche.
- Keine allgemeine Shell-API. Nur bestehende Restart-Wege.

## Fertigmeldung

Branch, Commits (SHA), geänderte Dateien, was geprüft wurde, Ort im UI,
was offen ist. Danach stoppen.
