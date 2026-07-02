# Server-Wissensbasis v1 — GERÜST (Phase 1)

Status: GERÜST — Struktur steht, Inhalte teils Platzhalter.
Quellen: `docs/onboarding-redesign/2026-07-02-konzept.md` (§5.4, §4.1, §4.2),
`docs/onboarding-redesign/2026-07-02-ist-zustand.md`.

**Zweck & Konsumenten:**
- Ab Phase 3: KI-Antwortvorschläge auf Cockpit-Karten (Mensch prüft, passt an, sendet unter eigenem Namen — §4.2).
- Ab Phase 4: Bot-Pate (offen deklarierter KI-Charakter, §4.1) — antwortet NUR aus dieser Wissensbasis, erfindet nie Fakten/Status.
- Ticket-Auto-Help-Nachfolger (falls Struct-Split-Entscheid „behalten", §6 Abriss-Vorsicht): Prompt speist sich aus dieser Basis.

**Aufbau-Prinzip (Drift-Schutz, §5.4):**
- Teil 1 (Server-Struktur) wird aus dem Server-as-Code-Soll-Modell (§5.1) **GENERIERT** — nie von Hand editieren.
- Teile 2–5 sind **manuell gepflegte Prosa**; finale user-sichtbare Formulierungen schreibt Claude (Codex liefert nur Platzhalter).
- Kein Ablauf-Wissen in LLM-Prompts hartkodieren (Lehre aus der FAQ-Prompt-Falle, faq.rs Z. 67–101) — alles Prozesswissen lebt hier.
- Gameplay-Fragen: langfristig Deadlock-Brain als Grounding; bis dahin ehrlich passen + auf `frag-die-community` verweisen.

---

## 1. Server-Struktur ⚙️ [GENERIERT — NICHT VON HAND EDITIEREN]

> Dieser Abschnitt wird bei jedem Server-as-Code-Sync aus dem Soll-Modell
> (zentrale Postgres, §5.1) neu erzeugt. Kopfzeile beim Generieren befüllen:
>
> `Generiert: <timestamp> | Soll-Modell-Version: <version/hash> | Generator: <binary>`

### 1.1 Kanäle (Platzhalter-Schema, ein Block pro Kanal)

```
Kanal: <name>                          [GENERIERT]
- ID: <channel_id>
- Kategorie: <kategorie>
- Typ: text | voice | forum
- Zweck (Kurztext fürs Grounding): <1–2 Sätze aus dem Soll-Modell>
- Sichtbar für: @everyone | <rollen>
- Schreiben erlaubt: ja | nein (Bot-only) | <rollen>
- Besonderheiten: <z. B. „einziges öffentliches Button-Panel (Steam-Verify)" |
  „Klartext, keine Embeds/Buttons" | „Default-Kanal im nativen Onboarding">
- Onboarding-relevant: ja/nein (Server-Guide-Checkliste, Default-Liste 7/5-Regel)
```

Erwartete Kern-Kanäle nach Phase 2 (Namen aus §4.6, hier nur als Vorschau —
verbindlich ist der Generator-Output): `frag-die-community`, `deadlock-rang`
(ex rang-auswahl, Steam-Verify-Panel), `deadlock-invite` (ex beta-zugang,
offene Lounge), `regelwerk` (ex hier-starten-regelwerk, Nachschlagewerk),
`server-support` (ex lag-kompensator), `allgemein`, `spieler-suche`,
Rang-Lanes-Sektion, Chill-Lanes.

### 1.2 Rollen (Platzhalter-Schema, ein Block pro Rolle/Gruppe)

```
Rolle: <name>                          [GENERIERT]
- ID: <role_id>
- Typ: rang-verifiziert | rang-unverifiziert | weiche (🔑 Invite-Gast / 🌱 Frischling) |
  ping | funktion (Mod/Pate/…) | insider/troll
- Vergabe: natives Onboarding | Steam-Verify | Selbstwahl (Kanäle&Rollen-Tab) | manuell
- Berechtigungs-relevant: ja/nein   ← Weiche-Rollen IMMER „nein" (reine Präferenz-Signale, §2)
- Lebenszyklus: <z. B. „🔑→🎮 bei Invite angenommen/erstes Match; 🌱 endet bei
  Nordstern-Event oder nach 30 Tagen">
```

### 1.3 Interaktionsflächen [GENERIERT]

- Steam-Verify-Panel in `deadlock-rang` — **einzige** öffentliche Button-Fläche (Prinzip 5).
- Bot-Pate: DM + privater Fallback-Kanal (Kategorie über `allgemein`, nur bei unzustellbarer DM, Auto-Cleanup 14 Tage).
- Paten-Cockpit (Mod-Bereich): Karten, Claim, KI-Vorschläge.
- Natives Onboarding: 3 Fragen (Weiche Pflicht / Pings optional inkl. „Streams" / Rang-Selbstauskunft 12 Optionen) + Default-Kanal-Liste (7/5-Regel) + Server-Guide-Checkliste.

### 1.4 Dynamische Namespaces (verwaltete Ausnahmen, §5.1) [GENERIERT]

TempVoice-Lanes, private Fallback-Kanäle, Ticket-Kanäle (TicketTool =
dokumentierte Fremd-Schreibinstanz) — Existenz und Regeln hier gelistet,
Einzelinstanzen nicht.

---

## 2. Journeys (manuelle Prosa; referenziert §2–§4 Konzept)

Gemeinsamer Rahmen: Eintritt = Member-Screening (Regeln) + natives Onboarding
(3 Fragen). Danach vollwertig drin — Bot-Onboarding ist Hilfe, nie Hürde.
Bestandsmitglieder (2368) = Zustand `legacy`: keine Journey, keine T0-DM.
T0-DM ist strikt einmalig pro User; Weiche-Wechsel = Event, kein Journey-Neustart.

### 2.1 🎮 Spieler — Ziel: erstes gemeinsames Match

1. Join → Screening → Weiche „Ich spiele Deadlock und suche Mitspieler" (vergibt bewusst KEINE Rolle; Erkennung: COMPLETED_ONBOARDING-Flag + weder 🔑- noch 🌱-Rolle).
2. Pings wählen, Rang-Selbstauskunft → „(unverifiziert)"-Rang-Rolle.
3. Server-Guide-Checkliste: Hallo in `frag-die-community` → Steam verknüpfen in `deadlock-rang` → in eine Lane springen → Pings anpassen.
4. T0-DM vom Bot-Paten (statisches Template ohne LLM, Weiche-personalisiert; Buttons inkl. „🔕 Lass mich einfach stöbern" + Datenschutzhinweis + Opt-out).
5. Optional Steam-Verify → echte Rang-Rolle, Invite-Fähigkeit, Rank-up-Feiern.
6. Lane öffnen/beitreten oder `spieler-suche` → **Nordstern-Event: erstes gemeinsames Match/Voice** (binnen 14 Tagen).
7. Paten-Angebot kommt vom Bot-Paten (Opt-in). Kadenz: T0 → T+2 (nur bei Null-Aktivität) → T+7 (Reibungs-Feedback) → Stille. 2× ignoriert = implizites Opt-out.

### 2.2 🌱 Frischling — Ziel: lernen + erstes Match

1. Weiche „Ich bin ganz neu und will's lernen — nehmt mich an die Hand" → Rolle `Frischling`. Diese Formulierung IST der dokumentierte Konsens fürs Paten-Angebot.
2. T0-DM: Paten-Angebot mit expliziter Opt-in-Bestätigung (Mensch-Paten-Matching NUR nach Ja, Prinzip 3).
3. Bei Ja: Mensch-Pate — Verpflichtung klein & klar: erste Reaktion am selben Tag + ein gemeinsames Match in Woche 1.
4. Pate vermittelt aktiv ins bestehende Coaching; Cockpit flaggt Lern-Bitten („bringt mir jemand das Spiel bei") automatisch als Coaching-Lead.
5. Neue-Spieler-Lane als niederschwelliger Einstieg; Zuschauen/Klicken ist legitime Teilnahme (Prinzip 4).
6. Lautlose 🌱 bleiben beim Bot-Paten (T+2/T+7-Kadenz).
7. Status endet beim Nordstern-Event oder nach 30 Tagen (Funnel-Event).

### 2.3 🔑 Invite-Gast — Ziel: Spiel installiert → konvertiert zu 🎮

1. Weiche „Ich hab Deadlock noch nicht — ich brauche einen Invite" → Rolle `Invite-Gast`; `deadlock-invite` wird zugespielt.
2. `deadlock-invite` = offene Lounge, Klartext-Anleitung: „Steam verknüpfen in #deadlock-rang → Invite kommt automatisch."
3. Steam-Verify in `deadlock-rang` → Steam-Bot schickt Freundschaftsanfrage.
4. **Mensch-Fenster:** Mitglieder dürfen schneller sein; Mensch-Invite läuft über das System (Ein-Klick „Ich lade persönlich ein" im Cockpit → pausiert Bot-Netz, Status „persönlich eingeladen von X"). Bot erkennt bereits vollzogene Invites vor Timer-Ablauf.
5. **Bot-Netz:** 3 h nach bestätigter Steam-Freundschaft ohne Mensch-Invite → Bot verschickt selbst, mit ehrlicher Meldung.
6. Failure-Pfade: Freundschaftsanfrage nach 24 h nicht angenommen → Erinnerung mit Bot-Name + Freundescode. Limited-User (GC-Code 6, echte Valve-Restriktion) → ehrliche Erklärung + Mensch übernimmt. **User-sichtbarer Status = interner Status, immer.**
7. Status-Abfrage: Phase 3 simple deklarierte Bot-DM; ab Phase 4 Paket-Tracker beim Bot-Paten (Steam ✓ → Freundschaft ✓ → Invite ✓).
8. Wartezeit aktiv füllen: Streams schauen, Lanes, `frag-die-community` (Typ-2-Konversion).
9. Invite angenommen / erstes Match → Rolle wechselt zu 🎮 (Funnel-Event) → Paten-Angebot.

---

## 3. FAQ-Prosa-Gerüst — die 15 häufigsten Neuen-Fragen

Format pro Frage: Antwort = **[PLATZHALTER — finalen Text schreibt Claude]** +
Stichpunkte (Fakten, die die finale Antwort enthalten MUSS). Belege: Ist-Zustand §4.

### F1: Wie bekomme ich einen Deadlock-Invite?
**Antwort: [PLATZHALTER]**
- Weiche 🔑 wählen (falls nicht geschehen: Kanäle&Rollen-Tab) → `deadlock-invite`
- Kern-Ablauf in einem Satz: Steam verknüpfen in `#deadlock-rang` → Invite kommt automatisch
- Beide Wege nennen: Mensch kann persönlich einladen ODER Bot nach 3 h automatisch
- Ehrlich: was der User tun muss (Steam-Link + Freundschaftsanfrage annehmen) — das erklärte das alte Panel nie (dokumentierter Churn-Hebel!)

### F2: Welchem Bot muss ich die Steam-Freundschaftsanfrage schicken?
**Antwort: [PLATZHALTER]**
- DIE dokumentiert unbeantwortete Ticket-Frage (User verließ Server <1 Tag)
- Bot-Name + Freundescode explizit nennen (Stolperfalle Freundescode, Ist-Zustand: 820142646 — beim Generieren aus Soll-Modell aktuell halten)
- Klarstellen: Anfrage kommt normalerweise VOM Bot nach Steam-Verify; User muss nur annehmen
- Schritt-für-Schritt: Steam → Freunde → Code eingeben (Fallback-Weg)

### F3: Wie lange dauert mein Invite und wo sehe ich den Status?
**Antwort: [PLATZHALTER]**
- Wartezeiten ehrlich kommunizieren (waren nirgends kommuniziert → Kern-Reibung)
- Zeitanker: Bot-Netz feuert 3 h nach bestätigter Freundschaft (Startwert, justierbar)
- Status-Abfrage: Bot-DM (Phase 3) / Paket-Tracker beim Bot-Paten (Phase 4)
- Versprechen: angezeigter Status = echter Status (keine Falschmeldungen mehr)

### F4: Mein Invite hängt / ich bekomme Fehlermeldungen — was tun?
**Antwort: [PLATZHALTER]**
- Häufigste Ursachen: Steam nicht verknüpft, Freundschaftsanfrage nicht angenommen (24-h-Erinnerung kommt), Limited-User
- Kein Ping-Spam mehr (Phase-0-Fix ②/④ referenzieren)
- Eskalationsweg: `frag-die-community` oder `server-support` → Mensch antwortet am selben Tag

### F5: Kann mich auch ein Mensch direkt einladen?
**Antwort: [PLATZHALTER]**
- Ja — der frühere Mod-Goodwill ist jetzt offizieller Weg (Mensch-Fenster)
- Mitglied meldet „Ich lade persönlich ein" im Cockpit → Bot-Netz pausiert
- Für den Gast ändert sich nichts am Ergebnis; Status zeigt „persönlich eingeladen von X"

### F6: Der Bot sagt, mein Steam-Account ist „limited" — warum bekomme ich keinen Invite?
**Antwort: [PLATZHALTER]**
- GC-Code 6 = echte Valve-Restriktion (Limited User), KEIN Bot-Fehler
- Ehrlich erklären, was ein Limited Account ist (kein 5-$-Kauf auf Steam)
- Ein Mensch übernimmt automatisch (Eskalations-Endstufe für diesen Fall)

### F7: Wie verknüpfe ich meinen Steam-Account?
**Antwort: [PLATZHALTER]**
- Panel in `#deadlock-rang` (ex `rang-auswahl` — alter Name für Bestands-User erwähnen, Übergangszeit)
- Ablauf: Button → Steam-OpenID-Login → fertig; Recheck-Möglichkeit
- Einziges öffentliches Button-Panel des Servers (bewusst so)

### F8: Was passiert bei der Steam-Verifizierung mit meinen Daten — und was bringt sie mir?
**Antwort: [PLATZHALTER]**
- Größter dokumentierter Skepsis-Punkt → Nutzen VOR Mechanik: echte Rang-Rolle, Invite-Fähigkeit, Rank-up-Feiern
- Bestehende vorbildliche Datenschutz-Erklärung übernehmen (aus altem Panel — behalten!)
- Nur OpenID (keine Passwörter, kein Kontozugriff); Löschung/Export möglich (privacy-Pfade)
- Mensch-Fallback bei Problemen (kein Owner-Direkt-Ping mehr)

### F9: Wie bekomme ich meine Rang-Rolle / wie ändere ich meinen Rang?
**Antwort: [PLATZHALTER]**
- Zwei Stufen: Selbstauskunft im Onboarding → „(unverifiziert)"-Rolle; Steam-Verify → echte, automatisch aktuelle Rang-Rolle
- Ändern: Kanäle&Rollen-Tab (Selbstauskunft) bzw. automatisch nach Verify
- 12 Rang-Optionen, 66 Rang-Rollen gesamt (Struktur-Teil referenzieren)

### F10: Ich bin ganz neu — bringt mir jemand das Spiel bei?
**Antwort: [PLATZHALTER]**
- Zwei dokumentierte solcher Bitten blieben unbeantwortet trotz Gratis-Coaching → diese Antwort MUSS sitzen
- Ja: 🌱-Weiche wählen → Paten-Angebot (Mensch, Opt-in); Pate spielt erstes Match mit
- Coaching-Programm ist kostenlos; Pate/Cockpit vermittelt aktiv (Lern-Bitten werden als Coaching-Lead geflaggt)
- Neue-Spieler-Lane für druckfreies Reinkommen

### F11: Wie funktioniert das kostenlose Coaching und wie melde ich mich an?
**Antwort: [PLATZHALTER]**
- Anmeldewege: [PLATZHALTER — Coaching-Bereich bleibt unangetastet, Ablauf aus bestehendem Coaching-System übernehmen: Website-Panel + Claim-Flow]
- Scrim-Bereich als Kultur-Aushängeschild erwähnen (Peer-Mentoring, Anti-Toxicity)
- Abgrenzung: Pate = erste Tage & erstes Match; Coaching = strukturiertes Lernen

### F12: Wo finde ich Mitspieler für ein Match?
**Antwort: [PLATZHALTER]**
- `spieler-suche` (aktivster Neuen-Kanal, 20 Autoren) + LFG/player_finder als Verweis-Ziel
- Rang-Lanes (deklarieren, reinsetzen) und Chill-Lanes (rang-egal)
- Community-Spielabend erwähnen, sobald live (Phase 5 — bis dahin weglassen)

### F13: Wie funktionieren die Lanes — und darf ich in jede rein?
**Antwort: [PLATZHALTER]**
- Lane öffnen: User deklariert Rang/Range, Bot benennt/sortiert automatisch; Lane bleibt OFFEN (Deklaration statt Türsteher)
- Sichtbare Norm zitieren: „Der Lane-Ersteller entscheidet, wer bleibt; bei Streit entscheiden Mods"
- Chill-Lanes = rang-egales Wohnzimmer; Neue-Spieler-Lane für 🌱
- 👻-Lurker-Feature (zuschauen ohne Druck) erwähnen, falls es bleibt [OFFEN: Owner-Entscheid Lurker, Konzept §8]

### F14: Wie stelle ich Pings/Benachrichtigungen ein (oder ab)?
**Antwort: [PLATZHALTER]**
- Kanäle&Rollen-Tab: Patchnotes, Spielersuche, Events/Turniere, Custom Games, Streams — jederzeit selbst umschaltbar
- Auch die Weiche (🎮/🔑/🌱) ist dort selbst änderbar (reine Präferenz, keine Rechte)
- Bot-Paten-DMs: Opt-out jederzeit, 2× nicht antworten reicht auch

### F15: Wo fange ich am besten an — und wo kann ich einfach Fragen stellen?
**Antwort: [PLATZHALTER]**
- Server-Guide-Checkliste als roter Faden (Hallo → Steam → Lane → Pings)
- `frag-die-community`: JEDE Frage bekommt am selben Tag eine menschliche Antwort (Garantie erst öffentlich versprechen, wenn intern 2+ Wochen stabil — bis dahin weichere Formulierung!)
- Bot-Pate ist ansprechbar (offen deklarierter Bot), verweist aktiv auf Menschen

**Übergangs-FAQ (befristet, nach Phase 2 löschen):**
- „Warum kann ich keine Bilder/Reactions posten?" — Alt-Komfort-Gate (Deadlocker-Rolle); nach Rechte-Sanierung obsolet.
- Klicks auf alte Buttons (`wdm:`/`aiob:`/`ob:`-custom_ids in alten Nachrichten) → Übergangs-Antwort mit Verweis auf neuen Weg.

---

## 4. Eskalations-Regeln (wann Mensch statt Bot/KI)

**Harte Bot-Paten-Grenzen (§4.1):**
- Erfindet NIE Fakten oder Status; gesteht Lücken ehrlich ein („weiß ich nicht — ich frag einen Menschen / frag in #frag-die-community").
- Antwortet nur aus dieser Wissensbasis; Gameplay ohne Grounding → passen + verweisen.
- LLM erst NACH erster aktiver User-Nachricht (T0-DM statisch, §5.6); vorher fließen keine User-Daten an einen KI-Prozessor.
- Konsens-Prinzip: Opt-out (explizit oder 2× ignoriert) = dauerhaft, kein erneuter Kontakt.

**Sofort an Mensch eskalieren (Karte im Cockpit, kein Bot-Antwortversuch):**
1. T&S-/Schadensrisiko: Suizid-Andeutungen, Selbstverletzung, Belästigung, Doxxing.
2. Minderjährigen-Themen (Art.-8-Abwägung dokumentiert, §5.6).
3. Moderations-/Konfliktfälle: Lane-Streit („Mods entscheiden"), Regelverstöße, Beschwerden über Mods oder den Bot.
4. Datenschutz: Auskunfts-, Lösch-, Export-Begehren (privacy-Pfade; nie vom LLM beantworten lassen).
5. Invite-Sonderfälle: Limited-User (GC-Code 6), widersprüchlicher/unklarer interner Status, alles was der Paket-Tracker nicht abbildet.
6. Frage außerhalb der Wissensbasis oder Unsicherheit des Modells.

**Cockpit-Eskalationskette (§4.2):**
- Neue Frage in `frag-die-community` unbeantwortet → Karte; Bot pingt bevorzugt online Mods.
- 2 h ohne Claim (Startwert) → Eskalation an Paten-Rolle.
- Endstufe: Bot antwortet selbst, offen deklariert als Bot. Time-to-first-response wird gemessen.

**KI-Antwortvorschläge (Cockpit):** immer Mensch prüft/sendet unter eigenem Namen; Transparenzpflicht in Regeln/Onboarding (§5.6 Punkt 4); Provider erst nach Compliance-Gate auf echtem User-Content (Mistral-Cutover VOR Phase-3-Echtbetrieb; MiniMax nur Dev mit synthetischen Daten).

---

## 5. Pflege-Regeln

- **Pflege-Owner:** [PLATZHALTER — Person/Rolle benennen; Vorschlag: Server-Owner als Freigeber + 1 Mod als Redakteur. Teil des Arbeitspakets Paten-Besetzung, Phase 3]
- **Review-Trigger (Pflicht, §5.4):** bei JEDEM Server-as-Code-Diff wird geprüft, ob Prosa (Teile 2–5) nachziehen muss. Struktur-Teil (Teil 1) regeneriert automatisch.
- **Weitere Trigger:** Phasen-Übergang (0→…→5), Änderungen am Invite-Flow/Steam-Verify, Kanal-Archivierung/-Neuanlage, Parameter-Justierung (3-h-Netz, 2-h-Eskalation, 24-h-Erinnerung, 14-Tage-Cleanup, 30-Tage-🌱, 180-Tage-Retention — alles Startwerte), Owner-Entscheide zu offenen Punkten (Lurker, Streams-Ping, R-Rollen-Event).
- **Feedback-Rückfluss:** T+7-Reibungs-Antworten + Cockpit-Fälle „Bot konnte nicht antworten" werden gesammelt und fließen als FAQ-Kandidaten ein (Review monatlich, zusammen mit 30-Tage-Kanal-Report).
- **Textstandard:** Deutsch, locker-nüchtern, konkret; finale user-sichtbare Texte schreibt Claude; Codex liefert nur `"Platzhalter"` + Stelle.
- **Versionierung:** Datei lebt im Repo (`docs/`), Änderungen via Commit; Generator-Kopfzeile (Teil 1) macht Struktur-Stand nachvollziehbar.
- **Verboten:** Handedits im GENERIERT-Teil; Prozesswissen in LLM-Prompts statt hier; Antwort-Garantie öffentlich versprechen vor 2 Wochen stabiler interner Messung.
