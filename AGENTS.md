# Token-Speichergrenze

Für Arbeiten an Authentifizierung und Zugängen gilt `docs/token-storage-db.md`. Bestehende Repositoryregeln bleiben bestehen. Keine Token-Dateien neu anlegen, Konto-Zugänge nur verschlüsselt in der DB und reine Session-/Einmalwerte als Lookup-Hash speichern. Infrastruktur-Schlüssel bleiben im vorhandenen Secret-Manager.

# Code review and task flow

Every task follows `CODE-REVIEW.md`: one orchestrator session per task, Opus at
medium effort (high only for risky areas), deterministic checks first, one
`/code-review` run plus one confirmation run, then one pull request with the
review result in its description. It also contains the orchestrator start prompt
and the 20-minute default-decision rule. Never push to `main`; only the
maintainer merges.
