# Gate-Runde 1: BLOCK

Geprüfter Code: `cdbf32cb`, Basis `origin/main` bei `600b832a`.

Der erste technische Versuch scheiterte ohne Urteil beim Erzeugen der Sandbox. Nach Freigeben der eigenen Testinstanz lieferte der Wiederholungsversuch ein inhaltliches Urteil von `gpt-6.1-sol`. Dieses Modell bleibt für die Folgerunde maßgeblich.

## Verifizierte offene Funde

1. `rust/crates/dl-community/src/invite_lounge.rs:635`: Der parallele Rückblick setzt `last_seen`, ohne eine Bitte anzulegen. Wenn er dieselbe frische Nachricht vor ihrem Gateway-Ereignis speichert, verwirft der Live-Pfad die Bitte als bereits verarbeitet. Der vorhandene CAS-Wiederholungsweg behebt diesen Fall nicht. Lösungsrichtung: Den Live-Cursor ausschließlich im Live-Pfad vorziehen. Historische Beobachtung von Code und Hinweis getrennt halten. Regression für beide Reihenfolgen Rückblick vor Gateway und Gateway vor Rückblick ergänzen.
2. `rust/crates/dl-community/src/invite_lounge.rs:446` und `:502`: Frischeprüfungen verwenden vor Datenbank-Wartezeiten aufgenommene Zeitpunkte. Eine Bitte kann während `load`, Claim oder `finish_attempt` ablaufen und trotzdem den externen Steam-Aufruf oder eine öffentliche Antwort erreichen. Lösungsrichtung: Die injizierbare Uhr unmittelbar vor dem jeweiligen Fremdaufruf erneut lesen. Den abgelaufenen Claim ohne Versand durch den Dienst abschließen. Auch den Hinweiszweig gegen Wartezeiten prüfen. Regressionen für Ablauf während DB-Zugriffen ergänzen.

## Clippy-Nacharbeit

`cargo clippy -p dl-community --all-targets --features testing --no-deps -- -D warnings` wurde auf main und auf dem Fix in demselben eigenen Worktree gemessen: 228 gegen 250 Diagnosen. Außerhalb von `invite_lounge_tests.rs` jeweils 134. Im Invite-Testmodul 94 gegen 116, also 22 zusätzliche `unwrap_used`-Diagnosen. Der produktive Watcher hat in beiden Läufen keine eigene Clippy-Diagnose. Die ergänzten und veränderten Testaufrufe müssen begründete `expect`-Aufrufe statt `unwrap` verwenden, ohne Lints abzuschalten. Das unveränderte `dl-central-db` scheitert zusätzlich in beiden Prüfrichtungen mit einer `explicit_auto_deref`-Diagnose in `platform_connections.rs:30`.

## Gate-Antwort

```text
[gpt-6.1-sol] BLOCK: Startup history can swallow live requests, and freshness checks can permit expired actions.
```

Die Details oben geben die beiden BLOCKING-Funde auf Deutsch wieder. Unveränderte Ausgabe: `/tmp/invite-fix-review-retry.log`.

Die Fundliste geht gemäß Arbeitsregel an einen neuen Fixer mit frischem Kontext. Dieser Blatt-Worker startet keinen weiteren Thread und repariert nicht im Implementierer-Kontext nach. Merge, Deploy und Live-Nachweis bleiben bis ALLOW offen.
