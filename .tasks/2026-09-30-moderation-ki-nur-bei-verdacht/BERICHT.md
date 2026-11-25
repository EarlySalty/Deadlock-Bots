status: abgeschlossen
Datum: 2026-09-30

# Bericht: Moderations-KI nur noch bei Heuristik-Verdacht

## Stand

- Branch `fix/moderation-ki-nur-bei-verdacht`, Basis `de34acc80dc3c1723397021593576ca94b20558d`
- Code-Commit `24dde7d4` (`fix(moderation): run content AI only on behavior signals`)
- Kein Merge, kein Deploy.

Diff-Stat gegen origin/main (Code-Commit):

```
 rust/bin/dl-bot/src/main.rs                        |   1 -
 rust/crates/dl-moderation/src/content_analyzer.rs  | 653 +--------------------
 rust/crates/dl-moderation/src/content_verifier.rs  | 162 +----
 rust/crates/dl-moderation/src/lib.rs               |   6 +-
 rust/crates/dl-moderation/src/moderation_system.rs | 370 +++++++-----
 .../crates/dl-moderation/src/moderation_verdict.rs | 144 -----
 6 files changed, 247 insertions(+), 1089 deletions(-)
```

## Umsetzung

1. `handle_message` kehrt ohne Verhaltenssignal direkt zurück. Die Inhalts-KI läuft nur noch über `evaluate_behavior_trigger` als Richter. Die Regel für fremde Einladungen in `scan_channel_ids` steht unverändert davor.
2. Entfernt: `high_confidence_scam_reason_conflict`, `high_confidence_scam_verification_conflict`, `explicit_scam_reason`, beide Konsistenz-Wiederholungen samt Prompts (`ANALYZER_CONSISTENCY_SYSTEM_PROMPT`, `CONSISTENCY_SYSTEM_PROMPT`, `analyzer_consistency_prompt`, `build_behavior_consistency_prompt`, `resolve_consistency`, `VerificationEvaluation`), das Feld `unresolved_consistency` und der Proposal-Zweig in `handle_message`. Entschieden wird nur noch über das strukturierte Urteil und die unveränderte `ActionPolicy`.
3. Toter Code entfernt: `ContentModerationPipeline::evaluate` und `evaluate_with_analysis`, `ContentVerifier::verify`, `VERIFIER_SYSTEM_PROMPT`, `build_verifier_prompt`, `ModerationInput::text_only` sowie der Pipeline-Parameter `analyze_flag_threshold` (dazu der Aufruf `env_f64_default("MOD_ANALYZE_FLAG_THRESHOLD", 0.5)` in `dl-bot`).
4. `cargo fmt -p dl-moderation` hat zusätzlich drei vorbestehende Fixture-Zeilen (`message_created_at`) in `lib.rs` und `moderation_system.rs` umgebrochen. Ohne das war der geforderte fmt-Check schon auf der Basis rot. Nur diese Crate, kein repo-weites fmt.

Nicht entfernt, weil noch Aufrufer bestehen: der Konfigschlüssel `runtime.moderation.analyze_flag_threshold` in `dl-core` (`runtime_config.rs`, Validierung, `settings_catalog.rs` und `tests/catalog_editor.rs`). Die Sektion nutzt `deny_unknown_fields`; ein Entfernen würde vorhandene Konfigdateien mit diesem Schlüssel beim Laden brechen. Der Schlüssel wirkt seit diesem Commit nicht mehr. Aufräumen wäre ein eigener kleiner Auftrag mit Blick auf die Live-Konfiguration.

## Tests

Entfernt oder umgestellt, weil sie das entfernte Verhalten prüften:

