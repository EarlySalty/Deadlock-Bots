# Voice-Features

## Worum geht es?
Diese Doku fasst die Voice-Funktionen zusammen, die du direkt im Server merkst: TempVoice-Lanes, den Router-Kanal, automatische Lane-Verteilung, Rang- und Statuslogik, Voice-Statistiken und die Steam-Link-Erinnerung per DM. Ziel ist, dass du schneller in passende Runden kommst und deine Lane ohne Moderation selbst steuern kannst.

## Wie nutze ich das?
- **TempVoice:** Betritt einen `(+)`-Staging-Channel für Chill, Ranked oder Street Brawl. Deine Lane wird automatisch erstellt (Chill 8, Ranked 6, Street Brawl 4 Plätze) und verschwindet wieder, wenn alle raus sind. **Alle Lanes landen in einer gemeinsamen Kategorie** — oben Ranked (nach Rang aufsteigend sortiert), darunter Casual, unten Street Brawl. Der Name verrät den Inhalt: *Ranked Phantom 3*, *Chill Lane 2 · Oracle*, *Street Brawl 1*. Für Ranked ist keine Verifizierung mehr nötig.
- **Router-Kanal:** Alternativ gibt es einen Router-Voice — dort landest du kurz und wählst per Panel `Casual`, `Ranked`, `Street Brawl` oder `Auto-Join`. Bei Auto-Join steckt dich der Bot bevorzugt in eine Lane mit Leuten, mit denen du öfter spielst, und mit 1–5 Mitgliedern.
- **Lane steuern:** Öffne <#1439564934592729161>. Dort findest du je nach Lane-Typ: `🇩🇪 DE`, `🇪🇺 EU`, `Owner übernehmen`, `🎚️ Limit setzen`, `🎯 Mein Rang`, `👢 Kick`, `🚫 Ban`, `♻️ Unban`, `👻 Lurker`, `🛡️ Tag-Filter`, `Duo Call`, `Trio Call`, `Normale Lane` — und bei Nicht-Ranked-Lanes zusätzlich `Umbenennen` und `Modus wechseln`.
- **Ranked-Lanes:** Der Owner kann per `🔓 Rang-Gate` die Lane exklusiv für verifizierte Ränge in einem Fenster machen (Dialog mit Mindestrang + Toleranz, Standard: eigener Rang ±1,5 Ränge). Erst dann erscheint ein 🔒 für alle außerhalb des Fensters; niemand wird entfernt, das Gate wirkt nur auf neue Joins, nochmal drücken schaltet es aus. Zusätzlich kannst du `💾 Preset speichern` und `🗂 Preset laden`.
- **Lane-Routing:** Neue oder niedrige Ränge landen bevorzugt in `🆕Neue Spieler Lane`. Dort erweitert der Bot die Zahl der Lanes automatisch, sobald eine Lane voll wird. `🗨️Off Topic Voice` erweitert sich ebenfalls automatisch, wenn genug Leute drin sind.
- **Street Brawl:** Nutze die Street-Brawl-Staging-Lane, wenn du genau diesen Modus willst. Diese Lanes sind fest auf 4 Plätze gedeckelt.
- **Voice-Status & Rank Voice Manager:** Verknüpfe Steam und sitze in einer Ranked/Comp-Lane. Der Bot ergänzt dann automatisch den Kanalstatus wie Lobby oder Match-Minuten und richtet Rangfenster bzw. Kanalnamen an der relevanten Gruppe aus.
- **Voice-Aktivität:** Mit `!vstats` siehst du deine Voice-Zeit und Punkte, mit `!vleaderboard`, `!vlb` oder `!voicetop` das Server-Ranking.
- **Voice-Feedback:** Nach längeren Sessions kann dich der Bot per DM um kurzes Feedback bitten (Formular mit Freitext) — das geht intern an das Team, nicht nach draußen.
- **Steam-Link-Nudge:** Wenn du ohne Steam-Link an einem späteren Tag wieder länger im Voice bist, bekommst du einmalig eine DM mit Link-Button oder dem Hinweis auf `/account_verknüpfen`.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
TempVoice speichert Owner, Presets, Bans, Lurker-Status und Tag-Filter serverseitig und räumt leere Lanes zyklisch wieder auf. Serverweite Voice-Trennungen werden als befristete Zutrittssperren gesetzt, haben Vorrang vor TempVoice-Owner-Rechten und stellen bestehende Kanalrechte anschließend wieder her. Der Router erstellt Lanes bedarfsweise (Casual über den Router mit 6 Plätzen) und nutzt Co-Player-Daten fürs Smart-Routing. Die Rang- und Statuslogik liest Rollen, Steam-Verknüpfungen und Presence-Daten, bildet daraus die relevante Lobby-/Match-Gruppe und aktualisiert Kanalnamen oder Berechtigungen asynchron. Das Voice-Tracking schreibt Sessions, Gesamtzeit und Punkte in die zentrale Datenbank und verschickt Feedback- oder Link-DMs nur nach klaren Regeln.

