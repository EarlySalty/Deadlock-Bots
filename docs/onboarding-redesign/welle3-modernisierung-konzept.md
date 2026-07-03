# Welle 3 — Server-Modernisierung (Konzept)

Stand: 2026-07-03, Grillme-Session mit Owner. Welle 2b (natives Onboarding, Regelwerk, Archiv) ist komplett live — dieses Dokument definiert den nächsten Ausbau.

## Ziel & Leitsätze

1. Den Discord-Umbau sauber abschließen: moderner, professioneller Server ohne AI-Slop-Optik.
2. Inspiration von großen Servern (Marvel Rivals, Genshin, Minecraft) — **umgebaut auf uns, nie 1:1 kopiert**.
3. Leitsatz Owner: „Professioneller = kleinere Hürde." Qualität rechtfertigt Reibung (z.B. Steam-Gate).
4. Alle sichtbaren Grafiken/Texte im Brand-Look der Websites (dunkles Anthrazit, Gold/Creme, Edel-Holz-Ästhetik); Text in Bildern nie KI-generiert.

## Bausteine & Reihenfolge

| # | Baustein | Inhalt | Status |
|---|----------|--------|--------|
| A | Struktur + Welcome-Hub | Soll-Modell v2 (Kategorien/Naming), `🧭willkommen`-Hub, Banner-Pipeline | zuerst |
| B | Linked Role „Steam verknüpft" | Nativer Verknüpfungs-Screen im Onboarding, Rang-Automatik, Vertrauens-Gate | nach A |
| C | LFG v2 | Forum mit Formular, Auto-Expiry, Router-Integration, Leaderboard öffentlich | nach A+B |
| D | Router-Auswahl-Flow | Geführter Voice-Flow ersetzt ➕-Creator; Button-Panel für unser TempVoice | mit C verschmolzen |

## A — Soll-Modell v2 (öffentlicher Teil)

| Kategorie | Kanäle |
|---|---|
| `🏛️ ─ INFORMATION ─` | 📜regelwerk · 🧭willkommen (neu, Nav-Hub, read-only) · 🔗deadlock-rang · 🎫server-support · 💌deadlock-invite |
| `📣 ─ NEUIGKEITEN ─` | 📢ankündigungen · 📝patchnotes · 🐛dev-updates · 🎥stream-updates |
| `💬 ─ COMMUNITY ─` | 🌐allgemein · 🎲off-topic · 💬frag-die-community · 💀memes · 👀leaks · 🎨kreativ-ecke · 📼gameplay-clips · 📺yt-videos · ❓feedback · 🤖bot-spam · 😡rage-room (ex haatteee) · nsfw (bleibt, nicht in Navigation) |
| `🎮 ─ DEADLOCK ─` | 🎯mitspieler-suche (wird LFG-Forum, Baustein C) · 📖guides-und-tipps · 🧩custom-games · 🏆rank-ups |
| `🎓 ─ COACHING ─` | eingedampft: Team-Textkanäle → Threads unter 🗓️scrim-planung; Alt-Kanäle (leo-team, deniz-team, team-1..4) **vorerst behalten**, Praxistest, Review später |
| Voice-Kategorien | Modus-Kategorien (Chill / Ranked / Street Brawl / Custom Game / Neue Spieler) **bleiben als Spawn-Ziele**; die fünf ➕-Creator werden durch den Router-Flow ersetzt (Baustein D); die drei `🚧sprach-kanal-verwalten` → ein Panel-Kanal |
| *(nicht öffentlich)* | Moderation · Streamer · VIP · `🎟️ ─ SUPPORT ─` (ex ❓Support, geschlossene Tickets auto-archivieren) · 📦 Archiv |

Naming-Konvention: jeder Kanal genau **ein** Emoji-Präfix, Kategorien als Trenner in Caps, alles Deutsch. Finale Emoji-Palette passend zur Gold-Linie wird als Liste vorgelegt. Leere Kategorien (Beta Zugang, Alt, Neue Spieler Lanes) werden aufgelöst. Rollout deklarativ über serversync; dabei Soll-Modell-Review der 68 vom Effektiv-Rechte-Guard geblockten Öffnungen (Incident-Rest Welle 2a) miterledigen.

## A — Welcome-Hub `🧭willkommen`

Embed-Serie (Vorbild Rivals `#welcome`, published + gepflegt via serversync analog regelwerk-publish):

1. Hero-Banner + 3-Zeilen-Intro (kein Marketing-Sprech)
2. Kanal-Navigation: pro Kategorie ein Embed, klickbare Kanal-Mentions + Ein-Satz-Beschreibung (nur öffentliche Kanäle)
3. Community-Team: Rollen-Hierarchie mit Namen, **bot-generiert aus Live-Rollen** (nie stale)
4. Links & Socials: Website, Twitch, Coaching, Invite als Button-Zeile
5. Schnellstart-Buttons: Regelwerk lesen / Rang verknüpfen / Support

