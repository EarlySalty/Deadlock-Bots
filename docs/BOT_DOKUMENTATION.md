# Deadlock Discord Bot - Dokumentation für Spieler

## Übersicht

Der Deadlock Bot bietet dir verschiedene Tools, um dein Spielerlebnis auf dem Server zu verbessern. Hier erfährst du, was die einzelnen Funktionen bringen und wie du sie nutzt.

---

## 🎮 Coaching

**Was bringt's dir?**
Kostenlose 1-on-1 Sessions mit erfahrenen Coaches aus der Community. Du beschreibst was du verbessern willst – ein Coach meldet sich und hilft dir gezielt weiter.

**Wie nutzt du es?**
1. Gehe in den Channel <#1494373349944459355>
2. Klicke auf den Button und fülle das Formular aus (Rang, Hero, deine Baustellen)
3. Ein Coach meldet sich bei dir im Coaching-Chat

**Wichtig:**
- Kommunikation läuft ausschließlich im Coaching-Chat – keine DMs oder Freundschaftsanfragen an Coaches
- Du kannst beliebig oft anfragen
- Nach dem Coaching bekommst du eine Feedback-Anfrage – bitte ehrlich ausfüllen, das hilft dem Team

---

## ⚖️ Team Balancer

**Was bringt's dir?**
Ganz einfach: Du willst ein faires 6v6 Match spielen? Der Team Balancer teilt alle Spieler automatisch in zwei möglichst ausgeglichene Teams ein - basierend auf Rang.

**Wie nutzt du es?**
1. Sei mit anderen Spielern in einem Voice-Channel
2. Nutze den Befehl:
   - `!balance auto` - Zeigt nur die Team-Aufteilung (ohne etwas zu verschieben)
   - `!balance start` - Erstellt zwei Team-Voice-Channels und bewegt alle Spieler automatisch
   - `!balance status` - Zeigt deinen aktuellen Rang-Status

**Beispiele:**
```
!balance auto          # Vorschau der Teams
!balance start        # Teams erstellen & starten
!balance status @User  # Rang von User prüfen
```

---

## 🏆 Turnier-Anmeldung

**Was bringt's dir?**
Du willst bei Turnieren mitmachen? Hier meldes du dich an - solo oder mit deinem Team.

**Voraussetzungen:**
- Du brauchst die **Turnier-Rolle** (erhältst du vom Team)
- Dein **Steam-Konto muss mit Discord verknüpft sein** (via `/account_verknüpfen`)

**Wie nutzt du es?**
1. Nutze den Slash Command: `/turnier`
2. Klicke auf "Anmelden"
3. Wähle ob du Solo oder mit einem Team teilnehmen willst
4. Dein Rang wird automatisch aus deiner Steam-Verknüpfung gezogen

**Wichtig:**
- Die Anmeldung ist nur während des aktiven Anmeldezeitraums möglich
- Du kannst dich jederzeit mit `/turnier` wieder abmelden

---

## 📹 Clip-Einreichung

**Was bringt's dir?**
Du hast ein geiles Gameplay gefilmt? Reiche es ein! Die besten Clips werden regelmäßig gesammelt und können für Community-Inhalte verwendet werden.

**Was musst du beachten??**
- **Mindestqualität: 1080p**
- Du brauchst die Erlaubnis des Erstellers, falls nicht du selbst
- Credit/Username muss angegeben werden

**Wie nutzt du es?**
1. Finde den Clip-Submit-Kanal
2. Klicke auf "Clip einsenden"
3. Gib den Link (YouTube/Twitch/etc.) ein
4. Füge Credit/Username hinzu
5. Optional: Kontext/Info zum Clip

**Zeitfenster:**
- Jede Woche von Sonntag 00:00 bis Samstag 23:00 (Berlin Zeit)
- Außerhalb des Fensters kannst du nicht einsenden

---

## 🆕 Neue Spieler Lanes

**Was bringt's dir?**
Du bist neu und willst mit anderen Neulingen spielen? Die Neuen-Spieler-Lanes sind speziell dafür da, damit Anfänger zusammenfinden können.

**Wie funktioniert's?**
- Die Voice-Channel werden automatisch erstellt und verwaltet
- Sobald genug Spieler da sind, können Teams gebildet werden
- Perfekt zum Üben und Kennenlernen

---

## 🔗 Steam-Verknüpfung

**Was bringt's dir?**
Dein Steam-Konto wird mit Discord verknüpft - dadurch:
- Automatische Rang-Rolle basierend auf deinem Deadlock-Rang
- Teilnahme an Turnieren
- Verifizierung als echter Spieler

**Wie nutzt du es?**
1. Nutze `/account_verknüpfen` auf dem Server
2. Folge den Anweisungen im Browser
3. Nach Verknüpfung: Freundschaftsanfrage an den Bot senden
4. Sobald du Freund bist und verifiziert: Du erhältst automatisch die Steam-Verified Rolle

---

## 📊 Anonymer Feedback-Kanal

**Was bringt's dir?**
Du hast Ideen, was man auf dem Server verbessern könnte? Oder willst Feedback zu deinem Spielerlebnis geben? Hier kannst du anonym abstimmen und Vorschläge machen.

**Wie nutzt du es?**
1. Finde den Feedback-Hub im Server
2. Klicke auf "Anonymes Feedback senden"
3. Beantworte die Fragen im Formular
4. Dein Feedback wird anonym an das Team weitergeleitet

---

## 🎯 Kurzübersicht aller Befehle

| Befehl | Funktion |
|--------|----------|
| `/turnier` | Turnier-Anmeldung öffnen |
| `!balance auto` | Team-Vorschau ohne Move |
| `!balance start` | Teams erstellen & starten |
| `!balance status` | Deinen Rang-Status anzeigen |
| `/account_verknüpfen` | Steam mit Discord verknüpfen |

---

## ❓ Hilfe

Wenn du Fragen hast oder Hilfe brauchst:
- Nutze `/faq` für häufige Fragen
- Ping das Team in **#support** oder **#的一般问题**
- Bei technischen Problemen: Schreib's in **#bug-reports**
