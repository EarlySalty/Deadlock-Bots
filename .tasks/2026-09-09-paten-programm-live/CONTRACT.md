# Contract: Paten-Programm live (Rolle, Kanal, Onboarding-Anschluss, Paten-Wissen)

status: aktiv
datum: 2026-09-09
klasse: mittel
repo: Deadlock-Bots

Dieser Contract ist der Maßstab für Implementierung und Merge-Kritiker. Nach dem
Anlegen ist er unveränderlich: der Hook lässt nur noch die `status:`-Zeile und
Anhänge unter `## Amendments` zu. Wer ein REQ oder INV ändern will, schreibt ein
Amendment mit Begründung; Produkt-, API- oder Datenänderungen entscheidet der User.

## Ziel

Wer im Discord-Onboarding "Ich bin ganz neu, nehmt mich an die Hand" wählt, bekommt sofort einen echten Menschen als Paten angeboten; Paten finden sich über eine Bewerbung im bestehenden Team-Panel, wissen aus einem Leitfaden und vom Bot, was sie tun, und keine Patenanfrage bleibt still liegen.

## Ausgangslage (Stand 2026-09-09, Belege in EVIDENCE.md)

Rolle "Pate" (1524047896297738311, 4 Mitglieder) und Kanal 🤝paten-zentrale (1524083665838276860, nur Paten, null Nachrichten) existieren. Anfrage, Übernehmen per Button, privater Paten-Kanal in Kategorie "Neue Spieler", Last-Limit 3, Datenschutz-Locks und 33 Tests sind gebaut. Das Angebot erreicht Neulinge nie: proaktive DMs sind aus, und selbst mit proaktiv käme das Paten-Angebot erst als T2-Nachricht zwei Tage nach T0 und nur bei völliger Stille. DB: 158 Profile, 26 T0, 0 T2, 0 Paten-Angebote, 0 Patenschaften. Es gibt keinen Bewerbungsweg für Paten, keine Eskalation bei liegen gebliebenen Anfragen, keinen Leitfaden, und der 🧭willkommen-Hub nennt Paten nicht.

## Anforderungen (user-sichtbares Verhalten)

