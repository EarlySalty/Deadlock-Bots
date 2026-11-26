# Token-Speichergrenze

Für Arbeiten an Authentifizierung und Zugängen gilt `docs/token-storage-db.md`. Bestehende Repositoryregeln bleiben bestehen. Keine Token-Dateien neu anlegen, Konto-Zugänge nur verschlüsselt in der DB und reine Session-/Einmalwerte als Lookup-Hash speichern. Infrastruktur-Schlüssel bleiben im vorhandenen Secret-Manager.

# Code review and task flow

Every change follows `CODE-REVIEW.md`: deterministic checks first, one deep review
with fixed fan-out, only high findings block, findings frozen after round 1, then
verification only. It also contains the orchestrator start prompt and the
20-minute default-decision rule. Work on your own branch; only the maintainer
merges to `main`.
