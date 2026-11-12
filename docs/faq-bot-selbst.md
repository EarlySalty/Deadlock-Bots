# FAQ-Bot selbst

## Worum geht es?
Der FAQ-Bot ist ein dokumentationsbasierter Server-Assistent. Er beantwortet Fragen zu Kanälen, Rollen, Abläufen und sichtbaren Bot-Features, merkt sich den laufenden Chat für eine begrenzte Zeit und wertet die erste Nachricht bestimmter Tickets intern aus.

## Wie nutze ich das?
Es gibt zwei sichtbare Zugänge, die beide dasselbe machen: Im FAQ-Panel klickst du auf `Frage stellen`, oder du nutzt `/faq`. In beiden Fällen erstellt der Bot dir einen **privaten FAQ-Chat-Kanal** in der FAQ-Kategorie. Dort schreibst du einfach normal und kannst auch Rückfragen stellen.

Der Bot merkt sich den bisherigen Verlauf innerhalb derselben Session (die letzten Nachrichten). Das heißt: Du musst nicht jede Anschlussfrage komplett neu formulieren, solange du im gleichen FAQ-Chat bleibst. Wenn du fertig bist, beendest du die Session über den `Chat beenden`-Button — einen Schließen-Befehl gibt es nicht.

Bei neuen Support-Tickets sammelt der Bot, was du zu Beginn schreibst, bis du kurz nichts mehr schickst. Daraus macht er eine interne Notiz fürs Team: ob die Doku eine passende Antwort hergibt, einen Antwortvorschlag und den Hinweis, ob besser ein Moderator draufschaut, zum Beispiel bei Banns, Sperren, Scam oder wenn du ausdrücklich einen Menschen willst.

Meldest du einen Scam, nennt die Notiz zusätzlich die Accounts aus deinem Text und ob sie auf dem Server sind. Am besten schreibst du den Namen in eine eigene Zeile, etwa `Username: name`, oder erwähnst den Account direkt. Die Notiz sagt auch, ob ein Bann in Frage käme. Gebannt wird dabei niemand; das entscheidet immer das Team. Screenshots prüft der Bot ohne ausdrückliche Freigabe des Teams nicht.

Die Shadow-Auswertung ist nur eine Entscheidungshilfe für das Team. Sie führt keine Aktion im Ticket aus und fordert nicht dazu auf, ein weiteres Ticket zu öffnen.

Wichtig ist der Zeitrahmen: FAQ-Sessions bleiben 24 Stunden aktiv. Danach schließt der Bot sie automatisch.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Der Bot beantwortet Fragen über das zentrale Deadlock Brain, das die Server-Doku kennt. Fragen und Antworten werden pro Session gespeichert, damit Rückfragen mit Kontext beantwortet werden können. Bei Tickets geht nur dein Text an das Brain, und das Ergebnis sieht nur das Team. Screenshots aus Scam-Meldungen gehen nur an den Bildprüfer der Moderation, wenn das Team diese Prüfung eingeschaltet hat; ab Werk ist sie aus.

## Grenzen & häufige Fragen
- Der FAQ-Bot kennt nur Server-Doku. Wenn etwas nicht dokumentiert ist, weiß er es im Zweifel nicht.
- Er kann keine internen Aktionen ausführen: keine Rollen vergeben, keine Bots neu starten, keine Tickets administrieren, keine Konfiguration ändern.
- Er teilt keine Secrets, Tokens, internen Pfade oder Admin-Details.
- Bei Beta-Invite-, Coaching- oder Channel-Fragen verweist er auf die dokumentierten Schritte und Orte, nicht auf versteckte Workarounds.
- Pro User ist nur ein aktiver privater FAQ-Chat gleichzeitig vorgesehen.
- Ticket-Shadow ist nur intern. Auch ein guter Kandidat wird nie automatisch ins Ticket gesendet.
- Es gibt keinen `/faqclose`-Befehl und keine Thread-Sessions mehr — das war das alte System.

## Für Devs (knapp)
- Rust live: `dl-community/src/faq.rs`: Panel (`/faqpanel`, idempotentes Panel-Healing), `/faq`, Button `faq_chat:start`, Close-Button `faq_chat:close:{session}`, Ticket-Shadow (Sammelfenster 45 s Ruhe, höchstens 180 s; Brain-Zustand, Kandidat, Eskalationsgründe; Scam-Shadow aus `ticket_triage.rs`)
- Ticket-Bildprüfung: Schalter `moderation.ticket_screenshot_review` (Standard aus), Ergebnis nur im Shadow-Log
- Grounding: Deadlock Brain, Verlauf = letzte 10 Nachrichten
- Wichtige DB-Tabellen: `bot.faq_chat_sessions`, `bot.faq_chat_messages`, Panel-ID im KV (`faq_chat:panel`)
- Python `cogs/faq_chat.py`/`server_faq.py` sind Legacy (dort lebte `/faqclose` + `server_faq_logs`)
