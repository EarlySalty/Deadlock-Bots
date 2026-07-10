# Onboarding und Invites

## Worum geht es?
Der Server führt neue Mitglieder gezielt durch Regeln, Rollen und die wichtigsten Bereiche. Der Deadlock-Beta-Invite ist davon unabhängig und bewusst einfach: Du fragst in <#1464736918951432222> nett nach einem Invite, und jemand aus der Community lädt dich ein.

## Wie nutze ich das?
**Onboarding:** Starte im <#1315684135175716975> über den Button `Hier starten` — der Bot legt dir einen privaten Onboarding-Thread an. Nach dem Discord-Member-Screening startet das Onboarding auch automatisch (Bots, bereits verifizierte und bereits durchgelaufene User werden übersprungen). Die festen Schritte: Regeln, Voice-Lanes, Mitspieler finden, Coaching, Custom Games, optionale Voice-Präferenzen (Ton/Alter) und die Steam-Verknüpfung. Beim Steam-Schritt prüfst du selbst per Button `Ich hab verknüpft` — der Bot checkt dann live, ob die Verknüpfung da ist.

Deine Voice-Tags kannst du jederzeit später über `/meine-tags` ändern; die Auswahl wird sofort gespeichert.

Zusätzlich gibt es das AI-Onboarding-Panel mit `Persönliche Tour starten` (3 kurze Fragen im Formular) und Quick-Buttons zu Spieler-Suche, Voice-Lanes, Feedback und Regelwerk. Und: Auf einigen Panels vergeben Reaktionen automatisch Rollen (Reaction Roles) — Reaktion entfernen kann die Rolle wieder entziehen.

**Deadlock-Invite:** <#1464736918951432222> ist eine offene Invite-Lounge — keine Befehle, keine Formulare, keine Vorbedingungen. So läuft es:

1. Schreib eine nette Frage in den Kanal („mag mich wer einladen? :)") und poste deinen **Steam-Freundescode** dazu (findest du in Steam unter Freunde → „Freund hinzufügen").
2. Ein Community-Mitglied fügt dich hinzu und lädt dich persönlich zum Playtest ein.
3. Der Bot schaut mit drauf: Bist du neu auf dem Server und fehlt der Freundescode bei deiner Anfrage, erinnert er dich freundlich daran — ohne Code kann dich niemand einladen. Wer schon länger dabei ist, bekommt den Hinweis nicht.

Zusätzlich lohnt sich die Steam-Verknüpfung in <#1398021105339334666> in jedem Fall: Damit bekommst du deine echte Rang-Rolle, und der Steam-Bot kann Invites auch automatisiert verschicken (läuft als Sicherheitsnetz im Hintergrund).

## Kosten / Premium
Alles kostenlos. Ein Invite kostet nie Geld — weder von der Community noch vom Bot. Es gibt einen optionalen Ko-fi-Support (freiwillig, kein Pflichtkauf und keine Beschleunigung).

## Was passiert technisch (kurz)?
Das Regelwerk-Panel und der Screening-Autostart erzeugen private Onboarding-Threads; der Flow speichert Entscheidungen wie Voice-Tags. In <#1464736918951432222> beobachtet der Bot Invite-Anfragen und erinnert bei fehlendem Freundescode. Der Steam-Bot kann Invites zusätzlich automatisiert über die Steam-Spielsuche verschicken.

## Grenzen & häufige Fragen
- **„Ich habe keinen Invite bekommen"**: Häufigster Grund ist ein „limited" Steam-Account — Steam blockiert Playtest-Invites, wenn auf dem Account noch keine ~5 $ ausgegeben wurden. Das ist eine Valve-Regel, die erst beim Invite-Versuch sichtbar wird; kein Bot und kein Community-Mitglied kann sie umgehen.
- Nach einem Invite kann es 1–2 Tage dauern, bis die Einladung bei Steam sichtbar ist.
- Ohne Freundescode in deiner Anfrage kann dich niemand einladen — der Bot erinnert dich im Kanal daran.
- Es gibt KEINE Onboarding-Option „Invite/Betazugang" und keinen Invite-Befehl — einfach im Kanal fragen.
- Ein Streamer-Setup-Flow im Onboarding existiert aktuell nicht (nur Hinweise); Streamer-Themen laufen über das Twitch-Dashboard.

## Für Devs (knapp)
- Rust: `dl-community/src/onboarding.rs` (+ `onboarding_steps.json`), `ai_onboarding.rs`, `tags_ui.rs`, `reaction_roles.rs`, Invite-Lounge-Watcher in `dl-community`; automatisierter Invite-Pfad im Steam-Bot: `steam-flows/src/betainvite/`
- Der Invite-Lounge-Watcher antwortet nur Mitgliedern, die weniger als 7 Tage auf dem Server sind (`NEWCOMER_MAX_JOIN_SECONDS`, `author_joined_at` aus dem Gateway-Event); unbekanntes Beitrittsdatum bedeutet keine Antwort. Trigger-Bedingungen und Cooldown vollständig in Deadlock-Docs: `internal/deadlock-bots/onboarding-und-invites-devs.md`
- Der User-Slash-Command `/betainvite` wurde entfernt; der Funnel ist nur noch über den Panel-Button (`betainvite:panel:start`, Admin: `/publish_betainvite_panel`) erreichbar
- Kanalnamen live: <#1464736918951432222>, <#1398021105339334666> (Server-as-Code-Renames)
