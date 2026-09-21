# Paket C: Messung von Hybrid-Retrieval und lokalem Reranker

Stand: 2026-09-18. Ergebnis: implementierbarer, getesteter Kandidat, aber kein nachgewiesener Mehrwert für produktive Antworten. Hybrid bleibt standardmäßig ausgeschaltet. B und C nicht allein wegen dieser Implementierung freigeben.

## Messbasis

109 öffentliche HTML-Dateien, 603 Chunks und alle 224 unveränderten Golden-Fälle: 212 positive und zwölf negative Fragen. Korpus und Labels wurden nicht angepasst. Jeder Fall wurde pro Modus einmal gemessen; die 224 Messwerte sind verschiedene Fragen, keine wiederholten Stichproben derselben Frage.

Verglichen wurden der unveränderte BM25-Pfad und derselbe Hybrid-Retrievalkern vor beziehungsweise nach dem Reranker. BM25 und Dense liefern je 24 Kandidaten. RRF verwendet k=60 und Gewichte 1:1; zwölf fusionierte Chunks gehen in den Reranker, sechs Chunks in den unveränderten Passagenpfad. Metadatenfilter und zusätzliche Quellen-/Aktualitätsgewichte waren für den Vergleich ausgeschaltet.

Embedding: Fastembed aus B. Reranker: cross-encoder/ms-marco-MiniLM-L6-v2, Revision 233902d25c440f23af6f7d6e94d2946bac0bee0a, FP32-ONNX, Tokenlimit 256, Batchgröße vier. Die fünf lokalen Modelldateien sind durch MODEL-C.sha256 festgelegt. Kein externer Embedding- oder Rerank-Endpunkt und keine Antwortgeneration wurden aufgerufen.

Die beiden vollständigen Teilbereiche 0 bis 111 und 112 bis 223 liefen nacheinander, jeweils mit drei Warm-up-Fragen und einer eigenen frisch migrierten Wegwerf-Datenbank. Der Messprozess war auf CPUs 0 und 1 begrenzt; der Server war nicht exklusiv reserviert. Modelldateien waren vorab installiert. Modellladen und Indexaufbau sind nicht Teil der Query-Latenz.

## Latenz und Retrievalqualität

| Größe | BM25 | Hybrid ohne Reranker | Hybrid mit Reranker |
|---|---:|---:|---:|
| Query p50, ms | 0.37 | 180.26 | 2449.02 |
| Query p95, ms | 3.00 | 402.65 | 4598.12 |
| Mindestens eine erwartete Quelle in Top-6 | 209/212 | 204/212 | 208/212 |
| Mittlerer Quellen-Recall@6 | 89.74% | 88.33% | 90.09% |
| MRR@6 | 0.8539 | 0.8727 | 0.8744 |
| Zulässige Golden-Belegauswahl vorhanden | 205/212 | 193/212 | 198/212 |
| Fehlender erwarteter Kontext | 6/212 | 15/212 | 9/212 |

Der Reranker allein benötigt p50 2216.85 ms und p95 4108.91 ms. Der höchste in den Teilprozessen beobachtete residente Speicher liegt bei 554.09 MiB. Die Zahlen messen den geladenen Retrievalkern einschließlich lokaler SQL-Suche, aber ohne HTTP, Worker-Dispatch, Kopie des KnowledgeBase-Snapshots und LLM. Der gemeinsame Hybrid-Präfix wird vor dem Reranking zeitlich erfasst; es sind keine zwei unabhängig bedienten HTTP-Anfragen.

Das Testbudget betrug ausdrücklich zehn Sekunden, damit auch langsame Fragen bewertet werden konnten. Das Dienst-Defaultbudget bleibt drei Sekunden. 60 von 224 Messwerten überschreiten bereits im Retrievalkern diese drei Sekunden. Die 198 prinzipiell renderbaren Belege sind daher keine Erfolgszahl des standardmäßig budgetierten Dienstes. Bei vergleichbarer Last würden zu langsame Hybrid-Anfragen fail-closed enden; daraus wird keine gemessene Live-Abstain-Quote abgeleitet.

Ein früherer Versuch mit 24 Reranker-Kandidaten überschritt beim Aufwärmen das Drei-Sekunden-Budget. Ein anschließender langer Lauf wurde vor dem abschließenden Bericht beendet; seine letzte Fortschrittsmarke lag bei 193/224. Für diese unvollständigen Läufe werden keine vollständigen Qualitätszahlen angegeben. Der anschließend vollständig gemessene Kandidat mit zwölf Reranker-Chunks ist der Konfigurationsdefault, nicht eine Produktionsfreigabe.

Quellen-Recall bezeichnet den pro positiver Frage gemittelten Anteil der erwarteten Dokumentpfade in den ersten sechs Chunks. MRR verwendet den Rang der ersten erwarteten Quelle. Mehrere passende Abschnitte derselben Datei sind noch keine vollständige Antwort. Die Belegprüfung sucht nach einer erlaubten Auswahl von höchstens vier Kandidaten, die der unveränderte grounded_response tatsächlich rendern kann und die alle erwarteten Antwortterme enthält. Der Auswahltest verwendet Kombinationen in Kandidatenreihenfolge, entsprechend der bestehenden Golden-Prüfung; freie Umordnungen durch das LLM werden nicht gesucht. Gefundene Auswahlen sind keine gemessene Antwortgenauigkeit oder Faithfulness.

Bei keinem der drei Modi gab es einen Treffer auf verbotene Kontextterme. Nur eine der zwölf negativen Fragen hatte gar keine lexikalisch zulässigen Kandidaten. Die übrigen negativen Fragen sind damit nicht als beantwortet oder als bestanden zu werten; ihre tatsächliche Abstention muss die Live-Evaluierung prüfen.

