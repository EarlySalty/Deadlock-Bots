status: aktiv (2026-10-02)

# Briefing: Spielerpool B

[Orchestrator] Paket B: Interview beim Concierge-Onboarding und `/spielerprofil`.
Auftrag: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/AUFTRAG.md`.
WORKER-REGELN.md, PAKETE.md und VERTRAG-A.md im selben Ordner vollständig lesen.

- Worktree: `/home/nathanael/.worktrees/Deadlock-Bots-pool-b`
- Branch: `feat/spielerpool-b`
- Intent-Thread: `011b713c-266a-46b6-b0ce-1d5faa060234`
- Rolle: `worker_mittel`
- Startvoraussetzung: Paket A auf origin/main gemergt.

Vertrag A beachten: `Scope` trägt Guild und Discord-ID. `save_preferences`
speichert Entwürfe unsichtbar; erst ein abgeschlossenes Interview veröffentlicht
das Profil. `preferred_discord_ids` dürfen nur veröffentlichte Profile derselben
Guild enthalten. DM-Opt-in bleibt beim Speichern der Präferenzen erhalten.
Bei Datenschutz-Opt-out den vorhandenen bewussten Opt-in-Weg benutzen, niemals
den Grabstein umgehen. Technische IDs unverändert lassen, deutsche Beschriftungen
für die festgelegten Enum-Werte liefern.

Du besitzt `rust/crates/dl-community/src/pool_interview.rs`, den minimalen
Onboarding-Einhänger in `concierge.rs`, die Registrierung von `/spielerprofil`
und nötige Modul- und Abhängigkeitsverdrahtung in dl-community. privacy.rs gehört A.

Beim Onboarding gilt vorerst die bestehende Concierge-Test-Allowlist. Die genannte
Kennung `DL_CONCIERGE_TEST_USER_ALLOWLIST` ist kein Auftrag, neue ENV-Config zu bauen.
Bestehenden Config- und Infisical-Weg benutzen. `/spielerprofil` ermöglicht den
beauftragten Test; die Guild-Mitgliedschaft serverseitig prüfen.

Das Interview fragt wann, was, mit wem und wie viele. Auswahlmenüs und Knöpfe
bevorzugen, Antworten dauerhaft und strukturiert je Discord-ID speichern.
Bestehende Präferenzen beim erneuten Aufruf vorladen und sinnvoll ändern können.
Rang, Stunden und Spiele nicht selbst abfragen; sie kommen aus dem Ingest.
Den bestehenden Concierge und sein Brain-Wissen einbinden, zentrale AI-Anbindung
erhalten. Keine eigenen Modellclients oder erfundenes Spielwissen.
Textkanal #mitspieler-suche und bestehendes LFG-Verhalten bleiben außerhalb des Auftrags.

Prüfe den vollständigen Interviewweg samt Persistenz und zulässigem Testzugang.
Ungeprüfte Nutzeraktionen in der Meldung benennen. Bei Gate-ALLOW selbst abschließen;
Merge mit dem Delegator koordinieren, damit Deploy und Release-Build nicht kollidieren.
