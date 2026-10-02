# Community-Punkte und Streamer

## Worum geht es?
Wir bringen die Community und unsere Partner-Streamer zusammen. Wer sein Discord mit Twitch verknüpft, sammelt Punkte, wenn er bei unseren Partner-Streamern zuschaut und mitchattet. Diese Punkte landen zusammen mit den Voice-Punkten in einem gemeinsamen Leaderboard. Jede Woche gibt es einen Clip-Contest mit Abstimmung der Community, die Top 3 laufen im Stream auf dach_lock. Außerdem kannst du Streamer für unser Streamer-Programm vorschlagen. Der Concierge erklärt das alles schon bei der Server-Tour und beantwortet Fragen dazu direkt.

## Wie nutze ich das?
- **Twitch verknüpfen:** Klick im Concierge auf `Twitch verknüpfen` oder in <#1398021105339334666> auf den gleichnamigen Knopf. Discord fragt dich, ob wir deine verknüpften Konten sehen dürfen. Wir lesen daraus nur dein Twitch-Konto. Twitch muss vorher in deinen Discord-Einstellungen unter Verbindungen hinzugefügt sein, sonst gibt es nichts zu finden.
- **Zuschauen:** 1 Punkt für alle vollen 5 Minuten, die du bei einem Partner zuschaust, während er live ist. Pro Kanal zählen höchstens 72 Punkte am Tag (6 Stunden), über alle Kanäle zusammen höchstens 144 Punkte am Tag.
- **Mitchatten:** 1 Punkt pro echter Nachricht im Chat eines Partners. Eine Nachricht zählt, wenn sie mindestens 10 Zeichen hat, kein Befehl mit `!` ist, nicht dasselbe wie deine letzte Nachricht in diesem Kanal ist und seit deiner letzten zählenden Nachricht dort mindestens eine Minute vergangen ist. Pro Kanal höchstens 30 Punkte am Tag.
- **Entdecken:** Schaust du zum ersten Mal bei einem Partner rein, bei dem du noch nie warst, gibt es einmalig 10 Punkte extra. Das geht höchstens 3 mal am Tag.
- **Voice:** Voice-Punkte sammelst du wie bisher, wenn du mit anderen im Voice aktiv bist. `!vstats` zeigt deinen Stand.
- **Clip-Contest:** Jede Woche werden Clips aus den Streams unserer Partner eingereicht und die Community stimmt in Discord ab. Die Top 3 laufen im Stream auf dach_lock. Platz 1 bringt 100, Platz 2 bringt 60, Platz 3 bringt 40 Punkte für die Person, die den Clip eingereicht hat. Wer abstimmt, bekommt 2 Punkte, dabei zählt eine Stimme pro Woche.
- **Streamer vorschlagen:** In <#1425215762460835931> steht unter den Clip-Regeln der Knopf `Streamer vorschlagen`. Im Fenster gibst du den Twitch-Kanal an, als Namen oder als Link (zum Beispiel `twitch.tv/name`), und schreibst kurz, warum er zu uns passt. Du bekommst sofort eine Antwort, die nur du siehst: Danke und wir schauen uns den Kanal an, der Kanal ist schon bekannt, er ist schon Partner, wir finden ihn nicht (dann Schreibweise prüfen) oder wir können ihn leider nicht aufnehmen. Jeden Kanal kannst du einmal vorschlagen, insgesamt bis zu 3 Kanäle in 24 Stunden. Das Team schaut sich jeden Vorschlag an und entscheidet selbst, wen es anspricht. Wird der Kanal Partner, bekommt die Person, die ihn als Erste vorgeschlagen hat, 150 Punkte.
- **Für Partner-Streamer:** 1 Punkt für alle 30 Minuten, die verknüpfte Community-Mitglieder bei dir zuschauen, 25 Punkte für jeden Raid an einen anderen Partner, 50 Punkte für jede Person, die über deine Einladung auf den Server kommt und hier wirklich aktiv wird, und 100, 60 oder 40 Punkte für die Plätze 1 bis 3 im Clip-Contest. Dafür gibt es ein eigenes Streamer-Leaderboard.

Der Tag zählt immer nach deutscher Zeit.

### Befehle
- `!punkte`: deine Punkte in der laufenden Season (Kalendermonat), aufgeteilt nach Voice, Zuschauen, Chat, Entdecken, Clip-Contest und Streamer-Vorschlägen, dazu dein Platz in der Season und insgesamt und deine Punkte dieser Woche. Bist du noch nicht mit Twitch verknüpft, steht dort, wie das geht.
- `!lb` oder `!leaderboard`: das gemeinsame Leaderboard der laufenden Season. `!lb gesamt` zeigt alle Punkte seit Beginn, `!lb woche` die laufende Woche ab Montag.
- `!streamerlb`: das Leaderboard der Partner-Streamer, ebenfalls mit `gesamt` oder `woche`.
- `!vstats` und `!vleaderboard` (`!vlb`, `!voicetop`) zeigen wie bisher nur die Voice-Punkte.

Eine Season ist ein Kalendermonat. Die Befehle haben dieselbe Bremse wie `!vstats`: höchstens 5 Abfragen in 30 Sekunden.

## Kosten / Premium
kostenlos. Punkte sind Anerkennung aus der Community, kein Geld und nichts zum Eintauschen.

## Was passiert technisch (kurz)?
Der Twitch-Bot zählt Zuschauzeit und Chat in den Kanälen aktiver Partner, solange sie live sind. Zugerechnet wird das erst, wenn ein Twitch-Konto fest mit einer Discord-ID verknüpft ist. Diese Verknüpfung kommt nur aus der Freigabe über Discord, niemals aus einem ähnlichen Namen. Die Punkte werden alle 10 Minuten in die zentrale Datenbank übernommen und dort mit Voice-Punkten, Clip-Contest und Streamer-Vorschlägen zu einem gemeinsamen Leaderboard zusammengeführt. Der Concierge beantwortet die Fragen "Wie bekomme ich Punkte?", "Wie verknüpfe ich Twitch?", "Wie funktioniert der Clip-Contest?", "Wie schlage ich einen Streamer vor?" und "Was bringt mir das als Streamer?" mit festen Antworten, auch wenn kein Sprachmodell erreichbar ist.

## Grenzen & häufige Fragen
- Ohne Verknüpfung wird dir nichts zugerechnet, und dein Name taucht nirgends auf. Sichtbar sind nur verknüpfte Mitglieder.
- Deine Datenschutz-Einstellungen gelten auch für dieses Leaderboard, so wie für die anderen Ranglisten auf dem Server.
- Follows und Subs bringen keine Punkte. Wir kaufen keine Views und belohnen nichts, was gegen die Regeln von Twitch verstößt.
- Gezählt wird nur bei Partner-Streamern, während sie live sind. Wer im eigenen Kanal streamt, bekommt dort keine Zuschauerpunkte. Bot-Konten und gesperrte Konten zählen nicht.
- Wenn nach dem Klick auf `Twitch verknüpfen` nichts gefunden wird: In Discord unter Einstellungen, Verbindungen Twitch hinzufügen und es dann nochmal versuchen.
- Ein Vorschlag ist keine Zusage. Ob ein Kanal Partner wird, entscheidet das Team.
- Ist der Twitch-Bot gerade nicht erreichbar, geht dein Vorschlag nicht verloren. Er ist gespeichert und wird automatisch nachgereicht.
- Wenn du einen Löschantrag stellst, werden auch deine Vorschläge gelöscht. Punkte für einen späteren Partner gibt es dann nicht mehr.
