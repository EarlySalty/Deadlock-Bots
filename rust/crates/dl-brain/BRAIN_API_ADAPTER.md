# Typisierter Brain-Command-Adapter

Stand: 2026-09-26. C9 verdrahtet den vorhandenen `BrainApiAnswerer` in die echte `brain`-Command-Composition. Der bestehende Pfad bleibt Default und wird nicht produktiv abgeschaltet.

Der Adapter verwendet ausschließlich den kanonischen `AsyncBrainClient` aus Deadlock-Brain, gepinnt auf `3b86d3cbe5ea39a67b8b1fbd8a3d48ab935982ef`. Im neuen Adapter existiert kein direkter Modell- oder RAG-Fallback.

## Runtime-Modi

`runtime.ai.brain_client_mode` in der normalen `bot.toml` steuert ausschließlich die Composition:

- `legacy` — Default; bisheriger `SharedBrainAnswerer`
- `typed` — sichtbare Antworten ausschließlich über brain-serve
- `shadow` — bisherige Antwort bleibt sichtbar, der typisierte Client läuft zusätzlich report-only

Für `typed`/`shadow` stehen `runtime.ai.brain_api_endpoint` und `runtime.ai.brain_api_scopes` als typisierte Werte in derselben Datei; `runtime.ai.brain_api_timeout_ms` ist optional und beträgt ohne Angabe 8000 Millisekunden. Der Endpunkt muss lokal sein, Scopes müssen nichtleer und gültig sein. Der API-Token kommt ausschließlich als `BRAIN_API_TOKEN` über den bestehenden Infisical-Secret-Bootstrap und steht niemals in der TOML. Bei fehlender oder ungültiger Konfiguration scheitert der Start, statt den Command still abzuschalten. Die bestehenden `BRAIN_CMD_ENABLED`-, Channel-Allowlist-, Open-Test-, Cooldown-, Längen- und Emoji-/Ausgabe-Regeln bleiben bestehen.

Im `shadow`-Modus protokolliert `ReportOnlyShadowBrainAnswerer` nur grobe Ergebnisarten des typisierten Pfads. Er verändert die sichtbare Antwort nicht und führt keine Merge-/Deployment-Aktion aus.

## Statusabbildung

- `answered` → bisherige `BrainOutcome::Answer`
- `build_rejected` → erklärender Text als `BrainOutcome::Answer`, damit das bestehende Discord-Ausgabeformat erhalten bleibt
- `insufficient_evidence` → `NoAnswer`
- `unavailable`, `unauthorized_evidence`, `provider_error`, `budget_exceeded` → Backendfehler; kein stiller Legacy-Fallback im typed-Modus

Die bestehende Einmal-Conversation pro Anfrage bleibt erhalten, weil der Command-Port keine vertrauenswürdige persistente Conversation-ID besitzt.

## Sicherheit und Prüfung

Der unabhängige Semantic-Review-Gate fand im vorhandenen direkten Mergepfad eine Base-SHA-TOCTOU-Race. C9 entfernt deshalb den mutierenden `updateBranch`-/`pulls.merge`-Pfad vollständig: das PR-Release-Gate besitzt nur Leserechte und meldet grüne Gates report-only; Branch-Update und Merge bleiben manuell.

Die C9-Unit-/Fixture-Tests starten keinen Bot und senden keine Discord-Nachricht. Die vollständigen `dl-bot`-DB-Integrationstests benötigen weiterhin `CENTRAL_TEST_DSN` und sind als separater lokaler Test dokumentiert.

Keine Produktionskonfiguration wurde geändert und kein Deployment ausgeführt.
