# Finaler Cutover-Bericht — 2026-07-02

Wartungsfenster 03:51–04:09 CEST. Reconciliation-Apply-Report:
`rust/docs/_work/sp1/reconciliation/2026-07-02-final-apply-report.json`.

## Ergebnis

- **Turniere**: bereits vor dem Wartungsfenster live auf zentraler Postgres (eigene, nicht geteilte DB — kein Reconciliation-Bedarf, Row-Counts vorab identisch).
- **Patchnotes-Bot**: live auf zentraler Postgres seit 04:08:47 CEST. Verifiziert: keine offene SQLite-Datei, aktive Verbindung zu :5434.
- **Website-Backend**: live auf zentraler Postgres (Rust) seit 04:09:07 CEST. Verifiziert: keine offene SQLite-Datei, aktive Verbindung zu :5434, `GET /api/coaching/coaches` → 200.
- **Steam-Bot / Steam-Core**: **bewusst NICHT auf Postgres umgestellt**, zurückgerollt auf den Pre-SP3-Commit (`af18814`) mit der alten geteilten SQLite. main hat den SP3-Code bereits (Merge bleibt bestehen, das ist reiner Code, sicher), aber der laufende Dienst nutzt bis auf Weiteres das alte Binary. Grund siehe unten.

## Kritischer Fund: `upsert_user` wird nirgends aufgerufen

`core.users` (die zentrale User-Spine-Tabelle) hatte vor diesem Wartungsfenster nur 8 jemals eingefügte Zeilen (alle 8 wieder gelöscht, vermutlich über `dl-community::privacy::delete_user_data`), obwohl `dl-bot`/`dl-web` seit 2026-07-01 mit erheblicher Aktivität liefen (voice_session_log 64k Zeilen, user_co_players 25k, etc.). Grund: `dl-central-db::upsert_user` ist zwar implementiert, wird aber **im gesamten Repository von keinem einzigen Aufrufer verwendet** — kein Discord-Event-Handler ruft sie auf.

Das blieb bisher folgenlos, weil kein anderer Schema-Teil hart gegen `core.users` referenziert — mit einer Ausnahme: `core.steam_links_user_guard()` (Trigger, `BEFORE INSERT/UPDATE OF discord_id`) lehnt jeden Steam-Link für eine `discord_id` ab, die nicht in `core.users` existiert. Das brach das finale Reconciliation-Apply mit `23503 violates user guard`.

**Sofortmaßnahme (nur für diesen Cutover, kein struktureller Fix):** 500 Minimal-Stub-Zeilen (nur `discord_id`, keine anderen Felder) in `core.users` nachgetragen für alle in `steam_links` referenzierten IDs, additiv per `ON CONFLICT DO NOTHING` — kein bestehender Wert überschrieben. Das hat das Apply entsperrt (1389 Zeilen erfolgreich angewendet).

**Deshalb bleibt Steam-Bot auf SQLite:** Der Stub-Backfill löst nur das historische Problem. Für jeden *neuen* Discord-User, der sich ab jetzt zum ersten Mal per Steam verknüpft, würde derselbe Guard-Fehler erneut auftreten, solange `upsert_user` nicht in einen echten Event-Pfad verdrahtet ist (z. B. Gateway-Message/Interaction-Handler in `dl-bot`). Das ist eine Architektur-Entscheidung (wo/wann aufrufen, Rate-Limits), keine, die im Rahmen dieses Wartungsfensters getroffen werden sollte.

**Offene Aufgabe:** `upsert_user` an einen geeigneten Stelle in `dl-bot`s Event-Loop verdrahten, verifizieren, danach Steam-Bot/-Core reguläres Cutover nachholen (Merge liegt bereits auf main, nur Binary-Austausch + `DEADLOCK_CENTRAL_DSN` im systemd-Unit nötig).

## Offene Datenkonflikte (bewusst nicht automatisch aufgelöst)

5 Zeilen mit echten beidseitigen Änderungen seit dem letzten Snapshot, unverändert im aktuellen Postgres-Zustand belassen (kein Auto-Overwrite laut Plan):

