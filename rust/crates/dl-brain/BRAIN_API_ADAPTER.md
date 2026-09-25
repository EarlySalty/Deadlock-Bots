# Typisierter Brain-Command-Adapter

Stand: 2026-09-25. Implementiert, **nicht im Bot verdrahtet**. Der bestehende `AiAnswerer`-Port erhält eine zusätzliche Implementierung `brain_api::BrainApiAnswerer`.

Der Adapter verwendet ausschließlich den kanonischen Async-Client aus Deadlock-Brain, gepinnt auf `bdcc6dec3424bd313d36e5f545de2a07df564c7f` (Brain PR #39 / CODEX A PR #38). Er erhält Endpunkt, Bearer-Token, Timeout, eindeutigen opaken Prozess-Namespace und Scope-Bindungen ausdrücklich vom späteren Composition Root. Keine neue Umgebungsvariable wird automatisch ausgewertet; keine Produktionskonfiguration wurde geändert.

## Unterstützter Pfad

`handle_brain_query` → bestehender `AiAnswerer`-Port → `BrainApiAnswerer` → typisierter `POST /v1/answer`.

Die bisherigen Command-Checks und Ausgabewege bleiben unverändert: Usage, Fragenlänge, Benutzer-Cooldowns, Backendfehler ohne verbrauchten Cooldown sowie Discord-Chunking. Der Adapter hält vier parallele Slots und einen expliziten Gesamt-Timeout einschließlich Slot-Wartezeit. Er übernimmt belegte Antworttexte ohne zusätzliche Modellverarbeitung; URL-haltige oder über 3800 UTF-16-Einheiten lange Antworten werden nicht als gültige GameOnly-Ausgabe ausgegeben. Fehlende Belege bleiben `NoAnswer`; Provider-, Budget- und ACL-Fehler bleiben Backendfehler, keine Ersatzantwort.

Der bestehende Port enthält keine vertrauenswürdige Conversation-ID. Deshalb erzeugt jede Anfrage eine separate Einmal-Conversation aus Namespace und monotoner Sequenz; er vermischt keine Nutzerhistorien. Der Namespace muss je Instanz eindeutig sein. Bearer-Authentifizierung und ACL bleiben serverseitig; Scope-Bindungen dürfen nicht aus einem Chattext kommen. Der lokale Konstruktor sperrt externe HTTP- und HTTPS-Ziele.

## Bewusst nicht umgestellt

| Bestehender Pfad | Grund |
| --- | --- |
| `dl-bot::modglue::SharedBrainAnswerer` einschließlich Builds | Runtime-Composition und Build-Publishing bleiben bestehen; der aktuelle API-Vertrag hat keinen Publishing-Endpunkt. |
| `dl-answer` FAQ/Concierge und OpenTest | Historie, Intent-/Patenaktionsmetadaten, Source-Kind/URLs und Spezialformatierung sind im öffentlichen Vertrag nicht vollständig abgebildet. |
| `dl-knowledge` Retrieval-API | Eine fertige Brain-Antwort darf nicht als angeblich rohe Retrieval-Evidenz an ein zweites Modell weitergereicht werden. |
| Feeder, Outbox, Community-Aktionen | Daten-/Aktionsproduzenten, keine im Adapter neu zu bauenden Antwort-Nebenwege. |

Diese Lücken benötigen vor einem vollständigen Cutover zusätzliche **Vertrags-/Codearbeit**. Sie werden nicht als reine Runtime-Prüfung ausgegeben. Bis dahin bleiben die alten Featurepfade unangetastet; der neue Adapter enthält keinen direkten Modell- oder RAG-Fallback.

## Reproduzieren

```sh
cargo fetch --manifest-path rust/Cargo.toml --locked
bash rust/scripts/check-brain-consumer.sh
```

Rust 1.97.1 mit rustfmt/clippy. Die Suite testet den existierenden Dispatcher mit einem echten lokalen HTTP-Fixture und dem neuen Adapter. Keine Bot- oder Knowledge-Binary wird gestartet, keine echte Nachricht verschickt. Die CI nutzt dieselben Tests, entfernte Anwendungsumgebung, versioniertes Lockfile, read-only Rechte und eng begrenzte Testartefakte. Das vorgeschlagene Release-Gate ist report-only und aktualisiert oder mergt keinen Branch.

## Lokale Übergabe

Claude verbindet erst nach Vertragsentscheidung den vorgesehenen `AiAnswerer`, prüft separate Tokens/Scopes, eindeutige Namespaces, Rollen-/ACL-Widerruf, Dispatcher-Cooldowns und Chunking in einer isolierten echten Runtime. Queue-Abbruch, Timeouts, leere Evidenz und unbekannte/fehlgeschlagene Antworten prüfen. Build-/FAQ-/Concierge-Parität darf nicht aus den Command-Fixtures abgeleitet werden. Kein Produktivservice wurde verändert oder neu gestartet.
