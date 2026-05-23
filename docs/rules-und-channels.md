# Rules und Channels

## Worum geht es?
Das Regelwerk ist nicht nur eine Textseite, sondern der Einstieg in den ganzen Server. Von dort aus startet dein Onboarding, und viele der wichtigsten Channels sind so aufgebaut, dass sie direkt mit Bot-Features zusammenarbeiten.

## Wie nutze ich das?
Lies zuerst `#regelwerk` und starte dort mit `Hier starten` dein Onboarding. Der Bot legt dir dafur einen eigenen Onboarding-Thread an, fuhrt dich durch Regeln, Voice-Lanes und wichtige Bereiche und schaltet je nach Auswahl weitere sinnvolle Wege frei. Wenn Discord-Member-Screening aktiv war, kann dieser Start auch automatisch passieren.

Fur Mitspieler und Voice ist die Grundstruktur relativ klar:
`#spieler-suche` ist fuer die Textsuche nach Mitspielern.
`#sprach-kanal-verwalten` ist das Panel, ueber das du eigene Lanes oeffnest und verwaltest.
`#rang-auswahl` hilft bei der Einordnung fuer Ranked- und Matchmaking-nahe Features.

Fur Support und Orientierung sind diese Kanale die wichtigsten:
`#ticket-eroeffnen` fuer Support-Faelle und Moderationsanliegen.
`#feedback-kanal` fuer ehrliches offenes Feedback.
`#ich-brauch-einen-coach` fuer kostenlose Coaching-Anfragen.
`#beta-zugang` fuer Deadlock-Zugang und den `/betainvite`-Flow.

Dazu kommen die Content- und Community-Bereiche:
`#patchnotes` fuer Patch-Zusammenfassungen.
`#clip-submission` fuer Highlight-Einsendungen.
`#twitch` fuer Live-Hinweise aus dem Streamer-Bereich.
`#custom-games-chat` und der Voice-`#Sammelpunkt` fur Custom Games.

Auch bei den Voice-Kategorien gibt es sichtbare Unterschiede. Es gibt oeffentliche oeffnen-Channels fuer Ranked/Competitive, Spass, Street Brawl und Neue-Spieler-Lanes. Ausserdem gibt es eine eigene Coaching-Voice-Kategorie, in der Coaching-Sessions erkannt und abgeschlossen werden.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Das Regelwerk-Panel ist eine persistente Nachricht mit Start-Button. Beim Start versucht der Bot zuerst, einen privaten Thread im Regelkanal zu erzeugen; wenn Discord das nicht hergibt, faellt er auf einen oeffentlichen Thread zurueck. Das Onboarding selbst ist schrittbasiert, kann optionale Praeferenzen wie Voice-Ton oder Altersgruppe speichern und verzahnt sich mit weiteren Features wie Steam-Linking, Streamer-Setup und Beta-Zugang.

## Grenzen & häufige Fragen
- Wenn dir bestimmte Kanale fehlen, ist fast immer das Onboarding oder die Rollen-Auswahl nicht komplett.
- `#regelwerk` ist nicht nur Info-Text. Wer den Start-Button ignoriert, verpasst oft genau die Hinweise, die spaeter im Ticket landen.
- Support, Coaching, LFG und Build-Fragen haben bewusst getrennte Orte. Das macht Antworten schneller und sauberer.
- Private Onboarding-Threads sind bevorzugt, aber nicht garantiert. Bei Discord-Problemen kann ein oeffentlicher Fallback entstehen.
- Einige Voice-Bereiche sind rein zweckgebunden, zum Beispiel die Coaching-Voices. Sie sind nicht dasselbe wie normale LFG-Lanes.
- Der FAQ-Bot kann dir zwar sagen, wohin du musst, aber er schaltet keine Rollen oder Kanale selbst frei.

## Für Devs (knapp)
- Cog: `cogs/rules_channel.py`
- Abhangigkeiten: `cogs/onboarding.py`, Welcome-DM- und Streamer-Onboarding-Komponenten, weitere Channel-Namen aus Onboarding-Kontexten und Embeds
- Wichtige DB-Tabellen: keine eigene Fach-Tabelle; Persistenz laeuft vor allem ueber abhängige Onboarding- und KV-Mechaniken
