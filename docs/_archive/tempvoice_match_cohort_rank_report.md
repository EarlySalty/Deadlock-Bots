# TempVoice Match Cohort / Rank Anchor Report

## Ziel

Die TempVoice- und Ranglogik wurde so angepasst, dass:

- die Lobby-/Match-Anzeige nur noch auf der tatsaechlich zusammengehoerigen Match-/Lobby-Gruppe basiert
- Rangberechnung, Anchor-Auswahl und Sorting nicht mehr auf allen Personen im Voice-Channel basieren
- der initiale Lane-Ersteller als stabile Elo-Basis erhalten bleibt, wenn er einen gueltigen Rang hat
- der operative TempVoice-Owner weiterhin normal wechseln kann

## Ausgangsproblem

### 1. Deadlock-Status im Channel blieb teilweise zu lange sichtbar

Vorher wurde der Channel-Status aus den Presence-Daten im gesamten Voice-Channel abgeleitet. Dabei konnten:

- Personen aus unterschiedlichen Lobbys/Matches vermischt werden
- Personen ohne gemeinsame Lobby den effektiven Slot-/Spieler-Kontext verfälschen
- der Rename-Cooldown das Zuruecksetzen des Suffixes blockieren, obwohl kein Match/Lobby-Status mehr aktiv war

### 2. Ranglogik betrachtete zu viele Personen

Die Ranglogik arbeitete bislang implizit mit allen anwesenden Mitgliedern im Voice-Channel. Dadurch konnten:

- Personen aus anderen Lobbys/Matches die Rangbasis beeinflussen
- unbeteiligte Personen Sorting und Anchor-Fallbacks mitbestimmen
- das Resultat ungenau werden, wenn Features wie Steam-Link / Presence nicht von allen genutzt wurden

### 3. Initialer Owner war nicht stabil von aktuellem Owner getrennt

`tempvoice_lanes.owner_id` war zugleich:

- aktueller TempVoice-Owner
- implizite Rang-/Elo-Basis

Sobald TempVoice den Owner auf einen anderen Nutzer uebertragen hat, war der urspruengliche Lane-Ersteller fuer die Rangbasis nicht mehr sauber getrennt vorhanden.

## Umgesetzte Aenderungen

### A. Gemeinsame Cohort-Logik fuer Match/Lobby

Neue Datei:

- `service/deadlock_voice_cohort.py`

Diese kapselt:

- Erkennung von `match` / `lobby` aus `live_player_state`
- Auswahl des besten Presence-Status pro User bei mehreren Steam-Accounts
- Bildung einer relevanten Channel-Cohort nach `(stage, server_id)`

Regeln:

- `match` hat Vorrang vor `lobby`
- innerhalb desselben Stages gewinnt die groessere Gruppe
- nur Mitglieder mit derselben Lobby / demselben Match-Cluster gelten als relevante Gruppe

### B. Deadlock-Voice-Status nutzt nur noch die relevante Cohort

Datei:

- `cogs/deadlock_voice_status.py`

Aenderungen:

- Presence-Entscheidung wurde auf die neue Cohort-Logik umgestellt
- `voice_slots` basiert nicht mehr auf der gesamten Voice-Belegung, sondern nur noch auf der relevanten Cohort
- beim Uebergang von `match/lobby -> kein Status` wird der Rename-Cooldown fuer das Entfernen des Suffixes uebergangen

Ergebnis:

- der Channel-Suffix spiegelt nur noch die relevante Gruppe wider
- haengende `in der Lobby` / `im Match` Anzeigen werden sauberer entfernt

### C. Rangberechnung nur fuer die relevante Match-/Lobby-Gruppe

Datei:

- `cogs/rank_voice_manager.py`

Neue Logik:

- `get_rank_relevant_members(channel)` bestimmt aus Steam-Links und Presence nur die relevante Cohort
- `get_channel_members_ranks(channel)` verwendet nur noch diese Mitglieder

Davon betroffen sind:

- Anchor-Fallbacks
- Permission-Berechnung
- Channel-Name im Rank-Manager
- Rangbasierte Folgeentscheidungen

Ergebnis:

- Rang-/Score-Berechnung basiert nicht mehr auf allen Leuten im Voice, sondern nur auf der relevanten Match-/Lobby-Gruppe

### D. Trennung zwischen aktuellem Owner und initialem Owner

Datei:

- `cogs/tempvoice/core.py`

Neue Laufzeitstruktur:

- `lane_owner`: aktueller TempVoice-Owner
- `lane_initial_owner`: urspruenglicher Lane-Ersteller

