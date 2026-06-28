# TempVoice — die komplette Anleitung

Diese Anleitung erklärt Schritt für Schritt, wie die automatischen Voice-Lanes
funktionieren: wie eine Lane entsteht, wie du sie über das Panel steuerst, was
beim Verschieben passiert und welche Einstellungen der Bot dauerhaft für dich
merkt. Eine kürzere Übersicht aller Voice-Funktionen steht in
[`voice-features.md`](voice-features.md) — dieses Dokument ist der Deep-Dive zu
TempVoice selbst.

---

## 1. Was ist TempVoice?

TempVoice gibt jeder Gruppe ihren eigenen Sprachkanal — automatisch. Du musst
keinen Kanal anlegen und niemanden um Rechte bitten: Du betrittst einen
Einstiegs-Kanal, und der Bot erstellt dir sofort eine eigene **Lane**, die dir
gehört. Geht die Lane leer, räumt der Bot sie von selbst wieder weg. Du
steuerst deine Lane komplett selbst über ein Button-Panel.

## 2. Eine Lane erstellen

Betritt einen der **Einstiegs-Kanäle** (Staging-Channels, meist mit `(+)`
markiert). Es gibt drei Sorten, je nachdem was du spielen willst:

| Einstiegs-Kanal | Wofür | Plätze (Standard) | Rang-System |
|---|---|---|---|
| **Chill** | Locker zocken, Quatschen, gemischte Ränge | 8 | nein |
| **Ranked / Comp** | Ranked-Runden mit Mindest-Rang-Filter | 6 | ja |
| **Street Brawl** | Genau dieser Spielmodus | 4 (fest) | nein |

Sobald du drin bist, wird **deine** Lane erstellt und du wirst hineingezogen. Du
bist automatisch der **Owner** dieser Lane und darfst sie steuern.

## 3. Das Steuerungs-Panel

Die Lane steuerst du im Kanal **`#sprach-kanal-verwalten`**. Das Panel zeigt dir
Knöpfe — welche genau, hängt vom Lane-Typ ab (Ranked hat mehr als Chill).

### Immer verfügbar

| Knopf | Was er macht |
|---|---|
| 🇩🇪 **DE** / 🇪🇺 **EU** | Setzt die Region deiner Lane. Die Wahl bleibt an **dir** hängen (siehe Abschnitt 5) und gilt auch für deine nächsten Lanes. |
| 👑 **Owner übernehmen** | Macht dich zum Owner — gedacht für den Fall, dass der ursprüngliche Owner die Lane verlassen hat. |
| 🎚️ **Limit setzen** | Begrenzt die Platzanzahl (0–99; 0 = kein Limit). |
| 🎯 **Mein Rang** | Deine Rang-Präferenz für die Lane-Benennung/Sortierung. |
| 👢 **Kick** / 🚫 **Ban** / ♻️ **Unban** | Mitglieder aus der Lane entfernen, sperren bzw. entsperren. |
| 🛡️ **Tag-Filter** | Zugangs-Filter: Mindest-Alter (z. B. 25+), Tonfall (Ragebaiter-Free), Ragebaiter blockieren. |
| 👻 **Lurker** | Schiebt stille Mitglieder, die nur „mithören", in einen reduzierten Zustand. |

### Schnell-Vorlagen

| Knopf | Was er macht |
|---|---|
| **Duo Call** | Stellt die Lane auf eine Duo-Runde (Name + Limit 2). |
| **Trio Call** | Stellt die Lane auf eine Trio-Runde (Name + Limit 3). |
| **Normale Lane** (Reset) | Setzt Vorlage/Limit wieder auf den Standard zurück. |

### Nur in Ranked-Lanes

| Knopf | Was er macht |
|---|---|
| ① **Haupt-Rang** → ② **Sub-Rang** | Mindest-Rang in **zwei Schritten** setzen (erst Haupt-, dann Sub-Rang). |
| 💾 **Preset speichern** | Merkt sich die aktuelle Lane-Konfiguration (siehe Abschnitt 5). |
| 🗂 **Preset laden** | Stellt eine gespeicherte Konfiguration wieder her. |

> **Wichtig:** Die Knöpfe wirken nur, wenn du selbst gerade in einer passenden
> Lane sitzt. Kick, Ban und Tag-Filter sind Owner-/Mod-Funktionen.