- `content_analyzer`: acht Pipeline- und Konsistenztests entfernt, neu `behavior_trigger_keeps_structured_verdict_without_consistency_retry` (je ein Analyse- und ein Verifier-Aufruf, keine Wiederholung).
- `moderation_verdict`: beide Konflikttests entfernt. `content_verifier`: Test des entfernten Prompts entfernt.
- `moderation_system`:
  - Neu `scan_channel_text_without_behavior_signal_skips_ai_and_case` und `scan_channel_image_without_behavior_signal_skips_ai_and_case`: null KI-Aufrufe, kein Fall, keine Discord-Aktion, obwohl die Fake-KI einen Scam bestätigen würde.
  - `takeover_unconfirmed_scam_verifier_is_not_rechecked_or_enforced` und `takeover_negated_scam_hint_in_reason_creates_no_review_card` ersetzen die beiden Konsistenz-Takeover-Tests: Ergebnis ist `ignored`, keine Karte, keine Sanktion.
  - Die Persist-Fehler-, Shadow-, Ignored- und Parse-Error-Tests laufen jetzt über Takeover- bzw. Burst-Signale statt über den Inhalts-Scan. Die Persist-Fehler-Tests prüfen zusätzlich, dass der KI-Richter tatsächlich lief.
  - `weak_hero_player_trash_talk_does_not_create_review_case`, `explicit_enforce_mode_executes_auto_action` und `content_scan_stays_limited_to_scan_channels` entfallen; ihr Pfad existiert nicht mehr, Durchsetzung ist über die Takeover-Tests abgedeckt.
- Unverändert grün: Takeover mit KI-Bestätigung bannt oder setzt Timeout und löscht die Welle (`takeover_signal_needs_real_ai_confirmation_before_auto_action`, `takeover_wave_deletes_all_messages_with_one_case_and_one_timeout`), fremde Einladungen werden gelöscht (`foreign_invite_in_scan_channel_is_deleted_noticed_and_logged`).

## Prüfkommandos

Im Verzeichnis `rust`, jeweils mit `PATH=/home/nathanael/.cargo/bin:$PATH`, Ausgabe in Datei, Exit-Code direkt geprüft:

- `cargo fmt -p dl-moderation -- --check`: exit 0
- `cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`: exit 0
- `SQLX_OFFLINE=true cargo check -p dl-bot`: exit 0
- `cargo test -p dl-moderation --features testing -- --include-ignored`: exit 0, 72 passed, 0 failed, 0 ignored, 0 filtered; 0 Doctests
- `git diff --check`: sauber

Die DB-Tests liefen gegen eine Wegwerf-TimescaleDB `timescale/timescaledb:2.17.2-pg16` (512 MB, 2 CPUs, zufälliger Loopback-Port, Datenbank `dlmod_test`) mit `CENTRAL_TEST_DSN`, `DEADLOCK_CENTRAL_DSN`, `TURNIER_TEST_DB_CONFIRM=throwaway-only` und `SQLX_OFFLINE=true` nur für diesen Lauf. Der erste Versuch mit Datenbankname `deadlock` wurde vom Test-Harness als Produktionsname verweigert (7 DB-Tests rot, kein Codefehler); danach mit `dlmod_test` grün. Container nach dem Lauf entfernt. Keine Community-Daten an externe Modelle, keine Sanktion an echten Mitgliedern.

## Selbstreview

`python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review --repo /home/nathanael/.worktrees/dl-moderation-ki-nur-bei-verdacht --base origin/main --head HEAD --timeout 900` auf `24dde7d4`:

`ALLOW: No merge-blocking defect established by the supplied diff.`

Ein NIT: `scan_channel_text_without_behavior_signal_skips_ai_and_case` läuft ohne Verhaltensdetektor. Nicht nachgezogen, damit der geprüfte Stand dem Review entspricht. Der Pfad „Detektor aktiv, kein Signal“ ist durch `scan_channel_image_without_behavior_signal_skips_ai_and_case` abgedeckt.

TESTNACHWEIS[TW-1]: 72 passed, 0 ignored | Baseline: nicht erhoben, kein Altfehlerurteil
