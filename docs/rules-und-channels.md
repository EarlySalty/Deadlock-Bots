# Rules und Channels

## Worum geht es?
Das Regelwerk ist nicht nur eine Textseite, sondern der Einstieg in den ganzen Server. Von dort aus startet dein Onboarding, und viele der wichtigsten Channels sind so aufgebaut, dass sie direkt mit Bot-Features zusammenarbeiten.

## Wie nutze ich das?
Lies zuerst <#1315684135175716975> und starte dort mit `Hier starten` dein Onboarding. Der Bot legt dir dafür einen privaten Onboarding-Thread an und führt dich durch Regeln, Voice-Lanes und die wichtigsten Bereiche. Nach dem Discord-Member-Screening startet das Onboarding auch automatisch.

Für Mitspieler und Voice ist die Grundstruktur relativ klar:
<#1376335502919335936> ist für die Textsuche nach Mitspielern.
<#1439564934592729161> ist das Panel, über das du eigene Lanes öffnest und verwaltest.
<#1398021105339334666> ist der Ort für die Steam-Verknüpfung und deine echte Rang-Rolle (Panel mit `Steam verknüpfen`, `Freundescode eingeben`, `📊 Rang prüfen`).

Für Support und Orientierung sind diese Kanäle die wichtigsten:
<#1459628609705738539> für Support-Fälle und Moderationsanliegen.
<#1465404160005378129> für ehrliches offenes Feedback.
<#1494373349944459355> für kostenlose Coaching-Anfragen (führt zur Website).
<#1464736918951432222> ist die offene Invite-Lounge: nett fragen + Steam-Freundescode posten, ein Community-Mitglied lädt dich ein. Der Bot passt mit auf und erinnert, wenn der Code fehlt.

Dazu kommen die Content- und Community-Bereiche:
<#1326973956825284628> für Patch-Zusammenfassungen.
<#1425215762460835931> für Highlight-Einsendungen.
<#1304169815505637458> für Live-Hinweise aus dem Streamer-Bereich.
<#1412411665713987635> und der Voice-<#1512624055814062090> für Custom Games.

Rollen bekommst du an mehreren Stellen automatisch: über das Onboarding (Voice-Tags, später änderbar per `/meine-tags`), über Reaction-Role-Panels (Reaktion = Rolle, Reaktion entfernen kann sie wieder wegnehmen) und über die Steam-Verknüpfung (echte Rang-Rolle, hält sich selbst aktuell).

Auch bei den Voice-Kategorien gibt es sichtbare Unterschiede: öffentliche Öffnen-Channels für Ranked/Competitive, Spaß, Street Brawl und Neue-Spieler-Lanes, dazu eine eigene Coaching-Voice-Kategorie, in der Coaching-Sessions erkannt und abgeschlossen werden. Bei Ranked-Lanes steht der Rang im Kanalnamen (Basis: Rang-Anker der Lane); der Zutritt ist auf ein Rang-Fenster um diesen Anker begrenzt.

Deine Daten gehören dir: `/datenschutz` bietet Daten-Export und endgültiges Löschen als Self-Service, `/datenschutz-optin` reaktiviert nach einem Opt-out. Erinnerungs-DMs lassen sich per `/retention-optout` abbestellen.

## Kosten / Premium
kostenlos

## Was passiert technisch (kurz)?
Das Regelwerk-Panel ist eine persistente Nachricht mit Start-Button (Admin-seitig per `/publish_rules_panel` gesetzt). Beim Start erzeugt der Bot einen privaten Thread im Regelkanal; klappt das nicht, bekommst du eine Fehlermeldung mit Hinweis — es gibt keinen öffentlichen Fallback-Thread. Das Onboarding ist schrittbasiert und speichert optionale Präferenzen wie Voice-Ton oder Altersgruppe.

