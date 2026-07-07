# Concierge (Onboarding 2.0) — Spec Slice A

Ergebnis der grillme-Session 2026-07-07 (Owner + Claude). Baut auf
`docs/onboarding-redesign/2026-07-02-konzept.md` (§4.1 Bot-Pate, §4.2 Paten,
§5.6 Compliance-Gate) und `2026-07-02-ki-charakter-design.md` auf.
Wo dieses Dokument abweicht, gilt dieses Dokument.

## 0. Begriffsklärung

Der „Bot-Pate" aus dem Konzept heißt jetzt **Concierge**. Kein Eigenname,
per Du, Hotel-Flair als Tonfarbe (Anspielung auf den Doorman, keine
Valve-Kopie, kein Lore-Wesen). Der **Pate ist immer ein Mensch**
(Rolle `1524047896297738311`), der Concierge ist der Bot. Charakter-Regeln
aus dem KI-Charakter-Design gelten unverändert (dünne Persona, dicker Ton,
keine Mensch-Simulation, Wärme über Zusagen und ASCII-`:)`).

## 1. Ziel & Messung

Erstkontakt so schnell wie möglich nach Join (Minuten bis wenige Stunden):
User schreibt eine eigene Nachricht in einem öffentlichen Kanal ODER joint
einen Voice. Integration (mitspielen) ist Folge davon. Der Concierge ist
Trichter zu Menschen, kein Aufenthaltsraum: jede Strecke endet Richtung
Mensch (Kanal, Lane, Steckbrief, Pate).

Funnel-Events (an bestehendes Journey/Insights-System):
`concierge_t0_sent`, `concierge_reply`, `concierge_tour_done`,
`steckbrief_posted`, `first_message`, `first_voice`, `nudge_sent`,
`pate_offered`, `opted_out`, `congrats_sent`.

## 2. Trigger & Gesprächs-Flow

