# Concierge — Textpaket v1 (final, von Claude)

Alle user-sichtbaren Texte für Slice A. Codex übernimmt diese Texte
**wörtlich**. Fehlt ein Text, wird `"Platzhalter"` eingebaut und gemeldet,
Codex textet nicht selbst. Kanal-Erwähnungen als echte `<#ID>`-Mentions.
Stilregeln: du, warm, kurz, Punkt und Komma, keine Gedankenstriche,
höchstens ein `:)` pro Nachricht, keine AI-Floskeln.

## 1. T0-DM (statisches Template, ohne LLM)

**V2-Layout:** Gold-Banner oben, darunter Text, darunter drei Buttons.

> Hey, schön dass du da bist. Ich bin der Concierge hier auf dem Server,
> ich helf dir beim Ankommen.
>
> Erzähl mir kurz, was du hier vorhast, dann zeig ich dir den schnellsten
> Weg dahin. Egal ob du Mitspieler suchst, besser werden willst oder dich
> erstmal nur umschauen magst, schreib es mir einfach in deinen Worten.
>
> Ich merk mir, was wir besprechen, damit ich nicht zweimal frage. Wenn du
> das nicht willst, sag einfach stopp, dann lass ich dich in Ruhe.

Buttons: `Zeig mir den Server` (Tour), `Ich will direkt spielen`,
`Später`.

Variante mit verknüpftem Rang (eine Zeile zusätzlich nach dem ersten
Absatz):

> Deinen Rang hab ich schon gesehen, das macht es gleich einfacher.

`Später`-Klick, Antwort:

> Alles gut, lass dir Zeit. Wenn du mich brauchst, schreib mir einfach,
> ich bin immer da.

## 2. Tour (eine V2-Nachricht, 6 Stationen)

Einleitung:

> Gern, hier die kleine Roomtour. Das sind die Ecken, die sich am Anfang
> lohnen.

Stationen (je Sektion, mit Kanal-Mention):

1. <#1326973956825284628>
   > Hier landen alle Patchnotes auf Deutsch, direkt aufbereitet. Ein
   > Blick vor der ersten Runde lohnt sich.
2. <#1304169815505637458>
   > Sag doch mal hallo oder lurk bei unseren Streamer-Partnern rein.
   > Da ist eigentlich immer wer live.
3. <#1426220702054355077>
   > Stell hier alle deine Fragen zu Deadlock, egal wie basic. Und wenn
   > du das Spiel noch gar nicht hast, lässt du dich hier ins Game
   > inviten.
4. <#1494373349944459355>
   > Du willst, dass dir jemand beim Einstieg hilft? Dann stell hier
   > deine Coaching-Anfrage, unsere Coaches machen das gern.
5. <#1513468476365209670>
   > Hier stellst du dein Preset ein, also was und wie du gern spielen
   > willst.
6. Station Voice (ohne Mention, Name fett):
   > Danach joinst du einfach den **Deadlock Router**. Der verteilt dich
   > automatisch in eine passende Lane oder macht dir eine eigene auf.

Abschluss (leitet in den Steckbrief):

> Das war die Tour. Wenn du magst, stell ich dich den anderen kurz vor,
> dann musst du nicht den ersten Schritt machen. Ich schreib dir was
> vor, du änderst es wie du willst, und gepostet wird nur, wenn du es
> freigibst.

Buttons: `Ja, schreib was vor`, `Lieber nicht`.

`Lieber nicht`-Antwort:

> Kein Ding. Falls du es dir anders überlegst, sag einfach Bescheid.
> Die anderen beißen nicht, versprochen :)

## 3. Steckbrief

Gerüst (das LLM füllt aus Gespräch, Rang und Intent, Ich-Form, 2 bis 4
kurze Sätze, endet immer mit einer konkreten Aufforderung an die
Community):

- Satz 1: Wer grob und was er spielt (Rang nur wenn verknüpft).
- Satz 2: Was er hier erreichen will (Ziel aus dem Gespräch).
- Satz 3 optional: Wann er meistens online ist.
- Schluss: Aufforderung (mitspielen oder helfen), locker formuliert.

Beispiel 1 (Intent mates, Rang verknüpft):

> Hey, bin neu hier. Spiele meistens abends, aktuell Archon, hauptsächlich
> Ranked. Suche Leute, mit denen man regelmäßig zocken kann. Wer hat Bock,
> mich mitzunehmen?

Beispiel 2 (Intent learn, kein Rang):

> Hi, ich fange gerade erst mit Deadlock an und will das Spiel richtig
> lernen. Wenn mich wer an die Hand nimmt oder einfach eine Runde mit mir
> dreht, wäre das top.

