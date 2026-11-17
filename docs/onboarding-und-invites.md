# Onboarding und Invites

## Worum geht es?
Der Server führt neue Mitglieder gezielt durch Regeln, Rollen und die wichtigsten Bereiche. Der Deadlock-Beta-Invite ist davon unabhängig und bewusst einfach: Du fragst in <#1464736918951432222> nett nach einem Invite, und jemand aus der Community lädt dich ein.

## Wie nutze ich das?
**Onboarding:** Starte im <#1315684135175716975> über den Button `Hier starten` — der Bot legt dir einen privaten Onboarding-Thread an. Nach dem Discord-Member-Screening startet das Onboarding auch automatisch (Bots, bereits verifizierte und bereits durchgelaufene User werden übersprungen). Die festen Schritte: Regeln, Voice-Lanes, Mitspieler finden, Coaching, Custom Games, optionale Voice-Präferenzen (Ton/Alter) und die Steam-Verknüpfung. Beim Steam-Schritt prüfst du selbst per Button `Ich hab verknüpft` — der Bot checkt dann live, ob die Verknüpfung da ist.

Deine Voice-Tags kannst du jederzeit später über `/meine-tags` ändern; die Auswahl wird sofort gespeichert.

Zusätzlich gibt es das AI-Onboarding-Panel mit `Persönliche Tour starten` (3 kurze Fragen im Formular) und Quick-Buttons zu Spieler-Suche, Voice-Lanes, Feedback und Regelwerk. Und: Auf einigen Panels vergeben Reaktionen automatisch Rollen (Reaction Roles) — Reaktion entfernen kann die Rolle wieder entziehen.

**Deadlock-Invite:** <#1464736918951432222> ist eine offene Invite-Lounge — keine Befehle, keine Formulare, keine Vorbedingungen. So läuft es:

1. Schreib eine nette Frage in den Kanal („mag mich wer einladen? :)") und poste deinen **Steam-Freundescode** dazu (findest du in Steam unter Freunde, „Freund hinzufügen").
2. Ein Community-Mitglied fügt dich hinzu und lädt dich persönlich zum Playtest ein.
3. Der Bot liest mit: Fehlt dein Freundescode, fragt er danach. Kennt der Bot deinen Code, fragt er nach einer halben Stunde nach, ob du schon eine Einladung hast. Hat dich unser Steam-Bot bis dahin schon eingeladen, spart er sich die Frage, und läuft gerade noch eine Einladung, wartet er sie ab. Antwortest du mit „nein", lädt er dich direkt ein, wenn ihr schon Steam-Freunde seid. Sonst schickt er dir erst eine Freundschaftsanfrage, und sobald du sie annimmst, kommt die Einladung. Antwortest du mit „ja", ist die Sache erledigt. Normaler Chat bleibt unbeantwortet.

Zusätzlich lohnt sich die Steam-Verknüpfung in <#1398021105339334666> in jedem Fall: Damit bekommst du deine echte Rang-Rolle, und der Steam-Bot kann Invites auch automatisiert verschicken (läuft als Sicherheitsnetz im Hintergrund).

## Kosten / Premium
Alles kostenlos. Ein Invite kostet nie Geld — weder von der Community noch vom Bot. Es gibt einen optionalen Ko-fi-Support (freiwillig, kein Pflichtkauf und keine Beschleunigung).

## Was passiert technisch (kurz)?
Das Regelwerk-Panel und der Screening-Autostart erzeugen private Onboarding-Threads; der Flow speichert Entscheidungen wie Voice-Tags. In <#1464736918951432222> bekommt das Deadlock Brain die Textnachrichten von Mitgliedern mit dem Stand der Anfrage und entscheidet den nächsten Schritt: nach dem Code fragen, nachfragen, ob schon eine Einladung da ist, einladen, helfen, auf die Community warten, die Anfrage schließen oder schweigen. Nicht bewertet werden Nachrichten ohne Text (nur Bild, Sticker oder Anhang), Nachrichten von Bots und Systemmeldungen, Nachrichten, die älter als das Anfragefenster von sieben Tagen sind oder vor dem ersten Start des Watchers geschrieben wurden, und Nachrichten von Mitgliedern, die der Datenverarbeitung widersprochen haben. Der Bot führt nur aus und schreibt keine eigenen Texte. Der Steam-Bot kann Invites zusätzlich automatisiert über die Steam-Spielsuche verschicken.

## Grenzen & häufige Fragen
- **„Ich habe keinen Invite bekommen"**: Häufigster Grund ist ein „limited" Steam-Account — Steam blockiert Playtest-Invites, wenn auf dem Account noch keine ~5 $ ausgegeben wurden. Das ist eine Valve-Regel, die erst beim Invite-Versuch sichtbar wird; kein Bot und kein Community-Mitglied kann sie umgehen.
- Nach einem Invite kann es 1–2 Tage dauern, bis die Einladung bei Steam sichtbar ist.
- Ohne Freundescode in deiner Anfrage kann dich niemand einladen; der Bot fragt im Kanal danach.
- Es gibt KEINE Onboarding-Option „Invite/Betazugang" und keinen Invite-Befehl — einfach im Kanal fragen.
- Ein Streamer-Setup-Flow im Onboarding existiert aktuell nicht (nur Hinweise); Streamer-Themen laufen über das Twitch-Dashboard.

## Für Devs (knapp)
- Rust: `dl-community/src/onboarding.rs` (+ `onboarding_steps.json`), `ai_onboarding.rs`, `tags_ui.rs`, `reaction_roles.rs`, Invite-Lounge-Watcher in `dl-community`; automatisierter Invite-Pfad im Steam-Bot: `steam-flows/src/betainvite/`
- Der Invite-Lounge-Watcher (`dl-community/src/invite_lounge.rs`) gibt die menschlichen Textnachrichten in <#1464736918951432222> mit dem Anfragestand als `discord_invite_turn` an das Brain (Ausnahmen siehe oben); es gibt keine Wortlisten und keine Beitrittsgrenze. Der Bot hält Zustand, Steam-Versand und Ausgang (`bot.kv_store`, ns `invite_lounge:requests`), das Brain schreibt die Texte über den Broker. Ein Rückblick nach dem Start und nach Ausfällen holt ungelesene Nachrichten ab der gespeicherten Marke nach, höchstens `REQUEST_WINDOW_SECONDS` (sieben Tage) zurück und nie vor den ersten Start. Eine Nachricht, deren Bewertung an einem Lesefehler scheitert, bleibt für den nächsten Rückblick liegen, und spätere Nachrichten desselben Mitglieds warten mit ihr. Jeder Schreibzugriff auf eine Anfrage läuft in einer Transaktion unter der Datenschutz-Sperre des Mitglieds und vergleicht den gelesenen Stand inhaltlich (geparst, nicht als JSON-Text); ein abgemeldetes Mitglied wird dabei nie neu geschrieben. Rollout: alte Zeilen bleiben lesbar, es braucht keine Migration.
- Der User-Slash-Command `/betainvite` wurde entfernt; der Funnel ist nur noch über den Panel-Button (`betainvite:panel:start`, Admin: `/publish_betainvite_panel`) erreichbar
- Kanalnamen live: <#1464736918951432222>, <#1398021105339334666> (Server-as-Code-Renames)
