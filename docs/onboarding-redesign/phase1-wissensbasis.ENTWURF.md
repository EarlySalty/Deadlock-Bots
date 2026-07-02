# Server-Wissensbasis v1 — GERÜST (Phase 1)

Status: FAQ-PROSA FINAL (F1–F15 von Claude geschrieben, 2026-07-02) —
Struktur-Teil 1 wartet auf Generator (Server-as-Code), Pflege-Owner offen (Phase 3).
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
**Antwort:**
> Der Weg ist kurz: Verknüpf deinen Steam-Account in `#deadlock-rang` (Button-Panel) und nimm danach die Freundschaftsanfrage unseres Steam-Bots an — das war's, dein Invite kommt automatisch. Entweder lädt dich vorher ein Mitglied persönlich ein, oder der Bot verschickt den Invite spätestens ein paar Stunden nach bestätigter Freundschaft von selbst. Falls du `#deadlock-invite` noch nicht siehst: Wähl im Kanäle-&-Rollen-Tab „Ich hab Deadlock noch nicht" aus, dann taucht der Kanal auf.

*Muss enthalten (Checkliste):*
- Weiche 🔑 wählen (falls nicht geschehen: Kanäle&Rollen-Tab) → `deadlock-invite`
- Kern-Ablauf in einem Satz: Steam verknüpfen in `#deadlock-rang` → Invite kommt automatisch
- Beide Wege nennen: Mensch kann persönlich einladen ODER Bot nach 3 h automatisch
- Ehrlich: was der User tun muss (Steam-Link + Freundschaftsanfrage annehmen) — das erklärte das alte Panel nie (dokumentierter Churn-Hebel!)

### F2: Welchem Bot muss ich die Steam-Freundschaftsanfrage schicken?
**Antwort:**
> Normalerweise musst du gar keine schicken — nach dem Steam-Verknüpfen schickt unser Bot **dir** eine Anfrage, du musst sie nur auf Steam annehmen. Kam nichts an? Dann geh den Weg selbst: Steam → Freunde → „Freund hinzufügen" → Freundescode **820142646** eingeben — damit findest du unseren Bot eindeutig, unabhängig vom Anzeigenamen. Sobald die Freundschaft steht, läuft dein Invite automatisch weiter.

*Muss enthalten (Checkliste):*
- DIE dokumentiert unbeantwortete Ticket-Frage (User verließ Server <1 Tag)
- Bot-Name + Freundescode explizit nennen (Stolperfalle Freundescode, Ist-Zustand: 820142646 — beim Generieren aus Soll-Modell aktuell halten)
- Klarstellen: Anfrage kommt normalerweise VOM Bot nach Steam-Verify; User muss nur annehmen
- Schritt-für-Schritt: Steam → Freunde → Code eingeben (Fallback-Weg)

### F3: Wie lange dauert mein Invite und wo sehe ich den Status?
**Antwort:**
> Sobald deine Steam-Freundschaft mit dem Bot bestätigt ist, dauert es maximal etwa 3 Stunden — oft geht es schneller, wenn dich ein Mitglied persönlich einlädt. Den aktuellen Stand (Steam verknüpft → Freundschaft bestätigt → Invite raus) kannst du jederzeit beim Bot per DM abfragen. Und was da steht, stimmt auch: Der angezeigte Status ist der echte Systemstatus, keine Beruhigungsmeldung.

*Muss enthalten (Checkliste):*
- Wartezeiten ehrlich kommunizieren (waren nirgends kommuniziert → Kern-Reibung)
- Zeitanker: Bot-Netz feuert 3 h nach bestätigter Freundschaft (Startwert, justierbar)
- Status-Abfrage: Bot-DM (Phase 3) / Paket-Tracker beim Bot-Paten (Phase 4)
- Versprechen: angezeigter Status = echter Status (keine Falschmeldungen mehr)

### F4: Mein Invite hängt / ich bekomme Fehlermeldungen — was tun?
**Antwort:**
> In fast allen Fällen hakt es an einem von drei Punkten: Steam ist noch nicht verknüpft, die Freundschaftsanfrage des Bots wurde auf Steam noch nicht angenommen (nach 24 h erinnern wir dich einmal daran), oder dein Steam-Account ist „limited" (siehe F6). Schau die drei Punkte kurz durch — wenn es dann immer noch klemmt, schreib einfach in `#frag-die-community` oder öffne ein Ticket in `#server-support`: Da schaut ein Mensch drauf, in der Regel noch am selben Tag.

