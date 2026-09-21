# Paket C: Prüfungen und Abnahmegrenzen

## Paket- und Datenbanktests

174 verschiedene Rust-Tests haben bestanden: die 151 bereits in B vorhandenen Paket- und Datenbanktests sowie 23 neue C-Tests. Gezählt werden unterschiedliche Testnamen, nicht wiederholte Ausführungen. Dazu kommen vier Python-Tests für das Zusammenführen der C-Messberichte.

Der vollständige Paketlauf nach Ergänzung des Qualitätschecks ergab 149 bestandene Tests. Die 24 sonst ignorierten Datenbanktests wurden gesondert ausdrücklich aktiviert und bestanden ebenfalls. Der danach ergänzte Messbereichstest bestand zusätzlich einzeln. Der Default-Test wurde nach Begrenzung des Reranker-Pools auf zwölf Kandidaten erneut ausgeführt; er erhöht die Anzahl unterschiedlicher Tests nicht.

```bash
PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 RAYON_NUM_THREADS=2 \
  bash scripts/central_test_db.sh \
  cargo test --package dl-knowledge --package dl-central-db --all-features -- --test-threads=2

PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 \
  bash scripts/central_test_db.sh \
  cargo test --package dl-central-db --all-features -- --ignored --test-threads=1
```

Die 23 neuen Rust-Tests prüfen Konfigurationsgrenzen und Default-aus, Kalenderdaten und optionale Metadaten, RRF-Gewichte und Deduplizierung, Filter vor Top-k, Quellen- und Aktualitätsprioritäten, Reranker-Reihenfolge und ungültige Scores, fehlende lokale Modelle, Worker-Fehler und die Kapazitätsbindung nach Timeout oder Client-Abbruch. Hinzu kommen Datenbanktests für fehlende beziehungsweise falsche Indexgenerationen, veraltete Chunks und manipulierte Texte sowie drei Tests des tatsächlichen Antwortpfads. Zwei weitere Tests trennen Quellentreffer von renderbaren Belegen und prüfen die Grenzen einer Golden-Teilmessung.

Die vier Python-Tests prüfen eine vollständige Zusammenführung sowie die Ablehnung von Lücken, doppelten Bereichen und unterschiedlichen Korpus-Hashes. Sie verwenden ausschließlich ausdrücklich benannte synthetische Fixtures, keine veränderten Golden-Dateien.

```bash
python3 ../.tasks/2026-09-17-rag-ausbau-visual-brain/test_auswertung_c.py
```

Es wird kein nachträglicher Test-first- oder Rot-Grün-Nachweis behauptet. Die Tests wurden während der Implementierung ergänzt und gegen den implementierten Stand ausgeführt.

## Build und Clippy

Der Default-Release-Build mit Fastembed-Embedding und lokalem Fastembed-Reranker funktioniert. Die Pakettests mit `--all-features` kompilieren auch den Candle-Laufweg. Die C-Latenzmessung verwendet Fastembed; ein eigener vollständiger C-Latenzvergleich mit Candle wurde nicht durchgeführt.

```bash
PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 \
  cargo build --release --package dl-knowledge

PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 \
  cargo clippy --package dl-knowledge --package dl-central-db --all-features -- -D warnings
```

Der vorgeschriebene Paket-Clippy-Lauf mit allen Features ist ohne Warnungen grün. Die zusätzlich ausprobierte strengere Variante mit `--all-targets -- -D warnings` ist davon zu unterscheiden: Sie beanstandet zwölf `unwrap_used`-Stellen in der unveränderten B-Datei `src/dense/tests.rs`. Eigene C-Teststellen wurden auf explizite Fehlermeldungen umgestellt. Die B-Implementierung und ihre Test-Erwartungen wurden nicht verändert; dieser zusätzliche All-Targets-Lauf wird nicht als grün ausgewiesen.

Formatiert wurde ausschließlich der eigene Hybrid-Modulbaum. Die wenigen neuen Zeilen im bestehenden Antwortpfad wurden lokal formatiert, nicht die gesamte Datei oder das Repository.

## Offline-Modellprüfung

