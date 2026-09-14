# RED-Runde 2 (Gate-Kritiker FUND 2)

Kommando:

    export PATH="$HOME/.rustup/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin:$PATH"
    cargo test -p dl-moderation --features testing -- --include-ignored

Baseline: 7 Tests scheitern mit `CENTRAL_TEST_DSN muss gesetzt sein` (keine Testdatenbank
in der Sandbox) und bleiben rot; sie treffen den Diff nicht. Alle anderen gruen.

## Rote Tests VOR dem Fix (FUND 2)

Beide neu, treffen das armierte Cleanup-Fenster beim Erkennen statt beim Durchsetzen.

1. `moderation_system::tests::shadow_mode_does_not_arm_takeover_cleanup` ... FAILED
   panicked at crates/dl-moderation/src/moderation_system.rs:1889:9
   `assertion failed: !detector.cleanup_armed(700).await`
   Grund: Im Shadow-Modus (enforce=false) armiert `detect()` das Cleanup-Fenster schon
   beim Erkennen, obwohl nichts durchgesetzt wird.

2. `moderation_system::tests::failed_case_persist_does_not_arm_takeover_cleanup` ... FAILED
   panicked at crates/dl-moderation/src/moderation_system.rs:1943:9
   `assertion left == right failed` (port.deletes == 1 statt 0)
   Grund: Bei gescheitertem `insert_case` (keine Sanktion) armiert `detect()` trotzdem das
   Cleanup-Fenster; die dritte Wellennachricht wird spurlos als Restwelle geloescht.

Gesamt: `test result: FAILED. 57 passed; 9 failed` (7 davon CENTRAL_TEST_DSN, 2 die obigen).