- `bot.twitch_streamer_invites`: 2 Zeilen (`suelze_`, `trippymelo`) — Deadlock-Bots-Domäne, nicht Teil dieses Cutover-Ziels.
- `coaching.coaches`: 3 Zeilen. Darunter ein inhaltlich relevanter Unterschied: Coach `dDmk0ckZ8lsjbALk` (discord_username `earlysalty`) steht in der Website-Quelle seit 2026-07-01T12:02 auf `status=inactive`, in Postgres noch auf `status=active` (Stand vor dem dl-bot-Cutover). Braucht manuelle Entscheidung, wer/was hier gilt.

## Reconciliation-Statistik

Kandidaten gesamt: 7361. `applied: 1389`, `noop: 8`, `conflict: 5`, `skipped: 5959` (davon der Großteil `steam.steam_rank_history` mit 4939 Zeilen — bekannte offene Owner-Entscheidung "Steam-Queue-Replay" aus dem Reconciliation-Plan, absichtlich nicht automatisch gemergt).

## Nachtrag — Steam-Bot-Cutover nachgeholt (05:21–05:28 CEST)

`upsert_user` per `CoreUserSync` (rust/crates/dl-discord/src/core_user_sync.rs) in dl-bot's
Gateway-Handler verdrahtet (`interaction_create`/`message`/`guild_member_addition`/
`guild_member_update`), 10-Minuten-Cooldown pro discord_id, DB-Write per `tokio::spawn`
entkoppelt vom Event-Dispatch (ein unabhängiger Kritiker fand zunächst einen blockierenden
`.await` vor dem fachlichen Dispatch — Risiko: Discords 3s-Interaction-ACK-Fenster vs. 10s
DB-Pool-Timeout; behoben und erneut grün verifiziert vor dem Merge nach main, main
`73713ce`). Live bestätigt: `core.users` bekommt seit dem Bot-Neustart organisch neue
Zeilen aus echtem Discord-Traffic (nicht nur dem historischen Stub-Backfill).

Steam-Bot/Steam-Core (Deadlock-Steam-Bot main `6dd650d`, central-postgres-sp3) danach
regulär auf zentrale Postgres umgestellt: Release-Binaries neu gebaut (Cargo-Fingerprint-
Cache hatte den Rollback-Binary-Austausch nicht erkannt — expliziter Rebuild via
`touch`+`cargo build --release` nötig, sonst wäre das alte SQLite-Binary stehen geblieben),
`Environment=DEADLOCK_DB_PATH=…` aus beiden systemd-Units entfernt (neue Binaries lesen
nur noch `DEADLOCK_CENTRAL_DSN`, bereits über die bestehende Infisical-Config verfügbar —
keine Secret-Änderung nötig). Wartungsfenster: Dienste gestoppt, `lsof` bestätigt keine
offene SQLite-Datei mehr, Dienste mit neuen Binaries gestartet.

Live-Beweis: `steam.steam_role_cleanup_pending` (6 durabler Backlog-Einträge aus der Zeit
vor dem Cutover) beim Boot vollständig gedraint (0 danach), `core.steam_links` mit 291
Zeilen in den ersten 5 Minuten aktualisiert (realer `friend_sync`-Lauf gegen die zentrale
DB), Steam-GC-Session aktiv, keine Journal-Fehler.

Offen (kein Blocker, aus unabhängigem Bug-Sweep `rust/docs/audit/2026-07-02-steam-domain-bug-sweep.md`
im Steam-Bot-Repo): 1 Hoch-Befund (parallele Link-Upserts können mehrere Primary-Accounts
pro User erzeugen — fehlende Partial-Unique-Constraint), 4 Mittel-Befunde (Rank-History
nicht Account-spezifisch, Subrank-Tiebreaker im Background-Sync inkonsistent zu
`/checkrank`, Ko-fi-Token-Race bei Doppelklick, Friend-Request-Status hängt an
prozesslokalem Waiter). Keiner davon wurde durch den Cutover verschärft — alle bestehen
bereits seit dem SP3-Merge im Code, unabhängig vom aktiven DB-Backend.

Separat gefunden, nicht behoben (anderes Themenfeld, nicht Teil dieses Cutovers):
`rust/bin/dl-bot/src/modglue.rs:3183` referenziert seit einem fremden Commit vom 2026-06-30
(`wip(enforcement): …`) die Crate `dl_db`, die seit der SP1-Migration nicht mehr im
Workspace existiert. Betrifft nur den Test-Build (`cargo test`/`clippy --all-targets`),
nicht den Release-Build (`cargo build --workspace` bleibt grün).
