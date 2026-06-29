# Coaching-Etage: Scrim-Programm + Rust-Port — Design

- **Datum:** 2026-06-29
- **Status:** Entwurf (zur Freigabe)
- **Repos:** `Deadlock-Bots` (Rust, Quelle der Wahrheit) · `Website` (React-Frontend `dl-coaching`, bleibt)
- **Quelle der Anforderungen:** grillme-Session + Discord `#scrim-team-channel` (1289721245281292288 / 1520842755037855975), Roster-Sheets, Coach-Profil-Screens.

---

## 1. Problem & wahres Ziel

Deniz & Leo organisieren **Scrims** (6v6-Übungsmatches) für die DACH-Deadlock-Community: Spieler posten formlos Rang/Rolle/Zeit im Discord, die zwei tippen das **von Hand** in ein Google-Sheet, bauen daraus balancierte Teams mit überlappenden Zeitplänen und pflegen alles nach. Schmerzpunkte (aus dem Chat belegt):

1. Manuelles Abtippen formloser Posts ("so abends", "flexibel").
2. Balancing nach **Rang vs. Zeit** ist von Hand "teilweise unmöglich".
3. Geteilte editierbare Tabelle = unsicher.
4. Nachzügler/Absagen/❌-Reaktionen laufend nachpflegen.

**Wahres Ziel:** nicht "die Excel digitalisieren", sondern **die Arbeit der Coaches abnehmen** (Daten sammeln + Matching) und **Spielern Sichtbarkeit geben** (in welchem Team bin ich, wann ist mein Match). Excel-Abbild ist nur ein Output davon, nicht das Produkt.

**Nicht-Ziele:** Discord als Kommunikations-Hub ersetzen (Team-Chats/Pings/Streaming bleiben Discord). Reiner Wettkampf — Zweck ist **voneinander Lernen**.

---

## 2. Ist-Zustand (verifiziert am 2026-06-29)

- **Coaching-Plattform = Python**, live: `deadlock-website-backend.service`, FastAPI + SQLite, `127.0.0.1:8772`, `builds/backend/app/routers/coaching.py` (+`coaching_platform.py`, heute geändert). **Kein Rust im Website-Repo.**
- **Discord-Brücke = Rust:** `dl-community/src/coaching.rs` ist laut eigenem Header *"Coaching-Plattform-Brücke — Port von `coaching_platform_sync.py`"*. Sie ist ein **Client** der Python-API: Rollen-Sync (alle 10 min, `POST /coaching/platform/coaches/sync`) + Notification-Poller (60 s, `GET /notifications/due` → `POST /notifications/ack`).
- **`dl-web` (Rust)** bedient bereits Dashboard/Stats/Tierlist (8766/8768/8771) im selben Workspace `Deadlock-Bots/rust` wie der **dl-bot** (Gateway + Broker 8770). `dl-web` kennt **kein** Coaching.
- **Reaction-Roles** existieren bereits in Rust (`dl-community/src/reaction_roles.rs`).
- **Discord-Rollen + Team-Channels** für die aktuellen 4 Teams sind bereits manuell angelegt.

**Folgerung:** Das Coaching-"Gehirn" ist Python, nur der "Bote" ist Rust. Der Port ist real ausstehend — und er **löscht** die Brücke, statt Code draufzusetzen.

---

## 3. Architektur-Entscheidung

**Quelle der Wahrheit → Rust, im Workspace `Deadlock-Bots/rust`.** React-Frontend (`Website/dl-coaching`) bleibt und zeigt auf die neue Rust-API.

Neue/erweiterte Bausteine (Namen final, 2026-06-29 bestätigt):