*Muss enthalten (Checkliste):*
- Häufigste Ursachen: Steam nicht verknüpft, Freundschaftsanfrage nicht angenommen (24-h-Erinnerung kommt), Limited-User
- Kein Ping-Spam mehr (Phase-0-Fix ②/④ referenzieren)
- Eskalationsweg: `frag-die-community` oder `server-support` → Mensch antwortet am selben Tag

### F5: Kann mich auch ein Mensch direkt einladen?
**Antwort:**
> Ja, ausdrücklich — das ist bei uns kein Gefallen, sondern der bevorzugte Weg. Wenn ein Mitglied dich persönlich einladen will, meldet es das kurz im System und der automatische Versand pausiert. Für dich ändert sich nichts am Ablauf oder Ergebnis; in deinem Status steht dann einfach „persönlich eingeladen von X". Kommt kein Mensch dazu, springt nach ein paar Stunden der Bot ein.

*Muss enthalten (Checkliste):*
- Ja — der frühere Mod-Goodwill ist jetzt offizieller Weg (Mensch-Fenster)
- Mitglied meldet „Ich lade persönlich ein" im Cockpit → Bot-Netz pausiert
- Für den Gast ändert sich nichts am Ergebnis; Status zeigt „persönlich eingeladen von X"

### F6: Der Bot sagt, mein Steam-Account ist „limited" — warum bekomme ich keinen Invite?
**Antwort:**
> Das ist leider eine Beschränkung von Valve, kein Fehler bei uns: Steam stuft Accounts als „limited" ein, die noch nie mindestens 5 $ im Steam-Store ausgegeben haben — und solche Accounts können keine Playtest-Einladungen über unseren Weg erhalten. Sobald du irgendwas für 5 $ auf Steam gekauft hast, fällt die Sperre weg. Dein Fall landet außerdem automatisch bei einem Menschen aus dem Team, der mit dir die Optionen durchgeht.

*Muss enthalten (Checkliste):*
- GC-Code 6 = echte Valve-Restriktion (Limited User), KEIN Bot-Fehler
- Ehrlich erklären, was ein Limited Account ist (kein 5-$-Kauf auf Steam)
- Ein Mensch übernimmt automatisch (Eskalations-Endstufe für diesen Fall)

### F7: Wie verknüpfe ich meinen Steam-Account?
**Antwort:**
> Geh in `#deadlock-rang` (hieß früher `rang-auswahl`) — dort findest du das einzige Button-Panel des Servers, mehr Buttons gibt's bei uns absichtlich nicht. Klick auf „Verknüpfen", logg dich auf der Steam-Seite ein, fertig. Wenn etwas nicht durchgelaufen ist, kannst du über dasselbe Panel jederzeit neu prüfen lassen.

*Muss enthalten (Checkliste):*
- Panel in `#deadlock-rang` (ex `rang-auswahl` — alter Name für Bestands-User erwähnen, Übergangszeit)
- Ablauf: Button → Steam-OpenID-Login → fertig; Recheck-Möglichkeit
- Einziges öffentliches Button-Panel des Servers (bewusst so)

### F8: Was passiert bei der Steam-Verifizierung mit meinen Daten — und was bringt sie mir?
**Antwort:**
> Was du davon hast: deine echte Rang-Rolle (automatisch aktuell statt Selbstauskunft), die Möglichkeit, Freunde per Invite reinzuholen, und Rank-up-Feiern, wenn du kletterst. Zur Technik: Die Verknüpfung läuft über den offiziellen Steam-Login (OpenID) — wir sehen nie dein Passwort und haben keinerlei Zugriff auf deinen Account, wir bekommen nur deine Steam-ID. Du kannst deine Daten jederzeit exportieren oder löschen lassen, und wenn irgendwas hakt, kümmert sich ein Mensch aus dem Team.

