# Design: Ticket-Shadow zeigt nur brauchbare Antwortkandidaten

**Datum:** 2026-07-25
**Status:** Zur Review
**Betrifft:** Rust-Ticket-Auto-Hilfe in `dl-community`

## Problem

Die Ticket-Auto-Hilfe läuft absichtlich dauerhaft im Shadow-Modus. Aktuell
erzeugt sie jedoch auch für die Entscheidungen `no` und `uncertain` eine
generische Nicht-Antwort im internen Discord-Logkanal. Solche Nachrichten
helfen weder dem Supportteam noch bei der Beurteilung, ob der Bot später eine
sinnvolle erste Anlaufstelle sein könnte.

Der verlinkte Produktionsfall wurde korrekt als `no` bewertet und nicht ins
Ticket gesendet. Das unerwünschte Verhalten ist ausschließlich der
anschließende generische Shadow-Post.

## Ziel

Der Shadow-Kanal enthält nur konkrete Antworten, die der Bot tatsächlich als
erste Hilfe vorgeschlagen hätte. Kann der Bot keine belastbare Antwort geben,
bleibt er in Discord vollständig still. Die technische Entscheidung bleibt
für jede Anfrage im Journal nachvollziehbar, damit keine blinden Flecken
entstehen.

## Fester Verhaltensvertrag

| Knowledge-Ergebnis | Journal | Shadow-Kanal | Ticket |
|---|---|---|---|
| `answered` mit nichtleerem Text | Entscheidung loggen | Antwortkandidat posten | nichts senden |
| `no` | Entscheidung loggen | nichts senden | nichts senden |
| `uncertain` | Entscheidung loggen | nichts senden | nichts senden |

Der Ticket-Helfer bleibt dauerhaft im Shadow-Modus:

- Das feste Ziel bleibt ausschließlich der interne, freigegebene Logkanal.
- Es gibt keinen automatischen Cutover und keinen Konfigurationsschalter für
  direkte Ticket-Antworten.
- Der Ticket-Kanal ist unter keinem Ergebnis ein Sendeziel.
- Ein späterer Live-Modus ist ausdrücklich nicht Teil dieses Designs.

## Umsetzung

Die bestehende Klassifikation `answered` / `no` / `uncertain` bleibt
unverändert. Nach dem strukturierten Entscheidungslog beendet
`handle_ticket_message` den Pfad sofort, wenn kein Antworttext vorhanden ist.
Nur ein vorhandener, nichtleerer Antworttext wird als Shadow-Nachricht
formatiert und an den fest verdrahteten Logkanal gesendet.

Der MCP-Server wird nicht geändert. Er ist nicht Teil des automatischen
Ticket-Antwortpfads.

## Fehlerbehandlung und Beobachtbarkeit

- Fehlende oder falsche Shadow-Konfiguration bleibt fail-closed: kein
  Knowledge-Aufruf und kein Discord-Post.
- Timeouts, Transportfehler und ungültige Antworten bleiben `uncertain` und
  damit Discord-still.
- `answered`, `no` und `uncertain` werden weiterhin mit Ticket-Kanal,
  Autor-ID und vorhandenen Entscheidungsfeldern geloggt.
- Privater Tickettext wird nicht zusätzlich in Logs geschrieben.

## Tests

- `answered` erzeugt genau einen Post im festen Shadow-Kanal und keinen Post
  im Ticket.
- `no` ruft Knowledge auf, protokolliert die Entscheidung und sendet keine
  Discord-Nachricht.
- `uncertain` protokolliert die Entscheidung und sendet keine
  Discord-Nachricht.
- Fehlender, identischer oder nicht freigegebener Shadow-Kanal bleibt
  fail-closed.

## Nicht-Ziele

- Keine neue Qualitäts-KI oder zweite Bewertungsschicht.
- Keine Änderung an `dl-knowledge`.
- Keine direkte oder automatische Ticket-Antwort.
- Keine Änderung am MCP-Server.
