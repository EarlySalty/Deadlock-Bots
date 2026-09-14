# RED: Regressionstests vor dem Fix

Befehl:
`rustup run 1.97.1-x86_64-unknown-linux-gnu cargo test -p dl-moderation --features testing -j 2 -- --include-ignored`

Ergebnis vor dem Fix: `test result: FAILED. 55 passed; 9 failed`.
Baseline ohne Fix: 55 passed / 7 failed (die 7 sind DB-abhaengige store::tests::* und
ragebait-Tests, Docker/Test-DB im Sandbox nicht verfuegbar, unabhaengig vom Fix).
Die zwei neuen Tests sind die zusaetzlichen roten:

## Test a: takeover_wave_deletes_all_messages_with_one_case_and_one_timeout
moderation_system.rs
```
assertion `left == right` failed
  left: 2
 right: 4
```
Vor dem Fix werden nur 2 der 4 Wellennachrichten geloescht (Nachrichten 3/4 brechen an
is_suppressed mit None ab). Erwartet nach Fix: 4 geloescht, 1 Case, 1 Timeout.

## Test b: takeover_mirrors_exactly_one_complete_message
moderation_system.rs
```
assertion `left == right` failed
  left: ["https://img/2000-0.png", "https://img/2000-1.png", "https://img/2000-2.png", "https://img/2000-3.png", "https://img/2001-0.png", "https://img/2001-1.png", "https://img/2001-2.png", "https://img/2001-3.png"]
 right: ["https://img/2001-0.png", "https://img/2001-1.png", "https://img/2001-2.png", "https://img/2001-3.png"]
```
Vor dem Fix wird ein 8er-Mix aus beiden Nachrichten gespiegelt. Erwartet nach Fix: genau
die 4 Bilder der ausloesenden Event-Nachricht (2001).
