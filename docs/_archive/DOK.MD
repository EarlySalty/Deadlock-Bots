Hier ist eine kompakte, aber vollständige Doku/README für den **TempVoice**-Ordner. Du kannst den Text 1:1 als `cogs/tempvoice/README.md` ablegen.

---

# TempVoice – Auto-Lanes + UI (DE/EU, MinRank, Owner-Claim, Kick/Ban)

Dieses Paket stellt ein schlankes, DB-zentriertes System für temporäre Voice-„Lanes“ bereit.
Es besteht aus zwei Cogs, die zusammen als **eine Extension** `cogs.tempvoice` geladen werden:

* **Core (Logik & Persistenz)**: Auto-Erstellung, Owner-Handling, Bans, Region-Filter, MinRank, Purge.
* **Interface (UI)**: Persistente Buttons/Selects im festen Textkanal (keine Text-Commands).

Die zentrale DB (`shared.db`) wird über `utils.deadlock_db.DB_PATH` verwendet.
Keine eigene DB-Schicht außerhalb davon.

---

## Verzeichnisstruktur

```
cogs/tempvoice/
├─ __init__.py      # Orchestriert die Cogs – lädt Core und Interface in richtiger Reihenfolge
├─ core.py          # Geschäftslogik: Auto-Lanes, Owner, Bans, Region, MinRank, Purge, DB
└─ interface.py     # UI: Buttons/Selects/Modal, persistente Panel-Message, ruft Core-APIs
```

> **Hinweis zu `util.py`:**
> In der aktuellen Aufteilung liegen alle „einzigen Wahrheiten“ (IDs, Tabellen, Kernlogik) im **Core**.
> Ein separates `util.py` ist nicht nötig; falls es existiert, enthält es nur leichte Helfer ohne Logik.

---

## Was macht welches Script?

### `__init__.py` (Extension-Setup)

* Exportiert **eine** Extension `cogs.tempvoice` für den Master-Loader.
* Erzeugt **zuerst** `TempVoiceCore`, übergibt ihn anschließend an `TempVoiceInterface`, damit die UI auf Core-Methoden zugreifen kann.
* Registriert beide Cogs beim Bot.

### `core.py` – **TempVoiceCore**

**Aufgaben:**

* **Auto-Lane**: Beim Join in einen der **Staging-Channels** wird eine Lane erstellt, der Nutzer rüber gemoved und in der DB erfasst.
* **Persistenz (DB)**:
  Tabellen:

  * `tempvoice_lanes(channel_id, guild_id, owner_id, base_name, category_id, created_at)`
  * `tempvoice_bans(owner_id, banned_id, created_at)`
  * `tempvoice_interface(guild_id, channel_id, message_id, updated_at)` *(vom Interface genutzt)*
  * `tempvoice_owner_prefs(owner_id, region, updated_at)` *(DE/EU-Präferenz pro Owner)*
* **Owner-Handling**:

  * Beim Verlassen des Owners geht die Lane an den **am längsten anwesenden** Member.
  * **Owner-Claim**: Button in der UI überträgt den Owner sofort an den Klicker (kein Timer).
* **Region-Filter (DE/EU)**:

  * Speichert **persönliche Präferenz** (DE/EU) des Nutzers in `tempvoice_owner_prefs`.
  * Wendet bei Lane-Erstellung die Präferenz des Owners automatisch an.
  * Setzt/entfernt `connect`-Overwrites für die Rolle **„English Only“** (ID konfiguriert).
* **MinRank**:

* **Nur** in den freigegebenen Kategorien aktiv (`MINRANK_CATEGORY_IDS`).
  * Setzt `connect=False` für zu niedrige Rank-Rollen, löscht Overwrites bei „Kein Limit“.
* **Kick/Ban/Unban**:

  * **Ban** ist „Owner-lokal“: Owner → viele gebannte User (in `tempvoice_bans`).
  * Beim Betreten der Lane wird ein gebannter User in einen Staging-Channel verschoben (Soft-Enforce).
  * Resolver akzeptiert `@Mention`, **ID** und **exakten Namen** (Case-insensitiv).