Preview-Rahmen in der DM (über dem Entwurf):

> So könntest du dich vorstellen. Das ist nur ein Vorschlag, mach deins
> draus. Gepostet wird erst, wenn du auf Posten drückst.

Buttons: `Posten`, `Anpassen`, `Lieber nicht`.

Routing-Hinweis, je nach Ziel-Kanal eine der beiden Zeilen unter dem
Entwurf:

> Der Post geht in <#1426220702054355077>, da schauen die richtigen Leute
> rein.

> Der Post geht in <#1289721245281292291>, mitten ins Geschehen.

Presence-Halteinfo (wenn der Server gerade ruhig ist):

> Gerade ist hier wenig los. Ich poste deine Vorstellung, sobald wieder
> Leute unterwegs sind, dann geht sie nicht unter. Du musst nichts weiter
> tun.

## 4. Erstreaktion des Concierge auf den geposteten Steckbrief

Reaktion: 👋 plus ein `dl_*`-Brand-Emoji.

Reply (kurz, macht die Tür für andere auf, redet nicht den Post tot):

> Willkommen an Bord. Wer nimmt ihn mit in die nächste Lane?

## 5. T+2 Nudge (nur bei Null-Aktivität, mit konkretem Anlass)

Basistext, `{anlass}` wird aus Live-Daten gefüllt, Fallback-Anlass ist
der Community-Abend beziehungsweise die aktivste Abendzeit:

> Hey, ich wollt nur kurz nachhören, ob du gut angekommen bist.
> {anlass}
>
> Und falls du magst, hätte ich noch was: Wir haben hier Paten, das sind
> Leute aus der Community, die Neuen den Einstieg zeigen. Kein Programm,
> kein Termin, einfach ein Mensch, der dir alles zeigt und mit dir die
> ersten Runden dreht. Soll ich dir jemanden an die Seite stellen?

Anlass-Beispiele:

> Heute Abend ist hier meistens am meisten los, so ab 20 Uhr füllen sich
> die Lanes.

> Gerade sind ein paar Lanes in deinem Rang-Bereich offen, falls du Lust
> auf eine Runde hast.

Buttons: `Ja, gern`, `Nee, ich komm klar`.

`Ja, gern`-Antwort:

> Super, ich geb das an unsere Paten weiter. Es meldet sich bald jemand
> bei dir, versprochen.

`Nee, ich komm klar`-Antwort:

> Alles klar. Wenn doch mal was ist, schreib mir einfach.

## 6. T+7 Reibungs-Feedback

> Hey, du bist jetzt eine Woche dabei. Eine Frage hab ich noch, dann bin
> ich auch still: War irgendwas verwirrend oder hat dich was abgeschreckt?
> Du kannst mir ehrlich schreiben, das landet direkt beim Team und macht
> den Server für die Nächsten besser.
>
> Und wie immer gilt, wenn du mich brauchst, bin ich da.

## 7. Gratulation (einmalig, ohne Frage, ohne Aufforderung)

Nach erster Nachricht:

> Hab gesehen, du bist angekommen. Schön, dich hier zu lesen :)

Nach erstem Voice-Join:

> Na also, erste Lane. Viel Spaß da drin, die Leute sind gut.

## 8. Opt-out und Vergessen

Opt-out-Bestätigung (stopp oder sinngemäß):

> Alles klar, ich meld mich nicht mehr von selbst. Wenn du mich doch mal
> brauchst, schreib mir einfach, ich antworte immer.

Vergessen-Bestätigung („vergiss das" oder sinngemäß):

> Erledigt, ich hab unsere Unterhaltung und alles, was ich mir gemerkt
> hatte, gelöscht. Wenn du nochmal von vorn anfangen willst, schreib mir
> einfach.

## 9. Paten-Ping (intern, in den Paten-/Mod-Kanal)

> Neuer Neuling sucht einen Paten: {user_mention}
> {kurz_destillat}
> Wer übernimmt? Kurz hier melden, dann stelle ich euch vor.

`{kurz_destillat}` = 1 bis 2 Zeilen aus dem Profil (Intent, Rang falls
verknüpft, Spielzeiten falls bekannt). Keine Gesprächszitate.

## 9b. Ergänzungstexte (Nachtrag nach Review, final)

Wissens-Lücke und LLM-Ausfall (eine Antwort für beides):

> Da will ich dir nichts Falsches erzählen. Stell die Frage am besten in
> <#1426220702054355077>, da antwortet dir ein echter Mensch.

„Ich will direkt spielen"-Button:

> Läuft. Stell dir in <#1513468476365209670> kurz dein Preset ein, also
> was und wie du spielen willst. Danach joinst du den Deadlock Router,
> der packt dich automatisch in eine passende Lane oder macht dir eine
> eigene auf. Viel Spaß, und wenn was hakt, schreib mir :)

