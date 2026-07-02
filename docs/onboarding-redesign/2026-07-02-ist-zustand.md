# Onboarding-Redesign — Ist-Zustand (Stand 2026-07-01/02)

Vollaufnahme des Servers „Deutsche Deadlock Community" (Guild `1289721245281292288`)
vor dem Onboarding-Neubau. Datenquellen: Discord-API-Snapshot über den Bot (Guild,
Kanäle, Rollen, natives Onboarding, Threads, letzte 50 Nachrichten aller 75 Textkanäle),
Server-Insights-CSVs (archiviert unter `docs/insights/discord/2026-07-01/`),
Code-Analyse Rust + Python (4 parallele Analyse-Agenten), qualitative
Nachrichten-Auswertung (5 Agenten über alle Kanal-Gruppen).

## 1. Harte Zahlen (Server-Insights, März–Juni 2026)

| Monat | Neue Mitglieder | % kommuniziert | % Kanäle geöffnet |
|---|---|---|---|
| März | 232 | 36,2 % | 60,3 % |
| April | 175 | 33,1 % | 57,7 % |
| Mai | 144 | 28,5 % | 54,9 % |
| Juni | 123 | 27,6 % | 53,7 % |

- Retention nach 1 Woche: 28–50 % (schwankend)
- ~50 % der Leaver gehen im ersten Monat
- Joins: primär Vanity-URL + Invites; Discovery ~15–20 %
- Gesamt: 2368 Mitglieder, ~590 online. Zwei getrennte Metriken: Tabelle oben =
  Aktivierung **neuer** Mitglieder (`guild-activation.csv`); serverweite
  Kommunikator-Quote aller Besucher fällt separat 33 % → 25 %
  (`guild-communicators.csv`)
- Boost-Level 2 (10 Boosts), Locale de, DISCOVERABLE, COMMUNITY

**Kernbefund:** Zulauf, Aktivierung und Kommunikator-Quote sinken seit 4 Monaten
gleichzeitig. Churn ist ein Erste-Wochen-Problem.

## 2. Der Ist-Funnel: fünf parallele Systeme

Ein Neuer durchläuft bis zu fünf Systeme, die sich gegenseitig nicht kennen:

