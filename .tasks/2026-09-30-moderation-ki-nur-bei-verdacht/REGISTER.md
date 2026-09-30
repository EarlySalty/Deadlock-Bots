status: aktiv
Datum: 2026-09-30

# Register

Ziel: Inhalts-KI nur nach Heuristik-Signal, Freitext-Konsistenzprüfung entfernt.
Stufe mittel. Ersetzt `fix/moderation-negierte-scam-hinweise`.
Worktree: `/home/nathanael/.worktrees/dl-moderation-ki-nur-bei-verdacht`.
Branch: `fix/moderation-ki-nur-bei-verdacht`, Basis `de34acc8`.

## Thread-Register (T3)

| Paket | Thread-ID | Modell | Status | Letzte Meldung |
| --- | --- | --- | --- | --- |
| A Umbau | 87451a50 | claude-opus-5-5 | in Arbeit | gestartet 30.09. |
| Alt: Negationsparser Gate-Fix | 47530bcd | gpt-6-luna | gestoppt, nicht wieder aufnehmen | ersetzt durch Paket A |

## Offen
Unabhängiger Review, lokales Merge-Gate, Merge, Deploy `deadlock-bot-rust`, Live-Beweis im Journal (keine KI-Urteile ohne Verhaltenssignal), danach alten Branch und Worktree `dl-moderation-negierte-scam-hinweise` löschen.
