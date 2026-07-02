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

Die Steam-Verknüpfung hat **drei** Schritte – ein reiner Login reicht nicht:

1. Verknüpfe Steam über `/account_verknüpfen` oder das Steam-Panel in <#1398021105339334666> (Button `Steam verknüpfen`). Der Login läuft über Steam selbst, ohne Passwort-Eingabe beim Bot.
2. Gib deinen **Steam-Freundescode** ein (Panel-Button `Freundescode eingeben`), damit der Bot dir die Freundschaftsanfrage schicken kann.
3. **Nimm die Steam-Freundschaftsanfrage des Bots an.** Erst damit gilt der Link als vollständig verifiziert – und erst dann funktionieren Verified-Rolle, Rang-Erkennung und Invite sauber.

Mehrere Accounts hinterlegt? Mit `/steam links` siehst du, was gespeichert ist; mit `/steam setprimary` setzt du deinen Hauptaccount (für viele Features zählt der Primäraccount). `/steam whoami` zeigt deine gespeicherte ID, `/steam unlink` entfernt eine Verknüpfung.

Rang prüfen: `/steam_rank` oder der Button `📊 Rang prüfen` im Panel zeigen den Rang nur an. Wenn deine **Rang-Rollen** nicht stimmen, nutze `/checkrank` – das gleicht die Rollen tatsächlich ab.

## Kein Beta-Invite / Invite kommt nicht an

Geh die Punkte der Reihe nach durch:

1. **In <#1464736918951432222> gefragt?** Der Weg ist bewusst simpel: nette Frage in den Kanal („mag mich wer einladen? :)") plus dein **Steam-Freundescode** (Steam → Freunde → „Freund hinzufügen"). Ohne Code kann dich niemand einladen — der Bot erinnert dich im Kanal daran.
2. **Freundschaftsanfrage angenommen?** Wer dich einlädt, muss erst mit dir auf Steam befreundet sein. Schau in deine Steam-Anfragen.
3. **„Limited User"?** Steam blockiert Playtest-Invites, wenn auf deinem Account noch keine ~5 $ ausgegeben wurden. Das ist eine Valve-Regel, die niemand umgehen kann — sobald dein Account die Schwelle erreicht, klappt es.
4. Ein Invite taucht nicht immer sofort in der Bibliothek auf – es kann **1–2 Tage** dauern. Prüfe deine Steam-Playtest-Einladungen später noch einmal.
5. Die Steam-Verknüpfung in <#1398021105339334666> lohnt sich zusätzlich — damit kann auch der Steam-Bot automatisiert einladen.

## Coaching: Zugang, Ablauf, Status

1. Coaching läuft über die **Website**: Im Coaching-Kanal führt dich der Button zur Anfrage-Seite (Login mit Discord, dann Formular mit Rang, Zielen und Verfügbarkeit). Auch `/coaching-anfrage` gibt dir den Website-Link.
2. Coaching ist **kostenlos**, und du darfst **mehrfach** anfragen. Deine Anfrage landet automatisch beim Coach-Team im Discord, ein Coach übernimmt sie.
3. Mit `/coaching-status` siehst du jederzeit den Stand deiner Anfrage.
4. Die Kommunikation läuft **nur im Coaching-Chat auf dem Server** – keine DMs, keine Freundschaftsanfragen an Coaches.
5. Nach der gemeinsamen Coaching-Session kommt eine Feedback-Anfrage per DM – bitte ehrlich ausfüllen, das hilft dem Team.

## Rang-Anzeige bei den Voice-Lanes

- Dein Server-Rang wird **automatisch** vergeben, sobald dein Steam-Account verifiziert verknüpft ist (siehe Abschnitt Steam oben).
- Bei Ranked Lanes erscheint der Rang automatisch im Kanalnamen; Basis ist der Rang-Anker der Lane (ursprünglicher Ersteller bzw. erstes Mitglied mit Rang).

## Was dieser Bereich nicht klärt

- Zwischenmenschliche Konflikte, Beschwerden über andere Mitglieder oder Moderationsfälle gehören nicht hierher – darum kümmert sich das Team direkt.
- Reine Gameplay- und Spielmechanik-Fragen zu Deadlock (Item-Builds, Timings, Map-Wissen) deckt diese Server-Doku nicht ab.
