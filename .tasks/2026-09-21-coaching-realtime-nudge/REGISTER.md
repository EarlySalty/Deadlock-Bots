# Register: Coaching-Benachrichtigungen realtime (Nudge)

status: aktiv (2026-09-21) — Phase: R1 abgeschlossen (REVIEW.md komplett, 14:21Z), Fix-Runde B läuft (Fixer d349eb7d), A-Finding (Doc-Kommentar) wartet auf Orchestrator-Entscheidung

Auftrag: `.tasks/2026-09-21-coaching-realtime-nudge/AUFTRAG.md` (Deadlock-Bots)
Interface-Vertrag: `POST /internal/master/v1/coaching/notifications-nudge`

## Thread-Register (T3)

| Paket | Thread-ID | Modell | Status | Worktree | Letzte Meldung |
|---|---|---|---|---|---|
| Intent (diese Session) | 7581f506 | glm-5.3-flash (Grok-Harness) | laufend | — | Steuerung, Bump-ups, Review-Kette |
| A Bot-Seite | 627622fc | grok-4.6 (worker_mittel) | Fehler: Provider 402, gesettelt, nicht wieder aufnehmen | /home/nathanael/.worktrees/coaching-nudge-bots-20260921 | 402 Payment Required (Grok-Balance) seit Start 13:32Z, 0 Assistant-Meldungen, Worktree blieb clean @ bb03deb5 |
| B Website-Backend | 0c036a22 | grok-4.6 (worker_mittel) | Fehler: Provider 402, gesettelt, nicht wieder aufnehmen | /home/nathanael/.worktrees/coaching-nudge-website-20260921 | 402 Payment Required (Grok-Balance) seit Start 13:32Z, 0 Assistant-Meldungen, Worktree blieb clean @ b271acc |
| A Nachfolger | daf759ab | glm-5.3-flash (worker_mittel) | fertig, Fertigmeldung 13:55:55Z; danach 13:58:08Z error (glm-Kontingent leer) bei unbefugtem Gate/Merge-Eigversuch — bewusst nicht geweckt, Auftrag komplett | /home/nathanael/.worktrees/coaching-nudge-bots-20260921 | Commit 8d1c6e1f auf origin gepusht; dl-broker 33 grün, dl-community 232 grün / 3 basisbedingt rot (auf bb03deb5 nachgemessen), cargo build -p dl-bot grün, Worktree clean |
| B Nachfolger | 4085e71d | glm-5.3-flash (worker_mittel) | fertig, Fertigmeldung 13:53:58Z, keine offenen Rückfragen | /home/nathanael/.worktrees/coaching-nudge-website-20260921 | Commit 2de42c3 auf origin gepusht, cargo build + cargo test 165 grün (Test-Postgres über CENTRAL_TEST_DSN), Worktree clean |
| R1 Paket A | 6a44ab2b | review_1 (glm) | fertig — Review abgeschlossen 14:18:25Z: 1 Mangel (neuer `///`-Doc-Kommentar handlers.rs:1759-1761 gegen die Vorgabe „keine neuen Code-Kommentare", Duldungsentscheidung beim Orchestrator), übrige Prüfpunkte ohne Befund, Baseline-Rots gegenprobt; Hinweis: Merge bbc3d05d bringt Main-Inhalte in den Branch-Diff, Merge-Stand vor Push nachprüfen; 14:21Z von der Wache gesettelt, Liste steht in REVIEW.md | /home/nathanael/.worktrees/coaching-nudge-bots-20260921 | Prüfauftrag vollständig abgearbeitet; read-only, nichts gefixt |
| R1 Paket B | 6aa8b223 | review_1 (glm) | fertig — Review abgeschlossen 14:10:47Z: 1 Mangel M1 (Reschedule macht ‚created‘ wieder fällig ohne Nudge, platform.rs:979-988/:1025/:998-1006), übrige Prüfpunkte ohne Befund; 14:14Z von der Wache gesettelt, Liste steht in REVIEW.md | /home/nathanael/.worktrees/coaching-nudge-website-20260921 | Prüfauftrag vollständig abgearbeitet inkl. Gegenprobe der „vierten Stelle“; read-only, nichts gefixt |
| R1 Paket A opus48-Nachfolger | 4f14e725 | opus48 (claude-opus-4-8) | Fehler: Org-Sperre „Claude subscription access disabled“ 14:05Z, 0 Review-Arbeit; kein identischer Neuanwurf, weil Alt-Reviewer 6a44ab2b lebt und dasselbe Review fährt (Doppelspawn vermieden); 14:14Z gesettelt. Stirbt 6a44ab2b doch: Nachfolger --rolle review_1 auf fable, dann astra | /home/nathanael/.worktrees/coaching-nudge-bots-20260921 | — |
| R1 Paket B opus48-Nachfolger | dbcc4f2d | opus48 (claude-opus-4-8) | Fehler: Org-Sperre „Claude subscription access disabled“ 14:05Z, 0 Review-Arbeit; entbehrlich, da 6aa8b223 das Review fertigstellte; 14:14Z gesettelt, kein Neuanwurf | /home/nathanael/.worktrees/coaching-nudge-website-20260921 | — |
| R1 Paket B Duplikat | e2597fb9 | review_1 → opus48 (claude-opus-4-8) | versehentlich 14:04Z doppelt angelegt (veralteter Register-Lesestand dieser Wache); starb 14:04:01Z sofort („organization has disabled Claude subscription access", 0 Review-Arbeit); Abbruchbestätigung; 14:09Z gesettelt | — | — |
| Fixer B (M1) | d349eb7d | gpt-6-astra (fixer) | blockiert: Codex-Guthaben 0 (Resend 14:57Z fehlgeschlagen); entbehrlich, M1 stattdessen von der Hauptsession umgesetzt | /home/nathanael/.worktrees/coaching-nudge-website-20260921 | Erster Lauf starb 14:19:39Z sofort am Codex-Limit (0 Assistant-Meldungen, Worktree blieb clean @ 2de42c3); Resend 14:57:39Z (Seq 675038) scheiterte nach 2 s identisch. Ursache lt. Provider-Log: credits.balance=0E-10, hasCredits=false (limitId premium), kein primary/secondary-Fenster, kein resetsAt — guthaben-basiert (Muster wie Grok-402), kein Auto-Reset |
| M1 Umsetzung (Hauptsession) | 7581f506 | glm-5.3-flash (Grok-Harness) | umgesetzt 15:12Z, Commit 3c993a5 auf origin gepusht | /home/nathanael/.worktrees/coaching-nudge-website-20260921 | Nudge-Bedingung in update_appointment erweitert (cancelled-Übergang ODER scheduled_at-Re-Arm), +3/−2; cargo build grün, rustfmt clean, discord_broker-Unit-Tests 7/0; R2 + volle Suite offen, sobald ein Provider lebt |
| R2 Paket B | f693cdd8 | glm-5.3-flash-token (OpenRouter) | laufend, gestartet 15:15Z | /home/nathanael/.worktrees/coaching-nudge-website-20260921 | prüft nur M1 gegen REVIEW.md, Diff b271acc..3c993a5 |

## Entscheidungen

- grok-4.6 ist bis 21.09. 19:37 handsperriert (`kontingent sperren grok-4.6 --stunden 4`), Grund
  Provider-402 „Grok Build usage balance exhausted". Aufheben mit `kontingent freigeben grok-4.6`,
  sobald die Balance aufgefüllt ist.
- glm-5.3-flash: Kontingent leer, automatisch gesperrt bis 21.09. 22:02.
- **Codex/gpt-6-astra: Guthaben leer (Befund Wache 15:05Z, Fixer-Log d349eb7d).** „Codex usage limit reached"
  ist hier guthaben-basiert: credits.balance=0E-10, hasCredits=false, kein resetsAt — hilft kein Warten auf ein
  Zeitfenster, nur Aufladung. Betrifft auch den Gate-Chain-Kandidaten astra (14:56Z „ohne Urteil" passend).
  Nimmt der Fixer d349eb7d nach Aufladung einen Tick-Nudge an, läuft er auf dem vollen im Thread stehenden Auftrag weiter.
- opus48 zeigt zwar Kontingent „frei", scheitert aber am T3-Spawn mit „Your organization has disabled
  Claude subscription access for Claude Code" (Befund e2597fb9, 14:04:01Z) — opus48 ist
  aktuell nicht nutzbar. Nächster der review_1-Zeile wäre fable; Neuanwurf-Budget der laufenden
  R1-Threads ist jedoch verbraucht, weitere Entscheidungen trifft die Hauptsession/Review-Wache.
- Nachfolger laufen auf dem nächsten Modell der worker_mittel-Zeile (grok, glm, opus48): glm-5.3-flash.
  Fällt glm auch aus, folgt opus48.
- **M1-Entscheidung (Hauptsession, 14:21Z): Fix.** Der Nudge feuert zusätzlich, wenn update_appointment durch
  eine scheduled_at-Änderung den created-Re-Arm auslöst (platform.rs:979-988) — der Bestandscode setzt den Reset
  genau als Benachrichtigungs-Wille. Kein separater Nudge für den Reminder-Anteil allein; der harmlose
  Leere-Nudge bei Cancel+Reschedule in einem Request bleibt akzeptiert (Bestandsverhalten). Umsetzung durch
  Fixer d349eb7d (astra), danach R2 nur gegen die M1-Liste (review_2_4).

## Branches

- Bot: `feat/coaching-notifications-nudge-20260921`, Basis bb03deb5, Arbeits-Commit 8d1c6e1f;
  Stand 14:08Z bbc3d05d (Merge von origin/main über den Arbeits-Commit, Worktree clean).
  Review-Range bleibt bb03deb5..8d1c6e1f.
- Website: `feat/coaching-notifications-nudge-20260921`, Basis b271acc, Arbeits-Commit 2de42c3
  (Stand 14:08Z unverändert, Worktree clean).

## Offene Punkte

- **Merge-Gate-Budget repariert (2026-09-21, Hauptsession):** Der Pre-Push-Gate verweigerte strukturell ohne Urteil
  („Slot von 347s liegt unter der Untergrenze von 420s"): 1800s Hook-Budget minus 60s Reserve minus Vorarbeit
  reichen nie für 5-Modell-Ketten (review_1/intent). Fix: `REGISTERED_HOOK_TIMEOUT_SECONDS` und der
  settings.json-Timeout auf 2400s (gpt-workers-Commit 725d319, nur die Konstante; die fremden uncommitteten
  GLM-Treiber-Edits in gate_hook.py/test_gate_hook.py blieben unangetastet im Baum). Tests: test_gate_hook.py
  292 grün / 1 skip, test_common.py 6 grün (uv + pytest). Hinweis: laufende Sessions lesen settings.json beim
  Start — der Merge-Push läuft deshalb ggf. aus einer frischen Session.
- **Merge auf main (Bot):** Merge-fähig verifiziert (bbc3d05d vs origin/main 65889a6e: Diff exakt die 4 Paketdateien,
  +107/−6). Erster Push-Versuch 14:52Z vom Gate ordnungsgemäß verweigert — Budget-Fix greift (Chain lief, glm-Slot
  407s über der Untergrenze), aber kein Modell hat geurteilt: glm/grok-4.6 am leeren Grok-Balance-Konto,
  opus48/fable an der Claude-Org-Sperre, astra gate-seitig ohne Urteil. Retry, sobald ein Reviewer wieder
  urteilsfähig ist (klarster Unblocker: Grok-Balance auffüllen — lässt glm und grok-4.6 gleichzeitig wieder zu).
  Danach Remote-Branch, lokaler Branch, Worktree löschen.
- **Merge auf main (Website):** steht noch aus (R1 B läuft), danach Deploy beider Dienste und Live-Prüfung.

- Review-Wache abgeschlossen (14:21Z): beide R1-Listen liegen in REVIEW.md (Paket B: M1; Paket A:
  Doc-Kommentar handlers.rs:1759-1761). Reviewer 6a44ab2b/6aa8b223 gesettelt, tote opus48-Nachfolger
  4f14e725/dbcc4f2d gesettelt, Review-Scheduler gelöscht, Haupt-Thread 7581f506 berichtet. Rückfragen
  weiterhin nummeriert im jeweiligen Thread beantworten, nie `--model` an send. Arbeiter-Threads
  daf759ab/4085e71d sind fertig — nicht mehr wecken.
- Beide R1-Threads haben ihren einmaligen Neuanwurf verbraucht. Erneuter Tod = kein weiterer Neuanwurf,
  sondern Vermerk hier und Meldung an die Hauptsession.
- Merge: gate_hook.py liegt in /home/nathanael/Documents/.claude/gpt-workers/ (nicht in
  ~/Documents/tools — daher der Irrtum des A-Workers). Haupt-Checkout Deadlock-Bots ist fremd-dirty
  (u. a. rust/crates/dl-broker/src/handlers.rs) und origin/main 74 Commits voraus (65889a6e) — Merge
  nicht im Haupt-Checkout fahren.
- Merge über Merge-Gate, dann Deploy (dl-bot Release + Website-Backend) und Live-Prüfung.
