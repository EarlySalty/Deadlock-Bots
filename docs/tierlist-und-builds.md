# Tierlist und Builds

## Worum geht es?
Der Server hat eine öffentliche Deadlock-Tierlist mit Hero-Einstufungen, Build-Empfehlungen und Verlauf pro Patch. Aus Usersicht ist das die zentrale Stelle, um schnell zu sehen, welche Heroes aktuell stark sind, welche Builds zum Hero hinterlegt wurden und wie sich die Meta verändert.

## Wie nutze ich das?
Öffne die Tierlist auf der Website (unter `/builds/`) und wähle dort den passenden Datenbereich aus: alle Ranks (`all`) oder die höheren Brackets `Phantom+` und `Eternus`. In der Tierlist siehst du für jeden Hero seine aktuelle Einstufung, Winrate, Match-Zahl und eine kurze Beschreibung. Wenn du tiefer reingehst, findest du darunter die für diesen Hero hinterlegten Builds; dazu gibt es eine Verlaufs-Ansicht pro Patch.

Builds kannst du als Spieler vor allem lesen, vergleichen und bewerten. Die Grundreihenfolge legt das Team fest; deine Up- und Downvotes entscheiden bei gleichrangigen Builds über die Reihenfolge und geben dem Team ein ehrliches Qualitäts-Signal. Für manche Heroes können außerdem Streamer-Hinweise auftauchen — es gibt also eine Twitch-Verbindung, die Details dazu kommen separat in den Twitch-Dokus.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Die Tierlist zieht regelmäßig externe Hero-Stats (standardmäßig alle 8 Stunden), bildet daraus pro Bucket Snapshots und ordnet Heroes anhand konfigurierter Winrate-Schwellen in Tiers ein — Heroes mit zu wenigen Matches (Standard: unter 500) fallen raus. Hinterlegte Builds und Streamer-Verknüpfungen werden zu jedem Hero mit ausgeliefert; Build-Votes werden getrennt gespeichert (mit kurzem Spam-Schutz von 5 Sekunden pro Absender).

## Grenzen & häufige Fragen
- Die Tierlist ist datengetrieben und patchabhängig. Nach einem frischen Patch können sich Einstufungen deutlich verschieben.
- Build-Votes sind ein Signal, aber kein Garant dafür, dass ein Build für jeden Rang oder jeden Spielstil optimal ist — und sie stehen in der Sortierung hinter der redaktionellen Reihenfolge.
- Ein automatischer Abgleich der Builds in den Spiel-Client (In-Game-Katalog) läuft aktuell nicht — Build-Pflege passiert redaktionell über das Dashboard.
- Twitch-Links bei Heroes sind nur ein Hinweis auf passende Streamer oder Creator. Die komplette Twitch-Feature-Doku kommt separat.

## Für Devs (knapp)
- Rust live: `dl-tierlist` (Public-API: Heroes/Tierlist/History/Votes + Admin-Routes inkl. Refresh; Settings: 8h-Intervall, min_matches 500, Buckets `all`/`phantom_plus`/`eternus`; Sortierung `(sort_order, votes)` in `data.rs`)
- Der alte Steam-Build-Sync (`MAINTAIN_BUILD_CATALOG`/Build-Publisher-Worker) ist nicht nach Rust portiert; Upsert/Delete halten nur die DB konsistent (`dl-dashboard/src/deadlock.rs`)
- Wichtige DB-Tabellen: Schema `tierlist.*` (Settings, Snapshots, Votes, Streamer, Hero-Meta); `steam.steam_tasks` gehört zum Steam-Schema