## Wechselwirkung mit candidate_is_relevant

| Diagnose auf den 212 positiven Fällen | BM25 | Hybrid | Hybrid + Reranker |
|---|---:|---:|---:|
| Erwartete Quelle vorhanden, danach kein Kandidat dieser Quelle mehr zulässig | 1 | 3 | 2 |
| Erwartete Termabdeckung vor Prüfung vorhanden, danach verloren | 2 | 5 | 3 |

Unter den Dense-only-Chunks, also Dense-Top-24 ohne Überschneidung mit BM25-Top-24, gibt es in 94 positiven Fällen mindestens einen Chunk aus einer erwarteten Quelle. In 40 Fällen verschwinden danach sämtliche zulässigen Kandidaten dieser erwarteten Quelle; in 12 Fällen geht eine zuvor vorhandene erwartete Termabdeckung verloren. Das sind Diagnosen auf Dokument- beziehungsweise Termebene. Sie beweisen nicht, dass jeder vorher verworfene Absatz semantisch richtig war, und sind nicht gleichbedeutend mit insgesamt unbeantwortbaren Fragen.

Die Schutzprüfung wurde nicht gelockert. VERTRAG-C.json dokumentiert den unveränderten Code gegenüber B. Die folgenden Fälle verlieren im Reranker-Pfad die erwartete Termabdeckung an dieser Prüfung:

- Was kann der Twitch-Bot und wie fange ich an? (public-integration.json, Fall 5)
- Warum sehe ich in meinem Dashboard noch keine Daten? (public-twitch.json, Fall 12)
- Was kann ich prüfen, wenn im Overlay keine Daten erscheinen? (public-twitch.json, Fall 14)

## Gegenüber BM25 verlorene und gerettete Belegfälle

Der Reranker rettet 1 Fälle, die BM25 nicht vollständig belegen konnte, verliert aber 8 zuvor belegbare Fälle. Netto sind sieben positive Fälle weniger belegbar. Der leicht höhere mittlere Quellen-Recall reicht deshalb nicht als Nutzenbeleg.

| Frage | BM25 | Hybrid + Reranker |
|---|---|---|
| Wofür ist der Steam-Bot da? | belegbar | nicht belegbar |
| Was kann der Twitch-Bot und wie fange ich an? | belegbar | nicht belegbar |
| Wo finde ich übersetzte Deadlock-Patchnotes? | belegbar | nicht belegbar |
| Woher nimmt der Bot die Patchnotes? | belegbar | nicht belegbar |
| Was ist der neueste Deadlock-Patch? | belegbar | nicht belegbar |
| Warum wurde mein Rang noch nicht meiner Community-Zuordnung hinzugefügt? | belegbar | nicht belegbar |
| Was kann ich im Turnierportal selbst erledigen? | belegbar | nicht belegbar |
| Was kann ich prüfen, wenn im Overlay keine Daten erscheinen? | belegbar | nicht belegbar |
| Wie prüfe ich meinen Deadlock-Rang über den Bot? | nicht belegbar | belegbar |

## Golden-Abnahme und nächste Freigabegrenze

Die ursprüngliche Golden-Retrieval-Prüfung bleibt bei „Wie melde ich mich für Scrims an?“ rot, weil der Kontextterm /scrim-signup fehlt. Paket C bewertet außerdem alle späteren Fälle, verändert dafür aber weder Korpus noch Erwartungslabels. Die Live-API-Evaluierung wurde nicht ausgeführt.

Kein Merge und keine Produktionsumschaltung. Für D sind insbesondere die verlorenen Belegfälle, die zusätzlichen Latenzkosten und das Budgetverhalten relevant. Eine Lockerung des Groundings ist aus diesen Ergebnissen nicht gerechtfertigt. Die endgültige Reranker-Modellwahl bleibt beim Nutzer; die vorhandene Implementierung ist austauschbar. Bis eine erneute Abnahme mehr tatsächlich nutzbare Antworten gegen BM25 zeigt, bleibt BM25 das Live-Verhalten.

## Rohdaten und Reproduktion

Die vollständigen JSON-Berichte wurden für die Messung erzeugt und validiert, aber nicht dauerhaft ins Repository übernommen. Sie umfassten mehr als zwei Megabyte und würden die Merge-Prüfung ohne zusätzlichen Betriebsnutzen belasten. Die reproduzierbaren Zusammenfassungen, Konfigurationen, Modellprüfsummen und Prüfregeln bleiben in dieser Akte erhalten.

`auswertung-c.py` lehnt verschiedene Modelle, Konfigurationen und Korpora sowie Lücken oder doppelte Fälle ab. Die konkreten Kommandos und Modellinstallation stehen in `BETRIEB-C.md`. Die Paket- und Offline-Prüfungen stehen in `TESTS-C.md` und `OFFLINE-C.json`. Die aktuelle Produktionsentscheidung auf dem 254-Fälle-Stand steht in `ENTSCHEIDUNG-SESSION4.md`.

Korpus-SHA-256: `0b49cedcf95529b13013d407a71186214ae332b0fdb467cffa8aef055ffdd6ce`

Golden-SHA-256: `53c03b22b5f1b00ece02491dd1c76cfd5f247544ee8bc5a47fc2767fae9b8841`

Embedding-Fingerabdruck: `26acc83cc509eb8d4bc1a955820f0af64f0a24d01da7da6882beb61a98b9d302`

Reranker-Fingerabdruck: `bd099a320eab5c9057da5846f286ba3e5a34139b10dad63b13723d3d08560bee`
