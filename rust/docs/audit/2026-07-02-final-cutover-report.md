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
