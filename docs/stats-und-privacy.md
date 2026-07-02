# Stats und Privacy

## Worum geht es?
Der Server sammelt sichtbare Aktivitäts- und Nutzungsdaten, damit du Statistiken, Leaderboards und persönliche Auswertungen bekommst. Gleichzeitig gibt es klare Opt-out- und Löschfunktionen, wenn du diese Speicherung nicht willst.

## Wie nutze ich das?
Für öffentliche Aktivitätsdaten gibt es eine Web-Ansicht unter dem Aktivitätsbereich der Website. Dort siehst du serverweite Heatmaps, Rang-Verteilungen, Lane-Tendenzen, Voice-Historien und öffentliche Leaderboards. Wenn du dich zusätzlich per Discord anmeldest, kannst du auch deine persönlichen Daten sehen, etwa deine eigene Historie oder wiederkehrende Mitspieler.

Im Discord selbst gibt es diese Stats-Kommandos:
- `!useranalysis` (auch `!ua`, `!analyze`) — persönliche Aktivitätsanalyse
- `!myactivity` — deine eigene Aktivität
- `!tleaderboard` (auch `!tlb`, `!texttop`) — Text-Leaderboard
- `!messagestats` (auch `!msgstats`) — Nachrichten-Statistiken
- `!memberevents` (auch `!mevents`) — Mitglieder-Ereignisse
- `!checkping` — Ping-Check
- Für Voice gibt es zusätzlich `!vstats` und `!vleaderboard` (siehe Voice-Doku)
- Team-Befehle mit besonderen Rechten: `!smartping` (Nachrichten-Verwaltung), `!serverstats` (Admin)

Zusätzlich kann der Server merken, wann du typischerweise aktiv bist und mit wem du oft zusammen spielst, damit Auswertungen und Empfehlungen sinnvoller werden.

Für Datenschutz und Kontrolle gibt es zwei Ebenen. `/datenschutz` zeigt dir zwei unabhängige Buttons: `Daten herunterladen` (Datenauszug als Datei, personenbezogene Fremd-IDs darin geschwärzt) und `Endgültig löschen` — du kannst also exportieren, löschen oder beides. Das Löschen setzt zugleich ein globales Opt-out, sodass neue Speicherung blockiert wird, bis du sie mit `/datenschutz-optin` wieder aktivierst.

Daneben existiert das Retention-System. Wenn du früher regelmäßig im Voice aktiv warst und dann lange wegbleibst, kann der Bot dir eine freundliche „Wir vermissen dich"-DM schicken. Wenn du diese DMs nicht willst, nutze `/retention-optout`. Mit `/retention-optin` kannst du sie später wieder erlauben. In solchen DMs gibt es auch einen Feedback-Button, falls du rückmelden willst, warum du weniger aktiv bist.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Die öffentliche Stats-Seite liefert aggregierte Daten aus Voice-, Text- und Aktivitätslogs (Heatmap, Rang-Verteilung, Lane-Präferenzen, Timeline, Bestzeiten, Voice-Historie, öffentliche Leaderboards). Für persönliche Web-Daten brauchst du eine Discord-Anmeldung, damit nur du deinen eigenen Verlauf siehst. Das Privacy-System exportiert oder löscht deine Daten tabellenübergreifend — inklusive Turnierdaten — und stoppt bei Opt-out auch künftige Erfassung für betroffene Features.

## Grenzen & häufige Fragen
- Öffentliche Leaderboards können Anzeigenamen zeigen. Persönliche Detaildaten sind getrennt und nur nach Login sichtbar.
- `/datenschutz` löscht nicht nur Leaderboard-Daten, sondern auch viele zusammenhängende Tracking-Einträge aus anderen Features, bis hin zu Turnier-Anmeldedaten.
- Der Export ist keine Pflicht vor dem Löschen — beide Buttons sind unabhängig.
- `/retention-optout` betrifft nur die „Wir vermissen dich"-Nachrichten, nicht automatisch jede andere Datenspeicherung. Dafür ist `/datenschutz` zuständig.
- Wenn du global opt-out bist, können einige Komfortfunktionen weniger gut funktionieren, etwa persönliche Analysen oder Aktivitätsmuster.
- Retention-DMs sind selten und an Aktivitätsregeln gebunden (früher regelmäßig aktiv, dann mindestens 2 Wochen weg; maximal eine solche DM pro Person, frühestens 30 Tage nach der letzten). Nicht jeder inaktive User bekommt automatisch eine Nachricht.
- Join-Quellen, Website-Zugänge und ähnliche Herkunftsdaten können ebenfalls analytisch erfasst werden, solange kein Opt-out gesetzt ist.

## Für Devs (knapp)
- Rust live: Public-Stats-Service `dl-stats` (Web-API inkl. `/api/public/me/*` + Discord-OAuth), Prefix-Stats in `dl-activity/src/stats_cmd.rs` + `text_stats.rs`, Privacy in `dl-community/src/privacy_ui.rs` + `privacy.rs` (Export mit Schwärzung, Erasure inkl. Turnier-Scope, Opt-out-Tombstone `core.user_privacy`), Retention in `dl-community/src/retention.rs`
- Voice- und Steam-Systeme respektieren das Privacy-Opt-out beim Schreiben (`dl-voice/src/tracker.rs`, Steam-Leave-Cleanup)
