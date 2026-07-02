# FAQ-Bot selbst

## Worum geht es?
Der FAQ-Bot ist ein dokumentationsbasierter Server-Assistent. Er beantwortet Fragen zu Kanälen, Rollen, Abläufen und sichtbaren Bot-Features, merkt sich den laufenden Chat für eine begrenzte Zeit und kann in bestimmten Ticket-Kategorien direkt den ersten Hilfsversuch posten.

## Wie nutze ich das?
Es gibt zwei sichtbare Zugänge, die beide dasselbe machen: Im FAQ-Panel klickst du auf `Frage stellen`, oder du nutzt `/faq`. In beiden Fällen erstellt der Bot dir einen **privaten FAQ-Chat-Kanal** in der FAQ-Kategorie. Dort schreibst du einfach normal und kannst auch Rückfragen stellen.

Der Bot merkt sich den bisherigen Verlauf innerhalb derselben Session (die letzten Nachrichten). Das heißt: Du musst nicht jede Anschlussfrage komplett neu formulieren, solange du im gleichen FAQ-Chat bleibst. Wenn du fertig bist, beendest du die Session über den `Chat beenden`-Button — einen Schließen-Befehl gibt es nicht.

Ein weiterer sichtbarer Bereich ist die Ticket-Auto-Hilfe. Wenn in einer dafür vorgesehenen Ticket-Kategorie ein neues Ticket aufgemacht wird und der User seine erste Nachricht schreibt, versucht der FAQ-Bot sofort einen stillen Erstcheck. Falls die Frage klar aus der Server-Doku beantwortbar ist, postet er direkt eine Antwort. Wenn nicht, bleibt er absichtlich still und übergibt implizit an menschlichen Support.

Dabei kümmert er sich nur um sach- und problembezogene Anliegen, also echte Fragen und konkrete „X funktioniert nicht"-Fälle. Bei zwischenmenschlichem Stress, Streit oder Beschwerden über andere Mitglieder hält er sich bewusst raus; das übernehmen Menschen. Da er bereits im Ticket antwortet, verweist er nicht zurück auf das Ticket-System.

Wichtig ist der Zeitrahmen: FAQ-Sessions bleiben 24 Stunden aktiv. Danach schließt der Bot sie automatisch.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Beim Start lädt der Bot alle Markdown-Dateien aus dem flachen `docs/`-Ordner und nutzt genau diesen Inhalt als Wissensbasis. Fragen und Antworten werden pro Session gespeichert, damit Rückfragen mit Kontext beantwortet werden können. Für die Ticket-Auto-Hilfe gibt es einen separaten Modus: Wenn keine sichere Antwort aus der Doku möglich ist, antwortet der Bot absichtlich gar nicht. Für Twitch-Fragen kann er zusätzlich eine Diagnose des Twitch-Setups abrufen (OAuth-Status, fehlende Scopes, Discord-Link) und daraus konkrete Schritte ableiten.

## Grenzen & häufige Fragen
- Der FAQ-Bot kennt nur Server-Doku. Wenn etwas nicht dokumentiert ist, weiß er es im Zweifel nicht.
- Er kann keine internen Aktionen ausführen: keine Rollen vergeben, keine Bots neu starten, keine Tickets administrieren, keine Konfiguration ändern.
- Er teilt keine Secrets, Tokens, internen Pfade oder Admin-Details.
- Bei Beta-Invite-, Coaching- oder Channel-Fragen verweist er auf die dokumentierten Schritte und Orte, nicht auf versteckte Workarounds.
- Pro User ist nur ein aktiver privater FAQ-Chat gleichzeitig vorgesehen.
- Ticket-Auto-Hilfe ist konservativ. Wenn der Bot unsicher ist, schweigt er lieber, statt etwas zu erfinden.
- Es gibt keinen `/faqclose`-Befehl und keine Thread-Sessions mehr — das war das alte System.

## Für Devs (knapp)
- Rust live: `dl-community/src/faq.rs` — Panel (`/faqpanel`, idempotentes Panel-Healing), `/faq`, Button `faq_chat:start`, Close-Button `faq_chat:close:{session}`, Ticket-Auto-Help (Kategorie-Check + KEIN_TREFFER-Protokoll), Twitch-Diagnose-Tool
- Grounding: flacher Loader für `docs/*.md` (`load_docs`), Verlauf = letzte 10 Nachrichten
- Wichtige DB-Tabellen: `bot.faq_chat_sessions`, `bot.faq_chat_messages`, Panel-ID im KV (`faq_chat:panel`)
- Python `cogs/faq_chat.py`/`server_faq.py` sind Legacy (dort lebte `/faqclose` + `server_faq_logs`)
