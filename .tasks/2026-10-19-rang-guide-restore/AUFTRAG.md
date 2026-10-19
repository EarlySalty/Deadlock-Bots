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

Tests `panel_restore_ueberspringt_rang_guide_kanal`, `rang_guide_legacy_cleanup_loescht_nie_fremde_autoren`, `rang_guide_stored_ids_are_live_v2_rejects_clobbered_kv`. Live: `POST /serversync/rang-guide-apply confirm=true` stellt den Guide wieder her.
