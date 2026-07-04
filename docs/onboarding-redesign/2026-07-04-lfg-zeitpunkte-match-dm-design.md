# LFG: Zeitpunkte im Gesuch + Match-Benachrichtigung (DM)

**Datum:** 2026-07-04
**Repo:** Deadlock-Bots (`rust/crates/dl-voice`)
**Kontext:** Onboarding-Redesign Welle 3, LFG-Forum „Mitspieler suchen"
**Status:** Design zur Freigabe

---

## 1. Ziel

Zwei Erweiterungen der bestehenden Mitspieler-Suche:

- **Teil A — „Wann willst du spielen?"** Im Gesuch ein grobes Zeitfenster angeben (jetzt / heute Abend / Wochenende / flexibel). Ein Klick, keine Uhrzeit-Tipperei.
- **Teil B — Match-Benachrichtigung.** Wer gerade selbst nichts postet, kann sich per Opt-in vormerken und bekommt **eine DM**, sobald ein passendes Gesuch auftaucht. Selbst-ablaufend, einmalig.

Der Clou: Statt Nutzer exakte Uhrzeiten eintippen zu lassen, kombinieren wir ein **grobes Fenster** mit den **echten Voice-Aktivitätsdaten** (`activity.voice_session_log`, 7,5 Monate, live weiterwachsend) — der Bot weiß aus den Daten, wann jemand typischerweise da ist, und pingt entsprechend zielgenau statt zur Unzeit.

## 2. Ist-Zustand (worauf wir aufbauen)

- **LFG-Panel** (`lfg_panel.rs`): Button „🔎 Mitspieler suchen" → Modus-Wahl → ephemeres Draft mit Rang-Von/Bis-Selects (11 Ränge mit Emojis) + Plätze-Select → Button „Suche veröffentlichen" → Forum-Thread im LFG-Forum. Preset (Rang+Plätze) wird pro User automatisch im kv-Store gespeichert/geladen.
- **Kern-Typen:** `LfgDraft {mode, rank_from, rank_to, slots, lane_id}`, `LfgUserPreset` (Serde, kv-persistiert), `LfgRankRange {min, max}`, `lfg_rank_range_from_draft(draft) -> Option<LfgRankRange>`.
- **Trigger-Punkt:** `handle_post_draft` (Z. 1682) erstellt den Forum-Post via `port.create_forum_post(forum_channel_id, draft)` (Z. 1881) und schreibt Zeilen über `self.pool: PgPool`.
- **Ranked-Gate:** `ranked_allowed(interaction, guild_id, mode)` — Ranked erfordert verifizierten Rang (`VERIFIED_RANK_ROLE_IDS`), liest primär `interaction.role_ids`, fällt auf REST zurück.
- **Voice-Daten:** `activity.voice_session_log` — live vom Rust-`VoiceTracker` (`tracker.rs`, wired `main.rs:1000`) befüllt. Spalten u. a. `user_id, guild_id, started_at (timestamptz), ended_at, duration_seconds, co_player_ids (jsonb)`. 65 025 Sessions, 813 User, Peak-Stunden abends (Berlin).
- **kv-Store:** `dl_central_db::kv::{get,set,delete}(pool, ns, key)`.

## 3. Teil A — Zeitfenster im Gesuch

### 3.1 Datenmodell

`LfgDraft` und `LfgUserPreset` bekommen je ein additives Feld. **Namens-Hinweis:** Dies ist das *Spiel*-Fenster des Gesuchs („wann will ich spielen") — begrifflich getrennt vom *Watch*-Horizont in Teil B 4.3 („wie lange soll ich Ausschau halten"). Zwei verschiedene Enums, damit nichts verwechselt wird.

```rust
play_window: Option<LfgPlayWindow>   // None = nicht gewählt
```

```rust
enum LfgPlayWindow { Jetzt, HeuteAbend, Wochenende, Flexibel }
```

Rückwärtskompatibel: alte kv-Presets ohne Feld deserialisieren mit `None` (Serde `#[serde(default)]`).

### 3.2 UI

Ein zusätzlicher String-Select **„Wann?"** im Draft-Panel (custom_id `lfg:when`), unter Rang und Plätze:

| Option | Label | Emoji |
|---|---|---|
| `jetzt` | Jetzt / bin bereit | ⚡ |
| `heute_abend` | Heute Abend | 🌙 |
| `wochenende` | Wochenende | 📅 |
| `flexibel` | Flexibel / wann was zusammenkommt | 🕒 |

Kein Pflichtfeld — bleibt „Wann?" leer, verhält sich das Gesuch wie heute. Der Wert fließt ins Preset (auto-save/-load wie Rang/Plätze).

### 3.3 Wirkung

- **Anzeige** im Forum-Post-Content: `… · 🌙 Heute Abend` in der Kopfzeile.
- **Matching** (Teil B): Das Fenster des Gesuchs geht als Signal in die Timing-Prüfung ein.

## 4. Teil B — Match-Watch + DM