### Banner-Pipeline

Scripted, kein Midjourney: SVG-Templates im Website-Look (Anthrazit, Grid-Textur, Creme/Gold-Condensed-Headlines, dünne Gold-Linien, Logo aus dl-brand) → PNG-Render (~1100×300). Neue Banner = Textzeile ändern; Serie bleibt pixel-konsistent.

## B — Linked Role „Steam verknüpft"

- Mechanik: Role-Connection-Metadata der Bot-App (`steam_verknuepft`, `rang`), OAuth über die Website (Discord-Login + Steam-Link existieren dort). Kein Verified-Server nötig; Screen erscheint nativ im Onboarding, überspringbar.
- **Stufe 1 — Rang-Automatik:** Verknüpfte bekommen die Rang-Rolle automatisch aus echten Daten (Reconcile hält aktuell). Rang-Panel bleibt Fallback für Unverknüpfte.
- **Stufe 2 — Vertrauens-Gate:** Posten in Mitspieler-Suche/LFG-Forum nur mit Verknüpfung (Anti-Scam: Hürde für Wegwerf-Accounts). Auch für Coaching-Zuordnung genutzt (Coach sieht echten Rang; Website-Anfragen ↔ Discord-Account).
- **Stufe 3 — kein Lese-/Chat-Gate:** Server bleibt ohne Verknüpfung voll nutzbar.

## C — LFG v2

- `🎯mitspieler-suche` wird Forum: Posten **nur per Formular** (Modus, Rang-Bereich, freie Plätze, Mikro) → sauber strukturierter Post. **Keine Tag-Pillen** (Owner-Entscheid; Region entfällt, DE-Community).
- Auto-Expiry: Posts laufen nach 24 h Inaktivität ab.
- Router-Integration: „Lane erstellen"-Button am Post spawnt die passende Voice-Lane **in der zum Modus passenden Kategorie** und verlinkt sie im Post.
- Posten nur mit Steam-Verknüpfung (B Stufe 2); lesen/antworten für alle.
- **Kein Punkte-/Season-System** (bewusst gegen Rivals-Vorbild: Engagement-Theater bei unserer Größe). Stattdessen: bestehendes Rank-Leaderboard (eigene Berechnungen, Sichtbarkeits-Opt-in) öffentlich machen. Optionales Nice-to-have später: „zuverlässiger Mitspieler"-Badge nach N erfolgreichen LFG-Runden — ohne Punktezähler.

## D — Router-Auswahl-Flow (mit C verschmolzen)

Der Deadlock Router ersetzt alle fünf ➕-Creator: geführter Auswahl-Flow (Modus wählen → Lane spawnt in Modus-Kategorie), Vorbild Rivals-LFG-Buttons. Verwaltung der eigenen Lane über **ein** Button-Panel (unser TempVoice, kein Fremd-Bot-Nachbau); ersetzt die drei `🚧sprach-kanal-verwalten`-Kanäle.

## Server Theme (Client-Einfärbung)

Discord-Feature „Server Themes" (Experiment-Rollout): Admin setzt Farbthema, Client wird für Mitglieder eingefärbt (individuell abschaltbar). Gate: **3 Server-Boosts** (Level 1), kein Verified nötig. Geplant: Gold/Edel-Palette aus dl-brand. → Owner prüft Boost-Stand + ob der Menüpunkt (Servereinstellungen → Server Theme) schon ausgerollt ist.

## Offene Owner-Punkte

1. Boost-Level + Server-Theme-Verfügbarkeit prüfen (s.o.).
2. Server Guide manuell umbauen (Bot-PUT ist user-only, 403) — **erst nach** Struktur-Rollout Welle 3, sonst doppelte Arbeit; dabei alte ❓server-faq-Action entfernen (löst den letzten geskippten 350003-Change).
3. TWITCH_INTERNAL_API_TOKEN rotieren (pgrep-Leak, offen aus Welle 2b).

## Umsetzungs-Phasen (DAG)

1. **W3.1 Struktur:** Soll-Modell v2 in serversync-Config, Emoji-Palette, Rechte-Review (68 Blocks), Rollout + Live-Beweis.
2. **W3.2 Hub:** Banner-Pipeline, Welcome-Publisher (Navigation/Team/Socials/Buttons), Rollout. Abhängig von W3.1.
3. **W3.3 Linked Role:** Metadata-Registrierung, Website-OAuth-Route, Rang-Sync, Onboarding-Einbindung, Gate auf Mitspieler-Suche. Parallel zu W3.2 möglich.
4. **W3.4 LFG + Router:** Forum-Umbau, Formular-Flow, Auto-Expiry, Router-Auswahl-Flow + Panel, Leaderboard-Veröffentlichung. Abhängig von W3.1–W3.3.

Jede Phase: Codex implementiert → Kritiker → Rework → Merge auf main → Deploy → Live-Beweis → CHANGELOG/Discord (user-sichtbare Texte schreibt Claude final).
