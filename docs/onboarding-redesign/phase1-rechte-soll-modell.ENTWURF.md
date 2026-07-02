# Soll-Rechte-Modell „Deutsche Deadlock Community" — ENTWURF (Phase 1, Owner-Review)

Status: **Entwurf zur Review** (Arbeitspaket „Soll-Rechte-Modell dokumentieren", Konzept §5.1/§5.3; Owner-Review = Definition of Done).
Quellen: Live-Snapshot `discord-live/channels.json` + `roles.json` (Stand 2026-07-01 23:50) und Konzept `docs/onboarding-redesign/2026-07-02-konzept.md` (§3, §4.6, §4.7, §5.1, §5.3).
Hinweis: Der Snapshot ist vor den P0-Fixes gezogen — die beiden „Rassist"-Rollen sind inzwischen umbenannt (P0-⑧), die Rechte-Lage ist davon unberührt.

**Leitplanken aus dem Konzept:**
- `Deadlocker` (1304216250649415771) stirbt; **@everyone wird die Basis** — offenes Zugangsmodell, kein Komfort-Gate mehr (§3).
- Weiche-/Ping-Rollen sind **reine Präferenz-Signale, nie Berechtigungs-Signale** (§2).
- Dynamische Namespaces (TempVoice, Tickets, Fallback-Kanäle) sind **deklarierte, verwaltete Ausnahmen** (§5.1).
- Bestehende **User-Ban-Overwrites werden als dokumentierte Ausnahmen migriert**, nicht stillschweigend weggeräumt (§5.1).
- Default = **melden statt revertieren**; Auto-Revert nur für Whitelist sicherheitskritischer Rechte (§5.1).

---

## 1. Rechte-Klassen

Jeder Kanal/jede Kategorie bekommt genau EINE Klasse. Kanäle erben per Kategorie-Sync; `CUSTOM` nur, wo hier deklariert.

| Klasse | Bedeutung | Overwrite-Muster |
|---|---|---|
| **P0 öffentlich** | sehen + schreiben für alle | **keine Overwrites** (alles aus @everyone-Basis) |
| **P1 Verlautbarung** | alle lesen, niemand schreibt | `@everyone: −SEND, −CREATE_PUBLIC_THREADS, −SEND_MESSAGES_IN_THREADS`; Sender-Rollen per Allow |
| **P2 Panel** | wie P1, aber ein Bot betreibt ein Panel | P1 + Bot-Rolle (managed): `+SEND, +EMBED_LINKS` |
| **F Funktionsbereich** | nur Funktionsrolle sieht den Bereich | `@everyone: −VIEW`; Funktionsrolle(n): `+VIEW` (Voice zusätzlich `+CONNECT`); `Community Moderator: +VIEW,+CONNECT` (Owner/Moderator brauchen NICHTS — beide Rollen tragen ADMINISTRATOR) |
| **D dynamischer Namespace** | Kanäle, die ein System zur Laufzeit anlegt/verwaltet | vom deklarativen Sync **ausgenommen**, eigene Regeln je System (s. 3.11) |
| **X dokumentierte Ausnahme** | individuelle User-Overwrites mit Grund | Ausnahme-Registry (s. Abschnitt 4), im DB-Modell mit `grund`, `angelegt_am`, `review_am` |

**Funktionsbereiche im Soll:** Moderation, VIP, Streamer, Coaching (konserviert, s. 3.6), Turnier/Caster (nur Caster-Chat + Caster-VC).
**Dynamische Namespaces:** TempVoice-Lanes (dl-voice), Ticket-Kanäle (TicketTool = dokumentierte Fremd-Schreibinstanz), Bot-Paten-Fallback-Kanäle (ab Phase 4).

---

## 2. @everyone-Basisrechte (Guild-Level) — Vorschlag

Befund: @everyone hat heute guild-weit bereits fast identische Rechte wie `Deadlocker` (Unterschied: Deadlocker hat zusätzlich nur USE_EXTERNAL_APPS). Das „Gate" war schon bisher weitgehend zahnlos — der Abriss ändert an der Basis fast nichts, er räumt vor allem die Kanal-Overwrites auf.

**Basis (behalten/setzen):**
VIEW_CHANNEL, SEND_MESSAGES, READ_MESSAGE_HISTORY, ADD_REACTIONS, EMBED_LINKS, ATTACH_FILES, USE_EXTERNAL_EMOJIS, USE_EXTERNAL_STICKERS, CREATE_PUBLIC_THREADS, SEND_MESSAGES_IN_THREADS, SEND_POLLS, SEND_VOICE_MESSAGES, CONNECT, SPEAK, STREAM, USE_VAD, SET_VOICE_CHANNEL_STATUS, USE_EMBEDDED_ACTIVITIES, REQUEST_TO_SPEAK, CHANGE_NICKNAME, CREATE_INSTANT_INVITE, USE_APPLICATION_COMMANDS ⚑

**Entfernen (heute vorhanden):**
- SEND_TTS_MESSAGES (Störpotential, kein Nutzen) ⚑
- CREATE_PRIVATE_THREADS (Konzept §4.1: „Private Threads sterben serverweit") ⚑
- Bit 47 (undokumentierte Altlast im Snapshot)

**Weiterhin NICHT vergeben (wie heute):** MENTION_EVERYONE, USE_SOUNDBOARD, USE_EXTERNAL_SOUNDS, USE_EXTERNAL_APPS, alle Mod-/Verwaltungsrechte.
Folge: alle heutigen `−SOUNDBOARD/−EXT_SOUNDS`-Denies in den Lanes sind redundant und entfallen.

⚑ = Owner-Entscheid nötig (s. Abschnitt 6).

---

## 3. Soll-Overwrites pro Kategorie (minimal)

Notation: nur die deklarierten Regeln; alles andere = Kategorie-Sync bzw. Basis. Kanalnamen = Zielzustand nach Kanal-Sanierung (§4.6), heutiger Name in Klammern.

### 3.1 Moderation — Klasse F
| Objekt | Overwrites |
|---|---|
| Kategorie | `@everyone: −VIEW` · `Community Moderator: +VIEW,+CONNECT` |
| moderator-only, community-moderator, Mod Voice | SYNC |
| bot-logs (stirbt Phase 3 → Cockpit) | + `Deadlock Patchnotes (managed): +VIEW,+SEND` |
| caster-chat | + `Turnier Caster: +VIEW` · `Turnier Moderation: +VIEW` · `Coach: +VIEW` |

Entfällt: alle Owner-/Moderator-Overwrites (ADMIN), User-Allows (s. 4.2), DL-Master-2-Overwrites (Bot-Rolle hat ADMIN).

### 3.2 Eingangsbereich — Klasse P1/P2
| Objekt | Overwrites |
|---|---|
| Kategorie | `@everyone: −SEND, −PUB_THREADS, −THREAD_MSG` (P1) |
| regelwerk (hier-starten-regelwerk), dev-updates | SYNC |
| ankündigungen | + `Community Moderator: +SEND,+MENTION_EVERYONE` ⚑ (heute senden auch Turnier Moderation/Coach/Caster — Owner-Entscheid 6.3) |
| patchnotes | + `Deadlock Patchnotes (managed): +SEND` |
| deadlock-rang (rang-auswahl) | P2: + Steam-/Rang-Bot-Rolle (managed `DL-Rang`): `+SEND,+EMBED` — einziges öffentliches Button-Panel (§4.3) |
| server-support (lag-kompensator) | Abweichung: `@everyone: +SEND` (öffentlich schreibbar) ⚑ |

faq-\*-Kanäle: archiviert, Overwrites entfallen.

### 3.3 Chat — Klasse P0
Kategorie + allgemein, off-topic, spieler-suche, frag-die-community (community-fragen), **deadlock-invite** (beta-zugang; §4.4 „jeder kann schreiben"): **0 Overwrites**.
Bot-Paten-Fallback-Kanäle liegen hier (über `allgemein`) als **Namespace D** (s. 3.11).
Entfällt: Deadlocker-, English-Only-, Server-Booster-, CM-Overwrites (alle redundant zur Basis bzw. ADMIN); rang-auswahl zieht als P2 in den Eingangsbereich.

### 3.4 Medien — Klasse P0, zwei P1-Feeds
| Objekt | Overwrites |
|---|---|
| Kategorie, game-guides, gameplay-clips, yt-videos | 0 (P0) |
| twitch (Live-Feed) | `@everyone: −SEND, −PUB_THREADS, −THREAD_MSG` (P1; Twitch-Bot sendet via ADMIN-Bot-Rolle) |
| stream-updates (heute in Streamer Only) | ⚑ Owner-Entscheid 6.4: öffentlicher Feed (P1, hierher) oder bleibt Streamer-intern |

### 3.5 Sonstiges — Klasse P0
Kategorie + alle Kanäle (rank-ups, leaks, memes, bot-spam, feedback, haatteee, nsfw, Zusammenlegungs-Experiment movement/art/mods/food): **0 Overwrites**. ⚑ rank-ups als Bot-Feed read-only? (6.5)
NSFW-Gating läuft über das Kanal-Flag `nsfw`, nicht über Rechte.

### 3.6 Coaching — Klasse F, KONSERVIERT
Der Bereich bleibt fachlich unangetastet (§4.6, eigenes Projekt). Im Soll-Modell v1 wird er als eigener Block übernommen, aber normalisiert:
- raus: leere Zombie-Overwrites, Owner/Moderator-Redundanz, 40-Bit-Voll-Masken → äquivalente Minimal-Sets (z. B. Komplett-Ausschluss = `−VIEW` statt 40 Einzel-Denies),
- User-Bans → Ausnahme-Registry (Abschnitt 4), Team-Kapitäns-User-Overwrites → Team-Rollen (Ticket fürs Coaching-Projekt),
- Struktur (Coach / Coaching Zugriff / Coaching All / Coaching Feedback / Team 1-4 / Team Leo / Team Deniz) bleibt.
Ist: 134 Overwrites; Interim-Soll ≈ 90; Ziel nach Coaching-Detail-Sanierung ≈ 25.

### 3.7 VIP — Klasse F
Kategorie: `@everyone: −VIEW` · `VIP: +VIEW,+CONNECT` · `Server Booster: +VIEW,+CONNECT` · `Server Unterstützer: +VIEW,+CONNECT`. Beide Kanäle SYNC.
(Ersetzt die heutigen 24-40-Bit-Masken; Rest kommt aus der Basis.)

### 3.8 Streamer — Klasse F
Kategorie: `@everyone: −VIEW` · `Streamer: +VIEW,+CONNECT`.
Streamer VC: + `Streamer VC Zugriff: +VIEW,+CONNECT`.
Entfällt: `Non Streamer Partner` (Rolle stirbt, §4.7), VIP-/Booster-/Unterstützer-Masken ⚑ (6.12: sollen VIP/Booster weiter reinschauen dürfen?).

### 3.9 Lanes (Voice)
| Bereich | Overwrites |
|---|---|
| Chill Lanes (TempVoice-Namespace D) | Kategorie: `Coach: +MOVE` — sonst 0 |
| Deadlock Router (Namespace D) | 0 |
| Street Brawl (Namespace D) | Kategorie: `Coach: +MOVE` — sonst 0 |
| Competitiv Lanes (Übergangsbestand bis Phase 5, §4.5) | Kategorie: `@everyone: −CONNECT` · `Steam Verifiziert️✅: +CONNECT` · `VC Move Rechte: +CONNECT` |
| Neue Spieler Lanes | Kategorie: `Coach: +MOVE` |
| Custom Game | Kategorie: 0 · Caster Channel: `@everyone: −CONNECT` + `Turnier Caster: +CONNECT` · Team 1/Team 2: 0 |
| AFK | `@everyone: −SPEAK, −STREAM, −SEND` |

`VC Move Rechte` hat MOVE_MEMBERS bereits guild-weit → alle heutigen `+MOVE`-Kanal-Overwrites der Rolle sind redundant und entfallen; nur `+CONNECT` in gegateten Lanes bleibt.
Funny/Grind-Custom-Ping-Overwrites entfallen komplett (Präferenz ≠ Berechtigung, §2).
Ab Phase 5 ersetzen offene Rang-Interesse-Lanes das Competitiv-Gate (Deklaration statt Türsteher) → dann fällt auch dieser Block.

### 3.10 Support/Tickets — Klasse P1 + Namespace D
| Objekt | Overwrites |
|---|---|
| Kategorie | `Ticket Tool (managed): +VIEW,+SEND,+MANAGE_CHANNELS,+MANAGE_MESSAGES,+EMBED,+ATTACH,+HISTORY` (dokumentierte Fremd-Schreibinstanz) |
| ticket-eröffnen | `@everyone: −SEND` (Panel von TicketTool) |
| ticket-N / closed-N | **Namespace D** — TicketTool verwaltet; Muster: `@everyone: −VIEW` + Ersteller `+VIEW,+SEND`; Drift-Check ignoriert diese Kanäle, prüft nur das Muster |

Der TicketTool-**User**-Overwrite (557628352828014614) wird durch die managed **Rolle** ersetzt.

### 3.11 Dynamische Namespaces — Regeln
| Namespace | System | Muster (Soll je Kanal) | Lebenszyklus |
|---|---|---|---|
| TempVoice-Lanes | dl-voice (engine.rs; konsumiert `Steam Verifiziert️✅` — Abriss-Vorsicht §6) | Ersteller-Overwrites gemäß Lane-Einstellungen (Lock/Hide/Limit), Lurker-Feature (`tv_lurker`) | auto-delete bei Leerung |
| Ticket-Kanäle | TicketTool (extern) | s. 3.10 | close → archive/delete durch TicketTool |
| Fallback-Kanäle (Phase 4) | Bot-Pate | `@everyone: −VIEW` · User: `+VIEW,+SEND` (nur User+Bot) | Auto-Cleanup nach 14 Tagen Inaktivität (§4.1) |

### 3.12 Kategorie „Alt" + Beta Zugang
Werden archiviert (§4.6). Im Soll-Modell existieren sie nicht; bis zur Archivierung gilt `@everyone: −VIEW` (Ist-Zustand von „Alt"). Die 54 Overwrites auf diesen Objekten entfallen mit der Archivierung. Die Rolle `Beta Zugang benötigt` verliert mit der offenen Invite-Lounge ihre letzte Funktion ⚑ (6.6).

---

## 4. Dokumentierte Ausnahmen (User-Overwrites aus channels.json)

### 4.1 User-Ban-Overwrites → Ausnahme-Registry (werden migriert, §5.1)
| # | User-ID | Ist (Snapshot) | Soll (Registry-Eintrag) |
|---|---|---|---|
| X1 | 685573558281175043 | 18 Objekte (Chat-, Medien-, Sonstiges-Bereich): deny VIEW,SEND,ADD_REACTIONS,EMBED,ATTACH,EXT_EMOJI/STICKER,Threads (tw. +MENTION_ALL) | 3 Regeln auf Kategorie-Ebene (Chat, Medien, Sonstiges): `−VIEW,−SEND` ⚑ 6.9 |
| X2 | 496268533496545283 | 30 Objekte (Chat, Sonstiges, Coaching inkl. Teams): deny VIEW,SEND,Threads | 3 Regeln auf Kategorie-Ebene (Chat, Sonstiges, Coaching): `−VIEW,−SEND` ⚑ 6.9 |
| X3 | 364796363709349912 | 17 Coaching-Objekte: 40-Bit-Voll-Deny; + low-elo-ranked `−VIEW` (Alt, entfällt); + namensvorschläge read-only (Alt, entfällt) | 1 Regel Kategorie Coaching: `−VIEW` (äquivalent, minimal) |
| X4 | 601742833438818357 | 16 Coaching-Objekte: `−CONNECT`; + Neue Spieler Lanes 2×: `−DEAFEN` | 1 Regel Kategorie Coaching: `−CONNECT`; die `−DEAFEN`-Einträge sind wirkungslos (User hat das Recht ohnehin nicht) → löschen ⚑ 6.10 |
| X5 | 702594328328929331 | high-elo-ranked: `−VIEW` bei gleichzeitigen Allows (widersprüchlich; Kanal in „Alt") | entfällt mit Archivierung — kein Registry-Eintrag nötig |

Jeder Registry-Eintrag braucht vom Owner/Mod-Team: **Grund** (Warum gesperrt?) und **Review-Datum**. Die Gründe sind aus dem Snapshot nicht rekonstruierbar → Pflicht-Input fürs Review.

### 4.2 Funktionale User-Overwrites → ersetzen oder löschen (keine Bans)
| User-ID | Wo | Soll |
|---|---|---|
| 557628352828014614 (TicketTool-Bot) | Support-Kategorie, ticket-eröffnen, tickets, transcipts | ersetzen durch managed Rolle `Ticket Tool` (3.10); transcipts stirbt mit „Alt" |
| 1355078189894078597 (Bot-Account, vermutl. Deadlock Master) | 4× faq-\*, ticket-17 | entfällt (faq-Kanäle archiviert; Bot-Rollen haben ADMIN) |
| 246716116498513920, 247139694398144513, 936257113720758402, 698246721003585566 (faq-User) | je 1 faq-Kanal | entfällt mit Archivierung |
| 1319615961523032074, 198564352914096128 (Ticket-/Feedback-Teilnehmer) | ticket-17/0057, erfahrungsberichte | Tickets = Namespace D; erfahrungsberichte-`+SEND` → Rolle `Coaching Feedback` nutzen |
| 193685907071696896, 503957305164038156 (Team-Kapitäne) | leo-team, deniz-team, Team-4 | Coaching-Projekt: durch Team-Rollen ersetzen |
| 271549384787755008, 702594328328929331 | moderator-only `+VIEW(+SEND)` | ⚑ 6.8: Rolle geben oder Overwrite entfernen |
| 335030285240631296 | bot-logs `+VIEW` | ⚑ 6.8: klären (Bot-Account? Dev?) |
| 318453395335675904 | coach-chat `+VIEW` | Coaching-Projekt: Rolle statt User-Overwrite |
| 279971744964542464 (2× leer), 886533600189755392, 1207015550409121855 (leer, closed-Tickets) | Neue Spieler Lanes, closed-0058/0059 | **löschen** (funktionslose Zombie-Artefakte, von Prinzip 6-Ausnahme gedeckt) |

---

## 5. Abweichungs-Liste Ist → Soll

1. **59 leere Overwrites** (allow=0, deny=0; v. a. @everyone/Booster/CM-Reste) → löschen, keinerlei Wirkung.
2. **30 Owner-/Moderator-Overwrites** → löschen: beide Rollen tragen ADMINISTRATOR, jedes Overwrite ist wirkungslos.
3. **23 Deadlocker-Overwrites** → entfallen mit Rollen-Abriss (§4.7). Kuriositäten im Ist: invertierte Gates — Custom Game Team 1/2 `Deadlocker: −CONNECT` bei `@everyone: +CONNECT` (Deadlocker durften NICHT connecten), clip-submission `Deadlocker: −VIEW` ⚑ 6.11.
4. **16 English-Only-Overwrites** → entfallen: bei offener Basis redundant; Rolle bleibt als Label (§4.7).
5. **12 Funny/Grind-Custom-Ping-Overwrites** → entfallen: Ping-Rollen sind Präferenz-Signale (§2).
6. **16 VC-Move-Rechte-Overwrites** → auf 1 reduziert (`+CONNECT` Competitiv): Rolle hat MOVE guild-weit.
7. **54 Overwrites auf Archiv-Kandidaten** („Alt" komplett, faq-\*, Beta Zugang, server-faq) → entfallen mit Archivierung (23 Objekte).
8. **40-Bit-Voll-Masken** (TicketTool, Coaching-Ausschlüsse, VIP/Booster/Unterstützer/NSP-Sets, Eingangsbereich-27-Bit-Deny) → äquivalente Minimal-Sets; Verhalten identisch, Modell lesbar.
9. **Eingangsbereich**: read-only künftig über `−SEND`+Thread-Denies statt 13-Allow/27-Deny-Maske.
10. **Beta Zugang** (`−VIEW` + Gate-Rolle) → deadlock-invite wird P0-öffentlich (§4.4); Gate-Rolle funktionslos.
11. **rang-auswahl** (Chat, CUSTOM-Wildwuchs) → deadlock-rang als P2-Panel-Kanal im Eingangsbereich.
12. **Kategorie-Sync als Default**: heute 115/118 Objekte mit Overwrites, viele CUSTOM ohne Grund (z. B. Sonstiges-Kanäle mit identischen Kopien) → im Soll erben Kanäle; CUSTOM nur deklariert.
13. **User-Bans** (85 der 115 User-Overwrites, 5 User) → Ausnahme-Registry, konsolidiert auf Kategorie-Ebene (Abschnitt 4.1).
14. **Coaching** bleibt strukturell wie heute (konserviert), verliert aber Zombies/Redundanz/Voll-Masken (134 → ≈90; Ziel ≈25 nach eigenem Projekt).
15. **Rollen ohne Rechte-Funktion im Soll**: Deadlocker, Non Streamer Partner, Streamer Onboading, Beta Zugang benötigt, 16 „LIVE PING"-Rollen (Konzept nennt 31 Streamer-Ping-Rollen — Zählung gegen Live-Rollenliste beim Abriss verifizieren, Cross-Repo-Reihenfolge §4.7 zwingend), Funny/Grind Custom Ping (bleiben als Ping-Label, verlieren Overwrites). `Lurker` bleibt (aktives TempVoice-Feature, ⚑ Konzept §8).

---

## 6. Offene Owner-Entscheidungen

| # | Frage | Vorschlag |
|---|---|---|
| 6.1 | @everyone-Basis: TTS entfernen? CREATE_PRIVATE_THREADS entfernen (private Threads sterben, §4.1)? USE_APPLICATION_COMMANDS behalten (Prinzip 5 verbietet nur EIGENE User-Slash-Commands, nicht externe Apps)? | ja / ja / behalten |
| 6.2 | server-support: öffentlich schreibbar (Support-Anliegen im Klartext) oder read-only (nur Ticket-Verweis)? | schreibbar |
| 6.3 | ankündigungen: senden nur CM (+ADMIN-Rollen) — oder weiterhin auch Turnier Moderation/Coach/Turnier Caster inkl. @everyone-Mention? | nur CM; Turnier-Ankündigungen über Bot |
| 6.4 | stream-updates: nach Ping-Rollen-Abriss öffentlicher Live-Feed in Medien (P1) oder Streamer-intern behalten? | öffentlich (Ziel des „Streams"-Pings) |
| 6.5 | rank-ups: öffentlich beschreibbar (Gratulieren) oder Bot-Feed read-only? | öffentlich lassen |
| 6.6 | Rolle `Beta Zugang benötigt`: mit offener Invite-Lounge abreißen (entrechten, löschen nach Frist)? | ja, in Welle 2c |
| 6.7 | Coach-MOVE: pro Lane-Kategorie (3 Overwrites, Vorschlag) oder guild-weit (0 Overwrites, aber Move überall)? | pro Kategorie |
| 6.8 | Einzel-User-Allows: moderator-only (271549384787755008, 702594328328929331), bot-logs (335030285240631296) — Rolle geben, behalten als Registry-Ausnahme oder entfernen? | Rolle geben oder raus |
| 6.9 | Ban-Konsolidierung auf Kategorie-Ebene: X1/X2 verlieren dann die Sicht auf ganze Kategorien statt Einzelkanäle (leicht strenger als heute). OK? Alternativ 1:1-Migration der Einzelkanal-Denies. | Kategorie-Ebene |
| 6.10 | X4: wirkungslose `−DEAFEN`-Einträge löschen? | ja |
| 6.11 | Custom Game Team 1/2 + Caster Channel: heutige invertierte/vollmaskierte Gates gewollt? Soll: Team-VCs offen, nur Caster-VC gegated. | wie Soll |
| 6.12 | Streamer-Bereich: behalten VIP/Server Booster/Server Unterstützer Einblick (heute ja, mit Spezial-Masken)? | nein — nur Streamer + Streamer VC Zugriff |
| 6.13 | AutoMod-Whitelist „sicherheitskritische Rechte" für Auto-Revert (§5.1) — Vorschlag: ADMINISTRATOR, MANAGE_GUILD, MANAGE_ROLES, MANAGE_CHANNELS, MANAGE_WEBHOOKS, MENTION_EVERYONE, BAN/KICK auf Nicht-Mod-Rollen | bestätigen |
| 6.14 | `Moderator` trägt ADMINISTRATOR (neben Owner). Bewusst? (Kein Rechte-Problem im Modell, aber jede ADMIN-Rolle ist Drift-blind — Overwrites greifen nicht.) | bestätigen/prüfen |
| 6.15 | Gründe + Review-Daten für die 4 Registry-Ausnahmen X1-X4 nachliefern (Pflichtfeld im DB-Modell). | Owner/Mod-Team |

---

## 7. Statistik Ist vs. Soll

**Ist (Snapshot 2026-07-01):**
- 118 Objekte (18 Kategorien, 73 Text, 24 Voice, 2 News, 1 Forum), nur 3 ohne Overwrites
- **519 Overwrites gesamt**: 404 Rollen-, 115 User-Overwrites
- davon: 59 leer (Zombies), 30 Owner/Mod-redundant (ADMIN), 23 Deadlocker, 16 English Only, 12 Custom-Ping, 54 auf Archiv-Kandidaten, 134 im Coaching-Bereich, 85 User-Ban-Einträge (5 User)
- 36 verschiedene Rollen + 20 User in Overwrites referenziert

**Soll (deklarierte Regeln):**
- **≈32 statische Regeln** (alle Bereiche außer Coaching) + **≈8 Registry-Regeln** (4 Ban-User, Kategorie-Ebene) + **Coaching-Konserve ≈90** (Ziel ≈25 nach Coaching-Projekt) + dynamische Namespaces (systemverwaltet, ungezählt)
- ergibt nach Welle 2a-2c: **≈130 deklarierte Regeln statt 519 roher Overwrites (−75 %)**; nach Coaching-Sanierung + Phase 5 (Competitiv-Gate weg): **≈65 (−87 %)**
- Drift-Ziel (Phase-2-DoD): Rechte-Drift = 0 gegen dieses Modell; Abweichung → Cockpit-Meldung, Auto-Revert nur Whitelist 6.13
