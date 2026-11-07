# TODO: Invite-Lounge Status, Brain entscheidet die Rückfrage

Stand: 2026-11-07. Quota ist erschöpft, die Arbeit ist nicht abgeschlossen.

## Ziel und Abnahme

- Postet ein Mitglied einen Freundescode in #deadlock-invite, antwortet der Brain mit dem echten Stand.
- 30 Minuten nach der Anfrage fragt der Brain mit Erwähnung nach, ob die Einladung angekommen ist.
- Die Antwort wertet der Brain aus (arrived / not_arrived / unclear) und schreibt den Text. Keine Wortlisten mehr.
- not_arrived löst die Einladung erneut aus, arrived bestätigt ohne neue Einladung, unclear fragt nach.
- Verboten: Merge nach main, Deploy, Restart, Live-Post auf Discord, Steam-Aktion.

## Brain (Repo Deadlock-Brain)

- Branch `brain-invite-status-20261106` ist gepusht, Commit `f5348e1a8214923eedb933a9bdbbb77b6a180f19`. Die Commit-Message endet mit `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`.
- Enthalten: `InviteCheckInOutcome`, `ConversationTask::DiscordInviteCheckIn`, `ReplySignal::InviteCheckIn`, `persona::invite_check_in_judge`, Prompt `invite_check_in_judge.txt`, Dispatch in `conversation.rs`, Test `invite_check_in_reports_the_judged_outcome` in `brain-api/tests/conversation_task.rs`.
- Geprüft: `cargo-slot check -p brain-api --all-targets` grün, `cargo-slot clippy -p brain-api -p brain-contracts --all-targets` ohne Meldungen.
- NICHT gelaufen: `cargo-slot test -p brain-api --test conversation_task invite`. Grund: brain-api-Tests werden als DB-Tests klassifiziert und nur auf ionos (100.89.130.17) gefahren. ionos antwortet nicht (ssh Timeout auf Port 22). `CARGO_SLOT_REMOTE=0` wird für diese Klassifizierung abgelehnt (Exit 78). Kein Umgehen über rohes `cargo test`.
- Nächster Schritt: sobald ionos erreichbar ist, den Test laufen lassen. Bei Fehlern im Brain fixen, neu committen, pushen, SHA in den Bots nachziehen.

## Bots (Worktree `/home/nathanael/.worktrees/invite-lounge-status-20261106`, Branch `invite-lounge-status-20261106`)

Unkommittiert, noch nicht gepusht:

- `rust/crates/dl-brain/Cargo.toml` und `Cargo.lock`: Brain-Pin auf `f5348e1a8214923eedb933a9bdbbb77b6a180f19` gesetzt (brain-client und brain-contracts).
- `rust/crates/dl-brain/src/lib.rs`, `brain_api.rs`: `ConversationReply::Answered { check_in }`, `DiscordConversation::InviteCheckIn`, Mapping von `ReplySignal::InviteCheckIn`.
- `rust/crates/dl-community/src/invite_lounge.rs`: `record_check_in_answer` ruft den Brain nur bei gebundener Antwort auf (Antwort auf Anfrage- oder Rückfrage-Nachricht, oder Erwähnung). `apply_check_in` setzt Arrived auf Attempted plus Confirmed (Post nur bei neuem Claim), NotArrived setzt `invite_now`, Unclear ändert nichts außer `last_seen`. Helfer `converse`, `reply_to_request`, `post_notice`, `check_in_open`. `answer_check_in` und die Wortliste sind entfernt.
- `rust/crates/dl-community/src/invite_lounge_status.rs`: `check_in_answer` und Wortlisten entfernt, Test `bestaetigung_meldet_den_eingang`.
- `rust/crates/dl-community/src/invite_lounge_tests.rs`: Helfer `setup_check_ins`, `check_in_texts`, `mention_texts`. Tests angepasst: `nachfrage_nach_dreissig_minuten_und_nein_loest_die_einladung_aus`, `ja_auf_die_nachfrage_bestaetigt_ohne_neue_einladung`, `nachfrage_nimmt_nur_antworten_mit_bezug_zum_bot_an`. Neu: `unklare_antwort_bleibt_offen_und_die_rueckfrage_wird_gelesen`, `brain_fehler_bei_der_antwort_verbraucht_sie_nicht`.
- `rust/crates/dl-community/src/answer_support.rs` (Test-Brain): `check_in_reply`, `always_with_check_ins`, Queue für InviteCheckIn.
- `rust/crates/dl-community/src/concierge.rs`: Pattern auf `Answered { text, actions, .. }` angepasst.

