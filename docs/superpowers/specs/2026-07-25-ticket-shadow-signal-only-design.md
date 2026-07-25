# Design: Ticket-Shadow trennt Urteil und Antwortkandidat

**Datum:** 2026-07-25
**Status:** Zur erneuten Review
**Betrifft:** Rust-Ticket-Auto-Hilfe in `dl-community`

## Problem

Die Ticket-Auto-Hilfe läuft absichtlich dauerhaft im Shadow-Modus. Der
aktuelle Pfad vermischt jedoch zwei unterschiedliche Fragen:

1. Gibt es genug belastbares, öffentliches Wissen für eine sichere Antwort?
2. Welche erste Antwort würde der Bot dem User konkret formulieren?

`dl-knowledge` beantwortet aktuell nur die erste Frage. Das Modell wählt
belegende Passagen; bei `no` liefert der FAQ-Pfad anschließend lediglich einen
festen generischen Nicht-Antwort-Text. Damit lässt sich nicht beurteilen, ob
der Bot bei persönlichen, technischen oder moderativen Anliegen eine
brauchbare erste Reaktion formulieren könnte.

## Ziel

Für jede erste, gültige Ticket-Nachricht entstehen intern zwei getrennte
Ergebnisse:

1. **Urteil:** `yes`, `no` oder `uncertain` aus dem bestehenden
   Knowledge-Pfad.
2. **Antwortkandidat:** eine unabhängig erzeugte, bestmögliche erste Antwort
   auf genau diese Nachricht.

Der Antwortkandidat wird auch bei `no` oder `uncertain` versucht. Nur so wird
im Shadow sichtbar, was der Bot tatsächlich formulieren würde und wo Prompt,
Wissen oder Modell verbessert werden müssen.

Beide Ergebnisse bleiben ausschließlich intern. Im Ticket selbst sendet der
Bot niemals etwas.

Für die erste gültige Nachricht wird pro Ticket dauerhaft höchstens ein
Auswertungsversuch gestartet. Nach erfolgreicher Shadow-Konfiguration
beansprucht der Bot die Ticket-ID atomar im vorhandenen zentralen KV-Store.
Dieser Claim überlebt Bot-Neustarts und bleibt auch dann bestehen, wenn
Generator oder Shadow-Post später fehlschlagen.

## Fester Verhaltensvertrag

| Urteil | Kandidatengenerierung | Shadow-Kanal | Ticket |
|---|---|---|---|
| `yes` | immer versuchen, mit sicherem Knowledge-Kontext | Urteil und Kandidat | nichts senden |
| `no` | immer versuchen, ohne erfundene Fakten | Urteil und Kandidat | nichts senden |
| `uncertain` | immer versuchen, Unsicherheit sauber behandeln | Urteil und Kandidat | nichts senden |
| beliebig + Generatorfehler | Fehler sichtbar protokollieren | Urteil und knapper Fehlerstatus | nichts senden |

Der Ticket-Helfer bleibt dauerhaft im Shadow-Modus:

- Das feste Discord-Ziel bleibt ausschließlich der interne, freigegebene
  Logkanal.
- Es gibt keinen automatischen Cutover und keinen Konfigurationsschalter für
  direkte Ticket-Antworten.
- Der Ticket-Kanal ist unter keinem Ergebnis ein Sendeziel.
- Aus Shadow-Ergebnissen folgt niemals automatisch eine Aktion.

## Architektur

### Restartfester One-shot

Vor Knowledge und Generator prüft der Ticket-Pfad in dieser Reihenfolge:
Guild, Ticket-Kategorie, nichtleeren getrimmten Text und die feste
Shadow-Konfiguration. Erst danach beansprucht er die Ticket-ID mit einem
atomaren `INSERT ... ON CONFLICT DO NOTHING` in `bot.kv_store`.

Nur ein neu angelegter Claim setzt die Auswertung fort. Ein bereits
vorhandener Claim beendet den Pfad still, auch in einer frisch gestarteten
Bot-Instanz. Ein Claim-Fehler stoppt vor Knowledge, Generator und Discord und
wird ausschließlich mit IDs sowie einem festen maschinenlesbaren Fehlerstatus
geloggt. Der Claim wird nach späteren Fehlern nicht zurückgenommen, damit ein
Ticket höchstens einen Auswertungsversuch erhält.

### Stufe 1: unabhängiges Urteil

Der bestehende Aufruf von `dl-knowledge` bleibt unverändert. Er liefert über
`KnowledgeLookup`:

- `Answer` → `yes` plus sichere öffentliche Passagen,
- `Unanswerable` → `no`,
- Timeout, Transport- oder Antwortfehler → `uncertain`.

Das Urteil wird vollständig geloggt, löst aber weder eine Ticket-Antwort noch
das Unterdrücken der zweiten Stufe aus.

### Stufe 2: unabhängiger Antwortkandidat

`FaqChat` erhält den bereits im Bot verwendeten `MiniMaxClient` über das
bestehende `TextGenerator`-Interface. Nach Stufe 1 wird der Generator immer
aufgerufen. Seine strukturierten Eingaben sind:

- die originale Ticket-Nachricht als nicht vertrauenswürdiges Datenfeld,
- das Urteil aus Stufe 1,
- bei `yes` die sicheren öffentlichen Knowledge-Passagen,
- bei `no` oder `uncertain` kein erfundener Ersatzkontext.