*Muss enthalten (Checkliste):*
- Größter dokumentierter Skepsis-Punkt → Nutzen VOR Mechanik: echte Rang-Rolle, Invite-Fähigkeit, Rank-up-Feiern
- Bestehende vorbildliche Datenschutz-Erklärung übernehmen (aus altem Panel — behalten!)
- Nur OpenID (keine Passwörter, kein Kontozugriff); Löschung/Export möglich (privacy-Pfade)
- Mensch-Fallback bei Problemen (kein Owner-Direkt-Ping mehr)

### F9: Wie bekomme ich meine Rang-Rolle / wie ändere ich meinen Rang?
**Antwort:**
> Es gibt zwei Stufen. Stufe 1: Beim Onboarding gibst du deinen Rang selbst an (12 Optionen) und bekommst die passende „(unverifiziert)"-Rolle — ändern kannst du das jederzeit im Kanäle-&-Rollen-Tab. Stufe 2: Verknüpfst du deinen Steam-Account in `#deadlock-rang`, bekommst du deine echte Rang-Rolle, die sich ab dann von selbst aktuell hält — da musst du nie wieder etwas anfassen.

*Muss enthalten (Checkliste):*
- Zwei Stufen: Selbstauskunft im Onboarding → „(unverifiziert)"-Rolle; Steam-Verify → echte, automatisch aktuelle Rang-Rolle
- Ändern: Kanäle&Rollen-Tab (Selbstauskunft) bzw. automatisch nach Verify
- 12 Rang-Optionen, 66 Rang-Rollen gesamt (Struktur-Teil referenzieren)

### F10: Ich bin ganz neu — bringt mir jemand das Spiel bei?
**Antwort:**
> Ja, genau dafür sind wir da. Wähl im Kanäle-&-Rollen-Tab „Ich bin ganz neu und will's lernen" — dann meldet sich unser Bot mit einem Paten-Angebot: Wenn du willst (nur dann!), bekommst du einen echten Menschen an die Seite, der sich am selben Tag meldet und in der ersten Woche ein Match mit dir spielt. Dazu gibt's unser kostenloses Coaching-Programm, in das dich dein Pate direkt vermittelt, und die Neue-Spieler-Lane, in der niemand von dir erwartet, dass du schon irgendwas kannst.

*Muss enthalten (Checkliste):*
- Zwei dokumentierte solcher Bitten blieben unbeantwortet trotz Gratis-Coaching → diese Antwort MUSS sitzen
- Ja: 🌱-Weiche wählen → Paten-Angebot (Mensch, Opt-in); Pate spielt erstes Match mit
- Coaching-Programm ist kostenlos; Pate/Cockpit vermittelt aktiv (Lern-Bitten werden als Coaching-Lead geflaggt)
- Neue-Spieler-Lane für druckfreies Reinkommen

### F11: Wie funktioniert das kostenlose Coaching und wie melde ich mich an?
**Antwort:**
> Unser Coaching ist komplett kostenlos: Erfahrene Spieler aus der Community nehmen sich Zeit für dich — von Grundlagen bis Rang-Aufstieg. Anmelden kannst du dich über den Coaching-Bereich auf unserer Website; deine Anfrage landet direkt beim Coach-Team hier im Discord, und ein Coach übernimmt sie. Dazu gibt's den Scrim-Bereich, in dem Teams gemeinsam üben und sich gegenseitig besser machen. Zur Einordnung: Dein Pate begleitet dich in den ersten Tagen bis zum ersten Match — Coaching ist das strukturierte Lernen danach.

*Muss enthalten (Checkliste):*
- Anmeldewege: Website-Coaching-Bereich → Anfrage wird in den Discord gespiegelt, Coach claimt (bestehender Flow bleibt unangetastet)
- Scrim-Bereich als Kultur-Aushängeschild erwähnen (Peer-Mentoring, Anti-Toxicity)
- Abgrenzung: Pate = erste Tage & erstes Match; Coaching = strukturiertes Lernen

### F12: Wo finde ich Mitspieler für ein Match?
**Antwort:**
> Drei Wege, alle unkompliziert: Schreib in `#spieler-suche`, wer du bist und worauf du Lust hast — das ist unser aktivster Kanal für genau das. Oder spring direkt in eine Voice-Lane: In den Chill-Lanes ist der Rang komplett egal, in den Rang-Lanes findest du Mitspieler auf deinem Niveau. Reinsetzen ist ausdrücklich erlaubt — die Lanes sind offen, du musst niemanden um Erlaubnis fragen.

