# Server-FAQ V2 — finale Texte (Claude, 2026-07-05)

Quelle: docs/onboarding-redesign/phase1-wissensbasis.ENTWURF.md F1–F15,
redigiert auf den Live-Stand 2026-07-05. Bewusst ENTFERNT (nicht live):
Bot-Pate/Paten-Angebot (Phase 3/4), Status-Abfrage per Bot-DM, 24h-Erinnerung,
Cockpit-Meldung bei persönlichen Invites, „einziges Button-Panel", Rank-up-Feiern.
AKTUALISIERT: Mitspieler → LFG-Forum 1522769149208821881, Lanes → Router-Panel
in 1513468476365209670, Ranked-Gate = Steam-Verify-Nutzen.

## Panel (Haupt-Message im Kanal 1483136301271355532 server-support)

[Titel] **❓ Server-FAQ · Deutsche Deadlock Community**
[Body] Wähl unten deine Frage aus — die Antwort siehst nur du.
Nichts Passendes dabei? Stell deine Frage in <#1426220702054355077> oder mach hier ein Ticket auf.

[Select placeholder] Wähl deine Frage …

## Select-Optionen (custom_id `faq:show`, value → Sektion)

| value | Label (Select-Option) | Emoji |
|-------|----------------------|-------|
| f1  | Wie bekomme ich einen Deadlock-Invite? | 🔑 |
| f2  | Muss ich dem Bot eine Steam-Freundschaftsanfrage schicken? | 🤝 |
| f3  | Wie lange dauert mein Invite? | ⏳ |
| f4  | Mein Invite hängt — was tun? | 🛠️ |
| f5  | Kann mich ein Mensch direkt einladen? | 💌 |
| f6  | Steam sagt „limited account" — warum kein Invite? | 🚧 |
| f7  | Wie verknüpfe ich meinen Steam-Account? | 🔗 |
| f8  | Was passiert bei der Verifizierung mit meinen Daten? | 🛡️ |
| f9  | Wie bekomme oder ändere ich meine Rang-Rolle? | 🏆 |
| f10 | Ich bin ganz neu — hilft mir jemand? | 🌱 |
| f11 | Wie funktioniert das kostenlose Coaching? | 🎓 |
| f12 | Wo finde ich Mitspieler? | 🎯 |
| f13 | Wie funktionieren die Voice-Lanes? | 🎙️ |
| f14 | Pings & Benachrichtigungen einstellen | 🔔 |
| f15 | Wo fange ich an — und wo kann ich fragen? | 🧭 |

## Antworten (ephemeral, V2-Gold-Container; jeweils Titel = Frage fett)

[f1] **🔑 Wie bekomme ich einen Deadlock-Invite?**
Der Weg ist kurz: Verknüpf deinen Steam-Account in <#1398021105339334666> (der Guide dort führt dich durch) und nimm danach die Freundschaftsanfrage unseres Steam-Bots an — dein Invite kommt dann automatisch. Falls du <#1464736918951432222> nicht siehst: Wähl im „Kanäle & Rollen"-Tab „Ich hab Deadlock noch nicht" aus, dann taucht der Kanal auf.

[f2] **🤝 Muss ich dem Bot eine Steam-Freundschaftsanfrage schicken?**
Normalerweise nicht — nach dem Verknüpfen schickt unser Bot **dir** eine Anfrage, du musst sie auf Steam nur annehmen. Kam nichts an? Dann geh den Weg selbst: Steam → Freunde → „Freund hinzufügen" → Freundescode **820142646** eingeben — damit findest du unseren Bot eindeutig, unabhängig vom Anzeigenamen. Sobald die Freundschaft steht, läuft dein Invite automatisch weiter.

[f3] **⏳ Wie lange dauert mein Invite?**
Sobald deine Steam-Freundschaft mit dem Bot bestätigt ist, dauert es in der Regel nur ein paar Stunden — oft schneller. Unsicher, ob alles durch ist? Über das Panel in <#1398021105339334666> kannst du deine Verknüpfung jederzeit neu prüfen lassen. Wenn es deutlich länger hängt, schau in „Mein Invite hängt — was tun?".

