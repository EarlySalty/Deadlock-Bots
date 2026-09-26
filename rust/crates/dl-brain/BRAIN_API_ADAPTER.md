# Typisierter Brain-Command-Adapter

Stand: 2026-09-26. C9 verdrahtet den vorhandenen `BrainApiAnswerer` in die echte `brain`-Command-Composition. Der bestehende Pfad bleibt Default und wird nicht produktiv abgeschaltet.

Der Adapter verwendet ausschließlich den kanonischen `AsyncBrainClient` aus Deadlock-Brain, gepinnt auf `3b86d3cbe5ea39a67b8b1fbd8a3d48ab935982ef`. Im neuen Adapter existiert kein direkter Modell- oder RAG-Fallback.

## Runtime-Modi

`BRAIN_CLIENT_MODE` steuert ausschließlich die Composition:

- `legacy` — Default; bisheriger `SharedBrainAnswerer`
- `typed` — sichtbare Antworten ausschließlich über brain-serve
- `shadow` — bisherige Antwort bleibt sichtbar, der typisierte Client läuft zusätzlich report-only

Für `typed`/`shadow` werden `BRAIN_API_ENDPOINT`, `BRAIN_API_TOKEN`, `BRAIN_API_SCOPES` und optional `BRAIN_API_TIMEOUT_MS` gelesen. Scopes sind nichtleer und kommen nur aus der vertrauenswürdigen Runtime-Konfiguration. Der BrainClient begrenzt Ziele auf lokale Endpunkte. Die bestehenden `BRAIN_CMD_ENABLED`-, Channel-Allowlist-, Open-Test-, Cooldown-, Längen- und Emoji-/Ausgabe-Regeln bleiben bestehen.

Im `shadow`-Modus protokolliert `ReportOnlyShadowBrainAnswerer` nur grobe Ergebnisarten des typisierten Pfads. Er verändert die sichtbare Antwort nicht und führt keine Merge-/Deployment-Aktion aus.

## Statusabbildung

- `answered` → bisherige `BrainOutcome::Answer`
- `build_rejected` → erklärender Text als `BrainOutcome::Answer`, damit das bestehende Discord-Ausgabeformat erhalten bleibt
- `insufficient_evidence` → `NoAnswer`
- `unavailable`, `unauthorized_evidence`, `provider_error`, `budget_exceeded` → Backendfehler; kein stiller Legacy-Fallback im typed-Modus

Die bestehende Einmal-Conversation pro Anfrage bleibt erhalten, weil der Command-Port keine vertrauenswürdige persistente Conversation-ID besitzt.

## Sicherheit und Prüfung

Das vorhandene PR-/Release-Gate wurde durch C9 nicht gelockert. Der vorherige Completion-Branch war nur Ausgangsmaterial; die dortige Änderung an `.github/workflows/pr-release-gate.yml` wurde bewusst nicht übernommen. Auto-Merge bleibt außerhalb des Brain-Adapters und report-only/sicher.

Die C9-Unit-/Fixture-Tests starten keinen Bot und senden keine Discord-Nachricht. Die vollständigen `dl-bot`-DB-Integrationstests benötigen weiterhin `CENTRAL_TEST_DSN` und sind als separater lokaler Test dokumentiert.

Keine Produktionskonfiguration wurde geändert und kein Deployment ausgeführt.
