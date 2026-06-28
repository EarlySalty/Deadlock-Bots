# Voice-Features

## Worum geht es?
Diese Doku fasst die Voice-Funktionen zusammen, die du direkt im Server merkst: TempVoice-Lanes, automatische Lane-Verteilung, Rang- und Statuslogik, Voice-Statistiken und die Steam-Link-Erinnerung per DM. Ziel ist, dass du schneller in passende Runden kommst und deine Lane ohne Moderation selbst steuern kannst. `Voice Reaction DM` ist dabei kein normales Community-Feature, sondern nur eine interne Benachrichtigung im Hintergrund.

## Wie nutze ich das?
- **TempVoice:** Betritt einen `(+)-` Staging-Channel in Chill, Ranked oder Street Brawl. Deine Lane wird automatisch erstellt und verschwindet wieder, wenn alle raus sind.
- **Lane steuern:** Öffne `#sprach-kanal-verwalten`. Dort findest du `🇩🇪 DE`, `🇪🇺 EU`, `👑 Owner Claim`, `🎚️ Limit setzen`, `🎯 Mein Rang`, `👢 Kick`, `🚫 Ban`, `♻️ Unban`, `👻 Lurker`, `🛡️ Tag-Filter`, `Duo Call`, `Trio Call` und `Normale Lane`.
- **Ranked-Lanes:** In Ranked setzt du den Mindest-Rang immer in zwei Schritten: erst `① Haupt-Rang`, dann `② Sub-Rang`. Zusätzlich kannst du `💾 Preset speichern` und `🗂 Preset laden`.
- **Lane-Routing:** Neue oder niedrige Ränge landen bevorzugt in `🆕Neue Spieler Lane`. Dort erweitert der Bot die Zahl der Lanes automatisch, sobald eine Lane voll wird. `🗨️Off Topic Voice` erweitert sich ebenfalls automatisch, wenn genug Leute drin sind.
- **Street Brawl:** Nutze die Street-Brawl-Staging-Lane, wenn du genau diesen Modus willst. Diese Lanes sind fest auf 4 Plätze gedeckelt.
- **Voice-Status & Rank Voice Manager:** Verknüpfe Steam und sitze in einer Ranked/Comp-Lane. Der Bot ergänzt dann automatisch den Kanalstatus wie Lobby oder Match-Minuten und richtet Rangfenster bzw. Kanalnamen an der relevanten Gruppe aus.
- **Voice-Aktivität:** Mit `!vstats` siehst du deine Voice-Zeit und Punkte, mit `!vleaderboard`, `!vlb` oder `!voicetop` das Server-Ranking.
- **Steam-Link-Nudge:** Wenn du ohne Steam-Link an einem spaeteren Tag wieder laenger im Voice bist, bekommst du einmalig eine DM mit Link-Button oder dem Hinweis auf `/account_verknüpfen`.
- **Voice Reaction DM:** Kein normaler Nutzer-Flow. Dafür musst du nichts tun.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
TempVoice speichert Owner, Presets, Bans, Lurker-Status und Tag-Filter serverseitig und räumt leere Lanes zyklisch wieder auf. Die Rang- und Status-Cogs lesen Rollen, Steam-Verknüpfungen und aktuelle Presence-Daten, bilden daraus die relevante Lobby-/Match-Gruppe und aktualisieren Kanalnamen oder Berechtigungen asynchron. Das Voice-Tracking schreibt Sessions, Gesamtzeit und Punkte in die zentrale Datenbank und verschickt optionale Feedback- oder Link-DMs nur nach klaren Regeln. `Voice Reaction DM` pollt separat einen externen Lead-Feed und ist für normale Community-Mitglieder nicht sichtbar.

## Grenzen & häufige Fragen
- TempVoice-Buttons wirken nur, wenn du gerade selbst in einer passenden Lane sitzt. Kick, Ban und Tag-Filter sind Owner-/Mod-Funktionen.
- `Owner Claim` ist für den Fall gedacht, dass der ursprüngliche Owner weg ist. Die operative Lane-Kontrolle kann wechseln, die Rang-Basis einer Lane bleibt intern trotzdem stabil.
- Mindest-Rang gibt es nur in Ranked/Comp. Du musst dafür verifiziert sein und kannst keinen höheren Rang setzen als deinen eigenen.
- Street-Brawl-Lanes ignorieren Rang-Caps und Mindest-Rang, haben aber immer maximal 4 Slots.
- Neue-Spieler-Routing greift nur für niedrige Ränge bzw. passende unverifizierte Einsteiger-Rollen. Wenn du sehr schnell erneut in einen Staging-Channel springst, kann wieder der normale TempVoice-Flow greifen.
- Voice-Status, Rangnamen und feinere Ranked-Zuordnung funktionieren am besten mit verknüpftem Steam-Account. Ohne Link oder mit veralteter Presence fällt der Bot auf einfachere Heuristiken zurück.
- Voice-Punkte zählen nur für aktive Sessions. Standardmäßig müssen genug aktive Leute im Call sein; reines Stumm-Rumsitzen zählt nicht dauerhaft mit.
- Voice-Feedback- oder Steam-Link-DMs können ausbleiben, wenn deine DMs geschlossen sind, du ein Privacy-Opt-out gesetzt hast oder eine ausgenommene Rolle trägst.
- `Voice Reaction DM` ist kein öffentliches Feature. Wenn du davon nie etwas siehst, ist das normal.

## Für Devs (knapp)
- `cogs/tempvoice/core.py`, `interface.py`, `util.py`, `lane_sorting.py`, `new_player_lanes.py`, `duo_lanes.py`: TempVoice, Routing, UI, Lurker, Presets, Auto-Lane-Erweiterung. Nutzt `RolePermissionVoiceManager` und optional `TagService`. Tabellen: `tempvoice_*`.
- `cogs/deadlock_voice_status.py`: Lobby-/Match-Suffixe pro Voice-Lane. Nutzt Steam-Link- und Presence-Daten. Tabellen: `steam_links`, `live_player_state`, `deadlock_voice_watch`.
- `cogs/rank_voice_manager.py`: Ranked-Anker, Score-Fenster, Rollen-Overwrites und Rangnamen. Tabellen: `voice_channel_settings`, `voice_channel_anchors`.
- `cogs/voice_activity_tracker.py`: Voice-Sessions, Punkte, Leaderboard und Voice-Feedback-DMs. Tabellen: `voice_stats`, `voice_session_log`, `voice_feedback_requests`, `voice_feedback_responses`, `kv_store`.
- `cogs/steam_link_voice_nudge.py`: einmalige Steam-Link-DM nach Voice-Nutzung. Tabellen: `steam_nudge_state`, `steam_links`, `kv_store`.
- `cogs/voice_reaction_dm.py`: interner DM-Bridge-Cog für externe Leads. Nutzt eine separate Outreach-Tabelle außerhalb der Community-DB.
