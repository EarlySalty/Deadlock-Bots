Du bist unabhaengiger Reviewer. Pruefe die interne Doku gegen den echten Quellcode. Aendere NICHTS an Dateien, gib nur einen Befund als Klartext aus.

Doku: /home/nathanael/repos/Deadlock-Docs/internal/wissensbasis/dl-ai-provider.md
Quelle: /home/nathanael/repos/Deadlock-Bots/rust/crates/dl-ai/src/ (Einstieg chat_provider.rs; weitere Dateien nach Bedarf, gezielt mit sed/rg, keine ganze Datei am Stueck)

Pruefe besonders diese Behauptungen der Doku gegen den Code:
1. Modellaufloesung: einkompilierter Default deepseek-v4-flash-0731, Ueberschreibung per Env und Param, KEINE dynamische Modell-Discovery. Stimmt das im Code?
2. Timeouts: 110 Sekunden fuer BotPate, 45 Sekunden im Fabrikweg, 60 Sekunden beim direkten FireworksClient. Stimmen diese Werte und Fundstellen?
3. reasoning_effort/Denkmodus: nur der direkte Client sendet es, der Adapter verwirft es. Stimmt das?
4. Die gemeinsame Trait-Schnittstelle (TextGenerator/ChatProvider), die dl-knowledge und Concierge nutzen.

Melde: 1) falsche oder erfundene Aussagen mit Zeilenbezug, 2) nicht passende Zeilenangaben, 3) fehlende wichtige Punkte, 4) Urteil brauchbar ja/nein, 5) konkrete Korrekturen als Liste.

Regeln: Deutsch, echte Umlaute ä ö ü ß, keine Em-Dashes. Kein Datei-Edit, kein git.