[f4] **🛠️ Mein Invite hängt — was tun?**
Fast immer liegt es an einem von drei Punkten: Steam ist noch nicht verknüpft (<#1398021105339334666>), die Freundschaftsanfrage des Bots wurde auf Steam noch nicht angenommen, oder dein Steam-Account ist „limited" (siehe die Frage dazu). Geh die drei kurz durch — wenn es dann immer noch klemmt, schreib in <#1426220702054355077> oder mach hier ein Ticket auf: Da schaut ein Mensch drauf.

[f5] **💌 Kann mich ein Mensch direkt einladen?**
Ja — Mitglieder mit verknüpftem Steam-Account können Freunde persönlich einladen. Frag am besten direkt die Person, die dich hergeholt hat, oder schreib in <#1426220702054355077>. Und wenn gerade kein Mensch greifbar ist: Der Bot übernimmt automatisch, du musst nichts extra tun.

[f6] **🚧 Steam sagt „limited account" — warum kein Invite?**
Das ist eine Beschränkung von Valve, kein Fehler bei uns: „Limited" sind Steam-Accounts, die noch nie mindestens 5 $ im Steam-Store ausgegeben haben — solche Accounts können über unseren Weg keine Playtest-Einladung erhalten. Sobald du einmalig für 5 $ irgendwas auf Steam gekauft hast, fällt die Sperre weg. Wenn du unsicher bist, mach ein Ticket auf — das Team geht die Optionen mit dir durch.

[f7] **🔗 Wie verknüpfe ich meinen Steam-Account?**
Geh in <#1398021105339334666> — der Guide dort führt dich Schritt für Schritt durch: Button klicken, auf der offiziellen Steam-Seite einloggen, Freundschaftsanfrage des Bots annehmen, fertig. Über dasselbe Panel kannst du den Stand jederzeit neu prüfen lassen.

[f8] **🛡️ Was passiert bei der Verifizierung mit meinen Daten?**
Was du davon hast: deine echte Rang-Rolle (hält sich ab dann von selbst aktuell), Zugang zu den Ranked-Funktionen (Ranked-Lanes öffnen und Ranked-Gesuchen beitreten) und du kannst Freunde per Invite reinholen. Zur Technik: Die Verknüpfung läuft über den offiziellen Steam-Login (OpenID) — wir sehen nie dein Passwort und haben keinerlei Zugriff auf deinen Account, wir bekommen nur deine Steam-ID. Deine Daten kannst du jederzeit exportieren oder löschen lassen.

[f9] **🏆 Wie bekomme oder ändere ich meine Rang-Rolle?**
Zwei Stufen: Beim Onboarding gibst du deinen Rang selbst an und bekommst die passende Rolle — das kannst du jederzeit im „Kanäle & Rollen"-Tab ändern. Verknüpfst du zusätzlich deinen Steam-Account in <#1398021105339334666>, bekommst du deine echte Rang-Rolle aus dem Spiel, die sich ab dann automatisch aktuell hält — da musst du nie wieder etwas anfassen.

[f10] **🌱 Ich bin ganz neu — hilft mir jemand?**
Ja, genau dafür sind wir da: In der Neue-Spieler-Lane erwartet niemand, dass du schon irgendwas kannst — spring einfach rein. Dazu gibt's unser komplett kostenloses Coaching (siehe die Frage dazu), und in <#1426220702054355077> ist keine Frage zu einfach. Sag dazu, dass du neu bist — dann holen dich alle da ab, wo du stehst.

[f11] **🎓 Wie funktioniert das kostenlose Coaching?**
Komplett kostenlos: Erfahrene Spieler aus der Community nehmen sich Zeit für dich — von den Grundlagen bis zum Rang-Aufstieg. Anmelden kannst du dich über den Coaching-Bereich auf unserer Website (https://deutsche-deadlock-community.de/coaching); deine Anfrage landet direkt beim Coach-Team hier im Discord und ein Coach übernimmt sie.

[f12] **🎯 Wo finde ich Mitspieler?**
Zwei Wege: Poste ein Gesuch in <#1522769149208821881> — Modus, Rang-Bereich und Spielzeit wählst du einfach per Klick, und mit 🔔 kannst du dich benachrichtigen lassen, sobald ein passendes Gesuch reinkommt. Oder spring direkt in eine Voice-Lane: Über das Panel in <#1513468476365209670> öffnest du eine Lane für Normale Lane, Ranked oder Street Brawl (für Ranked brauchst du die Steam-Verknüpfung). Reinsetzen ist ausdrücklich erlaubt — du musst niemanden um Erlaubnis fragen.

[f13] **🎙️ Wie funktionieren die Voice-Lanes?**
Du darfst in jede rein — unsere Lanes haben keine Türsteher. Wer eine Lane öffnet, wählt den Modus; bei Ranked ist der Rang-Bereich eine Ansage, kein Schloss. Die Regel dahinter ist einfach: Der Lane-Ersteller entscheidet, wer bleibt — bei Streit entscheiden die Mods. Die Chill-Lanes sind unser rang-egales Wohnzimmer, die Neue-Spieler-Lane ist für alle, die gerade erst anfangen. Nur zuhören ist übrigens auch völlig okay.

[f14] **🔔 Pings & Benachrichtigungen einstellen**
Oben links über dem Kanalbaum findest du „Kanäle & Rollen" — dort schaltest du Benachrichtigungs-Rollen selbst an oder aus und kannst auch deine Onboarding-Angaben jederzeit ändern (das sind reine Angaben über dich, keine Berechtigungen). Für einzelne Kanäle gilt der Discord-Standard: Rechtsklick auf den Kanal → Benachrichtigungen anpassen.

[f15] **🧭 Wo fange ich an — und wo kann ich fragen?**
Folg einfach der Server-Guide-Checkliste oben im Kanalbaum: Sag Hallo in <#1426220702054355077>, verknüpf deinen Steam-Account, such dir Mitspieler — in der Reihenfolge, ohne Zeitdruck. Und für alles andere gilt: <#1426220702054355077> ist genau dafür da. Es gibt keine dummen Fragen, und hier antworten dir echte Menschen — normalerweise noch am selben Tag.
