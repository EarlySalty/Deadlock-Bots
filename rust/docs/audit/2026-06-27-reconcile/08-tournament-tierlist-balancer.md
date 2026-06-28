# Reconcile 08: Tournament + Tierlist + Team-Balancer

Datum: 2026-06-27  
Scope: `cogs/customgames/turnier.py`, `cogs/customgames/tournament_store.py`, `cogs/deadlock_team_balancer.py`, `cogs/tierlist_public_cog.py`, `cogs/turnier_public_cog.py`, `service/turnier_public.py`, `service/tierlist_public.py` gegen `rust/crates/dl-tournament`, `rust/crates/dl-tierlist`, `rust/bin/dl-bot`, `rust/bin/dl-web`.

## Kurz-Summary

GAPs: **1 high / 2 medium / 6 low**.

Verifiziert erledigt:
- `f69fc8e`: Rust hat `TEAM_MAX_SIZE = 6` (`store.rs:9`, `discord_ui.rs:26`), `tn:*`-IDs tragen `team_size` (`discord_ui.rs:181-188`, `493-505`), Auto-Balance existiert im Store (`store.rs:563-690`) und laeuft gateway-gated alle 300s (`rust/bin/dl-bot/src/main.rs:446-462`).
- `/turnierpanel` existiert in Rust und postet Panel + `turnier_panel_*`-Buttons (`discord_ui.rs:457-474`, `711-724`).
- `!balance start` schneidet bei >12 Spielern nicht mehr still ab, sondern bricht mit Manual-Hinweis ab (`balance_cmd.rs:253-275`).
- Tierlist ist strukturell voll portiert: Cog ist nur Starter (`cogs/tierlist_public_cog.py:17-59`), echter Service ist `service/tierlist_public.py`; Rust bindet die gleichen Routen in `dl-tierlist` und startet sie in `dl-web` (`dl-tierlist/src/lib.rs:60-80`, `rust/bin/dl-web/src/main.rs:26-49`).

Stale/widerlegt aus dem Alt-Audit:
- Der alte `TEAM_MAX_SIZE=5`-Befund ist geschlossen.
- `team_size` der aktiven Periode ist aktuell kein Rust-only Discord-UI-Gap: Python `get_active_period_async()` liefert ebenfalls kein `team_size` (`tournament_store.py:575-590`), Rust `active_period_json()` spiegelt diese Auswahl (`store.rs:798-826`). Rust-Auto-Balance nutzt dagegen absichtlich `active_period()` mit `team_size` (`store.rs:572-581`) als Fix aus `f69fc8e`.

## GAP-Tabelle

