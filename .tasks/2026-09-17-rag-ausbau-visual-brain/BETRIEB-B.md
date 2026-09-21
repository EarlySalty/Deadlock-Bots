# Paket B: Aufbau und Betrieb

## Produktionsgrenze

Ohne das Subcommand `dense` lädt der Dienst weder Embedding-Modelle noch einen Dense-Datenbankpool. `/public/v1/ask`, BM25, Grounding, Passagenaufteilung und die bestehende Produktionsvalidierung bleiben unverändert. Paket B startet keinen Produktionsdienst und aktiviert dort keinen Index.

## Voraussetzungen des Betreibers

Vor dem Deployment muss der Betreiber `postgresql-16-pgvector` auf dem PostgreSQL-16-Host installieren. Das benötigt root und wurde durch Paket B nicht durchgeführt. Das vorhandene Testimage `timescale/timescaledb:2.17.2-pg16` enthält vector bereits.

Die Migration `2026091802_knowledge_dense.sql` läuft mit einer administrativen Migrationsrolle. Der Dienst führt keine Migration aus. Die NOLOGIN-Gruppenrolle `dl_knowledge_dml` erhält Schema-USAGE, DML auf Generationen und Embeddings, SELECT/UPDATE auf dem Aktivierungsverweis sowie USAGE/SELECT auf der Identitätssequenz. Sie erhält kein CREATE. Der Betreiber ordnet eine nichtadministrative Dienst-Anmelderolle dieser Gruppe zu. Eine zusätzliche Gruppenmitgliedschaft entfernt keine bereits bestehenden weitergehenden Rechte.

```sql
GRANT dl_knowledge_dml TO <Dienst-Anmelderolle>;
```

Die Verbindung verwendet den bestehenden zentralen Pool und `DEADLOCK_CENTRAL_DSN`. Zugangsdaten gehören nicht in diese Akte oder in Prozessargumente.

## Modelle einmalig bereitstellen

Beide nichtquantisierten Formate stammen aus `sentence-transformers/all-MiniLM-L6-v2`, Revision `1110a243fdf4706b3f48f1d95db1a4f5529b4d41`.

```bash
model_dir="$HOME/.local/share/dl-knowledge/models/all-MiniLM-L6-v2"
revision=1110a243fdf4706b3f48f1d95db1a4f5529b4d41
base="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/$revision"
install -d -m 0750 "$model_dir"
for file in config.json tokenizer.json tokenizer_config.json special_tokens_map.json model.safetensors; do
  curl --proto '=https' --tlsv1.2 --fail --location "$base/$file" --output "$model_dir/$file" || exit 1
done
curl --proto '=https' --tlsv1.2 --fail --location "$base/onnx/model.onnx" --output "$model_dir/model.onnx"
```

Dieser Setup-Schritt ist vom Dienststart getrennt. Fehlende lokale Dateien ergeben einen Fehler, keinen Download. `DL_KNOWLEDGE_MODELS` kann für die Dense-CLI ein anderes lokales Modellverzeichnis nennen. Bei fastembed sind die Hub-Funktionen deaktiviert. Das Cargo-Feature für den ONNX-Runtime-Download betrifft ausschließlich das Bauen.

Beide Laufwege verwenden maximal 256 Tokens, Batchgröße 16, maskiertes Mean-Pooling und L2-Normalisierung. Der Fingerabdruck umfasst die gelesenen Modelldateien, Laufwegversion und Verarbeitungseinstellungen. Ein Laufwegwechsel benötigt eine eigene Generation; eine Query mit fremdem Fingerabdruck wird abgewiesen.

## CLI für C und D

Aus `rust/`:

```bash
cargo build --release --package dl-knowledge --all-features --jobs 2
./target/release/dl-knowledge dense help
./target/release/dl-knowledge dense index fastembed
```

`index` verwendet ausschließlich den bestehenden `DEFAULT_DOCS_PATH`. Die JSON-Antwort nennt `index_generation`, `chunks`, `computed` und `reused`. Neue Generationen bleiben inaktiv. Fehler rollen den gesamten neuen Lauf zurück. Nach bestandenem Offline-Vergleich kann der Verantwortliche explizit aktivieren:

