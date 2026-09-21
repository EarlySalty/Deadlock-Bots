# Auftrag: toml-admin-dashboard

status: aktiv (2026-09-20)

## Ziel

Aus dem Discord-Admin die TOML-Betriebseinstellungen der Bots ändern, die
bereits TOML nutzen, ohne auf den Server zu gehen. Twitch gehört ins Twitch
Admin Dashboard. Nach dem Speichern soll der laufende Dienst den neuen Stand
übernehmen. Merge und Release bleiben nur für Code, nicht für Alltagsänderungen
an der Betriebsdatei.

## Arbeitsschritte

1. Bestand nicht neu bauen. Discord- und Steam-Editor im bestehenden Tab
   Betriebseinstellungen wiederverwenden. Keine neue Dashboard-Route.
2. Twitch: vorhandene Seite Betriebseinstellungen von origin/main live
   schalten. Die vom Startskript erwartete Datei
   `/var/lib/deadlock-twitch/config/bot.toml` aus der bereits belegten
   Nichtsecret-Datei `~/.config/deadlock-twitch/bot.toml` installieren, nicht
   aus Beispielwerten. Danach `deploy-twitch-release` auf origin/main und
   `bot-restart` für twitch-bot und twitch-dashboard.
3. Patchnotes: im selben Discord-Tab Betriebseinstellungen eine Karte
   Patchnotes ergänzen. Lesen und atomar schreiben genau
   `~/.config/deadlock-bots/patchnotes/bot.toml`. Nur bereits verdrahtete
   Nichtsecret-Felder, keine Secrets, keine freien Dateipfade.
4. Nach Speichern den betroffenen Dienst über den bestehenden Weg neu
   starten (`bot-restart` bzw. vorhandener Master-Restart). Keine neue
   allgemeine systemctl-API. Gespeicherter und aktiver Fingerabdruck bleiben
   getrennt sichtbar.
5. Browser: Discord-Admin Tab Betriebseinstellungen und Twitch-Admin
   `/twitch/admin/config/operating` durchklicken, speichern, Konflikt, Status.

## Fundstellen (Intent-Vorcheck)

- `Deadlock-Bots/service/static/operating-config.js:1`: Discord- und
  Steam-Karten, PATCH mit revision, Speichern startet keinen Dienst
- `Deadlock-Bots/service/static/dashboard.html:1656`: Tab
  Betriebseinstellungen, Anker `#betrieb`
- `Deadlock-Bots/rust/crates/dl-dashboard/src/operating_config.rs:1`:
  Admin-API mit guard_full / guard_mutate und CSRF
- Live `dl-web` enthält `/api/admin/betriebskonfiguration` und
  `/api/admin/steam-betriebskonfiguration`
- Live `dl-bot --config /home/nathanael/.config/deadlock-bots/bot.toml`
- `Deadlock-Steam-Bot/rust/crates/steam-config/src/editor.rs` und
  `rust/crates/steam-web/src/routes/config.rs`: interner GET/PATCH
- `Deadlock-Steam-Bot/rust/deploy/run-steam-bot.sh:22`:
  `STEAM_CONFIG_PATH=$HOME/.config/deadlock-steam-bot/bot.toml`
- `Deadlock-Twitch-Bot` origin/main
  `bot/admin_dashboard/src/pages/config/OperatingConfig.tsx`:
  Seite Betriebseinstellungen, Route `/config/operating`
- `Deadlock-Twitch-Bot` origin/main
  `rust/scripts/run_tb_bot_service.sh:7`: erwartet
  `/var/lib/deadlock-twitch/config/bot.toml`
- Live Twitch Release `818e2152`, Units lesen noch
  `/etc/deadlock-twitch/*.conf`
- `Deadlock--Patchnotes-Bot` live
  `~/.config/deadlock-bots/patchnotes/bot.toml`, Editor fehlt
- Restart-Bestand: `/usr/local/bin/bot-restart`,
  `Deadlock-Bots/rust/bin/dl-bot/src/master.rs` restart_embed

## Was nicht angefasst wird

- Turniere (fremder paralleler Stand, keine produktive TOML-Umschaltung)
- Secrets, Infisical-Exporte, ENV-Dateien auslesen
- Neue Admin-Loginwege, neue Dashboards, freie Dateipfade
- Modell- oder Preisänderungen in der Oberfläche
- Helix-Hinweis-Branch, Session 65d5c809, Merge-BLOCK nicht retrien
- Community-Posts, Testnachrichten nach außen

## Fertig-Kriterium

1. Discord-Admin `https://admin.deutsche-deadlock-community.de/admin#betrieb`:
   Discord, Steam und Patchnotes speichern in die Datei, die der Dienst liest.
2. Twitch-Admin
   `https://admin.deutsche-deadlock-community.de/twitch/admin/config/operating`:
   Seite sichtbar, Speichern schreibt
   `/var/lib/deadlock-twitch/config/bot.toml`.
3. Nach Speichern wechselt der aktive Fingerabdruck ohne SSH. Ort im UI plus
   PID-Wechsel nach `rolle-deploy-verifizierer`.
4. Alltagsweg ohne Merge und ohne Release für bloße Werteänderungen.

## Deploy-Weg

- Twitch: `deploy-twitch-release <origin/main-sha>` und
  `bot-restart twitch-bot twitch-dashboard`
- Discord/Web/Patchnotes/Steam: bestehender User-Unit-Weg über `bot-restart`
  (`dl-bot`, `web`, `patchnotes`, `steam-bot`, bei Katalogwechsel beide Cores
  zuerst stoppen)
- Ein Host, höchstens ein Cargo-Release-Build gleichzeitig

## Rahmen

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder
  Unter-Agenten spawnen.
- Keine Code-Kommentare schreiben, Code erklärt sich selbst.
- Nur den eigenen Branch pushen, nie main.
- Auftrag größer als beschrieben: Bump-up-Nachricht an den Intent-Thread, dann
  stoppen.
