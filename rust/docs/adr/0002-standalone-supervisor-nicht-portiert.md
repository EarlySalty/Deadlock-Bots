# ADR 0002: standalone_manager (Subprocess-Supervisor) wird nicht portiert

Status: angenommen · Datum: 2026-06-10 · Entscheider: Nani („verlasse mich auf dich") + Datenlage

## Kontext

`service/standalone_manager.py` startet Hilfsbots als Kind-Prozesse
(`asyncio.create_subprocess_exec`), mit Restart-on-Crash und Daily-Restart. Das
Dashboard bietet dafür Start/Stop/Logs. Kommunikation läuft über die Tabellen
`standalone_bot_state` (Status-Heartbeat) und `standalone_commands` (Befehls-Queue).

## Datenlage (10.6.2026, read-only geprüft)

- `standalone_bot_state` enthält 2 Einträge: `rank` (letzter Heartbeat **23.2.2026** —
  seit Monaten tot) und `steam` (Heartbeat heute — geschrieben vom **Rust-Steam-Bot**,
  der längst als eigener systemd-Service läuft, nicht als Kind-Prozess).
- `standalone_commands`: letzter Eintrag **19.2.2026**. Die Befehls-Queue wird nicht
  mehr benutzt.

Der Supervisor beaufsichtigt also real nichts mehr; Prozess-Lifecycle macht systemd.

## Entscheidung

- Der Subprocess-Supervisor wird **nicht** nach Rust portiert.
- Der **Tabellen-Vertrag bleibt**: `standalone_bot_state` wird weiterhin GELESEN
  (build_publisher-Nachfolger prüft den Steam-Runtime-Status darüber; das Dashboard
  bekommt eine reine Status-Ansicht). `steam_tasks` als DB-Queue zum Steam-Bot bleibt
  unverändert bestehen.
- Python-Code bleibt unangetastet liegen (Strangler-Regel: nichts löschen).

## Konsequenzen

- Die Dashboard-Endpunkte für Start/Stop/Restart von Standalone-Bots entfallen in
  Phase 9 ersatzlos; Prozesssteuerung ist Sache von systemd.
- Sollte je wieder ein Hilfsbot nötig sein: eigener systemd-Service, kein eingebauter
  Supervisor.