| Severity | Typ | Python-Ref | Rust-Ref | User-sichtbare/Korrektheits-Auswirkung | Aufwand |
|---|---|---|---|---|---|
| high | missing-feature | `cogs/customgames/turnier.py:157-225`, `558-676`, `707-882`, `924-973`, `1247-1332` | `rust/crates/dl-tournament/src/discord_ui.rs:341-454`, `697-729`; Backend-Methoden vorhanden aber unverdrahtet in `store.rs:750-975` | Der Admin-Dialog von `/turnier` fehlt: Zeitraum erstellen/beenden, Anmeldungen mit Paging entfernen, Teams erstellen/loeschen und alle Anmeldungen loeschen. Rust bietet nur User-Dashboard und `/turnierpanel`; Turnier-Admins koennen zentrale Verwaltungsaufgaben nach Cutover nicht mehr in Discord erledigen. | L |
| medium | missing-feature | `cogs/deadlock_team_balancer.py:516-548`, `591-634`, `734-755` | `rust/crates/dl-tournament/src/discord_ui.rs:725-728`; kein Treffer fuer `deadlock_tournament_entry_*` | Das alte persistente TeamBalancer-Turnierpanel (`deadlock_tournament_entry_team/solo`) ist nicht portiert. Bestehende alte Panel-Nachrichten reagieren nach Rust-Cutover nicht mehr; ausserdem fehlt der alte manuelle Rang-Auswahl-Flow ohne Steam-Link/Rollen-Gate. | M |
| medium | missing-feature | `cogs/deadlock_team_balancer.py:637-657`, `756-849` | `rust/crates/dl-tournament/src/balance_cmd.rs:194-205`, `651-667` | Prefix-Subcommands `!balance turnierstatus`, `!balance turnierliste`, `!balance austragen` und `!balance turnierpanel` fehlen. Teile sind ueber `/turnier`/Buttons ersetzbar, aber `turnierliste` und die Prefix-Aliase landen in Rust im Fallback "noch nicht verfuegbar". | M |
| low | behavioral-diff | `cogs/deadlock_team_balancer.py:687-701`, `756-799` | `rust/crates/dl-tournament/src/balance_cmd.rs:198-199`, `641-648` | Python nutzt den `discord.Member`-Converter und akzeptiert uebliche Member-Eingaben. Rust `parse_mentions()` akzeptiert nur `<@id>`/`<@!id>`; IDs oder Namen funktionieren fuer `manual`/`status` nicht. | S |
| low | behavioral-diff | `service/turnier_public.py:622-630`, `666-675` | `rust/crates/dl-tournament/src/web.rs:711-718`, `747-753` | Turnier-Web akzeptiert bei Team-Rename/Kick in Python int-aehnliche String-IDs (`"123"`). Rust erwartet fuer `team_id` eine JSON-Zahl; fehlerhafte/lockere Clients bekommen 400 statt Erfolg. | S |
| low | behavioral-diff | `service/turnier_public.py:520-527`, `581-583` | `rust/crates/dl-tournament/src/web.rs:596-605`, `659-666` | Python koerziert `team_name`/`name` per `str(...)`; Rust akzeptiert nur JSON-Strings. Nur relevant fuer fehlerhafte Clients, kann aber aus `123` in Python ein Team `"123"` machen, waehrend Rust "fehlt" meldet. | S |
| low | behavioral-diff | `service/tierlist_public.py:1216-1220` | `rust/crates/dl-tierlist/src/admin.rs:129-137` | Admin-PUT `/api/admin/hero/{id}`: Python speichert Nicht-String-`description` als String, Rust verwirft sie zu leerem String. Frontend sendet regulaer Strings, daher Edge-Case. | S |
| low | behavioral-diff | `service/tierlist_public.py:1455-1460` | `rust/crates/dl-tierlist/src/admin.rs:400-408` | Admin-PUT `/api/admin/settings`: Python koerziert Nicht-String-`description_text`/`descriptionText`, Rust setzt dann leer. Edge-Case fuer kaputte Admin-Clients. | S |
| low | behavioral-diff | `service/tierlist_public.py:387-406`, `1051-1064` | `rust/crates/dl-tierlist/src/votes.rs:55-73` | Vote-Body als gueltiges JSON-Array ergibt in Python `400 invalid_json` ("JSON-Objekt erwartet"), in Rust `400 invalid_vote`, weil nur `vote` fehlt. Status bleibt gleich, Error-Code/Message weichen ab. | S |

## Deliberate / nicht als GAP gemeldet

| Bereich | Python | Rust | Bewertung |
|---|---|---|---|
| `!balance start` >12 Spieler | `cogs/deadlock_team_balancer.py:177-281`, `716-730` oeffnet einen interaktiven Picker | `balance_cmd.rs:253-275` bricht mit Manual-Hinweis ab | Bewusst dokumentierter Kompromiss aus `f69fc8e`: der Prefix-Listener hat keinen Komponenten-Rueckkanal; wichtig ist, dass Rust nicht mehr still Spieler abschneidet. |
| Tierlist Admin-Auth | `service/tierlist_public.py:420-442` nutzt Dashboard-Validator oder direkten `_discord_sessions`-Fallback | `admin.rs:1-3`, `31-39` validiert per interner Dashboard-API | Bewusste Architektur-Abweichung, im Rust-Modul dokumentiert; kein Featureverlust solange Dashboard erreichbar ist. |
| Turnier-Web Rollencheck | Python laeuft im Bot-Prozess und prueft aus dem Gateway-Cache | `web.rs:8-13`, `562-568` laesst durch, wenn Rolle nicht pruefbar ist | Dokumentierter Fallback fuer separaten `dl-web`-Prozess; kein Drop gegen Python-Fallback-Verhalten bei Cache-Miss. |

## Verifikationsnotizen

- `7abda7c` selbst aendert nur `CHANGELOG.md`; Auto-Balance/Panel wurden deshalb am aktuellen Code verifiziert, nicht aus dem Commit-Text abgeleitet.
- `service/turnier_public.py` und `cogs/turnier_public_cog.py` sind in Rust ueber `dl-web`/`dl-tournament::web` abgebildet (`rust/bin/dl-web/src/main.rs:60-75`, `web.rs:325-340`).
- Tierlist low-Gaps sind reine fehlerhafte-Client-Edges; keine regulaere Website-Funktion fehlt.
