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

| Einstiegs-Kanal | Wofür | Plätze (Standard) | Rang-Gate möglich |
|---|---|---|---|
| **Chill** | Locker zocken, Quatschen, gemischte Ränge | 8 | nein |
| **Ranked / Comp** | Ranked-Runden, optional mit Rang-Gate | 6 | ja |
| **Street Brawl** | Genau dieser Spielmodus | 4 (fest) | nein |

**Alle Lanes landen in einer gemeinsamen Kategorie** — egal über welchen
Einstieg. Was in einer Lane läuft, erkennst du am Namen und an der Position:

- **Ranked:** `Ranked <Rang> <Subrang>` (z. B. *Ranked Phantom 3*) — der Rang
  kommt aus deiner Rang-Präferenz (🎯 Mein Rang) oder deiner verifizierten
  Rang-Rolle.
- **Casual:** `Chill Lane N` — mit gesetztem Rang zusätzlich als Info am Ende,
  z. B. *Chill Lane 2 · Oracle*.
- **Street Brawl:** `Street Brawl N`, unverändert.

**Für Ranked brauchst du keine Verifizierung mehr** — jeder kann Ranked-Lanes
erstellen und joinen, solange der Owner kein Rang-Gate geschaltet hat.

Sobald du drin bist, wird **deine** Lane erstellt und du wirst hineingezogen. Du
bist automatisch der **Owner** dieser Lane und darfst sie steuern.

**Alternative: der Router-Kanal.** Statt eines festen Einstiegs kannst du auch
den Router-Voice betreten. Dort wählst du per Panel `Casual`, `Ranked`,
`Street Brawl` oder `Auto-Join`. Bei Auto-Join sucht dir der Bot eine passende
offene Lane — bevorzugt eine mit Leuten, mit denen du öfter spielst, und mit
1–5 Mitgliedern. Über den Router erstellte Casual-Lanes starten mit 6 Plätzen.

## 3. Das Steuerungs-Panel

Die Lane steuerst du im Kanal **<#1439564934592729161>**. Das Panel zeigt dir
Knöpfe — welche genau, hängt vom Lane-Typ ab (Ranked hat mehr als Chill).

### Immer verfügbar

| Knopf | Was er macht |
|---|---|
| 🇩🇪 **DE** / 🇪🇺 **EU** | Setzt die Region deiner Lane. Die Wahl bleibt an **dir** hängen (siehe Abschnitt 5) und gilt auch für deine nächsten Lanes. |
| 👑 **Owner übernehmen** | Macht dich zum Owner, wenn der ursprüngliche Owner die Lane verlassen hat. Zuerst dürfen die aktivsten Mitglieder der Lane übernehmen; nach 20 Minuten darf es jeder in der Lane. |
| 🎚️ **Limit setzen** | Begrenzt die Platzanzahl (0–99; 0 = kein Limit). |
| 🎯 **Mein Rang** | Deine Rang-Präferenz für die Lane-Benennung/Sortierung. |
| 👢 **Kick** / 🚫 **Ban** / ♻️ **Unban** | Mitglieder aus der Lane entfernen, sperren bzw. entsperren. |
| 🛡️ **Tag-Filter** | Zugangs-Filter für die Lane. Durchgesetzt werden Mindest-Alter (z. B. 25+) und das Blockieren von Ragebaitern; die Tonfall-Präferenz ist nur eine Info und sperrt niemanden aus. |
| 👻 **Lurker** | Schiebt stille Mitglieder, die nur „mithören", in einen reduzierten Zustand. |

### Schnell-Vorlagen

| Knopf | Was er macht |
|---|---|
| **Duo Call** | Stellt die Lane auf eine Duo-Runde (Name + Limit 2). |
| **Trio Call** | Stellt die Lane auf eine Trio-Runde (Name + Limit 3). |
| **Normale Lane** (Reset) | Setzt Vorlage/Limit wieder auf den Standard zurück. |

### Nur in Nicht-Ranked-Lanes

| Knopf | Was er macht |
|---|---|
| **Umbenennen** | Gibt deiner Lane einen eigenen Namen. |
| **Modus wechseln** | Wechselt den Lane-Modus (z. B. Casual ↔ Street Brawl) — die Lane übernimmt die Regeln des neuen Modus. |

### Nur in Ranked-Lanes

