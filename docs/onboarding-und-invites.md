# Onboarding und Invites

## Worum geht es?
Der Server fuhrt neue Mitglieder gezielt durch Regeln, Rollen und die wichtigsten Bereiche. Gleichzeitig hangt der Zugang zu manchen Bereichen, vor allem zum Beta-Invite-Flow, direkt davon ab, dass das Onboarding sauber abgeschlossen wurde.

## Wie nutze ich das?
Starte im `#regelwerk` uber den Button `Hier starten`. Der Bot legt dir dafur bevorzugt einen eigenen Onboarding-Thread an und fuhrt dich durch die wichtigsten Schritte: Regeln, Voice-Lanes, Mitspieler finden, Coaching, optionale Voice-Praeferenzen und die Steam-Verknupfung. Wenn du streamst, kannst du dabei auch direkt das Streamer-Setup in DMs starten.

Wichtig fur neue Mitglieder ohne Deadlock-Zugang: Im Discord-Onboarding musst du die passende Option fur Invite oder Betazugang auswahlen, damit `#beta-zugang` sichtbar wird. Danach solltest du in `#customize` beziehungsweise bei den Rollen-Auswahlen noch die relevanten Server-Rollen setzen, weil sonst weitere Bereiche fehlen konnen.

Wenn du den Beta-Zugang brauchst, gehst du anschliessend in `#beta-zugang` und nutzt dort `/betainvite`. Fur diesen Flow braucht dein Steam-Account eine echte Kaufhistorie von mindestens 5 Euro. Reines Wallet-Aufladen oder reine Free-to-Play-Nutzung reicht dafur nicht. Die eigentliche Steam-Prufung und der Payment-Teil gehoren zur Steam-Integration; fur dich als User ist nur wichtig: Onboarding fertig, Channel sichtbar, `/betainvite` im richtigen Channel, 5-Euro-Voraussetzung erfullt.

Zusatzlich gibt es eine DM-Ebene: Manche Hilfen, etwa fur Steam-Linking, Beta-Fragen oder Streamer-Setup, konnen dir auch in einer Bot-DM erklaert werden. Aus Usersicht merkt man davon vor allem, dass der Bot an passenden Stellen automatisch nachfragt und dir personalisierte nachste Schritte zeigt.

## Kosten / Premium
Der normale Server-Onboarding-Flow ist kostenlos. Beim Beta-Invite kann ein externer Payment-Flow Teil der Steam-Integration sein; fur den User-Flow ist vor allem die 5-Euro-Kaufhistorie relevant.

## Was passiert technisch (kurz)?
Das Regelwerk-Panel startet einen privaten Onboarding-Thread, notfalls einen offentlichen Fallback-Thread. Der statische Flow zeigt dir feste Schritte, speichert einzelne Entscheidungen wie Voice-Tags und wartet bei der Steam-Verknupfung auf eine erfolgreiche Verifikation. Erganzend kann der Bot dir im Onboarding oder in DMs kurze Zusatzfragen stellen und daraus eine passendere Server-Tour ableiten.

## Grenzen & häufige Fragen
- Wenn `#beta-zugang` fehlt, liegt das fast immer daran, dass das Onboarding nicht vollstandig abgeschlossen wurde oder die falsche Option gewahlt wurde.
- Rollen-Auswahl nach dem Onboarding ist kein Bonus-Schritt. Ohne passende Rollen fehlen weiter Kanale.
- `/betainvite` funktioniert nur im richtigen Channel und nur, wenn die Steam-Voraussetzung erfullt ist.
- Die Steam-Mechanik selbst wird separat behandelt. Als Stichwort fur Folgefragen gilt: Steam-Integration.
- Wenn deine DMs geschlossen sind, konnen Teile wie Streamer-Setup oder Hilfen in DMs scheitern.
- Die automatische Nachfrage im Onboarding hilft beim Einstieg, ersetzt aber keine Moderation und keinen Support-Fall.

## Für Devs (knapp)
- Cogs: `cogs/rules_channel.py`, `cogs/onboarding.py`, `cogs/ai_onboarding.py`, `cogs/welcome_dm/dm_main.py`, `cogs/welcome_dm/dm_assistant.py`, `cogs/website_invite_cog.py`
- Abhangigkeiten: `StaticOnboarding`, Welcome-DM-Views, Steam-Linking, Website-Invite-Codes, optional Streamer-DM-Flow
- Wichtige DB-Tabellen: `kv_store`, `user_privacy`, `member_events`; Invite-Codes liegen im Namespace `website_invites`
