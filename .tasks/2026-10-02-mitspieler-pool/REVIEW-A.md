# Review Spielerpool A

## Paket A, Runde 1

Urteil des vorgeschriebenen Merge-Gates: BLOCK. Geprüfter Code-Stand: `f6454e7a` auf `feat/spielerpool-a`, Basis `origin/main`.

### Blockierender Befund: Steam-Herkunft von Mitspieler-Paaren fehlt

`rust/crates/dl-pool/src/store.rs:458`: `store_co_player` prüft nur, ob beide Nutzer aktuell ein bestätigtes Steam-Konto besitzen. Wird Konto A entkoppelt und anschließend Konto B verknüpft, kann ein noch ausstehendes Paar-Ergebnis für Konto A erfolgreich geschrieben werden. Dadurch entstehen Beziehungen aus dem alten Konto erneut, obwohl der Invalidierungstrigger sie nach dem Entkoppeln gelöscht hat.

Die Herkunft beider Steam-Konten fehlt auch in `rust/crates/dl-pool/src/types.rs:153` (`CoPlayer`) und `rust/crates/dl-central-db/migrations/20261002000000_spielerpool.sql:78` (`pool.co_players`).

Der Gate verlangt, beide ursprünglichen Steam-Identitäten mitzuführen und unter den Transaktionssperren gegen die aktuell verwendeten Konten zu prüfen. Ein Test muss das Entkoppeln, die Verknüpfung eines anderen Kontos und den anschließenden verspäteten Schreibversuch abdecken.

### Übergabe

Gemäß Auftrag wurden nach BLOCK keine eigenen Codekorrekturen vorgenommen. Eine spätere Korrektur muss notwendige Tabellen- oder API-Änderungen in `VERTRAG-A.md` nachziehen. Die bisherigen 17 grünen DB-Tests decken den beschriebenen Kontowechsel nicht ab. Lint- und Baselinebefunde stehen in `PRUEFUNG-A.md`.

Merge, Produktionsmigration, Release-Build, Neustart und Live-Prüfung stehen aus. Eigene Host- oder Repo-Locks wurden nicht erworben. Der fremde Repo-Lock bei PID `3643980` bleibt unangetastet; PID `3597035` war bei der letzten Prüfung beendet. Der reservierte Release-Slot wurde nicht genutzt.

### Gesicherter Arbeitsstand

- Worktree: `/home/nathanael/.worktrees/Deadlock-Bots-pool-a`.
- Branch: `feat/spielerpool-a`, eigener Remote-Branch gepusht.
- Implementierung und vom Gate geprüfter Code: `f6454e7a`.
- Bisherige Review-Artefakte: `c93ebbfa`.
- Schema: `rust/crates/dl-central-db/migrations/20261002000000_spielerpool.sql`; noch nicht in der zentralen Produktionsdatenbank angewendet.
- Datenschicht: `rust/crates/dl-pool/`; Privacy-Verdrahtung in `rust/crates/dl-community/src/privacy.rs`, Workspace- und Cargo-Abhängigkeiten sind enthalten.
- Vertrag: `VERTRAG-A.md`, einschließlich `own_sessions(Scope, limit, offset)` für eigene beendete Teilnehmer-Sessions mit eigenem Feedbackstand.
- Prüfungen: 14 Pool-DB-Tests und drei Pool-Privacy-Tests bestanden; strenger Pool-Clippy, Formatierung und Diff-Prüfung bestanden. Die unveränderten Community-Lintbefunde und der belegte Privacy-Baselinefehler sind in `PRUEFUNG-A.md` festgehalten.

### Verbindliche Wartevorgabe

Die neue ausdrückliche Nutzergrenze gilt: keine neuen Agenten oder Threads, keine automatischen Nachstarts und insgesamt höchstens zehn offene Threads. Der globale Sperrhinweis liegt unter `/home/nathanael/Documents/.tasks/2026-10-02-offene-branches/KEINE-NEUSTARTS`. B, C, E, D und frische Fixer starten vorerst nicht.

Paket A bleibt als bestehender Worker mit laufendem Auftrag erhalten. Branch und Worktree werden weder gelöscht noch archiviert. Der Worker führt keinen eigenen Fix nach BLOCK aus und startet keine Unter-Threads. Er settlet sich nicht selbst. Die Begrenzung der übrigen Threads liegt beim globalen Koordinator.

Fortsetzung erst auf ausdrückliche Nachricht des Orchestrators. Merge und Deploy benötigen weiterhin Gate-ALLOW und die erforderliche Freigabe. Bis dahin werden keine Produktionsmigration, kein Release-Build und kein Dienstneustart ausgeführt.