### 4.1 Einstieg: gepinnter Forum-Post „Warteraum"

Ein dauerhafter, gepinnter Forum-Thread im LFG-Forum: **„🔔 Warteraum — ich sag dir Bescheid"**. Starter-Nachricht erklärt das Feature und trägt einen Button **„🔔 Benachrichtige mich"** (custom_id `lfg:watch:start`).

- Erzeugung/Pinnen idempotent wie das LFG-Panel (kv-Key `lfg_watch_post` merkt die Message-/Thread-ID; existiert sie, wird nur editiert, nicht neu erstellt).
- Der Button ist persistent (überlebt Bot-Restarts, da er über custom_id dispatcht wird).

### 4.2 Watch-Builder (ephemer)

Klick auf „🔔 Benachrichtige mich" öffnet ein ephemeres Panel — **dieselben Bausteine wie das Gesuch-Draft**, damit UI und Rang-Logik nicht dupliziert werden:

- Modus-Select (casual / ranked / street brawl) — Ranked nur bei verifiziertem Rang (`ranked_allowed`, gleiche Prüfung).
- Rang-Von/Bis-Select (die 11 Emoji-Ränge, „Rang egal" erlaubt).
- Fenster-Select **„Wie lange soll ich Ausschau halten?"**: „die nächsten 3 Std." / „heute" / „dieses Wochenende" / „diese Woche".
- Button **„🔔 Aktivieren"**.

Default-Vorbelegung aus dem Voice-Profil: Hat der User typische Abendstunden, schlagen wir „heute" vor. (Vorbelegung, nicht erzwungen.)

### 4.3 Datenmodell: `activity.lfg_watches`

```sql
CREATE TABLE activity.lfg_watches (
    id              BIGSERIAL PRIMARY KEY,
    user_id         BIGINT      NOT NULL,
    guild_id        BIGINT      NOT NULL,
    mode            TEXT        NOT NULL,           -- casual | ranked | street_brawl
    rank_min        INT,                            -- NULL = nach unten offen (egal)
    rank_max        INT,                            -- NULL = nach oben offen (egal)
    window_kind     TEXT        NOT NULL,           -- now3h | today | weekend | week
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,           -- Selbst-Ablauf
    fired_at        TIMESTAMPTZ,                    -- NULL = scharf; gesetzt = schon ausgelöst (one-shot)
    matched_post_id BIGINT                          -- auslösendes Gesuch (Audit)
);

CREATE INDEX idx_lfg_watches_arm
    ON activity.lfg_watches (guild_id, mode, expires_at)
    WHERE fired_at IS NULL;

CREATE UNIQUE INDEX uq_lfg_watches_one_armed_per_user
    ON activity.lfg_watches (guild_id, user_id)
    WHERE fired_at IS NULL;
```

**Ein scharfer Watch pro User** (partielles Unique-Index). Erneutes „Aktivieren" ersetzt den bestehenden (Upsert). Bereits ausgelöste (`fired_at IS NOT NULL`) bleiben als Audit erhalten und blockieren nicht.

`window_kind` setzt `expires_at`:
- `now3h` → `now() + 3h`
- `today` → heute bis ~02:00 Berlin (Rest des Tages)
- `weekend` → Ende Sonntag Berlin
- `week` → `now() + 7d` (Cap)

### 4.4 Matcher (Hook in `handle_post_draft`)

Nachdem der Forum-Post steht (nach Z. 1881), läuft der Matcher **nicht-blockierend** (spawn), damit die Interaktion nicht verzögert:

1. `range = lfg_rank_range_from_draft(&draft)` des neuen Gesuchs.
2. Query: `SELECT … FROM activity.lfg_watches WHERE guild_id = $g AND mode = $m AND fired_at IS NULL AND expires_at > now() AND user_id <> $author`.
3. Pro Kandidat-Watch:
   - **Rang-Überlappung**: `[rank_min, rank_max] ∩ [range.min, range.max] ≠ ∅` (NULL/„egal" auf einer Seite ⇒ überlappt immer).
   - **Timing scharf?** — ODER-Verknüpfung dreier Signale:
     - Watcher **gerade in Voice** (offene Session in `voice_session_log`, `ended_at IS NULL`), **oder**
     - aktuelle Stunde ∈ **typische Aktivstunden** des Watchers (aus `voice_session_log`, s. 4.5), **oder**
     - aktuelle Zeit passt zum **Watch-Horizont** (`window_kind`: `now3h`/`today` ⇒ trivially jetzt; `weekend` ⇒ nur Sa/So) oder zum **`play_window` des Gesuchs** (z. B. Gesuch = „Jetzt" ⇒ jemand sucht gerade aktiv).
   - Beide bestanden ⇒ **DM senden**, dann `fired_at = now(), matched_post_id = post`.
4. Mehrere passende Watches ⇒ jeder Watcher bekommt seine eigene DM.

### 4.5 Typische Aktivstunden (Activity-Timeline)

Helper `typical_active_hours(pool, guild_id, user_id) -> HashSet<u8>`:

```sql
SELECT EXTRACT(HOUR FROM started_at AT TIME ZONE 'Europe/Berlin')::int AS hour,
       count(*) AS n
FROM activity.voice_session_log
WHERE guild_id = $g AND user_id = $u AND started_at > now() - interval '90 days'
GROUP BY hour;
```

Stunden mit `n ≥ schwelle` (z. B. oberes Drittel der eigenen Verteilung, mind. 3 Sessions) gelten als „typisch aktiv", jeweils ±1 Std. Puffer.

**Cold-Start:** < K (z. B. 10) Sessions insgesamt ⇒ kein Activity-Gate, es zählt nur `window_kind` (Fallback, damit neue User nicht durchs Raster fallen). Ergebnis wird kurz gecached (z. B. 15 min) — pro Gesuch wird die Query sonst je Watcher wiederholt.

### 4.6 DM-Inhalt

> 🔔 **Passendes Gesuch!**
> **Ranked** · Archon bis Phantom · 🌙 Heute Abend — von @Nutzer.
> Schau vorbei: {Link zum Forum-Thread}
> _(Einmalige Benachrichtigung. Für neue: wieder auf „🔔 Benachrichtige mich" im Warteraum.)_

Port bekommt dafür eine Methode `send_dm(user_id, content) -> Result<()>` (via `http.create_dm` + Nachricht).

## 5. Fehlerbehandlung & Edge Cases

- **DMs geschlossen** → Fehler loggen, Watch trotzdem `fired_at` setzen (kein Spam-Retry). v1 kein In-Channel-Fallback.
- **Autor == Watcher** → per Query ausgeschlossen (`user_id <> author`).
- **Rang „egal"** auf einer Seite → überlappt immer.
- **Abgelaufene Watches** → Matcher ignoriert sie (`expires_at > now()`); gelegentlicher Cleanup (Löschen `fired_at IS NOT NULL OR expires_at < now() - 7d`) lazy oder per bestehendem Housekeeping-Tick.
- **Matcher-Fehler** → nur loggen, niemals die Gesuch-Veröffentlichung scheitern lassen (läuft entkoppelt).
- **Ranked-Watch ohne verifizierten Rang** → Watch-Builder blockt beim Modus wie das Gesuch (`ranked_allowed`).

## 6. Tests (TDD)

**Unit:**
- Rang-Überlappung (beide gesetzt, eine egal, beide egal, disjunkt).
- Timing-Gate: in-Voice / typische-Stunde / Fenster einzeln und ODER-kombiniert; Cold-Start-Fallback.
- `window_kind` → `expires_at`-Berechnung (feste Referenzzeit als Parameter, kein `now()` im Test — deterministisch).
- Watch-Upsert: zweites Aktivieren ersetzt scharfen Watch; ausgelöster blockiert nicht.

**Integration (Test-DB, `rust/scripts/central_test_db.sh`):**
- Watch anlegen → passendes Gesuch → genau **eine** DM, `fired_at` gesetzt, `matched_post_id` korrekt.
- Zweites passendes Gesuch → **keine** DM (one-shot).
- Falscher Modus / disjunkter Rang / abgelaufen → keine DM.
- `typical_active_hours` gegen geseedete `voice_session_log`-Zeilen → korrekte Stundenmenge.

Die DM-Zustellung wird über den Port abstrahiert (Test-Port zählt `send_dm`-Aufrufe), keine echten Discord-Calls im Test.

## 7. Scope / YAGNI

**Drin:** grobes Zeitfenster im Gesuch, gepinnter Warteraum-Post + Button, ein-scharfer Watch pro User, aktivitätsgesteuertes Timing, one-shot Selbst-Ablauf, DM.

**Bewusst draußen (später ggf.):**
- Keine exakten Uhrzeiten (User-Pivot: grobes Fenster + Activity-Daten).
- Kein Dauer-Abo / stehende Subscription (nur one-shot).
- Kein „du spielst oft mit X" via `co_player_ids` (Datenbasis liegt, aber v1 out).
- Kein DM-Digest/Batching — eine DM je ausgelöstem Watch.
- Keine Lobby-Zustandsverfolgung über das Gesuch hinaus (ein Gesuch = das Lobby-Signal).

## 8. Betroffene Dateien

- `rust/crates/dl-voice/src/lfg_panel.rs` — `LfgWindowKind`, Draft/Preset-Feld, „Wann?"-Select, Content-Zeile, Watch-Builder-Flow, Matcher-Hook.
- `rust/crates/dl-voice/src/glue.rs` bzw. Port-Trait — `send_dm`, Warteraum-Post ensure/pin.
- **Neue Migration** `activity.lfg_watches` (+ Indizes) im zentralen DB-Crate/Migrations-Ordner.
- `dispatch.rs` — neue custom_ids (`lfg:watch:start`, Watch-Builder-Selects, `lfg:watch:activate`) sind schon vom Prefix-Dispatch abgedeckt, sofern im Panel-Interface geroutet.
- Doku + `CHANGELOG.md`.
