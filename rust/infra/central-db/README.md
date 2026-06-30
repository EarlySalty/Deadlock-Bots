# Deadlock Central DB

Dedizierte TimescaleDB-Instanz (Postgres 16) für die zentrale Datenschicht aller Deadlock-Bots — die eine geteilte DB, in die Deadlock-Bots, Website-Coaching, Steam, Turniere und Patchnotes zusammenlaufen. Twitch-Bot und TradingBot bleiben bewusst außen vor (eigene DBs).

## Zweck

Löst die SQLite-Fragmentierung strukturell: statt mehrerer loser Dateien mit gleichnamigen, aber unvereinbaren Tabellen läuft alles über eine Instanz mit Postgres-Schemas pro Domäne (`core`, `coaching`, `scrim`, `steam`, `turnier`, `patchnotes`, `activity`). `core` ist die geteilte Spine mit der Discord-User-ID als Join-Key. Loopback-only auf `127.0.0.1:5434` (5433 ist die Twitch-Instanz), kein öffentlicher Port.

## Hochziehen

Die Instanz braucht `DEADLOCK_CENTRAL_DSN` in der Umgebung — das Skript liest selbst keine Secrets. Erst via Infisical laden, dann starten, danach wieder aus der Env entfernen:

```bash
cd /home/naniadm/Documents/Deadlock-Bots
eval "$(/home/naniadm/Documents/Infisical/export_claude_secret.py --secret DEADLOCK_CENTRAL_DSN --no-confirm)"
./rust/infra/central-db/up.sh
unset DEADLOCK_CENTRAL_DSN
```

`up.sh` extrahiert das Passwort aus der DSN (ohne es zu loggen), startet den Container und wartet, bis `pg_isready` „healthy" meldet.

## Stoppen

Stoppt den Container, **ohne** Daten zu löschen (das Volume `deadlock_central_pgdata` bleibt erhalten):

```bash
cd /home/naniadm/Documents/Deadlock-Bots
./rust/infra/central-db/down.sh
```

## systemd-Vorlage

Für den reboot-festen Dauerbetrieb als System-Dienst. Die Unit ist eine Vorlage: vor dem Enable muss `POSTGRES_PASSWORD` über eine `EnvironmentFile` (oder via Infisical geladen) bereitstehen — niemals Klartext in die Unit schreiben. Ohne gesetztes Passwort startet der Container bewusst nicht.

```bash
sudo cp /home/naniadm/Documents/Deadlock-Bots/rust/infra/central-db/deadlock-central-db.service /etc/systemd/system/deadlock-central-db.service
sudo systemctl daemon-reload
sudo systemctl enable deadlock-central-db.service
```

## Backup

Die kompletten Daten liegen im benannten Volume `deadlock_central_pgdata`. Vor riskanten Aktionen den Mount-Pfad sichern (Volume-Snapshot oder `pg_dump`); das Volume wird durch `down.sh` nie angefasst.

```bash
docker volume inspect deadlock_central_pgdata
```

## Verifikation

Nach dem Hochziehen beweisen, dass der Container läuft, gesund ist und noch **keine** Domänen-Schemas trägt (die legt erst der Migrator `dl-central-migrate` an, nicht die Instanz-Initialisierung):

- [ ] `docker compose -f rust/infra/central-db/docker-compose.yml config`
- [ ] `docker ps --filter name=deadlock-central-postgres`
- [ ] `docker exec deadlock-central-postgres pg_isready -U deadlock -d deadlock`
- [ ] `docker exec deadlock-central-postgres psql -U deadlock -d deadlock -c '\dn'`

## Rollback

Reiner Stopp, Daten bleiben im Volume. Ein vollständiges Entfernen samt Daten (`docker compose down -v`) ist tabu — die Quelle wird nie zerstört.

```bash
cd /home/naniadm/Documents/Deadlock-Bots
./rust/infra/central-db/down.sh
```
