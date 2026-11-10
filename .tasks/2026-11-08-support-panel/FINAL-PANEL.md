# Final support-panel content

The German copy below comes from `assets/support_texts.toml`. The publisher sends three Components V2 messages. The existing banner appears inside the first gold container. Separators divide the text sections. The Ko-fi link button appears on the payment card. Stripe is disabled by default.

## Message 1: funding

# Server unterstützen

Ich baue einen großen Teil von dem, was hier läuft, selbst. Steam Bot, Twitch Bot, Patchnotes, Deadlock Brain, die Website, Turniere und die übrigen Community-Systeme. Das sind jede Woche viele Stunden, und der Betrieb kostet laufend Geld.

## Warum ich Unterstützung brauche

Server, APIs, Infrastruktur und die KI-Coding-Abos liegen je nach Monat ungefähr bei **50 bis 250 €**, manchmal auch darüber. 2026 sind bis zum 8. November schon **über 1.600 €** zusammengekommen. Meine Arbeitszeit ist darin nicht enthalten.

Ich habe mir oft überlegt, wie sich das langfristig tragen lässt. So wie bisher kann ich die Kosten nicht dauerhaft alleine stemmen.

## Zu den KI-Abos

Die Abos kosten Geld. Bei diesem Umfang sind sie aber deutlich günstiger, als dieselbe Arbeitszeit auch nur zum Mindestlohn an Entwickler zu zahlen.

Die Tools entwickeln die Projekte nicht alleine. Planung, Architektur, Umsetzung, Tests, Fehlersuche und Betrieb bleiben bei mir. Die Abos machen diesen Umfang überhaupt erst bezahlbar.

Was an Unterstützung reinkommt, geht zurück in Infrastruktur und Entwicklung. Gewinn ist nicht das Ziel.


## Message 2: benefits

## Steam Bot

Der Steam Bot wird auf ein Supporter-Modell umgestellt. Das gilt für die Steam-Verknüpfung, die automatischen Rang-Rollen, Rang-Abfragen, Match-Abfragen und die weiteren Funktionen des Bots.

Die Anbindung ist aufwendig. Deadlock ändert sich, und die Pflege läuft mit.

Geplant sind erweiterte Match-Abfragen, ähnlich dem priorisierten Abruf bei der Deadlock API: https://deadlock-api.com/patron
Echtzeit verspreche ich nicht. Was möglich ist, hängt von Steam und dem Game Coordinator ab.

## Deadlock Brain

Für Supporter ist vorgesehen, das Deadlock Brain ohne zusätzliches Abo und ohne Nutzungslimit zu nutzen. Das ist noch nicht eingerichtet. Bis dahin gilt das normale Tageslimit.

Geplant, und noch nicht fertig, ist ein Build-Helfer. Du wählst einen Helden und bekommst sinnvolle Items und Builds, dazu eine Erklärung, warum ein Item passt. Du kannst gegen bestimmte Gegner bauen. Die Vorschläge sollen sich an Patches anpassen. Der Helfer soll beim Verstehen helfen, die Entscheidung bleibt bei dir.

## Auf Discord

Supporter bekommen eine eigene Rolle, private Supporter-Channels im VIP-Bereich und Zugang zum Streamer-VC.

Geplant, und noch nicht eingerichtet, sind Einblicke in das, was gerade gebaut wird, Abstimmungen über neue Funktionen und früherer Zugang zu Betatests.

## Was noch kommt

**Patch-Watchlist.** Du suchst dir Helden und Items aus und bekommst Bescheid, wenn ein Patch genau daran etwas ändert.

**Weitere Match-Abfragen** im Steam Bot, soweit sie sich zuverlässig bauen lassen.

Ein eigenes Spielerprofil baue ich nicht. Dafür gibt es Statlocker und ähnliche Seiten. Eine eigene Funktion für Tierlisten und Builds gibt es ebenfalls nicht.

## Was kostenlos bleibt

Der Server wird nicht kostenpflichtig. Kostenlos bleiben Mitspieler suchen, die normalen Voice-Channels, Community-Aktivitäten, reguläre Turniere, Scrims und das normale Coaching.

Bei Turnieren und Coaching bringt ein Abo keinen Vorteil.


## Message 3: support options

## So kannst du unterstützen

**Discord-Server-Abo.** Über den Servernamen, dort wo Discord die Server-Abos anbietet.

**Server-Boost.** Zählt vorerst ebenfalls als Unterstützung.

**Ko-fi.** Einmalig oder monatlich. Dafür gibt es die Unterstützer-Rolle für 31 Tage. Nach dem Eingang schalte ich sie manuell frei, in der Regel innerhalb eines Tages.

**Stripe-Abo für den Server.** Ein eigenes Abo, getrennt von den Abos des Steam Bots und des Twitch Bots. Der Button erscheint erst, wenn die Zahlung deinem Discord-Account zugeordnet wird und die Rolle nach Ablauf oder Kündigung wieder entzogen wird.

**Twitch-Bot-Abo.** Der Twitch Bot hat eigene kostenpflichtige Abos. Wer so ein Abo hat, soll dieselben Discord-Supporter-Vorteile bekommen, ohne ein zweites Abo. Die automatische Freischaltung ist noch nicht eingerichtet. Bis dahin reicht eine kurze Nachricht, dann trage ich es manuell ein.

Danke, dass du den Server und die Projekte mitträgst.

## Buttons and release prerequisites

- Existing Ko-fi button: **Über Ko-fi unterstützen**, `https://ko-fi.com/deutschedeadlockcommunity`.
- New optional Stripe button: **Supporter-Abo abschließen**. It requires an explicit enabled setting and a valid `buy.stripe.com` payment link. No link is configured.
- Discord subscription and Twitch checkout buttons are absent because no suitable verified links exist in this implementation.

Before publication, resolve Discord role access, the Steam Plus copy that still promises free linking and queries, the Brain supporter exemption, and the payment integrations listed in [README.md](README.md). Stripe account binding and automatic grant/revoke, plus Twitch subscription synchronization, remain unimplemented. These are release prerequisites, not features delivered by this change. No deployment or Discord post has been authorized.
