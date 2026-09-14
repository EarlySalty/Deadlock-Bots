status: aktiv
Datum: 2026-09-10

# Plan: Solo-LFG Auto-Post Design

Ziel steht in CONTRACT.md. Nach jedem Milestone Status hier eintragen.

## Milestone 1: Builder und Tests in solo_watch.rs

- `lfg_post` baut den neuen Aufbau: Galerie `divider-mitspieler-finden.png`, Section
  mit Logo-Thumbnail und Titel „Mitspieler gesucht“ (`LFG_EMOJI_SEARCH`), Moduszeile,
  Sitzzeile mit „Plätze frei“-Wortlaut, `-#`-Detailzeile, Divider, CTA-Absatz,
  `-#`-Fußzeile mit 3-Stunden-Grenze. Kein Gedankenstrich in neuen Texten.
- `clean_rank` filtert `n/a`, `na`, `-`, `–`, `?`, `egal` (REQ-3).
- `solo_post_attachments()` liest die zwei Dateien vom Repo-Root, fehlende fallen weg
  (REQ-6); leerer Anhangsliste bedeutet alter Textaufbau ohne Galerie/Section.
- Port-Signatur: `post_lfg(channel_id, body, attachments)`, Mock und Aufrufort angepasst.
- Tests: bestehenden Post-Test umstellen, neu: Anhang vorhanden, Rückfall ohne Assets,
  Rang-Filter, CTA und Fußzeile.
- Erwarteter Zwischenzustand: alle dl-voice-Tests grün.
- Validierung: `SQLX_OFFLINE=true rust/scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice` + `cargo clippy -p dl-voice --all-targets -- -D warnings` + `cargo fmt --all -- --check`.
- Stop-Regel: rote Tests oder Clippy vor Milestone 2 fixen.

## Milestone 2: Glue-Anbindung

- `post_lfg` im `RouterGlue`: Anhänge über `lfg_panel_files` +
  `send_router_message_payload` als Multipart senden, Rückgabe Message-ID wie
  `post_rich` des LFG-Panels.
- Erwarteter Zwischenzustand: Workspace-Build grün, gesamte dl-voice-Testsuite grün.
- Validierung: `SQLX_OFFLINE=true cargo build` (Workspace) + Milestone-1-Befehle.

## Milestone 3: Preview und Auslieferung

- Preview-Karte mit dem echten Builder-JSON nach #bot-logs (1374364800817303632)
  posten (Review-Bühne, Produktdefault), User schauen lassen.
- Danach: `cargo build --release`, `systemctl --user restart deadlock-bot-rust.service`,
  Journal-Check, Live-Proof (nächster echter Eintrag oder Bot-Logs-Preview aus dem
  laufenden Dienst).

## Status

- [x] Milestone 1 (2026-09-10): Builder, clean_rank, solo_post_attachments, Port-Signatur,
      Mock, Tests grün.
- [x] Milestone 2 (2026-09-10): Glue-Multipart über lfg_panel_files +
      send_router_message_payload; dl-voice 447 Tests grün, clippy -D warnings sauber.
- [x] Review (2026-09-10): rust-reviewer, keine Blocker; Findings S1/K2/K3/K4/K5
      abgearbeitet, danach 24 solo_watch-Tests + clippy erneut grün → REVIEW.md.
      crate-weites fmt-Drift ist Vorbestand (mate_survey, scrim_record, dl-verbinder),
      nicht Teil dieses Diffs; beide Aufgabendateien fmt-sauber.
- [x] Milestone 3a (2026-09-10): Release-Build dl-bot fertig.
- [ ] Milestone 3b: Service-Neustart (Guardialeiste blockiert; Kommando beim User)
      und Live-Proof (Journal + erste echte Karte in #mitspieler-suche).
      Vorschau-Bild der Karte fürs Auge: Bilder/solo-lfg-karte-preview.png
      (HTML-Nachbildung, nicht aus Discord gerendert).

## Verifikationsstand

- `SQLX_OFFLINE=true rust/scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test -p dl-voice` → 447 passed, 0 failed, 4 ignored.
- `SQLX_OFFLINE=true cargo clippy -p dl-voice --all-targets -- -D warnings` → exit 0.
- `rustfmt --edition 2021 --check` auf solo_watch.rs und glue.rs → sauber.
- `git diff --check` → sauber.
