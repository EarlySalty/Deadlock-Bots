# Deadlock Bots – Spieler-Dokumentation

---

## 🎤 TEMPVOICE – Dein eigener Voice-Kanal

**Was?** Betrittst du einen (+)-Staging-Channel, wird automatisch ein neuer Kanal für dich erstellt. Du bist Lane-Owner und verwaltest ihn selbst.

**Was bringt's?**
- Kein "Kanal ist voll" oder "jemand Fremder stört"
- Dein Rang wird im Channel-Namen angezeigt (Ranked)
- Komplett automatisch – kein Admin nötig
- Channel löscht sich selbst wenn alle raus sind

**So nutzt du's:**
1. Irgendeinen (+)-Channel im Bereich Ranked, Chill, Neue Spieler oder Street Brawl betreten
2. Dein eigener Channel wird automatisch erstellt

**Lane-Management (wenn du in deiner Lane bist):**

| Was du willst | Wie |
|---|---|
| Spieler-Limit setzen | Button `🎚️ Limit` → Modal (0-99) |
| Duo / Trio machen | Button `Duo Call` / `Trio Call` |
| Nur-DE-Speaker erlauben | Button `🇩🇪 DE` |
| Rang-Filter setzen | Buttons `①` + `②` (Haupt-Rang + Sub-Rang) |
| Person kicken | Button `👢 Kick` (Owner/Mods) |
| Person bannen | Button `🚫 Ban` |
| Ban aufheben | Button `♻️ Unban` |
| Als Lurker rein (stumm, kein Slot) | Button `👻 Lurker` |
| Setup speichern/laden | Buttons `💾` / `📁` (nur Ranked) |

**Lane weg? Einfach rausgehen.** Wird automatisch aufgeräumt.

---

## 🔍 SPIELERSUCHE – Mitspieler finden

**Was?** Der Bot scannt automatisch Nachrichten im LFG-Kanal und schlägt dir passende Spieler und offene Lobbys vor.

**Was bringt's?**
- Du musst nicht manuell suchen wer online ist
- Vorschläge basieren auf Rang, Steam-Status und Aktivität
- Neue Spieler (Initiate–Arcanist) werden automatisch in Neue-Spieler-Lanes geroutet
- Co-Spieler die du oft hast werden priorisiert

**So nutzt du's:**
1. Gehe in den **#lfm** oder **#spieler-suche** Kanal
2. Schreibe einfach was du suchst, z.B.:
   - "suche +2 für Ranked"
   - "wer bock auf Chill?"
   - "lfm für Street Brawl"
3. Der Bot erkennt automatisch deine Intention und postet ein Embed mit passenden Vorschlägen

**Was du siehst:** Offene Lobbys, Steam-Status der Spieler (🟢 in Lobby / 🎮 im Match), passende Mitspieler nach Rang.

---

## 🔗 STEAM-VERKNÜPFUNG

**Was?** Verknüpfe deinen Steam-Account mit dem Bot für die Verified-Rolle.

**Was bringt's?**
- Dein Rang wird in Turnieren korrekt angezeigt
- Andere Spieler sehen deinen Steam-Status in Lanes (in Lobby / im Match)
- Spielersuche funktioniert richtig (du wirst nach Skill eingeordnet)
- Du bekommst die Verified-Rolle automatisch

**So nutzt du's:**
1. Schreib dem Bot eine DM: `/account_verknuepfen`
2. Klick auf "Steam Account verknüpfen"
3. Freundschaftsanfrage an Steam-Bot **820142646** senden
4. Sobald die Freundschaft steht → Automatic Verified-Rolle

**Wichtig:** Steam-Profil auf "Freunde können meinen Status sehen" stellen.

---

## 🏆 TURNIERE

**Was?** Melde dich für Turniere an – solo oder als Team. Dein Rang wird automatisch aus deiner Steam-Verknüpfung gelesen.

**Was bringt's?**
- Schnelle Anmeldung ohne Google Formulare
- Automatischer Rank-Check (kein "ich bin aber Archon"-Stress)
- Öffentliche Webseite zeigt allen den Anmeldestatus

**So nutzt du's:**
1. `/turnier` slashen
2. Klick auf **"Anmelden"** → solo oder Team wählen
3. Klick auf **"Mein Status"** um zu sehen wo du stehst
4. Vor dem Turnier: Link im Ticket öffnen um Anmeldungen zu checken

