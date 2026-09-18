# Paket C: Hybrid-Retrieval und lokaler Reranker

## Produktionsgrenze

Der Dienst bleibt ohne `DL_KNOWLEDGE_HYBRID=true` beim bisherigen BM25-Pfad. Weder Embedding-Modell noch Reranker noch Dense-Datenbankpool werden dann initialisiert. Die Defaultfeatures bauen die lokalen Laufwege mit ein, aktivieren sie aber nicht. Paket C verändert keine laufende Dienstkonfiguration, startet keinen Dienst neu und aktiviert keinen Produktionsindex.

Der Antwortvertrag `{answerable, answer, sources}`, Modell und Provider der Antwortgeneration, `candidate_is_relevant`, `grounded_response`, Passagenaufteilung und Rendering bleiben unverändert. C ändert nur die Auswahl und Reihenfolge der Chunks vor der bisherigen Passagenprüfung. Ein Dense-Treffer ist keine Antwortfreigabe.

## Konfiguration

`DL_KNOWLEDGE_HYBRID` akzeptiert `true`, `false`, `1`, `0`; nicht gesetzt bedeutet `false`. Bei ausgeschaltetem Schalter werden zusätzliche Hybrid-Optionen ignoriert. Bei aktivem Schalter werden unbekannte JSON-Felder und ungültige Werte abgewiesen.

`DL_KNOWLEDGE_HYBRID_OPTIONS` ist ein JSON-Objekt. Ohne Überschreibungen gelten:

```json
{
  "bm25_k": 24,
  "dense_k": 24,
  "fusion_k": 12,
  "output_k": 6,
  "rrf_k": 60.0,
  "bm25_weight": 1.0,
  "dense_weight": 1.0,
  "rerank": true,
  "rerank_max_tokens": 256,
  "rerank_batch_size": 4,
  "timeout_ms": 3000,
  "metadata": {
    "stand_min": null,
    "stand_exact": null,
    "quellen": [],
    "recency_weight": 0.0,
    "recency_half_life_days": 30.0,
    "source_weights": {}
  }
}
```

Alle Kandidatenlimits liegen zwischen 1 und 100; `output_k` darf nicht größer als `fusion_k` sein. Mindestens ein Retrieval-Gewicht muss positiv sein. Null deaktiviert den betreffenden Zweig für die Suche. Eine aktive Hybrid-Instanz lädt ihr Embedding-Modell und ihren Pool trotzdem einmalig beim Start. Reranking ist separat mit `rerank:false` abschaltbar. Es gibt keine neue externe Modellanbindung.

`stand_exact` vergleicht den vorhandenen Metadatenwert exakt und kann auch eine Versionsbezeichnung statt eines Datums enthalten. `stand_min` verlangt ein gültiges Datum im Format `YYYY-MM-DD`; nicht als Datum lesbare Stände erfüllen diesen Filter nicht. Quellenfilter und Quellenprioritäten vergleichen `quelle` exakt. Es gibt keine automatische Einstufung fremder Quellen als vertrauenswürdig. Alle aktiven Filter gelten gemeinsam.

BM25 wird vor seiner Top-k-Auswahl gefiltert. Dense verbindet die gespeicherten Datensätze bereits vor dem SQL-Limit mit den zulässigen IDs, Inhalts-Hashes, Ständen und Quellen des aktuell geladenen öffentlichen Korpus. Gelöschte oder inzwischen geänderte Chunks können dadurch nicht über einen alten Index zurückkommen. Der Dienst rendert ausschließlich die ursprünglichen Passagen des aktuell geladenen Korpus, niemals einen frei übernommenen SQL-Text. Zusätzlich werden die zurückgelieferten Text- und Metadatenfelder gegen diesen Korpus geprüft.

RRF summiert pro Chunk `Gewicht / (rrf_k + Rang)` mit Rangbeginn 1. Doppelte Einträge innerhalb eines Zweiges zählen nicht mehrfach. Gleichstände werden stabil über die Chunk-ID aufgelöst. Der Metadatenfaktor ist:

```text
Quellengewicht × (1 + Aktualitätsgewicht × 2^(-Alter / Halbwertszeit))
```

