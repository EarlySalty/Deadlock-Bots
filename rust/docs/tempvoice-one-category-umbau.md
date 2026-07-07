# TempVoice-Umbau: Eine Kategorie, offenes Ranked, Owner-Rang-Gate

Stand: 2026-07-07 · Status: Spec abgenommen, Umsetzung beauftragt

## Ziel & Motivation

Ranked-Voice wirkt heute ausladend: Die Ranked-Kategorie verweigert `@everyone`
CONNECT, dadurch sehen alle Unverifizierten an jeder Ranked-Lane ein 🔒-Schloss.
Umbau: **Alle Lanes (Ranked, Casual, Street Brawl) landen in einer offenen
Kategorie**, Unterscheidung nur noch über Name + Sortierung. Ein Schloss gibt es
nur noch an einzelnen Lanes, deren Owner bewusst ein Rang-Gate aktiviert.
Wir wollen messen, ob das die Ranked-Nutzung erhöht.

## Ist-Zustand (Referenzen)

- Router: `rust/crates/dl-voice/src/router.rs` — 3 Modi mit je eigener
  Kategorie + Staging (`ROUTER_MODES`), Ranked-Gate `ranked_allowed`
  (Verifiziert-Pflicht vor Lane-Erstellung).
- Kategorien heute: casual `1289721245281292290` (offen), ranked
  `1412804540994162789` (`@everyone` deny CONNECT), street_brawl
  `1357422957017698478`.
- Sortierung: zwei getrennte Sortierer in `adaptive.rs`
  (`sort_ranked_category`, `sort_chill_lanes`).
- Rang-Motor: `rank.rs` `RankVoiceManager` — setzt Connect-Overwrites nach
  Score-Fenster, Prinzip „niemals kicken, nur Rechte".
- Panel: `tempvoice/interface.rs` — u. a. `tv_rank_pref` (Panel-Rang,
  `voice.tempvoice_rank_pref`), versteckte Selects `min_rank_select` +
  `tv_subrank_perm`.

## Entscheidungen

### 1. Eine Kategorie

- Alle drei Modi erstellen Lanes in **`1289721245281292290`** (Chill, offen).
- Router-Buttons bleiben unverändert; sie bestimmen weiterhin Name, Limit und
  Gate-Anker — nur nicht mehr die Ziel-Kategorie. Staging→Kategorie-Map
  entsprechend umbiegen.
- **Verifikationspflicht für Ranked entfällt komplett** (`ranked_allowed`,
  `RankedVerifyRequired`-Pfade): jeder darf Ranked-Lanes erstellen und joinen,
  solange kein Gate aktiv ist.
- Alte Ranked-/Street-Brawl-Kategorien bleiben leer stehen (Löschen ist eine
  spätere manuelle Entscheidung, nicht Teil dieses Umbaus).
- Bestehende Live-Lanes werden **nicht** umgezogen; Temp-Lanes sterben leer,
  neue entstehen in der neuen Kategorie.

### 2. Namen

- **Anker-Rang** einer Lane: Panel-Rang-Präferenz (`tempvoice_rank_pref`) des
  Erstellers; falls nicht gesetzt, aus verifizierter Rang-Rolle initialisiert;
  falls beides fehlt, kein Rang.
