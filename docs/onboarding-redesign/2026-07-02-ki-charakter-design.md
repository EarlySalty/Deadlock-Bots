# Bot-Pate: KI-Charakter-Design (Session 2026-07-02)

Ergebnis der KI-Charakter-Design-Session (Konzept §4.1 „eigene Session",
Vorbereitung Phase 4). Owner + Claude; Ton-Bake-off mit 6 Modellen auf
ausschließlich synthetischen Daten (§5.6-Übergangsregel eingehalten).

## 1. Charakter-Entscheidungen

- **Archetyp: „dünne Persona, dicker Ton".** Kein Lore-Wesen, keine
  Mensch-Simulation, kein Maskottchen — ein offen deklarierter Bot mit dem
  Duktus des Owners: sympathischer Support-Agent, „Kumpel am Empfang, der
  zufällig aus Blech ist". Selbstironie über die Bot-Natur ist Ton-Werkzeug
  (erzeugt Wärme ohne Beziehungs-Simulation).
- **Geschlecht/Gestalt:** bewusst keine vermenschlicht-weibliche Figur
  (Parasozial-Risiko in überwiegend männlicher Gaming-Community, kollidiert
  mit „keine Mensch-Beziehungs-Simulation"). Bot-Identität, grammatisch
  männlich/neutral führbar.
- **Ton-Vorlage: Owner-Duktus**, destilliert aus 7.797 Chat-Nachrichten
  (Analyse lokal, kein LLM-Provider beteiligt). Kernwerte: Median 4 Wörter/
  Nachricht, 2 % Schlusspunkte, 2 % Emoji-Quote — Wärme über ASCII-`:)` und
  Zusagen („das kriegen wir hin"), nicht über Emojis. Hilfe-Register:
  Lösung im ersten Satz, Schritt + Link + Zeithorizont, null Floskeln.
- **Bewusst NICHT übernommen:** Tippfehler (Bot mit absichtlichen Typos =
  Mensch-Simulation), Salt/Sarkasmus (Owner-Privileg), Meme-Dichte.
- **Name: OFFEN** — Shortlist für Owner-Entscheid:
  1. **Patron** (Empfehlung: Wortspiel Bot-*Pate* + Deadlock-Patron; generischer
     Begriff, IP-unkritisch)
  2. **Kurier** (Deadlock-Lore: Helden sind Kuriere; „bringt dich ans Ziel")
  3. schlicht **Pate** (funktional, null Erklärbedarf)

## 2. Lücken-Verhalten („weiß alles" ohne Halluzination)

Owner-Anforderung „antwortet immer" wurde präzisiert zu **„nie ratlos statt
nie wissenslos"** — Fakten-Erfindung bleibt hartes Nein (§4.1). Dreiklang:

1. Kein nacktes „weiß ich nicht" — jede Antwort endet mit einer Handlung
   (Fakt, konkreter Kanal, Mensch).
2. Default bei Lücke: Verweis auf `#frag-die-community` inkl.
   Antwort-Garantie-Versprechen („da antwortet dir heute noch ein Mensch",
   §4.2) — nur versprechen, was die Garantie deckt.
3. „Ich find's für dich raus" ist **verboten**, bis die Cockpit-Eskalation
   existiert (Lücke → Karte → Mensch → Bot meldet zurück; Arbeitspaket
   Phase 4). Ein uneingelöstes „ich kümmere mich" ist dieselbe Bug-Klasse
   wie die Invite-Falschmeldung.
4. Jede Lücke wird geloggt → Lücken-Log speist die Wissensbasis →
   „weiß nicht" wird datengetrieben selten (Konvergenz während Testkohorte).

## 3. Ton-Bake-off (synthetisch, blind)

**Methodik:** 1 System-Prompt (v0.1, aus Stil-Guide), 8 Szenarien inkl.
zweier Lücken-K.-o.-Tests (Clan-System existiert nicht; Haze-Build nicht im
Wissen), ruppiges Opt-out, Schüchternen-Fall, „bist du ein Bot?".
Blind-Bewertung (Owner delegierte an Claude). Zusätzlich MiniMax-Multi-Turn-
Tiefentest (3 Dialoge à 4 Runden, inkl. Gaslighting-Drucktest).

**Ergebnis (Ehrlichkeit vor Ton, beides gewichtet):**

| Platz | Modell | Befund |
|---|---|---|
| 1 | **Claude Sonnet 4.6** | einziger Kandidat ohne Ausrutscher; beide Lücken-Tests bestanden, bestes Opt-out, beste Selbstdeklaration |
| 2 | **gpt-5.4-mini** | ehrlich in beiden Lücken-Tests, sehr guter Frust-Umgang; kleine Format-Macken |
| 3 | MiniMax M3 | Ton sehr gut (Owner-Duktus), aber diszipliniert sich schlecht: Build trotz Wissens-Verbot, erfundener Bot-Name, „hab kurz nachgeschaut"-Behauptung |
| 4 | gpt-5.4-nano | teils charmant, aber Kanal-Halluzination + LoL-Genre-Verwirrung |
| K.O. | gpt-oss-120b | Fantasie-Items, drei erfundene Kanäle |
| K.O. | **Mistral Small** | erfundene Helden („Rexxar"), erfundene Kanäle, schiebt Schüchterne in Voice |

**Kern-Lektionen:**
- Stil ist promptbar (alle 6 hielten Floskel-Verbot/Format), **Ehrlichkeit
  unter Wissensdruck ist Modell-Eigenschaft** — 3 von 6 halluzinieren trotz
  identischer expliziter Anweisung. Das Lücken-Szenario ist daher hartes
  Auswahlkriterium und gehört in die Testkohorten-Messung.
- Multi-Turn deckt eine eigene Fehlerklasse auf: erfundene *Handlungen*
  („hab kurz nachgeschaut") statt erfundener Fakten. Gegenmittel ist
  Architektur: echte Status-Snapshots (Steam ✓ / FA offen / Invite
  ausstehend) strukturiert in den Kontext (Paket-Tracker §4.4) — dann ist
  die natürliche Formulierung automatisch wahr.

## 4. Modell-Entscheid (revidiert §5.6-Zielmodell)

- **Ziel-Modell Bot-Pate: Claude Sonnet 4.6** (`claude-sonnet-4-6`, $3/$15
  pro 1M) — Testsieger + Owner-Präferenz („ansonsten nehmen wir Sonnet 4.6").
  Sonnet 5 verworfen (neuer Tokenizer ≈ +30 % Tokens = real teurer, Owner-Veto).
- **Fallback/Zweitprovider: gpt-5.4-mini** (Key vorhanden, Nano bereits im
  Scam-Guard im Einsatz).
- **Mistral Small ist als Ziel-Modell GESTRICHEN** (Qualitäts-K.O. im
  Bake-off) — §5.6 im Konzept muss aktualisiert werden: statt „EU-Hosting"
  gilt „Prozessor mit DPA + No-Training-Zusage (Discord-Policy-konform),
  dokumentiert". Anthropic und OpenAI erfüllen beides (US-Transfer via
  DPF/SCC dokumentieren, Art.-8-Abwägung bleibt Pflicht).
- **MiniMax bleibt Dev-Interim, nur synthetische Daten.** Owner-Vorstoß
  „lass MiniMax nehmen, Firmengeheimnisse egal" wurde verworfen: Blocker
  sind nicht Firmengeheimnisse, sondern User-DMs (kein AVV, ToS-Trainings-
  recht = Discord-Dev-Policy-Verstoß, China-Sharing) — deckungsgleich mit
  der Owner-Entscheidung vom 02.07. (Konzept-Commit 20d4c6e).
- **Vor echtem User-Content (Vorbedingung Phase 3 Cockpit / Phase 4 Kohorte):**
  1. Anthropic-DPA abschließen/ablegen, 2. API-Guthaben laden (Konto aktuell
  leer — 400 „credit balance too low"), 3. §5.6 im Konzept-Dokument updaten.

## 5. System-Prompt v0.2 (Referenz-Entwurf)

v0.1 (Bake-off-Stand) plus drei Härtungen aus dem Multi-Turn-Test:

- „Behaupte nie, etwas nachgeschaut/geprüft zu haben — Status kennst du nur,
  wenn er dir explizit als Kontext mitgegeben wurde; dann nenne die Quelle."
- „Versprich nichts über dein eigenes zukünftiges Verhalten (dazulernen,
  nachfassen, beschleunigen), das technisch nicht existiert."
- Kanonische Namen (Steam-Bot, Bot-Pate) und exakte Prozess-Zuständigkeiten
  (wer verschickt die Freundschaftsanfrage; was Support manuell kann) kommen
  aus der Wissensbasis, nie aus Modellwissen.

Vollständiger v0.1-Text + Stil-Guide + Szenarien: Session-Artefakte
(Scratchpad) — finale Fassung entsteht als Teil des Phase-4-Arbeitspakets
gegen die Wissensbasis v1 und wird dann hier im Repo versioniert.

## 6. Wissensbasis-Konsequenzen (Ergänzung zu phase1-wissensbasis.ENTWURF.md)

1. **Neue Pflicht-Sektion „Was es (noch) nicht gibt" (Negativ-Wissen):**
   Modelle erfinden dort, wo die Wissensbasis schweigt. Explizit aufnehmen:
   kein Clan-/Team-System, kein fester Spielabend (kommt Phase 5), Bot kann
   nichts beschleunigen/manuell anstoßen/dazulernen/nachfassen, keine
   Invites an Nicht-Mitglieder.
2. Kanonische Bot-Namen als Single Source (MiniMax erfand „Deadlock DE
   Community Bot").
3. Invite-Mechanik präzise: Steam-Bot (nicht „Steam") verschickt die FA;
   Support-Möglichkeiten ehrlich; 24h-Erinnerungspfad (§4.4).
4. Coaching-Modalitäten klären (geht es ohne Voice? beim Coaching-Team
   erfragen) — Schüchternen-Segment fragt genau das.
5. Opt-out-Wege als FAQ-Punkt („stopp"-Keyword kommt im Entwurf 0× vor).
6. Events: wo stehen sie wirklich, damit der Bot konkret verweisen kann.

## 7. Testkohorten-Messung (Phase 4, ergänzt DoD)

Zusätzlich zu Antwort-/Opt-out-Rate (Konzept §7): **Halluzinations-Audit** —
Stichprobe der Bot-Antworten gegen Wissensbasis diffen (erfundene Kanäle/
Namen/Zusagen zählen), plus Lücken-Log-Rate (wie oft Weiterleitung nötig →
Wissensbasis-Nachzug-Backlog).

## 8. Offene Punkte

- **Name** (Shortlist §1) — Owner-Entscheid, danach T0-Template-Texte
  (T0 ohne LLM, §4.1) als eigenes Textpaket.
- T0/T+2/T+7-Templates final texten (auf Basis Stil-Guide; T+2 nur bei
  Null-Aktivität, T+7 Reibungs-Feedback).
- Anthropic: Guthaben + DPA (Owner), dann §5.6-Update im Konzept.
- Cockpit-Eskalation „Bot meldet zurück" als Phase-4-Arbeitspaket einplanen
  (Voraussetzung für „ich find's raus"-Stufe des Lücken-Dreiklangs).
- Wissensbasis-Ergänzungen (§6) in phase1-wissensbasis.ENTWURF.md einarbeiten.