* **Purge & Rehydration**:

  * Beim Start: DB-Einträge laden (**Rehydrate**).
  * **Purge** leerer Lanes: sofort & verzögert (zweiter Sweep), inkl. Entfernen verwaister DB-Zeilen.
  * Kategorie-Umzug: Wird eine TempVoice-Lane manuell in eine andere Kategorie verschoben, passt der Bot Prefix/Limits gemäß den hinterlegten Staging-Regeln dieser Kategorie an.
* **Sichere Channel-Edits**: Locking + Cooldown, damit Namen/Limits nicht 429-anfällig sind.

**Wichtige öffentliche Methoden (vom Interface genutzt):**

* `parse_user_identifier(guild, raw) -> Optional[int]`
  (Ermittelt User-ID aus @, ID, Name)
* `set_region_pref(owner_id, region) / get_region_pref(owner_id) / apply_region(lane, region)`
  (Präferenz sichern/lesen, Overwrites setzen)
* `transfer_owner(lane, member_id)`
  (Sofortiger Owner-Transfer)
* `safe_edit_channel(lane, desired_name?, desired_limit?, reason?)`
  (Rate-begrenzte, threadsichere Channel-Edits)
* `refresh_name(lane)`
  (Titel neu zusammensetzen & anwenden)
* `db` / `first_guild()`
  (gezielter Zugriff für Interface-Funktionen)

**Konfigurations-Konstanten (oben in `core.py`):**

* `STAGING_CHANNEL_IDS`: Menge aller Staging-Voice-Channel-IDs
* `FIXED_LANE_IDS`: Voice-Channel-IDs, die TempVoice ignoriert (kein Purge/Rename/Interface)
* `MINRANK_CATEGORY_IDS`: Kategorien, in denen MinRank aktiv ist
* `RANKED_CATEGORY_ID`: (für Defaults pro Kategorie)
* `INTERFACE_TEXT_CHANNEL_ID`: Textkanal, in den das Interface gepostet wird
* `ENGLISH_ONLY_ROLE_ID`: Rolle, die für „Deutsch-Only“ gesperrt wird
* `DEFAULT_CASUAL_CAP` / `DEFAULT_RANKED_CAP`: Standard-Userlimits
* `RANK_ORDER`: Reihenfolge der Ränge (Name = Rollenname in Discord)

### `interface.py` – **TempVoiceInterface**

**Aufgaben:**

* **Persistentes Panel** im Textkanal `INTERFACE_TEXT_CHANNEL_ID`:

  * Speichert `message_id` in `tempvoice_interface`, editiert bei Neustart statt neu zu posten.
* **UI-View mit Buttons/Selects** (keine Text-Commands):

  * **🇩🇪 DE / 🇪🇺 EU**
    Speichert die User-Präferenz (DE/EU) und wendet sie auf die **aktuelle Lane** an.
    → Core: `set_region_pref(...)` + `apply_region(...)`
  * **🎚️ Limit setzen**
    Modal zur Eingabe (0–99), setzt `user_limit`.
    → Core: `safe_edit_channel(... desired_limit=...)`
  * **Mindest-Rang (Select)**
    Nur aktiv, wenn die Lane in `MINRANK_CATEGORY_IDS` liegt.
    → Core: `_apply_min_rank` (intern über `refresh_name`/private APIs)
  * **👢 Kick**
    Verschiebt gewählten User in den ersten erreichbaren Staging-Channel.
  * **🚫 Ban / ♻️ Unban**
    Ban/Unban per @/ID/Name (Resolver im Core).
    Ban: `connect=False` + optionaler Move in Staging, DB-Persistenz in `tempvoice_bans`.
  * **👑 Owner Claim**
    Sofortiger Transfer des Owner-Status an den Klicker.
    → Core: `transfer_owner(...)`
* **Berechtigungslogik**:
  Für sensitive Aktionen (Region wechseln, Limit, Kick/Ban/Unban) gilt:

  * Erlaubt, wenn **Owner** der Lane **oder** `manage_channels` **oder** `administrator`.

---

## Ereignisfluss (End-to-End)

1. **Join** eines Nutzers in **Staging**
   → `on_voice_state_update` (Core) erkennt Staging → `_create_lane(...)`
   → VoiceChannel wird erstellt, der Nutzer wird gemoved, Lane wird in DB gespeichert
   → Owner-Präferenz (DE/EU) wird sofort angewendet
   → Owner-Bans werden als Overwrites gesetzt
   → Name aktualisiert (MinRank-Suffix nur in Spezial-Kategorie)