- REQ-1: Schließt ein Mitglied das Discord-Onboarding mit der Frischling-Wahl ab (Rolle "Frischling" 1522384961481609236), enthält seine T0-Begrüßungs-DM direkt das Paten-Angebot mit den Buttons "Ja, gern" und "Nee, ich komm klar"; `pate_offered` wird gesetzt und das Journey-Ereignis `pate_offered` geschrieben. Für alle anderen Onboarding-Wahlen bleibt T0 wie bisher und das Paten-Angebot kommt weiterhin per T2.
- REQ-2: Proaktive Concierge-DMs sind im Live-Startskript eingeschaltet (`DL_CONCIERGE_PROACTIVE=1`). Bestehende Grenzen gelten unverändert: "stopp" setzt den Opt-out, höchstens drei ungefragte Kontakte je Person, Zustellunsicherheit wird wie bisher persistiert.
- REQ-3: Klickt der Neuling "Ja, gern", entsteht wie bisher die Karte in 🤝paten-zentrale mit Ping der Paten-Rolle und Button "Übernehmen". Neu wird jede Anfrage persistiert (Anlage, Übernahme, Eskalation, Abschluss). Bleibt eine Anfrage 2 Stunden ohne Übernahme, aktualisiert der Bot die Karte einmal mit dem Hinweis "seit 2 h offen" und pingt einmal den Owner (662995601738170389) in derselben Nachricht. Bleibt sie 24 Stunden ohne Übernahme, bekommt der Neuling genau eine ehrliche DM (gerade ist kein Pate frei, mit den drei Kanälen, in denen er trotzdem sofort Hilfe bekommt) und die Karte wird als "unbesetzt geschlossen" markiert; der Übernehmen-Button bleibt danach ohne Wirkung mit klarer Rückmeldung. Jede Stufe feuert je Anfrage höchstens einmal, auch über Bot-Neustarts hinweg.
- REQ-4: Im Panel 🤝teil-vom-team-werden (1544026617733710006) gibt es den neuen Bereich "Pate" mit eigenem Formular (Text in `assets/team_application_texts.toml`: was ein Pate tut, wie viel Zeit es braucht, Frage nach Erfahrung und Verfügbarkeit). Nimmt ein Mod die Bewerbung an, vergibt der Bot die Rolle "Pate" automatisch und schickt der Person eine DM mit Willkommen und Link auf den Paten-Leitfaden. Für alle anderen Bewerbungsarten ändert sich nichts.
- REQ-5: In 🤝paten-zentrale steht eine vom Bot gepflegte, angepinnte Leitfaden-Nachricht (Components V2, Brand-Look, Texte in einer TOML-Datei unter `assets/`): was ein Pate tut und nicht tut, wie Übernehmen und der private Paten-Kanal laufen, Limit 3, die zehn wichtigsten Server-Fakten mit Kanal-Links (Router, Neue-Spieler-Lane, Coaching, Rang-Verknüpfung, Regelwerk, Support, Mitspieler-Suche, Custom Games, Scrims, frag-die-community), Eskalation an Mods, Datenschutzregeln (keine Weitergabe von DM-Inhalten). Ändert sich die TOML, aktualisiert der Bot die Nachricht beim Start statt eine zweite zu posten.
- REQ-6: In 🤝paten-zentrale und in jedem privaten Paten-Kanal (`pate-<user_id>` in Kategorie 1465839366634209361) beantwortet der Bot Wissensfragen über den bestehenden HTTP-Wissenspfad, wenn er per Mention angesprochen wird; ohne Mention bleibt er still. Persönliche Aktionsbuttons (Patenwunsch, Steckbrief) gibt es dort weiterhin nicht.
- REQ-7: Der 🧭willkommen-Hub (1522460852114948166) bekommt einen Abschnitt "Paten" (Text in `assets/welcome_texts.toml`): was ein Pate ist, dass die Onboarding-Wahl "nehmt mich an die Hand" einen Paten bringt, und dass man dem Bot jederzeit per DM "ich hätte gern einen Paten" schreiben kann. Die Beschreibung der Onboarding-Option 🌱 lautet danach "Ich bin ganz neu und will's lernen, ein Pate aus der Community zeigt mir alles".
- REQ-8: Die Inventarzeile des Bots beim Start (aiglue) nennt den Paten-Stand: Anzahl Mitglieder mit Paten-Rolle, offene Anfragen, aktive Patenschaften, Leitfaden gepostet ja/nein.
- REQ-9: Für jede neue Verhaltensregel aus REQ-1, REQ-3 und REQ-4 gibt es einen Test, der vor der Änderung rot ist (Frischling-T0 mit Paten-Buttons; 2-h- und 24-h-Stufe je genau einmal, auch nach Neustart; Rollenvergabe bei Annahme einer Paten-Bewerbung).

## Invarianten (darf sich nicht ändern)

- INV-1: Der reaktive Weg (Neuling schreibt dem Bot ausdrücklich, dass er einen Paten möchte) bleibt unverändert und funktioniert unabhängig von `proactive`.
- INV-2: Opt-out ("stopp", `opted_out`, Privacy-Locks) gewinnt immer: kein Angebot, keine Eskalations-DM, keine Karte für Personen mit Opt-out; der Datenschutz-Scrub der Journey-Metadaten bleibt.
- INV-3: `claim_pate` bleibt der einzige Weg zu einer Patenschaft: nur in 🤝paten-zentrale, nur mit Paten-Rolle, Last-Limit 3, Unique-Index "ein aktiver Pate je Neuling".
- INV-4: Kein neuer `*_ENABLED`-Schalter. Alles Neue hängt an `DL_CONCIERGE_ENABLED` und den bestehenden Paten-IDs.
- INV-5: Bestehende Tests (33 Paten-Tests in concierge.rs, `pate_journey_metadata_scrub`, Team-Bewerbungs-Tests) werden nicht gelöscht oder abgeschwächt.
- INV-6: Discord-Kanäle und -Rollen nur per ID, Emojis in Bot-UI nur dl_*-Brand-Emojis, keine Unicode-Emojis in neuen Bot-Komponenten.
- INV-7: Team-Bewerbungen der anderen Bereiche (Moderation, Coach, Caster, Turnier, Coder, Eigene Idee) verhalten sich exakt wie bisher.
- INV-8: Nutzersichtbare Texte in Nutzersprache mit echten Umlauten, ohne Gedankenstriche und ohne internes Vokabular.