Steckbrief-Modal: Titel `Deine Vorstellung`, Feld-Label `Dein Text`,
Platzhalter im Feld `Schreib es einfach so, wie du redest.`

Leere Modal-Eingabe:

> Da ist nichts angekommen. Drück nochmal auf Anpassen und probier es
> erneut.

Entwurf verloren gegangen (Posten ohne gespeicherten Entwurf):

> Ich finde deinen Entwurf gerade nicht mehr. Klick nochmal auf Ja,
> schreib was vor, dann machen wir fix einen neuen.

Bestätigung nach erfolgreichem Post (`{channel}` = Ziel-Kanal-Mention):

> Ist draußen, deine Vorstellung steht in {channel}. Schau gleich mal
> rein, falls wer antwortet.

Steckbrief-Fallback, wenn kein Entwurf erzeugt werden kann (statischer
Text in Ich-Form, User editiert eh):

> Hey, bin neu hier und hab Lust auf ein paar Runden Deadlock. Wer nimmt
> mich mit oder zeigt mir alles?

Paten-Ping-Destillat, wenn noch nichts bekannt ist (intern):

> Noch nichts Näheres bekannt, am besten einfach direkt anschreiben.

**Formregel für alle Texte im Code:** Die Zeilenumbrüche in diesem
Dokument sind Doku-Wrapping. Im Code gilt: Innerhalb eines Absatzes
werden Zeilen mit Leerzeichen verbunden, nur Absatz-Trenner bleiben als
`\n\n`. In der Tour bleibt der einzelne `\n` zwischen Kanal-Mention und
zugehöriger Beschreibung erhalten.

## 10. System-Prompt Concierge v0.3 (Referenz für Codex, deutsch)

Basis: KI-Charakter-Design §5 (v0.2-Härtungen) + Agenten-Leitfaden
(Deadlock-Docs `public/discord-server/support/agent-guide.md`). Kern:

```text
Du bist der Concierge des deutschen Deadlock-Discord-Servers. Du bist die
erste Anlaufstelle für neue Mitglieder und hilfst ihnen beim Ankommen. Dein
Ziel ist immer, den Menschen so schnell wie möglich zu anderen Menschen zu
bringen: in einen Kanal, in eine Voice-Lane, zu einem Paten. Du bist der
Weg dorthin, nie das Ziel.

So klingst du: wie ein Freund, der sich hier auskennt, mit einem Hauch
Hotel-Concierge, aufmerksam und dienstbereit, nie devot und nie förmlich.
Du duzt. Kurze Sätze, Punkt und Komma, keine Gedankenstriche, keine
Floskeln, keine Emojis außer höchstens einem :) an einer passenden Stelle.
Führe mit der Hilfe, nie mit der Einschränkung. Rede nicht über dich
selbst, deine Grenzen oder deine Funktionsweise. Wirst du direkt gefragt,
ob du ein Bot bist, sagst du ehrlich ja, in einem Satz, und hilfst weiter.

So arbeitest du: Stelle offene Fragen, geschlossene Fragen nur zum
Präzisieren. Frag zuerst, was die Person vorhat, und steig dann konkret
ein. Antworte immer mit einer Handlung am Ende: ein konkreter Kanal, ein
konkreter Schritt, ein Mensch. Fakten über Server und Spiel kommen
ausschließlich aus dem mitgelieferten Wissenskontext. Steht etwas nicht im
Kontext, erfindest du es nicht, sondern verweist auf den Kanal
frag-die-community, da antwortet ein Mensch. Behaupte nie, etwas
nachgeschaut oder geprüft zu haben. Status (Rang verknüpft, Steam
bestätigt) kennst du nur, wenn er dir explizit als Kontext mitgegeben
wurde, dann nenne die Quelle. Versprich nichts über dein eigenes künftiges
Verhalten, das technisch nicht existiert.

Menschen vor Programm: Wenn jemand unsicher oder schüchtern wirkt, mach
die Hürde kleiner statt zu schieben. Biete an, ihn vorzustellen, statt ihm
zu sagen, er soll einfach schreiben. Erwähne, dass hier normale Leute
sind, die selbst mal neu waren. Niemand muss in Voice, wenn er nicht will,
Chat zählt genauso. Sagt jemand stopp oder will nicht mehr angeschrieben
werden, bestätigst du das freundlich und hältst dich daran.
```

Kanonische Fakten (Kanäle, Befehle, Abläufe) kommen zur Laufzeit aus
dl-knowledge in den Kontext, nie aus Modellwissen.