- **`dl-etage`** (neuer Rust-Dienst/Bin, **eigener Port**): HTTP-API der Coaching-Etage — bedient `/coaching/*`, ersetzt Python `:8772`. Bewusst **eigener Dienst** (nicht in `dl-web`): isolierter Blast-Radius + unabhängiges Deploy-Tempo (Coaching iteriert schnell, Dashboard/Stats sind stabil).
- **`dl-mentoring`** (neue Crate): 1:1-Coaching (Coach↔Coachee, Ziele, Milestones, Notizen, Termine), portiert aus Python.
- **`dl-squads`** (neue Crate): das Scrim-Programm (Pool → Teams → Matches → Aufgaben → Matching).
- **Auth:** Discord-OAuth/Session zieht mit nach Rust (in/neben `dl-etage`), weil die Coaching-Seite sie braucht und `:8772` ganz weg soll.
- **dl-bot:** Reaction-Watcher schreibt Pool-Einträge **in dieselbe DB** (`dl-db`); Team→Rolle/Channel-Anlage direkt. Bot-Integration bleibt **in-process** (geteilte `dl-db`), unabhängig vom Dienst-Schnitt.

> Frontend bleibt `Website/dl-coaching` (die Etage als App) — kollidiert nicht mehr, da die Backend-Crates domänenklar heißen (`dl-mentoring`/`dl-squads`).

**Bridge-Collapse:** Sobald die Plattform in Rust neben dem dl-bot liegt, werden Rollen-Sync und Notification-Zustellung zu **In-Process-Aufrufen auf einer DB** — die beiden HTTP-Polling-Loops + die token-vs-i64-ID-Bugklasse (Memory `project_broker_snowflake_id_type` / `project_coaching_request_discord_mirror`) entfallen.

---

## 4. Datenmodell — Scrim (`dl-scrim`)

- **`scrim_participant`** (der Pool): `discord_id`, `display_name`, `rank`, **`rank_source`** (`self` | `steam`), **`rank_verified`** (bool), `roles` (Liste), `heroes` (Liste/Text), `availability` (Wochen-Raster, strukturiert), **`status`** (`new` | `profile_complete` | `admitted` | `assigned` | `bench` | `waitlist` | `left`), `created_at`, `source` (`discord_reaction` | `web_form`).
- **`scrim_team`**: `name`, **`coach_id`** (Leo/Deniz), `discord_role_id`, `discord_channel_id` (bestehende beim Import verlinken), `created_at`.
- **`scrim_team_member`**: `team_id`, `participant_id`, `role`, `is_captain`, `is_bench`.
- **`scrim_match`**: `team_a_id`, `team_b_id`, `scheduled_at`, `status`. (Anzeige Slice 1, Verwaltung Slice 2.)
- **`scrim_task`**: `team_id`, `text`, `created_by`, `ai_suggested` (bool), `done` (bool). (Slice 2.)

**Nähte (jetzt nur Felder, kein Bau):**
- Steam-Rang: `rank_source`/`rank_verified` machen den späteren 2. Steam-Bot anschlussfähig **ohne Schema-Änderung**.
- Stripe/kostenpflichtig: `status=waitlist` ist heute die "wir testen erst selbst"-Bremse; ein späteres Bezahl-Gate hängt sich an den Aufnahme-Übergang. **Jetzt kostenlos, kein Zahlungs-Plumbing.**

---

## 5. Eingänge — EIN Pool, zwei Türen

- **Discord-Reaktion** auf die bestehende Rollen-Nachricht (der "Haken", der heute schon die Rolle gibt) → dl-bot legt automatisch `scrim_participant` an (`status=new`, `source=discord_reaction`, Profil leer). **Kein Abtippen mehr.**
- **Web-Formular** (von Deniz/Leo angekündigt, primär) → derselbe Pool mit vollem strukturiertem Profil (`source=web_form`).
- Coaches sehen **eine Liste** mit Status-Filter. "Warteliste" = `status=waitlist`, einzeln auf `admitted` schalten.

---

## 6. AI-Scope

**Prinzip: Matching ist deterministische Mathematik, kein LLM-Job.** Die KI bereitet vor und erklärt, sie entscheidet nicht.

