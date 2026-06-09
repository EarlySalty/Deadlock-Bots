## #57 — Lobby-Finder: Channel-Name "unbekannt" gefixt + Trigger-Rauschen reduziert

**Ausgangslage:** Der Lobby-Finder zeigte in seinen Antworten statt der echten "Neue Spieler Lane" den Platzhalter "# unbekannt". Außerdem feuerte er auf Nachrichten wie "suche leute zum zocken, schreib mir bitte priv" — eine Ankündigung, keine Lobby-Anfrage.

**Geändert:** Die Channel-ID für die Neue-Spieler-Lane war in `lfg.py` noch auf den alten Wert von vor der adaptiven-Lane-Einführung gesetzt (`1465839460485697556`), während der tatsächliche Anchor-Channel eine andere ID hat (`1470126503252721845`). ID korrigiert. Zusätzlich wurde ein Negativ-Signal ganz an den Anfang der Intent-Erkennung gelegt: Nachrichten, die gleichzeitig "schreib/schreibt/meldet" und "priv/privat" enthalten, werden jetzt sofort übersprungen — der User will selbst koordinieren, nicht vom Bot weitergeleitet werden.

**Jetzt:** Channel-Mention zeigt den richtigen Namen. Nachrichten mit explizitem "schreib priv"-Muster lösen keine Lobby-Vorschläge mehr aus.

## #56 — Dependency-Updates: aiohttp 3.14.0 + protobufjs-Lücken geschlossen

**Ausgangslage:** Dependabot meldete 11 offene Sicherheitslücken: 4× aiohttp (medium, Cross-Origin-Redirect + Deserialisierung unsicherer Daten) und 7× protobufjs (bis high, u.a. Code-Injection via Byte-Felder, Prototype-Pollution, DoS durch rekursive Expansion).

**Geändert:** aiohttp von 3.13.5 auf 3.14.0 angehoben (`requirements.txt` + `.venv`). protobufjs im steam_presence-Modul via `npm install` + `npm audit fix` aktualisiert: Top-Level-Package von 8.0.2 auf 8.4.2, zusätzlich die verschachtelten Kopien in den transitiven Abhängigkeiten (`steam-user`, `steam-session`, `steam-appticket`) auf gepatchte Versionen gebracht.

**Ergebnis:** `npm audit` meldet 0 Vulnerabilities. Bot läuft weiter ohne Verhaltensänderung.

## #55 — Coaching: Freigeben-Button-Fix + echte Umlaute

**Ausgangslage:** Der „Freigeben"-Button im Coaching-Request-Embed hatte zwei Bugs. Erstens: Wenn eine Anfrage bereits für alle offen war (kein reservierter Coach), bekamen Nicht-Server-Owner die Meldung „Nur der reservierte Coach oder ein Admin kann freigeben" — statt der korrekten Info „bereits für alle offen". Die eigentliche Info-Meldung war für alle außer dem Guild-Owner toter Code. Zweitens: Als „Admin" galten ausschließlich der Guild-Owner und eine hardcoded User-ID — Discord-User mit `Administrator`-Permission wurden geblockt, obwohl die Fehlermeldung selbst von „ein Admin" spricht.

**Geändert:** Logik-Reihenfolge korrigiert: Der „bereits offen"-Check läuft jetzt zuerst, bevor der Permission-Check greift. Außerdem wurde `guild_permissions.administrator` in den Admin-Check aufgenommen, konsistent mit dem restlichen Codebase-Pattern (`website_invite_cog._is_owner_or_admin`). Zusätzlich: Alle 14 Pseudo-Umlaute (`fuer`, `ue`, `oe` etc.) in der Datei durch echte Umlaute ersetzt.

**Wie's funktioniert:** Klickt jemand Freigeben auf einer bereits offenen Anfrage, sieht er jetzt die korrekte Info — unabhängig von seiner Rolle. Klickt ein Discord-Admin (mit `Administrator`-Flag) auf eine reservierte Anfrage, kann er sie freigeben. Der assigned Coach kann weiterhin immer freigeben.

## #54 — Coaching: Session-Abschluss wird jetzt an die Website gespiegelt

**Ausgangslage:** Wenn ein Admin mit `/coaching-session-beenden` eine Session beendete, landete das nur in der Bot-DB. Die Coaching-Website zeigte weiterhin die Session als aktiv — weil der Bot nie den `/platform/sync`-Endpunkt aufrief.

**Geändert:** Der `coaching_survey`-Cog sendet nach dem Session-Update jetzt automatisch einen HTTP-Call an das Website-Backend (`/platform/sync` auf Port 8772). Payload enthält Request-ID, Discord-IDs, Coach-Name und `session_status: "completed"` + die Bot-Session-UUID.

**Wie's funktioniert:** Der interne Token (derselbe wie im restlichen Stack) authentifiziert den Call. Das Backend macht ein Upsert: Session wird auf `completed` gesetzt, der Timestamp landet in `completed_at`. Damit sehen Spieler auf `/coaching/me` ihre Session-Historie korrekt — inklusive dem Abschluss. Der Coaching-Flow in Discord bleibt unverändert.

## #53 — Master-Broker: Discord-Invites auf Anfrage erstellen

**Hintergrund:** Der Twitch-Bot läuft als separater Prozess und hat keinen eigenen Discord-Guild-Mitgliedsstatus — er kann daher keine Invites direkt über die Discord-API erstellen. Gelöst über den Master-Broker, der bereits für andere Discord-Aktionen (Nachrichten senden, Channels anlegen, Rollen vergeben) als interner Proxy fungiert.

**Geändert:** Neuer Endpunkt `POST /internal/master/v1/discord/create-invite` im Master-Broker. Nimmt `channel_id` und optionalen `reason`, löst den Channel über den Haupt-Bot auf und ruft `channel.create_invite(max_age=0, max_uses=0, unique=True)` auf. Antwort enthält `invite_url`, `code`, `channel_id` und `guild_id`. Auth und Idempotency-Handling laufen identisch zu den anderen Broker-Endpunkten.

