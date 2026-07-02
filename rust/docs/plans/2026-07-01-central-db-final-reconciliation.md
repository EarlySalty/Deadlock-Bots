# Central DB Final Reconciliation + Cutover Plan

Datum: 2026-07-01
Status: Plan, keine Implementierung
Scope: Finaler Delta-Abgleich und Cutover der verbliebenen SQLite-Writer nach `central-postgres-sp1`.

## Kontext

`dl-bot`/`dl-web` laufen seit **2026-07-01 03:29:38 +0200** live gegen die zentrale Postgres-Instanz. Die zentrale DB wurde vorher per einmaligem, ledger-getriebenem ETL-Snapshot aus den SQLite-Quellen befuellt. Der alte Bulk-ETL ist **nicht** erneut sicher ausfuehrbar, weil sein Load-Modell `INSERT ... ON CONFLICT DO UPDATE SET ziel = EXCLUDED` die Quelle gewinnen laesst und damit inzwischen frischere Postgres-Writes ueberschreiben koennte.

Noch aktive SQLite-Writer:

| Dienst | Repo | Persistenz | Relevante Zielbereiche |
|---|---|---|---|
| `steam-bot.service` / `steam-core.service` | `/home/naniadm/Documents/Deadlock-Steam-Bot` | `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` via `DEADLOCK_DB_PATH` | `core.steam_links`, `steam.*`, Steam-nahe `tierlist`/Presence-/Task-Daten |
| `deadlock-patchnotes.service` | `/home/naniadm/Documents/Deadlock--Patchnotes-Bot` | dieselbe `deadlock.sqlite3` | `patchnotes.changelog_posts`, `patchnotes.deadlock_changelogs`, `bot.kv_store` mit `ns='patchnotes_bot'` |
| `deadlock-website-backend.service` | `/home/naniadm/Documents/Website` | `/home/naniadm/Documents/Website/builds/backend/deadlock.db` | `coaching.*`, Website-`meta_*` nach Ledger |

## Cutoff-Fakten

Die lokale `central-postgres-sp1`-Historie ist in `main` gemerged. Relevante Commits:

| Marke | Commit | Zeitpunkt |
|---|---|---|
| Letzter P2-ETL-Implementierungsstand | `3da3a01` (`feat(sp1): P2 abgeschlossen`) | 2026-06-30 12:27:05 +0200 |
| P3-Barriere, Consumer PG-only | `99c556f` | 2026-07-01 01:58:48 +0200 |
| Live-Sync-Runner ergaenzt | `641d469` | 2026-07-01 03:03:38 +0200 |
| Merge nach `main` | `e7d2ab8` | 2026-07-01 03:23:04 +0200 |
| Bekannter Live-Start `dl-bot`/`dl-web` | Auftrag | 2026-07-01 03:29:38 +0200 |

Der **operative Delta-Cutoff** muss der tatsaechliche Zeitpunkt des Live-ETL-Snapshots sein: Snapshot-Verzeichnis/Manifest/Log von `dl-central-sync` bzw. dem P4-Lauf. Falls dieser Zeitpunkt nicht eindeutig rekonstruierbar ist, wird konservativ `2026-06-30 12:27:05 +0200` als Kandidaten-Cutoff verwendet. Das erzeugt mehr False-Positives, aber keine False-Negatives; False-Positives werden durch Hash-/Target-Vergleich als `noop` oder Konflikt klassifiziert.

## Harte Regeln

- Keine DSN ausgeben, loggen oder in Dateien schreiben. Secret nur ueber Infisical/Wrapper.
- Kein Re-Run des alten Bulk-ETL gegen Prod.
- Kein `DELETE` aus alten SQLite-Dateien; alte Dateien plus WAL/SHM bleiben als Snapshot/Rollback-Artefakte erhalten.
- Kein automatisches "SQLite gewinnt" bei bestehender Postgres-Zeile.
- Jede Delta-Aktion erzeugt einen auditierbaren Bericht: Quelle, Ziel-PK, Quell-Hash, Ziel-vorher-Hash, Aktion, Entscheidung.
- Tabellen ohne verlässlichen Zeitstempel werden nicht per Zeitfilter allein reconciled.

## Delta-Erkennung

### T0 - Baseline sichern