Das Alter wird relativ zum neuesten gültigen Datum im geladenen Korpus gemessen, nicht relativ zur Systemzeit. Ohne aktiviertes Aktualitätsgewicht und ohne Quellenprioritäten ist der Faktor 1. Derselbe Faktor wird auch beim Sortieren der lokalen Cross-Encoder-Sigmoid-Scores angewandt. Das sind Rangsignale, keine kalibrierten Wahrscheinlichkeiten der Beantwortbarkeit und keine Grounding-Schwellen.

## Modelle und Ressourcen

Embedding: `DL_KNOWLEDGE_EMBEDDER=fastembed` oder `candle`, lokale Dateien wie in `BETRIEB-B.md` unter `DL_KNOWLEDGE_MODELS`. Das Query-Modell muss zum Fingerabdruck der aktiven Indexgeneration passen.

Reranker: `DL_KNOWLEDGE_RERANK_MODELS` benennt ein lokales Verzeichnis mit `model.onnx`, `config.json`, `tokenizer.json`, `tokenizer_config.json` und `special_tokens_map.json`. Ohne Angabe wird `~/.local/share/dl-knowledge/models/ms-marco-MiniLM-L6-v2` verwendet. Gemessener Modellkandidat ist `cross-encoder/ms-marco-MiniLM-L6-v2`, Revision `233902d25c440f23af6f7d6e94d2946bac0bee0a`. Die endgültige Modellwahl bleibt beim Nutzer; Messung und Freigabegrenzen stehen in `MESSUNG-C.md`.

Die Dateien werden ausschließlich lokal geöffnet; fehlende oder leere Dateien führen zum Fehler. Der Dienst hat keinen Modell-Downloadpfad. Der Fingerabdruck umfasst Modell- und Tokenizer-Dateien sowie Tokenlimit und Batchgröße. Der austauschbare `Reranker`-Trait verlangt genau einen endlichen Relevanz-Score pro Eingabedokument.

Modelle werden einmalig geladen. Synchrone Inferenz findet im Antwortpfad in einem Blocking-Worker statt, nicht auf dem asynchronen HTTP-Executor. Pro Dienstinstanz läuft höchstens eine Hybrid-Anfrage gleichzeitig, ohne wachsende Warteschlange. Belegte Kapazität, Zeitüberschreitung, ungültige Scores, Modellfehler, fehlender aktiver Index oder SQL-Fehler ergeben `answerable:false`; es gibt keinen stillen Rückfall auf ungefiltertes BM25.

Ein abgebrochener Request oder Timeout kann eine bereits laufende ONNX-Inferenz nicht hart unterbrechen. Die Kapazität bleibt bis zu deren tatsächlichem Ende belegt. Danach wird kein verspätetes Ergebnis mehr an den aufrufenden Request gegeben. Das Modell erzeugt im Leerlauf keine Inferenzjobs.

Die ONNX-Laufzeit bestimmt ihre internen Threads aus der verfügbaren CPU-Affinität. Die Grenze von einer Anfrage ist daher nicht automatisch eine Grenze von einem CPU-Kern. Für den späteren Betreiberstart sollte die CPU-Affinität entsprechend der Messung begrenzt werden, beispielsweise auf zwei freigegebene CPUs. Paket C ändert dafür keine systemd-Unit.

## Getrennter Modell-Setup

Erst vor einem freigegebenen Deployment durch den Betreiber, nicht beim Dienststart:

```bash
model_dir="$HOME/.local/share/dl-knowledge/models/ms-marco-MiniLM-L6-v2"
revision=233902d25c440f23af6f7d6e94d2946bac0bee0a
base="https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2/resolve/$revision"
install -d -m 0750 "$model_dir"
for file in config.json tokenizer.json tokenizer_config.json special_tokens_map.json; do
  curl --proto '=https' --tlsv1.2 --fail --location "$base/$file" --output "$model_dir/$file" || exit 1
done
curl --proto '=https' --tlsv1.2 --fail --location "$base/onnx/model.onnx" --output "$model_dir/model.onnx" || exit 1
```

Die fünf SHA-256-Werte stehen in `MODEL-C.sha256`. Die Installation muss sie im Modellverzeichnis mit `sha256sum --check` prüfen. C hat seine Modellkopie nur unter dem ignorierten `rust/target/paket-c/models/` im eigenen Worktree abgelegt.