## Grenzen & häufige Fragen
- Wenn dir bestimmte Kanäle fehlen, ist fast immer das Onboarding oder eine Rollen-Vergabe (Reaction Roles) nicht komplett.
- <#1315684135175716975> ist nicht nur Info-Text. Wer den Start-Button ignoriert, verpasst oft genau die Hinweise, die später im Ticket landen.
- Support, Coaching, LFG und Build-Fragen haben bewusst getrennte Orte. Das macht Antworten schneller und sauberer.
- Der Deadlock-Invite hängt NICHT an einer Onboarding-Auswahl und braucht keinen Befehl — nette Frage + Freundescode in <#1464736918951432222> reicht.
- Einige Voice-Bereiche sind rein zweckgebunden, zum Beispiel die Coaching-Voices. Sie sind nicht dasselbe wie normale LFG-Lanes.
- Der FAQ-Bot kann dir zwar sagen, wohin du musst, aber er schaltet keine Rollen oder Kanäle selbst frei.

## Kanal-Register (IDs sind stabil, Namen können sich ändern)

In allen Dokus werden Kanäle als `<#ID>` referenziert — Discord löst das immer zum aktuellen Namen auf. Diese Tabelle übersetzt die IDs (Namen = Stand 2026-07-03; die Umbenennungen auf regelwerk/deadlock-rang/deadlock-invite/server-support kommen mit der nächsten Struktur-Welle):

| Kanal-Mention | Name (Stand heute) | Zweck |
|---|---|---|
| <#1315684135175716975> | ⚖️hier-starten-regelwerk | Regeln + Onboarding-Start |
| <#1464736918951432222> | 🗝️beta-zugang | offene Invite-Lounge |
| <#1398021105339334666> | 🏆rang-auswahl | Steam-Verknüpfung + Rang-Rolle |
| <#1376335502919335936> | spieler-suche | LFG-Textsuche |
| <#1439564934592729161> | sprach-kanal-verwalten | TempVoice-Panel |
| <#1459628609705738539> | ticket-eröffnen | Support-Tickets |
| <#1483136301271355532> | ❤️lag-kompensator | Technik-Probleme mit dem Server/Bot |
| <#1465404160005378129> | feedback-kanal | anonymes Feedback |
| <#1494373349944459355> | ich-brauch-einen-coach | Coaching-Einstieg |
| <#1426220702054355077> | frag-die-community | offene Fragen an die Community |
| <#1326973956825284628> | patchnotes | Patch-Zusammenfassungen |
| <#1425215762460835931> | clip-submission | Clip-Einsendungen |
| <#1304169815505637458> | twitch | Live-Hinweise Streamer |
| <#1412411665713987635> | custom-games-chat | Custom Games (Text) |
| <#1512624055814062090> | Sammelpunkt | Custom Games (Voice) |

## Für Devs (knapp)
- Rust live: `dl-community/src/onboarding.rs` (Panel + Threads + Screening-Autostart), `reaction_roles.rs`, `tags_ui.rs`, `privacy_ui.rs`, `retention.rs`; Ranked-Voice: `dl-voice/src/rank.rs` (Anker + Subrang-Fenster ±9 + Overwrites)
- Kanal-Renames aus Server-as-Code (`dl-server-as-code/src/rules.rs`) sind GEPLANT, aktuell aber zurückgerollt (Rechte-Incident 2026-07-03): `beta-zugang` → `deadlock-invite`, `rang-auswahl` → `deadlock-rang`, `lag-kompensator` → `server-support`, `hier-starten-regelwerk` → `regelwerk`; kommen mit der nächsten Struktur-Welle wieder
- Doku-Konvention: Kanäle IMMER als `<#ID>` schreiben (rename-fest), nie als Klartext-`#name`; neue Kanäle in die Register-Tabelle oben eintragen
- Wichtige DB-Tabellen: keine eigene Fach-Tabelle; Persistenz läuft über Onboarding- und KV-Mechaniken