Neue DB-Spalte:

- `tempvoice_lanes.initial_owner_id`

Migration:

- falls die Spalte noch nicht existiert, wird sie angelegt
- Altdaten erhalten `initial_owner_id = owner_id`

Verhalten:

- bei Lane-Erstellung werden `owner_id` und `initial_owner_id` auf denselben Nutzer gesetzt
- spaetere Owner-Wechsel ueberschreiben nur `owner_id`
- `initial_owner_id` bleibt stabil

### E. Initialer Owner priorisiert die Elo-/Anchor-Basis

Datei:

- `cogs/rank_voice_manager.py`

Neue Regel:

- wenn der initiale Owner einen gueltigen Rang hat, wird er als Anchor-/Elo-Basis verwendet
- nur wenn der initiale Owner keinen brauchbaren Rang hat, faellt die Logik auf relevante Cohort-Mitglieder zurueck

Ergebnis:

- der initiale Lane-Ersteller bestimmt weiterhin die Elo-Basis, auch wenn der operative Owner inzwischen gewechselt hat

### F. Lane-Sorting nutzt ebenfalls Initial-Owner und Cohort

Datei:

- `cogs/tempvoice/lane_sorting.py`

Neue Reihenfolge fuer Chill-Lanes:

1. Initial-Owner-Rang
2. falls nicht vorhanden: Durchschnitt der relevanten Match-/Lobby-Cohort
3. falls ebenfalls nicht vorhanden: bestehende Fallbacks aus Base-Name / Channel-Name

Ergebnis:

- Sorting wird nicht mehr von allen Channel-Mitgliedern beeinflusst
- der initiale Owner bleibt priorisiert, wenn er einen Rang hat

## Bewusste Verhaltensaenderung

Die Trennung zwischen TempVoice-Owner und Elo-Basis ist absichtlich.

Ab jetzt gilt:

- TempVoice-Funktionen wie Owner-Rechte, Ban/Kick, Region, Presets laufen weiter ueber den aktuellen `owner_id`
- Rang-/Anchor-/Elo-Basis kann weiterhin ueber den `initial_owner_id` laufen

Das ist keine unbeabsichtigte Nebenwirkung, sondern die gewuenschte neue Produktlogik.

## Verifikation

### Datenmodell

Pruefen:

```sql
SELECT channel_id, owner_id, initial_owner_id, base_name
FROM tempvoice_lanes
ORDER BY created_at DESC
LIMIT 10;
```

Erwartung:

- direkt nach Lane-Erstellung: `owner_id == initial_owner_id`
- nach spaeterem Owner-Wechsel: `owner_id != initial_owner_id` moeglich

### Anchor-Basis

Pruefen:

```sql
SELECT channel_id, user_id, rank_name, rank_value, anchor_subrank, score_min, score_max
FROM voice_channel_anchors
ORDER BY updated_at DESC
LIMIT 10;
```

Erwartung:

- wenn der initiale Owner einen Rang hat, bleibt `voice_channel_anchors.user_id` auf diesem initialen Owner
- der Anchor springt nicht nur deshalb auf den aktuellen TempVoice-Owner, weil sich `owner_id` geaendert hat

### Live-Szenarien

#### Szenario 1

- User A mit Rang erstellt Lane
- User B joint
- A verlaesst Lane

Erwartung:

- TempVoice-Owner wird B
- `initial_owner_id` bleibt A
- Rank-Anchor / Elo-Basis bleibt A

#### Szenario 2

- mehrere Personen sitzen im selben Voice
- nur ein Teil davon befindet sich in derselben Lobby / im selben Match

Erwartung:

- Anzeige, Rank-Cohort und Sorting nutzen nur diese relevante Gruppe

#### Szenario 3

- Match / Lobby endet

Erwartung:

- Channel-Suffix wird auch dann entfernt, wenn der normale Rename-Cooldown noch aktiv waere

## Technische Dateien

- `service/deadlock_voice_cohort.py`
- `cogs/deadlock_voice_status.py`
- `cogs/rank_voice_manager.py`
- `cogs/tempvoice/core.py`
- `cogs/tempvoice/lane_sorting.py`
- `tests/test_deadlock_voice_cohort.py`

## Validierung im Arbeitsstand

Erfolgreich:

- `python3 -m py_compile` fuer die geaenderten Module
- `python3 -m unittest tests/test_deadlock_voice_cohort.py`

Nicht in dieser Shell ausfuehrbar:

- bestehende Tests, die `discord.py` benoetigen, da das Paket in der lokalen Shell-Umgebung nicht installiert war
