# Research: Discord-Onboarding aktuelle Ränge

status: aktiv
datum: 2026-09-07
klasse: mittel

## Auftrag

Onboarding-Rangfrage und Rangbilder auf den Stand der Deadlock-API bringen.

## Beobachtungen (belegt, Datei:Zeile)

- Live-Onboarding (GET `/guilds/1289721245281292288/onboarding`, 2026-09-07):
  fünf Prompts. Rang-Prompt id `1420468904618102918`, 12 Optionen, Titel noch
  Initiate/Seeker/Alchemist/Arcanist/Ritualist/Emissary/Archon/Oracle/Phantom/
  Ascendant/Eternus plus „Neu im Game“. Emoji-IDs zeigen auf Guild-Emojis
  `initiate`…`archon` (alte Namen).
- Live-Rollen: Subrang- und Unverifiziert-Rollen heißen bereits Acolyte,
  Sentinel, Mystic, Ritualist, Emissary. Die Onboarding-Option „Alchemist“
  zeigt auf Rolle `1492960350755225730` = „Acolyte (unverifiziert)“.
- Deadlock-API `GET /v1/assets/ranks`: Tier 0 Obscurus, 1 Initiate, 2 Seeker,
  3 Acolyte, 4 Sentinel, 5 Mystic, 6 Ritualist, 7 Emissary, 8 Oracle, 9 Phantom,
  10 Ascendant, 11 Eternus. Bilder `rank01_lg.png` … `rank11_lg.png`.
- Steam-Bot `rank_name()` (`steam-flows/src/rank.rs:87`) hat denselben Stand
  seit dem Matchmaking-Update 2026-07-30. Rollen-IDs hängen am Tier-Index.
- Discord-Onboarding-Builder `build_welle2b_onboarding_config`
  (`serversync.rs:5767`) übernimmt den Rang-Prompt unverändert
  (`find_rank_prompt` `serversync.rs:6099`). Test
  `onboarding_builder_baut_drei_prompts_und_uebernimmt_rank_prompt_unveraendert`
  (`serversync.rs:10042`) zementiert das.
- Builder emittiert drei Prompts und würde die Live-Prompts 3 und 4
  (Starthilfe, Steam) beim Apply streichen.
- Guild-Emojis: 11 alte Rang-Emojis (`alchemist`, `arcanist`, `archon`, …),
  keine `acolyte`/`sentinel`/`mystic`. Discord kann das Bild eines Emojis nicht
  per PATCH ändern, nur Name; Bildwechsel = umbenennen, neu hochladen, alt löschen.
- `LFG_RANK_SELECT_OPTIONS` (`lfg_panel.rs:312`) nutzt dieselben Emoji-IDs.
  `rank_index` (`tempvoice/logic.rs:41`) speist LFG, TempVoice und Anzeigenamen.
- `dl-stats` `RANK_ORDER` (`ranks.rs:13`) filtert `deadlock_rank_name`; Steam-Bot
  schreibt bereits „Acolyte“. Alte Liste würde umbenannte Ränge aus der Stats
  werfen.
- „Neu im Game“ teilt die Initiate-unverifiziert-Rolle; Overlay darf nur Optionen
  mit Custom-Emoji-ID anfassen, sonst wird Unbekannt zu Initiate umbenannt.

## Hypothesen (unbelegt — nie als Fakt weiterreichen)

- Die verifizierten Major-Rang-Rollen (IDs `13314575…`) antworten auf
  GET `/guilds/{id}/roles` nicht; Subränge existieren. Für Onboarding irrelevant,
  weil die Frage unverifizierte Rollen vergibt.
- Discord erlaubt in dieser Guild fünf Onboarding-Prompts (Live hat 5); der
  Kommentar „hart maximal 4“ in `serversync.rs:6097` ist veraltet.

## Wahrscheinlich zu ändernde Dateien

- `rust/bin/dl-bot/src/serversync.rs`
- `rust/crates/dl-voice/src/{lfg_panel,tempvoice/logic,rank,adaptive}.rs`
- `rust/bin/dl-bot/src/modglue.rs`
- `rust/crates/dl-activity/src/lfg.rs`
- `rust/crates/dl-stats/src/ranks.rs`
- `rust/crates/dl-community/src/scrim_signup.rs`

## Risiken / Seiteneffekte

- Onboarding-PUT mit dem heutigen Builder ohne Extra-Prompt-Erhalt löscht
  Starthilfe und Steam-Frage. Overlay plus Extra-Erhalt ist Pflicht vor Apply.
- Emoji-IDs ändern sich beim Neu-Upload; LFG und Onboarding müssen dieselben
  neuen IDs tragen, sonst fehlen Bilder in der LFG-Auswahl.
- Ritualist/Emissary sind offizielle Namen mit verschobenem Index; Aliasse
  dürfen diese Wörter nicht auf den alten Index zurückbiegen.

## Offene Fragen

- keine
