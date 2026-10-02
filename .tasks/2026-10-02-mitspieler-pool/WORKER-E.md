status: aktiv (2026-10-02)

# Briefing: Spielerpool E

[Orchestrator] Paket E: gemeinsamer Session-Kanal auf dem Discord-Server.
Auftrag: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/AUFTRAG.md`.
WORKER-REGELN.md, PAKETE.md und VERTRAG-A.md im selben Ordner vollständig lesen.

- Worktree: `/home/nathanael/.worktrees/Deadlock-Bots-pool-e`
- Branch: `feat/spielerpool-e`
- Intent-Thread: `011b713c-266a-46b6-b0ce-1d5faa060234`
- Rolle: `worker_mittel`
- Startvoraussetzung: Paket A auf origin/main gemergt.

Vertrag A konkret: Discord-/Guild-/Kanal-/Session-IDs an JSON-Grenzen immer als
Dezimalstrings senden. Session zuerst `Pending`, nach Kanalzuordnung `Open`,
nach erfasstem Beitritt `Active`. Unbeigetretene Sessions erst nach einer Stunde
`Expired`, benutzte Sessions bei tatsächlichem Ende `Ended`. Feedback ist nur
für beigetretene Teilnehmer beendeter Sessions zulässig. Nach Profil-Löschen
verschwinden gemeinsame Sessionzeilen; eigene Kanäle ohne verbleibende Zuordnung
muss E ebenfalls entfernen. Discord-Erstellung und DB-Zuordnung bei Fehlern
ausgleichen. Diese Zustandsgrenzen vollständig übernehmen.

Du besitzt `rust/crates/dl-voice/src/pool_session.rs`, nötige Modul- und
Abhängigkeitsverdrahtung in dl-voice und den internen Loopback-Endpunkt im dl-bot
samt minimalem Einhänger. main.rs wird auch von C angefasst: E mergt zuerst.
Port, Auth-Header, Fehlerantworten und konkret verwendete Config mit D abstimmen.
Den verbindlichen E-Vertrag als VERTRAG-E.md im zentralen Aufgabenordner schreiben
und dem Delegator melden, sobald er feststeht. Bestehenden internen Token nutzen.

Vertrag aus PAKETE.md erhalten: POST `/internal/pool/session`, Initiator,
Ziel-Discord-IDs und Modus; Antwort enthält session_id, channel_id und channel_url.
Nur Loopback binden und serverseitig authentifizieren. Guild-Mitgliedschaft,
zulässige Profile und Eingaben prüfen. Der Website-Handler leitet den Initiator
aus der Discord-Session ab, niemals aus einem frei wählbaren Browser-Feld.

Der Bot eröffnet den Kanal für die Beteiligten und pingt sie dort. Die Session
und Teilnehmer in der zentralen DB speichern. Ohne Beitritt nach einer Stunde
löschen, Zeitablauf und Beitritt nach Bot-Neustart zuverlässig erkennen.
DMs nur mit aktivem Profil-Opt-in, Default aus. Keine Feedback-Benachrichtigung.
Bestehende Voice-Tower-, LFG- und Forum-Pfade nicht verändern.
Kanaltyp und bestehende Server-Kategorie aus dem vorhandenen Session-Verhalten
ableiten; falls daraus eine offene Produktentscheidung folgt, Bump-up melden.

Live-Prüfung ohne unerwünschte Pings an fremde Mitglieder. Echte Kontoaktionen
und die volle Stunde gehören in die abschließende Nutzer-Testanleitung, sofern
sie hier noch nicht belegt werden können.
