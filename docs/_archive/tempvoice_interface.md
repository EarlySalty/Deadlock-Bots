# TempVoice Interface – Dokumentation

## Übersicht

Das TempVoice Interface wird automatisch erstellt, wenn ein User einen **Staging-Channel** betritt und dadurch eine temporäre Lane erzeugt. Das Interface ermöglicht dem Lane-Owner und Mods, die Lane direkt über Discord-Buttons und Selects zu steuern.

---

## Wo erscheint das Interface?

Das Interface wird **nur im dedizierten Interface-Kanal** gepostet (globale Nachrichten ohne Lane-Bindung). Das Senden in den Voice-Call-Chat der Lane selbst ist deaktiviert, da es dort redundant wäre.

---

## Layout (5 Rows)

### Alle Lanes (Ranked + Casual)

| Row | Komponenten |
|-----|-------------|
| 0 | 🇩🇪 DE · 🌍 EU · 👑 Owner Claim · 🔢 Limit |
| 1 | 🦵 Kick · 🚫 Ban · ✅ Unban |
| 3 | 🔄 Reset · 👥 Duo · 👥 Trio · 👻 Lurker |

### Ranked Lanes (zusätzlich)

| Row | Komponenten |
|-----|-------------|
| 1 | 🦵 Kick · 🚫 Ban · ✅ Unban · 👻 Lurker *(verschoben)* |
| 2 | ① Haupt-Rang Select (Dropdown mit Emojis) |
| 3 | 🔄 Reset · 👥 Duo · 👥 Trio · 💾 Preset speichern · 🗂 Preset laden |
| 4 | ② Sub-Rang Select (1–6) |

> **Hinweis:** Bei Ranked-Lanes wird der Lurker-Button auf Row 1 verschoben, damit Row 4 für den Sub-Rang Select frei wird (Discord erlaubt max. 5 Rows).

---

## Button-Beschreibungen

### Row 0

| Button | Funktion |
|--------|----------|
| 🇩🇪 DE | Setzt den Sprachfilter auf **Deutsch only** |
| 🌍 EU | Setzt den Sprachfilter auf **alle EU-Sprachen** |
| 👑 Owner Claim | Übernimmt den Lane-Owner (wenn der Owner die Lane verlassen hat) |
| 🔢 Limit | Öffnet ein Modal zum Setzen des Spielerlimits (0 = kein Limit) |

### Row 1

| Button | Funktion |
|--------|----------|
| 🦵 Kick | Wählt ein Mitglied zum Kicken aus (Ephemeral-Select) |
| 🚫 Ban | Bannt einen User per @Mention, Name oder ID (Modal) |
| ✅ Unban | Hebt einen Ban auf (Modal) |
| 👻 Lurker *(Ranked)* | Schaltet den Lurker-Modus für den eigenen User um |

### Row 3 (Quick Templates)

| Button | Funktion |
|--------|----------|
| 🔄 Reset | Setzt Lane-Name und Limit auf den Standardwert zurück |
| 👥 Duo | Setzt Limit auf 2 (Duo-Lane) |
| 👥 Trio | Setzt Limit auf 3 (Trio-Lane) |
| 👻 Lurker *(Non-Ranked)* | Schaltet den Lurker-Modus um |
| 💾 Preset speichern *(Ranked)* | Speichert aktuellen Lane-Zustand als Preset (Modal) |
| 🗂 Preset laden *(Ranked)* | Lädt ein gespeichertes Preset (Ephemeral-Select) |

---

## Mindest-Rang (nur Ranked)

Das Setzen des Mindest-Rangs erfolgt in **zwei Schritten**:

### Schritt 1 – Haupt-Rang (Row 2)
- Dropdown mit allen Deadlock-Rängen (Initiate bis Eternus)
- Jeder Rang zeigt das zugehörige Server-Emoji
- Kein "Kein Limit" – für Ranked ist immer ein Rang erforderlich
- Nach Auswahl: ephemeral Bestätigung, Rang wird **noch nicht angewendet**

### Schritt 2 – Sub-Rang (Row 4)
- Dropdown mit Sub-Rang 1–6
- Erst nach Auswahl wird der kombinierte Rang angewendet (z. B. `Archon 3`)
- Wenn kein Haupt-Rang gepended ist: Fehlermeldung

### Rang-Validierung
- User können keinen höheren Rang setzen als ihren eigenen Rang
- Der Min-Rank-Check erfolgt anhand der Rollen des Users

### Beispiel-Flow
```
User wählt "Archon" aus Dropdown ① →
Bot antwortet ephemeral: "Haupt-Rang Archon gespeichert – jetzt ② Sub-Rang (1-6) auswählen."

User wählt "4" aus Dropdown ② →
Bot wendet "archon 4" als Mindest-Rang an
Bot antwortet ephemeral: "Mindest-Rang gesetzt auf: Archon 4."
```

---

## Pending-State

Der ausgewählte Haupt-Rang wird im In-Memory-Dict `_pending_main_rank` gespeichert:
```python
_pending_main_rank: dict[int, str] = {}  # lane_id → rank_name
```
Der Eintrag wird beim Anwenden (Sub-Rang gewählt) automatisch gelöscht.

---

## Wichtige IDs & Konstanten (aus `core.py`)

| Konstante | Bedeutung |
|-----------|-----------|
| `RANKED_CATEGORY_ID` | Kategorie-ID der Ranked/Comp-Lane |
| `MINRANK_CATEGORY_IDS` | Set aller Kategorien mit Min-Rank-Funktion |
| `RANK_ORDER` | Reihenfolge aller Ränge (Index = Stärke) |
| `INTERFACE_TEXT_CHANNEL_ID` | Kanal für globale Interface-Nachrichten |

---

## Datenbankstruktur (tempvoice_interface)

```sql
CREATE TABLE tempvoice_interface (
    guild_id    INTEGER,
    channel_id  INTEGER,
    message_id  INTEGER,
    category_id INTEGER,
    lane_id     INTEGER,  -- NULL = globale Nachricht
    updated_at  TEXT,
    PRIMARY KEY (guild_id, message_id)
);
```

- `lane_id IS NULL` → globale Interface-Nachricht im Interface-Kanal
- `lane_id IS NOT NULL` → lane-spezifische Nachricht (wird bei Lane-Löschung entfernt)

---

## Persistente Views

Beim Bot-Start werden persistente Views registriert, damit Buttons nach Neustart weiter funktionieren:

```python
# cog_load in TempVoiceInterface
bot.add_view(MainView(core, util, include_minrank=True, include_presets=True))   # Ranked
bot.add_view(MainView(core, util, include_minrank=False, include_presets=False)) # Casual
```

Alle Custom IDs müssen konsistent bleiben:

| Custom ID | Komponente |
|-----------|------------|
| `tv_region_de` | DE Button |
| `tv_region_e` | EU Button |
| `tv_owner_claim` | Owner Claim Button |
| `tv_limit_btn` | Limit Button |
| `tv_kick` | Kick Button |
| `tv_ban` | Ban Button |
| `tv_unban` | Unban Button |
| `tv_lurker` | Lurker Button |
| `tv_minrank` | Haupt-Rang Select |
| `tv_subrank_perm` | Sub-Rang Select (permanent) |
| `tv_tpl_reset` | Reset Button |
| `tv_tpl_duo` | Duo Button |
| `tv_tpl_trio` | Trio Button |
| `tv_preset_save` | Preset speichern Button |
| `tv_preset_load` | Preset laden Button |
