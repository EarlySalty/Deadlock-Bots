# Cutover-Checkliste Phase 1: Public-Stats (:8768) + Tierlist (:8771)

Status: BEREIT, wartet auf Freigabe des Betreibers. Bis dahin bedient Python beide Ports.

## Verifikations-Stand (10.6.2026)

- Tierlist: 7 Lese-Endpunkte gegen live-Python auf Prod-Daten gedifft → **JSON identisch**;
  Vote inkl. Rate-Limit gegen DB-Kopie verifiziert; Fehlerfälle + Security-Header identisch.
- Stats: 18 Endpunkte gegen live-Python gedifft (ohne `generated_at`) → **JSON identisch**,
  inkl. Heatmap/Timeline/Leaderboards/Fehlerfälle; Index-HTML byte-identisch.
- Me-Endpunkte: funktional gegen DB-Kopie mit Test-Secret verifiziert (echte Session-Cookies
  bleiben gültig: Codec ist byte-kompatibel, bewiesen per Python-Fixture-Test).
- Workspace-Gate grün: fmt, clippy -D warnings, 30+ Tests.

## Flip-Reihenfolge

1. **dl-web-Service anlegen** (`~/.config/systemd/user/dl-web.service`):
   - `WorkingDirectory` = Repo-Root (Deadlock-Bots), `ExecStart` = Release-Binary `rust/target/release/dl-web`
   - Env wie der Bot (Infisical-Wrapper): braucht `PUBLIC_STATS_SESSION_SECRET`/`SESSIONS_ENCRYPTION_KEY`,
     `TWITCH_INTERNAL_API_TOKEN` (+ ggf. `TURNIER_INTERNAL_API_TOKEN`), `PUBLIC_STATS_CALLBACK_URL`
   - vorher `cargo build --release` ausführen
2. **Python-Seite deaktivieren:** `cogs.public_stats_cog` und `cogs.tierlist_public_cog`
   in `cog_blocklist.json` eintragen, Bot neu starten — Ports 8768/8771 werden frei.
3. **dl-web starten** — bindet 8768 + 8771 (Original-Ports, Caddy unverändert).
4. **Smoke:** `/health` (8768), `/api/tierlist` (8771), Website-Login-Flow einmal durchklicken
   (OAuth läuft weiter übers Python-Dashboard 8766 — das bleibt bis Phase 9 an).
5. **Refresh-Kontrolle:** Nach dem Flip schreibt NUR noch Rust Tierlist-Snapshots
   (`DL_TIERLIST_REFRESH` nicht setzen = an). Python-Cog ist geblockt → kein Doppel-Refresh.

## Rollback

dl-web stoppen → Blocklist-Einträge entfernen → Bot neu starten. Keine Datenmigration nötig
(gleiche DB, gleiche Tabellen).

## Bekannte bewusste Abweichungen

- Co-Player-Rang-Heuristik der Stats entfällt (war im Original wirkungslos — Ergebnisse
  wurden immer herausgefiltert; dokumentierter stiller Bug).
- Tierlist-Admin-Auth validiert per interner Dashboard-API statt In-Process-Dict
  (gleiche Sessions, gleicher Cookie, sliding TTL serverseitig).
- JSON-Whitespace: aiohttp sendet `", "`-Separatoren, Rust kompakt — semantisch identisch.
