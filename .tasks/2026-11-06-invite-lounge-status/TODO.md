# TODO: Invite-Lounge Status, Brain entscheidet die Rückfrage

Stand: 2026-11-07. Nicht gemergt, nicht deployt, kein Live-Test.

## Ziel und Abnahme

- Postet ein Mitglied einen Freundescode in #deadlock-invite, antwortet der Brain mit dem echten Stand.
- 30 Minuten nach der Anfrage fragt der Brain mit Erwähnung nach, ob die Einladung angekommen ist.
- Die Antwort wertet der Brain aus (arrived / not_arrived / unclear) und schreibt den Text. Keine Wortlisten mehr.
- not_arrived löst die Einladung erneut aus, arrived bestätigt ohne neue Einladung, unclear fragt nach.
- Verboten: Merge nach main, Deploy, Restart, Live-Post auf Discord, Steam-Aktion.

## Brain (Repo Deadlock-Brain)

- Branch `brain-invite-status-20261106` ist gepusht, HEAD `b30f2879f9394443139cd4af2ff1006a1a55386c` (Merge mit origin/main). Enthält den Judge-Commit `f5348e1a`.
- Enthalten: `InviteCheckInOutcome`, `ConversationTask::DiscordInviteCheckIn`, `ReplySignal::InviteCheckIn`, `persona::invite_check_in_judge`, Prompt `invite_check_in_judge.txt`, Dispatch in `conversation.rs`, Test `invite_check_in_reports_the_judged_outcome` in `brain-api/tests/conversation_task.rs`.
- Geprüft: `cargo-slot check -p brain-api -p brain-contracts --all-targets` grün, `cargo-slot clippy -p brain-api -p brain-contracts --all-targets` Exit 0.
- NICHT gelaufen: `cargo-slot test -p brain-api --test conversation_task invite` und `cargo-slot test -p brain-api --lib`. Grund: brain-api-Tests werden als DB-Tests klassifiziert und nur auf ionos (100.89.130.17) gefahren. ionos ist nicht erreichbar (ssh Exit 255, cargo-slot Exit 78). Kein Umgehen über rohes `cargo test`.
- Nächster Schritt: sobald ionos erreichbar ist, beide Testläufe ausführen. Bei Fehlern im Brain fixen, neu committen, pushen, SHA in den Bots nachziehen.

## Bots (Worktree `/home/nathanael/.worktrees/invite-lounge-status-20261106`, Branch `invite-lounge-status-20261106`)

- `af9fe0f4`: Judge-Commit (Brain entscheidet).
- `29438acb`: Merge von origin/main (d940c35a). Konflikte aufgelöst: Brain-Pin auf `b30f2879f9394443139cd4af2ff1006a1a55386c` (`dl-brain/Cargo.toml` beide Einträge, `Cargo.lock` drei Brain-Quellen), Test-Brain-Import behält `DiscordAnswerTask` und `InviteCheckInOutcome`.
- Enthalten (Kurzfassung): `record_check_in_answer` ruft den Brain nur bei gebundener Antwort auf. `apply_check_in`: Arrived setzt Attempted und Confirmed (Post nur bei neuem Claim), NotArrived setzt `invite_now`, Unclear ändert nichts außer `last_seen`. Wortliste und `answer_check_in` entfernt.

Verifiziert nach dem Merge:

- `cargo-slot check -p dl-brain`: Exit 0.
- `cargo-slot check -p dl-community --all-targets`: Exit 0. Fünf Warnungen sind die bekannten feature-gegateten Test-Helfer (`gate_first_call`, `gated_knowledge_server`, `gated_two_response_knowledge_server`, `block_next_port_call`, `evidence_server`).
- `cargo-slot test -p dl-brain`: 37 bestanden.
- `cargo-slot test -p dl-community invite_lounge`: 70 bestanden.
- `cargo-slot clippy -p dl-community -p dl-brain --all-targets`: Exit 0. Meldungen bei `dl-brain/src/lib.rs:217` und `concierge.rs:1572` liegen nicht in den geänderten Zeilen.
- `rustfmt --check` auf alle gestagten Rust-Dateien: sauber.
- `.tasks/2026-11-06-invite-lounge-status/VORSCHAU.md` und `AUFTRAG.md` auf Brain-Entscheidung umgestellt.

Nicht gelaufen:

- DB-backed dl-community-Tests (4 Stück, u. a. `privacy_loeschung_funktioniert_im_turniere_runtime`, `self_service_signup_schreibt_nur_im_legacy_runtime`): brauchen `CENTRAL_TEST_DSN`, also ionos. Setup-Fehler vor jeder Logik.

## Offen, in dieser Reihenfolge

1. ionos erreichbar machen oder warten. Dann Brain-Tests (siehe Brain oben) und DB-Tests laufen lassen.
2. Gate Brain: `gate-review --repo /home/nathanael/.worktrees/brain-invite-status-20261106 --base origin/main` mit Skill `rolle-merge-schleuse`. Vor Merge.
3. Gate Bots: `gate-review --repo /home/nathanael/.worktrees/invite-lounge-status-20261106 --base origin/main`. Erst nach Brain-Merge auf main, weil der Pin sonst nicht auf main liegt.
4. Bei BLOCK: Skill `gate-learning` vor dem Fix, Ledger-Zeile per flock nach `/home/nathanael/.claude/gate-learning/ledger.jsonl`.
5. Reihenfolge für Deploy: Brain-Branch nach main, dann Bots. Die Bots brauchen den Brain-Pin.
6. Nicht gemacht: Live-Test in Discord und Steam (AUFTRAG verbietet das ohne Freigabe).

## Offene Punkte für den Nutzer

- Die Status-Notices (30-Minuten-Nachfrage, FA raus, Einladung raus, ...) laufen über den Brain. Der Claim wird vor dem Brain-Aufruf committet (`post_notice` nach CAS). Fällt der Brain bei einer Notice aus, bleibt der Zustand geclaimt und die Nachricht fehlt. Das gilt schon für die bisherigen Status-Notices und wird mit der Nachfrage relevanter. Nicht umgesetzt. Fix-Weg: Brain-Text vor dem CAS erzeugen, bei Fehler ohne Zustandswechsel abbrechen. Das ändert drei Stellen (`check_in`, `dispatch_due`, `finish_dispatch`) und die Reihenfolge der `ScriptedBrain`-Antworten in den Tests. Vor dem Live-Test umsetzen.
- Gate-Lern-Themen noch nicht ausgewertet (`python3 -I /home/nathanael/Documents/claude-config/tools/gate-learning/ledger-themes.py`).

## Entscheidungen (getroffen, dem Nutzer nennen)

- Arrived, NotArrived oder Unclear kommen aus der Brain-Auswertung. Konfidenz unter 0.7 zählt als Unclear.
- Brain-Fehler bei der Antwort des Nutzers: kein Zustandswechsel, kein Post, Nachricht bleibt unverbraucht.
- Ein "ja" ohne Bezug auf Anfrage oder Rückfrage wird ignoriert und löst keinen Brain-Aufruf aus.