*Muss enthalten (Checkliste):*
- `spieler-suche` (aktivster Neuen-Kanal, 20 Autoren) + LFG/player_finder als Verweis-Ziel
- Rang-Lanes (deklarieren, reinsetzen) und Chill-Lanes (rang-egal)
- Community-Spielabend erwähnen, sobald live (Phase 5 — bis dahin weglassen)

### F13: Wie funktionieren die Lanes — und darf ich in jede rein?
**Antwort:**
> Ja, du darfst in jede rein — unsere Lanes haben keine Türsteher. Wer eine Rang-Lane öffnet, gibt an, für welchen Rang-Bereich sie gedacht ist, und der Bot benennt und sortiert sie automatisch; das ist eine Ansage, kein Schloss. Die Regel dahinter ist einfach: Der Lane-Ersteller entscheidet, wer bleibt — und bei Streit entscheiden die Mods. Daneben gibt's die Chill-Lanes (Rang komplett egal, unser Wohnzimmer) und die Neue-Spieler-Lane. Nur zuschauen und zuhören ist übrigens völlig okay.

*Muss enthalten (Checkliste):*
- Lane öffnen: User deklariert Rang/Range, Bot benennt/sortiert automatisch; Lane bleibt OFFEN (Deklaration statt Türsteher)
- Sichtbare Norm zitieren: „Der Lane-Ersteller entscheidet, wer bleibt; bei Streit entscheiden Mods"
- Chill-Lanes = rang-egales Wohnzimmer; Neue-Spieler-Lane für 🌱
- 👻-Lurker-Feature (zuschauen ohne Druck) als eigenen Satz ergänzen, sobald Owner-Entscheid bestätigt [OFFEN: Konzept §8 — aktuell nur der neutrale Schluss-Satz oben]

### F14: Wie stelle ich Pings/Benachrichtigungen ein (oder ab)?
**Antwort:**
> Alles selbst steuerbar, jederzeit: Oben links über dem Kanalbaum findest du „Kanäle & Rollen" — dort schaltest du Pings für Patchnotes, Spielersuche, Events/Turniere, Custom Games und Streams einzeln an oder aus. Auch deine Start-Auswahl (Mitspieler suchen / Invite brauchen / ganz neu) kannst du dort ändern — das ist nur eine Angabe über dich, keine Berechtigung. Und die Nachrichten vom Bot: Ein Klick auf Opt-out genügt — oder ignorier ihn einfach zweimal, dann versteht er das auch.

*Muss enthalten (Checkliste):*
- Kanäle&Rollen-Tab: Patchnotes, Spielersuche, Events/Turniere, Custom Games, Streams — jederzeit selbst umschaltbar
- Auch die Weiche (🎮/🔑/🌱) ist dort selbst änderbar (reine Präferenz, keine Rechte)
- Bot-Paten-DMs: Opt-out jederzeit, 2× nicht antworten reicht auch

### F15: Wo fange ich am besten an — und wo kann ich einfach Fragen stellen?
**Antwort:**
> Folg einfach der Server-Guide-Checkliste (oben im Kanalbaum): Sag Hallo in `#frag-die-community`, verknüpf deinen Steam-Account, spring in eine Lane, stell deine Pings ein — in der Reihenfolge, ohne Zeitdruck. Und für alles andere gilt: `#frag-die-community` ist genau dafür da, es gibt keine dummen Fragen, und hier antworten dir echte Menschen — normalerweise noch am selben Tag. Unser Bot hilft auch gern weiter, sagt dir aber ehrlich, wenn er etwas nicht weiß, und holt dann einen Menschen dazu.

*Muss enthalten (Checkliste):*
- Server-Guide-Checkliste als roter Faden (Hallo → Steam → Lane → Pings)
- `frag-die-community`: JEDE Frage bekommt am selben Tag eine menschliche Antwort (Garantie erst öffentlich versprechen, wenn intern 2+ Wochen stabil — bis dahin weichere Formulierung! → deshalb oben „normalerweise")
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