1. **Natives Discord-Onboarding** (aktiv, 5 Prompts): Erziehungs-Prompt („⚠️ Skippe
   nicht einfach alles durch"), Interessen-Frage (vergibt Ping-Rollen), Rang-Frage
   (12 Optionen → „(unverifiziert)"-Rollen), Interessen/Kanäle-Frage, Meta-Prompt
   („Denk daran, das Onboarding abzuschließen :)"). Default-Kanäle enthalten
   mehrere seit Monaten tote Kanäle.
2. **Discord Member-Screening** (Regeln akzeptieren, `MEMBER_VERIFICATION_GATE_ENABLED`).
3. **Bot-Wizard** (Rust, `dl-community/onboarding.rs` + `rust/bin/dl-bot/src/onboardglue.rs`
   — in Letzterem leben auch die `wdm:`-Legacy-Handler samt Gate-Rollen-Vergabe,
   die `pending_verify`-SQL-Implementierung und die `dma:fallback`-Handler): 10
   Embed-Schritte in privatem Thread im Regelwerk-Kanal, Auto-Start nach Screening
   oder Panel-Button `rp:panel:start`. **195 Zombie-Threads** im Regelwerk-Kanal.
4. **AI-Onboarding** (`ai_onboarding.rs`, `aiob:*`): Panel-Posting nie verdrahtet
   (Trigger lebt im abgeschalteten Python `ai_connector`) — effektiv tot.
5. **KI-DM-Assistent + FAQ-System** (`dm_assistant.rs`, `faq.rs`): MiniMax-Freitext;
   scheitert nachweislich an Anfängerfragen.

### Kritische Code-Befunde (Rust)

- **Der Wizard vergibt selbst keine Rolle.** Das Zugangs-Gate (Onboard-Complete
  `1304216250649415771`) hängt an Legacy-Buttons `wdm:q4:confirm` /
  `aiob:rules_confirm`. `ob:done` schreibt nichts Persistentes → kein
  Completion-Tracking, keine Funnel-Messung.
- Abschluss-Erkennung fragil: `RoleEvent::Gained` nur aus Cache-Diff; verpasstes
  Event = Abschluss-Nachricht kommt nie; `bot.onboarding_pending_verify` ohne
  Expiry/Reconcile.
- Streamer-Schritt verspricht Buttons („Setup starten", „📊 Demo ansehen"), die der
  Rust-Wizard nie rendert.
- `rp:panel:start` ohne Dedupe → Mehrfachklick = beliebig viele Threads.
- Drei custom_id-Generationen parallel (`wdm:`, `aiob:`, `ob:`/`rp:`); alte
  Nachrichten tragen alte IDs. `wdm:*`-Restpfade antworten mit Platzhalter.
- DSGVO-Handling (`privacy.rs` Z. 661–843) hartkodiert auf `ai_onboarding`-KV-Namespaces.
- FAQ-Prompt kodiert den Onboarding-Ablauf als Prosa (faq.rs Z. 67–101) — stille
  Drift-Falle bei jedem Umbau.
- `/publish_rules_panel` postet immer neu statt zu editieren (Duplikat-Risiko).

### DB-Berührungspunkte des Alt-Systems

`bot.kv_store` (ns `onboarding:auto_start`, `ai_onboarding:persistent_views`,
`ai_onboarding:sessions`), `bot.onboarding_pending_verify`, `core.steam_links`
(READ), `core.user_tags` (WRITE tone/age), `core.user_privacy`,
`activity.member_events`, `activity.member_leave_surveys` (Migration
`0010_activity_moderation_content_patchnotes.sql`). ETL-Ledger `bot.toml`
referenziert die KV-Namespaces.

### Python-Altlast (Bot disabled, Code liegt noch)

Drei parallele Systeme (StaticOnboarding-Wizard, `welcome_dm`-Paket mit
Streamer-OAuth-Flow, totes `ai_onboarding`). Python hatte, was Rust nicht hat:
In-Discord-Twitch-OAuth-Streamer-Flow (in Rust bewusst durch Website ersetzt),
Content-Creator-Spezial-View, Freundescode-Modal. Bekannte Python-Schwächen:
kein Restart-Persist, kein Dedupe, Text-Bugs, Tippfehler („Onboading").

### Angrenzende Systeme (Wiederverwendbarkeit)

| System | Zweck | Beim Neubau |
|---|---|---|
| reaction_roles | Reaktion→Rolle + Einmal-DM | ✅ reifer Baustein |
| rules-panel | Panel + Wizard-Einstieg | ⚠️ im selben Struct wie Wizard — Abriss reißt Panel mit |
| leave_survey | Exit-Umfrage (Bucket `onboarding_failed`) | ✅ Mess-Instrument, weiterlaufen lassen — **aber DMs schlagen aktuell 6/6 fehl** |
| retention | Aktivitäts-Tracking + Miss-You-DM | ✅ nicht anfassen |
| faq | KI-Selbsthilfe | ⚠️ Prompt muss beim Neubau nachgezogen werden / System stirbt |
| coaching | Website-Panel + Claim-Flow | ✅ Vorbild fürs Paten-Cockpit |
| tempvoice/adaptive | Lanes + Neue-Spieler-Routing | ✅ Träger des neuen Lane-Modells; konsumiert Verified-Rolle (Literal dupliziert in engine.rs) |
| lfg/player_finder | Lobby-Matching | ✅ Verweis-Ziel |

## 3. Server-Struktur-Befunde

### Das „Gate" ist eine Fiktion

`@everyone` hat global VIEW+SEND+CONNECT+SPEAK; die Kern-Kanäle haben keine
@everyone-Denies. Die „Deadlocker"-Rolle schaltet **keinen einzigen versteckten
Kanal** frei — sie vergibt nur Komfort-Rechte (Reactions, Embeds, Attachments,
externe Emojis, Threads, Polls, Slash-Commands, Voice-Extras). Wer den Wizard
ignoriert, sitzt dauerhaft ohne Reactions/Bilder da, ohne zu wissen warum.
Versteckt sind nur Funktionsbereiche (Mod, VIP, Streamer, Coaching-Teams,
Tickets, Beta, „Alt"). Overwrite-Wildwuchs: individuelle User-Bans als
Channel-Overwrites, redundante English-Only-Allows, Soundboard-Denies verstreut.

### Kanal-Aktivität (75 Textkanäle)

- **Lebendig:** allgemein (~93 Msg/Tag, 10 Autoren), Coaching/Scrim-Teams (sehr
  aktiv), off-topic, spieler-suche (20 Autoren!), memes (21 Autoren), rank-ups,
  haatteee (10 Autoren), leaks, yt-videos, caster-chat
- **Tot (>14 Tage, teils >12 Monate):** 36 Kanäle, darunter der komplette
  Eingangsbereich (server-faq seit April, rang-auswahl seit April,
  ich-brauch-einen-coach seit April, feedback-kanal seit Mai, lag-kompensator
  seit März) und die Kategorie „Alt" (14 Kanäle Friedhof)
- **Bot-Monologe:** twitch (100 % Bot, doppelte Offline-Embeds, 0-Viewer-Pings),
  patchnotes (Doppel-Feed deutsch+englisch), dev-updates (92 % Bot),
  beta-invite-Kanal (100 % Bot-Spam)
- System-Kanal (Join-Meldungen) = `bot-logs` (Mod-Kanal) → **kein öffentlicher
  Willkommens-Moment**

### Rollen (~190)

66 Rang-Rollen (verified + unverifiziert), 31 Streamer-Ping-Rollen (16 „LIVE
PING" + 15 „ist live"), ~30 Meme-/Insider-Rollen (darunter `Vollzeit Rassist` /
`Teilzeit Rassist` — Discovery-Risiko), Zombie-Rollen („neue Rolle", „Streamer
Onboading", „Non Streamer Partner"), Divider-Rollen. Discord-Limit: 250 Rollen.
**Achtung:** Die `Lurker`-Rolle (1447747896253485127) ist KEIN Zombie — sie
gehört zum aktiven TempVoice-„👻 Lurker"-Feature (`tv_lurker`-Button in
`tempvoice/engine.rs`/`interface.rs`: Rolle + Nick + User-Limit +1, mit
DB-Store und Cleanup).

## 4. Was die echten Nachrichten zeigen

### Beta-Invite = größter dokumentierter Churn-Hebel

Dokumentierter Komplett-Fail (01.07.2026): Neuling startet Invite → 6 identische
„⏱️ Zeitlimit erreicht"-Pings in 27 Min → Ticket → Auto-Bot beantwortet die
Kernfrage („welchem bot soll ich eine fa schicken?") nicht → User verlässt den
Server nach <1 Tag. Gleichzeitig widersprüchliche Kommunikation: user-sichtbar
„Freundschaft bestätigt, du musst nichts tun" vs. bot-logs „nicht bestätigt nach
5 Versuchen". Panel erklärt nicht, dass Steam-Link + Freundschaftsanfrage nötig
sind; Wartezeiten nirgends kommuniziert; funktionierender Workaround = manuelles
Einladen durch Mods (inoffizieller Goodwill).

### Weitere Kern-Reibungen

- **Leave-Survey-DMs schlagen zu 100 % fehl** (6/6 „DM: failed" in bot-logs) —
  von Abbrechern erfährt der Server nie, warum sie gehen.
- Neue stellen sich fast nur in `spieler-suche` vor; viele bekommen **nur die
  Bot-Antwort oder gar keine**. Zwei explizite „bringt mir jemand das Spiel
  bei"-Bitten blieben unbeantwortet — trotz kostenlosem Coaching-Programm.
  Coaching-Funnel und Hilfesuchende finden nicht zueinander.
- FAQ-Bot scheitert an Anfänger-/Server-Fragen; der Owner rettet manuell
  (dokumentiert bis nachts 3:43 Uhr); ohne Rettung bleibt die Frage unbeantwortet.
- Steam-Panel steht unter irreführendem Namen `rang-auswahl` (dort vorbildliche
  Datenschutz-Erklärung — behalten!); Stolperfalle Freundescode 820142646.
- Englische Bot-Texte auf deutschem Server (TicketTool „Support will be with you
  shortly", DeadlockAssistant „Please use: /store first!").
- Scrim-Bereich: bester Kultur-Kern (Anti-Toxicity-Manifest, Peer-Mentoring,
  10/10-Testimonials), aber organisatorisch chaotisch (Permissions, Termine als
  Screenshots, fehlender „Coaching abschließen"-Button).
- `lag-kompensator` enthält Suizid-„Witz" in semi-offizieller Owner-Nachricht.

## 5. Evidenz-Recherche: Was aktiviert Newcomer wirklich?

Drei Recherche-Agenten (Discord-offiziell, Community-Forschung, Gaming-Praxis):

**Hart belegt:**
- Menschliche Antwort auf den ersten Beitrag: Wiederkehr 44 % → 56 % (+12 pp,
  n=2.777; Antwort-*Qualität* egal) — Joyce & Kraut 2006
- Menschlich besetzter Newcomer-Raum mit Antwort-Garantie wirkt (Wikipedia-
  Teahouse-RCT, n=14.766; kleiner, signifikanter Effekt)
- Sichtbare Normen am Einstiegspunkt: +70 % Newcomer-Beteiligung (r/science-RCT)
- Negative Erstreaktion = stärkster Retention-Killer (400k Wikipedia-Revisionen)
- Bindung durch soziale Dichte / wiederholtes Zusammenspiel, nicht Ko-Präsenz
  (WoW-Gilden-Längsschnitt)

**Ebenso hart belegt (Null-Effekte):**
- Generische Auto-Willkommensnachricht: kein Effekt (French-Wikipedia-RCT, n=57.084)
- Reine UI-/Technik-Onboarding-Interventionen: durchgängig wirkungslos
  (Wikimedia Growth Team 2012–2014)

**Folklore:** Discords eigene Feature-Empfehlungen sind zahlenfrei; die
kursierende „80 % mehr Retention durch Onboarding" ist eine quellenlose
SEO-Erfindung.

**Synthese wörtlich:** „Welcome-Kanäle aktivieren nicht — Menschen, die dort
antworten, tun es. Der Kanal ist nur so gut wie die garantierte Erstreaktion
dahinter."

## 6. Rohdaten

- API-Snapshot + Nachrichten-Dumps: Session-Scratchpad (flüchtig); reproduzierbar
  über Bot-Token + Guild-ID
- Insights-CSVs: `docs/insights/discord/2026-07-01/` (dauerhaft, mit Refresh-Routine)
- Analyse-Volltexte: Workflow-Journale der Session vom 2026-07-01/02
