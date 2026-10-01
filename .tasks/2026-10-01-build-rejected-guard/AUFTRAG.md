status: aktiv
Datum: 2026-10-01

# Auftrag: BuildRejected-Guard in Bots #459

## Ziel

Im aktuellen Bots Typed Consumer erhalten `Answered` und `BuildRejected` exakt denselben fail-closed Antwortvertrag: höchstens 3.800 UTF-16-Codeeinheiten und keine `http://`- oder `https://`-URLs. Ungültige Antworten werden als Backendfehler zurückgegeben.

## Befund

Die unabhängige Abnahme `/home/nathanael/Documents/.tasks/2026-10-01-offene-arbeit/abnahmen/brain-c9-sechs-consumer-gruppe.md` bestätigt F1: `rust/crates/dl-brain/src/brain_api.rs:94` gibt `BuildRejected` ohne den Guard für `Answered` zurück. Der Brain-Public-Response-Vertrag erlaubt bis zu 64 KiB Text.

## Arbeit

1. Ändere ausschließlich `rust/crates/dl-brain/src/brain_api.rs` im bestehenden Worktree `/home/nathanael/.worktrees/open-pr-459-brain-consumer-20261001`, Branch `codex/fix-c9-consumer-wiring`.
2. Stelle sicher, dass `Answered` und `BuildRejected` dieselbe URL- und UTF-16-Längenvalidierung verwenden und ihre bisherigen gültigen Statusabbildungen behalten.
3. Ergänze gezielte Regressionen für URL-haltige und zu lange `BuildRejected`-Texte. Keine Regression darf das globale 64-KiB-API-Limit ändern.

## Grenzen

Keine Änderungen im Brain-Repo oder an anderen Bots-Dateien. Keine Modell-/RAG-Fallbacks, Retries, Scope-Erweiterungen oder API-Änderungen. Keine Kommentare ergänzen. Keine Builds, Tests, Gate-Läufe, Commits, Pushes, Einzelintegrationen, Main-Merges oder Live-Schritte. Host-Resource- und TokenDB-Holds bleiben aktiv.

## Abschluss

Nach den Quelländerungen sofort stoppen und Pfade, Diffumfang sowie die nicht ausgeführten Prüfungen melden. Danach warten: frisches Selbstgate und unabhängige Intent-Abnahme kommen separat; schwere Tests und Gate-Runs erst nach Host-Gatefreigabe.