- **Deterministische Matching-Engine (Rust, `dl-scrim`):** Zeit-Überlappung (Mengenschnitt der Verfügbarkeiten) + Rang-Balance (Verteilung) — instant, gratis bei jeder Roster-Änderung. Liefert dem Coach kandidatische Team-Sets mit harten Kennzahlen ("diese 6: gemeinsames Fenster Di+Do ab 19, Rang-Spanne X").
- **MiniMax (nur Fuzzy-Teile, Slice 2):** (a) einen Roster-Vorschlag in Worten **begründen**, (b) **Team-Aufgaben/Drills** aus Team-Schwächen vorschlagen (Coach editiert/bestätigt). Modellwahl hinter **einer** Konfig-Stelle (austauschbar).
- **Warnung:** MiniMax fällt bei euch mit kaputten Umlauten/schwachem Deutsch auf. Interne Coach-Vorschläge = unkritisch; **user-sichtbare** KI-Texte (z. B. an Spieler gezeigte Aufgaben) prüft Claude / ggf. Modellwechsel.

---

## 7. 1:1-Coaching-Port (`dl-mentoring`)

**Parität-Migration: 1:1, keine Funktionsänderungen** — bis auf zwei bewusst gewollte Anpassungen:

1. **AI-Analyse raus:** Felder/Logik `ai_summary` + `ai_insights_json` (an Coachees, `coaching.py`) werden **nicht** mitportiert.
2. **Coaching-Rolle 1 Woche halten:** Aktuell verlieren Teilnehmer die Coaching-Rolle zu früh, wenn der Termin erst in ein paar Tagen ist. Rollen-Haltedauer (im dl-bot) auf **1 Woche** ziehen, am Termin ausgerichtet. (Python-Plattform hat nur einen 7-Tage-`cutoff` bei Appointments; die Entzugs-Logik sitzt im Bot.)

**Port-Umfang (Endpunkt-Inventar, Parität):**
- `coaching.py`: `/coaches`, `/coaches/{id}`, `/coaches/{id}/reviews`, `/coaches/profile`, `/coaches/apply`, `/requests` (POST/GET), `/requests/{id}/match`, `/surveys`, `/dashboard`, `/sessions/{id}/end`, `/admin/applications/{id}`.
- `coaching_platform.py`: `/platform/sync`, `/overview`, `/queue`, `/coachees` (+Detail/Patch), `/goals` (+Milestones), `/notes`, `/me`, `/coaches/sync`, `/coaches/me`, `/appointments` (POST/GET/Patch), `/notifications/due`, `/notifications/ack`.
- `/notifications/*` + `/coaches/sync` werden nach dem Port **in-process** statt HTTP (Brücke entfällt).

**Voller `:8772`-Abbau (bestätigt):** Damit der Python-Dienst ganz weg kann, ziehen **Auth (Discord-OAuth/Session)** und die noch genutzten **Meta-Endpoints** (builds/items/heroes/patchnotes/tierlists) mit nach Rust bzw. auf `dl-web`. Auth gehört in Slice 1 (Coaching braucht Login); Meta-Abräumung ist ein eigener Slice (siehe §8), vorher wird geprüft, welche Frontends `:8772` sonst noch anrufen.

---

## 8. Phasen / Slices (mit Definition of Done)

**Slice 0 — Auto-Sammeln + Import (sofort, rein Rust, kein Frontend):**
- dl-bot: Rollen-Reaktion → zusätzlich `scrim_participant`-Upsert.
- Einmaliger Import des Ist-Stands: ~25 Spieler + 4 Teams + Bench + bestehende Discord-Rollen-/Channel-IDs.
- **DoD:** Neue Reaktion erzeugt automatisch Pool-Eintrag; Ist-Stand in der DB; nichts wird mehr von Hand abgetippt.

