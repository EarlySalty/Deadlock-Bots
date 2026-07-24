# FAQ-Bot selbst

## Worum geht es?
Der FAQ-Bot ist ein dokumentationsbasierter Server-Assistent. Er beantwortet Fragen zu Kanälen, Rollen, Abläufen und sichtbaren Bot-Features, merkt sich den laufenden Chat für eine begrenzte Zeit und wertet die erste Nachricht bestimmter Tickets intern aus.

## Wie nutze ich das?
Es gibt zwei sichtbare Zugänge, die beide dasselbe machen: Im FAQ-Panel klickst du auf `Frage stellen`, oder du nutzt `/faq`. In beiden Fällen erstellt der Bot dir einen **privaten FAQ-Chat-Kanal** in der FAQ-Kategorie. Dort schreibst du einfach normal und kannst auch Rückfragen stellen.

Der Bot merkt sich den bisherigen Verlauf innerhalb derselben Session (die letzten Nachrichten). Das heißt: Du musst nicht jede Anschlussfrage komplett neu formulieren, solange du im gleichen FAQ-Chat bleibst. Wenn du fertig bist, beendest du die Session über den `Chat beenden`-Button — einen Schließen-Befehl gibt es nicht.

Für neue Tickets in der vorgesehenen Kategorie läuft eine interne Shadow-Auswertung: Stufe 1 bewertet die erste Nachricht mit `yes`, `no` oder `uncertain`. Stufe 2 erzeugt unabhängig vom Urteil immer einen kurzen deutschen Antwortkandidaten. Urteil und Kandidat erscheinen ausschließlich im internen Shadow-Kanal; der Bot schreibt niemals automatisch ins Ticket. Technische Generatorfehler bleiben dort knapp sichtbar.

Die Shadow-Auswertung ist nur eine Entscheidungshilfe für das Team. Sie führt keine Aktion im Ticket aus und fordert nicht dazu auf, ein weiteres Ticket zu öffnen.

Wichtig ist der Zeitrahmen: FAQ-Sessions bleiben 24 Stunden aktiv. Danach schließt der Bot sie automatisch.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Beim Start lädt der Bot alle Markdown-Dateien aus dem flachen `docs/`-Ordner und nutzt genau diesen Inhalt als Wissensbasis. Fragen und Antworten werden pro Session gespeichert, damit Rückfragen mit Kontext beantwortet werden können. Im Ticket-Shadow trennt er das Knowledge-Urteil (`yes`/`no`/`uncertain`) von der anschließenden deutschen Antwortformulierung; nur das interne Team sieht das Ergebnis. Für Twitch-Fragen kann er zusätzlich eine Diagnose des Twitch-Setups abrufen (OAuth-Status, fehlende Scopes, Discord-Link) und daraus konkrete Schritte ableiten.

## Grenzen & häufige Fragen
- Der FAQ-Bot kennt nur Server-Doku. Wenn etwas nicht dokumentiert ist, weiß er es im Zweifel nicht.
- Er kann keine internen Aktionen ausführen: keine Rollen vergeben, keine Bots neu starten, keine Tickets administrieren, keine Konfiguration ändern.
- Er teilt keine Secrets, Tokens, internen Pfade oder Admin-Details.
- Bei Beta-Invite-, Coaching- oder Channel-Fragen verweist er auf die dokumentierten Schritte und Orte, nicht auf versteckte Workarounds.
- Pro User ist nur ein aktiver privater FAQ-Chat gleichzeitig vorgesehen.
- Ticket-Shadow ist nur intern. Auch ein guter Kandidat wird nie automatisch ins Ticket gesendet.
- Es gibt keinen `/faqclose`-Befehl und keine Thread-Sessions mehr — das war das alte System.

## Für Devs (knapp)
- Rust live: `dl-community/src/faq.rs` — Panel (`/faqpanel`, idempotentes Panel-Healing), `/faq`, Button `faq_chat:start`, Close-Button `faq_chat:close:{session}`, Ticket-Shadow (Knowledge-Urteil + interner Kandidat), Twitch-Diagnose-Tool
- Grounding: flacher Loader für `docs/*.md` (`load_docs`), Verlauf = letzte 10 Nachrichten
- Wichtige DB-Tabellen: `bot.faq_chat_sessions`, `bot.faq_chat_messages`, Panel-ID im KV (`faq_chat:panel`)
- Python `cogs/faq_chat.py`/`server_faq.py` sind Legacy (dort lebte `/faqclose` + `server_faq_logs`)
