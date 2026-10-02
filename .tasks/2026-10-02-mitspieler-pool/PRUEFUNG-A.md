# Prüfung Paket A

Stand: 2026-10-02, Gate-Runde 1 BLOCK, vor Merge.

Ausgangspunkt: frisch geholtes `origin/main`, Commit `47ea7994`. Eigenes Worktree `/home/nathanael/.worktrees/Deadlock-Bots-pool-a`, Branch `feat/spielerpool-a`. Fremde Änderungen im geteilten Checkout bleiben erhalten.

## Grüne Prüfungen

- `cargo check -p dl-pool`: bestanden.
- `cargo test -p dl-pool -- --test-threads=2`: 14 Integrationstests bestanden, keine fehlgeschlagenen oder übersprungenen Tests.
- `cargo test -p dl-community pool_privacy_tests -- --test-threads=2`: drei Tests bestanden. Belegt sind vollständige Löschung über alle Guilds, Entfernen einer gemeinsamen Session mit fremdem Feedback, redigierter eigener Export und gemeinsamer Rollback nach einem späteren Fehler.
- `cargo clippy -p dl-pool --all-targets -- -D warnings`: bestanden.
- `cargo fmt --package dl-pool --package dl-community -- --check`: bestanden.
- `git diff --check`: bestanden.
- Schreibprüfung des Vertrags mit `no-em-dashes`: bestanden; anschließend auf natürliche Formulierungen geprüft.

Die aktuelle Toolchain wird explizit über `/home/nathanael/.cargo/bin/cargo +stable` und die Compiler-Konfiguration `build.rustc` gewählt. Das Standard-Cargo im Login-Shell-Pfad ist zu alt für die vorhandene Lockfile-Version 4. Compilerwahl erfolgt per Cargo-Konfiguration, ohne neue Umgebungsvariablen. Beim Baseline-Gegenbeweis wurde dasselbe Test-Target kurz für den unveränderten Stand verwendet. Dadurch blieb bei einem anschließenden Lauf ein altes eingebettetes Migrationsartefakt übrig; die drei Pool-Privacy-Tests meldeten fehlende Pool-Tabellen. Danach wurden ausschließlich die `dl-central-db`-Artefakte im eigenen Test-Target entfernt und die Tests frisch gebaut. Die erneute Prüfung verwendet wieder die vollständige neue Migration. Produktivdaten wurden dabei nicht berührt.

Die Datenbanktests verwenden eine secretfreie, ignorierte `rust/test-database.json` mit lokaler Socket-/Peer-Verbindung und dem vorhandenen TestDb-Harness. Jeder DB-Test erstellt eine frisch migrierte isolierte Datenbank und entfernt sie danach.

## Vorhandener Fehler außerhalb des Pools

`cargo test -p dl-community privacy_contract_tests -- --test-threads=2` besteht neun von zehn Tests. Der Test `privacy::privacy_contract_tests::alle_migration_user_id_spalten_sind_im_privacy_vertrag` meldet elf fremde Einordnungen, keine Pool-Spalte:

- `activity.twitch_invite_evidence_queue.user_id`
- `activity.twitch_invite_members.user_id`
- `activity.twitch_invite_messages.user_id`
- `bot.twitch_invite_joins.inviter_twitch_user_id`
- `bot.twitch_invite_joins.streamer_twitch_user_id`
- `bot.twitch_invite_joins.user_id`
- `bot.twitch_personal_invites.inviter_twitch_user_id`
- `bot.twitch_personal_invites.streamer_twitch_user_id`
- `bot.twitch_streamer_invite_code_history.twitch_user_id`
- `patchnotes.guild_dispatch.approved_by_user_id`
- `patchnotes.guild_settings.updated_by_user_id`

Gegenbeweis: derselbe exakte Test wurde in einem separaten unveränderten Worktree auf `47ea7994` ausgeführt. Er meldet exakt dieselben elf Spalten, im Ausgangsstand `rust/crates/dl-community/src/privacy.rs:4690`. Der temporäre Baseline-Worktree ist anschließend entfernt worden. Der Test wird weder abgeschwächt noch werden fremde Datenmodelle innerhalb Paket A eingeordnet. Dieser Befund geht in den vorgeschriebenen Merge-Gate.

## Community-Lint der Abhängigkeiten

`cargo clippy -p dl-community --lib -- -D warnings` scheitert vor der Prüfung von `dl-community` in unveränderten `dl-discord`-Dateien. Die aktuelle Toolchain meldet `clippy::result_large_err` für bestehende `serenity::Error`-Rückgaben, unter anderem `adapter.rs:115`, `adapter.rs:310`, `dispatch.rs:731` und `gateway.rs:812`. Diese Dateien gehören nicht zu Paket A. Der gezielte Community-Lint mit `--no-deps -- -D warnings` erreicht `dl-community` und meldet dort zwei weitere bestehende `result_large_err`-Funde in der unveränderten Datei `team_applications.rs:957` und `team_applications.rs:1135`. Es werden keine Lints abgeschaltet. Die neuen Pool-Dateien bestehen den strengen Lint; keine Warnung wurde in den geänderten Privacy-Abschnitten gemeldet.

## Steam-Provenienz

Vor dem Gate wurde der Ingest-Leser mit dem Snapshot-Schreiber abgeglichen: ein Snapshot darf nur das derzeit ausgewählte bestätigte Steam-Konto verwenden. Mitspieler-Paare erfordern weiterhin bestätigte Konten beider Profile. Die bestehenden Tests prüfen zusätzlich ein abweichendes zweites Konto und einen verspäteten Paar-Write nach Unlink.

## Noch offen

Der vorgeschriebene Gate prüfte den gepushten Code-Stand `f6454e7a` und meldete BLOCK wegen fehlender Steam-Herkunft bei verspäteten Mitspieler-Ergebnissen nach einem Kontowechsel. Der genaue Befund und die Übergabe an einen frischen Fixer stehen in `REVIEW-A.md`. Nach BLOCK wurden keine eigenen Codekorrekturen vorgenommen.

Merge, Migration als postgres, Deploy, Dienstneustart und Live-Beweis stehen aus. Interview, echte API-Daten, Website und Discord-Sessionkanal sind in Paket A noch nicht live prüfbar. Paket A hat keinen Release-Build gestartet und keine zentrale Produktionsmigration angewendet. Der fremde Build mit PID `3597035` ist beendet. Der Repo-Lock bei PID `3643980` bleibt unangetastet. Eigene Locks wurden nicht erworben, der reservierte Release-Slot wurde nicht genutzt.