## #52 — Fix: TempVoice-Interface lädt wieder korrekt

**Problem:** Durch die neuen Router-Buttons (Umbenennen + Modus wechseln) kam es beim Bot-Start zu einem Fehler, der das komplette TempVoice-Modul am Laden hinderte — alle Lane-Funktionen waren damit ausgefallen.

**Ursache:** Der Lurker-Button hat intern row 3 als Standard, was die Reihe bereits auf 4 Items brachte. Die zwei neuen Buttons auf row 3 machten 6 Items — Discord erlaubt maximal 5 pro Reihe.

**Geändert:** Die neuen Router-Buttons landen jetzt auf row 4, die in der Standard-Lane-Ansicht bisher leer war.

## #51 — Router: Smart-Routing bevorzugt bekannte Mitspieler

**Problem:** Der Router hat beim Suchen einer freien Lane einfach die erste passende genommen — ohne Rücksicht darauf, ob da jemand drin sitzt, mit dem man schon oft gespielt hat.

**Geändert:** Beim Smart-Routing (Auto-Join an) wird jetzt zuerst geprüft, ob in einer der passenden Lanes ein bekannter Mitspieler sitzt. Erst wenn keine solche Lane gefunden wird, greift der bisherige Fallback (erste freie Lane mit Platz).

**Wie's funktioniert:** Beim Join holt der Router die Top-20-Co-Player aus der bestehenden `user_co_players`-Tabelle (die trackt, wie viele gemeinsame Voice-Sessions zwei User hatten). Dann werden alle passenden Lanes durchsucht: Liegt die User-ID eines Mitglieds in dieser Co-Player-Liste, wird diese Lane priorisiert. Findet sich kein Co-Player, landet man wie gewohnt in der nächsten freien Lane.

## #50 — TempVoice: Neues Router-System mit Spielmodus-Wahl

**Problem:** Wer in einen Sprachkanal wollte, landete in einem von drei fixen Staging-Kanälen (Casual, Ranked, Street Brawl) — Modus-Wahl durch das Betreten des richtigen Kanals. Das war unflexibel: keine Möglichkeit den Modus zu ändern, keine smarte Verteilung in laufende Lanes, keine einheitliche Einstiegsstelle.

**Geändert:** Ein einzelner Router-Sprachkanal ersetzt die drei Eingänge für alle, die über diesen neuen Weg kommen wollen. Die alten Staging-Kanäle laufen unverändert weiter. Dazu gibt es einen Interface-Textkanal mit einer persistenten UI aus drei Modus-Buttons (Casual, Ranked, Street Brawl) und einem Auto-Join-Toggle.

**Wie's funktioniert:**

- **Modus wählen:** User klickt im Textkanal auf einen der drei Buttons — wird als Standard gespeichert und gilt für alle folgenden Joins.
- **Auto-Join aus (grau):** Beim Betreten des Router-VC wird sofort eine eigene private Lane im passenden Bereich erstellt.
- **Auto-Join an (grün):** Smart Routing — der Bot sucht eine bestehende Lane mit weniger als 6 Personen und demselben Modus. Findet er keine, wird eine neue erstellt.
- **Ranked-Gate:** Wer Ranked wählt, braucht einen verifizierten Rang (Steam-Verknüpfung). Ohne Rang kommt eine DM mit Link zum Info-Kanal, kein Move.
- **Neue Spieler:** Werden weiterhin automatisch in die New-Player-Lane umgeleitet, bevor das Modus-Routing greift — bestehende Logik unverändert.
- **Modus-Wechsel einer laufenden Lane:** Lane-Owner können ihren Kanal nachträglich auf einen anderen Modus (inkl. Off Topic) umstellen — der Kanal zieht in die passende Discord-Kategorie um. Dazu gibt es neue Buttons im Lane-Control-Interface.
- **Off-Topic-Lanes** landen in der Router-Kategorie, genau wie der bestehende permanente Off-Topic-Kanal.

## #49 — Coaching wird jetzt fair auf alle Coaches verteilt

**Problem:** Eingehende Coaching-Anfragen liefen nach dem „Wer zuerst klickt"-Prinzip — der erste Coach, der auf „Claimen" drückte, bekam die Session. In der Praxis griff dadurch meist immer derselbe Coach zu, während andere kaum drankamen; eine echte Verteilung gab es nicht. Zusätzlich lief im Hintergrund noch ein altes, längst totes Zweit-System mit fest eingetragener Coach-Liste mit, das die Lage nur unübersichtlicher machte.

**Geändert:** Neue Anfragen werden jetzt automatisch **fair** einem Coach zugewiesen und **24 Stunden exklusiv für ihn reserviert**. Es gibt einen **Freigeben-Button**, und nach Ablauf der 24 Stunden öffnet sich die Anfrage automatisch für alle. Wer Coach ist, ergibt sich dynamisch aus der Coach-Rolle — keine fest hinterlegte Namensliste mehr. Das alte parallele Claim-System wurde komplett entfernt.

**Wie's funktioniert:** Sobald eine Anfrage analysiert und gepostet wird, wählt der Bot aus allen Trägern der Coach-Rolle (der Server-Inhaber ist bewusst ausgenommen) den aus, der **am längsten nicht mehr an der Reihe war** — bei Gleichstand den mit den **wenigsten laufenden Sessions**. Dieser Coach steht sichtbar im Embed und hat 24 Stunden exklusiv Zeit zu übernehmen; andere Coaches sehen die Anfrage, können sie in diesem Fenster aber nicht claimen. Drückt der reservierte Coach (oder ein Admin) auf „Freigeben", oder laufen die 24 Stunden ab, wird die Anfrage für alle Coaches geöffnet — ein Hintergrund-Check im Minutentakt erledigt das Ablaufen automatisch und aktualisiert die Nachricht. Lässt sich gerade kein passender Coach ermitteln, ist die Anfrage sofort für alle offen, genau wie vorher.