**Voraussetzung:** Tournament-Rolle + Steam verknüpft.

---

## 🎬 CLIPS EINSENDEN

**Was?** Reiche Gameplay-Clips für die wöchentlichen Content-Videos ein – mit Credit-Trail.

**Was bringt's?**
- Deine Clips können in Videos verwendet werden
- Klarer Prozess: Link + Credit → kein Chaos
- Strukturiertes Format

**So nutzt du's:**
1. Im Channel **#🎬-clips** → Button **"Clip einsenden"** klicken
2. Bestätigen dass du Rechte am Clip hast
3. Modal ausfüllen: Link, Credit/Username, optionale Info
4. **Wichtig:** Mindestens 1080p, wöchentliches Zeitfenster (Sonntag–Samstag)

---

## 🐛 BUG MELDEN

**Was?** Strukturiertes Ticket-System mit KI-Unterstützung. Die KI analysiert dein Problem sofort und versucht direkt zu helfen.

**Was bringt's?**
- Schneller Support ohne Wartezeit
- KI kann oft direkt helfen (Cogs neu laden, etc.)
- Kategorisiert → richtige Leute sehen es
- Persönlicher Channel, nur du und Admins

**So nutzt du's:**
1. `/ticket` slashen
2. Kategorie wählen:
   - Steam-Verifizierung
   - Beta-Invite
   - Bot-Command
   - Build-Publishing
   - AI-Features
   - User-Management
   - Sonstiges
3. Kurzbeschreibung + Details im Modal
4. Bot erstellt privaten Channel → KI antwortet automatisch

---

## 💬 ANONYMES FEEDBACK

**Was?** Sende Feedback an die Admins – komplett anonym. Admins sehen nicht wer was geschrieben hat.

**Was bringt's?**
- Ehrliches Feedback ohne Konsequenzen
- Strukturiertes Format macht es für Admins nützlich

**So nutzt du's:**
1. Im Channel **#💬-feedback** → Button **"Anonymes Feedback senden"** klicken
2. 5 Felder ausfüllen:
   - Spielerlebnis
   - Server-Nutzung
   - Verbesserungsvorschläge
   - Wünsche
   - Weitere Mitteilungen
3. Absenden – fertig

---

## 📊 VOICE STATS

**Was?** Trackt wie lange du in Voice-Kanälen bist und zeigt dir Statistiken + das Server-Ranking.

**So nutzt du's:**

```
!vstats
```
Zeigt: Gesamtzeit, Punkte, Live-Session

```
!vleaderboard
```
oder kurz:
```
!vlb
```
Server-weites Voice-Ranking

```
!rrang info @User
```
Zeigt Rang-Info eines Users (welche Rolle + Wert)

---

## 🆕 NEUE SPIELER – Automatisches Lane-Routing

**Was?** Neue-Spieler-Lanes werden automatisch erstellt wenn genug Leute im Anchor-Channel sind.

**Was bringt's?**
- Dynamisch:数量的 passt sich an die Spieleranzahl an
- Nie zu viele / zu wenige Lanes
- Leere Extra-Lanes werden automatisch gelöscht

**So nutzt du's:**
1. Gehe in den **"🆕Neue Spieler Lane"** Anchor-Channel
2. Bei 6+ Leuten → automatisch Lane 2 erstellt
3. Bei 12+ → Lane 3, etc.
4. Einfach normal beitreten – alles andere passiert automatisch

---

## 🔒 DATENSCHUTZ (DSGVO)

**Was?** Volle Kontrolle über deine Daten – Export als JSON oder vollständige Löschung.

**So nutzt du's:**

```
/datenschutz
```
→ "Daten herunterladen" für JSON-Export aller gespeicherten Daten

```
/datenschutz
```
→ Erst "Bestätigen", dann "Endgültig löschen" für vollständige Löschung

```
/datenschutz-optin
```
Speicherung wieder aktivieren

---

## Commands auf einen Blick

| Command | Was es tut |
|---|---|
| `/turnier` | Turnier-Anmeldung |
| `/account_verknuepfen` | Steam verknüpfen |
| `/ticket` | Bug/Ticket erstellen |
| `/datenschutz` | Daten-Export/Löschung |
| `/datenschutz-optin` | Speicherung aktivieren |
| `!vstats` | Eigene Voice-Stats |
| `!vleaderboard` / `!vlb` | Server-Voice-Ranking |
| `!rrang info [@User]` | Rang-Info |
