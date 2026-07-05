# W3.4c — Sprachkanal-UX: Reorder + Router-VC-Panels + Voreinstellungen ohne Lane

Owner-Entscheide (2026-07-05):
- Panels zusätzlich in den TEXT-CHAT DES ROUTER-VC (1513468587195633674), NICHT in erstellte Lanes.
- Ankündigung = Lesart A: nur erklären, wie das Design funktioniert (kein Auto-Route-Feature), mit Kanal-Link.
- Voreinstellungen (Name/Limit/Rang) müssen konfigurierbar sein, AUCH wenn man in keiner Voice-Lane ist; beim Lane-Erstellen automatisch angewandt.

## Slice 1 — Reorder 📖sprachkanal-verwalten (1513468476365209670)

Ziel: VIER V2-Messages in fester Reihenfolge (Repost über Publisher-Muster, KV-IDs):
1. **Anleitung** (neu): Kurztext + Button „📖 Ausführliche Anleitung" (custom_id `voice:guide:detail`) → ephemeral V2 (Mechanik = regelwerk:show:*).
2. **Mitspieler finden**: bestehendes LFG-Panel (lfg:create:start + lfg:watch:start) — nur Repost an Position 2.
3. **Lane erstellen**: Modus-Buttons (router_spawn_casual/ranked/street_brawl) + Kurzhinweis (Ranked = verifizierter Rang).
4. **Lane verwalten** (unten): tv_*-Buttons (Owner/Umbenennen/Limit/Modus/Presets/Kick/Ban/Unban) + „⚙️ Voreinstellungen"-Button (Slice 3).

