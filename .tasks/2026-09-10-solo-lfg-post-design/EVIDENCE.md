status: aktiv
Datum: 2026-09-10

# EVIDENCE: Solo-LFG Auto-Post Design

1. `rust/crates/dl-voice/src/solo_watch.rs:875` — `lfg_post`, jetziger Builder.
2. `rust/crates/dl-voice/src/solo_watch.rs:775` — `mode_and_emoji`, Modus und Emoji.
3. `rust/crates/dl-voice/src/solo_watch.rs:30` — `MAX_POST_AGE`, 3-Stunden-Grenze.
4. `rust/crates/dl-voice/src/solo_watch.rs:667` — Aufrufort `post_lfg`, Port Zeile 109.
5. `rust/crates/dl-voice/src/lfg_panel.rs:962` — V2-Körper mit Galerie und
   `body["attachments"]`, `lfg_media_gallery` Zeile 995.
6. `rust/crates/dl-voice/src/lfg_panel.rs:75` — `LFG_EMOJI_SEARCH` (dl_lfg_sucht, ID live).
7. `rust/crates/dl-voice/src/lfg_panel.rs:410` — `LfgPanelAttachment`, `relative_path` skip.
8. `rust/crates/dl-voice/src/glue.rs:2514` — `post_rich` mit Multipart.
9. `rust/crates/dl-voice/src/glue.rs:442` — `lfg_panel_files` liest Banner vom Repo-Root.
10. `rust/crates/dl-voice/src/glue.rs:475` — `send_router_message_payload`.
11. `rust/crates/dl-voice/src/router.rs:526` — Banner optional via `fs::read().ok()`.
12. `assets/welcome-banners/divider-mitspieler-finden.png`, `logo-badge.png` — vorhanden.
13. `rust/crates/dl-voice/src/solo_watch.rs:1587` — Test, der den Postaufbau festhält.