- Ranked: `Ranked <Rang> <Subrang>` aus dem Anker (z. B. „Ranked Emissary 3");
  ohne Anker schlicht `Ranked`. Index-Suffix zur Deduplizierung wie bisher.
- Casual: `Chill Lane N · <Rang>` — Rang am Ende, rein informativ; ohne Anker
  `Chill Lane N` wie bisher.
- Street Brawl: unverändert `Street Brawl N`.
- Achtung: Die Rang-Sortierung parst den Rang aus dem Lane-Namen — das
  Namensschema ist API, nicht Deko. Rename-Flows dürfen den Rang-Teil nicht
  zerstören bzw. der Sortierer muss damit robust umgehen.

### 3. Sortierung

- **Ein** Sortierer für die gesamte Kategorie (Merge von
  `sort_ranked_category` + `sort_chill_lanes`):
  - Block 1: Ranked-Lanes, nach Rang **aufsteigend** (klein → groß).
  - Block 2: Casual-Lanes.
  - Block 3: Street-Brawl-Lanes.
  - Innerhalb der Blöcke stabile Reihenfolge wie bisher.
- Positions-Updates **gebündelt** über den Bulk-Endpoint
  (`PATCH /guilds/{gid}/channels` mit Positions-Array), nicht einzeln pro
  Channel — Rate-Limit.

### 4. Rang-Gate (neuer Kern)

- Neuer Panel-Button `🔓 Rang-Gate`, nur bei Ranked-Lanes, nur Owner.
- Klick → **ephemeres Embed** (nur Owner sichtbar) mit:
  - Erklärung, was das Gate bedeutet und welche Folgen es hat (Schloss für
    alle außerhalb des Fensters; nur verifizierte Rang-Rollen zählen; niemand
    der schon drin ist wird entfernt).
  - Zwei Dropdowns: **Mindestrang** + **Subrang-Toleranz**, vorbefüllt mit
    Anker **±1,5 Ränge** (= ±9 Subränge; 1 Rang = 6 Subränge).
  - Bestätigen-Button. Gate wird erst nach Bestätigung aktiv.
- Motor: bestehender `RankVoiceManager` (Score-Fenster-Logik
  weiterverwenden, nicht neu bauen).
- **Batch-Apply:** Overwrites in **einem** Channel-PATCH mit komplettem
  `permission_overwrites`-Array setzen — keine Einzel-PUTs pro Rolle
  (Rate-Limit). Gate aus → ein PATCH, der die Gate-Overwrites entfernt.
- Gate ist ein Toggle: nochmal drücken → aus, Lane wieder offen.
- **Kein Kick, nie:** Gate wirkt nur auf neue Joins. Einziger Disconnect
  bleibt Owner-Kick/-Ban.
- Die alten Dauer-Selects `min_rank_select`/`tv_subrank_perm` verschwinden
  aus dem Panel-UI; ihr Backend bleibt und wird vom Gate-Dialog genutzt.

### 5. Ankündigung + Anleitung

- Ankündigung als **Discord Components V2**-Nachricht (Gold-Branding), die
  den Umbau 1:1 erklärt.
- Darin ein Button `📖 Anleitung` (custom_id, kein Link): ephemere Antwort
  mit der Schritt-für-Schritt-Anleitung (Lane erstellen, Panel-Rang setzen,
  Gate aktivieren/deaktivieren).
- User-sichtbare Texte werden von Claude final geschrieben; Implementierung
  nutzt Platzhalter mit Datei/Zeile-Hinweis.

### 6. Bewusste Grenzen

- Discord-Limit 50 Channels pro Kategorie: bekannt, keine Overflow-Lösung in
  diesem Umbau (erst wenn Zahlen es erzwingen).
- Mindestrang-Fenster verliert seine permanente UI; Code bleibt als
  Gate-Motor aktiv.

## Tests (Pflicht, TDD)

- Sortier-Reihenfolge: gemischte Kategorie → Ranked aufsteigend, dann Casual,
  dann Street Brawl; stabile Ordnung innerhalb der Blöcke.
- Namensbau: Anker-Präzedenz (Panel-Pref > Rolle > leer) für alle drei Modi.
- Fenster-Mapping: Anker ±1,5 Ränge → korrektes Subrang-/Score-Fenster.
- Gate-Toggle: an → Overwrites gesetzt (ein Apply), aus → entfernt; kein
  Disconnect vorhandener Member.
- Batch-Verhalten: Apply erzeugt genau einen Channel-PATCH bzw. einen
  Bulk-Positions-Call.

## Abschluss-Checkliste

1. Build + Tests + Clippy grün.
2. `CHANGELOG.md` (user-sichtbar) ergänzen.
3. Merge auf main, Deploy, Restart, Live-Beweis (PID, /proc/exe, Journal).
4. Ankündigung mit finalen Texten in Discord posten.