1. Exakten Live-ETL-Snapshot finden:
   - bevorzugt Snapshot-Verzeichnis unter `data/central-etl-snapshots` oder Runner-Log mit Snapshot-Pfad und Dateizeit.
   - Snapshot-Dateien mit SHA256 und mtime ins Reconciliation-Manifest aufnehmen.
   - falls kein Snapshot existiert: `C_etl = 2026-06-30 12:27:05 +0200` als konservativen Kandidaten-Cutoff markieren.
2. Scope aus Ledger ableiten, nicht manuell raten:
   - `rust/crates/dl-central-etl/ledger/deadlock-sqlite3/{core,steam,bot,patchnotes,tierlist,activity}.toml`
   - `rust/crates/dl-central-etl/ledger/website/{coaching,tierlist,content,patchnotes,core}.toml`
3. Fuer jede Tabelle Klassifikation speichern:
   - `updated_at`/aehnliche Uhr vorhanden: Zeitfilter plus Hashvergleich.
   - append-only: PK/ID/captured_at/posted_at plus Hashvergleich.
   - keine Uhr: vollstaendiger PK-/Hashvergleich gegen Original-Snapshot oder Target.
   - Queue/State-Tabelle: separate Konfliktregel, keine pauschale Quelle-gewinnt-Regel.

### T1 - Kandidaten bilden

Fuer jeden Source-Snapshot nach `C_etl`:

- Neue Zeile: Source-PK existiert nicht im Original-Snapshot oder nicht im Target.
- Geaenderte Zeile: Source-PK existiert, aber kanonischer Hash ueber alle gemappten Zielspalten unterscheidet sich vom Original-Snapshot.
- Unsichere Zeile: kein Original-Snapshot und kein belastbarer Zeitstempel; nur Target-Diff vorhanden.

Hash-Regel: Werte zuerst mit der bestehenden Ledger-/ETL-Konvertierung in Zieltypen normalisieren, dann hashen. SQLite-Rohstrings duerfen nicht direkt mit PG-JSON/Timestamps verglichen werden.

Zeitfilter sind nur Vorfilter. Eine Zeile mit `updated_at >= C_etl` wird Kandidat, aber erst der Hash-/Target-Vergleich entscheidet `noop`, `apply` oder `conflict`.

### T2 - Safe-Merge-Regeln

Pro Kandidat wird die Zielzeile gelesen und mit drei Ständen verglichen: Original-Snapshot, aktuelle SQLite-Quelle, aktuelle Postgres-Zeile.

| Zustand | Aktion |
|---|---|
| Target fehlt, Source-PK ist neu | Insert erlaubt |
| Target entspricht Original-Snapshot, Source ist geaendert | Update erlaubt |
| Target entspricht Source | `noop` |
| Target unterscheidet sich von Original und Source unterscheidet sich ebenfalls | Konflikt, kein automatisches Update |
| Target hat eindeutig neueren PG-Zeitstempel als Source | PG gewinnt vorlaeufig, Konflikt/Skip reporten |
| Source hat neueren Zeitstempel, Target blieb seit Snapshot unveraendert | Update erlaubt |
| Kein Original-Snapshot, keine Uhr, Target-Diff | Konflikt oder manuelle Entscheidung, kein Auto-Apply |

Fuer Tabellen ohne echten PK im Ziel ist `replace_rows_in_tx` aus dem alten ETL verboten. Diese Tabellen brauchen eine explizite Tabellenpolicy; sonst bleiben sie auf `manual`.

## Tabellen-Policies

### Steam / Core

- `core.steam_links`: hoher Konfliktrang. PK `(discord_id, steam_id)`, Owner-Guard und User-Guard muessen aktiv bleiben. Automatisch erlaubt sind nur Inserts oder Updates, wenn PG noch dem Original-Snapshot entspricht. Konflikte bei `verified`, `primary_account`, `steam_display_name`, `updated_at`, `deadlock_rank*`, `deadlock_rank_updated_at`, `is_steam_friend` werden nicht geloest.
- `core.users`: hoher Konfliktrang. `dl-central-db::upsert_user` aktualisiert `last_seen = now()` und Profilfelder per COALESCE. Reconciliation darf `last_seen`, `username`, `global_name`, `avatar`, `raw` nicht aus altem SQLite zuruecksetzen.
- `steam.steam_tasks`: hoher Konfliktrang. Zwei Queue-Welten koennen existieren: PG-Producer aus `dl-bot` und SQLite-Producer/-Consumer aus Steam. IDs koennen kollidieren. Offene Entscheidung: alte SQLite-Tasks mit Original-ID uebernehmen, mit neuer PG-ID replayen oder nur nicht-terminale Tasks manuell mergen.
- `steam_rank_history`: eher append-only. Insert-on-conflict/noop nach `(user_id, captured_at)`-Semantik; bei gleicher Identitaet und anderer Payload Konflikt.
- `steam_rank_assignments`, `steam_cleanup_poll_state`, `steam_friendship_miss_tracker`, `steam_beta_invites`, `steam_role_cleanup_pending`: State-Tabellen, nur source-gewinnt wenn PG unveraendert seit Snapshot.
- Presence-/Party-State (`activity.live_player_state`, `voice.deadlock_party_members`, `voice.deadlock_voice_watch`, soweit vom Steam-Core geschrieben): letzte SQLite-Version kann fachlich die aktuellste sein, aber nur anwenden, wenn PG nicht parallel aktualisiert wurde.