```bash
./target/release/dl-knowledge dense activate 12 none
./target/release/dl-knowledge dense activate 13 12
./target/release/dl-knowledge dense activate 12 13
```

Die Zahlen sind Beispiele. Das zweite Argument ist der erwartete bisherige Stand, `none` nur beim ersten Index. Unvollständige Generationen und inzwischen geänderte aktive Stände werden abgelehnt. Alte Generationen bleiben für eine Rückschaltung erhalten.

```bash
printf '%s\n' 'Wie verknüpfe ich Steam mit Discord?' |
  ./target/release/dl-knowledge dense search fastembed 6
```

Der Query-Text kommt über stdin, wird nicht protokolliert und nicht an Postgres übertragen. Die Suche bindet nur den lokalen Vektor. Treffer enthalten `index_generation`, `chunk_id`, `doc_path`, `title`, `section`, `text`, `stand`, `quelle` und Cosine-`score`. Es ist eine exakte Suche innerhalb der am Suchbeginn gelesenen aktiven Generation, ohne approximativen Index.

## Indexvertrag und Anschluss durch C

Die Chunk-ID besteht aus Dokumentpfad, Abschnittsname und Vorkommensnummer gleichnamiger Abschnitte. Andere Dokumente und reine Textänderungen verändern die ID nicht. Umbenannte oder zusätzlich eingefügte gleichnamige Abschnitte können neue Zuordnungen erzeugen. Der Inhalts-Hash umfasst genau Titel, Abschnitt und vorhandenen Chunk-Text als Embedding-Eingabe. Reine Änderungen an `stand` oder `quelle` übernehmen den Vektor und aktualisieren die Metadaten.

Ein Postgres-Transaktionslock serialisiert Indexbauten. Vollständige Generationen desselben Modell-Fingerabdrucks dienen als Cache. Alte Generationen werden nicht automatisch gelöscht. Vor der ersten ausdrücklichen Aktivierung gibt es noch keinen aktiven Index, danach genau einen Aktivierungsverweis.

C erhält `Embedder`, `dense::local::load`, `dense::from_chunks`, `store::build_generation`, `store::activate` und `store::dense_search`. B führt synchrone CPU-Inferenz nur in der CLI aus. Beim späteren HTTP-Anschluss muss C eine dauerhaft geladene Modellinstanz, begrenzte Parallelität und einen separaten Blocking-Worker vorsehen. Für erste CLI-Läufe auf diesem Host ist dieselbe CPU-Begrenzung wie in der Messung sinnvoll.

## Messung wiederholen

Aus `rust/`, zunächst mit `backend=fastembed`, anschließend mit `backend=candle`. Jede Ausführung des Testskripts erzeugt eine eigene Wegwerf-Datenbank. `dense bench` verweigert andere Datenbanknamen als `deadlock_test` sowie eine bereits gefüllte Generationstabelle.

```bash
backend=fastembed
task_dir="$(pwd)/../.tasks/2026-09-17-rag-ausbau-visual-brain"
PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= \
  cargo build --release --package dl-knowledge --no-default-features \
  --features "dense-$backend" --jobs 2 || exit 1
install -D target/release/dl-knowledge "target/paket-b/dl-knowledge-$backend" || exit 1
PATH="$HOME/.cargo/bin:$PATH" RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 RAYON_NUM_THREADS=2 \
  bash scripts/central_test_db.sh \
  taskset -c 0,1 "target/paket-b/dl-knowledge-$backend" \
  dense bench "$backend" /home/nathanael/repos/Deadlock-Docs/evals \
  "$task_dir/MESSUNG-B-$backend.json"
```

Vor Verwendung neu heruntergeladener Modelle ist `MODEL-B.sha256` mit `sha256sum --check` aus dem Modellverzeichnis zu prüfen. Die sechs Dateien der Messung wurden damit erfolgreich geprüft.

Messergebnisse und Grenzen stehen in `MESSUNG-B.md`, Testnachweise und die offene Golden-Abnahme in `TESTS-B.md`.