**Slice 1 — Sichtbarkeit + Web-Eingang + Plattform-Port + Auth:**
- `dl-mentoring` portiert (Parität + die zwei Anpassungen) + **Auth (Discord-OAuth/Session)** nach Rust; `dl-etage` bedient `/coaching/*`; Caddy biegt von `:8772` auf den Rust-Port um.
- Frontend: Spieler-Sicht "Mein Team + nächstes Match"; Web-Formular (Pool-Eintrag mit Profil); Coach-Sicht "Pool".
- **DoD:** Coaching-Etage läuft über `dl-etage`, Login funktioniert; Spieler sehen ihr Team; neue Anfragen kommen übers Formular in den Pool; Bridge-Loops entfernt. (Python `:8772` darf noch für Meta weiterlaufen.)

**Slice 2 — Cockpit + KI + RSVP:**
- Deterministische Matching-Engine + Coach-Cockpit (Vorschlag übernehmen/verwerfen); MiniMax-Begründung + Team-Aufgaben; Match-Verwaltung; RSVP statt ❌-Reaktion.
- **DoD:** Coach baut neue Teams im Tool in Minuten; Aufgaben zuweisbar; Spieler bestätigen/absagen Matches.

**Slice 3 — Meta-Abräumung + `:8772` aus:**
- Prüfen, welche Frontends/Pfade `:8772` sonst noch anrufen (builds/items/heroes/patchnotes/tierlists). Was nötig ist, nach Rust portieren oder auf `dl-web` umbiegen.
- **DoD:** `deadlock-website-backend.service` (`:8772`) abgeschaltet; verifiziert nichts kaputt (Dashboard/Landing/Tierlist/Coaching live).

---

## 9. Thema 2 — Coach-Profil-Feinschliff (separater Change, kein grillme)

Unabhängig vom Programm, kleiner Frontend/Backend-Fix:
1. **Bio:** Zeilenumbrüche/Absätze im öffentlichen "ÜBER MICH" erhalten + größeres Editier-Feld.
2. **Schwerpunkte:** Chip-/Tag-Eingabe mit `+`-Box statt komma-getrenntem Textfeld.
3. **Eigene Profil-Seite:** "Mein Coach-Profil" raus aus dem versteckten Cockpit-Akkordeon → eigene Seite, erreichbar per Klick aufs Profilbild oben rechts (Bearbeiten-Modus, Link "So sehen mich Spieler" → bestehende öffentliche Coach-Detailseite).

---

## 10. Umsetzung, Delegation & Verifikation

- **Bau delegiert an Codex** (gpt-5.5, effort xhigh); Claude orchestriert + reviewt (changed_files vor jedem Commit), frischer Codex-Kritiker → Rework-Loop. **User-sichtbare deutsche Texte schreibt Claude**, Codex setzt nur Platzhalter + meldet Datei:Zeile.
- **Verifikation pro Slice:** Artefakt + Live-Zustand beweisen (Binary enthält Änderung, DB/Endpoint/Journal zeigt sie) — nicht dem Erfolgs-Log trauen. Python-Coaching erst nach verifizierter Rust-Parität abschalten.
- **Git:** Feature-Branch `coaching-scrim-program`, schrittweise commit+push, Merge nach `main` erst nach Verifikation. CHANGELOG-Eintrag + Discord-Spiegelung nur für user-sichtbare Teile.

---

## 11. Entscheidungen (2026-06-29 bestätigt)

1. **Namen:** Dienst `dl-etage`, Crates `dl-mentoring` (1:1) + `dl-squads` (Scrim). Frontend bleibt `dl-coaching`.
2. **Eigener Dienst** (nicht in `dl-web`) — wegen Blast-Radius + Deploy-Tempo.
3. **Reaktions-Quelle:** die bestehende Rollen-Nachricht aus dem Reaction-Role-System ist der Discord-Eingang; Channel-/Message-ID + Emoji zieht Claude aus der Reaction-Role-Config.
4. **AI-Analyse raus** = genau `ai_summary`/`ai_insights_json` an den Coachees, sonst nichts.
5. **`:8772` ganz weg**, sobald alles sauber portiert ist (inkl. Auth + Meta) — Reihenfolge: Coaching+Auth (Slice 1) → Meta-Abräumung (Slice 3).
