# Steam-Integration

## Worum geht es?
Die Steam-Integration verbindet dein Discord-Konto mit deinem Steam-Account. Dadurch kann der Server deinen Deadlock-Rang sauber erkennen, dir nach bestätigter Steam-Freundschaft die Steam-Verified-Rolle geben und Beta-Invites automatisiert verschicken.

Zusätzlich gibt es einen Rank-Lookup für verknüpfte Accounts und einen Hintergrund-Sync, der Freundschaften, Verifizierung und Rangdaten aktuell hält.

## Wie nutze ich das?
Für die normale Verknüpfung nutzt du `/account_verknüpfen` oder das Steam-Link-Panel. Der Flow läuft in zwei Schritten:

1. Du meldest dich kurz über Steam an. Das läuft über Steam OpenID, also ohne Passwort-Eingabe beim Bot.
2. Danach musst du die Steam-Freundschaft mit dem Bot bestätigen. Erst dann gilt dein Link als vollständig verifiziert.

Wenn du mehrere Steam-Accounts hinterlegt hast, kannst du mit `/steam links` prüfen, was gespeichert ist, und mit `/steam setprimary` deinen Hauptaccount setzen.

Für den Rank-Lookup gibt es zwei Wege. Entweder nutzt du den Button `Rank Check` im Steam-Panel oder den Command `/steam_rank`. Standardmäßig fragt der Bot deinen eigenen verknüpften Account ab. Optional kann auch gezielt nach SteamID oder Account-ID gesucht werden.

Für einen Beta-Invite gilt der User-Pfad:
1. Onboarding vollständig abschließen.
2. In der Kanal- und Rollen-Auswahl die passenden Rollen wählen.
3. In `#beta-zugang` `/betainvite` ausführen.
4. Falls dein Steam-Account noch nicht verknüpft oder noch nicht als Freund bestätigt ist, führt dich der Bot durch die fehlenden Schritte.
5. Nach erfolgreicher Prüfung verschickt der Bot den Invite an deinen Steam-Account.

Wichtig: Dein Steam-Konto braucht die übliche Steam-Voraussetzung für Einladungen. Praktisch heißt das: Der Account muss mindestens 5 Euro echte Kaufhistorie haben. Reines Wallet-Aufladen oder Free-to-Play reicht dafür normalerweise nicht.

## Kosten / Premium
Die Steam-Verknüpfung, Friend-Sync, Rank-Erkennung und der Beta-Invite selbst sind kostenlos.

Im Beta-Invite-Flow gibt es zusätzlich eine optionale Ko-fi-Unterstützung. Das ist kein Pflichtkauf für den Invite. Im Bot ist kein fester Preis hinterlegt. Wenn du unterstützen willst, bekommst du als Dankeschön 30 Tage Community-Extras beziehungsweise die Supporter-Vorteile.

## Was passiert technisch (kurz)?
Die Verknüpfung nutzt Steam OpenID und speichert danach die technische Steam-ID zusammen mit deinem Discord-Konto. Eine bestätigte Steam-Freundschaft ist die zweite Freigabe: Erst dann werden Verifizierung und rangbasierte Features aktiv.

Ein Hintergrunddienst synchronisiert regelmäßig die Steam-Freundesliste des Bot-Accounts. Sobald dein Account dort als Freund bestätigt auftaucht, werden Verifizierung und Rollen nachgezogen. Der Rank-Lookup fragt Deadlock-Profildaten über die Steam-Bridge ab und aktualisiert die Rangdaten automatisch im Hintergrund.

Beim Beta-Invite legt der Bot einen Ticket-Flow an, prüft Steam-Link und Freundschaft, sendet dann die Einladung über Steam und merkt sich den Status. Optional erkannte Ko-fi-Zahlungen aktivieren zusätzlich zeitlich begrenzte Supporter-Extras.

## Grenzen & häufige Fragen
- Nur ein OpenID-Login reicht nicht. Die Steam-Freundschaft mit dem Bot muss ebenfalls bestätigt sein.
- Wenn dein Steam-Account als eingeschränkt gilt, kann Steam Einladungen blockieren.
- Rank-Lookups und automatische Rangrollen funktionieren nur sauber mit verifiziertem Steam-Freund-Link.
- Wenn du mehrere Steam-Accounts verknüpft hast, zählt für viele Features dein gesetzter Primäraccount.
- Ein Invite erscheint nicht immer sofort in deiner Bibliothek. Prüfe im Zweifel später noch einmal die Steam-Playtest-Einladungen.
- Vorübergehende Steam- oder Game-Coordinator-Probleme können einzelne Invite-Versuche verzögern. In solchen Fällen hilft meist ein späterer Retry.

## Für Devs
Der Flow hängt an einem OpenID/OAuth-Linkdienst, einer Steam-Task-Queue und mehreren periodischen Sync-Schleifen. Verifizierung basiert fachlich auf `verified=1` plus bestätigter Bot-Freundschaft. Rank- und Invite-Funktionen greifen auf dieselbe Steam-Bridge zu, damit nicht mehrere Dienste direkt gegen Steam sprechen.
