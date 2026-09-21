# Öffentliche Wissenssuche

Der Dienst lädt ausschließlich den bestehenden öffentlichen Corpuspfad. `--config <JSON>` erlaubt Loopback-Bindung, einen reinen Retrievaldienst und ein ausdrücklich konfiguriertes lokales Suchprofil. Ein anderer produktiver Corpuspfad ist nicht zulässig. Ohne `hybrid` bleibt der bisherige lexikalische Abruf aktiv.

```json
{
  "bind": "127.0.0.1:8896",
  "retrieval_only": true,
  "hybrid": {
    "embedding_model": "/absoluter/geprüfter/modellpfad",
    "embedding_profile": "mini_lm",
    "reranker_model": "/absoluter/geprüfter/rerankerpfad",
    "options": { "rerank": false, "timeout_ms": 3000 }
  }
}
```

Das Beispiel ist keine Aktivierungsempfehlung. Das konkrete Profil wird erst nach gleicher Corpusversion, unabhängigen Fragen und tatsächlichen Antwortproben freigegeben. `multilingual_e5_small` kodiert Dokumente mit `passage: ` und Fragen mit `query: `; Modellbytes, Präfixvertrag und Tokenlimit bestimmen einen eigenen Fingerprint. Gleiche Vektordimension bedeutet keine Austauschbarkeit. Ein Modellwechsel baut eine neue Indexgeneration.

Die produktive Datenbankverbindung verwendet bei fehlendem `database_url` den bestehenden zentralen Infisical-Connector. Explizite Adressen sind ausschließlich lokale Peer-Verbindungen ohne Passwort. Es werden keine neuen Umgebungsvariablen oder Zugangsdaten benötigt.

## Snapshotwechsel und kuratierte FAQ

`POST /internal/reload` bleibt der vorhandene Loopback-Vertrag. Eine angenommene Anfrage besitzt ihre Arbeit auch nach einem Clientabbruch. Ein zweiter gleichzeitiger Reload erhält `409 reload_busy`. Indexaufbau geschieht nebenläufig zum alten vollständigen Snapshot; erst die gemeinsame Veröffentlichung von Densegeneration, lexikalischem Index und FAQ belegt kurz den Schreibzugriff. Ein fehlgeschlagener Aufbau veröffentlicht nichts.

`GET /healthz` bestätigt `generation` als Hash der tatsächlich geladenen öffentlichen HTML-Dateien und `faq_generation` als SHA256 der tatsächlich geladenen `faq-manifest.json`-Bytes; ein fehlendes Manifest ergibt `null`. Ein externer Export darf erst nach Übereinstimmung beider Werte den neuen Stand bestätigen. Eine reine FAQ-Änderung benötigt ebenfalls Reload.

Das FAQ-Manifest liegt neben `public/`. Es enthält ausschließlich geprüfte wörtliche Fragen, öffentliche Dateipfade, Abschnitts-IDs und vollständige Quelldateihashes. Exakter Vergleich normalisiert nur Großschreibung und Leerraum. Negation, Satzzeichen, Reihenfolge und angehängte Anliegen bleiben bedeutungstragend. Der Schnellpfad liefert Quellen, keine fertig generierte Antwort.

## Reproduzierbare Offlineabnahme

`public-bench <JSON>` verwendet einen ausdrücklich ausgewählten öffentlichen Snapshot und ausschließlich eine eigene lokale Datenbank mit Präfix `dl_knowledge_eval_`. Konfigurationsfelder: `settings` wie oben, `public_snapshot`, `evaluation`, `report`, optional `first`, `count` und `holdout`. Der Basismodus verlangt unverändert die vollständigen sechs Dateien und mindestens 224 Basisfälle. Der getrennte Holdoutmodus verlangt ausschließlich als `synthetisch` markierte Fälle. Archivierte Zielseiten werden als Corpusentzug ausgewiesen; sie werden nicht als Suchverlust gezählt.

Die Berichte trennen BM25, Dense, Fusion, Reranking und die tatsächliche Ausgabe ganzer Belege. Hohe Relevanz oder richtige Quellen-IDs beweisen keine Beantwortbarkeit. Reale Antworten des unveränderten zentralen Generators werden separat geprüft.

`public-shadow <JSON>` startet dieselben HTTP-Routen auf einem freien Loopback-Port gegen einen vorbereiteten Snapshot. Felder: `bind`, `public_snapshot`, `settings`. `settings: null` aktiviert die unveränderte lexikalische Route als Vergleich; konfigurierte Hybridprofile dürfen nur eine eigene lokale Testdatenbank nutzen. Antwortgenerierung ist in diesem Testdienst abgeschaltet. Anschließend den Testdienst beenden und die eigene Testdatenbank entfernen.

Für die PostgreSQL-Tests kann `tests/database.example.json` als lokale `tests/database.local.json` kopiert werden. Der vorhandene zentrale Testharness legt daraus isolierte Testdatenbanken an und räumt sie wieder auf. Die lokale Datei bleibt unversioniert.
