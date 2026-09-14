# EVIDENCE: Neue-Spieler-Lanes in Chill

## Wachstums-Automatik
- rust/crates/dl-voice/src/adaptive.rs:21 `NP_TARGET_CATEGORY_ID = 1465839366634209361`
- rust/crates/dl-voice/src/adaptive.rs:22 `NP_ANCHOR_CHANNEL_ID = 1470126503252721845`
- rust/crates/dl-voice/src/adaptive.rs:23 `NP_LANE_BASE_NAME = "🆕Neue Spieler Lane"`
- rust/crates/dl-voice/src/adaptive.rs:43 `CHILL_CATEGORY_ID = 1289721245281292290`
- rust/crates/dl-voice/src/adaptive.rs:410 `maybe_route_new_player` scannt `NP_TARGET_CATEGORY_ID`
- rust/crates/dl-voice/src/adaptive.rs:442 `sync_new_player` ruft `sync_managed(..., NP_TARGET_CATEGORY_ID, NP_ANCHOR_CHANNEL_ID, NP_LANE_BASE_NAME, 6, None)`
- rust/crates/dl-voice/src/adaptive.rs:527 `create_voice_channel(guild, category_id, anchor_id, name)` legt gewachsene Lane in `category_id` an
- rust/crates/dl-voice/src/adaptive.rs:550 `sort_tempvoice_category` SKIP_IDS = [CASUAL_STAGING, PERMANENT_CHILL, PINNED_CHILL_END], plus DUO-Anker; NP-Anker fehlt

## LFG-Label über Kategorie (der Bruch bei reinem Umzug)
- rust/bin/dl-bot/src/modglue.rs:2671 `LFG_CATEGORIES`: Chill `1289721245281292290` = "Casual", NP `1465839366634209361` = "New Player"
- rust/bin/dl-bot/src/modglue.rs:2846 Label aus `channel.parent_id` gegen `LFG_CATEGORIES`
- rust/bin/dl-bot/src/modglue.rs:2854 `"New Player" => LaneLabel::NewPlayer, _ => Casual`
- rust/bin/dl-bot/src/modglue.rs:2922 `if label == NewPlayer { limit = limit.min(6) }`
- rust/crates/dl-activity/src/lfg.rs:583 Anfänger-Routing bevorzugt `LaneLabel::NewPlayer`

## Temp-Voice fasst NP-Lanes nicht an
- rust/crates/dl-voice/src/tempvoice/engine.rs:252 `staging_channels = {casual 1501089974093873232, street_brawl 1357422958544420944, comp 1412804671432818890}`
- NP-Anker `1470126503252721845` ist kein Staging-Kanal, löst keinen Erstellen-Flow aus
- rust/crates/dl-voice/src/tempvoice/store.rs:29 Temp-Voice trackt Lanes per DB (`owner_id`, `channel_id`), verwaltet nur Eigene

## Rechte-Vererbung beim Nachlegen
- rust/crates/dl-voice/src/glue.rs:2836 `overwrites = anchor.permission_overwrites.clone()`
- rust/crates/dl-voice/src/glue.rs:2828 `parent_id = category_id` (die übergebene Kategorie)

## solo_watch überwacht Chill bereits
- rust/crates/dl-voice/src/status.rs:30 `TARGET_CATEGORY_IDS` enthält Chill `1289721245281292290`
- rust/crates/dl-voice/src/solo_watch.rs:752 `allowed_category` = TARGET_CATEGORY_IDS oder NP-Kategorie
- rust/crates/dl-voice/src/solo_watch.rs:775 `mode_and_emoji` labelt NP-Kategorie als "New Player" (kosmetisch, Nicht-Ziel)

## Anker anderswo per ID ausgeschlossen (bleibt korrekt nach Umzug)
- rust/crates/dl-voice/src/router.rs:51 `NON_LANE_CHANNEL_IDS` enthält `1470126503252721845`
- rust/crates/dl-voice/src/rank.rs:52 `EXCLUDED_CHANNEL_IDS` enthält `1470126503252721845` (rank läuft nur auf Ranked-Kategorie)

## NP-Kategorie hat weitere Nutzer (nicht anfassen)
- rust/crates/dl-community/src/concierge.rs:61 `DEFAULT_PATE_CATEGORY_ID = 1465839366634209361`