Der Kandidat darf keine Aktionen ausführen und bekommt keine Tools. Bei
fehlendem Wissen soll er trotzdem eine hilfreiche erste Reaktion formulieren:
das Anliegen passend aufnehmen, nichts entscheiden oder erfinden und
höchstens die konkret fehlende Information erfragen. Bei moderativen Fällen
bestätigt er nur die Aufnahme und dass das Team prüft; er verspricht weder
Strafen noch Ergebnisse.

Der vorhandene Discord-Sendepfad mit deaktivierten Mentions bleibt erhalten.

## Sprach- und Qualitätsvertrag

Der Antwortkandidat:

- ist ausschließlich auf sauberem Deutsch und spricht den User mit `du` an;
- klingt locker, ruhig und menschlich, wie ein echtes Community-Teammitglied;
- ist kurz und gut lesbar: normalerweise zwei bis fünf kurze Sätze, keine
  Textwand;
- beginnt direkt mit der hilfreichen Reaktion statt mit einer Überschrift;
- nennt einen konkreten nächsten Schritt oder eine wirklich relevante
  Rückfrage, wenn das möglich ist;
- wiederholt die User-Nachricht nicht unnötig;
- verweist niemals darauf, ein Ticket zu öffnen, weil der User bereits darin
  schreibt;
- behauptet keine Prüfung, Aktion, Strafe, Ursache oder Account-Information,
  die nicht tatsächlich bekannt ist;
- enthält keine internen Begriffe, Modellnamen, Quellenpfade oder
  Systemerklärungen;
- vermeidet typische KI-Floskeln wie „Gerne!“, „Natürlich!“, „Als KI“,
  „Zusammenfassend“ oder künstliche Standard-Empathie;
- verwendet keine Emojis, Marketing-Sprache oder übertriebene Förmlichkeit.

Das Modell gibt ausschließlich den Antworttext aus. Urteil und technische
Metadaten ergänzt der Bot selbst.

## Shadow-Ausgabe

Pro geprüftem Ticket entsteht genau ein interner Shadow-Eintrag:

```text
🧪 FAQ-Shadow
Urteil: no
Ticket: <#...>

Kandidat:
<natürlicher Antworttext>
```

Kann der Generator technisch keinen Kandidaten liefern, ersetzt ein kurzer
interner Status den Kandidatentext. Dadurch bleiben auch Ausfälle sichtbar,
ohne eine falsche Nutzerantwort vorzutäuschen.

## Fehlerbehandlung und Beobachtbarkeit

- Fehlende oder falsche Shadow-Konfiguration bleibt fail-closed: kein
  Claim, kein Knowledge-Aufruf, keine Generierung und kein Discord-Post.
- Persistenzfehler beim Ticket-Claim bleiben fail-closed und loggen nur IDs,
  `reason` und `error_class`, niemals Ticket-, Kandidaten- oder Fehlerrohtext.
- Stufe 1 loggt weiterhin jedes Urteil samt vorhandener Confidence-, Quellen-
  und Fehlerfelder.
- Stufe 2 loggt `generated`, `timeout`, `empty` oder `unavailable`.
- Privater Tickettext wird nicht zusätzlich in das Journal geschrieben.
- Der interne Shadow-Eintrag enthält nur den formulierten Kandidaten und den
  Link zum ursprünglichen Ticket, nicht noch einmal den Rohtext.

## Tests

- `yes` ruft den Generator mit sicherem Knowledge-Kontext auf und postet
  Urteil plus Kandidat ausschließlich im Shadow-Kanal.
- `no` ruft den Generator trotzdem auf und postet Urteil plus Kandidat
  ausschließlich im Shadow-Kanal.
- `uncertain` ruft den Generator trotzdem auf und macht die Unsicherheit im
  Prompt verfügbar, ohne eine Tatsache zu erfinden.
- Generatorfehler erzeugen einen knappen internen Fehlerstatus und niemals
  eine Ticket-Nachricht.
- Der Prompt serialisiert die Ticket-Nachricht als nicht vertrauenswürdige
  Daten und enthält den Sprach- und Qualitätsvertrag.
- Fehlender, identischer oder nicht freigegebener Shadow-Kanal bleibt
  fail-closed.
- Zwei frische `FaqChat`-Instanzen mit gemeinsamem Claim-State werten dasselbe
  Ticket zusammen genau einmal aus.
- Der atomare KV-Claim liefert beim ersten Insert `true`, beim Konflikt
  `false` und überschreibt den ersten Wert nicht.
- Claim-Persistenzfehler stoppen vor Knowledge, Generator und Discord und
  bleiben im Journal redigiert sichtbar.
- Die Produktionskonfiguration bleibt fest auf den internen Logkanal
  verdrahtet.

## Nicht-Ziele

- Kein Live-Modus und keine direkte Ticket-Antwort.
- Keine automatische Qualitätsfreigabe oder zweite Bewertungs-KI.
- Keine Aktionen, Diagnosen oder Tool-Aufrufe durch den Kandidatengenerator.
- Keine Änderung an der Answerability-Entscheidung von `dl-knowledge`.
- Keine automatische Beförderung guter Shadow-Antworten in den Live-Betrieb.
