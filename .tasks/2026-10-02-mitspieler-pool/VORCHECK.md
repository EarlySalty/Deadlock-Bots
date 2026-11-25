# Vorcheck: mitspieler-pool

Lesefreundlich, ohne Edits, ohne Tests, ohne Builds. Ergebnis als Antwort im Thread, kein
Artefakt im Repo. Du bist der einzige Thread dafür. Keine Unter-Threads oder Unter-Agenten.
Codesuche zuerst über `graphify query` (Skill `code-suche`), grep nur zum Nachlesen.

## Vorhaben (Kontext)

Spielerprofil und Mitspieler-Pool für den Discord der Deutschen Deadlock Community:

1. Beim Discord-Onboarding führt der Deadlock-Brain-Bot ein kurzes Interview (Spielzeiten,
   Ranked/Casual, mit wem und wie vielen man spielt, Voice usw.) und speichert die
   Präferenzen in der DB je Discord-User-ID.
2. Steam-ID wird ohne Steam-Bot-Freundschaft verknüpft (z. B. Steam OpenID oder
   Discord-Connections). Mit der Steam-ID holen wir bei der Deadlock-API Matchhistorie,
   Rang, Stunden/Spiele und bauen daraus ein Spielmuster (wann gezockt).
3. Website mit Discord-Login: Pool durchsuchen, Profile mit Spielzeiten, "recently played
   with" aus der Matchhistorie, Knopf "Zusammen spielen".
4. "Zusammen spielen": Bot öffnet proaktiv einen temporären Kanal auf dem Server für die
   Beteiligten, 1 h Haltezeit, wird gelöscht, wenn niemand kommt. DMs dazu nur per Opt-in.
5. Nach der Session auf der Website Feedback per Checkboxen (keine Benachrichtigung).

## Fragen

Finde für jede Frage den Bestand mit `pfad:zeile`:

- A. Interview/Onboarding: Wo lebt der Deadlock-Brain-Bot bzw. die Brain-Anbindung im
  Discord (Concierge, Onboarding-Rollen Frischling/Server-Tour, `dl-knowledge`, Repo
  `~/repos/Deadlock-Brain`)? Gibt es bereits einen DM- oder Interview-Flow mit Speicherung
  von Nutzerpräferenzen? Welche Tabelle(n)?
- B. Steam-Verknüpfung: Welche Wege gibt es heute (`core.steam_links`, Steam OpenID,
  Discord-Connections im OAuth von `dl-web`)? Läuft irgendeiner ohne Steam-Bot?
- C. Deadlock-API: Wo gibt es schon Clients für Matchhistorie, Rang, Stunden
  (Deadlock-Bots, Deadlock-Brain, Deadlock-Steam-Bot, Deadlock-Twitch-Bot `!rank`)?
  Wird Matchhistorie schon irgendwo gespeichert? `user_activity_patterns`: wer schreibt sie?
- D. Bestehende Mitspieler-Bausteine: `dl-voice/src/lfg_panel.rs`, `lfg_watch.rs`,
  `mate_survey.rs`, `solo_watch.rs`, `dl-activity/src/player_finder.rs`,
  `lfg_freetext`: was ist live (Flags in `scripts/run_dl_bot_service.sh`), welche Tabellen,
  was davon lässt sich für Pool/Matching/Feedback wiederverwenden?
- E. Temporäre Kanäle: Gibt es schon Code, der Voice/Text-Kanäle für Nutzer anlegt und
  nach Leerlauf löscht (Router-Lanes, `RouterLfgLaneSpawner`)?
- F. Website + Discord-Login: Wo würde eine Seite `/mitspieler` leben (Repo `Website`,
  `dl-dashboard`, `dl-web` Port 8766)? Wie läuft der Discord-Login heute, wo liegt die
  Session?
- G. Zentrale DB: Wo liegen Migrationen (`dl-central-db/migrations`), letzte Nummer?

## Antwortformat

- Fundstellen: je eine Zeile `<pfad>:<zeile>` plus in eigenen Worten, was dort steht,
  gruppiert nach A bis G
- Betroffene Repos: Liste
- Geschätzte Zahl der Stellen: Zahl
- Risiko: Prod-DB, Auth, Ban-Risiko, Datenschutz oder "keins erkennbar"
- Empfehlung je Frage in einem Satz: wiederverwenden, umbauen oder neu

Nur melden, was gelesen wurde. Eine Stelle ohne Fund ist eine Vermutung, keine Fundstelle.
Danach `python3 ~/Documents/tools/t3-thread.py settle --selbst`.