| Knopf | Was er macht |
|---|---|
| 🔓 **Rang-Gate** | Macht deine Lane exklusiv für verifizierte Ränge in einem Fenster (nur Owner). Ein Klick öffnet eine Erklärung mit zwei Auswahlfeldern — Mindestrang und Toleranz, vorbefüllt mit deinem Rang ±1,5 Ränge. Erst nach dem Bestätigen ist das Gate aktiv; ab dann sehen alle außerhalb des Fensters ein 🔒 an der Lane und können nicht mehr joinen. **Niemand wird entfernt** — das Gate wirkt nur auf neue Joins. Nochmal drücken schaltet es wieder aus. |
| 💾 **Preset speichern** | Merkt sich die aktuelle Lane-Konfiguration (siehe Abschnitt 5). |
| 🗂 **Preset laden** | Stellt eine gespeicherte Konfiguration wieder her. |

Die früheren Dauer-Menüs „① Haupt-Rang → ② Sub-Rang" gibt es nicht mehr —
der Mindest-Rang wird jetzt ausschließlich über den Rang-Gate-Dialog gesetzt.

> **Wichtig:** Die Knöpfe wirken nur, wenn du selbst gerade in einer passenden
> Lane sitzt. Kick, Ban und Tag-Filter sind Owner-/Mod-Funktionen.

## 4. Modus wechseln

Da alle Lanes in derselben Kategorie stehen, entscheidet nicht mehr die
Kategorie über die Regeln, sondern der **Modus der Lane** (Ranked, Casual,
Street Brawl). Der Bot merkt sich den Modus jeder Lane dauerhaft — auch über
einen Bot-Neustart hinweg.

- Bei Nicht-Ranked-Lanes wechselst du den Modus über **🔄 Modus wechseln** im
  Panel; die Lane übernimmt dann Regeln und Standard-Limit des neuen Modus
  (z. B. Street Brawl → fest 4 Plätze).
- Ranked-Funktionen wie das Rang-Gate gibt es nur in Ranked-Lanes.

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
- **Sortierung in der gemeinsamen Kategorie:** Oben stehen die Ranked-Lanes,
  nach Rang aufsteigend sortiert (klein → groß), darunter die Casual-Lanes,
  ganz unten Street Brawl. So findest du Runden auf deinem Niveau auf einen
  Blick.

## 7. Voice-Status (Lobby / Match)

Wenn du deinen **Steam-Account verknüpft** hast und in einer Ranked/Comp-Lane
sitzt, ergänzt der Bot den Kanal-Status automatisch — z. B. „in der Lobby (3/6)"
oder die laufende Match-Minute. Ohne Steam-Link oder bei veralteten Daten fällt
er auf einfachere Anzeigen zurück.

## 8. Häufige Fragen

- **Meine Knöpfe tun nichts.** Du musst selbst in der Lane sitzen, die du steuern
  willst, und (für Kick/Ban/Filter) Owner oder Mod sein.
- **Warum hat eine Lane ein 🔒?** Der Owner hat dort das Rang-Gate aktiviert und
  dein verifizierter Rang liegt außerhalb des eingestellten Fensters (oder du
  hast keinen verifizierten Rang). Alle anderen Lanes bleiben offen.
- **Muss ich für Ranked verifiziert sein?** Nein — Ranked-Lanes erstellen und
  joinen geht ohne Verifizierung. Nur wenn der Owner ein Rang-Gate schaltet,
  zählt ausschließlich die verifizierte Rang-Rolle für den Zutritt.
- **Fliege ich raus, wenn das Gate angeht?** Nein, nie. Wer beim Aktivieren
  schon in der Lane ist, bleibt drin — das Gate wirkt nur auf neue Joins.
  Rauswerfen kann dich nur der Owner per Kick/Ban.
- **Woher kommt der Rang im Lane-Namen?** Aus deiner 🎯-Rang-Präferenz; ohne
  gesetzte Präferenz aus deiner verifizierten Rang-Rolle. Bei Casual ist er
  reine Info, bei Ranked ist er zusätzlich die Vorgabe fürs Rang-Gate.
- **Street Brawl bleibt bei 4 Plätzen.** Das ist so gewollt — Street-Brawl-Lanes
  ignorieren Rang-Gates und haben immer maximal 4 Slots (solange die Lane im
  Street-Brawl-Modus bleibt; siehe Abschnitt 4).
- **Region ändert sich „von selbst" für neue Lanes.** Richtig — die Region hängt
  an dir als Owner und wird auf jede neue Lane übernommen.
- **Lane ist weg.** Lanes verschwinden automatisch, sobald niemand mehr drin ist.

---

*Technische Referenz für Entwickler steht im Dev-Abschnitt von
[`voice-features.md`](voice-features.md).*
