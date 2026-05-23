# Community-Tools

## Worum geht es?
Diese Doku bündelt die Community-Helfer außerhalb der reinen Voice-Steuerung: Tags, Bug-Tickets, LFG-Matching, anonymes Feedback, Clip-Einsendungen, lane-basierte Mitspieler-Vorschläge und die Leave-Survey nach einem Server-Austritt. Die Features helfen dir dabei, schneller die richtigen Leute zu finden, Probleme sauber zu melden und Feedback ohne Umwege loszuwerden.

## Wie nutze ich das?
- **Tags-System:** Nutze `/meine-tags`. Dort kannst du für dich `25+` oder `U25` sowie `Banter-OK` oder `Ragebaiter-Free` setzen und speichern. Diese Angaben werden später von LFG- und TempVoice-Filtern genutzt.
- **Bug-Reports:** Nutze `/ticket` oder den Button im Channel `#ticket-eroeffnen`. Du wählst eine Kategorie, beschreibst das Problem im Modal und bekommst danach einen privaten `ticket-...`-Kanal. Für technische Kategorien versucht Codex direkt eine erste Lösung oder einen Workaround.
- **LFG-Matching:** Wenn du **nicht** im Voice sitzt, schreibst du in `#spieler-suche` einfach Dinge wie `suche +2 für Ranked`, `wer bock auf Chill?` oder `lfm für Street Brawl`. Der Bot antwortet mit passenden Lobbys, Rang-Hinweisen und einem Vorschlag, welche Staging-Lane du selbst aufmachen solltest.
- **Player-Finder:** Wenn du **schon** mit 1 bis 4 Leuten in einer überwachten Voice-Lane sitzt und dann in `#spieler-suche` postest, sucht der Bot gezielt passende Einzelspieler für genau eure Lane. Dabei werden Steam-Status, Aktivität und grober Rang berücksichtigt.
- **LFG-Filter per Text:** Schreibst du in deiner LFG-Nachricht `25+` oder `ragebaiter free`, filtert der Bot Kandidaten entsprechend strenger.
- **Feedback-Hub:** Im `#feedback-kanal` klickst du auf `Anonymes Feedback senden`. Danach beantwortest du bis zu fünf Fragen zu Spielerlebnis, Server, Verbesserungen und Wünschen. Deine Nachricht wird anonym intern weitergereicht.
- **Clip-Submission:** Im `#clip-submission` klickst du auf `Clip einsenden`, bestätigst die Nutzungserlaubnis und füllst dann Link, Credit/Username und optionale Infos aus. Mindestqualität ist 1080p.
- **Clip-Wochenfenster:** Einsendungen laufen in einem Wochenfenster von Sonntag bis Samstag. Nach Ablauf wird gesammelt ein Wochen-Dump erzeugt.
- **Leave-Survey:** Wenn du den Server verlässt, kann dir der Bot per DM eine kurze Austrittsumfrage schicken. Du wählst zuerst einen Grund aus und bekommst danach eine Folgefrage; für längeres Feedback gibt es zusätzlich einen Web-Link.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Die Community-Tools speichern nur die nötigen Zustände serverseitig: Tags, Ticket-Metadaten, Clip-Einsendungen, Survey-Antworten und LFG-relevante Aktivitätsdaten. LFG und Player-Finder kombinieren Discord-Status, Voice-Historie, Steam-Präsenz und vorhandene Tags, um Vorschläge zur Laufzeit zu berechnen. Ticket, Feedback-Hub, Clip-Panel und Leave-Survey arbeiten mit persistenten Buttons oder Views, damit sie Neustarts überleben. Zeitfenster, Cooldowns und Folge-DMs laufen über Hintergrund-Tasks oder Event-Listener.

## Grenzen & häufige Fragen
- `/meine-tags` ändert nur deine eigenen sichtbaren Tags. Mod-Tags wie `ragebaiter` setzt du nicht selbst.
- LFG und Player-Finder reagieren nur im `#spieler-suche`-Umfeld und nur, wenn die Nachricht wirklich wie eine Mitspielersuche aussieht. Reiner Smalltalk wird ignoriert.
- Der große Lobby-Finder und der lane-basierte Player-Finder sind bewusst getrennt: außerhalb vom Voice bekommst du Lobby-Vorschläge, innerhalb einer Lane eher Mitspieler-Vorschläge.
- LFG-Vorschläge hängen stark an Rangdaten, Steam-Link und Aktivität. Ohne Link oder mit sehr wenig Historie werden Antworten ungenauer oder konservativer.
- Beim Ticket-System landen nicht alle Kategorien direkt im Auto-Fix. Manche Tickets werden nur aufgenommen und anschließend manuell weiterverfolgt.
- Clip-Einsendungen haben einen 60-Sekunden-Cooldown pro Person und der Link muss wie eine echte URL aussehen.
- Der Clip-Dump ist ein Sammel-Export, kein sofortiges öffentliches Posting deines Clips.
- Leave-Surveys kommen nicht unbegrenzt oft. Nach einem kürzlich gesendeten Survey, bei Bans oder bei geschlossenen DMs wird nichts mehr nachgeschoben.
- Feedback-Hub ist anonym im Sinne der Weitergabe. Der Bot speichert aber weiterhin die technische Referenz, damit die Nachricht intern zugeordnet und zugestellt werden kann.

## Für Devs (knapp)
- `cogs/tags/core.py`, `interface.py`, `mod_commands.py`: User-Tags und Mod-Tags, inkl. `/meine-tags`. Tabellen: `user_tags`, `user_mod_tags`.
- `cogs/bug_reporter.py`: Ticket-Modal, privater Ticket-Channel und Codex-Autoreply. Nutzt `service.issue_reports` und `kv_store`.
- `cogs/lfg.py`: Haupt-Lobby-Finder für `#spieler-suche`, inklusive Rang- und Textfilter. Nutzt `steam_links`, `live_player_state`, `user_activity_patterns`, `user_co_players`.
- `cogs/player_finder.py`: Mitspieler-Vorschläge für bereits laufende Voice-Lanes. Nutzt `voice_session_log`, `user_activity_patterns`, `steam_links`, `live_player_state`.
- `cogs/feedback_hub.py`: anonymer Feedback-Button und Formular. Nutzt `kv_store` für die persistente Interface-Nachricht.
- `cogs/clip_submission.py`: Clip-Interface, Wochenfenster und Dump-Export. Tabellen: `clip_submissions`, `clip_windows`, `clip_window_submissions`, `persistent_views`.
- `cogs/leave_survey.py`: Exit-DM mit Dropdown, Follow-up-Modal und Web-Link. Tabellen: `member_leave_surveys`, `member_events`, `voice_stats`, `voice_session_log`, `message_activity`, `user_retention_tracking`.