2. **Owner verlässt die Lane**
   → Längste Anwesenheit bestimmt neuen Owner (DB-Update).
   → Wenn Lane **leer** → Channel löschen + DB-Zeile löschen.

3. **UI-Interaktionen**

   * Region-Button → Präferenz speichern + Overwrites setzen
   * Limit-Modal → Limit einstellen
   * MinRank-Select → nur in Spezial-Kategorie wirksam
   * Kick/Ban/Unban → Move-to-Staging bzw. Overwrites + DB-Persistenz
   * Owner-Claim → sofortiger Transfer

4. **Neustart des Bots**

   * Interface findet/editiert die bestehende Panel-Message (per DB)
   * Core **rehydriert** alle Lanes aus DB
   * **Purge** alle **leeren** Lanes (inkl. Fallback-Sweep)
   * Verwaiste DB-Zeilen werden bereinigt

---

## Berechtigungen & Voraussetzungen

* **Bot-Intents**: `members`, `voice_states`, `guilds`, `message_content` (optional für Logging)
* **Bot-Rechte**:

  * *Manage Channels* (Voice erstellen/ändern, Overwrites setzen)
  * *Move Members* (Kick = Move in Staging)
* **Rolle „English Only“** (ID: `ENGLISH_ONLY_ROLE_ID`) muss existieren.
* **Konfiguration** der IDs oben in `core.py` prüfen/anpassen:

  * `STAGING_CHANNEL_IDS`, `MINRANK_CATEGORY_IDS`, `INTERFACE_TEXT_CHANNEL_ID`, `ENGLISH_ONLY_ROLE_ID`

---

## Quickstart

1. IDs in `core.py` setzen.
2. Bot starten – `main_bot.py` lädt `cogs.tempvoice` automatisch (Auto-Discovery).
3. **Staging** beitreten → Lane wird erstellt, DE/EU Präferenz angewandt.
4. **Interface-Buttons** im Textkanal nutzen:

   * 🇩🇪/🇪🇺, 🎚️ Limit, 👑 Owner Claim, 👢 Kick, 🚫 Ban, ♻️ Unban, MinRank (nur Special-Kategorie).
5. Neustart testen → Leere Lanes werden weggeräumt; Panel bleibt erhalten.

---

## Erweiterbarkeit

* **Neue UI-Aktionen**: In `interface.py` als Button/Modal/Select ergänzen und über die **öffentlichen Core-Methoden** arbeiten.
* **Neue Policies** (z. B. weitere Filter): Im **Core** implementieren und als schlanke Fassade (Wrapper) bereitstellen – das Interface bleibt dünn.
* **Weitere Persistenz**: Neue, kleine Tabellen im Core (`TVDB._create_tables`) hinzufügen. Keine separate DB-Logik streuen.

---

## Troubleshooting

* **Interface: Textkanal nicht gefunden**
  → `INTERFACE_TEXT_CHANNEL_ID` prüfen (muss ein **TextChannel** sein).
* **DE/EU tut nichts**
  → Rolle „English Only“ existiert? Bot hat *Manage Channels*?
* **Ban via `@`/Name** funktioniert nicht
  → Exakte Schreibweise nötig. Bei Mehrdeutigkeit wird abgebrochen. (Optional: Fuzzy-Resolver nachrüstbar.)
* **Purge nach Neustart greift nicht**
  → Prüfe, ob der Channel wirklich **leer** ist und ob der Bot *Manage Channels* hat.

---

## Design-Prinzipien

* **Eine Quelle der Wahrheit** (Core): Alle Regeln & IDs zentral.
* **UI ist dünn**: Buttons rufen Core-Methoden auf, kein „zweites Gehirn“ im Interface.
* **DB-first**: Rehydration + Purge sichern konsistenten Zustand über Neustarts.
* **Rate-safe**: Edit-Locks & Cooldown vermeiden 429.

---

Wenn du willst, packe ich dir das als fertige `README.md` in Markdown mit Emojis/Badges – sag Bescheid und ich formatiere dir das exakt für dein Repo.
