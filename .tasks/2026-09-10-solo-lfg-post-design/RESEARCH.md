status: aktiv
Datum: 2026-09-10

# Research: Solo-LFG Auto-Post Design

## Befund

Der Post wird in `solo_watch.rs::lfg_post` gebaut: Gold-Container (type 17) mit
einem Textblock (Modus-Überschrift, Sitzzeile, `-#`-Detailzeile) und zwei Buttons.
Keine Bilder, keine Section, kein CTA-Text. Der Rang wird ungefiltert als
`· {rank}` angehängt, deshalb steht im Live-Post „Casual · n/a“.

## Evidence (Fundstellen)

1. `rust/crates/dl-voice/src/solo_watch.rs:875` bis `941` — `lfg_post`: aktueller
   Builder, Buttons `JOIN_CUSTOM_ID` + `LANE_LINK_LABEL`, `allowed_mentions` stumm.
2. `rust/crates/dl-voice/src/solo_watch.rs:775` bis `780` — `mode_and_emoji`:
   Kategorie zu Modus-Name und dl_-Emoji (Casual/Ranked/Street Brawl/New Player).
3. `rust/crates/dl-voice/src/solo_watch.rs:30` — `MAX_POST_AGE = 3 Stunden`
   (Fußzeilen-Behauptung dagegen prüfbar).
4. `rust/crates/dl-voice/src/lfg_panel.rs:962` bis `1000` —
   `lfg_panel_body_for_attachments`: V2-Körper mit Galerie
   (`attachment://{filename}`) und `body["attachments"] = json!(attachments)`;
   `relative_path` ist `#[serde(skip)]` (lfg_panel.rs:413).
5. `rust/crates/dl-voice/src/lfg_panel.rs:75` — `LFG_EMOJI_SEARCH`
   (`dl_lfg_sucht`, `1522801046509064263`), produktiv im Einsatz.
6. `rust/crates/dl-voice/src/glue.rs:2514` bis `2538` — `post_rich`/`edit_rich`
   des LFG-Panels: Multipart über `lfg_panel_files` (glue.rs:442, liest Dateien am
   Repo-Root) und `send_router_message_payload` (glue.rs:475, payload_json + PNG-Parts).
7. `rust/crates/dl-voice/src/router.rs:526` bis `551` — `voice_guide_detail_reply`:
   Banner optional via `std::fs::read().ok()`, ohne Datei wird ohne Galerie gepostet
   (Muster für REQ-6).
8. `assets/welcome-banners/` — `divider-mitspieler-finden.png` und
   `logo-badge.png` existieren (43 KB / 41 KB).
9. `rust/crates/dl-voice/src/solo_watch.rs:667` — Aufrufort `port.post_lfg(LFG_CHANNEL_ID, body)`;
   Port-Trait Zeile 109; Mock in Tests Zeile 1241.
10. Tests `post_ist_gold_karte_mit_beitreten_knopf` (solo_watch.rs:1587) und Helfer
    `post_text`/`post_buttons` (Zeile 1562 bis 1581) müssen auf den neuen Aufbau
    umgestellt werden.

## Umgang mit „n/a“

Das Rangfeld ist Freitext aus dem Modal (Vorbelegung ist der verifizierte
Steam-Rang, sonst leer). „n/a“ ist Nutzereingabe. Der Builder filtert künftig
Platzelfüllungen (REQ-3) statt am Modal zu drehen; der DM-Vorschautext bleibt
unberührt (Nicht-Ziel).

## Verifikationsweg des Repos

`SQLX_OFFLINE=true rust/scripts/central_test_db.sh env SQLX_OFFLINE=true cargo test
-p dl-voice`, `cargo clippy -p dl-voice --all-targets -- -D warnings`,
`cargo fmt --all -- --check` (Stand aus WORKFLOW.md, dl-voice 201 Tests grün).
