# Review Spielerpool A

## Paket A, Runde 1

Urteil des vorgeschriebenen Merge-Gates: BLOCK. Geprüfter Code-Stand: `f6454e7a` auf `feat/spielerpool-a`, Basis `origin/main`.

### Blockierender Befund: Steam-Herkunft von Mitspieler-Paaren fehlt

`rust/crates/dl-pool/src/store.rs:458`: `store_co_player` prüft nur, ob beide Nutzer aktuell ein bestätigtes Steam-Konto besitzen. Wird Konto A entkoppelt und anschließend Konto B verknüpft, kann ein noch ausstehendes Paar-Ergebnis für Konto A erfolgreich geschrieben werden. Dadurch entstehen Beziehungen aus dem alten Konto erneut, obwohl der Invalidierungstrigger sie nach dem Entkoppeln gelöscht hat.

Die Herkunft beider Steam-Konten fehlt auch in `rust/crates/dl-pool/src/types.rs:153` (`CoPlayer`) und `rust/crates/dl-central-db/migrations/20261002000000_spielerpool.sql:78` (`pool.co_players`).

Der Gate verlangt, beide ursprünglichen Steam-Identitäten mitzuführen und unter den Transaktionssperren gegen die aktuell verwendeten Konten zu prüfen. Ein Test muss das Entkoppeln, die Verknüpfung eines anderen Kontos und den anschließenden verspäteten Schreibversuch abdecken.

### Übergabe

Gemäß Auftrag wurden nach BLOCK keine eigenen Codekorrekturen vorgenommen. Der frische Fixer übernimmt den Befund und zieht notwendige Tabellen- oder API-Änderungen in `VERTRAG-A.md` nach. Die bisherigen 17 grünen DB-Tests decken den beschriebenen Kontowechsel nicht ab. Lint- und Baselinebefunde stehen in `PRUEFUNG-A.md`.

Merge, Produktionsmigration, Release-Build, Neustart und Live-Prüfung stehen aus. Eigene Host- oder Repo-Locks wurden nicht erworben. Der fremde Repo-Lock bei PID `3643980` bleibt unangetastet; PID `3597035` war bei der letzten Prüfung beendet. Der reservierte Release-Slot wurde nicht genutzt.

Fortsetzung erst nach dem Fixer und der Nachricht `[Freigabe] Merge, Deploy, Live-Prüfung, Aufräumen`.