Verifiziert:

- `cargo-slot check -p dl-brain`: grün.
- `cargo-slot check -p dl-community --all-targets`: grün. Restwarnungen sind feature-gegatete Test-Helfer (`gate_first_call`, `gated_knowledge_server`, `block_next_port_call`, `evidence_server`), die auch in HEAD nur aus gegateten Tests genutzt werden.
- `cargo-slot test -p dl-brain`: 36 bestanden.
- `cargo-slot test -p dl-community invite_lounge`: 70 bestanden.
- `cargo-slot test -p dl-community` gesamt: 295 bestanden, 4 fehlgeschlagen:
  - `privacy::runtime_gate_privacy_tests::privacy_loeschung_funktioniert_im_turniere_runtime`
  - `reaction_roles::runtime_gate_tests::reaction_role_schreibt_roster_nur_im_legacy_runtime`
  - `scrim_signup::tests::self_service_signup_schreibt_nur_im_legacy_runtime`
  - `scrim_signup::tests::signup_vergibt_teilnehmer_rolle_nur_wenn_sie_fehlt`
  Diese vier liegen nicht im Invite-Lounge-Pfad. Ob sie schon in HEAD fehlschlagen, ist nicht geprüft.

## Offen, in dieser Reihenfolge

1. Erledigt: die vier roten Tests scheitern alle an `TestHarness("CENTRAL_TEST_DSN muss gesetzt sein; rust/scripts/central_test_db.sh richtet die Testinstanz ein")`, also am Test-Setup, bevor Testcode läuft. Es ist ein Umgebungsfehler (zentrale Test-DB fehlt), kein Logikfehler. Sie laufen nur mit Test-DB, also auf ionos (`tools/test-db/test-db-run`), das ist aktuell nicht erreichbar. Eine HEAD-Baseline bringt denselben Setup-Fehler, weil der Fehler vor jeder Logik fällt.
2. Erledigt: `cargo-slot clippy -p dl-community --all-targets` Exit 0. Die Meldungen (`#[must_use]` in `dl-brain/src/lib.rs:217`, `concierge.rs:1572`, viele andere Crates) liegen nicht in meinen Zeilen.
3. Erledigt: `rustfmt --check` auf alle geänderten Dateien sauber.
4. Brain-Test auf ionos laufen lassen, sobald erreichbar (siehe Brain oben).
5. Bots committen (Trailer `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`), pushen.
6. `.tasks/2026-11-06-invite-lounge-status/VORSCHAU.md` und `AUFTRAG.md` prüfen. Wo sie das Wortlisten-Verhalten beschreiben, auf Brain-Entscheidung umstellen.
7. Gate: `gate-review --repo /home/nathanael/.worktrees/invite-lounge-status-20261106 --base origin/main` mit Skill `rolle-merge-schleuse`. Bei BLOCK nach Skill `gate-learning` klassifizieren, Klasse fixen, Ledger-Zeile per flock anhängen.
8. Reihenfolge für Deploy: Brain-Branch muss vor den Bots nach main, weil die Bots den Brain-Pin brauchen. Der Brain-Merge braucht ebenfalls das Gate.
9. Nicht gemacht: Live-Test in Discord und Steam (AUFTRAG verbietet das ohne Freigabe). Postgres-Tests (Feature `testing`) nur auf ionos.

## Offene Entscheidungen, die ich getroffen habe (dem Nutzer nennen)

- Arrived oder NotArrived oder Unclear kommen aus der Brain-Auswertung. Konfidenz unter 0.7 zählt als Unclear.
- Brain-Fehler oder Silent: kein Zustandswechsel, kein Post, Nachricht bleibt unverbraucht.
- Ein "ja" ohne Bezug auf Anfrage oder Rückfrage wird ignoriert und löst keinen Brain-Aufruf aus.
- Die Gate-Lern-Themen sind noch nicht ausgewertet (`python3 -I /home/nathanael/Documents/claude-config/tools/gate-learning/ledger-themes.py`).