Der echte lokale Cross-Encoder wurde im Container `rust:1.88-bookworm` mit `--network none`, `--read-only`, schreibgeschütztem Modell- und Binary-Mount, unprivilegierter UID, verworfenen Linux-Capabilities und zwei CPUs geprüft. `hybrid check` lieferte zwei endliche Scores in Eingabereihenfolge; der passende Steam-Beleg erhielt den höheren Score. Das ist ein Schnittstellen-Smoke-Test, kein Qualitätsnachweis für den gesamten deutschen Korpus.

Ein zweiter Lauf mit absichtlich fehlendem Modellverzeichnis endete mit Fehler und ohne Download. Die vorhandenen Modelldateien blieben unverändert. Die Rohdaten stehen in `OFFLINE-C.json`, die Datei-Prüfsummen in `MODEL-C.sha256`.

## Antwortvertrag und unveränderte Schutzprüfungen

`VERTRAG-C.json` enthält den Vergleich mit B-Commit `508419ee`. `candidate_is_relevant`, `grounded_response`, `candidates_for`, `build_prompt`, `parse_html_file`, `parse_llm_selection` und `grounding_terms` sind bytegleich geblieben.

Die Antwortpfadtests verwenden einen kontrollierten Selektions-Generator. Sie zeigen den unveränderten JSON-Vertrag, serverseitiges Rendern sowie Ablehnung bei fehlenden lexikalischen Belegen, aktivem Quellenfilter und Reranker-Fehler. Sie rufen keinen externen Provider auf und ersetzen keine Live-API-Abnahme.

## Golden: bekannte rote Abnahme bleibt sichtbar

Die unveränderte ursprüngliche Prüfung wurde ausdrücklich aktiviert:

```bash
DL_DOCS_PATH=/home/naniadm/.local/share/dl-knowledge/current/public \
DL_GOLDEN_DIR=/home/nathanael/repos/Deadlock-Docs/evals \
PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= \
  cargo test --package dl-knowledge --all-features \
  tests::golden_retrieval_corpus -- --ignored --exact
```

Ergebnis: weiterhin Fehler bei „Wie melde ich mich für Scrims an?“, weil `/scrim-signup` im erwarteten Kontext fehlt. Die Trefferliste entspricht der in B dokumentierten Abweichung. Korpus und Erwartungslabels wurden nicht angepasst. Dieser Test bricht beim ersten Fehler ab und ist ausdrücklich nicht Teil der 174 grünen Rust-Paket- und Datenbanktests.

Der zusätzliche C-Vergleich erfasst dagegen alle 224 unveränderten Golden-Fälle und trennt Quellentreffer, lexikalisch verlorene Belege und zulässige Auswahlmöglichkeiten. Ergebnisse und verbleibende Abweichungen stehen in `MESSUNG-C.md`. Es gab keine Live-API-Evaluierung und keine gemessene Faithfulness- oder Abstain-Quote tatsächlich generierter Antworten.

## Ausführung und Produktion

Ein erster 24er-Reranker-Lauf überschritt das standardmäßige Drei-Sekunden-Budget beim Aufwärmen. Ein weiterer langer Lauf mit erhöhtem Testbudget wurde vor dem abschließenden Bericht beendet; zuletzt lag eine Fortschrittsmarke bei 193 von 224 Fällen vor. Daraus werden keine vollständigen Qualitäts- oder Latenzzahlen abgeleitet.

Anschließend wurde die Messung fallweise gesichert und in zwei Teilbereiche aufgeteilt. Das Zusammenführen akzeptiert nur gleiche Modelle, Konfigurationen, Korpus- und Golden-Hashes sowie jeden der 224 Fälle genau einmal. Die vollständigen Rohberichte werden getrennt vom Messprotokoll mitgespeichert.

Alle Schreibtests und Indexaktivierungen liefen ausschließlich in eigenen Wegwerf-Datenbanken über `central_test_db.sh`. Kein Produktionsindex wurde aktiviert, kein Dienst neugestartet, keine Produktionskonfiguration umgestellt und kein Merge ausgeführt.