### Patchnotes

- `patchnotes.changelog_posts`: kein `updated_at` im Ziel. Kandidaten ueber `id`, `url`, `posted_at` und Hash ueber `title/url/posted_at/raw_content/translated_content`. Gleiche `url` mit unterschiedlichem `id` ist Konflikt.
- `patchnotes.deadlock_changelogs`: analog ueber `id`, `url`, `posted_at`, `content`.
- `bot.kv_store` mit `ns='patchnotes_bot'`: keine Uhr. Nur ueber Original-Snapshot-Hash sicher. Ohne Original-Snapshot ist jeder Unterschied zwischen PG und SQLite ein manueller Konflikt.

### Website Backend

- `coaching.requests`: hoher Konfliktrang wegen Union aus Bot- und Website-Quelle. Keys: `request_uid`, `bot_request_id`, `website_request_id`. Felder `status`, `assigned_coach_id`, `assigned_coach_username`, `reserved_until`, `notify_discord_at`, `message_id`, `role_*`, `ai_summary`, `ai_insights_json` koennen von unterschiedlichen Seiten geaendert werden. Automatisch nur, wenn Target noch Snapshot entspricht.
- `coaching.sessions`, `coaching.coaches`, `coaching.coach_applications`, `coaching.coachees`, `coaching.appointments`, `coaching.goals`, `coaching.milestones`, `coaching.session_notes`: `updated_at` nutzen, wo vorhanden; sonst Hashvergleich. Bei Coach-/Request-Statusfeldern Konflikt statt Auto-Overwrite.
- Website-`meta_*` in `tierlist`, `content`, `patchnotes`, `core.meta_users`: je nach Router teils aktiv. Ohne eindeutigen Owner keine automatischen Updates auf bestehende PG-Zeilen.

## Cutover-Reihenfolge

### Vorbereitung ohne Wartungsfenster

1. PG-faehige Builds/Configs fuer Steam-Core, Steam-Bot, Patchnotes und Website-Backend fertigstellen und gegen Wegwerf-DB testen.
2. Delta-Reconciliation im Dry-Run auf konsistenten Snapshots laufen lassen, keine Prod-Writes.
3. Konfliktbericht reviewen. Alle High-Risk-Konflikte muessen vor dem Wartungsfenster eine Entscheidung haben.
4. Rollback-Artefakte vorbereiten: alte Service-Units/Env, alte Binaries, SQLite-Dateipfade, PG-Backup-Runbook.

### Wartungsfenster

1. Externe Writer stoppen:
   - `deadlock-website-backend.service`
   - `deadlock-patchnotes.service`
   - `steam-bot.service`
   - `steam-core.service`
2. Fuer das letzte Delta auch PG-Writer mit geteilten Tabellen pausieren oder in Maintenance setzen. Sicherste Variante: `deadlock-bot-rust.service` und `deadlock-web-rust.service` kurz stoppen, weil sie `core`, `coaching`, `bot.kv_store`, `steam.steam_tasks` und angrenzende Tabellen beruehren koennen.
3. Pruefen, dass keine Prozesse mehr `deadlock.sqlite3`, `deadlock.sqlite3-wal`, `deadlock.sqlite3-shm` oder `Website/builds/backend/deadlock.db` offen halten.
4. Finale SQLite-Snapshots erstellen und mit SHA256/mtime manifestieren.
5. Finale Delta-Reconciliation zuerst als Dry-Run, dann Apply:
   - Apply in kleinen, schema-/domainweisen Transaktionen.
   - Konflikt = Abbruch fuer die betroffene Domain, keine stille Fortsetzung.
   - Audit-Report mit `applied`, `noop`, `conflict`, `skipped`.