1. **T0:** Event `NativeOnboardingCompleted` (bestehende Onboarding-Brücke)
   → DM. **T0 ist statisches Template ohne LLM** (Konzept §4.1,
   Datenminimierung), Text siehe Textpaket. Enthält: Begrüßung, EINE offene
   Frage („erzähl mir, was du hier vorhast"), dezente Buttons als Abkürzung
   (Tour / Mitspieler / später), Transparenz-Satz zum Gedächtnis, Hinweis
   auf „stopp".
2. **DM zu (Discord 50007):** Fallback privater Textkanal nach dem Muster
   aus `faq.rs` (Kanal nur User + Bot, Auto-Aufräumen). KV-Claim gegen
   Doppel-Erstellung wie in `onboarding_bridge.rs`.
3. **User antwortet frei** → ab jetzt LLM-Gespräch über
   `LlmUseCase::BotPate` (dl-ai). Start-Modell laut Owner-Entscheid vom
   07.07.: **DeepSeek über Fireworks** (`deepseek-v4-flash`, Fireworks hat
   laut Owner einen AVV/DPA; Nachweis wird als Launch-Gate abgelegt,
   §10). Provider bleibt per Env umschaltbar (`DL_LLM_PROVIDER_BOT_PATE`);
   der frühere Entscheid „gpt-5.4-mini" vom 02.07. ist damit überholt. Der System-Prompt
   kommt aus dem Textpaket (Concierge v0.3). Gesprächsprinzip:
   **immer offene Fragen zuerst, geschlossene Fragen und Buttons nur zum
   Präzisieren.**
4. **Stille Intent-Klassifikation** aus dem Gespräch in genau eine von vier
   Kategorien: `improve` (besser werden), `mates` (feste Mitspieler),
   `learn` (Spiel lernen), `casual` (entspannt zocken). Wird im Profil
   gespeichert, nie dem User als Formular gezeigt.

## 3. Die drei Ausgänge (jede Strecke endet Richtung Mensch)

- **Spielen:** Preset in <#1513468476365209670> (📖sprachkanal-verwalten)
  erklären, dann Voice **Deadlock Router** (`1513468587195633674`), der
  automatisch in eine passende Lane verteilt oder eine erstellt. Ergänzend
  🎯mitspieler-suche (`1522769149208821881`). Bei `learn` zusätzlich
  Coaching-Hinweis auf <#1494373349944459355>.
- **Fragen:** Antworten über `dl-knowledge` `POST /public/v1/ask`
  (BM25 + LLM, fail-closed). Bei Lücke KEIN Raten: Verweis auf
  <#1426220702054355077> (💬frag-die-community) nach dem Lücken-Dreiklang
  aus dem KI-Charakter-Design (nie ratlos, jede Antwort endet mit einer
  Handlung). „Ich find's für dich raus" bleibt verboten (kein Cockpit).
- **Umschauen:** Mini-Tour, 6 Stationen (Texte im Textpaket), als eine
  V2-Nachricht mit Sektionen, kein Klick-Marathon:
  1. <#1326973956825284628> 📝patchnotes
  2. <#1304169815505637458> 🎥deadlock-streamer (hallo sagen, lurken)
  3. <#1426220702054355077> 💬frag-die-community (Fragen, Game-Invite)
  4. <#1494373349944459355> 🛠️ich-brauch-einen-coach
  5. <#1513468476365209670> 📖sprachkanal-verwalten (Preset)
  6. Voice „Deadlock Router" (verteilt in Lanes)
  Die Tour endet IMMER im Steckbrief-Angebot.

## 4. Steckbrief

- Spieler-Sicht, keine Persona-Vorstellung: Rang, Modus, Ziele, konkrete
  Aufforderung („wer hat Bock, mit mir zu spielen oder mir zu helfen?").
- Der Concierge entwirft ihn **in Ich-Form** als Gedankenbrücke aus
  Gespräch + Rang (Rang-Verknüpfung, falls vorhanden) + Intent. Der User
  passt an oder verwirft; nichts wird ohne explizite Freigabe gepostet.
- Preview in der DM mit Buttons: Posten / Anpassen / Lieber nicht.
  „Anpassen" = Modal mit dem Entwurf als editierbarem Text.
- **Routing:** sucht der Steckbrief aktiv Hilfe oder einen Game-Invite →
  <#1426220702054355077>; lockere Vorstellung → <#1289721245281292291>
  (🌐allgemein). Der Concierge sagt dem User, wohin der Post geht.
- **Nie ins Leere:** Post nur, wenn der Server gerade aktiv ist
  (Presence-/Aktivitätssignal, Schwelle konfigurierbar). Sonst hält der
  Concierge den Post zurück und postet später, mit kurzer Info an den User.
- Nach dem Post: Concierge reagiert selbst als Erster (Emoji-Reaktion +
  ein kurzer Reply, Text im Textpaket), dezenter Mod-Ping (konfigurierbar,
  Rolle per Config).

## 5. Kadenz (hart begrenzt)

Maximal **3 ungefragte Kontakte**, danach nur noch reaktiv:

1. **T0** (Join-Tag): Begrüßung wie §2.
2. **T+2** nur bei Null-Aktivität (nichts geschrieben, kein Voice): ein
   einladender Nudge mit konkretem Anlass (z. B. aktive Lanes im
   Rang-Bereich), hier hängt das **Paten-Angebot** dran (§7).
3. **T+7** Reibungs-Feedback („war was verwirrend?"), übernimmt die
   Exit-Reibungs-Erhebung aus Konzept Phase 0/4.

Zusätzlich einmalig, zählt nicht als Kontakt: **Gratulation** beim ersten
Erfolgsmoment (erste eigene Nachricht ODER erster Voice-Join), kurz,
freundlich, ohne Frage und ohne Aufforderung.

Jede DM an den Concierge wird jederzeit beantwortet (reaktiv, ohne Limit).

## 6. Gedächtnis & Datenschutz

Zwei Schichten in Postgres:

- `concierge_conversations`: Roh-Verlauf (Runden wörtlich), damit das LLM
  nahtlos weiterchatten kann.
- `concierge_profiles`: Destillat wie bei einem Support-Agenten: Intent,
  Rang, Spielzeiten, Funnel-Status, was schon versucht wurde
  (Steckbrief ja/nein, Tour ja/nein, Pate angeboten/gewünscht).

Regeln:

- **Retention 90 Tage rollierend ab letzter Konversation** für BEIDE
  Schichten. Jede Interaktion refresht die Uhr. Nach 90 Tagen Stille:
  Verlauf löschen, Profil löschen oder anonymisieren (Reaper-Job, Muster
  bestehende Retention `retention.rs`).
- **Erasure:** an das bestehende Lösch-System anbinden, Concierge-Daten
  gehören zum Erasure-Umfang.
- **Opt-out:** „stopp" als Keyword UND natürlichsprachlich erkannt
  („lass mich in Ruhe", „schreib mir nicht mehr"). Wirkung: Status
  `opted_out`, keine ungefragten DMs mehr, reaktiv bleibt möglich.
  „Vergiss das/mich" → sofortige Löschung von Verlauf + Profil,
  Bestätigung an den User.
- Transparenz-Satz in T0 (Textpaket), kein verstecktes Gedächtnis.

## 7. Pate (Schnittstelle, Vollausbau = Slice B)

- Pool = Rolle `1524047896297738311`. Trigger: User äußert Wunsch, oder
  Angebot im T+2-Nudge. Nur mit Zustimmung, nie ungefragt zugeteilt.
- **Slice A baut nur die Übergabe:** Zustimmung wird im Profil vermerkt
  (`pate_requested`), der Concierge pingt die Paten-Rolle in einem
  Mod-/Paten-Kanal mit dem Steckbrief-Destillat. Das Matching
  (Rang-Nähe, Spielzeiten, Load-Limit 2-3, privater Textkanal mit
  Concierge-Vorstellung) ist Slice B.

## 8. Optik

ALLE Concierge-Nachrichten als **Components V2** (Muster LFG-Panel /
Twitch-Embeds): Gold `0xC8A86B`, `dl_*`-Emojis, Grafiken (Header-Banner)
aus dem Brand-Asset-Fundus. Professionell und schick ist Teil des Konzepts,
kein Nice-to-have. Assets liefert Claude, Codex bindet nur ein.

## 9. Nicht-Ziele Slice A

- Kein Umbau/Abriss des bestehenden FAQ-Chats (bleibt parallel).
- Keine Persona-Übernahme der Alt-DM-Streams (Steam-Nudge, Voice-Feedback,
  LFG-Watch) — das ist Slice C (erst Persona/Ton, dann gemeinsames
  Gedächtnis).
- Kein Paten-Matching-Automat (Slice B).
- Kein Cockpit, kein „ich find's für dich raus".
- Bestandsuser werden nicht angeschrieben; sie erreichen den Concierge,
  indem sie ihm eine DM schreiben.

## 10. Launch-Gates (vor Aktivierung für echte neue Joins)

1. **Fireworks-DPA/AVV als Nachweis abgelegt** (Owner bestätigt 07.07.,
   dass Fireworks das anbietet; Dokument sichern, §5.6-Anforderung
   „Prozessor mit DPA + No-Training-Zusage" gilt weiter). Bis dahin läuft
   der Concierge im Testmodus hinter Config-Flag mit Test-Allowlist
   (nur benannte Test-User bekommen T0).
2. Texte final von Claude reviewt (Codex schreibt KEINE finalen
   user-sichtbaren Texte; fehlt ein Text, `"Platzhalter"` + Meldung).
3. Vertragstests grün (§11).

## 11. Tests (TDD, Vertrag zuerst)

- Kadenz: nie mehr als 3 ungefragte Kontakte; T+2 nur bei Null-Aktivität;
  Gratulation genau einmal; nach `opted_out` keine ungefragte DM mehr.
- Retention: Reaper löscht/anonymisiert exakt nach 90 Tagen Stille;
  Interaktion refresht; Erasure räumt Concierge-Tabellen mit ab.
- Routing: Hilfe/Invite-Steckbrief → frag-die-community, locker →
  allgemein; Presence-Gate hält Posts zurück.
- 50007: Fallback-Kanal wird genau einmal erstellt (Claim).
- Lücken-Verhalten: dl-knowledge fail-closed → Verweis-Antwort, nie Raten
  (Fixture-Test gegen den Prompt-Vertrag, soweit deterministisch testbar).

## 12. Umsetzung

Implementierung durch Codex-Worker auf diesem Branch
(`feature/onboarding-concierge`), Review + finale Texte + Verifikation
durch Claude, Merge → Deploy → Live-Beweis nach Standardablauf.
Wissensdoku nach Abschluss in Deadlock-Docs (`internal/deadlock-bots/`).