## Grenzen & häufige Fragen
- TempVoice-Buttons wirken nur, wenn du gerade selbst in einer passenden Lane sitzt. Kick, Ban und Tag-Filter sind Owner-/Mod-Funktionen.
- Eine serverweite Voice-Trennung verhindert neue Beitritte in den bereits belegten Kanal; sie trennt oder verschiebt niemanden nachträglich.
- `Owner übernehmen` ist für den Fall gedacht, dass der ursprüngliche Owner weg ist — dann dürfen bevorzugt die aktivsten Mitglieder der Lane übernehmen, nach 20 Minuten jeder in der Lane. Die Rang-Basis einer Lane bleibt intern trotzdem stabil.
- Das Rang-Gate gibt es nur in Ranked-Lanes und nur für den Owner. Aktiv wirkt es als echte Zutritts-Beschränkung über verifizierte Rang-Rollen; wer beim Aktivieren schon drin ist, bleibt drin. Ranked-Lanes erstellen und joinen geht ohne Verifizierung, solange kein Gate aktiv ist.
- Beim Tag-Filter werden Mindestalter und `Ragebaiter-Free` durchgesetzt; die Ton-Präferenz (`Banter-OK`) ist eine Info und sperrt niemanden aus.
- Street-Brawl-Lanes ignorieren Rang-Caps und Mindest-Rang, haben aber immer maximal 4 Slots.
- Neue-Spieler-Routing greift nur für niedrige Ränge bzw. passende unverifizierte Einsteiger-Rollen.
- Voice-Status, Rangnamen und feinere Ranked-Zuordnung funktionieren am besten mit verknüpftem Steam-Account. Ohne Link oder mit veralteter Presence fällt der Bot auf einfachere Heuristiken zurück.
- Voice-Punkte zählen nur für aktive Sessions. Standardmäßig müssen genug aktive Leute im Call sein; reines Stumm-Rumsitzen zählt nicht dauerhaft mit.
- Voice-Feedback- oder Steam-Link-DMs können ausbleiben, wenn deine DMs geschlossen sind, du ein Privacy-Opt-out gesetzt hast oder eine ausgenommene Rolle trägst.

## Für Devs (knapp)
- Rust live: `dl-voice/src/tempvoice/` (engine + interface: Staging, Buttons, Presets, Tag-Filter, Owner-Claim-Regeln), `voice_pair_guard.rs` (serverweite, bidirektionale Zutrittstrennung), `router.rs` (Router-VC, Smart-Routing), `adaptive.rs` (New-Player-/Off-Topic-/Sortier-Automatik), `rank.rs` (Anker, Subrang-Fenster ±9, Overwrites), `status.rs` (Kanalstatus), `stats.rs` (`!vstats`, Leaderboards, Admin-Debug), `feedback.rs` (Feedback-DM + Modal), `nudge.rs` (Steam-Link-DM), `tracker.rs` (Sessions, respektiert Privacy-Opt-out)
- Admin: Panel-Post per `!tvpanel`/`!tempvoicepanel`/`!tvinterface`
