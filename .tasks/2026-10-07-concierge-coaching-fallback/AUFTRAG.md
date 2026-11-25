# Auftrag: Concierge spricht als Concierge, Coaching-Verweis, Nani-Fallback nur für Bot-Fragen

Auftraggeber: Haupt-Orchestrator (Claude-Hauptsession, Thread d3a1741e-82bc-4a48-865b-2845c663dca7)
Rolle: Blatt-Worker. Gate-Fixschleife fährst du selbst (frische Fixer-Subagenten bis ALLOW, Regel claude-config `orchestrierung/ABLAUF.md` Schritt 7). Keine T3-Threads starten. Meldung erst bei Abschluss oder wenn du feststeckst.
Stufe: mittel. Repo: Deadlock-Bots (plus Deadlock-Docs-Korpus und Deadlock-Brain, falls die Antwort dort entsteht). Eigener Worktree je Repo von origin/main, Branch `fix/concierge-coaching-fallback`.

## Befund vom Nutzer (Discord, 2026-10-07, #bot-logs)

1. "@Bot wie countert man Pocket" → "Dazu habe ich gerade keine gesicherten Infos. Frag bitte Nani hier im Discord." Dieser Nani-Fallback war nur für Fragen zum Twitch-Bot gedacht, nicht für Spielfragen. Hart codiert in `rust/bin/dl-bot/src/modglue.rs` (um Zeile 718 und 4797). Spielfragen gehören zum Spielwissen (Brain/Knowledge), und wenn dort nichts Gesichertes kommt, zum vorhandenen Spiel-Fallback (`modglue.rs:47`, "Frag mich was zum Spiel ..."), nie zu "Frag Nani". Klären, warum diese Frage im Nani-Zweig landete (Klassifizierung, Brain gerade im Umbau?), Ursache beheben.
2. "@Bot ich bin schlecht in dem Game was soll ich tun" → "... Schreib dem Concierge, wenn du gern einen Paten hättest." Der Bot IST der Concierge und darf nicht in dritter Person auf sich verweisen. Beim Coaching-Wunsch soll er:
   - in Discord den Kanal verlinken: `<#1494373349944459355>` (🛠️ich-brauch-einen-coach),
   - außerhalb von Discord (Twitch-Chat und andere Plattformen) nach Discord bzw. auf https://deutsche-deadlock-community.de/coaching verweisen.
   Pate-Angebot nur in Ich-Form ("Sag mir Bescheid, wenn du einen Paten willst").

## Vorgehen

- Graphify zuerst, Bestand suchen: Concierge-Prompt und Stilvertrag (`dl-community/src/concierge.rs`), Wissenskorpus (Deadlock-Docs `public/discord-server/paten.html`, `dm-concierge.html`, `faq-bot-selbst.html` u. a. formulieren "Schreib dem Concierge"), `dl-knowledge`, Twitch-Antwortwege im Brain. Regel des Nutzers: alle Nutzerantworten laufen über das eine Brain; keinen neuen Bot-eigenen Antworttext bauen, sondern die zentrale Stelle korrigieren (Memory `ein-brain-statt-bot-antworten`). Liegt der Antwortweg gerade im Brain-Umbau, dort ansetzen und den Stand im Bericht nennen.
- Kanalverweis als Discord-Mention aus einer Konfiguration/Konstante, nicht als freier LLM-Text; Plattform (Discord vs. Twitch) entscheidet Mention oder Link.
- Doku-Korpus: Sätze, die den Concierge in dritter Person nennen und vom Bot wörtlich übernommen werden, so umformulieren, dass der Bot sie in Ich-Form nutzen kann; öffentliche Seiten bleiben für Leser verständlich.
- Texte über Skills `rolle-doku-redakteur` und `no-em-dashes`, echtes Deutsch mit Umlauten, nicht bottig.
- Keine Code-Kommentare. Kein Python erweitern.

## Abschluss (Agent 1)

fmt/clippy/betroffene Tests, Selbstprüfung `gate_hook.py --review`, Merge-Gate mit eigener Fixschleife bis ALLOW, Merge per `git push origin HEAD:main`, Deploy und Neustart der betroffenen Dienste (dl-bot per `systemctl --user restart`, Brain/Docs nach deren Weg), Live-Beweis mit beiden Beispielfragen im Testkanal (#bot-logs): Pocket-Frage bekommt Spielantwort oder Spiel-Fallback, Coaching-Frage verlinkt `<#1494373349944459355>` ohne "dem Concierge". Branches und Worktrees löschen, Bericht an den Haupt-Orchestrator, `t3-thread.py settle --selbst`.
