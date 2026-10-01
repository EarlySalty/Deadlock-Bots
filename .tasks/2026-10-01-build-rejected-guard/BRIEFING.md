status: aktiv
Datum: 2026-10-01

# Worker-Briefing: BuildRejected-Guard

Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder Unter-Agenten starten.

Intent-Thread: `72827c3e-d4e0-428a-8e4d-8d740679e668`.
Auftrag: `.tasks/2026-10-01-build-rejected-guard/AUFTRAG.md`.
Worktree: `/home/nathanael/.worktrees/open-pr-459-brain-consumer-20261001`.
Branch: `codex/fix-c9-consumer-wiring`.
Ausgangs-HEAD: `46edc1023a819ba0ba3c37b7de96c333f485f797`.

Setze nur den Auftrag um. Ausschließlich `rust/crates/dl-brain/src/brain_api.rs` ändern. `Answered` und `BuildRejected` müssen identisch auf URLs (`http://`, `https://`) und mehr als 3.800 UTF-16-Codeeinheiten fail-closed reagieren. Gültige Statusabbildungen behalten. Gezielte Regressionen für URL und oversized `BuildRejected` ergänzen. Das globale 64-KiB-API-Limit bleibt unverändert.

Keine Kommentare ergänzen. Keine Builds, Tests, Gate-Läufe, Commits, Pushes, Merges oder Live-Schritte. Keine weiteren Dateien oder Repositories ändern. Nach dem Quellfix sofort stoppen und `[Bump-up] Paket F1: Grund: Quellfix vorbereitet, Ressourcengrenzen verhindern weitere Prüfungen. Erledigt: ... Worktree: /home/nathanael/.worktrees/open-pr-459-brain-consumer-20261001 Offen: frisches Selbstgate und unabhängige Intent-Abnahme; Tests/Gate nach Host-Gatefreigabe` an den Intent-Thread melden.
