## #108 — Rust-Neuaufbau: der FAQ-Chat und der Ticket-Auto-Helfer

**Ausgangslage:** Über das FAQ-Panel bekommt jeder auf Knopfdruck einen privaten Chat-Kanal, in dem ein Bot Fragen zum Server beantwortet — mit der Server-Dokumentation als Wissensbasis, strikten Sicherheitsregeln (keine internen Details, nichts erfinden) und Gesprächs-Gedächtnis für Rückfragen. Chats schließen nach 24 Stunden oder per Knopf. Zusätzlich liest der Ticket-Auto-Helfer die erste Nachricht in neuen Support-Tickets mit: Kann er das Anliegen aus der Doku klar lösen, antwortet er sofort — sonst schweigt er und überlässt das Ticket den Menschen.

**Geändert:** Komplett in Rust portiert: gleiche Knopf-Kennungen, gleiche System-Anweisungen wortgleich (inklusive der Invite- und Coaching-Sonderregeln), gleiche Tabellen, gleiches Gedächtnis-Fenster (letzte 10 Nachrichten), gleiche Schweige-Regel (KEIN_TREFFER-Protokoll). Die Antworten laufen über die bestehende MiniMax-Anbindung. Zwei ehrlich dokumentierte Annäherungen: Der Ticket-Helfer springt auf die erste Nachricht im Ticket-Kanal an statt auf dessen Erstellung (sichtbar gleiches Verhalten), und die optionale Patchnotes-Anreicherung der Antworten fehlt noch.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Prompt-Aufbau, Doku-Sammlung (alphabetisch, nur Markdown) und der komplette Session-Lebenszyklus (anlegen, Verlauf kappen, schließen, Ablauf) sind mit Tests abgesichert.

## #107 — Rust-Neuaufbau: die Clip-Einsendungen

**Ausgangslage:** Im Clip-Kanal steht ein Einsende-Interface: Button drücken, Verwendungserlaubnis bestätigen, dann Link, Credit und Kontext ins Formular — mit Link-Prüfung und 60-Sekunden-Bremse gegen Spam. Die Einsendungen sammeln sich in einem Wochenfenster (Sonntag 0 Uhr bis Samstag 23 Uhr deutscher Zeit); nach Ablauf bekommt der Clip-Kurator genau einmal eine Text-Datei mit allen Einsendungen der Woche per DM.

