# Deterministisches PR-Gate

Auftrag vom 23. September 2026: ausschließlich EarlySalty/Deadlock-Bots ändern.
Eigener Worktree: /home/nathanael/.worktrees/db-deterministic-pr-gate-20260923.
Branch: ci/deterministic-pr-gate-20260923. Basis: ff635f7b354cb09909c01ddd6f773d0682dd89c9.

PR-Testbetrieb: offen lassen, kein Merge, kein main-Push, kein Deploy und kein Neustart.
Keine fremden Worktrees, produktiven DSNs, Secrets oder Schutzregeln ändern.

Abnahme: Required PR Gate auf jedem PR; Fehler, fehlende Jobs und unerwartete
Skips blockieren. Rust, Python, DB-Verträge, Secrets, Dependencies, Semgrep,
Trivy, Actions und CodeQL prüfen. Bestehende erfolgreiche Config-Commands
übernehmen. Scanner- und Gate-Gegenproben ausführen. Policy und verbleibende
Blocker in .github/SECURITY-CI.md und REPORT.md dokumentieren.