**Betroffen:** Vor allem die Coaches (faire Reihenfolge statt Windhundprinzip). Für anfragende Spieler bleibt der Ablauf gleich — nur ist schneller klar, wer sich kümmert.

## #48 — Security Guard: MiniMax-Label erscheint jetzt nachträglich im Mod-Embed

**Problem:** Das MiniMax-Bild-Urteil lief blockierend *vor* dem Mod-Alert — wenn MiniMax länger brauchte (oder per Timeout abbrach bei 8 s), stand im ersten Post „nicht verfügbar" und der Bot wartete die ganze Zeit, bevor er überhaupt Timeout und Löschung ausführte.

**Geändert:** Alert wird sofort gepostet (mit „⏳ wird ermittelt…" als Platzhalter), Timeout + Löschung laufen direkt danach. MiniMax läuft als Hintergrund-Task mit 45 s Spielraum und aktualisiert das Embed per Edit, sobald das Urteil vorliegt.

**Wie's funktioniert:** `asyncio.create_task` startet den AI-Check entkoppelt vom Hauptpfad. Sobald MiniMax antwortet, sucht die Task das Feld „MiniMax-Einschätzung" im bereits geposteten Embed und überschreibt es. Schlägt das Edit fehl (Nachricht gelöscht, Bot-Rechte weg), wird das still ignoriert.

## #47 — Security Guard: Timeout-Bug gefixt, AI-Scam-Erkennung repariert, öffentliche Scam-Meldung

**Problem:** Der Security Guard hat Scam-Accounts zwar erkannt und den Alert im Mod-Channel gepostet, aber keine einzige Aktion ausgeführt — kein Timeout, keine Nachrichtenlöschung. Der Bot hat den Mod-Alert sogar doppelt gepostet (der Case erschien zweimal), weil der Crash den State zurückgesetzt und beim nächsten Trigger erneut ausgelöst hat. Dazu hat die AI immer "kein Scam" zurückgegeben, obwohl MiniMax intern bereits 100 % Scam erkannt hatte.

**Was war kaputt:**
- py-cord 2.7.1 kennt den Parameter `timed_out_until`, nicht `communication_disabled_until` (discord.py-mainline-Name). Jeder Timeout-Aufruf ist mit einem `TypeError` abgestürzt — ungebated, bis zu `discord.client` hochgereicht, wo er stillschweigend weggeloggt wurde. Weder Timeout noch Nachrichtenlöschung (die danach kamen) liefen je.
- `max_output_tokens=120` beim Scam-Text-Check war zu niedrig für Thinking-Modelle: MiniMax M3 hat alle 120 Token für den internen Denkprozess verbraucht, kein JSON-Response überlebt — die AI hat daher immer `False` zurückgegeben, egal wie eindeutig der Scam war.

**Geändert:**
- Timeout-Aufruf auf `timed_out_until` umgestellt (überall, inkl. Timeout-Aufhebung per Button).
- `max_output_tokens` für Scam-Text-Check von 120 auf 1500 angehoben, damit Thinking + JSON beide reinpassen.
- Nach jeder automatischen Scam-Aktion (Ban oder Timeout) postet der Bot jetzt kurz in den betroffenen Channel(s): `🔒 Scam erkannt — Account wurde automatisch gebannt/gesperrt.` — so sehen User, die den Scam gesehen haben, dass das Mod-System reagiert hat.

**Wie's jetzt funktioniert:** Scam erkannt → Beweis in Mod-Channel spiegeln → Timeout setzen (funktioniert jetzt) → Nachrichten löschen → öffentliche Kurzmeldung im betroffenen Channel → DM an User. Der AI-Text-Check liefert jetzt bei eindeutigen Scams (wie Fake-MrBeast-Crypto oder USDT-Withdrawal-Screenshots) korrekt `is_scam: true` mit hoher Confidence.

## #46 — Master-Dashboard: Login hält jetzt 2 Wochen statt 6 Stunden

**Problem:** Die Anmeldung am Master-Dashboard galt nur 6 Stunden. Da der Twitch-Admin-Bereich auf dieser zentralen Sitzung aufsetzt, fiel man dort regelmäßig nach wenigen Stunden raus — das Dashboard wirkte „tot": Oberfläche lädt, aber jede Aktion läuft ins Leere, weil die Sitzung im Hintergrund abgelaufen war.

**Geändert:** Die Standard-Lebensdauer der Master-Dashboard-Sitzung von 6 Stunden auf 14 Tage angehoben.

**Wie's funktioniert:** Die Sitzung gilt jetzt zwei Wochen und verlängert sich bei jeder Nutzung automatisch (Sliding-Refresh) — wer regelmäßig reinschaut, bleibt praktisch dauerhaft angemeldet. Der Wert ist weiterhin per Umgebungsvariable überschreibbar; der neue Standard greift, solange nichts anderes gesetzt ist. Einmal anmelden reicht damit für zwei Wochen statt mehrmals täglich neu einloggen zu müssen.

**Betroffen:** Login-Komfort im Admin-/Dashboard-Bereich; für normale Discord-Nutzer nichts sichtbar.

## #45 — Crypto-Scam: Jetzt automatisch gelöscht + Ban-Button für Mods

**Problem:** Ein Einzel-Scam (eine Nachricht, ein Kanal) wurde vom Bot erkannt (Confidence 0,97), aber weder automatisch gelöscht noch direkt gesperrt — weil er in keine der Auto-Delete-Kategorien fiel. Der Moderator konnte anschließend auch nicht direkt über den Bot bannen, da "Accept" nur einen 24h-Timeout ausgelöst und keine Ban-Option angeboten hat.

**Geändert:** `scam` als vollständige Kategorie im KI-Moderator eingeführt: im System-Prompt beschrieben, in die erlaubten Kategorien aufgenommen und in die Auto-Delete-Liste eingetragen. Zusätzlich ist ein neuer **Ban-Button** in jede Moderations-Review-Nachricht eingebaut worden.

**Wie's jetzt funktioniert:** Stuft die KI eine Nachricht mit `category=scam` und `verdict=delete` bei Confidence ≥ 0,90 ein, wird sie sofort gelöscht und der User automatisch 24h stummgeschaltet — ohne Mod-Interaktion. Liegt die Confidence zwischen 0,78 und 0,90, landet ein Vorschlag im Review-Kanal mit drei Buttons: **Accept** (Nachricht löschen + Timeout), **Ban** (Nachricht löschen + permanenter Serverausschluss) und **Deny** (Fall ablehnen). Der Ban-Pfad greift auch bei allen anderen Kategorien — nicht nur bei Scam.

## #44 — Gekaperte Stamm-Accounts: Scam-Bilder werden gestoppt

**Problem:** Ein langjähriges, etabliertes Mitglied wurde gehackt und hat in Sekunden Krypto-/Casino-Scam-Screenshots (gefälschte Auszahlungs-„Beweise", ein Fake-Promi-Giveaway, eine Casino-Bonusseite) über fünf, sechs Kanäle gestreut. Der Bot hat nicht reagiert — aus zwei Gründen. Erstens: Das harte Durchgreifen aus #43 galt nur für neue Accounts unter 30 Tagen; ein gekapertes Alt-Mitglied fiel komplett durch dieses Raster. Zweitens: Der Bild-Check hing an einer KI, die die Bilder technisch gar nicht „sehen" kann — das hier eingesetzte Modell ist reiner Text. Die in #43 erwähnte Bild-Mitlesung lief faktisch ins Leere, das Bild-Urteil stand deshalb immer auf „0 %".

**Geändert:** Ein neuer Schnellpfad speziell gegen Account-Übernahmen, der für **alle** Mitglieder gilt — ausdrücklich auch alte, etablierte (nur Mods sind ausgenommen). Und die Bilder werden jetzt von einem echten Bild-Verstehen-Dienst gelesen statt vom blinden Text-Modell.

**Wie's funktioniert:** Postet ein Account innerhalb von **30 Sekunden Bilder in mindestens zwei verschiedenen Kanälen** — der typische Fingerabdruck einer übernommenen Identität, die Werbung streut — greift der Bot sofort durch, ohne auf ein KI-Urteil zu warten (deterministisch, deshalb auch unabhängig davon, ob die KI etwas erkennt). Reihenfolge: erst die Bilder als Beweis in den Mod-Kanal kopieren (solange die Links noch leben), dann den Account **24 Stunden stummschalten** und sämtliche dieser Nachrichten kanalübergreifend löschen. Der Betroffene bekommt eine DM mit dem Hinweis, sein Account sei womöglich gehackt — Passwort ändern, 2FA aktivieren, beim Mod-Team melden. Die Mods sehen eine Übersicht mit den Bildern und zwei Buttons: **Bann** (endgültig) oder **Stummschaltung aufheben** (Fehlalarm). Die 24 Stunden sind bewusst umkehrbar gehalten.

**Warum die KI jetzt wirklich mitliest:** Zusätzlich schickt der Bot ein repräsentatives Bild durch ein echtes Bild-Verstehen. Das liefert den Mods jetzt ein konkretes Urteil im Alarm — z. B. „Scam, 100 % — gefälschtes Promi-Konto bewirbt ein Krypto-Casino-Giveaway" — statt der bisherigen stummen „0 %", weil das alte Modell die Bilder nie sehen konnte. Wichtig: Dieses Urteil ist nur noch ein Hinweis fürs Mod-Team, kein Auslöser — die Quarantäne läuft auch dann, wenn die KI gerade nichts sagt.

## #43 — Spam-Schutz greift jetzt selbst durch

**Problem:** Wenn ein neuer Account (jünger als 30 Tage) in kurzer Zeit dieselbe Werbung oder dieselben Bilder über mehrere Kanäle streut — das klassische Spam-Wellen-Muster — hat der Bot das zwar erkannt, aber nur den Mods gemeldet und dann gewartet. Die Spam-Nachrichten blieben stehen, bis jemand von Hand eingriff. Erkennung lief, Reaktion nicht.

**Geändert:** Der Bot greift jetzt beim Muster selbst durch, statt auf eine KI-Bestätigung zu warten — die kam vorher nämlich oft gar nicht (siehe unten).

**Wie's funktioniert:** Schlägt das Muster an (ab 3 Nachrichten in 3 Kanälen, oder schon ab 2 Kanälen mit Bildern/Scam-Wörtern; Mods und etablierte Accounts ausgenommen), läuft der Reihe nach: erst die Bilder als Beweis in den Mod-Kanal kopieren (bevor die Links durchs Löschen tot sind), dann die Nachrichten löschen und den Account **1 Stunde stummschalten**. Die Mods bekommen eine Übersicht mit Texten, Bildern und zwei Buttons — **Bann** oder **Timeout aufheben**. Die Stunde ist bewusst kurz und umkehrbar, falls es doch ein harmloser Neuling war. Die KI prüft Text und Bilder weiterhin, aber nur noch als Hinweis fürs Mod-Team, nicht als Auslöser.

**Warum vorher „0 %" dastand:** Die Text-Prüfung lief gegen einen KI-Dienst, der auf diesem Bot gar nicht eingerichtet ist (Zugangsschlüssel fehlt) — es kam nie eine Antwort zurück, und „keine Antwort" wurde als 0 % angezeigt. Jetzt läuft die Prüfung über den Dienst, der hier wirklich konfiguriert ist und auch die Bilder mitliest. Dessen interne „Denk"-Abschnitte werden vor der Auswertung herausgefiltert, weil das Ergebnis sonst beim Einlesen zerbrach.

## #42 — Direkte Channel-ID für interne API-Posts

- Interne Bot-Posts können jetzt einen beliebigen Discord-Kanal direkt ansprechen (statt nur "all"/"twitch")
- Wird genutzt für Moderations-Alerts aus dem Twitch-Bot

## #41 — Changelog-Posts direkt in Discord

- Neue Changelogs landen jetzt automatisch als Discord-Embed im Dev-Update-Kanal
- Twitch-Bot-Änderungen können zusätzlich gezielt im Twitch-Bot-Kanal gepostet werden
- Admin-interne Änderungen werden dabei bewusst weggelassen – nur was User sehen sollen kommt rein
- Admins können Einträge auch per `/changelog post` manuell auslösen

## #40 — Steam-Verifikation: Watchdog gegen stille Disconnects

- Ein neuer automatischer Watchdog überwacht jetzt dauerhaft ob der Steam-Bot eingeloggt ist
- Fällt die Steam-Verbindung lautlos weg (wie heute ~16 Uhr), erkennt der Watchdog das innerhalb von 30 Sekunden und startet den Bot nach 2 Minuten automatisch neu
- Vorher konnte eine solche Verbindungsunterbrechung unbemerkt über eine Stunde anhalten, weshalb Steam-Verifikationen in diesem Zeitraum fehlschlugen
- Betroffene User können die Verknüpfung jetzt erneut starten – sie wird wieder funktionieren

## #39 — FAQ-Bot kennt jetzt fast alle Server-Themen

- Der FAQ-Bot kann jetzt deutlich mehr Fragen beantworten: TempVoice und Lanes, Coaching, Onboarding & Beta-Invite, Steam-Verknüpfung, Twitch-Dashboard inkl. Tarife, Turniere, Patchnotes-Bot und unsere Websites
- Antworten zu Preisen und kostenlosen vs. bezahlten Features sind jetzt verlässlich, weil die zugrundeliegende Doku konkret und aktuell ist
- Veraltete, sich überschneidende Doku wurde archiviert, damit der Bot keine widersprüchlichen Aussagen mehr mischt

## #38 — Coaching-Infos überarbeitet und bereinigt

- Coaching ist jetzt überall klar als kostenlos und ohne Limit beschrieben
- Hinweis auf Feedback nach dem Coaching eingefügt – User werden gebeten ehrlich zu antworten
- Technische Details (Rollen-Zuweisung etc.) aus den Nutzer-Infos entfernt
- KI-Analyse erstellt keine Priorität mehr, sondern direkt nutzbare Fokuspunkte
- Alle alten Referenzen auf das nicht mehr existierende Coaching-Modul aus den Docs entfernt

## #37 — Coaching-Dokumentation und bessere KI-Antworten

- Der FAQ-Bot weiß jetzt zuverlässig wo und wie Coaching beantragt wird und verweist direkt auf den richtigen Channel
- Neue Coaching-Dokumentation hinterlegt, damit der Bot konkrete Antworten geben kann
- Die KI-Analyse bei Coaching-Anfragen listet jetzt direkt die Fokuspunkte für den Coach – ohne Wiederholung der User-Angaben
- Veraltete Referenz auf den alten Coaching-Bot entfernt

## #36 — Rang-Präferenz für Chill-Lanes

- Neuer Button „🎯 Mein Rang" im TempVoice-Interface (für alle Lane-Typen sichtbar)
- Über ein Dropdown-Menü kannst du Haupt-Rang und Sub-Rang auswählen und speichern
- Wenn du eine Chill-Lane erstellst, wird der gewählte Rang automatisch als Kanalname verwendet
- Dein Sub-Rang bestimmt außerdem die Sortierposition deiner Lane in der Kategorie

## #35 — Leere Sprachkanäle werden jetzt zuverlässig gelöscht

- Leere Lanes in der Ranked-Kategorie werden jetzt korrekt automatisch entfernt
- Channels bleiben nach einem Bot-Neustart nicht mehr dauerhaft erhalten
- Namen von laufenden Lanes werden nach einem Neustart nicht mehr vom Bot überschrieben

## #34 — Tag-Filter-Bestätigung nur noch für dich sichtbar

- Nach dem Speichern des Tag-Filters wird die Bestätigung nur noch dir angezeigt, nicht mehr im Channel

## #33 — Bestimmter Channel bleibt immer ganz unten in der Kategorie

- Ein festgelegter Voice-Channel wird nach jedem Neu-Sortieren automatisch ans Ende der Kategorie verschoben
- Er bleibt immer unter allen anderen Lanes, egal welche Ränge neu erstellt werden

## #32 — Channel aus TempVoice-Verwaltung ausgenommen

- Ein bestimmter Voice-Channel wird vom Bot nicht mehr umbenannt, gelöscht oder verschoben
- Der Channel bleibt weiterhin über die Verwaltungsinterfaces anpassbar

## #31 — Austritts-Umfrage: verstehen, warum Leute gehen

- Wenn jemand den Server verlässt, bekommt er automatisch eine freundliche Nachricht mit einer kurzen Umfrage — die Fragen passen sich an, je nachdem ob jemand neu war oder schon länger aktiv dabei
- Nach der ersten Antwort kommt eine gezielte Nachfrage, damit klarer wird, was genau das Problem war
- Wer ausführlicher Feedback geben will (auch mit Bildern), bekommt einen Link zu einer Feedback-Seite auf der Website
- Neue Auswertung im Admin-Dashboard zeigt, aus welchen Gründen Leute gehen und wie oft geantwortet wird
- Gebannte Mitglieder werden von der Umfrage ausgenommen

## #30 — Lobby-Finder treffsicherer und übersichtlicher

- Passt wirklich eine Lobby zu deinem Rang, wird dir gezielt die eine vorgeschlagen statt einer langen Liste
- Rang-Schnitt einer Lobby zeigt jetzt den echten Rang-Namen (z. B. Oracle) statt kryptischer Kürzel
- Aufgeräumteres Layout mit mehr Abstand, klarem Beitritts-Hinweis und Warnung wenn eine Lobby fast voll ist
- Falsche "über deinem Rang"-Warnung bei Lobbys unter deinem Rang behoben
- Erkennt mehr Nachrichten wie "jemand am start?" oder "noch wer wach?" und sortiert auch unverifizierte Rang-Rollen sauber ein

## #29 — Ehrliche Member-Herkunft im Admin-Dashboard

- Kreis-Diagramm "Wo kommen unsere Member her?" zeigt jetzt einen eigenen Website-Bereich statt Website-Joins in "Persönlich" zu verstecken
- Historische Joins ohne klare Quelle werden nicht mehr als "Twitch" geschätzt, sondern ehrlich als "Unbekannt" markiert
- Pro Website-Subseite gibt es jetzt einen eigenen Discord-Invite-Code (Landing, Streamer, Mitspieler, Coaching, Helden, Guides)
- Hover über Twitch- oder Website-Stück zeigt die Aufschlüsselung pro Streamer bzw. Subseite
- `/website-invite-recreate` rotiert jetzt gezielt nur eine ausgewählte Subseite

## #28 — Alle GitHub Code-Scanning-Alerts behoben

- Sicherheitslücke in der Node.js-Abhängigkeit protobufjs durch Paket-Override geschlossen
- XSS-Gefahr in der Aktivitätsstatistik-Seite behoben (HTML-Escaping für API-Daten)
- Integrity-Attribut für externes Chart.js-CDN-Skript hinzugefügt
- YAML-Syntaxfehler im Auto-Merge-Workflow behoben
- Log-Injection-Schwachstelle im Master-Broker gefixt (sanitisierte Log-Werte)
- Über 50 Code-Quality-Hinweise bereinigt (leere Except-Blöcke, ungenutzte Variablen, Import-Stil)
- 23 bestätigte False Positives (URL-Redirection, Cookie-Injection, SQL-Queries) als solche markiert und geschlossen

## #27 — Dependabot Auto-Merge wieder voll funktionsfähig

- Fehlende Hilfsdatei ergänzt, die den Python-Security-Scanner blockiert hat
- Syntaxfehler in der Auto-Merge-Workflow-Datei behoben (Actionlint-Fehler)
- Alle drei Ursachen, die Dependabot-PRs vom automatischen Merge abgehalten haben, sind beseitigt

## #26 — GitHub Actions Minutenverbrauch stark reduziert

- Neun tägliche Workflows auf wöchentliche oder event-basierte Trigger umgestellt
- Dashboard DAST und Auth-Guardrails laufen jetzt wöchentlich statt täglich
- Security-Scans, Secret-Scanning und Incident-Automation brauchen keinen täglichen Lauf mehr
- Semgrep meldet Findings als Artifact statt den Build zu blockieren

## #25 — SecurityGuard: Kein Auto-Ban ohne AI-Bestätigung

- Jeder Burst-Trigger wird jetzt zuerst von der KI geprüft — kein Ban ohne AI-Bestätigung
- Wenn die KI nicht sicher genug ist, landet der Fall nur als Vorschlag im Mod-Channel (inkl. Ban-Button für manuelle Entscheidung)
- Verhindert Fehlbans bei neuen Accounts die zufällig in mehreren Channels aktiv sind
- Bilder werden ebenfalls AI-geprüft wenn Text alleine nicht ausreicht

## #24 — SecurityGuard: Ban-Logs persistent in DB gespeichert + Review-Channel aktiv

- Jeder automatische Ban/Timeout wird jetzt dauerhaft in der Datenbank gespeichert — Fälle bleiben auch nach Bot-Neustart nachvollziehbar
- Die gespeicherten Daten umfassen Grund, betroffene Channels, Nachrichten-Snippets und Anhang-Anzahl
- Auch KI-erkannte Scam-Fälle bei etablierten Accounts (Timeout-Proposals) werden persistiert
- Ein dedizierter Review-Channel zeigt ab sofort alle Incidents als Embed an

## #23 — KI-Moderationskontext deutlich verbessert

- Kontext-Nachrichten zeigen jetzt relative Zeitstempel (z.B. "[2min ago]") — die KI erkennt ob der Kontext frisch oder veraltet ist
- Nachrichten vom selben User der bewertet wird, sind mit ">>>" markiert — so sieht die KI das Verhaltensmuster des Users klar
- Reply-Chain: wenn jemand auf eine Nachricht antwortet, bekommt die KI die Original-Nachricht direkt mit — Reaktionen werden im richtigen Kontext bewertet
- System-Prompt erklärt jetzt explizit: Reaktion auf eine Provokation wird milder bewertet als die Provokation selbst

## #22 — KI-Moderationsfilter entschärft: weniger False Positives

- Schwellenwert für Moderationsvorschläge von 55% auf 78% Konfidenz angehoben
- Die KI holt jetzt nur noch 12 statt 25 Nachrichten Kontext — verhindert, dass Kontext-Rauschen aus dem Gaming-Channel die Bewertung verfälscht
- System-Prompt präzisiert: Gaming-Klischees (Nationalitäten), kurze Ein-Wort-Antworten und einmalige Beleidigungen werden nicht mehr gemeldet
- Wichtig: Wer schlechtes Verhalten meldet oder kommentiert, wird nicht mehr selbst geflaggt

## #21 — Bild-Scam-Erkennung: KI analysiert Screenshots in mehreren Channels

- Accounts die nur Bilder senden (z.B. gefälschte X-Posts mit Investitions-Gewinnen) werden jetzt erkannt
- Sobald jemand Bilder in 2+ verschiedenen Channels schickt, prüft die KI automatisch ob es Scam ist
- Junge Accounts (<30 Tage) werden bei bestätigtem Bild-Scam direkt gebannt
- Ältere Accounts bekommen einen 24h-Timeout + DM und Mods sehen den Fall im Mod-Channel
- Message-Verlauf wird jetzt für alle Accounts getrackt, nicht nur für neue

## #20 — Klare Grenze: jung = bis 30 Tage, etabliert = älter als 30 Tage

- Accounts bis 30 Tage alt werden bei erkanntem Scam direkt gebannt
- Accounts älter als 30 Tage (möglicherweise gehackt) bekommen stattdessen einen 24h-Timeout und eine DM
- Der KI-Scam-Check läuft jetzt für alle Accounts, nicht mehr nur für neue
- Mehrkanal-Burst-Erkennung bleibt weiterhin auf Accounts unter 30 Tagen begrenzt

## #19 — Etablierte Scam-Accounts werden sofort getimeoutet und per DM informiert

- Accounts die älter sind und möglicherweise gehackt wurden, werden nicht mehr nur gemeldet, sondern sofort für 24h stummgeschaltet
- Der betroffene User bekommt automatisch eine DM: Grund, Dauer, und der Hinweis sich beim Mod-Team zu melden sobald der Account wieder sicher ist
- Mods sehen im Mod-Channel trotzdem ein Info-Embed mit zwei Buttons: "Ban" (eskalieren) oder "Timeout aufheben" (falls False Positive)
- Kein manuelles Bestätigen mehr nötig — der Bot handelt sofort

## #18 — Scam-Erkennung mit KI und erweiterter Account-Prüfung

- Scam-Nachrichten werden jetzt auch von Accounts erkannt, die bis zu einem Monat alt sind (vorher nur 24 Stunden)
- Einzelne Nachrichten mit verdächtigen Inhalten werden per KI automatisch auf Scam geprüft — kein Multi-Channel-Spam mehr nötig
- Neue Accounts werden bei KI-bestätigtem Scam automatisch gebannt
- Ältere, etablierte Accounts (möglicherweise gehackt) werden nicht automatisch gebannt — stattdessen erscheint ein Mod-Vorschlag mit Ban-Button im Mod-Channel
- Das bisherige Join-Zeitfenster als Bedingung wurde entfernt

## #17 — TempVoice greift nicht mehr in fremde Voice-Kategorien ein

- Channels außerhalb der TempVoice-Kategorien (Chill, Comp, Street Brawl) werden nicht mehr umbenannt oder gelöscht
- Betrifft z.B. Custom Games oder andere manuelle Voice-Channels
- Verhindert, dass Channels mit Namen wie "Lane 1" in falschen Kategorien fälschlicherweise als TempVoice-Lane erkannt werden

## #16 — FAQ-Bot antwortet automatisch in neuen Tickets

- Wenn in der Support-Kategorie ein neues Ticket aufgemacht wird, analysiert der FAQ-Bot die erste Nachricht des Users
- Hat der Bot eine passende Antwort aus der Dokumentation, antwortet er direkt im Ticket
- Kann der Bot das Problem nicht lösen, schreibt er gar nichts – der Mensch übernimmt dann wie gewohnt
- Gilt für alle Channels die mit "ticket-" beginnen in der ❓Support-Kategorie

## #15 — FAQ-Bot erkennt Onboarding- und Invite-Probleme automatisch

- FAQ-Bot gibt jetzt bei "kein Invite" oder "kann nicht herunterladen" sofort eine Schritt-für-Schritt-Anleitung aus
- Checkliste: Onboarding abgeschlossen? Richtige Option gewählt? Rollen gesetzt? /betainvite verwendet? Steam-Kauf vorhanden?
- Bot weist freundlich aber klar darauf hin wenn das Onboarding-Lesen das Problem gelöst hätte
- Neue Dokumentationsdatei mit dem vollständigen Invite-Ablauf für die Bot-Wissensbasis

## #14 — Tracking-Invite und Auswertung für Website-Joins

- Bot legt automatisch einen permanenten Tracking-Invite an und merkt sich den Code in der DB
- Neuer Slash-Befehl `/website-invite` zeigt Status, Code und bisherige Nutzungen
- `/website-invite-recreate` erzeugt bei Bedarf einen neuen Code (z.B. wenn jemand den alten löscht)
- `/join-quellen [tage]` aggregiert die Member-Joins der letzten N Tage nach Quelle (Website, Vanity, Twitch-Streamer, persönliche Einladungen)
- Channel-Default ist der Welcome-Channel, kann via `WEBSITE_INVITE_CHANNEL_ID` Env-Var überschrieben werden

## #13 — TempVoice Sweep löscht keine Staging-Channels mehr

- Staging-Channels sind jetzt gegen den automatischen Sweep geschützt
- Hintergrund: Der Bot hatte den alten Chill Lanes Staging-Channel selbst gelöscht, weil dessen Name einem Lane-Muster entsprach

## #12 — Chill Lanes Staging Channel auf neuen Channel aktualisiert

- Der Staging-Channel für Chill Lanes wurde nach dem Löschen des alten Channels auf den neuen Channel umgestellt
- Alle betroffenen Stellen im Code wurden aktualisiert: TempVoice, LFG und User-Retention-Links

## #11 — Steam-Verknüpfung: Link wird jetzt immer frisch beim Klick erstellt

- Der „Via Steam verknüpfen"-Button im Onboarding generiert den Login-Link jetzt erst beim Klicken – nicht mehr beim Laden der Nachricht
- Dadurch können Links nicht mehr ablaufen, bevor jemand draufklickt
- Das Problem „invalid Launch" bei der Steam-Verifizierung ist damit behoben

## #10 — Vollständige Security-Fortress hinzugefügt

- Neuer Security-Scan läuft täglich: prüft Workflow-Integrität, findet Secrets im Code, scannt Python auf Sicherheitslücken und bekannte CVEs in Dependencies
- Alle Workflow-Dateien sind jetzt auf genaue Commit-Hashes gepinnt — kein Supply-Chain-Angriff über gemutete Action-Tags möglich
- JavaScript-Abhängigkeiten werden auf bekannte Schwachstellen geprüft
- Security-Fortress ergänzt den bestehenden Deep-Scan sinnvoll, ohne Laufzeit zu verschwenden

## #9 — Dependabot-PRs werden jetzt automatisch gemerged + CI-Laufzeiten halbiert

- Dependabot-PRs werden ab jetzt automatisch approved und direkt gemerged (nicht mehr blockiert durch Lint oder DAST)
- Lint und DAST-Scans überspringen Dependabot-PRs, da sie nur Dependency-Dateien ändern — kein Sicherheitsverlust
- Security-Scans laufen weiterhin täglich; Container/IaC/Supply-Chain scannen jetzt wöchentlich statt täglich
- Security-Incident-Automation läuft jetzt täglich statt alle 6 Stunden — 75 % weniger Runs
- Dependency-Review hat keinen sinnlosen Tages-Schedule mehr (läuft weiterhin auf jedem PR)

## #8 — CI-Artifacts werden nach 30 Tagen automatisch gelöscht

- Alle automatisch erzeugten CI-Berichte (Security-Scans, Performance-Reports, Logs) werden ab jetzt nach 30 Tagen automatisch von GitHub entfernt
- Verhindert, dass sich der GitHub-Actions-Speicher dauerhaft volläuft

## #7 — Steam-Bot startet wieder und updated Ränge

- Steam-Bot lief seit dem 27. April nicht mehr — FAs annehmen/senden und Rang-Updates funktionierten nicht
- Ursache: Drei kombinierte Bugs verhinderten den Start (falscher Pfad auf Linux, fehlende Env-Variablen für den Node-Prozess, veraltete native Module)
- Der Bot läuft jetzt stabil und verarbeitet wieder Rang-Checks und Freundschaftsanfragen

## #6 — DB-Pfad fest im Code, kein Datenverlust mehr bei Neustarts

- Der Bot nutzt jetzt immer `data/deadlock.sqlite3` direkt im Repo — egal welche Umgebungsvariablen gesetzt sind oder nicht
- Davor: Nach der Token-Rotation fehlte die `DEADLOCK_DB_PATH`-Variable → Bot startete mit einer leeren Fallback-DB → alle User wirkten wie Neulinge (kein Voice-Verlauf, keine Steam-Links)
- Die fehlenden 2.5 Tage Daten (276 Voice-Sessions, Steam-Links, Nudge-Status etc.) wurden in die Haupt-DB zurückgespielt

## #5 — Steam-Nudge-DM geht nicht mehr mehrfach an denselben User

- Wenn die ursprüngliche Nudge-DM gelöscht wurde (z. B. vom User selbst), schickt der Bot keine zweite DM mehr — die Nachricht ist weg, die Benachrichtigung bleibt trotzdem gesetzt
- Fehlschläge beim Speichern des „bereits benachrichtigt"-Flags werden jetzt im Log sichtbar, statt still ignoriert zu werden

## #4 — Tag-System: bessere Sortierung in Voice-Lanes und LFG

- Du kannst dir jetzt selbst zwei Tags setzen: deinen Lieblings-Ton (Banter-OK oder Ragebaiter-Free) und optional eine Altersangabe (25+ oder U25)
- Setzen geht entweder direkt im Onboarding nach dem Server-Join oder jederzeit per `/meine-tags`
- Voice-Lane-Owner können in ihrer Lane einen 🛡️ Tag-Filter setzen, damit nur Leute mit passendem Ton oder Alter joinen können
- Wer wiederholt Ragebait fährt, bekommt automatisch einen Ragebaiter-Mod-Tag (14 Tage), der ihn aus Ragebaiter-Free-Lanes raushält — Mods können den Tag jederzeit anpassen
- LFG-Suche kann jetzt auch nach Tags filtern, damit Mitspieler besser zur eigenen Stimmung passen

## #3 — Tierlist-Backend: WR-Daten alle 8 Stunden automatisch

- Neuer Service liefert die Hero-Tierliste der Website mit Live-Winrates pro Skill-Bucket
- Daten werden alle 8 Stunden automatisch aktualisiert, immer auf Basis des aktuellen Patches
- Drei Skill-Buckets verfügbar: All, Phantom+, Eternus
- Admin-Endpunkte für Beschreibungen, Streamer-Listen und Schwellen — Login über bestehenden Discord-Flow
- Build-Voting (👍 / 👎) mit Rate-Limit pro Browser

## #2 — Voice Feedback geht nicht mehr an bestehende User nach Bot-Neustart

- Nutzer, die beim Neustart bereits im Voice-Call saßen, bekommen kein fälschliches „erstes Mal"-Feedback mehr
- Prüfung erweitert: beide Tabellen (voice_stats und voice_session_log) werden gecheckt, nicht nur eine
- Feedback-Retry bei DMs-deaktiviert läuft jetzt nicht mehr ewig: der ursprüngliche Zeitstempel bleibt erhalten und fällt nach 72 Stunden aus dem Fenster

## #1 — Sicherheitslücke: Bot-API nicht mehr von außen erreichbar

- Der interne Statistik-Server (Port 8768) war versehentlich von außen direkt erreichbar
- Jetzt lauscht er nur noch auf localhost — externer Zugriff ohne Caddy nicht mehr möglich