**Geändert:** Komplett in Rust portiert: gleiche Knopf-Kennungen (das bestehende Interface im Kanal funktioniert nach dem Umstieg weiter), gleiche Formular-Felder und Fehlertexte, gleiche Tabellen, gleiche Fenster-Berechnung inklusive Zeitzonen-Logik, gleiches Dump-Format. Das Interface aktualisiert sich weiter alle 5 Minuten (Fenster-Countdown im Embed), der Fenster-Wächter prüft alle 2 Minuten auf abgelaufene Fenster.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Fenster-Berechnung ist gegen konkrete Kalenderdaten getestet (inklusive „Sonntag selbst ist Fensterstart"), Dump-Format und URL-Prüfung ebenso, und ein Datenbank-Test spielt den kompletten Lebenszyklus durch: Fenster anlegen → Einsendung zuordnen → Dump markieren.

## #106 — Rust-Neuaufbau: die Exit-Umfrage

**Ausgangslage:** Wer den Server verlässt, bekommt eine kurze Abschieds-Umfrage per DM — mit Fragen, die zum Nutzer-Typ passen: Kurzbesucher (unter 2 Tagen, nie im Voice, kaum Nachrichten) werden anders gefragt als langjährig Aktive (ab 14 Tagen mit regelmäßigen Sessions, vielen Nachrichten oder über einer Stunde Voice) oder stille Mitglieder. Gebannte und Nutzer mit Datenschutz-Opt-out werden übersprungen, und niemand wird öfter als alle 30 Tage befragt.

**Geändert:** Komplett in Rust portiert: gleiche Typ-Einstufung, gleiche Grund-Auswahllisten und Nachfragen (wortgleich), gleiche Tabelle samt Einmal-Token für den Web-Fragebogen. Die Auswahl-Kennungen der Knöpfe sind unverändert — auch Umfrage-DMs, die noch vom alten Bot verschickt wurden, funktionieren nach dem Umstieg weiter, weil die Antwort über die Datenbank ihrer offenen Umfrage zugeordnet wird.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Verlassen → Sperren-Checks → Einstufung → DM mit Grund-Auswahl → Nachfrage-Formular → Antwort und Zeitpunkt landen in der Datenbank, eine Zusammenfassung im Log-Kanal. Die Einstufungs-Regeln und alle Texte sind mit Tests abgesichert (8 Klassifikations-Fälle).

## #105 — Rust-Neuaufbau: Beitritts-Protokoll und eine wichtige Umstiegs-Korrektur

**Ausgangslage:** Beim geplanten Teil-Umstieg wäre ein stilles Problem entstanden: Der alte Aktivitäts-Analyzer und sein neues Rust-Pendant hätten beide den Mitspieler-Zähler hochgezählt — alle Werte doppelt. Außerdem schreibt bisher nur der alte Analyzer die Beitritts-/Austritts-Ereignisse, auf denen die Join-Quellen-Auswertung und die Bindungs-Funktionen aufbauen.

**Geändert:** Die Umstiegs-Checkliste warnt jetzt ausdrücklich, dass der alte Analyzer beim Umstieg abgeschaltet werden muss, und der Rust-Bot schreibt die Beitritts-/Austritts-Ereignisse selbst weiter (gleiche Tabelle). Ehrlich dokumentierte Übergangs-Lücke: Die Herkunfts-Zuordnung neuer Beitritte (welcher Einladungs-Link) fehlt noch — solche Beitritte zählen als „Unbekannt", bis das Einladungs-Abgleich-Verfahren portiert ist. Die Checkliste wurde außerdem auf den aktuellen Stand gebracht: Alle drei ursprünglichen Umstiegs-Voraussetzungen sind gebaut, es gibt keine Blocker mehr.

**Wie es jetzt funktioniert:** Nach dem Umstieg gehen keine Beitritts-Daten verloren und nichts wird doppelt gezählt.

## #104 — Rust-Neuaufbau: die Website-Invites

**Ausgangslage:** Jede Website-Unterseite (Landing, Streamer, Mitspieler, Coaching, Helden, Guides) hat ihren eigenen permanenten Einladungs-Link in den Server — so lässt sich nachvollziehen, über welche Seite neue Mitglieder kommen. Der Bot prüft beim Start, ob die gespeicherten Codes noch existieren, und erstellt fehlende neu. Dazu gehört die Join-Quellen-Auswertung, die Beitritte nach Herkunft aufschlüsselt (Website-Seite, Vanity-Link, Twitch-Streamer, persönliche Einladungen …).

**Geändert:** Beides in Rust portiert: gleiche Speicher-Verträge (inklusive der Landing/Haupt-Spiegelung), gleicher Start-Lebenszyklus (tote Codes werden ersetzt, lebende bleiben unangetastet), und die komplette Herkunfts-Klassifikation samt Balken-Anzeige wortgleich. Die Auswertung liest dieselben Beitritts-Ereignisse, die der bestehende Python-Analyzer weiter schreibt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Acht Klassifikations-Fälle und der Speicher-Vertrag (inklusive Spiegelung) sind getestet. Der Start-Check wartet 20 Sekunden nach dem Hochfahren, damit die Discord-Verbindung steht.

## #103 — Rust-Neuaufbau: das LFG-Routing-Herz

**Ausgangslage:** Wenn die Gruppensuche eine Mitspieler-Anfrage erkennt, entscheidet eine Routing-Logik, wohin der Suchende gelotst wird: Street-Brawl- und Ranked-Absicht aus Schlüsselwörtern (Ranked zählt erst ab Emissary), Anfänger primär in die Neue-Spieler-Lanes mit Casual-Rückfall, Rang-Toleranzen je Lane-Typ (±2 Ranked, ±3 Casual), und Lanes mit bekannten Mitspielern gewinnen vor der vollsten passenden.

**Geändert:** Diese Entscheidungslogik ist vollständig in Rust portiert und mit sieben Referenz-Entscheidungen aus dem laufenden Python-Original abgesichert — jedes Szenario (Casual, Ranked, zu niedriger Rang für Ranked, Street Brawl, Anfänger-Rückfall, Mitspieler-Vorrang, alles voll) liefert exakt dieselbe Entscheidung. Der sichtbare Antwort-Teil (Embed mit Empfehlung und Spielervorschlägen im Ausgabe-Kanal) folgt als letzter Schritt auf dieser Basis.

**Wie es jetzt funktioniert:** Noch unverändert über den Python-Bot — die Rust-Seite hält jetzt aber die komplette Entscheidungskette (Erkennung → Bewertung → Routing) getestet vor.

## #102 — Rust-Neuaufbau: der Modus-Wechsel — das Lane-Panel ist komplett

**Ausgangslage:** Der letzte gesperrte Panel-Knopf: Lane-Besitzer können ihre bestehende Lane in einen anderen Modus umziehen (Ranked, Casual, Street Brawl, Off Topic) — die Lane wandert in die Ziel-Kategorie, der Datenbank-Eintrag zieht nach, und der Name passt sich an (Ranked übernimmt den Rang des Besitzers, sonst kehrt der gespeicherte Basisname zurück).

**Geändert:** In Rust angeschlossen mit Auswahlmenü, demselben Ranked-Gate (verifizierter Rang nötig, sonst Hinweis auf den Info-Kanal) und derselben Umzugs-Reihenfolge. Damit sind **alle** Funktionen des Lane-Panels im neuen System verfügbar.

**Wie es jetzt funktioniert:** Knopf → Modus wählen → Lane zieht um. Der Voice-Bereich ist damit funktional vollständig portiert.

## #101 — Rust-Neuaufbau: Anfänger-Lanes, Off-Topic-Duo und die Rang-Sortierung

**Ausgangslage:** Drei Spezial-Systeme rund um die Voice-Lanes fehlten noch im Rust-Port: Die Anfänger-Einsortierung (wer höchstens Arcanist ist — verifiziert oder mit Unverifiziert-Rolle — wird beim Betreten der Sammel-Kanäle in die Neue-Spieler-Kategorie umgeleitet, mit 4-Minuten-Rückkehr-Fenster in den normalen Ablauf), die selbst-wachsenden Lanes („Neue Spieler Lane" legt ab 6 Personen nach, „Off Topic Voice" ab 2 genau eine zweite; Leere werden abgebaut und Nummern rücken nach) und die Rang-Sortierung der Chill-Lanes (Lanes mit Rang-Namen werden nach Haupt- und Unterrang auf ihre Plätze sortiert).

**Geändert:** Alle drei in Rust portiert und an den zentralen Ereignis-Verteiler gehängt: Die Anfänger-Umleitung sitzt als Vorab-Haken im Join-to-create (genau wie das Original — greift kein Haken, läuft alles normal weiter), die Wachstums-Pläne beider Lane-Familien teilen sich eine Logik mit zwei Spielarten und sind mit Referenz-Plänen aus dem Python-Original abgesichert (inklusive der Feinheit, dass besetzte Lanes vor leeren nachrücken), und die Sortierung verschiebt nur, was tatsächlich falsch steht.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Fünf Test-Gruppen decken die Pläne, die Namens-/Index-Erkennung, die Anfänger-Rang-Auflösung und die Sortier-Reihenfolge ab — jeweils gegen Referenz-Ausgaben des Originals. Damit ist der Voice-Bereich funktional vollständig portiert; im Lane-Panel fehlt nur noch der Modus-Wechsel-Knopf.

## #100 — Steam-Panel-Buttons repariert + Scam-Nachrichten werden wieder gelöscht

**Ausgangslage:** Ein systematischer Bug-Audit über alle Bots hat zwei stille Fehler gefunden. Erstens: Die Buttons im öffentlichen Steam-Panel („Steam verknüpfen", „Rang prüfen") liefen seit dem Steam-Umzug auf einen Programmfehler — wer klickte, bekam einfach keine Reaktion. Ursache war ein Lesefehler beim Weiterreichen der Klick-Daten: Der Code griff auf das Feld „values" zu, erwischte dabei aber eine eingebaute Python-Funktion gleichen Namens statt der eigentlichen Auswahl-Werte, und stürzte beim Umwandeln ab. Zweitens: Der SecurityGuard übergab beim Löschen von Scam-Nachrichten einen Begründungs-Parameter, den die Discord-Bibliothek in der eingesetzten Version gar nicht kennt — die Löschung brach mit einem Typfehler ab, der von der Fehlerbehandlung nicht abgefangen wurde. Erkannte Scam-Nachrichten blieben dadurch in bestimmten Fällen einfach stehen.

**Was wurde geändert:** Der Panel-Code liest die Auswahl-Werte jetzt korrekt aus dem Daten-Wörterbuch der Interaktion und prüft dabei den Typ. Der SecurityGuard löscht ohne den nicht unterstützten Parameter — die Begründung steht weiterhin vollständig im Moderations-Log.

**Wie es jetzt funktioniert:** Klick auf „Steam verknüpfen" oder „Rang prüfen" leitet wieder sauber an den Steam-Dienst weiter und antwortet im Chat. Erkennt der SecurityGuard eine Scam-Nachricht (egal ob Einzelfall oder Nachrichten-Serie), wird sie wieder zuverlässig entfernt.

## #100 — Rust-Neuaufbau: der Lane-Router

**Ausgangslage:** Der Router-Voice-Kanal sortiert Spieler nach ihrer gespeicherten Vorliebe (Ranked, Casual, Street Brawl) automatisch in eine passende Lane ein — bevorzugt zu bekannten Mitspielern aus dem Co-Spieler-Graphen, sonst in die erste Lane mit Platz, und wenn nichts passt, wird eine neue erstellt. Ranked verlangt eine verifizierte Rang-Rolle, sonst gibt es eine freundliche DM mit dem Verifikations-Hinweis.

**Geändert:** Komplett in Rust angeschlossen — gleiche Vorlieben-Tabelle, gleiche Panel-Knöpfe (Modus-Wahl plus Auto-Join-Schalter, Kennungen unverändert), gleiche Auswahl-Logik samt Co-Spieler-Vorrang aus dem bereits portierten Aktivitäts-Graphen, gleiche Lane-Neuerstellung über die TempVoice-Maschine (die dafür gelernt hat, Lanes auch für Nutzer zu bauen, die im Router statt im Sammel-Kanal stehen). Wer den Modus im Panel wählt, während er im Router steht, wird sofort einsortiert. Damit ist der letzte als Umstiegs-Voraussetzung markierte Baustein abgehakt; einzige dokumentierte Rest-Lücke des Voice-Bereichs ist die separate Anfänger-Einsortierung samt ihrer Spezial-Lanes.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Lane-Wahl ist getestet — Sammel-Kanäle und volle oder leere Lanes werden nie gewählt, Mitspieler-Lanes gewinnen vor der erstbesten. Der Router hängt als Subscriber am zentralen Ereignis-Verteiler wie alle anderen Voice-Bausteine.

## #99 — Rust-Neuaufbau: die Feedback-DMs nach den ersten Voice-Runden

**Ausgangslage:** Wer seine allererste Voice-Session (mindestens 5 Minuten, mit Mitspielern) beendet, bekommt eine freundliche DM mit Feedback-Knopf und einem 4-Fragen-Formular; nach mindestens vier verschiedenen Voice-Tagen folgt einmalig eine zweite, kürzere Nachfrage. Die Antworten landen in der Datenbank und beim Owner. Das war die letzte offene Lücke des Voice-Tracker-Ports.

**Geändert:** Komplett in Rust angeschlossen: Texte und Formular-Fragen wortgleich (das Formular kann jetzt auch im neuen System mehrzeilige Antworten — dafür wurde der Modal-Baukasten um mehrzeilige Felder erweitert), gleiche Auslöse-Regeln (Erst-Session-Erkennung VOR dem Speichern, Mitspieler-Pflicht, 5-Minuten-Grenze, Vier-Tage-Regel einmalig), gleiche Tabellen, gleicher Knopf — auch alte, vor dem Umstieg verschickte Feedback-DMs funktionieren weiter, weil der Knopf seine Anfrage über die Datenbank wiederfindet statt über den Prozess-Speicher.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Tests spielen beide Auslöser durch: Erst-Session verschickt genau eine DM (kurze oder einsame Sessions nicht), die zweite Nachfrage kommt erst ab vier Voice-Tagen und genau einmal. Namen werden ab elf Mitspielern als „+N weitere" gekappt wie im Original.

## #98 — Rust-Neuaufbau: die Min-Rang-Sperre im Panel

**Ausgangslage:** Comp/Ranked-Lane-Besitzer können eine Rang-Untergrenze setzen — Rang-Rollen unterhalb der Schwelle verlieren das Verbinden-Recht, und der Lane-Name bekommt ab Emissary den Zusatz „• ab X". Der Panel-Knopf war im Rust-Port noch gesperrt.

**Geändert:** Komplett angeschlossen: Auswahlmenü mit allen Rängen (plus „Kein Min-Rang" zum Zurücksetzen), Rollen-Sperren exakt nach der Original-Punktelogik (Haupt- und Unterrang-Rollen, Kurzformen inklusive), Aufräumen der Sperren beim Zurücksetzen, Namens-Zusatz über die bestehende, getestete Namenslogik. Nur für Comp/Ranked-Lanes — Chill-Lanes lehnen mit klarer Ansage ab.

**Wie es jetzt funktioniert:** Wie vorher — Besitzer wählt die Schwelle im Panel, niedrigere Ränge können nicht mehr verbinden. Im Panel ist damit nur noch der Lanes-Modus-Wechsel offen.

## #97 — Rust-Neuaufbau Phase 5 (Teil 3): Player-Finder portiert (bleibt aus)

**Ausgangslage:** Der Player-Finder schlägt auf eine Suche im LFG-Kanal passende Mitspieler vor — gefiltert nach typischer Spielzeit, Wochentag, Voice-Aktivität der letzten 14 Tage und Rang-Nähe (±3), sortiert nach Steam-Status (Lobby vor Match vor „im Spiel" vor Discord-online). Er steht vor einem kompletten Redesign.

**Geändert:** Der Logik-Kern ist vereinbarungsgemäß nach Rust portiert, bleibt aber per Schalter deaktiviert (Standard: aus): alle Filter, die Kandidaten-Kette mit den drei „Lebenszeichen"-Bedingungen, die Status-Beschriftungen und ihre Rangfolge, die Datenbank-Zugriffe auf Muster, Aktivität und Steam-Presence (mit 2-Minuten-Frische-Grenze) sowie die 60-Sekunden-Abklingzeit pro Nutzer. Das geplante Redesign kann damit direkt auf der Rust-Basis aufsetzen statt auf dem Alt-Code.

**Wie es jetzt funktioniert:** Gar nicht — und das ist Absicht. Der Schalter bleibt aus, bis das Redesign steht; die Logik ist mit Tests abgesichert, damit beim Redesign klar ist, was das Alt-Verhalten war.

## #96 — Rust-Neuaufbau Phase 7 (Teil 2): die Coaching-Brücke

**Ausgangslage:** Der Bot hält die Coaching-Plattform der Website synchron: Alle zehn Minuten übermittelt er das Coach-Roster (wer die Coach-Rolle trägt, mit Namen und Avatar), und jede Minute holt er fällige Termin-Benachrichtigungen ab und stellt sie als DM zu — Termin geplant, Erinnerung zwei Stunden vorher, Absage.

**Geändert:** Beide Abläufe sind in Rust portiert: gleiche Schnittstellen und Kopfzeilen, gleiche Token-Kette, die DM-Texte wortgleich inklusive Berlin-Zeitformat („Mi, 10.06. um 19:00 Uhr", sommer- wie winterzeitfest), und die zwei wichtigen Schutzregeln des Originals: Ein leeres Coach-Roster wird nie übermittelt (sonst würde ein Discord-Schluckauf alle Coaches von der Website wischen), und Nutzer mit deaktivierten DMs werden als zugestellt bestätigt statt endlos erneut versucht.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Datums-Formatierung und alle drei DM-Texte sind mit Tests gegen das Original-Format abgesichert (inklusive Sommer-/Winterzeit-Wechsel). Die Loops starten erst mit der Gateway-Übernahme.

## #95 — Rust-Neuaufbau Phase 6 (Teil 4): der Sicherheits-Wächter

**Ausgangslage:** Der Sicherheits-Wächter schützt den Server vor Scam-Wellen und gekaperten Accounts über drei Pfade: das deterministische Takeover-Muster (Bilder in mehreren Kanälen binnen 30 Sekunden — sofortige Quarantäne ohne KI-Urteil), Mehrkanal-Bursts junger Accounts (mit KI-Bestätigung ab 78 % Sicherheit) und Keyword-Einzeltreffer (Telegram-Werbung, Gewinnversprechen und Co.), bei denen etablierte Accounts einen reversiblen Vorschlag bekommen statt des direkten Vollzugs.

**Geändert:** Alle drei Pfade sind in Rust portiert — gleiche Schwellen, gleiche Schlagwort-Liste, gleicher KI-Prompt, gleiche Fall-Akte in derselben Tabelle, gleiche Reihenfolge im Vollzug (Benachrichtigung an den Betroffenen, dann Aktion, dann Beweise löschen, öffentliche Notiz, Mod-Alarm mit Ban/Timeout-aufheben/Unban-Knöpfen). Der Ereignis-Verteiler liefert dafür jetzt auch Anhang-Zahlen und Account-Alter mit. Eine bewusst konservative Übergangs-Einschränkung: Die Bild-Inhalts-Prüfung (lief über ein externes Kommandozeilen-Werkzeug) ist noch nicht angebunden — Fälle, die nur über Bildinhalte bestätigt würden, landen deshalb als reversibler 60-Minuten-Vorschlag statt als automatischer Vollzug. Nie schärfer als das Original, im Zweifel milder.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Neun Tests decken die Detektions-Kerne ab — Takeover zündet bei Bildern in zwei Kanälen, nicht beim Doppelpost im selben Kanal und nicht außerhalb des Zeitfensters; die Burst-Regeln und Altersgrenzen entscheiden exakt wie das Original; kaputte KI-Antworten fallen sicher auf „kein Scam" zurück.

## #94 — Rust-Neuaufbau Phase 7 (Teil 1): die Onboarding-Knöpfe

**Ausgangslage:** Neue Mitglieder durchlaufen das Onboarding über feste Knöpfe im Regelkanal — der wichtigste davon ist die Regelbestätigung, die die Zugangs-Rolle vergibt. Dazu kommen der Steam-Login-Knopf und die Hinweis-Knöpfe des DM-Assistenten.

**Geändert:** Die bestehenden Knöpfe sind mit unveränderten Kennungen in Rust angeschlossen: Regelbestätigung vergibt die Onboarding-Rolle (mit ehrlicher Rückmeldung, falls die Vergabe scheitert), der Steam-Knopf holt einen frischen Einmal-Login-Link vom Steam-Dienst, und die vier Assistenten-Hinweise (Steam, FAQ, Streamer, Beta) antworten wortgleich wie bisher. Die Schritt-Navigation des geführten Kanal-Flows sagt übergangsweise ehrlich, dass sie umgebaut wird — der volle Flow folgt.

**Wie es jetzt funktioniert:** Die Knöpfe unter den bestehenden Nachrichten im Regelkanal funktionieren nach dem Umstieg weiter — insbesondere kommt jedes neue Mitglied über die Regelbestätigung an seine Rolle.

## #93 — Rust-Neuaufbau: Server-Warnungen auch im neuen Empfänger

**Ausgangslage:** Parallel zum Umbau bekam der Changelog-Empfänger eine neue Aufgabe: Der Server-Monitor postet Speicher-Warnungen als Embed in den Admin-Kanal (mit Ping bei Warnung/Kritisch, Entwarnung ohne) — Lehre aus dem Speicher-Vorfall vom 10. Juni.

**Geändert:** Die neue Warn-Route ist im Rust-Empfänger nachgezogen — gleicher Pfad, gleiche Prüfungen (Token, Pflichtfelder, nur die drei bekannten Stufen), gleiche Embeds mit Stufen-Symbol und -Farbe, gleicher Admin-Ping bei Warnung und Kritisch. Damit bleibt der Server-Monitor auch nach dem Bot-Umstieg ohne Anpassung funktionsfähig.

**Wie es jetzt funktioniert:** Unverändert — der Monitor schickt seine Meldung an den lokalen Empfänger, der Admin-Kanal bekommt das Embed, bei ernsten Stufen klingelt der Ping.

## #92 — Rust-Neuaufbau: Aufräumdienst beim Start

**Ausgangslage:** Nach einem Bot-Neustart können verwaiste Lanes übrig bleiben — Kanäle, die in der Datenbank als Lanes geführt werden, aber leer sind oder gar nicht mehr existieren.

**Geändert:** Der Start-Aufräumlauf ist portiert: 30 Sekunden nach dem Start (wenn der Discord-Zwischenspeicher sicher gefüllt ist — sonst sähen fälschlich alle Lanes leer aus) werden bekannte Lanes geprüft und leere oder verschwundene abgebaut, inklusive Datenbank-Eintrag.

**Wie es jetzt funktioniert:** Wie das Original mit seiner Start-Verzögerung — nur dass die Schutz-Wartezeit hier großzügiger gewählt ist, weil der Rust-Prozess schneller hochkommt als sein Discord-Zwischenspeicher.

## #91 — Rust-Neuaufbau: der Lurker-Modus

**Ausgangslage:** Der 👻-Lurker-Knopf im Lane-Panel war im Rust-Port noch gesperrt. Lurker sind stille Zuhörer: Sie bekommen die Lurker-Rolle, heißen sichtbar „Lurker", und das Lane-Limit wächst um eins, damit sie keinen Spielplatz blockieren — beim Verlassen wird alles automatisch zurückgebaut.

**Geändert:** Komplett in Rust angeschlossen, mit derselben Umschalt-Logik (Knopf an = Rolle + Name + Limit hoch, Knopf aus = alles zurück inklusive des gespeicherten Original-Namens), demselben Datenbank-Merkzettel und denselben Sicherungen: Schlägt die Rollen-Vergabe fehl, wird der Datenbank-Eintrag zurückgerollt; der Namens-Wechsel ist unkritisch und bricht nichts ab. Auch das automatische Aufräumen beim Verlassen der Lane hängt jetzt am zentralen Ereignis-Verteiler.

**Wie es jetzt funktioniert:** Wie vorher — Knopf drücken macht dich zum Lurker, nochmal drücken (oder die Lane verlassen) macht es rückgängig. Damit sind von den Panel-Funktionen nur noch der Lanes-Modus-Wechsel und die Min-Rang-Auswahl offen.

## #90 — Rust-Neuaufbau: Lane-Tag-Filter angeschlossen

**Ausgangslage:** Lane-Besitzer können ihre Lane filtern („nur 25+", „Ragebaiter blockieren") — der Filter setzt Verbinden-Sperren für betroffene Nutzer und trennt sie beim Beitritt. Im Rust-Port war der Panel-Button bisher als „noch nicht freigeschaltet" markiert, weil das Tag-System fehlte.

**Geändert:** Mit dem neuen Tag-System ist der Filter jetzt vollständig angeschlossen: Speichern in derselben Tabelle, Durchsetzung beim Beitritt (Sperre plus Trennung), sofortige Anwendung beim Speichern über das Panel (zwei Auswahlmenüs: Alters-Filter, Ragebaiter-Block), und die Sofort-Reaktion, wenn die Moderation jemandem den Ragebaiter-Marker verpasst, während er in einer geschützten Lane sitzt. Die Aufhebe-Logik respektiert Besitzer-Banns: Wer zusätzlich vom Besitzer gebannt ist, behält seine Sperre auch wenn der Filter ihn freigeben würde. Zwei Original-Eigenheiten sind dokumentiert übernommen: Der gespeicherte „Ton"-Filter wurde auch bisher nie durchgesetzt (toter Zweig), und ohne Tag-Dienst bleibt der Filter wirkungslos.

**Wie es jetzt funktioniert:** Wie im Original — Besitzer stellt den Filter im Panel ein, betroffene Nutzer können nicht mehr verbinden und werden getrennt, alle Komponenten laufen über die getestete Engine und das getestete Tag-System.

## #89 — Rust-Neuaufbau Phase 6 (Teil 3): das Tag-System

**Ausgangslage:** Tags sind das Bindeglied zwischen drei Systemen: Im Onboarding wählen Nutzer ihre Alters- und Ton-Präferenz („25+", „banter_ok", „ragebaiter-free"), die Lane-Filter im Voice-Bereich werten sie aus, und die Moderation vergibt den zeitlich befristeten „Ragebaiter"-Marker (14 Tage Standard-Laufzeit), der bei wiederholtem Fehlverhalten automatisch gesetzt wird.

**Geändert:** Das Tag-System ist als zentrale Anlaufstelle in Rust portiert: gleiche Tabellen, gleiche Gültigkeits-Prüfungen (nur bekannte Schlüssel und Werte, normalisiert), gleiche Standard-Laufzeit, gleicher 5-Minuten-Aufräumlauf für abgelaufene Mod-Tags, und Änderungs-Ereignisse für die angeschlossenen Systeme — das Pendant zu den bisherigen Bot-internen Benachrichtigungen, an denen Lane-Filter und Moderation hängen.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Tests decken die Kernregeln ab: ungültige Tags werden abgewiesen, Wert-Wechsel feuern genau ein Ereignis (gleicher Wert keins), der Ragebaiter-Marker läuft nach 14 Tagen ab und wird vom Aufräumlauf entfernt, und ein Neustart stellt den kompletten Zustand aus der Datenbank wieder her. Damit ist der Unterbau für die noch offenen Lane-Tag-Filter und die Ragebaiter-Automatik der Moderation gelegt.

## #88 — Rust-Neuaufbau Phase 8 (Teil 3): die Turnier-Website

**Ausgangslage:** Die öffentliche Turnier-Seite (Anmeldung per Discord-Login, Team-Verwaltung, Turnierbaum-Vorschau) lief als Web-Server im Bot-Prozess: Einmal-Login-Token, Sitzungs-Cookies mit CSRF-Schutz und elf API-Routen über dem Turnier-Unterbau.

**Geändert:** Komplett in Rust portiert und in den Web-Prozess umgezogen: derselbe Login-Fluss (Weiterleitung zum Link-Dienst, Einmal-Token einlösen, 6-Stunden-Sitzung mit gleitender Verlängerung), dieselben Sicherheits-Kopfzeilen, derselbe Turnierbaum-Generator (Team-Schnitt aus Rang-Punkten, Setzliste Erster-gegen-Letzter, Freilose, Finale/Halbfinale-Beschriftung — inklusive der Original-Eigenheit, dass Unterrang 0 als Mitte gewertet wird) und alle Anmelde-Regeln: offener Zeitraum, verifizierter Steam-Account als Rang-Quelle, Team-Pflichten, Nur-Ersteller-Rechte bei Umbenennen und Rauswerfen. Eine dokumentierte Übergangs-Notiz: Die Turnier-Rollen-Prüfung lief bisher über den Discord-Zwischenspeicher des Bots und wurde stillschweigend übersprungen, wenn der nicht bereit war — der Web-Prozess hat keinen Discord-Zugang, also gilt vorerst genau dieses Original-Ausweichverhalten.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Rust-Version lief parallel zum Live-Original gegen dieselben echten Daten: die Übersichts-Antwort ist JSON-identisch, die Seite byte-identisch, abgewiesene Anfragen liefern dieselben Status-Codes. Dazu Tests für den kompletten Login-und-Anmelde-Fluss und den Turnierbaum mit Referenz-Ausgabe aus dem Python-Original.

## #87 — Rust-Neuaufbau Phase 8 (Teil 2): der Turnier-Unterbau

**Ausgangslage:** Anmeldungen, Teams, Turnier-Zeiträume und die Einmal-Anmelde-Links der Turnier-Website werden in vier Tabellen verwaltet — mit Regeln wie „Team-Namen sind 2–32 Zeichen und pro Server einmalig (Groß/Klein egal)", „Team-Anmeldung braucht ein existierendes Team" und „eine neue Turnier-Phase deaktiviert automatisch die alte".

**Geändert:** Der komplette Unterbau ist in Rust portiert — gleiche Tabellen, gleiche Prüfungen, gleiche Rückmeldungen (eingefügt/aktualisiert/unverändert, inklusive der Feinheit, dass ein fehlender Anzeigename den alten nicht überschreibt). Die Einmal-Token für den Website-Login verhalten sich identisch: einmal eingelöst oder abgelaufen heißt ungültig, alte Token werden beim Anlegen neuer weggeräumt.

**Wie es jetzt funktioniert:** Sieben Tests decken die Regeln ab — Team-Anlage ist wiederholbar statt doppelt, Anmeldungs-Wechsel von Solo zu Team, Phasen-Wechsel deaktiviert den Vorgänger, Token sind strikt einmalig. Discord-Menüs und die Turnier-Website docken als Nächstes hier an.

## #86 — Rust-Neuaufbau Phase 8 (Teil 1): der Team-Balancer

**Ausgangslage:** Für Custom Games und Turniere teilt der Balancer die anwesenden Spieler anhand ihrer Rang-Punkte in zwei gleich große Teams — er probiert alle Kombinationen durch und bewertet jede mit einer Formel aus Summen-Differenz, Durchschnitts-Differenz und Team-Varianz.

**Geändert:** Der Algorithmus ist in Rust portiert — verhaltensgleich bis in die Kombinations-Reihenfolge und mit Referenzwerten aus dem Python-Original auf neun Nachkommastellen abgesichert. Zwei Eigenheiten des Originals wurden dabei dokumentiert (und bewusst übernommen statt still „verbessert"): Bei großer Rang-Spreizung dominiert der Varianz-Anteil der Formel und der Balancer baut dann homogene statt gleich starker Teams — ob das so gewollt ist, ist eine fachliche Entscheidung für später. Und bei nur zwei oder drei Spielern liefert der Notfall-Pfad ein leeres zweites Team.

**Wie es jetzt funktioniert:** Identisch zu vorher — gleiche Eingabe, gleiche Teams. Der Discord-Ablauf drumherum (Auswahl-Menü, Team-Kanäle, Verschieben) folgt mit dem Custom-Games-Port.

## #85 — Rust-Neuaufbau Phase 6 (Teil 2): der KI-Moderations-Kern

**Ausgangslage:** Der KI-Moderator bewertet jede Nachricht im Haupt-Chat mit einem bewusst rau kalibrierten Regelwerk („Gaming-Ton ist normal, sei NICHT überempfindlich") und entscheidet dreistufig: eindeutige Fälle (explizites NSFW, Scam) werden ab 90 % Sicherheit sofort gelöscht, echte Verstöße ab 78 % als Vorschlag mit Bestätigen/Ban/Ablehnen-Buttons an die Mods gegeben, und Ragebait wird nur gezählt — vier Treffer in zwei Stunden eskalieren zu einem Mod-Vorschlag.

**Geändert:** Dieser Kern ist in Rust portiert: das Regelwerk wortgleich, die Antwort-Auswertung mit denselben Gültigkeits-Prüfungen und Grenzwert-Klemmungen, das komplette Schwellen-Routing, das Ragebait-Zeitfenster in denselben Tabellen, die Fall-Akte (wer, was, Kategorie, Sicherheit, Mod-Entscheidung) und die Review-Buttons mit unveränderten Kennungen. Pro Nutzer gilt weiter die 2-Sekunden-Bremse, Moderatoren werden nie gescannt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Auswertung und Routing sind mit Testfällen abgesichert — auch die Eckfälle: „delete" außerhalb der Sofort-Kategorien wird trotz 95 % nur zum Vorschlag, ungültige KI-Antworten landen sicher bei „braucht Kontext" statt in einer Aktion. Bewusst noch offen: die Kontext-Nachladung bei Grenzfällen, Bild-Bewertung und die Ton-Tag-Sonderschwellen — sie folgen mit dem Tag-System.

## #84 — Rust-Neuaufbau Phase 6 (Teil 1): die KI-Anbindung — und der Streamer-Erkenner denkt wieder mit

**Ausgangslage:** Mehrere Module brauchen die MiniMax-KI: der Streamer-Erkenner für unklare Namens-Paare, die Gruppensuche für die Zweitprüfung, später Moderation und Chat. Seit dem Rust-Port des Streamer-Erkenners lief dieser im reinen Heuristik-Modus.

**Geändert:** Die KI-Anbindung ist in Rust portiert — beide Betriebsarten des Originals: der Token-Plan-Modus (Anthropic-kompatible Schnittstelle) und der Standard-Modus (klassische Chat-API), mit derselben Schlüssel-Suchreihenfolge und denselben Standard-Einstellungen. Die nicht konfigurierten OpenAI-/Gemini-Pfade wurden bewusst weggelassen. Direkt angeschlossen: Der Streamer-Erkenner bekommt seine KI-Zweitmeinung zurück — wortgleicher Prompt, Temperatur 0, knappes Antwort-Limit, JSON-Auswertung wie gehabt. Ohne konfigurierten Schlüssel fällt er automatisch auf die konservative Heuristik zurück.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Beide API-Betriebsarten sind gegen einen Mock-Server verifiziert — Kopfzeilen, Anfrage-Aufbau und Antwort-Auswertung (inklusive des Falls, dass die KI Denk-Fragmente mitliefert, die übersprungen werden). Damit ist die größte dokumentierte Lücke aus dem Brücken-Port geschlossen.

## #83 — Rust-Neuaufbau Phase 5 (Teil 2): die LFG-Erkennung

**Ausgangslage:** Die Gruppensuche erkennt im LFG-Kanal automatisch, ob eine Nachricht eine Mitspieler-Suche ist — über eine Wortmuster-Heuristik, die auf 500 echten Kanal-Nachrichten kalibriert wurde („wer bock", „suche +2", „jemand wach?", inklusive Privat-Kontakt-Ausnahme), plus Bewertungs-Formeln für Rang-Nähe und Zeit-Übereinstimmung bei den Empfehlungen.

**Geändert:** Die komplette Erkennungs-Heuristik, die Rang-Nähe-Bewertung (Unterrang-genau, drei Toleranz-Stufen), die Zeit-Übereinstimmung (typische Stunden ±2 mit Mitternachts-Übergang, typische Wochentage) und das Wunsch-Filter-Parsing („25+", „ragebaiter-free" — mit Ziffern-Grenzen, sodass „125+ hp" nicht zündet) sind in Rust portiert und mit Referenzfällen aus dem laufenden Python-Original abgesichert — 17 Beispiel-Nachrichten liefern exakt dieselbe Ja/Nein-Entscheidung.

**Wie es jetzt funktioniert:** Diese Bausteine sind die testbare Grundlage; der sichtbare Ablauf (Nachricht erkennen → in die passende Lane lotsen → Mitspieler vorschlagen) folgt, sobald die KI-Zweitprüfung mit der KI-Schicht portiert ist.

## #82 — Rust-Neuaufbau Phase 5 (Teil 1): Aktivitätsmuster und Mitspieler-Graph

**Ausgangslage:** Wer wann typischerweise im Voice ist und wer mit wem spielt, wird in zwei Tabellen gepflegt, aus denen Gruppensuche, Statistiken und Empfehlungen lesen. Zwei Schreiber füttern sie: ein 6-Stunden-Lauf für die Muster und ein 10-Minuten-Takt für die aktuellen Voice-Paarungen.

**Geändert:** Beide Schreiber sind in Rust portiert: Der Muster-Lauf wertet die letzten 14 Tage aus (typische Top-3-Stunden und -Wochentage, Sitzungs-Zähler, Minuten, letzte Aktivität — bis auf die Reihenfolge bei Gleichstand identisch zur Python-Sortierung) und überschreibt idempotent. Der 10-Minuten-Takt erfasst alle Paarungen in Voice-Kanälen mit mindestens zwei echten Mitgliedern bidirektional samt Anzeigenamen. Dabei wurde ein stiller Zähl-Fehler des Originals NICHT übernommen: Der alte 6-Stunden-Lauf addierte zusätzlich die kompletten 2-Wochen-Mitspieler-Aggregate bei jedem Lauf erneut auf — viermal am Tag dieselben Sitzungen obendrauf, zusätzlich zum 10-Minuten-Takt. Im Rust-Port schreibt nur noch der korrekte, inkrementelle Pfad.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Muster-Berechnung ist mit Referenzwerten aus dem Python-Original abgesichert, das Mitspieler-Tracking mit Datenbank-Tests gegen das echte Schema (beidseitige Einträge, Akkumulation über mehrere Takte, Namens-Pflege). Beide Läufe starten erst mit der Gateway-Übernahme.

## #81 — Rust-Neuaufbau Phase 4c (Teil 4): das Lane-Steuerungs-Panel

**Ausgangslage:** Das Interface-Panel im Voice-Bereich ist die Schaltzentrale für Lane-Besitzer: Region, Besitz übernehmen, Limit, Kick/Bann/Entbannen, Schnell-Vorlagen, Presets, Rang-Präferenz und Umbenennen — alles über Buttons unter einer festen Nachricht.

**Geändert:** Der Kern des Panels ist in Rust portiert, mit unveränderten Button-Kennungen — das bestehende Panel im Kanal funktioniert nach dem Umstieg ohne Neuposten weiter. Umgesetzt: Region DE/EU (sperrt bzw. entsperrt die English-Only-Rolle am Kanal und merkt sich die Wahl), Besitz-Übernahme mit den Original-Regeln (nur wenn der Besitzer weg ist, nur die drei am längsten Verbundenen, mindestens 20 Minuten im Kanal — mit denselben erklärenden Ablehnungs-Texten), Limit-Dialog mit Vorlagen-Obergrenzen (Street Brawl bleibt bei 4), Kick/Bann/Entbannen über Mitglieder-Auswahllisten (Bann gilt besitzerweit über alle Lanes), Duo/Trio/Reset-Schnellknöpfe, Preset speichern/laden, Rang-Präferenz für Chill-Lanes und Owner-Umbenennen. Dafür wurde die TempVoice-Engine um eine saubere Befehls-Fassade erweitert (Claim-Prüfung, Limit-Kappung, Region, Besitzwechsel mit Bann-Tausch).

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Jeder Button prüft zuerst „bist du in einer Lane?" und „gehört sie dir?" mit den bekannten Antworten. Workspace-weit bleiben alle Tests grün; die Claim-/Limit-/Bann-Logik sitzt in der getesteten Engine. Drei Panel-Funktionen sind bewusst noch nicht freigeschaltet und sagen das ehrlich an: Tag-Filter, Lurker-Modus und der Lanes-Modus-Wechsel — ihr Unterbau (Tag-Filter-System, Lurker-Verwaltung, Router-Lanes) folgt vor dem Voice-Umstieg.

## #80 — Rust-Neuaufbau Phase 4c (Teil 3): das Rang-Türsteher-System

**Ausgangslage:** Comp/Ranked-Lanes haben einen Rang-Anker: Der Erstbesitzer (oder das erste rangierte Mitglied) bestimmt ein Score-Fenster von ±9 Unterrang-Punkten (anderthalb Hauptränge), und nur Rang-Rollen in diesem Fenster dürfen verbinden. Der Kanal heißt nach dem Anker („Phantom 3"). Wichtigste Eigenschaft: Es wird nie jemand rausgeworfen — nur die Verbinden-Rechte werden gesteuert.

**Geändert:** Komplett in Rust portiert: die Rang-Erkennung aus Rollennamen (Unterrang-Rollen wie „Asc 3" schlagen Haupt-Rollen, Kurzformen werden aufgelöst, Unterrang-Fallback aus dem verknüpften Steam-Account, sonst Mitte), die Fenster-Berechnung, die Anker-Wahl mit Erstbesitzer-Vorrang (direkt an die neue TempVoice-Engine angebunden), die Sammel-Aktualisierung der Kanal-Rechte in einem einzigen Discord-Aufruf (Jedermann-Sperre, erlaubte Unterrang-Rollen rein, nicht mehr erlaubte Rang-Rollen raus, fremde Einstellungen unangetastet) und die Wahl der „wirklich spielenden" Gruppe über dieselbe Presence-Kohorten-Logik wie der Live-Status. Anker überleben Neustarts über dieselbe Datenbank-Tabelle.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Fenster-Berechnung, Rollen-Erkennung, Rollen-Auswahl im Fenster und Namensgebung sind mit Referenzwerten aus dem Python-Original abgesichert (acht Testfälle, inklusive der Eckfälle Eternus 6 und Initiate 1). Das System hängt als weiterer Subscriber am zentralen Ereignis-Verteiler.

## #79 — Rust-Neuaufbau Phase 4c (Teil 2): der Live-Status an den Voice-Lanes

**Ausgangslage:** Die Anzeige „Lane 1 - im Match Min 17 (4/6)" an den Voice-Kanälen kommt von einem Minuten-Worker, der Steam-Presence-Daten mit den Kanal-Mitgliedern abgleicht und daraus die größte zusammen spielende Gruppe ermittelt — inklusive Party-Aufstockung für Mitglieder ohne verknüpften Steam-Account.

**Geändert:** Komplett in Rust portiert: Presence-Auswertung (mit Frische-Grenze von 3 Minuten, Minuten-Erkennung auch aus dem lokalisierten Steam-Text), Kohorten-Wahl (Match schlägt Lobby, bei Gleichstand die größere Gruppe, Server-Zuordnung vor Unbekannt), Party-Abgleich (beste Party nach Überlappung mit der Gruppe, gemeldeter Größe und Frische; fehlende unverknüpfte Mitspieler werden bis Partygröße aufgestockt) und die komplette Rename-Disziplin: 6 Minuten Abstand zwischen Umbenennungen (10 ab Match-Minute 25), Match-Ende und Status-Löschung dürfen den Abstand umgehen, und reine Mitgliederzahl-Änderungen ohne Spielstatus benennen NIE um. Auch die Standort-Tabelle, die anderen Diensten sagt, in welchem Kanal eine Steam-ID gerade sitzt, wird identisch gepflegt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die gesamte Auswertungs-Kette ist mit Referenzwerten aus dem Python-Original abgesichert — dieselben Presence-Zeilen ergeben dieselbe Einstufung, dieselben Gruppen, dieselben Suffixe, dieselbe Umbenennen/Warten-Entscheidung in allen sechs Regelfällen. Der Worker läuft im selben 60-Sekunden-Takt gegen dieselben Tabellen.

## #78 — Rust-Neuaufbau Phase 4c (Teil 1): die Steam-Verknüpfungs-Erinnerung

**Ausgangslage:** Wer ohne verknüpften Steam-Account regelmäßig im Voice ist, bekommt einmalig eine freundliche DM mit dem Verknüpfungs-Link — aber bewusst erst am zweiten Voice-Tag und erst nach 30 Minuten am Stück, damit niemand beim ersten Reinschnuppern angeschrieben wird.

**Geändert:** Komplett in Rust portiert, als weiterer Subscriber des Ereignis-Verteilers: gleiche Ausnahmen (Datenschutz-Widerspruch, ausgenommene Rollen, bereits verknüpft, bereits erinnert), gleiche Merkzettel in denselben Datenbank-Feldern (Erst-Sichtung, Erledigt-Status, DM-Referenz), gleicher DM-Inhalt samt Datenschutz-Erklärung, frischem Steam-Login-Link vom Steam-Dienst und Schließen-Button — dessen Kennung unverändert bleibt, damit auch die Schließen-Buttons ALTER, vor dem Umstieg verschickter DMs weiter funktionieren.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Tests decken die Kernregeln ab: Tag eins wird nur vorgemerkt (keine DM), ab Tag zwei startet die 30-Minuten-Beobachtung, wer verknüpft oder erledigt ist wird übersprungen, und der Versand schreibt Erledigt-Status und DM-Referenz korrekt in die Datenbank. Nebenbefund dieser Etappe: Der „voice_reaction_dm"-Baustein stellte sich als falsch einsortierter Twitch-Verkaufs-Melder heraus (liest die Twitch-Bot-Datenbank, standardmäßig aus) — der gehört in den Twitch-Bot-Umbau und wird dort übernommen, nicht hier.

## #77 — Rust-Neuaufbau Phase 4b: TempVoice-Kern

**Ausgangslage:** TempVoice ist mit Abstand das größte Einzelstück des Bots (~6.700 Zeilen): Beitritt in einen Sammel-Kanal erstellt automatisch eine eigene Voice-Lane, mit Besitzer-Logik, Bann-Listen, Rang-Namen und Aufräumen.

**Geändert:** Der Verhaltens-Kern ist in Rust: Join-to-create aus allen drei Sammel-Kanälen (Chill mit Rang-Namen aus Präferenz oder Rollen, Street Brawl mit festen 4er-Lanes, Comp/Ranked), Besitzer-Lebenszyklus (Auto-Übergabe an das am längsten verbundene Mitglied beim Verlassen, Besitzer-Nachtrag bei herrenlosen Lanes), Owner-Bann-Listen als Kanal-Rechte, Löschen leerer Lanes, Namens-Schutzregeln (45-Sekunden-Fenster, nie bei Live-Match-Anzeige) und die Rang-Mathematik (Haupt- und Unterränge, Kurzformen wie „Asc 3", Durchschnittsrang) — alles mit Referenzwerten aus dem Python-Original abgesichert. Datenbank-Verträge unverändert: Lanes, Bann-Listen, Voreinstellungen und Rang-Präferenzen nutzen dieselben Tabellen, Erstbesitzer und Quell-Kanal bleiben bei Updates erhalten wie bisher.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** 21 Tests decken die Kernpfade ab — Lane-Erstellung mit Rang-Namen, Präferenz schlägt Rollen, Besitzer-Übergabe an den Ältesten, Löschung beim letzten Verlassen, Bann-Rechte beim Erstellen — gespielt gegen einen Discord-Mock und die echten Tabellen-Schemata. Bewusst noch offen (vor dem Voice-Umstieg): das Steuerungs-Panel mit seinen Buttons, Tag-Filter/Lurker-Sonderlogik und die Rang-Berechtigungs-Kopplung — sie folgen mit dem Rang-Lane-Manager.

## #76 — Rust-Neuaufbau Phase 4a: das Voice-Tracking-Fundament

**Ausgangslage:** Das Voice-Session-Tracking ist die Datenbasis für die halbe Community-Statistik — Bestenlisten, Heatmaps, Mitspieler-Netzwerk und Gruppensuche lesen alle aus den Tabellen, die es schreibt. Im Original ist es einer von fünf Lauschern, die sich unkoordiniert dasselbe Voice-Ereignis teilen.

**Geändert:** Der Tracking-Kern ist als erster Subscriber des zentralen Ereignis-Verteilers in Rust portiert: Sessions starten ab zwei aktiven (ungestummten) Nicht-Bots im Kanal, Stummschalten beendet die Aktivität — außer für Mitglieder mit der Schonfrist-Rolle, die drei Minuten Karenz bekommen. Beim Ende werden Punkte berechnet (1 pro Minute plus Bonus bei vollen Kanälen), Gesamtwerte hochgezählt und die Session mit Mitspielern, Nutzerverlauf und Spitzenbelegung in die Historie geschrieben — feldgenau in dieselben Tabellen und Formate wie bisher, inklusive der Datenschutz-Regel: Wer widersprochen hat, wird nie erfasst. Die drei Wartungs-Schleifen (Keep-Alive, Schonfrist-Ablauf, Aufräumen verwaister Sessions) laufen wie im Original.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Acht Tests spielen den Lebenszyklus komplett durch — Beitritt zu zweit startet Sessions, allein nicht, Stummschalten ohne Rolle beendet, mit Rolle hält die Karenz, Opt-out wird nie erfasst, verwaiste Sessions räumt der Wächter auf — und prüfen die Datenbank-Schreibvorgänge gegen das echte Tabellen-Schema; die Punkteformel ist mit Referenzwerten aus dem Python-Original abgesichert. Bewusst noch offen für 4b: das Feedback-Nachrichten-System nach der ersten Session und die Statistik-Befehle — beides kommt, bevor der Voice-Bereich umgeschaltet wird.

## #75 — Rust-Neuaufbau Phase 3 komplett: der Streamer-Erkenner

**Ausgangslage:** Das letzte Stück der Brücken-Phase: Der Abgleich, der alle 6 Stunden neue Twitch-Streamer gegen die Discord-Mitgliederliste hält — mit Namens-Normalisierung (Akzente raus, Leetspeak übersetzt, Anhängsel wie „TTV" und „live" entfernt), Ähnlichkeits-Berechnung und der Drei-Wege-Entscheidung: automatisch verknüpfen, als Vorschlag mit Bestätigen/Ablehnen-Buttons an die Mods geben, oder verwerfen.

**Geändert:** Komplett in Rust portiert, inklusive des Ähnlichkeits-Algorithmus aus Pythons Standardbibliothek (difflib), der Zeichen für Zeichen nachgebaut wurde. Damit das beweisbar stimmt, wurden Referenzwerte direkt aus dem laufenden Python-Original gezogen und als Tests eingebacken — „N4ni" wird zu „nani", „drag_skope | TTV" zu „dragskope", und die Ähnlichkeit von „dragskope" zu „dragscope" ist auf zwölf Nachkommastellen identisch. Der Merkzettel bewerteter Streamer nutzt dieselbe Datei wie bisher — offene Vorschläge überleben also auch den Umstieg. Die Review-Buttons laufen über das neue Klick-Routing samt Mod-Rechte-Prüfung.

**Wie es jetzt funktioniert:** Wie bisher — alle 6 Stunden automatisch (der erste Lauf nach einem Neustart wird übersprungen, den Voll-Abgleich startet ein Admin bewusst per Kommando), nur eindeutige, exakte Namens-Treffer werden automatisch verknüpft, alles im Graubereich geht an die Mods. Eine ehrliche Übergangs-Einschränkung: Die KI-Zweitmeinung bei unklaren Fällen kommt erst mit der KI-Schicht in Phase 6 — bis dahin gilt die konservative Heuristik, exakt so, wie sich das Original verhält, wenn seine KI nicht verfügbar ist. Damit ist Phase 3 abgeschlossen; live geht das gesammelt mit dem Bot-Umstieg.

## #74 — Rust-Neuaufbau Phase 3b: Twitch-Klick-Tracking und das letzte Glied der Klick-Kette

**Ausgangslage:** #73 hatte das Klick-Routing gebaut, aber zwei Lücken gelassen: Die Twitch-Live-Buttons („Auf Twitch ansehen" unter Live-Ankündigungen) wurden noch nicht verarbeitet, und es fehlte die Übergabe von der echten Discord-Verbindung an das Routing — Klicks und Slash-Befehle kamen also noch nirgends an.

**Geändert:** Beide Lücken sind zu. (1) Die Twitch-Live-Brücke: Ein Klick auf den Live-Button wird beim Twitch-Bot als Zählung verbucht (mit Einmal-Schlüssel pro Klick — doppelt zählt nicht) und der Nutzer bekommt seinen persönlichen Twitch-Link als nur für ihn sichtbare Antwort. Beim Start holt sich der Bot alle aktiven Live-Ankündigungen, damit auch Buttons unter älteren Nachrichten funktionieren — und wenn ein Klick auf eine unbekannte Ankündigung trifft, lädt er die Liste einmal frisch nach, statt ins Leere zu laufen (das konnte das Original nicht). (2) Die Discord-Verbindung reicht jetzt alle Klicks, Eingabefenster und Slash-Befehle ans Routing weiter — mit derselben 2-Sekunden-Regel wie bisher: Dauert eine Antwort länger, erscheint erst „Bot denkt nach", dann die echte Antwort.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Gegen einen Mock-Twitch-Bot getestet: Klick-Zählung mit korrektem Einmal-Schlüssel und feldgenauem Inhalt, Standard-Beschriftung bei leerem Button-Text, Nachlade-Verhalten bei unbekannten Klicks, Hinweis bei wirklich abgelaufenen Ankündigungen. Damit ist die Abhängigkeit aus #72 aufgelöst: Vermittler und Discord-Verbindung können beim Umschalten gemeinsam wandern, weil die Klick-Verarbeitung jetzt komplett in Rust existiert. Es fehlt aus Phase 3 noch der Streamer-Erkennungs-Scan (läuft alle 6 Stunden) — danach ist die Brücken-Phase komplett.

## #73 — Rust-Neuaufbau Phase 3a: Klick-Verarbeitung und Steam-Brücke

**Ausgangslage:** Phase 2 hatte die offene Flanke benannt: Wer die Discord-Verbindung besitzt, muss auch alle Button-Klicks verarbeiten — sonst posten wir tote Knöpfe. Außerdem ist die Steam-Brücke (der „dünne Arm", über den /betainvite, /steam, Rang-Checks und die Link-Panels mit dem Rust-Steam-Bot reden) bisher Python.

**Geändert:** Zwei Bausteine in Rust: (1) Ein zentrales Klick- und Befehls-Routing — Buttons werden über exakte Kennungen oder Präfixe (z. B. alle `betainvite:`-Schritte mit einem Eintrag) an ihre Verarbeiter geleitet, Slash-Befehle kommen aus einer Registry, die gleichzeitig die Discord-Definitionen fürs Synchronisieren liefert. (2) Die komplette Steam-Brücke: alle 14 Slash-Befehle, beide Panels samt Alt-Kennungen früher geposteter Panels, der gesamte Einladungs-Funnel, das Freundescode-Eingabefenster (einzige lokale UI — alles andere wird 1:1 durchgereicht) und die `!steam_*`-Admin-Kommandos.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Für die Tests wurde ein Mock-Steam-Bot hochgefahren, der jede Anfrage aufzeichnet: Das Leitungsformat (Ereignis-Art, Nutzer/Server/Kanal, Befehlsname samt Argumenten, Freundescode als Werteliste) ist feldgenau identisch zum Python-Original, ebenso die Rückrichtung — Text, Embed, Sichtbarkeit, Link-Button und interaktive Buttons aus der Steam-Bot-Antwort. Auch der Ausfall-Fall („Steam-Bot nicht erreichbar") antwortet wortgleich. Live geht davon noch nichts — es fehlt bewusst das letzte Stück (die Übergabe der Klicks von der echten Discord-Verbindung an dieses Routing), das zusammen mit der Twitch-Brücke in Phase 3b kommt.

## #72 — Rust-Neuaufbau Phase 2: Vermittler, Changelog-Dienst und Event-Fundament

**Ausgangslage:** Andere Bots (z. B. der Twitch-Bot) führen Discord-Aktionen über den internen „Master-Broker" aus — eine lokale Schnittstelle mit Doppel-Absicherung (nur localhost + Token) und Schutz gegen versehentliche Doppel-Ausführung: Jede Aktion trägt einen Einmal-Schlüssel; Wiederholungen liefern das gespeicherte Ergebnis statt z. B. eine Nachricht zweimal zu senden. Dazu kommt der Changelog-Empfänger, über den diese Ankündigungen hier gepostet werden. Beides hing bisher am Python-Prozess.

**Geändert:** Beide Dienste sind vollständig in Rust nachgebaut (gehen noch NICHT live), dazu zwei Fundament-Stücke für alles Weitere: ein zentraler Event-Verteiler — künftig hören alle Bereiche auf EINE normalisierte Ereignis-Quelle statt wie bisher fünf Voice- und sechs Nachrichten-Lauscher nebeneinander — und die Discord-Anbindung selbst, sauber getrennt in Sofort-Aktionen (funktionieren ohne Live-Verbindung) und Live-Daten (brauchen die Gateway-Session, die bis zum Umschalten beim Python-Bot bleibt).

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Der Rust-Vermittler wurde auf einem Testport neben den laufenden Python-Vermittler gestellt: Authentifizierungs-Fehler, Antwort-Format und Fehlertexte sind deckungsgleich; die Doppel-Ausführungs-Sperre ist mit eigenen Tests abgedeckt (Wiederholung liefert Cache, anderer Inhalt zum selben Schlüssel wird abgelehnt, parallele Wiederholung wartet aufs Ergebnis). Wichtige Erkenntnis für den Umschaltplan: Interaktive Buttons, die der Vermittler postet, werden vom Besitzer der Live-Verbindung verarbeitet — Vermittler und Live-Verbindung müssen deshalb ZUSAMMEN umgeschaltet werden, nicht einzeln. Das ist dokumentiert und eingeplant.

## #71 — Rust-Neuaufbau: Aktivitäts-Statistiken portiert — Phase 1 komplett

**Ausgangslage:** Die öffentliche Statistik-Seite (Voice-Heatmaps, Rang-Verteilung, Bestenlisten, persönliche Statistiken mit Discord-Login) war mit 22 Endpunkten der zweite große Webdienst im Python-Bot-Prozess. Pikantes Detail aus der Analyse: Die „Rang-Schätzung über Mitspieler" für Spieler ohne verknüpften Steam-Account war seit jeher wirkungslos — sie lieferte Kategorien, die an jeder einzelnen Verwendungsstelle wieder herausgefiltert wurden. Ein stiller Bug, den nie jemand bemerkt hat, weil das Ergebnis „funktionierte".

**Geändert:** Alle 22 Endpunkte sind nach Rust portiert (gehen noch NICHT live): die acht Analyse-Ansichten, beide Bestenlisten, die sechs persönlichen Me-Endpunkte samt Discord-Login-Flow und die Sicherheits-Schicht (CORS-Allowlist, Cache-Regeln, signierte Session-Cookies). Die wirkungslose Mitspieler-Heuristik wurde nicht mitgenommen — verhaltensgleich, aber ehrlich. Nebenbei wurde aus „eine Datenbank-Abfrage pro Sitzungszeile" (das Original fragte den Rang desselben Spielers hunderte Male pro Anfrage neu ab) ein Zwischenspeicher pro Anfrage.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Wieder Testport gegen laufende Python-Version, gleiche Datenbank: **18 von 18 vergleichbaren Endpunkten liefern identisches JSON** — inklusive Heatmap-Bucketing, Wochentrends, Rundungen und Fehlerfällen; die ausgelieferte HTML-Seite ist byte-identisch. Die login-pflichtigen Me-Endpunkte wurden mit selbst signierten Test-Sessions gegen eine Datenbank-Kopie durchgespielt (echte Logins bleiben beim Umschalten gültig, weil die Cookie-Signierung nachweislich byte-gleich ist). Damit ist Phase 1 des Neuaufbaus komplett: Tierlist + Statistiken warten fertig verifiziert auf die Umschalt-Freigabe — die Checkliste dafür liegt in `rust/docs/02-cutover-phase1.md`.

## #70 — Rust-Neuaufbau: Tierlist komplett portiert und auf echten Daten bewiesen

**Ausgangslage:** Die öffentliche Tierlist (Hero-Winrates, Build-Votes, Admin-Pflege) lief als einer von sechs Webdiensten im Python-Bot-Prozess. Ihre Admin-Anmeldung griff dabei direkt in die internen Session-Daten des Dashboards — das funktioniert nur, solange alles in einem Prozess steckt, und genau diese Verquickung soll weg.

**Geändert:** Die komplette Tierlist ist jetzt in Rust nachgebaut (geht noch NICHT live, läuft parallel zur Python-Version): alle öffentlichen Endpunkte (Heldenliste, Tierlist in drei Rang-Buckets, Verlauf), das Build-Voting mit 5-Sekunden-Sperre pro Absender, die Admin-Endpunkte und der automatische Daten-Abruf von der Deadlock-API. Zwei Dinge wurden dabei sauberer gelöst: Die Admin-Anmeldung fragt jetzt über eine offizielle interne Schnittstelle beim Dashboard nach statt heimlich in dessen Speicher zu greifen, und die Session-Cookies werden byte-identisch zum Original signiert — ein Nutzer bleibt beim späteren Umschalten eingeloggt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Rust-Version wurde auf einem Testport gegen die laufende Python-Version gestellt — gleiche Datenbank, gleiche Anfragen. Ergebnis: alle sieben Lese-Endpunkte liefern **identisches JSON** bis aufs letzte Feld, alle Fehlerfälle (ungültige Build-ID, unbekannter Build, fehlende Anmeldung) antworten mit denselben Statuscodes und Texten, und das Voting wurde gegen eine Datenbank-Kopie durchgespielt (Stimme zählt, Sperre greift). Dafür musste sogar Pythons Rundungsverhalten exakt nachgebaut werden — die naive Variante rundete 50.365 in die falsche Richtung. Umgeschaltet wird erst nach Freigabe; bis dahin bedient weiterhin Python den Live-Betrieb.

## #69 — Rust-Neuaufbau gestartet: das Fundament steht

**Ausgangslage:** Der Bot ist über die Jahre zu einem 71.000-Zeilen-Python-Prozess gewachsen, der neben Discord auch sechs Webdienste gleichzeitig betreibt. Vieles ist doppelt (drei identische Server-Hüllen, zwei parallele Turnier-APIs, doppeltes Einladungs-Tracking), die Konfiguration ist über 38 Dateien verstreut — und ein Bot-Neustart reißt alle Websites mit, weil alles in einem Prozess steckt.

**Geändert:** Unter `rust/` beginnt der komplette Neuaufbau in Rust — nach dem bewährten Muster von Steam- und Twitch-Bot: Der Python-Bot läuft unverändert weiter, fertige Teile übernehmen einzeln und erst nach Freigabe. Phase 0 legt das Fundament:

- **Zwei getrennte Prozesse** statt einem: Bot (Discord + interne Schnittstellen) und Web (alle Websites + Dashboard). Künftige Bot-Neustarts treffen die Websites damit nicht mehr.
- **Eine zentrale Konfiguration** statt 166 verstreuter Umgebungs-Zugriffe — beim Start einmal gelesen und geprüft, mit denselben Variablennamen wie bisher.
- **Eine Datenbank-Schicht** auf die bestehende gemeinsame Datenbank: ein serialisierter Schreibkanal, beliebig viele parallele Leser, und sie weigert sich, bei falschem Pfad stillschweigend eine leere Datenbank anzulegen (klassische Fehlerquelle).

**Wie es jetzt funktioniert:** Beide Rust-Prozesse starten, lesen die echte Bot-Datenbank (116 Tabellen erkannt, rein lesend geprüft) und warten sauber auf ihr Stopp-Signal — sie binden noch keinen Port und übernehmen noch keine Funktion. Die Datenbank bleibt der Vertrag zwischen Alt und Neu: Rust ändert kein Schema, bevor ein Bereich offiziell übernommen wird. Der vollständige Plan mit Phasen, Architektur-Entscheidungen und Schema-Snapshot liegt in `rust/docs/`; 14 automatische Tests plus Format- und Lint-Prüfung sichern jede weitere Phase ab.

## #68 — Steam-Brücke rendert jetzt echte Buttons

**Ausgangslage:** Die Antworten des Steam-Dienstes (Einladungs-Flow, Link-Panel) kamen in Discord ohne sichtbare Schaltflächen an — die Brücke registrierte nur unsichtbare Platzhalter-Buttons, und die Texte verwiesen auf Schaltflächen, die niemand sehen konnte.

**Geändert:** Der Steam-Dienst liefert seine Buttons jetzt mit Beschriftung und Stil mit, und die Brücke baut daraus echte Discord-Schaltflächen an jeder Antwort. Die Panel-Buttons („🎟️ Einladung starten", „🔗 Steam verknüpfen", „🔢 Freundescode eingeben", „📊 Rang prüfen") haben sichtbare Beschriftungen bekommen.

**Betroffen:** Alle, die den Einladungs-Flow oder das Steam-Panel nutzen — der Ablauf ist jetzt klickbar statt rätselhaft. Bereits gepostete Panels zeigen die neuen Beschriftungen erst nach einem Neu-Posten der Panels.

## #67 — Steam-Austritts-Ban wieder entfernt + Reparatur-Befehl durchgereicht

**Ausgangslage:** Mit #66 hatte der interne Vermittler eine Ban-Route bekommen, damit der Steam-Dienst beim Server-Verlassen nach einem Playtest-Invite automatisch bannen kann. Diese Automatik ist auf Community-Entscheidung wieder gestrichen — den Server zu verlassen ist kein Bann-Grund.

**Geändert:** Die Ban-Route ist komplett ausgebaut (der Vermittler kann wieder nur Rollen, Nachrichten, DMs, Kanäle und Invites). Zusätzlich reicht die Steam-Brücke den neuen Admin-Befehl zum erneuten Senden einer Steam-Freundschaftsanfrage an den Steam-Dienst durch — das Werkzeug, mit dem versehentlich gekündigte Freundschaften (siehe Steam-Bot #22) wieder angeknüpft werden.

**Betroffen:** Niemand verliert etwas — gebannt wegen Austritt wird nicht mehr, und Admins haben einen Reparatur-Befehl mehr.

## #66 — Steam-Austritts-Ban wieder möglich (Broker-Ban-Route)

**Ausgangslage:** Der neue Steam-Dienst sollte beim Verlassen des Servers nach einem Playtest-Invite wieder bannen können (Anti-Missbrauch gegen Invite-Farming) — aber der interne Vermittler, über den der Dienst Discord-Aktionen ausführt, kannte bisher nur Rollen vergeben/entziehen und DMs, kein Bannen.

**Geändert:** Der interne Vermittler bekommt eine Ban-Route. Sie bannt per User-ID und funktioniert deshalb auch dann, wenn die Person den Server schon verlassen hat. Auth, Idempotenz und Server-Allowlist laufen wie bei den bestehenden Rollen-Routen.

**Betroffen:** Server-Moderation — der automatische Ban beim Verlassen nach einem Invite greift wieder (die eigentliche Entscheidung trifft der Steam-Dienst).

## #65 — Steam-Slash-Commands wieder verfügbar

**Ausgangslage:** Seit der Steam-Umstellung (Eintrag #62) fehlten die Slash-Commands rund um Steam — die alten Bausteine, die sie bereitstellten, sind abgeschaltet. Die Verknüpfungs-Buttons im Panel liefen weiter, aber Befehle wie `/account_verknüpfen`, `/steam links` oder `/checkrank` waren weg.

**Geändert:** Der dünne Steam-Vermittler im Bot registriert die Commands jetzt selbst und leitet sie an den neuen Steam-Dienst weiter, der die eigentliche Arbeit macht. Wieder da: `/account_verknüpfen`, `/steam links`, `/steam whoami`, `/steam setprimary`, `/steam unlink`, `/steam_rank`, `/checkrank` sowie die Admin-Befehle `/steam_rank_sync`, `/subrank_sync`, `/sync_steam_friends` und `/publish_steam_panel`.

**Wie's funktioniert:** Jeder Command schickt Name und Argumente in einem festen Format an den Steam-Dienst und rendert dessen Antwort (Text, Embed oder Login-Button). Befehle mit Eingabe (z. B. eine SteamID bei `/steam whoami`) übergeben diese mit; `/checkrank` löst die @-Mention vorher zum Discord-User auf. Die langlaufenden Admin-Syncs über die ganze Freundesliste bekommen ein größeres Zeitlimit und melden sich erst, wenn der Dienst fertig ist, statt vorzeitig abzubrechen.

**Betroffen:** Alle, die ihren Steam-Account verwalten oder ihren Rang prüfen, sowie Admins, die einen Sofort-Sync auslösen.

## #64 — Voice-Nudge-DM repariert: Steam-Link-Button funktioniert wieder

**Ausgangslage:** Seit der Steam-Umstellung (Eintrag #62) holte sich die freundliche Erinnerungs-DM ("verknüpf doch mal deinen Steam-Account"), die nach 30 Minuten im Voice verschickt wird, ihre Login-URL noch über den alten Weg: Sie suchte sich zur Laufzeit das passende Steam-Cog im Bot zusammen. Genau dieses Cog war aber abgeschaltet. Die Suche fand stattdessen den Nudge-Baustein selbst und lief dann beim Erzeugen der URL in einen Fehler — die DM kam entweder ohne funktionierenden Button oder gar nicht.

**Geändert:** Die Erinnerungs-DM und der zugehörige Health-Check fragen jetzt direkt den neuen Steam-Dienst nach einer frischen Login-URL, statt im Bot nach einem Cog zu suchen.

**Wie's funktioniert:**
- Es gibt jetzt eine zentrale Stelle, die beim Steam-Dienst einen Einmal-Link anfordert (15 Minuten gültig) — denselben, den auch der Steam-Verknüpfen-Button im Server-Panel erzeugt. Die Nudge-DM nutzt genau diese Stelle, dadurch gibt es keinen zweiten, abweichenden Weg mehr, der kaputtgehen kann.
- Der stündliche Selbsttest des Bots prüfte bislang, ob das alte (jetzt abgeschaltete) Steam-Modul geladen ist, und meldete deshalb dauerhaft einen Fehlalarm. Er prüft jetzt stattdessen, ob der neue Steam-Dienst auf seinem Health-Endpunkt antwortet.
- Der nicht mehr benötigte Watchdog der alten Steam-Bridge wurde abgeschaltet.

**Betroffen:** Alle, die nach längerer Zeit im Voice die Steam-Verknüpfungs-Erinnerung bekommen — der Button im DM führt wieder zuverlässig zum Login.

## #63 — Coaching: Coach-Roster-Sync zur Website + Termin-DMs

**Ausgangslage:** Die Coaching-Website wusste nicht, wer eigentlich Coach ist — sie kannte nur die Coaches, die zufällig schon einmal eine Session gespiegelt hatten. Und für die neuen Coaching-Termine der Website gab es keinen Weg, Spieler in Discord zu benachrichtigen.

**Geändert:** Ein neuer Baustein hält Discord und Website synchron: Er meldet alle Träger der Coach-Rolle an die Website und stellt Termin-Benachrichtigungen als DM zu.

**Wie's funktioniert:**
- **Rollen-Sync:** Beim Start, alle 10 Minuten und sofort bei jeder Rollenänderung (mit 5-Sekunden-Sammelfenster, damit mehrere Änderungen kurz hintereinander nur einen Sync auslösen) wird die komplette Coach-Liste mit Namen und Avataren übertragen. Leere Listen werden nie gesendet — Schutz davor, das Website-Roster versehentlich zu leeren. Ist der Mitglieder-Cache direkt nach dem Start noch leer, wird er einmal explizit nachgeladen.
- **Termin-DMs:** Der Baustein fragt die Website im Minutentakt nach fälligen Benachrichtigungen und schickt dem Spieler je nach Typ eine DM — Einladung bei Terminanlage, Erinnerung unter 2 Stunden vor Start, Absage-Info. Zeiten werden in deutscher Zeit formatiert. Hat ein Spieler DMs deaktiviert, wird die Benachrichtigung als erledigt markiert statt endlos neu versucht; bei Netzwerkfehlern wird sie beim nächsten Durchlauf erneut zugestellt.

## #62 — Steam-Umschaltung: alte Steam-Bausteine deaktiviert

Die Umschaltung auf den Rust-Steam-Dienst ist vollzogen: Über die Cog-Blockliste sind alle neun alten Steam-Bausteine (Verknüpfung, Freundes-Abgleich, Rang-Rollen, Aufräumer, Playtest-Trichter, Guard-Automatik, Token-Verwaltung) deaktiviert — der Code bleibt unangetastet liegen und kann im Notfall mit einer Zeile reaktiviert werden. Discord-seitig übernimmt der schlanke Brücken-Cog (#60/#61); die gesamte Logik läuft im Rust-Dienst.

Der erste Live-Abgleich unter neuer Führung: 284 Steam-Freunde geprüft, 281 Verknüpfungen bestätigt, keine fälschlich entfernt. Der stündliche Verified-Rollen-Abgleich und die Voice-Erinnerung laufen unverändert weiter — sie kollidieren nicht mit dem neuen Dienst.

## #61 — Steam-Brücke: Playtest-Funnel-UI + einheitliches Event-Format

Nachtrag zu #60: Der Brücken-Cog kann jetzt auch den Playtest-Einladungs-Ablauf bedienen — drei Slash-Befehle (/betainvite für alle, Panel-Veröffentlichung und Statistik für Admins) und alle Funnel-Buttons werden als Ereignisse an den Rust-Dienst weitergereicht, der sämtliche Texte und Entscheidungen liefert. Das Panel hängt seine Buttons weiterhin lokal an, damit sie nach Bot-Neustarts klickbar bleiben.

Außerdem wurde das Übertragungsformat zwischen Brücke und Rust-Dienst vereinheitlicht: Alle Ereignis-Arten (Klick, Server-Verlassen, Admin-Befehl, Slash-Befehl) senden ihre Daten jetzt in derselben verschachtelten Struktur. Vorher nutzten drei der vier Arten ein abweichendes flaches Format — die Gegenseite hätte sie kommentarlos abgelehnt.

## #60 — Discord-Arm für den Rust-Steam-Bot: Broker erweitert + neuer Brücken-Cog

Der Steam-Bot zieht nach Rust um — Discord bleibt aber beim Haupt-Bot. Damit der Rust-Dienst alle nötigen Discord-Aktionen auslösen kann, wurde der interne Vermittler (Master-Broker) um zwei Operationen erweitert: Rolle entfernen und Direktnachricht senden — beide mit derselben Absicherung wie die bestehenden Operationen (Token-Pflicht, Wiederholungsschutz über Idempotenz-Schlüssel, Guild-/Rollen-Whitelists).

Neu dazu kommt ein bewusst dünner Brücken-Cog: Er rendert die Steam-Verknüpfungs-Panels (auch ältere, bereits gepostete bleiben klickbar), öffnet das Freundescode-Eingabefenster, leitet jeden Klick, jedes Server-Verlassen und die Steam-Admin-Befehle als Ereignis an den Rust-Dienst weiter und zeigt dessen Antwort an. Er enthält selbst keinerlei Steam-Logik — fällt der Rust-Dienst aus, antwortet er mit einem freundlichen Hinweis statt zu crashen.

Aktiv wird das Ganze erst beim Umschalten: Dann werden die neun alten Steam-Bausteine über die Blockliste deaktiviert und der Rust-Dienst übernimmt.

## #59 — Coaching-Modal: Interaction-Timeout-Fix (Defer vor DB-Calls)

**Ausgangslage:** Wenn ein User das Coaching-Formular ausgefüllt und auf „Absenden" gedrückt hat, kam manchmal die Fehlermeldung „Beim Absenden der Anfrage ist ein Fehler aufgetreten". Das passierte weil der Bot nach dem Modal-Submit zunächst eine synchrone DB-Operation (`INSERT INTO coaching_requests`) auf dem Event-Loop ausgeführt hat — ohne die Discord-Interaction vorher zu bestätigen. Discord erwartet eine Antwort innerhalb von 3 Sekunden; wenn der DB-Call (z.B. durch Lock-Contention mit dem Background-Analyse-Task) auch nur kurz blockiert, läuft der Interaction-Token ab und jede nachfolgende `send_message`-Antwort schlägt mit 404 fehl.

**Geändert:** Der `on_submit`-Handler ruft jetzt sofort `interaction.response.defer(ephemeral=True)` auf, bevor irgendeine DB-Arbeit beginnt. Damit ist das 3-Sekunden-Fenster gesichert und der Bot hat anschließend bis zu 15 Minuten Zeit für die eigentliche Verarbeitung. Die Erfolgs- und Fehlermeldungen werden nun via `interaction.followup.send()` geschickt statt `interaction.response.send_message()`.

**Ergebnis:** Das Formular nimmt Anfragen zuverlässig entgegen, egal ob die DB kurz ausgelastet ist oder ein paralleler Analyse-Task läuft.

## #58 — Admin-Session-Validierung: Sliding TTL beim Cross-Dashboard-Check

**Ausgangslage:** Der interne `validate-session`-Endpoint (den der Twitch-Bot nutzt, um zu prüfen ob eine aktive Admin-Session im Discord-Bot existiert) griff direkt auf das Session-Dict zu und machte die Ablauf-Prüfung manuell. Dabei wurde die Session-Laufzeit weder verlängert noch der zentrale Cleanup-Pfad durchlaufen — jeder Zugriff lief am eigentlichen Session-Management vorbei.

**Geändert:** Der Handler nutzt jetzt `validate_discord_session()` statt des direkten Dict-Lookups. Das ist dieselbe Methode, die auch alle anderen Session-Zugriffspfade (Browser-Request, CSRF-Check etc.) verwenden.

**Wie es jetzt funktioniert:** Jede erfolgreiche Cross-Dashboard-Validierung verlängert die Session-Laufzeit wie ein normaler Zugriff. Gleichzeitig werden abgelaufene Einträge über den einheitlichen Cleanup-Pfad entfernt, statt still im Dict zu bleiben.

## #57 — Lobby-Finder: Channel-Name "unbekannt" gefixt + Trigger-Rauschen reduziert

**Ausgangslage:** Der Lobby-Finder zeigte in seinen Antworten statt der echten "Neue Spieler Lane" den Platzhalter "# unbekannt". Außerdem feuerte er auf Nachrichten wie "suche leute zum zocken, schreib mir bitte priv" — eine Ankündigung, keine Lobby-Anfrage.

**Geändert:** Die Channel-ID für die Neue-Spieler-Lane war in `lfg.py` noch auf den alten Wert von vor der adaptiven-Lane-Einführung gesetzt (`1465839460485697556`), während der tatsächliche Anchor-Channel eine andere ID hat (`1470126503252721845`). ID korrigiert. Zusätzlich wurde ein Negativ-Signal ganz an den Anfang der Intent-Erkennung gelegt: Nachrichten, die gleichzeitig "schreib/schreibt/meldet" und "priv/privat" enthalten, werden jetzt sofort übersprungen — der User will selbst koordinieren, nicht vom Bot weitergeleitet werden.

**Jetzt:** Channel-Mention zeigt den richtigen Namen. Nachrichten mit explizitem "schreib priv"-Muster lösen keine Lobby-Vorschläge mehr aus.

## #56 — Dependency-Updates: aiohttp 3.14.0 + protobufjs-Lücken geschlossen

**Ausgangslage:** Dependabot meldete 11 offene Sicherheitslücken: 4× aiohttp (medium, Cross-Origin-Redirect + Deserialisierung unsicherer Daten) und 7× protobufjs (bis high, u.a. Code-Injection via Byte-Felder, Prototype-Pollution, DoS durch rekursive Expansion).

**Geändert:** aiohttp von 3.13.5 auf 3.14.0 angehoben (`requirements.txt` + `.venv`). protobufjs im steam_presence-Modul via `npm install` + `npm audit fix` aktualisiert: Top-Level-Package von 8.0.2 auf 8.4.2, zusätzlich die verschachtelten Kopien in den transitiven Abhängigkeiten (`steam-user`, `steam-session`, `steam-appticket`) auf gepatchte Versionen gebracht.

**Ergebnis:** `npm audit` meldet 0 Vulnerabilities. Bot läuft weiter ohne Verhaltensänderung.

## #55 — Coaching: Freigeben-Button-Fix + echte Umlaute

**Ausgangslage:** Der „Freigeben"-Button im Coaching-Request-Embed hatte zwei Bugs. Erstens: Wenn eine Anfrage bereits für alle offen war (kein reservierter Coach), bekamen Nicht-Server-Owner die Meldung „Nur der reservierte Coach oder ein Admin kann freigeben" — statt der korrekten Info „bereits für alle offen". Die eigentliche Info-Meldung war für alle außer dem Guild-Owner toter Code. Zweitens: Als „Admin" galten ausschließlich der Guild-Owner und eine hardcoded User-ID — Discord-User mit `Administrator`-Permission wurden geblockt, obwohl die Fehlermeldung selbst von „ein Admin" spricht.

**Geändert:** Logik-Reihenfolge korrigiert: Der „bereits offen"-Check läuft jetzt zuerst, bevor der Permission-Check greift. Außerdem wurde `guild_permissions.administrator` in den Admin-Check aufgenommen, konsistent mit dem restlichen Codebase-Pattern (`website_invite_cog._is_owner_or_admin`). Zusätzlich: Alle 14 Pseudo-Umlaute (`fuer`, `ue`, `oe` etc.) in der Datei durch echte Umlaute ersetzt.

**Wie's funktioniert:** Klickt jemand Freigeben auf einer bereits offenen Anfrage, sieht er jetzt die korrekte Info — unabhängig von seiner Rolle. Klickt ein Discord-Admin (mit `Administrator`-Flag) auf eine reservierte Anfrage, kann er sie freigeben. Der assigned Coach kann weiterhin immer freigeben.

## #54 — Coaching: Session-Abschluss wird jetzt an die Website gespiegelt

**Ausgangslage:** Wenn ein Admin mit `/coaching-session-beenden` eine Session beendete, landete das nur in der Bot-DB. Die Coaching-Website zeigte weiterhin die Session als aktiv — weil der Bot nie den `/platform/sync`-Endpunkt aufrief.

**Geändert:** Der `coaching_survey`-Cog sendet nach dem Session-Update jetzt automatisch einen HTTP-Call an das Website-Backend (`/platform/sync` auf Port 8772). Payload enthält Request-ID, Discord-IDs, Coach-Name und `session_status: "completed"` + die Bot-Session-UUID.

**Wie's funktioniert:** Der interne Token (derselbe wie im restlichen Stack) authentifiziert den Call. Das Backend macht ein Upsert: Session wird auf `completed` gesetzt, der Timestamp landet in `completed_at`. Damit sehen Spieler auf `/coaching/me` ihre Session-Historie korrekt — inklusive dem Abschluss. Der Coaching-Flow in Discord bleibt unverändert.

## #53 — Master-Broker: Discord-Invites auf Anfrage erstellen

**Hintergrund:** Der Twitch-Bot läuft als separater Prozess und hat keinen eigenen Discord-Guild-Mitgliedsstatus — er kann daher keine Invites direkt über die Discord-API erstellen. Gelöst über den Master-Broker, der bereits für andere Discord-Aktionen (Nachrichten senden, Channels anlegen, Rollen vergeben) als interner Proxy fungiert.

**Geändert:** Neuer Endpunkt `POST /internal/master/v1/discord/create-invite` im Master-Broker. Nimmt `channel_id` und optionalen `reason`, löst den Channel über den Haupt-Bot auf und ruft `channel.create_invite(max_age=0, max_uses=0, unique=True)` auf. Antwort enthält `invite_url`, `code`, `channel_id` und `guild_id`. Auth und Idempotency-Handling laufen identisch zu den anderen Broker-Endpunkten.

## #52 — Fix: TempVoice-Interface lädt wieder korrekt

**Problem:** Durch die neuen Router-Buttons (Umbenennen + Modus wechseln) kam es beim Bot-Start zu einem Fehler, der das komplette TempVoice-Modul am Laden hinderte — alle Lane-Funktionen waren damit ausgefallen.

**Ursache:** Der Lurker-Button hat intern row 3 als Standard, was die Reihe bereits auf 4 Items brachte. Die zwei neuen Buttons auf row 3 machten 6 Items — Discord erlaubt maximal 5 pro Reihe.

**Geändert:** Die neuen Router-Buttons landen jetzt auf row 4, die in der Standard-Lane-Ansicht bisher leer war.

## #51 — Router: Smart-Routing bevorzugt bekannte Mitspieler

**Problem:** Der Router hat beim Suchen einer freien Lane einfach die erste passende genommen — ohne Rücksicht darauf, ob da jemand drin sitzt, mit dem man schon oft gespielt hat.

**Geändert:** Beim Smart-Routing (Auto-Join an) wird jetzt zuerst geprüft, ob in einer der passenden Lanes ein bekannter Mitspieler sitzt. Erst wenn keine solche Lane gefunden wird, greift der bisherige Fallback (erste freie Lane mit Platz).

**Wie's funktioniert:** Beim Join holt der Router die Top-20-Co-Player aus der bestehenden `user_co_players`-Tabelle (die trackt, wie viele gemeinsame Voice-Sessions zwei User hatten). Dann werden alle passenden Lanes durchsucht: Liegt die User-ID eines Mitglieds in dieser Co-Player-Liste, wird diese Lane priorisiert. Findet sich kein Co-Player, landet man wie gewohnt in der nächsten freien Lane.

## #50 — TempVoice: Neues Router-System mit Spielmodus-Wahl

**Problem:** Wer in einen Sprachkanal wollte, landete in einem von drei fixen Staging-Kanälen (Casual, Ranked, Street Brawl) — Modus-Wahl durch das Betreten des richtigen Kanals. Das war unflexibel: keine Möglichkeit den Modus zu ändern, keine smarte Verteilung in laufende Lanes, keine einheitliche Einstiegsstelle.

**Geändert:** Ein einzelner Router-Sprachkanal ersetzt die drei Eingänge für alle, die über diesen neuen Weg kommen wollen. Die alten Staging-Kanäle laufen unverändert weiter. Dazu gibt es einen Interface-Textkanal mit einer persistenten UI aus drei Modus-Buttons (Casual, Ranked, Street Brawl) und einem Auto-Join-Toggle.

**Wie's funktioniert:**

- **Modus wählen:** User klickt im Textkanal auf einen der drei Buttons — wird als Standard gespeichert und gilt für alle folgenden Joins.
- **Auto-Join aus (grau):** Beim Betreten des Router-VC wird sofort eine eigene private Lane im passenden Bereich erstellt.
- **Auto-Join an (grün):** Smart Routing — der Bot sucht eine bestehende Lane mit weniger als 6 Personen und demselben Modus. Findet er keine, wird eine neue erstellt.
- **Ranked-Gate:** Wer Ranked wählt, braucht einen verifizierten Rang (Steam-Verknüpfung). Ohne Rang kommt eine DM mit Link zum Info-Kanal, kein Move.
- **Neue Spieler:** Werden weiterhin automatisch in die New-Player-Lane umgeleitet, bevor das Modus-Routing greift — bestehende Logik unverändert.
- **Modus-Wechsel einer laufenden Lane:** Lane-Owner können ihren Kanal nachträglich auf einen anderen Modus (inkl. Off Topic) umstellen — der Kanal zieht in die passende Discord-Kategorie um. Dazu gibt es neue Buttons im Lane-Control-Interface.
- **Off-Topic-Lanes** landen in der Router-Kategorie, genau wie der bestehende permanente Off-Topic-Kanal.

## #49 — Coaching wird jetzt fair auf alle Coaches verteilt

**Problem:** Eingehende Coaching-Anfragen liefen nach dem „Wer zuerst klickt"-Prinzip — der erste Coach, der auf „Claimen" drückte, bekam die Session. In der Praxis griff dadurch meist immer derselbe Coach zu, während andere kaum drankamen; eine echte Verteilung gab es nicht. Zusätzlich lief im Hintergrund noch ein altes, längst totes Zweit-System mit fest eingetragener Coach-Liste mit, das die Lage nur unübersichtlicher machte.

**Geändert:** Neue Anfragen werden jetzt automatisch **fair** einem Coach zugewiesen und **24 Stunden exklusiv für ihn reserviert**. Es gibt einen **Freigeben-Button**, und nach Ablauf der 24 Stunden öffnet sich die Anfrage automatisch für alle. Wer Coach ist, ergibt sich dynamisch aus der Coach-Rolle — keine fest hinterlegte Namensliste mehr. Das alte parallele Claim-System wurde komplett entfernt.

**Wie's funktioniert:** Sobald eine Anfrage analysiert und gepostet wird, wählt der Bot aus allen Trägern der Coach-Rolle (der Server-Inhaber ist bewusst ausgenommen) den aus, der **am längsten nicht mehr an der Reihe war** — bei Gleichstand den mit den **wenigsten laufenden Sessions**. Dieser Coach steht sichtbar im Embed und hat 24 Stunden exklusiv Zeit zu übernehmen; andere Coaches sehen die Anfrage, können sie in diesem Fenster aber nicht claimen. Drückt der reservierte Coach (oder ein Admin) auf „Freigeben", oder laufen die 24 Stunden ab, wird die Anfrage für alle Coaches geöffnet — ein Hintergrund-Check im Minutentakt erledigt das Ablaufen automatisch und aktualisiert die Nachricht. Lässt sich gerade kein passender Coach ermitteln, ist die Anfrage sofort für alle offen, genau wie vorher.

**Betroffen:** Vor allem die Coaches (faire Reihenfolge statt Windhundprinzip). Für anfragende Spieler bleibt der Ablauf gleich — nur ist schneller klar, wer sich kümmert.

## #48 — Security Guard: MiniMax-Label erscheint jetzt nachträglich im Mod-Embed

**Problem:** Das MiniMax-Bild-Urteil lief blockierend *vor* dem Mod-Alert — wenn MiniMax länger brauchte (oder per Timeout abbrach bei 8 s), stand im ersten Post „nicht verfügbar" und der Bot wartete die ganze Zeit, bevor er überhaupt Timeout und Löschung ausführte.

**Geändert:** Alert wird sofort gepostet (mit „⏳ wird ermittelt…" als Platzhalter), Timeout + Löschung laufen direkt danach. MiniMax läuft als Hintergrund-Task mit 45 s Spielraum und aktualisiert das Embed per Edit, sobald das Urteil vorliegt.

**Wie's funktioniert:** `asyncio.create_task` startet den AI-Check entkoppelt vom Hauptpfad. Sobald MiniMax antwortet, sucht die Task das Feld „MiniMax-Einschätzung" im bereits geposteten Embed und überschreibt es. Schlägt das Edit fehl (Nachricht gelöscht, Bot-Rechte weg), wird das still ignoriert.

## #47 — Security Guard: Timeout-Bug gefixt, AI-Scam-Erkennung repariert, öffentliche Scam-Meldung

**Problem:** Der Security Guard hat Scam-Accounts zwar erkannt und den Alert im Mod-Channel gepostet, aber keine einzige Aktion ausgeführt — kein Timeout, keine Nachrichtenlöschung. Der Bot hat den Mod-Alert sogar doppelt gepostet (der Case erschien zweimal), weil der Crash den State zurückgesetzt und beim nächsten Trigger erneut ausgelöst hat. Dazu hat die AI immer "kein Scam" zurückgegeben, obwohl MiniMax intern bereits 100 % Scam erkannt hatte.

**Was war kaputt:**
- py-cord 2.7.1 kennt den Parameter `timed_out_until`, nicht `communication_disabled_until` (discord.py-mainline-Name). Jeder Timeout-Aufruf ist mit einem `TypeError` abgestürzt — ungebated, bis zu `discord.client` hochgereicht, wo er stillschweigend weggeloggt wurde. Weder Timeout noch Nachrichtenlöschung (die danach kamen) liefen je.
- `max_output_tokens=120` beim Scam-Text-Check war zu niedrig für Thinking-Modelle: MiniMax M3 hat alle 120 Token für den internen Denkprozess verbraucht, kein JSON-Response überlebt — die AI hat daher immer `False` zurückgegeben, egal wie eindeutig der Scam war.

**Geändert:**
- Timeout-Aufruf auf `timed_out_until` umgestellt (überall, inkl. Timeout-Aufhebung per Button).
- `max_output_tokens` für Scam-Text-Check von 120 auf 1500 angehoben, damit Thinking + JSON beide reinpassen.
- Nach jeder automatischen Scam-Aktion (Ban oder Timeout) postet der Bot jetzt kurz in den betroffenen Channel(s): `🔒 Scam erkannt — Account wurde automatisch gebannt/gesperrt.` — so sehen User, die den Scam gesehen haben, dass das Mod-System reagiert hat.

**Wie's jetzt funktioniert:** Scam erkannt → Beweis in Mod-Channel spiegeln → Timeout setzen (funktioniert jetzt) → Nachrichten löschen → öffentliche Kurzmeldung im betroffenen Channel → DM an User. Der AI-Text-Check liefert jetzt bei eindeutigen Scams (wie Fake-MrBeast-Crypto oder USDT-Withdrawal-Screenshots) korrekt `is_scam: true` mit hoher Confidence.

## #46 — Master-Dashboard: Login hält jetzt 2 Wochen statt 6 Stunden

**Problem:** Die Anmeldung am Master-Dashboard galt nur 6 Stunden. Da der Twitch-Admin-Bereich auf dieser zentralen Sitzung aufsetzt, fiel man dort regelmäßig nach wenigen Stunden raus — das Dashboard wirkte „tot": Oberfläche lädt, aber jede Aktion läuft ins Leere, weil die Sitzung im Hintergrund abgelaufen war.

**Geändert:** Die Standard-Lebensdauer der Master-Dashboard-Sitzung von 6 Stunden auf 14 Tage angehoben.

**Wie's funktioniert:** Die Sitzung gilt jetzt zwei Wochen und verlängert sich bei jeder Nutzung automatisch (Sliding-Refresh) — wer regelmäßig reinschaut, bleibt praktisch dauerhaft angemeldet. Der Wert ist weiterhin per Umgebungsvariable überschreibbar; der neue Standard greift, solange nichts anderes gesetzt ist. Einmal anmelden reicht damit für zwei Wochen statt mehrmals täglich neu einloggen zu müssen.

**Betroffen:** Login-Komfort im Admin-/Dashboard-Bereich; für normale Discord-Nutzer nichts sichtbar.

## #45 — Crypto-Scam: Jetzt automatisch gelöscht + Ban-Button für Mods

**Problem:** Ein Einzel-Scam (eine Nachricht, ein Kanal) wurde vom Bot erkannt (Confidence 0,97), aber weder automatisch gelöscht noch direkt gesperrt — weil er in keine der Auto-Delete-Kategorien fiel. Der Moderator konnte anschließend auch nicht direkt über den Bot bannen, da "Accept" nur einen 24h-Timeout ausgelöst und keine Ban-Option angeboten hat.

**Geändert:** `scam` als vollständige Kategorie im KI-Moderator eingeführt: im System-Prompt beschrieben, in die erlaubten Kategorien aufgenommen und in die Auto-Delete-Liste eingetragen. Zusätzlich ist ein neuer **Ban-Button** in jede Moderations-Review-Nachricht eingebaut worden.

**Wie's jetzt funktioniert:** Stuft die KI eine Nachricht mit `category=scam` und `verdict=delete` bei Confidence ≥ 0,90 ein, wird sie sofort gelöscht und der User automatisch 24h stummgeschaltet — ohne Mod-Interaktion. Liegt die Confidence zwischen 0,78 und 0,90, landet ein Vorschlag im Review-Kanal mit drei Buttons: **Accept** (Nachricht löschen + Timeout), **Ban** (Nachricht löschen + permanenter Serverausschluss) und **Deny** (Fall ablehnen). Der Ban-Pfad greift auch bei allen anderen Kategorien — nicht nur bei Scam.

## #44 — Gekaperte Stamm-Accounts: Scam-Bilder werden gestoppt

**Problem:** Ein langjähriges, etabliertes Mitglied wurde gehackt und hat in Sekunden Krypto-/Casino-Scam-Screenshots (gefälschte Auszahlungs-„Beweise", ein Fake-Promi-Giveaway, eine Casino-Bonusseite) über fünf, sechs Kanäle gestreut. Der Bot hat nicht reagiert — aus zwei Gründen. Erstens: Das harte Durchgreifen aus #43 galt nur für neue Accounts unter 30 Tagen; ein gekapertes Alt-Mitglied fiel komplett durch dieses Raster. Zweitens: Der Bild-Check hing an einer KI, die die Bilder technisch gar nicht „sehen" kann — das hier eingesetzte Modell ist reiner Text. Die in #43 erwähnte Bild-Mitlesung lief faktisch ins Leere, das Bild-Urteil stand deshalb immer auf „0 %".

**Geändert:** Ein neuer Schnellpfad speziell gegen Account-Übernahmen, der für **alle** Mitglieder gilt — ausdrücklich auch alte, etablierte (nur Mods sind ausgenommen). Und die Bilder werden jetzt von einem echten Bild-Verstehen-Dienst gelesen statt vom blinden Text-Modell.

**Wie's funktioniert:** Postet ein Account innerhalb von **30 Sekunden Bilder in mindestens zwei verschiedenen Kanälen** — der typische Fingerabdruck einer übernommenen Identität, die Werbung streut — greift der Bot sofort durch, ohne auf ein KI-Urteil zu warten (deterministisch, deshalb auch unabhängig davon, ob die KI etwas erkennt). Reihenfolge: erst die Bilder als Beweis in den Mod-Kanal kopieren (solange die Links noch leben), dann den Account **24 Stunden stummschalten** und sämtliche dieser Nachrichten kanalübergreifend löschen. Der Betroffene bekommt eine DM mit dem Hinweis, sein Account sei womöglich gehackt — Passwort ändern, 2FA aktivieren, beim Mod-Team melden. Die Mods sehen eine Übersicht mit den Bildern und zwei Buttons: **Bann** (endgültig) oder **Stummschaltung aufheben** (Fehlalarm). Die 24 Stunden sind bewusst umkehrbar gehalten.

**Warum die KI jetzt wirklich mitliest:** Zusätzlich schickt der Bot ein repräsentatives Bild durch ein echtes Bild-Verstehen. Das liefert den Mods jetzt ein konkretes Urteil im Alarm — z. B. „Scam, 100 % — gefälschtes Promi-Konto bewirbt ein Krypto-Casino-Giveaway" — statt der bisherigen stummen „0 %", weil das alte Modell die Bilder nie sehen konnte. Wichtig: Dieses Urteil ist nur noch ein Hinweis fürs Mod-Team, kein Auslöser — die Quarantäne läuft auch dann, wenn die KI gerade nichts sagt.

## #43 — Spam-Schutz greift jetzt selbst durch

**Problem:** Wenn ein neuer Account (jünger als 30 Tage) in kurzer Zeit dieselbe Werbung oder dieselben Bilder über mehrere Kanäle streut — das klassische Spam-Wellen-Muster — hat der Bot das zwar erkannt, aber nur den Mods gemeldet und dann gewartet. Die Spam-Nachrichten blieben stehen, bis jemand von Hand eingriff. Erkennung lief, Reaktion nicht.

**Geändert:** Der Bot greift jetzt beim Muster selbst durch, statt auf eine KI-Bestätigung zu warten — die kam vorher nämlich oft gar nicht (siehe unten).

**Wie's funktioniert:** Schlägt das Muster an (ab 3 Nachrichten in 3 Kanälen, oder schon ab 2 Kanälen mit Bildern/Scam-Wörtern; Mods und etablierte Accounts ausgenommen), läuft der Reihe nach: erst die Bilder als Beweis in den Mod-Kanal kopieren (bevor die Links durchs Löschen tot sind), dann die Nachrichten löschen und den Account **1 Stunde stummschalten**. Die Mods bekommen eine Übersicht mit Texten, Bildern und zwei Buttons — **Bann** oder **Timeout aufheben**. Die Stunde ist bewusst kurz und umkehrbar, falls es doch ein harmloser Neuling war. Die KI prüft Text und Bilder weiterhin, aber nur noch als Hinweis fürs Mod-Team, nicht als Auslöser.

**Warum vorher „0 %" dastand:** Die Text-Prüfung lief gegen einen KI-Dienst, der auf diesem Bot gar nicht eingerichtet ist (Zugangsschlüssel fehlt) — es kam nie eine Antwort zurück, und „keine Antwort" wurde als 0 % angezeigt. Jetzt läuft die Prüfung über den Dienst, der hier wirklich konfiguriert ist und auch die Bilder mitliest. Dessen interne „Denk"-Abschnitte werden vor der Auswertung herausgefiltert, weil das Ergebnis sonst beim Einlesen zerbrach.

## #42 — Direkte Channel-ID für interne API-Posts

- Interne Bot-Posts können jetzt einen beliebigen Discord-Kanal direkt ansprechen (statt nur "all"/"twitch")
- Wird genutzt für Moderations-Alerts aus dem Twitch-Bot

## #41 — Changelog-Posts direkt in Discord

- Neue Changelogs landen jetzt automatisch als Discord-Embed im Dev-Update-Kanal
- Twitch-Bot-Änderungen können zusätzlich gezielt im Twitch-Bot-Kanal gepostet werden
- Admin-interne Änderungen werden dabei bewusst weggelassen – nur was User sehen sollen kommt rein
- Admins können Einträge auch per `/changelog post` manuell auslösen

## #40 — Steam-Verifikation: Watchdog gegen stille Disconnects

- Ein neuer automatischer Watchdog überwacht jetzt dauerhaft ob der Steam-Bot eingeloggt ist
- Fällt die Steam-Verbindung lautlos weg (wie heute ~16 Uhr), erkennt der Watchdog das innerhalb von 30 Sekunden und startet den Bot nach 2 Minuten automatisch neu
- Vorher konnte eine solche Verbindungsunterbrechung unbemerkt über eine Stunde anhalten, weshalb Steam-Verifikationen in diesem Zeitraum fehlschlugen
- Betroffene User können die Verknüpfung jetzt erneut starten – sie wird wieder funktionieren

## #39 — FAQ-Bot kennt jetzt fast alle Server-Themen

- Der FAQ-Bot kann jetzt deutlich mehr Fragen beantworten: TempVoice und Lanes, Coaching, Onboarding & Beta-Invite, Steam-Verknüpfung, Twitch-Dashboard inkl. Tarife, Turniere, Patchnotes-Bot und unsere Websites
- Antworten zu Preisen und kostenlosen vs. bezahlten Features sind jetzt verlässlich, weil die zugrundeliegende Doku konkret und aktuell ist
- Veraltete, sich überschneidende Doku wurde archiviert, damit der Bot keine widersprüchlichen Aussagen mehr mischt

## #38 — Coaching-Infos überarbeitet und bereinigt

- Coaching ist jetzt überall klar als kostenlos und ohne Limit beschrieben
- Hinweis auf Feedback nach dem Coaching eingefügt – User werden gebeten ehrlich zu antworten
- Technische Details (Rollen-Zuweisung etc.) aus den Nutzer-Infos entfernt
- KI-Analyse erstellt keine Priorität mehr, sondern direkt nutzbare Fokuspunkte
- Alle alten Referenzen auf das nicht mehr existierende Coaching-Modul aus den Docs entfernt

## #37 — Coaching-Dokumentation und bessere KI-Antworten

- Der FAQ-Bot weiß jetzt zuverlässig wo und wie Coaching beantragt wird und verweist direkt auf den richtigen Channel
- Neue Coaching-Dokumentation hinterlegt, damit der Bot konkrete Antworten geben kann
- Die KI-Analyse bei Coaching-Anfragen listet jetzt direkt die Fokuspunkte für den Coach – ohne Wiederholung der User-Angaben
- Veraltete Referenz auf den alten Coaching-Bot entfernt

## #36 — Rang-Präferenz für Chill-Lanes

- Neuer Button „🎯 Mein Rang" im TempVoice-Interface (für alle Lane-Typen sichtbar)
- Über ein Dropdown-Menü kannst du Haupt-Rang und Sub-Rang auswählen und speichern
- Wenn du eine Chill-Lane erstellst, wird der gewählte Rang automatisch als Kanalname verwendet
- Dein Sub-Rang bestimmt außerdem die Sortierposition deiner Lane in der Kategorie

## #35 — Leere Sprachkanäle werden jetzt zuverlässig gelöscht

- Leere Lanes in der Ranked-Kategorie werden jetzt korrekt automatisch entfernt
- Channels bleiben nach einem Bot-Neustart nicht mehr dauerhaft erhalten
- Namen von laufenden Lanes werden nach einem Neustart nicht mehr vom Bot überschrieben

## #34 — Tag-Filter-Bestätigung nur noch für dich sichtbar

- Nach dem Speichern des Tag-Filters wird die Bestätigung nur noch dir angezeigt, nicht mehr im Channel

## #33 — Bestimmter Channel bleibt immer ganz unten in der Kategorie

- Ein festgelegter Voice-Channel wird nach jedem Neu-Sortieren automatisch ans Ende der Kategorie verschoben
- Er bleibt immer unter allen anderen Lanes, egal welche Ränge neu erstellt werden

## #32 — Channel aus TempVoice-Verwaltung ausgenommen

- Ein bestimmter Voice-Channel wird vom Bot nicht mehr umbenannt, gelöscht oder verschoben
- Der Channel bleibt weiterhin über die Verwaltungsinterfaces anpassbar

## #31 — Austritts-Umfrage: verstehen, warum Leute gehen

- Wenn jemand den Server verlässt, bekommt er automatisch eine freundliche Nachricht mit einer kurzen Umfrage — die Fragen passen sich an, je nachdem ob jemand neu war oder schon länger aktiv dabei
- Nach der ersten Antwort kommt eine gezielte Nachfrage, damit klarer wird, was genau das Problem war
- Wer ausführlicher Feedback geben will (auch mit Bildern), bekommt einen Link zu einer Feedback-Seite auf der Website
- Neue Auswertung im Admin-Dashboard zeigt, aus welchen Gründen Leute gehen und wie oft geantwortet wird
- Gebannte Mitglieder werden von der Umfrage ausgenommen

## #30 — Lobby-Finder treffsicherer und übersichtlicher

- Passt wirklich eine Lobby zu deinem Rang, wird dir gezielt die eine vorgeschlagen statt einer langen Liste
- Rang-Schnitt einer Lobby zeigt jetzt den echten Rang-Namen (z. B. Oracle) statt kryptischer Kürzel
- Aufgeräumteres Layout mit mehr Abstand, klarem Beitritts-Hinweis und Warnung wenn eine Lobby fast voll ist
- Falsche "über deinem Rang"-Warnung bei Lobbys unter deinem Rang behoben
- Erkennt mehr Nachrichten wie "jemand am start?" oder "noch wer wach?" und sortiert auch unverifizierte Rang-Rollen sauber ein

## #29 — Ehrliche Member-Herkunft im Admin-Dashboard

- Kreis-Diagramm "Wo kommen unsere Member her?" zeigt jetzt einen eigenen Website-Bereich statt Website-Joins in "Persönlich" zu verstecken
- Historische Joins ohne klare Quelle werden nicht mehr als "Twitch" geschätzt, sondern ehrlich als "Unbekannt" markiert
- Pro Website-Subseite gibt es jetzt einen eigenen Discord-Invite-Code (Landing, Streamer, Mitspieler, Coaching, Helden, Guides)
- Hover über Twitch- oder Website-Stück zeigt die Aufschlüsselung pro Streamer bzw. Subseite
- `/website-invite-recreate` rotiert jetzt gezielt nur eine ausgewählte Subseite

## #28 — Alle GitHub Code-Scanning-Alerts behoben

- Sicherheitslücke in der Node.js-Abhängigkeit protobufjs durch Paket-Override geschlossen
- XSS-Gefahr in der Aktivitätsstatistik-Seite behoben (HTML-Escaping für API-Daten)
- Integrity-Attribut für externes Chart.js-CDN-Skript hinzugefügt
- YAML-Syntaxfehler im Auto-Merge-Workflow behoben
- Log-Injection-Schwachstelle im Master-Broker gefixt (sanitisierte Log-Werte)
- Über 50 Code-Quality-Hinweise bereinigt (leere Except-Blöcke, ungenutzte Variablen, Import-Stil)
- 23 bestätigte False Positives (URL-Redirection, Cookie-Injection, SQL-Queries) als solche markiert und geschlossen

## #27 — Dependabot Auto-Merge wieder voll funktionsfähig

- Fehlende Hilfsdatei ergänzt, die den Python-Security-Scanner blockiert hat
- Syntaxfehler in der Auto-Merge-Workflow-Datei behoben (Actionlint-Fehler)
- Alle drei Ursachen, die Dependabot-PRs vom automatischen Merge abgehalten haben, sind beseitigt

## #26 — GitHub Actions Minutenverbrauch stark reduziert

- Neun tägliche Workflows auf wöchentliche oder event-basierte Trigger umgestellt
- Dashboard DAST und Auth-Guardrails laufen jetzt wöchentlich statt täglich
- Security-Scans, Secret-Scanning und Incident-Automation brauchen keinen täglichen Lauf mehr
- Semgrep meldet Findings als Artifact statt den Build zu blockieren

## #25 — SecurityGuard: Kein Auto-Ban ohne AI-Bestätigung

- Jeder Burst-Trigger wird jetzt zuerst von der KI geprüft — kein Ban ohne AI-Bestätigung
- Wenn die KI nicht sicher genug ist, landet der Fall nur als Vorschlag im Mod-Channel (inkl. Ban-Button für manuelle Entscheidung)
- Verhindert Fehlbans bei neuen Accounts die zufällig in mehreren Channels aktiv sind
- Bilder werden ebenfalls AI-geprüft wenn Text alleine nicht ausreicht

## #24 — SecurityGuard: Ban-Logs persistent in DB gespeichert + Review-Channel aktiv

- Jeder automatische Ban/Timeout wird jetzt dauerhaft in der Datenbank gespeichert — Fälle bleiben auch nach Bot-Neustart nachvollziehbar
- Die gespeicherten Daten umfassen Grund, betroffene Channels, Nachrichten-Snippets und Anhang-Anzahl
- Auch KI-erkannte Scam-Fälle bei etablierten Accounts (Timeout-Proposals) werden persistiert
- Ein dedizierter Review-Channel zeigt ab sofort alle Incidents als Embed an

## #23 — KI-Moderationskontext deutlich verbessert

- Kontext-Nachrichten zeigen jetzt relative Zeitstempel (z.B. "[2min ago]") — die KI erkennt ob der Kontext frisch oder veraltet ist
- Nachrichten vom selben User der bewertet wird, sind mit ">>>" markiert — so sieht die KI das Verhaltensmuster des Users klar
- Reply-Chain: wenn jemand auf eine Nachricht antwortet, bekommt die KI die Original-Nachricht direkt mit — Reaktionen werden im richtigen Kontext bewertet
- System-Prompt erklärt jetzt explizit: Reaktion auf eine Provokation wird milder bewertet als die Provokation selbst

## #22 — KI-Moderationsfilter entschärft: weniger False Positives

- Schwellenwert für Moderationsvorschläge von 55% auf 78% Konfidenz angehoben
- Die KI holt jetzt nur noch 12 statt 25 Nachrichten Kontext — verhindert, dass Kontext-Rauschen aus dem Gaming-Channel die Bewertung verfälscht
- System-Prompt präzisiert: Gaming-Klischees (Nationalitäten), kurze Ein-Wort-Antworten und einmalige Beleidigungen werden nicht mehr gemeldet
- Wichtig: Wer schlechtes Verhalten meldet oder kommentiert, wird nicht mehr selbst geflaggt

## #21 — Bild-Scam-Erkennung: KI analysiert Screenshots in mehreren Channels

- Accounts die nur Bilder senden (z.B. gefälschte X-Posts mit Investitions-Gewinnen) werden jetzt erkannt
- Sobald jemand Bilder in 2+ verschiedenen Channels schickt, prüft die KI automatisch ob es Scam ist
- Junge Accounts (<30 Tage) werden bei bestätigtem Bild-Scam direkt gebannt
- Ältere Accounts bekommen einen 24h-Timeout + DM und Mods sehen den Fall im Mod-Channel
- Message-Verlauf wird jetzt für alle Accounts getrackt, nicht nur für neue

## #20 — Klare Grenze: jung = bis 30 Tage, etabliert = älter als 30 Tage

- Accounts bis 30 Tage alt werden bei erkanntem Scam direkt gebannt
- Accounts älter als 30 Tage (möglicherweise gehackt) bekommen stattdessen einen 24h-Timeout und eine DM
- Der KI-Scam-Check läuft jetzt für alle Accounts, nicht mehr nur für neue
- Mehrkanal-Burst-Erkennung bleibt weiterhin auf Accounts unter 30 Tagen begrenzt

## #19 — Etablierte Scam-Accounts werden sofort getimeoutet und per DM informiert

- Accounts die älter sind und möglicherweise gehackt wurden, werden nicht mehr nur gemeldet, sondern sofort für 24h stummgeschaltet
- Der betroffene User bekommt automatisch eine DM: Grund, Dauer, und der Hinweis sich beim Mod-Team zu melden sobald der Account wieder sicher ist
- Mods sehen im Mod-Channel trotzdem ein Info-Embed mit zwei Buttons: "Ban" (eskalieren) oder "Timeout aufheben" (falls False Positive)
- Kein manuelles Bestätigen mehr nötig — der Bot handelt sofort

## #18 — Scam-Erkennung mit KI und erweiterter Account-Prüfung

- Scam-Nachrichten werden jetzt auch von Accounts erkannt, die bis zu einem Monat alt sind (vorher nur 24 Stunden)
- Einzelne Nachrichten mit verdächtigen Inhalten werden per KI automatisch auf Scam geprüft — kein Multi-Channel-Spam mehr nötig
- Neue Accounts werden bei KI-bestätigtem Scam automatisch gebannt
- Ältere, etablierte Accounts (möglicherweise gehackt) werden nicht automatisch gebannt — stattdessen erscheint ein Mod-Vorschlag mit Ban-Button im Mod-Channel
- Das bisherige Join-Zeitfenster als Bedingung wurde entfernt

## #17 — TempVoice greift nicht mehr in fremde Voice-Kategorien ein

- Channels außerhalb der TempVoice-Kategorien (Chill, Comp, Street Brawl) werden nicht mehr umbenannt oder gelöscht
- Betrifft z.B. Custom Games oder andere manuelle Voice-Channels
- Verhindert, dass Channels mit Namen wie "Lane 1" in falschen Kategorien fälschlicherweise als TempVoice-Lane erkannt werden

## #16 — FAQ-Bot antwortet automatisch in neuen Tickets

- Wenn in der Support-Kategorie ein neues Ticket aufgemacht wird, analysiert der FAQ-Bot die erste Nachricht des Users
- Hat der Bot eine passende Antwort aus der Dokumentation, antwortet er direkt im Ticket
- Kann der Bot das Problem nicht lösen, schreibt er gar nichts – der Mensch übernimmt dann wie gewohnt
- Gilt für alle Channels die mit "ticket-" beginnen in der ❓Support-Kategorie

## #15 — FAQ-Bot erkennt Onboarding- und Invite-Probleme automatisch

- FAQ-Bot gibt jetzt bei "kein Invite" oder "kann nicht herunterladen" sofort eine Schritt-für-Schritt-Anleitung aus
- Checkliste: Onboarding abgeschlossen? Richtige Option gewählt? Rollen gesetzt? /betainvite verwendet? Steam-Kauf vorhanden?
- Bot weist freundlich aber klar darauf hin wenn das Onboarding-Lesen das Problem gelöst hätte
- Neue Dokumentationsdatei mit dem vollständigen Invite-Ablauf für die Bot-Wissensbasis

## #14 — Tracking-Invite und Auswertung für Website-Joins

- Bot legt automatisch einen permanenten Tracking-Invite an und merkt sich den Code in der DB
- Neuer Slash-Befehl `/website-invite` zeigt Status, Code und bisherige Nutzungen
- `/website-invite-recreate` erzeugt bei Bedarf einen neuen Code (z.B. wenn jemand den alten löscht)
- `/join-quellen [tage]` aggregiert die Member-Joins der letzten N Tage nach Quelle (Website, Vanity, Twitch-Streamer, persönliche Einladungen)
- Channel-Default ist der Welcome-Channel, kann via `WEBSITE_INVITE_CHANNEL_ID` Env-Var überschrieben werden

## #13 — TempVoice Sweep löscht keine Staging-Channels mehr

- Staging-Channels sind jetzt gegen den automatischen Sweep geschützt
- Hintergrund: Der Bot hatte den alten Chill Lanes Staging-Channel selbst gelöscht, weil dessen Name einem Lane-Muster entsprach

## #12 — Chill Lanes Staging Channel auf neuen Channel aktualisiert

- Der Staging-Channel für Chill Lanes wurde nach dem Löschen des alten Channels auf den neuen Channel umgestellt
- Alle betroffenen Stellen im Code wurden aktualisiert: TempVoice, LFG und User-Retention-Links

## #11 — Steam-Verknüpfung: Link wird jetzt immer frisch beim Klick erstellt

- Der „Via Steam verknüpfen"-Button im Onboarding generiert den Login-Link jetzt erst beim Klicken – nicht mehr beim Laden der Nachricht
- Dadurch können Links nicht mehr ablaufen, bevor jemand draufklickt
- Das Problem „invalid Launch" bei der Steam-Verifizierung ist damit behoben

## #10 — Vollständige Security-Fortress hinzugefügt

- Neuer Security-Scan läuft täglich: prüft Workflow-Integrität, findet Secrets im Code, scannt Python auf Sicherheitslücken und bekannte CVEs in Dependencies
- Alle Workflow-Dateien sind jetzt auf genaue Commit-Hashes gepinnt — kein Supply-Chain-Angriff über gemutete Action-Tags möglich
- JavaScript-Abhängigkeiten werden auf bekannte Schwachstellen geprüft
- Security-Fortress ergänzt den bestehenden Deep-Scan sinnvoll, ohne Laufzeit zu verschwenden

## #9 — Dependabot-PRs werden jetzt automatisch gemerged + CI-Laufzeiten halbiert

- Dependabot-PRs werden ab jetzt automatisch approved und direkt gemerged (nicht mehr blockiert durch Lint oder DAST)
- Lint und DAST-Scans überspringen Dependabot-PRs, da sie nur Dependency-Dateien ändern — kein Sicherheitsverlust
- Security-Scans laufen weiterhin täglich; Container/IaC/Supply-Chain scannen jetzt wöchentlich statt täglich
- Security-Incident-Automation läuft jetzt täglich statt alle 6 Stunden — 75 % weniger Runs
- Dependency-Review hat keinen sinnlosen Tages-Schedule mehr (läuft weiterhin auf jedem PR)

## #8 — CI-Artifacts werden nach 30 Tagen automatisch gelöscht

- Alle automatisch erzeugten CI-Berichte (Security-Scans, Performance-Reports, Logs) werden ab jetzt nach 30 Tagen automatisch von GitHub entfernt
- Verhindert, dass sich der GitHub-Actions-Speicher dauerhaft volläuft

## #7 — Steam-Bot startet wieder und updated Ränge

- Steam-Bot lief seit dem 27. April nicht mehr — FAs annehmen/senden und Rang-Updates funktionierten nicht
- Ursache: Drei kombinierte Bugs verhinderten den Start (falscher Pfad auf Linux, fehlende Env-Variablen für den Node-Prozess, veraltete native Module)
- Der Bot läuft jetzt stabil und verarbeitet wieder Rang-Checks und Freundschaftsanfragen

## #6 — DB-Pfad fest im Code, kein Datenverlust mehr bei Neustarts

- Der Bot nutzt jetzt immer `data/deadlock.sqlite3` direkt im Repo — egal welche Umgebungsvariablen gesetzt sind oder nicht
- Davor: Nach der Token-Rotation fehlte die `DEADLOCK_DB_PATH`-Variable → Bot startete mit einer leeren Fallback-DB → alle User wirkten wie Neulinge (kein Voice-Verlauf, keine Steam-Links)
- Die fehlenden 2.5 Tage Daten (276 Voice-Sessions, Steam-Links, Nudge-Status etc.) wurden in die Haupt-DB zurückgespielt

## #5 — Steam-Nudge-DM geht nicht mehr mehrfach an denselben User

- Wenn die ursprüngliche Nudge-DM gelöscht wurde (z. B. vom User selbst), schickt der Bot keine zweite DM mehr — die Nachricht ist weg, die Benachrichtigung bleibt trotzdem gesetzt
- Fehlschläge beim Speichern des „bereits benachrichtigt"-Flags werden jetzt im Log sichtbar, statt still ignoriert zu werden

## #4 — Tag-System: bessere Sortierung in Voice-Lanes und LFG

- Du kannst dir jetzt selbst zwei Tags setzen: deinen Lieblings-Ton (Banter-OK oder Ragebaiter-Free) und optional eine Altersangabe (25+ oder U25)
- Setzen geht entweder direkt im Onboarding nach dem Server-Join oder jederzeit per `/meine-tags`
- Voice-Lane-Owner können in ihrer Lane einen 🛡️ Tag-Filter setzen, damit nur Leute mit passendem Ton oder Alter joinen können
- Wer wiederholt Ragebait fährt, bekommt automatisch einen Ragebaiter-Mod-Tag (14 Tage), der ihn aus Ragebaiter-Free-Lanes raushält — Mods können den Tag jederzeit anpassen
- LFG-Suche kann jetzt auch nach Tags filtern, damit Mitspieler besser zur eigenen Stimmung passen

## #3 — Tierlist-Backend: WR-Daten alle 8 Stunden automatisch

- Neuer Service liefert die Hero-Tierliste der Website mit Live-Winrates pro Skill-Bucket
- Daten werden alle 8 Stunden automatisch aktualisiert, immer auf Basis des aktuellen Patches
- Drei Skill-Buckets verfügbar: All, Phantom+, Eternus
- Admin-Endpunkte für Beschreibungen, Streamer-Listen und Schwellen — Login über bestehenden Discord-Flow
- Build-Voting (👍 / 👎) mit Rate-Limit pro Browser

## #2 — Voice Feedback geht nicht mehr an bestehende User nach Bot-Neustart

- Nutzer, die beim Neustart bereits im Voice-Call saßen, bekommen kein fälschliches „erstes Mal"-Feedback mehr
- Prüfung erweitert: beide Tabellen (voice_stats und voice_session_log) werden gecheckt, nicht nur eine
- Feedback-Retry bei DMs-deaktiviert läuft jetzt nicht mehr ewig: der ursprüngliche Zeitstempel bleibt erhalten und fällt nach 72 Stunden aus dem Fenster

## #1 — Sicherheitslücke: Bot-API nicht mehr von außen erreichbar

- Der interne Statistik-Server (Port 8768) war versehentlich von außen direkt erreichbar
- Jetzt lauscht er nur noch auf localhost — externer Zugriff ohne Caddy nicht mehr möglich
