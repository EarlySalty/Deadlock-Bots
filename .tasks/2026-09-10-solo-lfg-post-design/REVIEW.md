status: aktiv
Datum: 2026-09-10

# Review: Solo-LFG Gold-Karte

Reviewer: rust-reviewer (frisches Kontextfenster, read-only).
Urteil: keine Blocker, Merge-fähig nach Live-Proof.

## Findings und Verarbeitung

- S1 (Soll) Teilmengen-Anhänge ungetestet → erledigt: neuer Test
  `fehlt_nur_ein_asset_faellt_nur_diese_block_weg` (nur Banner, nur Logo).
- K1 (Kann) TOCTOU is_file/read → bewusst nicht geändert: Restfall
  vernachlässigbar, Code ist defensiver als das Panel-Vorbild.
- K2 (Kann) Fußzeile an MAX_POST_AGE koppeln → erledigt: Kommentar an
  POST_FOOTER_LINE.
- K3 (Kann) Doppel-Allokation im Rang-Suffix → erledigt: push_str statt format!.
- K4 (Kann) Router-Kommentar über dem falschen Import → erledigt: Zeile verschoben.
- K5 (Kann) Testschärfe → erledigt: Galerie-Position 0 wird geprüft,
  Mixed-Case-Junks (N/A, Egal) aufgenommen.
- H1 crate-weites fmt-Drift (mate_survey, scrim_record, dl-verbinder) →
  Vorbestand, vom Contract korrekt nicht angefasst. Die beiden Aufgabendateien
  sind fmt-sauber. PLAN.md-Vermerk ergänzt.

## Nach dem Review verifiziert

- rustfmt auf solo_watch.rs: sauber.
- clippy -p dl-voice --all-targets -- -D warnings: exit 0.
- cargo test -p dl-voice --lib solo_watch: 24 passed, 0 failed.

## Live-Gang

- Release-Build dl-bot: fertig (`rust/target/release/dl-bot`, 2026-09-10 19:34).
- Service-Neustart: in dieser Sitzung durch die Guardialeiste blockiert
  (Produktiv-Mutation, Hard-Wait). Kommando liegt beim User:
  `systemctl --user restart deadlock-bot-rust.service`
- Live-Proof (Journal, erste echte Karte in #mitspieler-suche): offen, nach
  Neustart nachzuholen.
