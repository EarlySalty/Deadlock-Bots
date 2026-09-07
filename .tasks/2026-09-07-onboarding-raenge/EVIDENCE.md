# Evidence: Discord-Onboarding aktuelle Ränge

status: aktiv
datum: 2026-09-07
contract: CONTRACT.md

Repo-Aufklärung vor dem ersten Edit. Jede Zeile ist eine Fundstelle `pfad:zeile`,
keine Vermutung. Der Hook (R11) gibt Quellcode-Edits erst frei, wenn hier
mindestens 3 Fundstellen stehen. Drei ist die Untergrenze, nicht das Ziel.

## Analoge Implementierungen (wie löst das Repo so etwas schon?)

- `rust/bin/dl-bot/src/serversync.rs:6099` — `find_rank_prompt` erkennt den
  Rang-Prompt an 12 Optionen, von denen mindestens 10 auf `(unverifiziert)`-Rollen
  zeigen, und klont ihn 1:1.
- `rust/bin/dl-bot/src/serversync.rs:5827` — nach dem Fund läuft nur
  `sanitize_carried_over_channel_ids`, Titel und Emojis bleiben live.
- `rust/crates/steam-flows/src/rank.rs:87` (Steam-Bot) — `rank_name()` ist die
  aktuelle Namensliste (Obscurus…Eternus) mit Kommentar zum Rename 2026-07-30;
  Rollen-IDs bleiben am Tier-Index.
- `rust/crates/dl-voice/src/lfg_panel.rs:312` — Rang-Emojis sind hart kodierte
  Guild-Emoji-IDs plus Name, dasselbe Muster für Onboarding-Overlay.

## Bestehende Abstraktionen (werden wiederverwendet, nicht nachgebaut)

- `NativeOnboardingPrompt` / `NativeOnboardingOption`
  (`serversync.rs:383`, `serversync.rs:398`) — Live-IDs, `emoji` als JSON-Objekt
  `id`/`name`/`animated`.
- `native_onboarding_put_payload` (`serversync.rs:5858`) — wandelt GET-Emoji-
  Objekte in PUT-Felder `emoji_id`/`emoji_name`/`emoji_animated`.
- `option_points_to_unverified_role` (`serversync.rs:6145`) — Erkennung über
  Rollennamen-Suffix `(unverifiziert)`.
- `tempvoice::logic::rank_index` (`tempvoice/logic.rs:41`) — einzige Index-
  Quelle für LFG-Select-Values.

## Relevante Tests (laufen vorher, laufen nachher)

- `rust/bin/dl-bot/src/serversync.rs:10042` —
  `onboarding_builder_baut_drei_prompts_und_uebernimmt_rank_prompt_unveraendert`
  — muss Overlay und Extra-Prompt-Erhalt prüfen statt 1:1-Kopie.
- `rust/bin/dl-bot/src/serversync.rs:10081` —
  `onboarding_payload_laesst_neue_ids_weg_und_nutzt_name_resolved_ids` — Rang-
  Prompt-ID und Option-IDs bleiben.
- `rust/crates/dl-voice/src/tempvoice/logic.rs:221` — `rank_scores_wie_python`
  (Emissary-Index verschiebt sich, Arch-Alias bleibt auf Tier 7).
- `rust/crates/dl-voice/src/lfg_panel.rs:4528` —
  `lfg_select_flow_archon_bis_phantom_slots_3_erstellt_ranked_post_mit_auto_tags`
  — Select-Value/Emoji folgen den neuen Namen, Alias `archon` bleibt parsebar.

## Öffentliche Schnittstellen und Verträge (dürfen nicht brechen)

- Discord PUT `/guilds/{id}/onboarding` (`serversync.rs:880`) — bestehende
  Prompt-/Option-IDs Pflicht, sonst legt Discord neue Objekte an.
- Unverifizierte Rang-Rollen-IDs (Steam-Bot `UNVERIFIED_RANK_ROLE_IDS`,
  `steam-flows/src/rank.rs:144`) — Onboarding-`role_ids` bleiben diese IDs.
- LFG-Select-Custom-IDs und Draft-Speicher: Values dürfen sich ändern, alte
  Values müssen über Aliasse weiter einen Index > 0 liefern.

## Änderungsfläche (welche Dateien voraussichtlich angefasst werden)

- `rust/bin/dl-bot/src/serversync.rs` — Overlay + Extra-Prompts + Tests
- `rust/crates/dl-voice/src/lfg_panel.rs` — Select-Optionen
- `rust/crates/dl-voice/src/tempvoice/logic.rs` — RANK_ORDER + Aliasse
- `rust/crates/dl-voice/src/rank.rs` — Anzeigenamen
- `rust/bin/dl-bot/src/modglue.rs` — Rollen-Namenslisten
- `rust/crates/dl-activity/src/lfg.rs` — RANK_NAMES
- `rust/crates/dl-stats/src/ranks.rs` — RANK_ORDER/COLORS
- `rust/crates/dl-community/src/scrim_signup.rs` — RANK_ROLE_IDS-Namen

## Offene Architekturfrage

- keine
