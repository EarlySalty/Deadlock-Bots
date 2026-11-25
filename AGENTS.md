# Token-Speichergrenze

Für Arbeiten an Authentifizierung und Zugängen gilt `docs/token-storage-db.md`. Bestehende Repositoryregeln bleiben bestehen. Keine Token-Dateien neu anlegen, Konto-Zugänge nur verschlüsselt in der DB und reine Session-/Einmalwerte als Lookup-Hash speichern. Infrastruktur-Schlüssel bleiben im vorhandenen Secret-Manager.