6. Verifikation:
   - Kandidatenbilanz: `applied + noop + conflict + skipped == candidates`.
   - Keine offenen High-Risk-Konflikte fuer freizugebende Dienste.
   - Stichproben-Round-Trip je Domain.
   - `core.steam_links` Trigger/Owner-Guard aktiv.
7. Neue Dienste starten:
   - `steam-core.service` mit zentraler Postgres-Konfiguration, Health/API abwarten.
   - `steam-bot.service` danach, weil er Steam-Core nutzt.
   - `deadlock-patchnotes.service` mit zentraler DB.
   - `deadlock-website-backend.service` mit zentraler DB.
   - danach `deadlock-bot-rust.service` / `deadlock-web-rust.service` wieder starten, falls pausiert.
8. Smoke-Checks:
   - Steam-Link-/Rank-Leseweg, Steam-Core-Heartbeat, Task-Queue.
   - Patchnotes: neuester Post lesbar, KV-State vorhanden.
   - Website-Coaching: Request-Liste, neue Anfrage blockiert bei No-Show-Ban, Coach-Profil.
   - Dashboard/API liest zentrale Tabellen.

## Rollback

Rollback ist nur belastbar, solange alte SQLite-Snapshots unveraendert erhalten bleiben.

| Zeitpunkt | Vorgehen |
|---|---|
| Vor Delta-Apply | Neue Dienste nicht starten, alte Services wieder starten. Keine Datenbewegung passiert. |
| Nach Delta-Apply, vor neuen Writes | PG aus Pre-Apply-Backup oder Audit-Inverse zuruecksetzen, alte Services mit unveraenderter SQLite starten. |
| Nach neuen Writes in PG | Nicht blind zurueckrollen. Erst PG-Writes seit Cutover als Reverse-Delta nach SQLite replayen oder fachlich akzeptierten Datenverlust dokumentieren. |

Alte SQLite-Dateien bleiben mindestens als read-only Archiv:

- `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` plus WAL/SHM bzw. finaler Backup-Snapshot.
- `/home/naniadm/Documents/Website/builds/backend/deadlock.db`.

Services werden fuer Rollback nicht geloescht, sondern nur wieder auf alte Unit/Env/Binary-Kombination gestellt. Kein Schema-Drop in Postgres.

## Offene Entscheidungen

1. **Owner-Policy fuer `core.steam_links`:** Welche Seite gewinnt pro Spalte? Besonders `verified`, `primary_account`, `is_steam_friend`, `deadlock_rank*`, `deadlock_rank_updated_at`, `updated_at`.
2. **`core.users` Konfliktpolicy:** Darf irgendeine SQLite-Quelle Profilfelder oder `last_seen` ueberschreiben? Vorschlag: PG gewinnt immer fuer `last_seen` und vorhandene Profilwerte.
3. **Steam-Queue-Replay:** Wie werden `steam.steam_tasks` aus alter SQLite und neuer PG-Welt zusammengefuehrt, wenn IDs oder Status auseinanderlaufen?
4. **Coaching-Union:** Bei `coaching.requests` muss entschieden werden, ob Website-Felder, Bot-Felder oder neuere `updated_at` je Feld gewinnen. Pauschales Row-Level-Overwrite ist zu grob.
5. **Patchnotes-KV:** Ohne Original-Snapshot ist `bot.kv_store(ns='patchnotes_bot')` nicht automatisch konfliktfrei. Entscheidung: manuell je Key oder PG/SQLite als Owner festlegen.
6. **Website-`meta_*` Owner:** Einige Website-Meta-Tabellen koennen auch von Rust-Dashboard/Tierlist gelesen oder geschrieben werden. Owner pro Tabelle klaeren.

## Definition of Done

- Exakter oder konservativ sicherer `C_etl` im Reconciliation-Manifest dokumentiert.
- Dry-Run-Report fuer alle drei verbleibenden Dienste vorhanden.
- Keine High-Risk-Konflikte ohne explizite Entscheidung.
- Finaler Delta-Apply schreibt keine Zeile, deren Target seit Snapshot unabhaengig geaendert wurde.
- Alle drei Rest-Dienste laufen nach dem Wartungsfenster gegen die zentrale DB.
- Alte SQLite-Dateien und Pre-Apply-PG-Backup sind vorhanden und nicht geloescht.
