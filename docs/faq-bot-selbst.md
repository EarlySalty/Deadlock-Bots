# FAQ-Bot selbst

## Worum geht es?
Der FAQ-Bot ist ein dokumentationsbasierter Server-Assistent. Er beantwortet Fragen zu Kanalen, Rollen, Ablaufen und sichtbaren Bot-Features, merkt sich den laufenden Chat fur eine begrenzte Zeit und kann in bestimmten Ticket-Kategorien sogar direkt den ersten Hilfsversuch posten.

## Wie nutze ich das?
Es gibt zwei sichtbare Zugange. Im FAQ-Panel klickst du auf `Frage stellen`; dadurch erstellt der Bot dir einen privaten FAQ-Chat-Channel. Alternativ kann per `/faq` ein FAQ-Thread erzeugt werden. In beiden Fallen kannst du dort einfach normal schreiben und auch Rueckfragen stellen.

Der Bot merkt sich den bisherigen Verlauf innerhalb derselben Session. Das heisst: Du musst nicht jede Anschlussfrage komplett neu formulieren, solange du im gleichen FAQ-Chat oder Thread bleibst. Wenn du fertig bist, kannst du die Session selbst beenden, entweder uber den `Chat beenden`-Button oder im Thread mit `/faqclose`.

Ein weiterer sichtbarer Bereich ist die Ticket-Auto-Hilfe. Wenn in einer dafuer vorgesehenen Ticket-Kategorie ein neues Ticket aufgemacht wird und der User seine erste Nachricht schreibt, versucht der FAQ-Bot sofort einen stillen Erstcheck. Falls die Frage klar aus der Server-Doku beantwortbar ist, postet er direkt eine Antwort. Wenn nicht, bleibt er absichtlich still und uebergibt implizit an menschlichen Support.

Dabei kuemmert er sich nur um sach- und problembezogene Anliegen, also echte Fragen und konkrete "X funktioniert nicht"-Faelle. Bei zwischenmenschlichem Stress, Streit oder Beschwerden ueber andere Mitglieder haelt er sich bewusst raus; das uebernehmen Menschen. Da er bereits im Ticket antwortet, verweist er nicht zurueck auf das Ticket-System.

Wichtig ist der Zeitrahmen: FAQ-Sessions bleiben 24 Stunden aktiv. Danach schliesst der Bot sie automatisch. Das gilt sowohl fur die privaten FAQ-Chats als auch fur die Thread-basierten Sitzungen.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Beim Start laedt der Bot alle Markdown-Dateien aus dem flachen `docs/`-Ordner und nutzt genau diesen Inhalt als Wissensbasis. Fragen und Antworten werden pro Session gespeichert, damit Rueckfragen mit Kontext beantwortet werden koennen. Fuer die Ticket-Auto-Hilfe gibt es einen separaten Modus: Wenn keine sichere Antwort aus der Doku moeglich ist, antwortet der Bot absichtlich gar nicht.

## Grenzen & häufige Fragen
- Der FAQ-Bot kennt nur Server-Doku. Wenn etwas nicht dokumentiert ist, weiss er es im Zweifel nicht.
- Er kann keine internen Aktionen ausfuehren: keine Rollen vergeben, keine Bots neu starten, keine Tickets administrieren, keine Konfiguration aendern.
- Er teilt keine Secrets, Tokens, internen Pfade oder Admin-Details.
- Bei Beta-Invite-, Coaching- oder Channel-Fragen verweist er auf die dokumentierten Schritte und Orte, nicht auf versteckte Workarounds.
- Pro User ist nur ein aktiver privater FAQ-Chat gleichzeitig vorgesehen.
- Ticket-Auto-Hilfe ist konservativ. Wenn der Bot unsicher ist, schweigt er lieber, statt etwas zu erfinden.

## Für Devs (knapp)
- Cogs: `cogs/faq_chat.py`, `cogs/server_faq.py`
- Abhangigkeiten: `AIConnector`, Patchnote-Kontext aus `service.changelogs`, flacher Loader fuer `docs/*.md`
- Wichtige DB-Tabellen: `faq_chat_sessions`, `faq_chat_messages`, `server_faq_logs`, `kv_store`