## Nicht-Ziele

- Kein Paten-Cockpit mit KI-Antwortvorschlägen und Time-to-first-response-Messung (Konzept 2026-07-02, Abschnitt Paten-Cockpit), das ist ein späterer Ausbau.
- Keine Änderung an T2/T7-Kadenz für Nicht-Frischlinge.
- Keine automatische Zuteilung eines Paten ohne Klick eines Menschen.
- Keine Community-Ankündigung aus diesem Contract heraus; der Aufruf "Wer hat Bock, Paten zu werden" wird nach dem Deploy über den Skill community-ankuendigung entworfen und nur mit Go des Nutzers gepostet.
- Kein Umbau des Wissensdienstes dl-knowledge; die neue Doku-Seite `public/discord-server/paten.html` entsteht im Repo Deadlock-Docs und geht über `tools/deploy_corpus.sh` live (eigener Schritt, Klasse niedrig).

## Erlaubter Änderungsbereich

- rust/crates/dl-community/src/concierge.rs
- rust/crates/dl-community/src/team_applications.rs
- rust/crates/dl-community/src/lib.rs
- rust/crates/dl-central-db/migrations/
- rust/crates/dl-central-db/src/
- rust/crates/dl-central-db/tests/
- rust/crates/dl-activity/src/journey.rs
- rust/bin/dl-bot/src/aiglue.rs
- rust/bin/dl-bot/src/journeyglue.rs
- rust/bin/dl-bot/src/serversync.rs
- rust/bin/dl-bot/src/main.rs
- assets/team_application_texts.toml
- assets/welcome_texts.toml
- assets/paten_leitfaden.toml
- scripts/run_dl_bot_service.sh
- rust/.sqlx/
- .tasks/2026-09-09-paten-programm-live/

## Verbotene Änderungen

- Schema oder Semantik von `bot.concierge_patenschaften` und `bot.concierge_profiles` (nur additive Migrationen für die neue Anfrage-Tabelle).
- Löschen oder Umbenennen bestehender Discord-Rollen, -Kanäle oder -Kategorien; keine neuen Kanäle außer den bestehenden `pate-<user_id>`-Kanälen.
- Andere Concierge-Texte (T0 für Nicht-Frischlinge, Tour, T2, T7, Steckbrief) und die Steckbrief-Mod-Ping-Logik.
- Lint-, CI- und Hook-Konfiguration.
- Alles außerhalb des erlaubten Bereichs.

## Offene Produktfragen

- keine

## Amendments

- 2026-09-09, Erlaubter Änderungsbereich, alt: ohne Port-Implementierungen -> neu: zusätzlich `rust/bin/dl-bot/src/modglue.rs` (neue Portmethoden `add_role` für TeamApplicationPort und `edit_channel_v2` für ConciergePort, nur additive Methoden) und `rust/bin/dl-bot/src/serversync/welcome_publish.rs` (neue Sektion "paten"), Grund: dl-community erreicht Discord nur über die Port-Impls in dl-bot und der Willkommen-Hub definiert seine Sektionen fest im Code (PLAN.md Scope-Warnung); rein technisch, kein Verhalten außerhalb der REQ, Freigabe-Datei durch den User nötig, entschieden von Orchestrator
- 2026-09-09, Erlaubter Änderungsbereich, alt: modglue.rs nur `add_role` und `edit_channel_v2` -> neu: zusätzlich die additive Portmethode `pin_message` für ConciergePort (REQ-5 verlangt eine angepinnte Leitfaden-Nachricht), Grund: Review-Hinweis 7, rein technisch, entschieden von Orchestrator
