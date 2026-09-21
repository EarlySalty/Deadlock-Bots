# Vorcheck: toml-admin-dashboard

status: aktiv (2026-09-20)

Lesefreundlich, ohne Edits, ohne Tests. Ergebnis als Antwort im Thread, kein
Artefakt im Repo.

## Auftrag an den Vorcheck

- Frage: Welche TOML-Dateien und Admin-Editoren existieren schon für Discord,
  Steam, Twitch und Patchnotes, welche Pfade liest der laufende Dienst, und
  welche Stellen fehlen noch für Speichern aus dem bestehenden Admin plus
  Übernahme ohne SSH?
- Repos: Deadlock-Bots, Deadlock-Steam-Bot, Deadlock-Twitch-Bot,
  Deadlock--Patchnotes-Bot. Live-Units und Dateien unter
  `~/.config/deadlock-*` sowie `/var/lib/deadlock-twitch` mitlesen.
  Graphify zuerst, grep nur zum Nachlesen.

## Antwortformat

- Fundstellen: je eine Zeile in der Form `<pfad>:<zeile>`, dahinter in eigenen
  Worten, was dort steht
- Betroffene Repos: <Liste>
- Geschätzte Zahl der Stellen: <Zahl>
- Risiko: <Prod-DB, Auth, Ban-Risiko, echtes Geld oder „keins erkennbar">

Nur melden, was gelesen wurde. Nichts raten: eine Stelle ohne Fund ist eine
Vermutung, keine Fundstelle.

Bekannter Bestand zum Gegenprüfen, nicht als Wahrheit übernehmen:

- Discord Admin Tab Betriebseinstellungen:
  `service/static/dashboard.html` data-tab=betrieb, JS
  `service/static/operating-config.js`, API
  `rust/crates/dl-dashboard/src/operating_config.rs`
- Live dl-web PID 3680452 enthält `/api/admin/betriebskonfiguration` und
  Steam-Proxy. Live dl-bot startet mit
  `--config /home/nathanael/.config/deadlock-bots/bot.toml`
- Steam: `~/.config/deadlock-steam-bot/bot.toml`, Editor in
  `rust/crates/steam-config/src/editor.rs` und
  `rust/crates/steam-web/src/routes/config.rs`
- Twitch Editor auf origin/main:
  `bot/admin_dashboard/src/pages/config/OperatingConfig.tsx`,
  Route `/config/operating`. Live Release `818e2152` ohne diesen Stand.
  Startskript origin/main erwartet
  `/var/lib/deadlock-twitch/config/bot.toml`. Datei liegt derzeit unter
  `~/.config/deadlock-twitch/bot.toml`
- Patchnotes live mit
  `~/.config/deadlock-bots/patchnotes/bot.toml`, kein Editor
- Speichern startet keinen Dienst. Vorhanden: `bot-restart` und
  `dl-bot` master restart_embed
