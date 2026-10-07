# K: requestgebundener Privatfix

Stand: 8. Oktober 2026, Umsetzung. ENTSCHEIDUNG-K-PRIVATFIX-0015.md ersetzt die früheren Projektions- und Threadrechtevoraussetzungen. Wache 33 bestätigt diese Entscheidung. Keine neue Produktfreigabe nötig.

## Verbindlicher Schnitt

Beide Öffentlichkeitssperren in answer_discord_event entfernen. Bei can_reply im ursprünglichen event.channel_id oder in der DM antworten. Für private Kanal-/Thread-/DM-Anfragen Discord-Nachrichtenlesen anderer Personen requestgebunden ausschalten, einschließlich Tool- und Evidencepfad. Eigene Frage, Spiel-/Server-/Dokuwissen und eigener erlaubter Invite-Status bleiben möglich. Öffentliche Kanäle unverändert. Kein Game-only-Fix, neuer Resolver oder Projektionsbau als Voraussetzung.

## Konkrete technische Naht

Der bestehende AsyncBrainClient auf 7da630186fe7d55a7eef4156192dcac78002b008 übermittelt die Personenbindung, aber keine requestgebundene Readrestriktion. Bloßes Entfernen der Guards würde Discordlive bei relevanten Fragen weiter aufrufen.

Begrenzter Anschlussvorschlag im vorhandenen Pfad: answer_for_discord_with_read_access(&Query, u64, bool); bei false restriktiver Header x-discord-read-access: disabled. HTTP/API übernehmen die Restriktion bei erhaltener Personen-/Actorbindung. Interner Anfragekontext und bestehender Discordlivepfad setzen sie vor Reads, Tools und Evidencefreigabe durch. Bestehender öffentlicher Default unverändert. Kein Query.answer_context, kein neuer Connector, kein Modell-/Timeoutwechsel. Die konkrete Form bleibt bis Sourceprüfung ein Vorschlag, keine fertige Fähigkeit.

Eigener sauberer Brainworktree /home/nathanael/.worktrees/brain-k-private-read-20261008, Branch fix/brain-private-read-access-20261008, frisch von origin/main b7289d11. Ungeprüfter Orts-WIP liegt unverändert im bisherigen Brain-/Twitchbaum und in privater bytegebundener Retention. Derselbe native Worker a3d648bce719faf6c führt den Auftrag fort. Keine Doppelwriter oder neuen T3-Threads. Botsconsumer erst auf einen gesicherten kompatiblen Commit pinnen.

## Tatsächlicher Baselinebeleg

Hauptsession führte aus:

```text
SQLX_OFFLINE=true /home/nathanael/.local/bin/cargo-slot +1.97.1 test --manifest-path /home/nathanael/.worktrees/bots-k-live-20261007/rust/Cargo.toml --target-dir /tmp/k-privatfix-bots-check-target-20261007 -p dl-brain --locked --offline --jobs 3 --no-fail-fast -- --include-ignored
```

Vollständiger Log: /home/nathanael/.worktrees/brain-k-live-20261007/.tasks/2026-10-07-brain-grafik-ki/K/privatfix-bots-baseline-gate-1.log. Unabhängig ausgewertet: Exit 0, 21 passed, 0 failed, 0 ignored, 0 filtered; zwei weitere Ziele mit jeweils 0 Tests. Kompilation 58,42 Sekunden. Unveränderte Botsquelle, kein Privatfixbeweis. Runner tatsächlich nach Nutzerreparatur a593c5d ausgeführt; konkrete Main-Hookwirkung noch offen.

## Erhaltene Grenzen

Verweigertes mcp.rs und verweigerte Logs nicht über andere Werkzeuge oder Worker lesen. Threadrechte-/Mitleseprojektion ist ein späteres Paket, keine Voraussetzung dieses Fixes. Keine fremden IDs oder Community-Rohdaten an Codiermodelle oder Git, keine Secretsuche, keine Datenbank-Handkorrektur. Alte K-Braininstallation nicht als aktuellen Gesamtstand aktivieren. Regulärer Deploy gegen aktuellen origin/main, Schutz-Hooks nicht umgehen.

## Offener Beweis

Staff-/Thread-/DMfälle müssen Brainantwort am Eingang und ausbleibenden Discordlive-/read_messages-Zugriff zeigen. Danach regulärer Gate, Merge, Brain-/Botslieferung, Neustart und echte Nutzerprobe. Bisher kein Privatfixsourcecommit, Main-Hooklauf, Privatfixdeploy oder Privatfixlivebeweis.

TESTNACHWEIS[TW-1]: 21 passed, 0 ignored | Baseline: unveränderte Botsquelle 0 rot
MERGEPROTOKOLL[MS-1]: 2 Git-Schritte einzeln | Anläufe: 0 | Gate: frischer Mainfetch und isolierter Brainworktree, noch kein Sourcekandidat
