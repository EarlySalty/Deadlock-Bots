# Steam-Integration

## Worum geht es?
Die Steam-Integration verbindet dein Discord-Konto mit deinem Steam-Account. Dadurch kann der Server deinen Deadlock-Rang sauber erkennen, dir nach bestätigter Steam-Freundschaft die Steam-Verified-Rolle geben und Beta-Invites automatisiert verschicken.

Zusätzlich gibt es einen Rank-Lookup für verknüpfte Accounts und einen Hintergrund-Sync, der Freundschaften, Verifizierung und Rangdaten aktuell hält.

## Wie nutze ich das?
Für die normale Verknüpfung nutzt du `/account_verknüpfen` oder das Steam-Panel in <#1398021105339334666>. Das Panel hat drei Buttons: `Steam verknüpfen`, `Freundescode eingeben` und `📊 Rang prüfen`. Der Flow läuft in drei Schritten:

1. Du meldest dich kurz über Steam an. Das läuft über Steam OpenID, also ohne Passwort-Eingabe beim Bot. Der persönliche Link aus dem Panel ist 15 Minuten gültig und einmalig verwendbar.
2. Du gibst deinen Steam-Freundescode ein (Button oder Formular-Fallback).
3. Der Bot schickt dir eine Steam-Freundschaftsanfrage — erst wenn du die annimmst, gilt dein Link als vollständig verifiziert.

Alternativ gibt es den Browser-Weg über die `/link`-Webseiten (mit Discord-Login und Steam-OpenID) — das ist derselbe Flow ohne Discord-Panel.

Nützliche Befehle drumherum: `/steam links` zeigt, was gespeichert ist; `/steam setprimary` setzt deinen Hauptaccount; `/steam whoami` löst deine gespeicherte ID auf; `/steam unlink` entfernt eine Verknüpfung.

Für den Rank gibt es zwei Wege mit unterschiedlicher Wirkung:
- Button `📊 Rang prüfen` im Panel oder `/steam_rank`: reine Abfrage, ändert nichts an deinen Rollen.
- `/checkrank`: fragt ab UND synchronisiert deine Rang-Rollen (speichert den Stand).
Die automatischen Rang-Rollen kommen aus der Steam-Profilkarte deines verknüpften Accounts; es gibt verifizierte und unverifizierte Rollensets. Der Hintergrund-Sync hält das aktuell (Rang standardmäßig stündlich, Freunde alle 6 Stunden).

Für einen Beta-Invite: nett in <#1464736918951432222> fragen und deinen Steam-Freundescode dazu posten — ein Community-Mitglied lädt dich ein. Der Steam-Bot kann Invites zusätzlich automatisiert verschicken (Sicherheitsnetz). Details dazu stehen in „Onboarding und Invites".

## Kosten / Premium
Die Steam-Verknüpfung, Friend-Sync, Rank-Erkennung und der Beta-Invite selbst sind kostenlos.

Im Beta-Invite-Flow gibt es zusätzlich eine optionale Ko-fi-Unterstützung. Das ist kein Pflichtkauf für den Invite. Wenn du unterstützen willst, bekommst du als Dankeschön 30 Tage Community-Extras; der Ko-fi-Token muss dabei als Nachricht bei Ko-fi angegeben werden (Zuordnung wartet max. 24 Stunden).

## Was passiert technisch (kurz)?
Die Verknüpfung nutzt Steam OpenID und speichert danach die technische Steam-ID zusammen mit deinem Discord-Konto. Eine bestätigte Steam-Freundschaft ist die zweite Freigabe: Erst dann werden Verifizierung und rangbasierte Features aktiv.

Ein Hintergrunddienst synchronisiert regelmäßig die Steam-Freundesliste des Bot-Accounts (alle 6 Stunden; im Invite-Ticket zusätzlich ein schneller Poll). Sobald dein Account als Freund bestätigt ist, werden Verifizierung und Rollen nachgezogen. Der Rank-Sync liest die Deadlock-Profilkarte und aktualisiert Rangdaten im Hintergrund.

## Grenzen & häufige Fragen
- Nur ein OpenID-Login reicht nicht. Freundescode + bestätigte Steam-Freundschaft mit dem Bot gehören dazu.
- **„Limited User"**: Steam blockiert Playtest-Invites, wenn auf dem Account noch keine ~5 $ ausgegeben wurden. Das ist eine Valve-Regel, keine Bot-Prüfung — sie schlägt erst beim Invite-Versuch zu. Beim automatisierten Weg wiederholt der Bot den Versuch von selbst (alle 3 Tage, bis zu 5-mal).
- Steam kann außerdem verlangen, dass die Freundschaft mit dem Einladenden mindestens 30 Tage besteht — auch das erkennt der Bot beim automatisierten Weg und versucht es später erneut.
- Rank-Lookups und automatische Rangrollen funktionieren nur sauber mit verifiziertem Steam-Freund-Link.
- Wenn du mehrere Steam-Accounts verknüpft hast, zählt für viele Features dein gesetzter Primäraccount.
- Ein Invite erscheint nicht immer sofort — nach Erfolg kann es 1–2 Tage dauern, bis die Einladung bei Steam sichtbar ist.
- Vorübergehende Steam- oder Game-Coordinator-Probleme können einzelne Invite-Versuche verzögern. In solchen Fällen hilft meist ein späterer Retry.

## Für Devs
Rust live: Panel/Commands in `Deadlock-Bots/rust/crates/dl-bridges/src/steam.rs`; Link-, Rank- und Invite-Flows im Steam-Bot (`steam-flows/src/link.rs`, `rank.rs`, `friend_sync.rs`, `betainvite/`), Web-Routen in `steam-web/src/routes/link.rs`. Verifizierung = OpenID-Link + bestätigte Bot-Freundschaft. Rank- und Invite-Funktionen greifen auf dieselbe Steam-Bridge zu, damit nicht mehrere Dienste direkt gegen Steam sprechen.
