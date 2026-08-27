# Grillme: Discord-Verifizierung / Anti-Scam-Gate fuer neue Accounts

Datum: 2026-08-22
Legende: BUILD-P0/P1/P2 = bauen (Prioritaet), DROP = nicht bauen, KLAEREN = offen, MODERN = bauen mit bewusster Modernisierung

## Bestand (nachgeschlagen, nicht zur Debatte)

- `rust/crates/dl-discord/src/gateway.rs:446` `guild_member_addition` feuert `MemberEvent::Join`
  mit `account_created_at` (Unix-Sek. aus der Snowflake). Alterserkennung existiert bereits.
- `MemberEvent` kennt zusaetzlich `ScreeningCompleted` und `NativeOnboardingCompleted`
  (`rust/crates/dl-discord/src/dispatcher.rs:130ff`).
- Zugang laeuft heute ueber natives Discord-Onboarding; alter Wizard/Welcome-DM-Flow ist entfernt
  (`rust/crates/dl-community/src/onboarding.rs`). Konstante `ONBOARD_COMPLETE_ROLE_ID` vorhanden.
- Kein Kick-Code im Bot vorhanden.
- Guild: 1289721245281292288. Datenlage laut Nutzer-Screenshot: 6 % der Neuzugaenge < 1 Monat, < 1 % < 1 Tag.

## Agenda (Cluster nach Entscheidungsachse)

- A Ausloesung und Schwelle
- B Ablauf des Gates (Start, Frage, Versuche)
- C Antwortbewertung (Heroliste vs. Judge)
- D Konsequenz (Frist, Kick, Wiederkehr)
- E Ort und Sichtbarkeit (Kanal/DM/Rolle)
- F Betrieb (Mod-Override, Logs, False Positives)

## Entscheidungen

### A Ausloesung und Schwelle

- **A1 Schwelle: Accountalter unter 1 Monat loest das Gate aus.** BUILD-P0
  (Zwischenstand 2 Monate am 2026-08-22 auf 1 Monat korrigiert: "1 Monat alte Accounts reichen,
  2 Monate waeren auch nicht so der Unterschied".)

### C Antwortbewertung

- **C1 Zwei Pfade.** BUILD-P0
  - Antwort enthaelt einen bekannten Heronamen -> bestanden (Listenabgleich).
  - Antwort beschreibt einen Hero, ohne ihn zu nennen -> Bewertung durch Judge:
    klingt die Beschreibung plausibel und ist sie auf Deutsch.
- **C2 Sprache: deutsche Antwort ist Pflicht, englische Antwort faellt durch.** BUILD-P0
  Kriterium ist die Sprache, nicht das Herkunftsland; AT und CH schreiben deutsch und sind damit abgedeckt.
  Einwand des Agenten (15 % Other laut Discord-Insights, rund 100 Mitglieder) wurde gehoert und verworfen:
  bewusst grobes Pre-Gate, 100 % Trefferquote ist nicht das Ziel, ein Teil rauszufiltern reicht.

### B Ablauf des Gates

- **B1 Start per Bot-DM.** BUILD-P0 Neuer Account bekommt eine DM: Account zu neu,
  bitte verifizieren, dass du kein Bot bist. Verifizierung muss aktiv gestartet werden.
- **B2 DM nicht zustellbar -> Kick.** BUILD-P0
- **B3 Rejoin nach Kick: freier Zugang fuer alle Gekickten, das Gate greift beim zweiten Join nicht mehr.** BUILD-P0
  Begruendung des Nutzers: ein Scammer joint denselben Server selten ein zweites Mal.

### D Frist und Konsequenz

- **D1 Frist 24 Stunden, danach Kick.** BUILD-P0
- **D2 Bedrohungsbild laut Nutzer:** Scammer schreiben Mitglieder per DM an
  (Art-Verkauf, "dein Steam/Discord-Account wurde reportet"). Server-Spam ist nicht das Hauptproblem.
- **D3 Vollquarantaene bis zur bestandenen Antwort.** BUILD-P0
  Unverifizierte sehen keine Kanaele ("Ban ohne Ban"). Gilt auch fuer echte Neulinge.
  Agenten-Einwand (DM-Scam laeuft an jedem Server-Mute vorbei, 24 h reichen fuer die Mitgliederliste)
  wurde damit adressiert: ohne Kanalzugang keine Mitgliederliste.

### C Antwortbewertung (Fortsetzung)

- **C3 Drei Versuche** (Nutzer: "2 3 oder sowas"), Bewertung jedes Versuchs durch den AI-Judge. BUILD-P0

### F Betrieb

- **F1 Kein Mod-Log, keine Mod-Freigabe noetig.** DROP Der Nutzer will davon nichts mitbekommen.
- **F2 Fail-open.** BUILD-P0 Kann der Judge nicht urteilen (technischer Ausfall, Timeout, Provider-Fehler
  ODER unsicheres Urteil), schaltet der Bot frei.

### A Ausloesung (Fortsetzung)

- **A2 Datenlage 2026-08-22** (`activity.guild_member_directory`, Sync vom selben Tag):
  2506 aktive Mitglieder ohne Bots, davon 5 mit Account juenger als 1 Monat, 14 juenger als 2 Monate.
- **A3 Kein rueckwirkendes Gate.** BUILD-P0 Greift ausschliesslich beim Join, die 5 bestehenden
  jungen Accounts bleiben unberuehrt.
- **D4 Nach dem dritten abgelehnten Versuch sofort Kick**, ohne den Rest der 24-Stunden-Frist. BUILD-P0
- **C4 Feste Frage, kein Fragenwechsel.** BUILD-P0 "Nenne einen Hero aus Deadlock."
  Begruendung des Nutzers: wer sich fuer das Spiel interessiert, kennt mindestens einen Hero.
  Risiko der auswendig gelernten Antwort wurde angesprochen und akzeptiert.

### E Ausnahmen und Quelle

- **E1 Persoenliche Einladung ist die einzige Ausnahme.** BUILD-P1
  Join ueber einen persoenlichen Invite-Code eines Mitglieds -> kein Gate.
  Join ueber Vanity-URL, Server-Discovery ("Server entdecken") oder Suche -> Gate greift.
  Bestand: `InviteTracker` (`rust/crates/dl-discord/src/gateway.rs:452`) liefert beim Join bereits
  `join_source_kind` und `inviter_name`; `dl-community/src/invites.rs:33 classify_join_source`
  klassifiziert die Quelle. Kein Neubau noetig.
- **E2 Kein Inviter-Check.** BUILD-P0 Jeder persoenliche Invite zaehlt, unabhaengig davon,
  wie alt oder wie lange dabei der Einladende ist. Agenten-Einwand (Zweitaccount laedt sich selbst ein)
  verworfen: "unwahrscheinlich, die machen sich neue Accounts".

## Vom Agenten entschieden (keine Nutzerentscheidung noetig)

- Bots (OAuth-Join) sind vom Gate ausgenommen (`MemberEvent::Join.is_bot`).
- Judge laeuft ueber den bestehenden zentralen LLM-Weg mit dem Repo-Default (DeepSeek v4 Flash).
- Natives Discord-Onboarding laeuft technisch vor dem Bot-Event; die Quarantaene-Rolle greift
  unmittelbar danach und blendet die Kanaele aus.
- Nach bestandener Antwort: Quarantaene-Rolle entfernen, normaler Zugang, keine weitere Meldung.

## Abdeckung

Alle sechs Agenda-Cluster (A-F) entschieden. Offen bleibt nur der Wortlaut der Bot-DM (KLAEREN).