## 4. Lane verschieben & Modus wechseln

Eine Lane ist nicht fest an ihren Einstieg gebunden. Wird sie in eine **andere
Kategorie** verschoben (oder wechselt den Modus), **passt sie sich an**: Sie
übernimmt die Regeln, das Standard-Limit und das Rang-Verhalten der neuen
Kategorie.

Konkret heißt das:

- Verschiebst du eine **Street-Brawl-Lane** (4 Plätze, kein Rang) nach
  **Ranked**, gelten ab dann die Ranked-Regeln — inklusive Mindest-Rang-Option
  und dem Ranked-Standardlimit. Das alte 4er-Limit klebt **nicht** mehr fest.
- Umgekehrt verliert eine nach **Chill** verschobene Lane das Rang-System.

Die neue Zuordnung bleibt auch nach einem Bot-Neustart erhalten — der Bot merkt
sich, zu welcher Kategorie die Lane zuletzt gehörte, und wendet die richtigen
Regeln an.

## 5. Welche Einstellungen der Bot dauerhaft merkt

TempVoice speichert serverseitig, damit deine Lanes sich „wie beim letzten Mal"
verhalten:

- **Region (DE/EU):** hängt an **dir als Owner**, nicht an der einzelnen Lane.
  Jede neue Lane von dir startet direkt mit deiner Region.
- **Rang-Präferenz:** dein über „🎯 Mein Rang" gewählter Rang, für Benennung und
  Einsortierung deiner Lanes.
- **Presets (Ranked):** ein gespeichertes Preset bewahrt **Name, Limit,
  Mindest-Rang und Region** zusammen. Über „🗂 Preset laden" holst du genau diese
  Kombination zurück — praktisch, wenn du immer dieselbe Ranked-Runde aufmachst.
- **Tag-Filter:** dein gesetzter Zugangs-Filter bleibt für die Lane aktiv.
- **Bans / Lurker-Status:** bleiben innerhalb der Lane bestehen.

Owner, Presets, Bans, Lurker-Status und Tag-Filter überstehen auch einen
Bot-Neustart — die Lane wird danach mit deinen Einstellungen wiederhergestellt.

## 6. Automatisches Lane-Routing

Damit nie „alles in einer Lane" hängt, verteilt der Bot mit:

- **🆕 Neue-Spieler-Lane:** Einsteiger und niedrige Ränge landen bevorzugt hier.
  Wird eine Lane voll, öffnet der Bot automatisch die nächste.
- **🗨️ Off-Topic-Voice:** erweitert sich ebenfalls automatisch, wenn genug Leute
  drin sind.
- **Chill-/Ranked-Lanes** werden nach Rang einsortiert, damit passende Runden
  beieinander stehen.

## 7. Voice-Status (Lobby / Match)

Wenn du deinen **Steam-Account verknüpft** hast und in einer Ranked/Comp-Lane
sitzt, ergänzt der Bot den Kanal-Status automatisch — z. B. „in der Lobby (3/6)"
oder die laufende Match-Minute. Ohne Steam-Link oder bei veralteten Daten fällt
er auf einfachere Anzeigen zurück.

## 8. Häufige Fragen

- **Meine Knöpfe tun nichts.** Du musst selbst in der Lane sitzen, die du steuern
  willst, und (für Kick/Ban/Filter) Owner oder Mod sein.
- **Mindest-Rang lässt sich nicht setzen.** Das geht nur in Ranked/Comp, du musst
  verifiziert sein, und du kannst keinen höheren Rang verlangen als deinen eigenen.
- **Street Brawl bleibt bei 4 Plätzen.** Das ist so gewollt — Street-Brawl-Lanes
  ignorieren Rang-Caps und Mindest-Rang und haben immer maximal 4 Slots (solange
  die Lane in der Street-Brawl-Kategorie bleibt; siehe Abschnitt 4).
- **Region ändert sich „von selbst" für neue Lanes.** Richtig — die Region hängt
  an dir als Owner und wird auf jede neue Lane übernommen.
- **Lane ist weg.** Lanes verschwinden automatisch, sobald niemand mehr drin ist.

---

*Technische Referenz für Entwickler steht im Dev-Abschnitt von
[`voice-features.md`](voice-features.md).*
