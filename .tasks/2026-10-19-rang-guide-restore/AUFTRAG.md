# Rang-Guide: Restore-Skip und Reparatur der überschriebenen V2-Nachricht

## Ziel

`#deadlock-rang` zeigt wieder den markierten Components-V2-Guide. Steam-Panel-Restore in dl-bot schreibt diesen Kanal nicht mehr. Ein Apply repariert eine gespeicherte ID, deren Live-Nachricht keine V2-Marker mehr hat, statt no-op.

## Ursache

Zwei Schreiber auf derselben Discord-Nachricht: Steam-Bot-Refresh und `spawn_panel_restore`. Die Live-Nachricht 1522584813511643156 hatte nach dem PATCH V2-Flagge plus Steam-Buttons ohne Marker 31001-31026. Alter Cleanup ließ sie liegen, Payload-Hash-no-op hätte sie stehen lassen.

## Änderung

- Panel-Restore überspringt Kanal 1398021105339334666.
- Cleanup-Kandidaten: Bot-Autor plus `steam_link_panel:`-custom_id; klassisch mit Embeds, V2 ohne Marker.
- no_op nur wenn gespeicherte IDs live als V2 mit Marker existieren.
- Cleanup löscht keine IDs, die Apply gerade gepostet oder editiert hat.

## Abnahme

Tests `panel_restore_ueberspringt_rang_guide_kanal`, the legacy cleanup classification tests, `rang_guide_legacy_cleanup_preserves_repaired_messages`, and the existing `scan_contains_every_stored_id` coverage. The latter replaces the older branch's dedicated visibility helper. Live: `POST /serversync/rang-guide-apply confirm=true` restores the guide after the gated merge and deployment.

## Takeover 2026-11-25 (sweep R3)

Ported 6064c055 onto e65d3805 without reverting the shared history scanner, V2 writer or scan-gap handling. Format check for dl-bot and dl-bridges and git diff --check passed. Remote compilation started but its result is not accessible through the connector; the initial command included `--exact` with an unqualified name, so even a successful build would not establish that the regression ran. Required next check: `cargo-slot test --locked -p dl-bot rang_guide_legacy_cleanup_preserves_repaired_messages` (no `--exact`), plus the Steam restore and legacy classification tests, clippy and the merge gate. No gate verdict, live KV proof, merge, deployment, or T3 settlement yet.
