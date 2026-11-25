status: aktiv (2026-10-02)

# Briefing: Spielerpool C

[Orchestrator] Paket C: Spielmuster aus der Deadlock-API.
Auftrag: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/AUFTRAG.md`.
WORKER-REGELN.md, PAKETE.md und VERTRAG-A.md im selben Ordner vollständig lesen.

- Worktree: `/home/nathanael/.worktrees/Deadlock-Bots-pool-c`
- Branch: `feat/spielerpool-c`
- Intent-Thread: `011b713c-266a-46b6-b0ce-1d5faa060234`
- Rolle: `worker_mittel`
- Startvoraussetzung: Paket A auf origin/main gemergt.

Vertrag A konkret: `steam_links_for_ingest` liefert ein bestätigtes Konto pro
veröffentlichtem Profil. Snapshot und Matches atomar über `store_api_snapshot`
schreiben, gemeinsame Paare als vollständigen Zählerstand über `store_co_player`.
Heatmap umfasst 168 UTC-Buckets mit Montag 0. Beobachtungszeitraum und tatsächlich
beobachtete Spiele/Stunden erhalten; Gesamtwerte nur bei belegtem API-Gesamtwert.
Gelöschte Profile niemals wieder anlegen. API-Zeitraum, Skala und Felder als
VERTRAG-C.md dokumentieren und D über den Delegator übergeben.

Du besitzt `rust/crates/dl-activity/src/pool_ingest.rs`, nötige Modul- und
Abhängigkeitsverdrahtung in dl-activity und den minimalen Start in
`rust/bin/dl-bot/src/main.rs`. Paket E ändert ebenfalls main.rs: E mergt zuerst,
danach C rebasen und den gemeinsamen Pfad erneut durch den Gate prüfen.
Bei eigenem ALLOW vor E nur Bereitschaft melden und auf Merge-Reihenfolge warten.

Verknüpfte Steam-Identitäten aus dem bestehenden zentralen Mapping verwenden.
Aus echten Deadlock-API-Daten Rang, Matchhistorie, Spiele, gespielte Stunden,
Heatmap aus Wochentag und Stunde sowie Mitspieler-Paare schreiben. Zeitbezug
eindeutig dokumentieren, damit die Website Europe/Berlin korrekt darstellt.
Matchdaten idempotent verarbeiten, keine wiederholte Aufsummierung. API-Fehler,
Limits und fehlende Daten offen unterscheiden; keine Werte ausdenken oder Nullwerte
als belegte Spielstatistik ausgeben. Stunden aus Spielen als solche ausweisen,
falls die API keine gesamten Spielstunden liefert. Produktrelevante Datenlücken
als Bump-up melden, bevor die Bedeutung geändert wird.

Die Datenschicht und Website dürfen Namen anderer Mitspieler nur anzeigen,
wenn diese selbst im zulässigen Pool sind. Kein Deadlock-Steam-Bot ändern.
Vorhandene Matching- und Aktivitätslogik bei Bedarf lesen und sinnvoll nutzen.
Echte API-Daten und die geschriebenen Aggregationen nachweisen; für den finalen
Nutzertest einen nachvollziehbaren Ingest-Status hinterlassen.
