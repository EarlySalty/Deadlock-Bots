# Community-Tools

## Worum geht es?
Diese Doku bündelt die Community-Helfer außerhalb der reinen Voice-Steuerung: Tags, LFG-Matching, anonymes Feedback, Clip-Einsendungen, den FAQ-Chat, die Bot-DM-Hilfe und die Leave-Survey nach einem Server-Austritt. Die Features helfen dir dabei, schneller die richtigen Leute zu finden, Probleme sauber zu melden und Feedback ohne Umwege loszuwerden.

## Wie nutze ich das?
- **Tags-System:** Nutze `/meine-tags`. Dort kannst du für dich `25+` oder `U25` sowie `Banter-OK` oder `Ragebaiter-Free` setzen und speichern. Diese Angaben werden von LFG- und TempVoice-Filtern genutzt.
- **Support-Tickets:** Über den Button im Channel <#1459628609705738539> öffnest du einen privaten Ticket-Kanal fürs Team. Die erste Nachricht wird nur intern im Shadow geprüft: Stufe 1 bewertet `yes`, `no` oder `uncertain`, Stufe 2 versucht unabhängig vom Urteil immer, einen kurzen deutschen Antwortkandidaten zu erzeugen. Urteil und Kandidat erscheinen ausschließlich im internen Shadow-Kanal; der Bot schreibt niemals automatisch ins Ticket. Technische Generatorfehler bleiben dort knapp sichtbar.
- **FAQ-Chat:** Über das FAQ-Panel (`Frage stellen`) oder `/faq` bekommst du einen privaten Frage-Kanal, in dem der FAQ-Bot Serverfragen beantwortet — mit Gedächtnis für den bisherigen Verlauf. Die Session schließt nach 24 Stunden automatisch oder sofort über den Schließen-Button.
- **Bot-DM-Hilfe:** Du kannst dem Bot auch direkt per DM schreiben — er beantwortet Serverfragen dort genauso doku-basiert wie im FAQ-Chat.
- **LFG-Matching:** Wenn du **nicht** im Voice sitzt, schreibst du in <#1376335502919335936> einfach Dinge wie `suche +2 für Ranked`, `wer bock auf Chill?` oder `lfm für Street Brawl`. Der Bot erkennt die Absicht und antwortet mit passenden Lobbys, Rang-Hinweisen und einem Vorschlag, welche Lane du selbst aufmachen solltest. Auch Mitspieler, mit denen du öfter zusammen spielst, fließen in die Vorschläge ein.
- **LFG-Filter per Text:** Schreibst du in deiner LFG-Nachricht `25+` oder `ragebaiter free`, filtert der Bot Kandidaten entsprechend strenger.
- **Feedback-Hub:** Im <#1465404160005378129> klickst du auf `Anonymes Feedback senden`. Danach beantwortest du bis zu fünf Fragen zu Spielerlebnis, Server, Verbesserungen und Wünschen. Deine Nachricht wird anonym intern weitergereicht.
- **Clip-Submission:** Im <#1425215762460835931> klickst du auf `Clip einsenden`, bestätigst die Nutzungserlaubnis und füllst dann Link, Credit/Username und optionale Infos aus. Mindestqualität ist 1080p.
- **Clip-Wochenfenster:** Einsendungen laufen in einem Wochenfenster von Sonntag bis Samstag. Nach Ablauf wird gesammelt ein Wochen-Dump erzeugt.
- **Leave-Survey:** Wenn du den Server verlässt, kann dir der Bot per DM eine kurze Austrittsumfrage schicken. Du wählst zuerst einen Grund aus und bekommst danach eine Folgefrage; für längeres Feedback gibt es zusätzlich einen Web-Link.
- **Scrims:** Über die Coaching-Website kannst du dich für Scrims anmelden (Details in der Coaching-Doku).

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Die Community-Tools speichern nur die nötigen Zustände serverseitig: Tags, Clip-Einsendungen, Survey-Antworten, FAQ-Sessions und LFG-relevante Aktivitätsdaten. LFG kombiniert Discord-Status, Voice-Historie, Steam-Präsenz und vorhandene Tags zur Laufzeit; dazu kommen Cooldowns und ein Entscheidungs-Log. FAQ-Chat, Feedback-Hub, Clip-Panel und Leave-Survey arbeiten mit persistenten Buttons, damit sie Neustarts überleben.

## Grenzen & häufige Fragen
- `/meine-tags` ändert nur deine eigenen sichtbaren Tags. Mod-Tags wie `ragebaiter` setzt das Team (`/mod-tag`), nicht du selbst.
- LFG reagiert nur im <#1376335502919335936>-Umfeld, nur wenn du nicht schon im Voice bist, und nur wenn die Nachricht wirklich wie eine Mitspielersuche aussieht. Reiner Smalltalk wird ignoriert.
- Einen automatischen Mitspieler-Vorschlag für Leute, die schon in einer Lane sitzen (Player-Finder), gibt es aktuell nicht — das Feature ist abgeschaltet.
- LFG-Vorschläge hängen stark an Rangdaten, Steam-Link und Aktivität. Ohne Link oder mit sehr wenig Historie werden Antworten ungenauer oder konservativer.
- Es gibt keinen `/ticket`-Befehl — Tickets laufen über den Button in <#1459628609705738539>.
- Clip-Einsendungen haben einen 60-Sekunden-Cooldown pro Person und der Link muss wie eine echte URL aussehen.
- Der Clip-Dump ist ein Sammel-Export, kein sofortiges öffentliches Posting deines Clips.
- Leave-Surveys kommen nicht unbegrenzt oft. Nach einem kürzlich gesendeten Survey, bei Bans oder bei geschlossenen DMs wird nichts mehr nachgeschoben.
- Feedback-Hub ist anonym im Sinne der Weitergabe. Der Bot speichert aber weiterhin die technische Referenz, damit die Nachricht intern zugeordnet und zugestellt werden kann.
- Wer generell keine Bot-DMs oder Datenspeicherung mehr will: `/datenschutz` (Export/Löschen/Opt-out) und `/retention-optout` (keine Erinnerungs-DMs) sind der Self-Service dafür.

## Für Devs (knapp)
- Rust live: `dl-community/src/tags_ui.rs` + `tags.rs` (User-/Mod-Tags), `feedback_hub.rs`, `clips.rs`, `leave_survey.rs`, `faq.rs` (FAQ-Chat + interner Ticket-Shadow), `dm_assistant.rs` (DM-Hilfe), `dl-activity/src/lfg.rs` (Intent, Cooldown, Co-Player, Decision-Log)
- Player-Finder (`dl-activity/src/player_finder.rs`) existiert im Code, ist aber per Default deaktiviert (`PLAYER_FINDER_ENABLED=false`)
- Bug-Reporter (`cogs/bug_reporter.py`) wurde beim Rust-Cutover bewusst nicht portiert
