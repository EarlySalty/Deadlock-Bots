# Tierlist und Builds

## Worum geht es?
Der Server hat eine oeffentliche Deadlock-Tierlist mit Hero-Einstufungen, Build-Empfehlungen und Verlauf pro Patch. Aus Usersicht ist das die zentrale Stelle, um schnell zu sehen, welche Heroes aktuell stark sind, welche Builds zum Hero hinterlegt wurden und wie sich die Meta veraendert.

## Wie nutze ich das?
Oeffne die Tierlist auf der Website und waehle dort den passenden Datenbereich aus, zum Beispiel alle Ranks oder hoehere Brackets wie `Phantom+` oder `Eternus`. In der Tierlist siehst du fuer jeden Hero seine aktuelle Einstufung, Winrate, Match-Zahl und eine kurze Beschreibung. Wenn du tiefer reingehst, findest du darunter die fuer diesen Hero hinterlegten Builds.

Builds kannst du als Spieler vor allem lesen und vergleichen. Die Reihenfolge ist nicht zufaellig: Builds werden zusammen mit Vote-Zahlen angezeigt und dadurch im Frontend sinnvoll sortiert. Wenn dir ein Build hilft, kannst du ihn positiv bewerten; wenn er aus deiner Sicht schlecht oder veraltet ist, kannst du ihn runtervoten. So entsteht ueber die Zeit eine brauchbare Community-Sortierung, ohne dass du selbst irgendetwas publishen musst.

Wenn du beobachten willst, ob ein neuer oder geaenderter Build schon angekommen ist, schaust du einfach spaeter noch einmal auf denselben Hero. Neue oder aktualisierte Build-Daten tauchen nach dem Backend-Sync auf der oeffentlichen Seite auf. Fuer manche Heroes koennen dort ausserdem Streamer-Hinweise auftauchen. Es gibt also eine Twitch-Verbindung, die Details dazu kommen aber separat in den Twitch-Dokus.
## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Die Tierlist zieht regelmaessig externe Hero-Stats, bildet daraus pro Bucket Snapshots und ordnet Heroes anhand konfigurierter Winrate-Schwellen in Tiers ein. Hinterlegte Builds und Streamer-Verknuepfungen werden zu jedem Hero mit ausgeliefert; Build-Votes werden getrennt gespeichert. Die eigentliche Build-Auslieferung auf die Zielplattform passiert asynchron im Hintergrund, deshalb koennen neue Builds oder Updates mit Verzoegerung sichtbar werden.

## Grenzen & häufige Fragen
- Die Tierlist ist datengetrieben und patchabhaengig. Nach einem frischen Patch koennen sich Einstufungen deutlich verschieben.
- Build-Votes sind ein Signal, aber kein Garant dafuer, dass ein Build fuer jeden Rang oder jeden Spielstil optimal ist.
- Neue Builds erscheinen nicht immer sofort. Der Hintergrund-Worker verarbeitet sie in Intervallen und kann bei externen Problemen warten.
- Twitch-Links bei Heroes sind nur ein Hinweis auf passende Streamer oder Creator. Die komplette Twitch-Feature-Doku kommt separat.

## Für Devs (knapp)
- Cogs: `cogs/tierlist_public_cog.py`, `cogs/build_publisher.py`
- Abhangigkeiten: `service/tierlist_public.py`, Deadlock-API, Steam-Bridge-Taskqueue
- Wichtige DB-Tabellen: `tierlist_settings`, `tierlist_snapshots`, `tierlist_snapshot_heroes`, `tierlist_build_votes`, `tierlist_streamers`, `tierlist_hero_meta`, `deadlock_hero_builds`, `hero_build_clones`, `steam_tasks`
