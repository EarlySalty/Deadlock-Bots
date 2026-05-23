# Stats und Privacy

## Worum geht es?
Der Server sammelt sichtbare Aktivitaets- und Nutzungsdaten, damit du Statistiken, Leaderboards und persoenliche Auswertungen bekommst. Gleichzeitig gibt es klare Opt-out- und Loeschfunktionen, wenn du diese Speicherung nicht willst.

## Wie nutze ich das?
Fur oeffentliche Aktivitaetsdaten gibt es eine Web-Ansicht unter dem Aktivitaetsbereich der Website. Dort siehst du serverweite Heatmaps, Rang-Verteilungen, Lane-Tendenzen, Voice-Historien und oeffentliche Leaderboards. Wenn du dich zusaetzlich per Discord anmeldest, kannst du auch deine persoenlichen Daten sehen, etwa deine eigene Historie oder wiederkehrende Mitspieler.

Im Discord selbst gibt es ausserdem Stats-Kommandos. Sichtbar sind vor allem persoenliche Aktivitaets- oder Chat-Ansichten wie `!myactivity`, `!tleaderboard` und `!messagestats`. Zusaetzlich kann der Server merken, wann du typischerweise aktiv bist und mit wem du oft zusammen spielst, damit Auswertungen und Empfehlungen sinnvoller werden.

Fur Datenschutz und Kontrolle gibt es zwei Ebenen. Mit `/datenschutz` kannst du zuerst einen Datenauszug herunterladen und danach deine gespeicherten Daten loeschen lassen. Das setzt zugleich ein globales Opt-out, sodass neue Speicherung blockiert wird, bis du sie mit `/datenschutz-optin` wieder aktivierst.

Daneben existiert das Retention-System. Wenn du fruher regelmaessig im Voice aktiv warst und dann lange wegbleibst, kann der Bot dir eine freundliche "Wir vermissen dich"-DM schicken. Wenn du diese DMs nicht willst, nutze `/retention-optout`. Mit `/retention-optin` kannst du sie spaeter wieder erlauben. In solchen DMs gibt es auch einen Feedback-Button, falls du rueckmelden willst, warum du weniger aktiv bist.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Die oeffentliche Stats-Seite liefert aggregierte Daten aus Voice-, Text- und Aktivitaetslogs. Fuer persoenliche Web-Daten brauchst du eine Discord-Anmeldung, damit nur du deinen eigenen Verlauf siehst. Das Privacy-System kann deine Daten tabellenuebergreifend exportieren oder loeschen und stoppt bei Opt-out auch kuenftige Erfassung fuer betroffene Features.

## Grenzen & häufige Fragen
- Oeffentliche Leaderboards koennen Anzeigenamen zeigen. Persoenliche Detaildaten sind getrennt und nur nach Login sichtbar.
- `/datenschutz` loescht nicht nur Leaderboard-Daten, sondern auch viele zusammenhaengende Tracking-Eintraege aus anderen Features.
- `retention-optout` betrifft nur die "Wir vermissen dich"-Nachrichten, nicht automatisch jede andere Datenspeicherung. Dafuer ist `/datenschutz` zustandig.
- Wenn du global opt-out bist, koennen einige Komfortfunktionen weniger gut funktionieren, etwa persoenliche Analysen oder Aktivitaetsmuster.
- Retention-DMs sind selten und an Aktivitaetsregeln gebunden. Nicht jeder inaktive User bekommt automatisch sofort eine Nachricht.
- Join-Quellen, Website-Zugange und aehnliche Herkunftsdaten koennen ebenfalls analytisch erfasst werden, solange kein Opt-out gesetzt ist.

## Für Devs (knapp)
- Cogs: `cogs/public_stats_cog.py`, `cogs/privacy_controls.py`, `cogs/privacy_core.py`, `cogs/user_activity_analyzer.py`, `cogs/user_retention.py`
- Abhangigkeiten: Public-Stats-Webserver, Discord-OAuth fuer persoenliche Ansichten, Voice-/Text-Tracker, Privacy-Checks in mehreren Cogs
- Wichtige DB-Tabellen: `user_privacy`, `voice_session_log`, `voice_stats`, `message_activity`, `user_activity_patterns`, `user_co_players`, `member_events`, `user_retention_tracking`, `user_retention_messages`, `server_faq_logs`, `kv_store`