## Offline-Prüfung und Messung

Aus `rust/`:

```bash
PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= \
  cargo build --release --package dl-knowledge --jobs 2
./target/release/dl-knowledge hybrid help
```

`hybrid check` liest ein JSON-Objekt über stdin, lädt ausschließlich den lokalen Reranker und gibt dessen Fingerabdruck und Scores zurück:

```json
{"question":"Wie verknüpfe ich Steam?","documents":["Steam mit Discord verbinden.","Ein Turnier anmelden."]}
```

`hybrid bench <Eval-Verzeichnis> <Bericht.json> [Runden] [Start] [Anzahl]` lädt und validiert alle 224 unveränderten Fälle der sechs öffentlichen Golden-Dateien. Ohne Bereichsangabe misst es die gesamte Suite, ansonsten den ausdrücklich angegebenen nullbasierten Teilbereich. Für die C-Abnahme wurden zwei nicht überlappende Bereiche mit je 112 Fällen gemessen. Er ist nur mit `deadlock_test`, dem Testskript-Marker und einer leeren Indexgenerationstabelle erlaubt. Er baut und aktiviert ausschließlich in dieser frischen Wegwerf-Datenbank einen Testindex. Es erfolgen keine Aufrufe der Antwortgeneration.

```bash
for start in 0 112; do
  part=$((start / 112 + 1))
  PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 RAYON_NUM_THREADS=2 \
    DL_KNOWLEDGE_EMBEDDER=fastembed \
    DL_KNOWLEDGE_HYBRID_OPTIONS='{"timeout_ms":10000,"fusion_k":12}' \
    DL_KNOWLEDGE_RERANK_MODELS="$(pwd)/target/paket-c/models/ms-marco-MiniLM-L6-v2" \
    bash scripts/central_test_db.sh \
    taskset -c 0,1 target/release/dl-knowledge hybrid bench \
    /home/nathanael/repos/Deadlock-Docs/evals \
    "../.tasks/2026-09-17-rag-ausbau-visual-brain/MESSUNG-C-$part.json" 1 "$start" 112 || exit 1
done
python3 ../.tasks/2026-09-17-rag-ausbau-visual-brain/auswertung-c.py \
  ../.tasks/2026-09-17-rag-ausbau-visual-brain/MESSUNG-C.json \
  ../.tasks/2026-09-17-rag-ausbau-visual-brain/MESSUNG-C-1.json \
  ../.tasks/2026-09-17-rag-ausbau-visual-brain/MESSUNG-C-2.json
```

Die vollständige JSON-Ausgabe wird erst nach erfolgreichem Abschluss atomar geschrieben. Fallweise JSONL-Checkpoints und gesonderte Fortschrittsdateien sind keine fertigen Messberichte. Teilberichte weisen ihren Bereich ausdrücklich aus; das Zusammenführen verlangt dieselben Modelle und Daten sowie jeden Fall genau einmal. Das Zehn-Sekunden-Budget im Messkommando ist nur für die Auswertung gedacht, nicht das Drei-Sekunden-Defaultbudget des Dienstes. Quellen-Recall, Ränge, lexikalisch abgelehnte Passagen und die Existenz einer zulässigen, serverseitig renderbaren Auswahl werden getrennt ausgewiesen. Ein möglicher Golden-Beleg ist keine tatsächlich vom Sprachmodell ausgewählte Antwort.

## Freigabe und Rückweg

Vor einer Produktionsnutzung bleiben die pgvector-Installation und administrative Migration aus B nötig. C führt beim Dienststart keine Migration und keine Aktivierung aus. D muss den Vergleich gegen BM25, die offenen Golden-Erwartungen und die vollständige Antwortabnahme bewerten. B und C werden gemeinsam reviewt; keine Umschaltung allein wegen erfolgreicher Unit-Tests.

Nach einer späteren ausdrücklichen Freigabe bleibt der Rückweg der ausgeschaltete Hybrid-Schalter beim Betreiberstart. Die Index-Aktivierung und Rückschaltung zwischen Generationen bleiben die expliziten Compare-and-Swap-Kommandos aus B. Kein automatischer Merge, Restart oder Rollback des laufenden Produktionsdienstes wurde ausgeführt.