Heutiges Router-Panel (msg 1522516664279760999) = EIN Block mit allem → wird in Messages 1/3/4 aufgeteilt. Bestehende custom_ids NICHT umbenennen (Handler unangetastet). Die bestehenden Detailtexte („Lane erstellen", „Deine Lane gehört dir", „Die Buttons im Detail") wandern aus dem Panel in die ephemere Detail-Anleitung.

## Slice 2 — Router-VC-Text-Chat (1513468587195633674)

Dieselben 4 Messages in gleicher Reihenfolge in den eingebauten Text-Chat des ➕Deadlock-Router-VC posten (gleicher Publisher, zweites Ziel; KV-Keys pro Kanal getrennt). Nach dem Posten anpinnen (Best-effort). tv_*-Klicks ohne Lane liefern die bestehende „du bist in keiner Lane"-Antwort — okay; „⚙️ Voreinstellungen" funktioniert überall.

## Slice 3 — Default-Voreinstellungen inkl. MODUS + Auto-Route (Owner-Update 2026-07-05)

Ist: voice.tempvoice_presets (user_id, category_id, name, member_limit, min_rank …); tv_preset_save braucht Lane (engine.lane_preset_snapshot). Router-VC-Join zeigt heute nur Panel-Hinweis, kein Auto-Move.
Soll:
- **Default-Preset pro User** (Upsert, EIN Datensatz „standard"): MODUS (casual/ranked/street_brawl) + Lane-Name + Limit (+ optional Rang-Bereich).
- **Erste Modus-Wahl speichert den Default**: Klickt ein User ohne Default einen router_spawn_*-Button, wird dieser Modus als Default gespeichert (+ ephemerer Hinweis „Als dein Standard gespeichert — ändern über ⚙️").
- **Auto-Route beim Router-VC-Join** (1513468587195633674): Hat der User einen Default → sofort Lane im Default-Modus erstellen/routen (Ranked-Gate weiter prüfen; bei fehlendem Verify → ephemerer Hinweis + kein Move, Default bleibt). Ohne Default → heutiges Verhalten (erst wählen).
- **⚙️ Voreinstellungen** (custom_id `tv_prefs_open`, Button in Anleitungs- + Verwalten-Panel): ephemere Ansicht ZEIGT aktuelle Defaults (Modus/Name/Limit/Rang oder „noch keiner gesetzt") + Buttons: Modus ändern (3 Buttons), Name+Limit ändern (Modal), Rang ändern (Selects), Default löschen. Wenn User gerade in eigener Lane sitzt: zusätzlich „Auf aktuelle Lane anwenden".
- tv_preset_save/load (Lane-Snapshots) bleiben unverändert. Privacy-Vertrag (privacy.rs) für neue Spalten/Datensätze prüfen.

## Slice 6 — 30s-Cooldown bei Lane-Erstellung entfernen (Owner 2026-07-05: „unnötig")

Der sichtbare 30-Sekunden-Cooldown beim Lane-Erstellen (router_spawn_* / VC-Join-Spawn)
fliegt komplett raus — kein Warte-Hinweis mehr. Der Own-Lane-Block (eine eigene Lane
gleichzeitig) bleibt. ERSATZ als stiller Flood-Guard: erst ab >4 Erstellungen
innerhalb von 60s pro User greift eine kurze Bremse (ephemerer Hinweis) — normale
Nutzung erreicht das nie, schützt nur vor Join/Leave-Bounce-Spam auf die Discord-API.
Bestehende Cooldown-Tests entsprechend umbauen (nicht löschen: Flood-Guard testen).

## Slice 5 — LFG-Panel als gepinnter Forum-Post (Owner: „maybe")

Im Forum 1522769149208821881 einen Bot-Post „So findest du Mitspieler" erstellen (V2, gleiche Buttons lfg:create:start + lfg:watch:start), Thread anpinnen. VORHER verifizieren, dass die lfg:*-Handler aus Thread-Kontext korrekt funktionieren (Interaction-Channel = Thread, Gesuch muss trotzdem als neuer Forum-Post landen, nicht im Thread). Publisher-Muster (KV, idempotent). Wenn Pin-Limit/Verhalten des Forums das Konzept bricht → als Befund melden statt Workaround erfinden.

## Slice 4 — Ankündigung (postet Claude nach Owner-GO + Live-Beweis)

Version 2 nach Owner-Feedback („Kurz und ehrlich" raus; Modus-Speicherung erklären):

**🎙️ Voice-Lanes — einmal sauber erklärt**
Weil's öfter Fragen gab: Wenn du in ➕Deadlock Router joinst, wirst du beim ersten Mal nicht sofort verschoben — du wählst erst, was du spielen willst (Casual, Ranked oder Street Brawl). Genau diese Wahl merkt sich der Bot als deinen Standard: Ab dann landest du beim Router-Join sofort in deiner eigenen Lane, ohne Extra-Klick.
Deinen Standard (Modus, Lane-Name, Limit) kannst du jederzeit ansehen und ändern — über ⚙️ Voreinstellungen in <#1513468476365209670>, auch wenn du gerade in keiner Voice bist. Und deine laufende Lane managst du direkt über das Verwalten-Panel — im Kanal oder direkt im Router-Chat.
Wenn dir Mitspieler fehlen: „Mitspieler finden" postet dein Gesuch in <#1522769149208821881> — mit 🔔 wirst du benachrichtigt, sobald ein passendes reinkommt.

## Finale Texte (Claude; byte-genau übernehmen)

[Anleitung Titel] **🎙️ So funktionieren unsere Voice-Lanes**
[Anleitung Body]
Bei uns joinst du nicht in volle Kanäle — du bekommst deine eigene Lane:
1. Join ➕Deadlock Router oder klick unten einen Modus-Button. Beim **ersten Mal** wählst du, was du spielen willst (Casual, Ranked, Street Brawl) — der Bot merkt sich das als deinen Standard.
2. **Ab dann** geht's beim Router-Join sofort in deine eigene Lane — ohne Extra-Klick. Standard ändern? ⚙️ Voreinstellungen, jederzeit, auch ohne in einer Voice zu sein.
3. Deine Lane gehört dir: Name, Limit, Kick — alles über „Lane verwalten" steuerbar.
4. Mitspieler findest du über „Mitspieler finden" — oder lass dich mit 🔔 benachrichtigen, sobald ein passendes Gesuch reinkommt.
[Anleitung Button] 📖 Ausführliche Anleitung
[Detail-Anleitung zusätzlich:] Abschnitt ⚙️ erklärt: erste Modus-Wahl = Standard; Ansehen/Ändern/Löschen im ⚙️-Menü; Name & Limit wirken auf jede neue Lane; „Auf aktuelle Lane anwenden", wenn du in deiner Lane sitzt.

[Detail-Anleitung (ephemeral); Gliederung — Bestandstexte aus dem heutigen Router-Panel wiederverwenden:]
**📖 Voice-Lanes im Detail**
**Lane erstellen** — (Bestandstext „Lane erstellen" + Satz zu Router-VC-Join: erst Modus wählen, dann Verschiebung) Ranked geht nur mit verifiziertem Rang — Steam verknüpfen in <#1398021105339334666>.
**Deine Lane gehört dir** — (Bestandstext)
**Die Buttons im Detail** — (Bestandstext tv_*-Erklärungen)
**⚙️ Voreinstellungen** — Name, Limit (und Rang-Bereich) jederzeit festlegen, auch ohne in einer Lane zu sein — wird bei jeder neuen Lane automatisch angewandt. 💾 Presets sichern zusätzlich den Stand einer laufenden Lane.
**Mitspieler finden** — Gesuch per Klick (Modus, Rang, Wann), erscheint in <#1522769149208821881>; 🔔 benachrichtigt dich bei passenden Gesuchen.

[Ankündigungs-ENTWURF → Owner-Review]
**🎙️ Voice-Lanes — einmal sauber erklärt**
Kurz und ehrlich, weil's öfter Fragen gab: Wenn du in ➕Deadlock Router joinst, wirst du **nicht sofort verschoben** — das ist Absicht. Du wählst erst deinen Modus (Casual, Ranked oder Street Brawl), dann erstellt dir der Bot deine eigene Lane und zieht dich automatisch rüber. Keine vollen Kanäle, keine Fremden in deiner Runde — deine Lane, deine Regeln.
Neu aufgeräumt: In <#1513468476365209670> findest du jetzt alles in klarer Reihenfolge — Anleitung, Mitspieler finden, Lane erstellen, Lane verwalten. Dieselben Panels stehen auch direkt im Router-Chat.
Auch neu: Über ⚙️ Voreinstellungen legst du Name und Limit deiner Lane **im Voraus** fest — auch wenn du gerade in keiner Voice bist. Jede neue Lane startet dann direkt mit deinen Einstellungen.
Und wenn dir Mitspieler fehlen: „Mitspieler finden" postet dein Gesuch in <#1522769149208821881> — mit 🔔 wirst du benachrichtigt, sobald ein passendes reinkommt.
