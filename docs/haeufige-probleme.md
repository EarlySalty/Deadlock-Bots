# Häufige Probleme & Selbsthilfe

Diese Seite bündelt die Anliegen, die im Support am häufigsten auftauchen, mit konkreten Selbsthilfe-Schritten. Wenn ein Punkt dein Problem nicht löst, schildere es einfach hier im Ticket – das Team schaut es sich dann an.

## Twitch-Bot kommt nicht in meinen Stream / Chat

Wenn der Bot deinem Kanal nicht beitritt oder nichts tut, liegt es fast immer an der Twitch-Autorisierung:

1. Öffne die Verwaltungsseite deines Streamer-Dashboards. Dort siehst du, ob **Twitch-OAuth verbunden** ist, welche **Scopes fehlen** und wie der Status der **Discord-Verbindung** ist.
2. Fehlen Scopes oder muss der Token neu autorisiert werden, zeigt die Verwaltungsseite einen **Reconnect-Link**. Nutze ihn und vergib bei der Twitch-Abfrage wirklich alle angefragten Berechtigungen.
3. Ohne gültige Twitch-Autorisierung bleiben besonders Raid- und Teile der Analyse-Funktionen eingeschränkt – ein sauberer Reconnect ist deshalb der erste Schritt.

Wenn OAuth verbunden ist, keine Scopes fehlen und es trotzdem nicht läuft, schildere das hier im Ticket mit deinem Twitch-Namen.

## "Ich habe autorisiert, aber es steht auf inaktiv"

Das heißt meistens, dass die Autorisierung **unvollständig** ist – nicht, dass gar nichts angekommen wäre:

1. Geh auf die Verwaltungsseite und prüfe gezielt die **fehlenden Scopes**. Schon ein einzelner fehlender Scope kann Funktionen ausbremsen.
2. Wird dort ein **Reconnect** angeboten, mach ihn komplett neu und bestätige bei Twitch wirklich alle Berechtigungen – nicht nur einen Teil.
3. Prüfe zusätzlich die **Discord-Verbindung** auf derselben Seite. Beide Seiten gehören zum vollständigen Setup.

Bleibt der Status nach einem vollständigen Reconnect (alle Scopes vergeben, OAuth und Discord verbunden) weiter inaktiv, schildere es hier im Ticket.

## Steam lässt sich nicht verbinden / Rang wird nicht erkannt

Die Steam-Verknüpfung hat **zwei** Schritte – ein reiner Login reicht nicht:

1. Verknüpfe Steam über `/account_verknüpfen` (alternativ `/link`) oder das Steam-Link-Panel. Der Login läuft über Steam selbst, ohne Passwort-Eingabe beim Bot.
2. **Bestätige danach die Steam-Freundschaft mit dem Bot.** Erst damit gilt der Link als vollständig verifiziert – und erst dann funktionieren Verified-Rolle, Rang-Erkennung und Invite sauber.
3. Mehrere Accounts hinterlegt? Mit `/steam links` siehst du, was gespeichert ist; mit `/steam setprimary` setzt du deinen Hauptaccount (für viele Features zählt der Primäraccount).

Hinweis: Ist dein Steam-Account eingeschränkt, kann Steam Aktionen wie Einladungen blockieren. Deinen Rang prüfst du mit `/steam_rank` oder dem `Rank Check`-Button im Steam-Panel.

## Kein Beta-Invite / #beta-zugang fehlt / Invite kommt nicht an

Geh die Punkte der Reihe nach durch:

1. **Onboarding vollständig abgeschlossen?** Starte es notfalls neu in `#regelwerk` über `Hier starten`. Du musst dort die Option für **Invite/Betazugang** gewählt haben, sonst wird `#beta-zugang` nicht sichtbar.
2. **Rollen gesetzt?** Nach dem Onboarding in `#customize` bzw. der Rollen-Auswahl die passenden Rollen wählen – fehlen sie, fehlen Kanäle.
3. **`/betainvite` im richtigen Channel** (`#beta-zugang`) ausführen.
4. **Steam-Voraussetzung:** Dein Steam-Account braucht mindestens **5 € echte Kaufhistorie**. Reines Wallet-Aufladen oder Free-to-Play reicht dafür nicht.
5. Ein Invite taucht nicht immer sofort in der Bibliothek auf – prüfe später noch einmal deine Steam-Playtest-Einladungen. Vorübergehende Steam-Probleme lassen sich oft mit einem späteren erneuten Versuch lösen.

## Coaching: Zugang, Ablauf, Status

1. Coaching anfragen in `#ich-brauch-einen-coach` über den Coaching-Button oder mit `/coaching-anfrage`. Im Formular Rang, Main-Hero, Verfügbarkeit und Ziele/Probleme eintragen.
2. Coaching ist **kostenlos**, und du darfst **mehrfach** anfragen.
3. Mit `/coaching-status` siehst du, ob deine Anfrage noch analysiert wird, auf einen Coach wartet oder schon läuft.
4. Die Kommunikation läuft **nur im Coaching-Chat auf dem Server** – keine DMs, keine Freundschaftsanfragen an Coaches.
5. Nach der Session kommt eine Feedback-Anfrage – bitte ehrlich ausfüllen, das hilft dem Team.

## Rang-Anzeige bei den Voice-Lanes

- Dein Server-Rang wird **automatisch** vergeben, sobald dein Steam-Account verifiziert verknüpft ist (siehe Abschnitt Steam oben).
- Bei Ranked Lanes erscheint der Rang automatisch im Kanalnamen; Basis ist der ursprüngliche Lane-Ersteller.

## Was dieser Bereich nicht klärt

- Zwischenmenschliche Konflikte, Beschwerden über andere Mitglieder oder Moderationsfälle gehören nicht hierher – darum kümmert sich das Team direkt.
- Reine Gameplay- und Spielmechanik-Fragen zu Deadlock (Item-Builds, Timings, Map-Wissen) deckt diese Server-Doku nicht ab.
