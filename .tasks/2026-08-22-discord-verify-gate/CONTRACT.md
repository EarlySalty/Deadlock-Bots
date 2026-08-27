status: aktiv
datum: 2026-08-27

# Contract: Discord Verify-Gate fuer neue Accounts

Quelle der Entscheidungen: `.tasks/2026-08-22-discord-verify-gate/GRILLME.md` (Grill-Session 2026-08-22, alle Cluster A-F entschieden). Klasse: hoch (Auto-Kick echter Menschen, Bot-DM an Fremde; gedeckt durch Default "frische Accounts hart, Auto-Ban ok").

## Ziel

Neue Discord-Mitglieder mit einem Konto juenger als 1 Monat, die nicht ueber einen persoenlichen Invite kommen, muessen per Bot-DM eine feste Frage beantworten ("Nenne einen Hero aus Deadlock"). Bis zur bestandenen Antwort sind sie in Vollquarantaene (keine Kanaele sichtbar). Wer nicht besteht oder nicht antwortet, wird gekickt. Zweck: Pre-Gate gegen Scam-Accounts, die Mitglieder per DM anschreiben.

## Anforderungen (user-sichtbar, pruefbar)

- **REQ-01** Beim Join eines Nicht-Bot-Accounts, dessen Konto juenger als 30 Tage ist und der NICHT ueber einen persoenlichen Invite-Code eines Mitglieds kam, vergibt der Bot sofort die Quarantaene-Rolle und schickt eine DM. (GRILLME A1, A3, B1, E1, E2)
- **REQ-02** Quarantaene = keine Kanaele sichtbar ("Ban ohne Ban"), gilt auch fuer echte Neulinge. (D3)
- **REQ-03** Die DM erklaert kurz (Account zu neu, bitte bestaetigen, dass du kein Bot bist) und die Verifizierung muss vom Nutzer aktiv gestartet werden; danach kommt die feste Frage "Nenne einen Hero aus Deadlock." (B1, C4)
- **REQ-04** Antwortbewertung zweistufig: (a) enthaelt die Antwort einen bekannten Deadlock-Heronamen -> bestanden (Listenabgleich); (b) sonst bewertet der Judge (zentraler LLM-Weg, Repo-Default DeepSeek v4 Flash), ob die Beschreibung plausibel einen Hero meint UND auf Deutsch ist. Englische Antwort faellt durch. (C1, C2)
- **REQ-05** Drei Versuche. Jeder Versuch wird bewertet. Nach dem dritten abgelehnten Versuch sofort Kick, ohne den Rest der Frist abzuwarten. (C3, D4)
- **REQ-06** Frist 24 Stunden ab Join. Ohne bestandene Antwort danach Kick. (D1)
- **REQ-07** DM nicht zustellbar -> Kick. (B2)
- **REQ-08** Judge kann nicht urteilen (Timeout, Provider-Fehler oder unsicheres Urteil) -> fail-open, Bot schaltet frei. (F2)
- **REQ-09** Nach bestandener Antwort: Quarantaene-Rolle entfernen, normaler Zugang, keine weitere Meldung. (Agenten-Entscheid)
- **REQ-10** Rejoin nach Kick: wer einmal ueber dieses Gate gekickt wurde, bekommt beim naechsten Join freien Zugang, das Gate greift nicht mehr. (B3)
- **REQ-11** Bots (OAuth-Join) sind vom Gate ausgenommen. (Agenten-Entscheid)
- **REQ-12** Kein rueckwirkendes Gate: greift nur beim Join, bestehende junge Accounts bleiben unberuehrt. (A3)

## Invarianten (darf sich nicht aendern)

- **INV-01** Kein Mod-Log-Post und keine Mod-Freigabe fuer den Gate-Ablauf. (F1)
- **INV-02** AI-Aufruf nur ueber den bestehenden zentralen LLM-Connector, Modell konfigurierbar, nie hart verdrahtet.
- **INV-03** Persistenz ausschliesslich zentrale PG (pending-Zustand und Gekickten-Liste), kein SQLite, keine ENV/Dateien fuer Config, Secrets aus Infisical.
- **INV-04** Discord-Kanaele/Rollen immer per ID, nie per Name.
- **INV-05** Kein bestehender Join-/Onboarding-/Analytics-Pfad wird gebrochen (MemberEvent::Join fuettert weiter Journey/Analytics).
- **INV-06** User-sichtbare Texte: natives Deutsch, echte Umlaute, keine Em-Dashes.

## Nicht-Ziele

- Kein Inviter-Alters-/Dauer-Check (jeder persoenliche Invite zaehlt). (E2)
- Keine 100%-Trefferquote; bewusst grobes Pre-Gate. (C2)
- Kein Fragenwechsel, keine Fragenrotation. (C4)
- Kein Server-Spam-Schutz (das Bedrohungsbild ist DM-Scam). (D2)
- Kein Shadow-Modus als Zwischenstufe (Nutzer hat scharfe Aktion als BUILD-P0 entschieden).

## Erlaubter Aenderungsbereich

- `rust/crates/dl-discord/` (Join-Event, DM-Versand, Kick, Rollen-Verwaltung)
- `rust/crates/dl-community/` (Gate-Logik, Antwortbewertung, Invite-Klassifizierung nutzen)
- `rust/bin/dl-bot/` (Verdrahtung, Scheduler/Frist-Kick, DM-Interaktions-Handler)
- `rust/crates/dl-central-db/migrations/` (neue Tabellen pending-verify + gekickt)
- `rust/crates/dl-central-db/src/` (Store-Modul verify_gate.rs und lib.rs-Export)
- Heroliste: bestehende Datenquelle nutzen, sonst neue Konstante/Tabelle
- `.tasks/2026-08-22-discord-verify-gate/` (Artefakte)

## Verbotener Bereich

- Kein Eingriff in Twitch-/Steam-/Turnier-Repos.
- Keine Aenderung an nativem Discord-Onboarding.
- Kein neues *_ENABLED-Flag als Krucke; Gate haengt an klarer Verdrahtung.

## Offene Produktfragen

- Keine blockierenden. DM-Wortlaut (GRILLME KLAEREN) formuliert der Orchestrator (rolle-doku-redakteur) und legt ihn im Ergebnis vor; reversibel, kein Gate.

## Amendments
- 2026-08-27: erlaubter Bereich, nur dl-central-db/migrations/ -> zusaetzlich rust/crates/dl-central-db/src/ (verify_gate.rs Store und lib.rs-Export), Grund: migrations/ allein kann den Rust-Store nicht halten, Ungenauigkeit beim Contract-Schreiben, entschieden von Orchestrator
