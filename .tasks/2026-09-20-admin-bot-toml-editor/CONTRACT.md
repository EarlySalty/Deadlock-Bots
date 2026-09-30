# Bot-TOMLs in den Admin-Dashboards

## Nutzerauftrag
Nicht geheime TOML-Betriebskonfigurationen ohne Serverzugang bearbeiten. Discord-Admin bündelt Discord, Steam, Patchnotes, Brain und Turniere; Twitch bleibt im Twitch-Admin. Tests, Review, Merge, Deployment, Neustart und echte Live-Prüfung gehören dazu.

## Umsetzung
Ein zentraler, registrierter Dateieditor im bestehenden Rust-Dashboard. Die serverseitige TOML-Registry liegt neben der externen Discord-Konfiguration und ist selbst niemals ein Editierziel. Der Browser darf nur bekannte Bot-IDs auswählen, keine Dateipfade, Programme oder Units. Schemaprüfung erfolgt durch den nativen Discord-Lader bzw. ausdrücklich konfigurierte, nebenwirkungsfreie Prüfprogramme des jeweiligen Bots. Zugangsdaten bleiben in Infisical. Keine generische ENV-Tabelle, keine zweite Bot-Konfigurationsquelle.

Lesen liefert TOML ohne Originalkommentare (keine versehentlichen Geheimnisse in Kommentaren). Speichern erhält unveränderte Kommentare, verwendet Revisionsvergleich und Dateisperre, validiert die gesamte Datei, sichert Vorgängerversionen und ersetzt atomar. Aktivieren ist getrennt und auf feste Bot-Units begrenzt. Der Zustand des laufenden Prozesses wird nicht aus erfolgreichem Speichern abgeleitet. Nicht fertig ausgerollte TOML-Verbraucher werden ehrlich als nicht verbunden angezeigt und nicht blind neu gestartet.

## Scope
rust/crates/dl-core/**
rust/crates/dl-dashboard/**
rust/bin/dl-web/**
service/static/operating-config.js
service/static/bot-config-editor.*
service/static/dashboard.html
scripts/admin-bot-config/**
.tasks/2026-09-20-admin-bot-toml-editor/**

Twitch-Anschluss in einem separaten isolierten Worktree: bestehende Admin-Auth/CSRF, interne serverseitige Brücke mit bestehendem Diensttoken, eigene Admin-Seite. Keine Änderungen an den parallel laufenden TOML-Migrationen oder an Profil/STT/Brain-Portierungen ohne sichere Integration.

## Abnahme
Auth vor Datei-/Prozesszugriff, CSRF auf Mutationen, keine Secrets in Antworten/Fehlern, keine freien Pfade/Units/Programme, Schema- und Größenprüfung, CAS/Lock/atomare Speicherung, Erhaltung unveränderter Werte, ehrlicher Laufzeitstatus, Draft-Erhalt bei Fehlern, sichere Aktivierung, bestehende Tests und Self-Review-Gate. Keine Live-/Mergebehauptung ohne tatsächlichen Nachweis.
