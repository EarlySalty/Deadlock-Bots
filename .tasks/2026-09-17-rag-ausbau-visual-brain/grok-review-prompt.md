Du bist unabhaengiger Reviewer. Pruefe die interne Doku gegen den echten Quellcode. Aendere NICHTS an Dateien, gib nur einen Befund als Klartext aus.

Doku: /home/nathanael/repos/Deadlock-Docs/internal/wissensbasis/dl-knowledge-engine.md
Quelle: /home/nathanael/repos/Deadlock-Bots/rust/bin/dl-knowledge/src/main.rs

Vorgehen:
- Lies die Doku.
- Pruefe stichprobenartig 8 bis 10 der Zeilenangaben aus der Doku gegen den echten Code (nutze sed oder rg auf die Quelle, keine ganze Datei am Stueck).
- Achte besonders auf den Retrieval-Pfad (BM25), die Abstain-Logik und den Korpuspfad.

Melde in dieser Struktur:
1. Falsche oder erfundene Aussagen (mit Zeilenbezug).
2. Zeilenangaben, die nicht passen.
3. Fehlende wichtige Punkte.
4. Urteil: brauchbar als interne Referenz, ja oder nein.
5. Konkrete Korrekturen als Liste.

Regeln: Deutsch, echte Umlaute ä ö ü ß, keine Em-Dashes. Kein Datei-Edit, kein git.
