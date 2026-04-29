# AI-Moderator Cog (2026-04-24)

## Ziel
Neues Cog `cogs/ai_moderator.py`, das Nachrichten in Chat-Channel `1289721245281292291` via MiniMax-M2.7 (Text + Bilder) klassifiziert und je nach Konfidenz auto-moderiert oder Moderatoren per Accept/Deny-Buttons im Channel `1315684135175716978` einbindet. Alle Aktionen werden im Log-Channel `1374364800817303632` festgehalten (ohne Buttons, nur Infos + Original-Message). Ragebait wird pro User mit einer 2h-Rolling-Window-Schwelle (4 Hits) zu `persistent_ragebait` eskaliert.

## Plan
`/home/naniadm/.claude/plans/ich-m-chte-f-r-meinen-dapper-hennessy.md`

## Architektur
- Einzelnes File: `cogs/ai_moderator.py` (Config, Cog, Views, Modal, SQL-Schema) – Blaupause: `cogs/security_guard.py`
- `cogs/ai_connector.py` bekommt neue Methode `generate_multimodal(provider, prompt, images, ...)` für MiniMax-Bildinput (Anthropic- und OpenAI-kompatibles Content-Array)
- Persistenz: neue Tabellen `ai_moderation_cases` + `ai_moderation_ragebait_hits` in `data/deadlock.sqlite3`
- Persistente Views (`custom_id` mit Case-ID) für Bot-Restart-Resilienz

## Flow
1. `on_message` – skip Bots/Mods/andere Channels; Cooldown pro User (2s)
2. Stufe 1: MiniMax-Klassifikation mit `verdict/category/confidence/reason/needs_context`
3. Bei mittlerer Konfidenz oder `needs_context` → Stufe 2 mit 25-Messages-Kontext
4. Verdict-Handling: Auto-Delete + 24h-Timeout bei NSFW/Kat. ≥0.90; Mod-Vorschlag bei 0.55–0.89; Ragebait → Counter; sonst nichts
5. Mod-Buttons (`manage_messages`): Accept = 1D-Timeout + Delete + Log; Deny = Modal mit Pflicht-Begründung + Log
6. Log-Channel erhält Embed + forwarded Original bei jeder Aktion

## Status
GPT-Worker 1 (`cogs/ai_connector.py`) und GPT-Worker 2 (`cogs/ai_moderator.py`) haben die Implementierung geliefert. Statische Verifikation fuer beide Files erfolgreich.

## Fortschritt
- GPT-Worker 1 erweitert `cogs/ai_connector.py` um `generate_multimodal(...)` fuer MiniMax inkl. Bild-Content-Arrays fuer Token-Plan und Standard-Endpoint.
- Verifikation fuer `cogs/ai_connector.py` erfolgreich: `py_compile` + Signatur-Check fuer `AIConnector.generate_multimodal`.
- GPT-Worker 2 hat `cogs/ai_moderator.py` komplett neu angelegt: Config, deutsches Moderations-Prompt, `on_message`-Flow, SQLite-Schema, Ragebait-Counter, persistente Accept/Deny-UI, Deny-Modal und Log-Embeds.
- Verifikation fuer `cogs/ai_moderator.py` erfolgreich: `python3 -m py_compile` + `ast.parse(...)`.

## Offen
- Live-Smoke-Test im Zielchannel `1289721245281292291` (passiert beim naechsten echten Chat)

## Erledigt nach Review
- Review durch Claude: DynamicItem-basierte persistente Buttons, saubere DB-Operationen, defensives AI-JSON-Parsing, korrektes `manage_messages`-Gate, Cleanup-Loop aktiv.
- Bot-Restart via `deadlock-services.sh restart bot`. systemd: active, 47/47 Cogs geladen, `cogs.ai_connector` und `cogs.ai_moderator` beide geladen.
- DB-Tabellen `ai_moderation_cases` + `ai_moderation_ragebait_hits` im `data/deadlock.sqlite3` verifiziert.
- Keine Runtime-Errors in journalctl.

---

# Tierlist Public Backend Cog (2026-04-28)

## Ziel
Neuer Backend-Cog `tierlist_public` für die öffentliche Deadlock-Tierliste: dedizierter aiohttp-Service, Snapshot-Refresh aus deadlock-api, Public- und Admin-API auf Basis des bestehenden Dashboard-Discord-OAuth-Cookies.

## Fortschritt
- Spec, bestehende Service-Patterns (`turnier_public`, `public_stats`), Dashboard-Session-Handling und DB-Schema-Setup gesichtet.
- `service/db.py` um `tierlist_*`-Tabellen + Snapshot-Indizes erweitert.
- `service/dashboard.py` exportiert jetzt `validate_discord_session(...)` für Cookie-Wiederverwendung.
- Neuer `service/tierlist_public.py` mit Refresh-Loop, Public-/Admin-API, In-Memory-Vote-Rate-Limit und on-the-fly Tier-Berechnung.
- Neuer Cog `cogs/tierlist_public_cog.py`; Loader-Anpassung war nicht nötig, da `bot_core/cog_loader.py` bereits Auto-Discovery über `setup()` macht.
- Tests `tests/test_tierlist_refresh.py` und `tests/test_tierlist_endpoints.py` ergänzt; lokaler `unittest`-Lauf grün.

## Offen
- Umgebung hat kein `python`-Alias und kein installiertes `pytest`; Verifikation lief daher mit `python3` bzw. `python3 -m unittest`.

---

# LFG Lobby/Lane Vorschlags-Overhaul (2026-04-22)

## Ziel
`cogs/lfg.py` Vorschlagslogik bereinigen: Flow-Trennung Lobby-Suche vs Spielersuche, Offtopic-Filter, Juice-Kammer als Eternus-Preset, Präsenz-basierte Füll-Anzeige, Voll-Hinweis, Staging-Verlinkung, Rank-Warnung.

## Plan
`/home/naniadm/.claude/plans/lfg-py-ich-bin-nicht-delegated-breeze.md`

## Status
Implementierung durch GPT-Worker abgeschlossen. Statische Verifikation (`py_compile`, `ast.parse`) erfolgreich.

## Offen
- Claude-Review.
- Live-Tests (A-H) im Bot-Prozess.
- Commit+Push.

## Entscheidungen (aus Rückfragen)
- **Flow-Trennung**: User im gescannten VC = Spielersuche (nur Ping, keine Lobby-Vorschläge). User nicht im VC = Lobby-Suche (nur Lobbys, keine Mitspielerliste, kein Ping).
- **Offtopic**: Substring `"off topic voice"` (case-insensitive) im Channel-Namen filtern.
- **Juice Kammer** (Channel-ID `1493690350580138114`): fest als Eternus (Rank 11) einstufen.
- **Staging-IDs**: Casual `1330278323145801758`, Street Brawl `1357422958544420944`, Ranked `1412804671432818890`. New Player: Kategorie `1465839366634209361` scannen, ersten Channel mit <6 Leuten (adaptive Channels).
- **Voll-Hinweis**: ab 6 Leuten im VC.
- **Füll-Anzeige**: kombiniert "Deadlock-aktiv / VC-Gesamt" via Steam-Presence.
- **Rank-Warnung**: ab >1.5 Ränge Diff Suffix `⚠️ höher als dein Rang`.
- **Neue-Spieler-Erkennung**: bestehend ok, nicht anfassen.

## Erledigt
- Konstanten für Staging, Juice Kammer, Offtopic-Filter, Voll-Schwelle und Rank-Warnung ergänzt.
- `LaneInfo` um `deadlock_active_count` erweitert; VC-Scan zählt jetzt Deadlock-aktive User via Steam-Presence.
- Offtopic-Channels werden in allen relevanten Kategorie-Scans übersprungen; Juice Kammer wird fest als `Eternus (fix)` gerankt.
- Lobby-Feldtext auf `aktiv im VC` umgestellt, inkl. Voll-Hinweis ab 6 Leuten.
- Neue Helper für Presence-Load und Staging-Auflösung eingebaut; Staging-Hinweise verlinken jetzt mit `<#channel>`.
- Dispatcher trennt jetzt sauber zwischen Spielersuche (nur Mitspieler-Embed + `Deine Lobby`) und Lobby-Suche (nur Lobby-Embed).
- Decision-Log enthält jetzt den Mode `player` oder `lobby`.

---

# Aktivitäts-Tracking: Text-Scoring + Leaderboards + Public-API (2026-04-17)

## Ziel
Server-Grinding fair machen: Text wird konversations-qualität-gescored (nicht Spam-Count), getrennte Leaderboards Voice/Text auf Discord, Public-API + Discord-OAuth damit die Website (dl-activity) Leaderboard + Personal-Dashboard zeigen kann.

## Plan
`/home/naniadm/.claude/plans/wir-haben-ja-aktivit-ts-piped-crown.md`

## Arbeitsteilung
- **Claude (Orchestrator + Frontend):** Website `dl-activity` Vite-Subprojekt
- **GPT-Worker A (Backend-Tracking):** DB-Schema (`text_stats`, `text_conversation_log`), on_message Hybrid-Scoring (10-Min-Sessions, sqrt-Diminishing, Multi-User-Bonus ×1.5, Reply-Bonus), `!tleaderboard` + Footer-Eigenposition in `!vleaderboard`
- **GPT-Worker B (Backend-API):** 8 neue `/api/public/*` Endpoints in `service/public_stats.py` + Discord-OAuth (login/callback/logout, signed Session-Cookie)

## Erledigt
- GPT-Worker A: DB-Schema für Text-Scoring ergänzt (`text_stats`, `text_conversation_log` inkl. Indizes).
- GPT-Worker A: `UserActivityAnalyzer` um Hybrid-Text-Scoring mit 10-Min-Sessionfenstern, Reply-Bonus, Interaktionsbonus und periodischem Flush erweitert.
- GPT-Worker A: Discord-Commands ergänzt/erweitert: `!tleaderboard` neu, `!vleaderboard` Footer mit eigener Position + Embed-Empty-State.
- GPT-Worker B: `service/public_stats.py` um `/api/public/leaderboard/voice`, `/api/public/leaderboard/text`, `/api/public/me`, `/api/public/me/stats`, `/api/public/me/voice-history`, `/api/public/me/text-history`, `/api/public/me/heatmap`, `/api/public/me/co-players` erweitert.
- GPT-Worker B: Discord-OAuth in `service/public_stats.py` ergänzt (`/auth/discord/login`, `/auth/discord/callback`, `/auth/discord/logout`) inkl. signiertem `dl_session`-Cookie, signiertem OAuth-State-Cookie und CORS/Preflight für Dev-Origins.
- GPT-Worker B: Smoke-Verifikation lokal sauber: `python3 -m py_compile service/public_stats.py`, `python3 -c "from service import public_stats"`, `grep -n "def _handle_" service/public_stats.py`.

## Offen
- Discord Developer Portal: Redirect-URI `http://127.0.0.1:8768/auth/discord/callback` (bzw. Prod-URL) als OAuth-Redirect eintragen
- Live E2E im Bot-Prozess (Bot-Neustart erforderlich, bewusst verschoben)

## Entscheidungen
- Session-Signing-Secret: `SESSIONS_ENCRYPTION_KEY` wird wiederverwendet (Fallback im Code eingebaut, `PUBLIC_STATS_SESSION_SECRET` bleibt optional).
- `DISCORD_OAUTH_CLIENT_ID` + `DISCORD_OAUTH_CLIENT_SECRET` existieren bereits in Infisical.
- Redirect-URI: Default `http://127.0.0.1:8768/auth/discord/callback` genügt lokal; Prod-URL via `DISCORD_OAUTH_REDIRECT_URI` setzbar.

---

# Coaching Overhaul (2026-04-16)

## Ziel
Coaching-System Bot+Website stabilisieren: Parsing raus, echte Ränge, kritische Bugs weg, API abgesichert.

## Status
Durchgang 1 abgeschlossen. Änderungen liegen unstaged — noch nicht committed, User review offen.

## Erledigt

### Bot (`Deadlock-Bots/`)
- `cogs/coaching_panel.py`: `_split_rank_input` + `_split_games_hours` entfernt. Modal-Placeholder mit echten Deadlock-Rängen (Archon/Ascendant/Emissary als Beispiele). Rohtext wird jetzt direkt in `rank` / `games_played` gespeichert, `subrank`/`hours_played` bleiben leer.
- `cogs/coaching_request.py`: AI-Prompt und Embed auf Rohdaten umgestellt (kein künstliches `Subrank N/A` mehr). `_get_availability_label` entfernt. CoachClaim-Callback komplett umgebaut: `defer()` + `followup.send` überall (eliminiert Double-Response-Crash). Thread-Create-Fehler werden sauber gemeldet, DB bleibt konsistent. DM-Fail an den User ist kein fataler Fehler mehr — Session + Thread bleiben aktiv, Coach wird informiert. Zusätzlich outer try/except, damit der Button nie stumm crasht.
- `cogs/coaching_survey.py`: `on_voice_state_update` hat jetzt `@commands.Cog.listener()` — Voice-Events werden endlich empfangen, Survey-Trigger funktioniert wieder in Echtzeit.
- `Docs/deadlock-bots/coaching.md` komplett auf den echten Flow umgeschrieben (Panel → Modal → AI → Coach-Claim → Thread, inkl. Rang-Liste).

### Website-Backend (`Website/builds/backend/app/routers/coaching.py`) — via GPT-Worker `36de3803fac3`
- `require_bot_token()` Dependency (Header `X-Bot-Token`, hmac.compare_digest, 503 wenn ENV fehlt, 401 bei falschem Token) an `POST /requests`, `PATCH /requests/{id}/match`, `POST /surveys`.
- Anonymitäts-Leak in Reviews gefixt: stabiles `sha256(user_id+coach_id)[:6]`-Label statt Username-Präfix.
- Neue ENV: `COACHING_BOT_TOKEN`.

## Offen / bewusst verschoben
- `GET /api/coaching/requests` weiterhin public — nicht akut, aber sollte in einem Folge-Pass auch bot-gated werden.
- Sync-Layer Bot↔Website (aktuell getrennte DBs).
- Frontend-Routen `/coaching/apply`, `/coaching/dashboard`, Coaching-anfragen-Button-Logik.
- Discord-Rolle automatisch bei approvter Coach-Application vergeben.
- User-seitiges Cancel bewusst ausgelassen (User-Entscheidung).

## Verifikation
- `python3 -m py_compile` sauber für: `coaching_panel.py`, `coaching_request.py`, `coaching_survey.py`, `coaching_role_manager.py`, `builds/backend/app/routers/coaching.py`.
- Kein Commit / kein Push bisher.

## Nächster Schritt
User reviewt Änderungen. Bei OK: `COACHING_BOT_TOKEN` setzen (Infisical), dann commit+push in beiden Repos.

---

# Tag-System Phase 1 — Welle 1 (2026-04-29)

## Ziel
Nur Foundation aus Phase 1 umsetzen: DB-Schema, neues `cogs/tags/`-Cog mit `TagService`, Cache/Rehydration/Cleanup und dedizierte Tests.

## Fortschritt
- Plan und Spec für Phase 1 gesichtet, Scope auf Welle 1 begrenzt.
- Bestehende Patterns in `service/db.py`, `cogs/tempvoice/` und den vorhandenen Async-Tests abgeglichen.
- TDD durchgezogen: `tests/test_tag_service.py` zuerst angelegt, danach `service/db.py` und `cogs/tags/` bis zum grünen Lauf ergänzt.
- `TagService` implementiert: User-/Mod-Tag-CRUD, In-Memory-Cache, Rehydration via `cog_load()`, 5-Minuten-Cleanup-Loop und `bot.dispatch(...)`-Events.
- Kein separater Loader-Patch nötig: `bot_core/cog_loader.py` nutzt Auto-Discovery; `cogs/tags/__init__.py` stellt dafür ein konsistentes `setup()` bereit.
- Verifikation grün: `pytest tests/test_tag_service.py -v` (in temporärer venv mit `pytest`) und `ruff check service/db.py cogs/tags tests/test_tag_service.py`.

## Offen
- Welle 2 ist bewusst offen: Commands, Onboarding-, TempVoice-, AI-Mod- und LFG-Integration wurden in dieser Welle nicht angefasst.
- Änderungen sind absichtlich uncommitted; Commit/Push bleibt beim Orchestrator.

---

# Tag-System Phase 1 — Welle 2 / TempVoice Tag-Filter (2026-04-29)

## Fortschritt
- `cogs/tempvoice/core.py` um `LaneTagFilter`, DB-Rehydration/Persistenz, `_apply_tag_filter()`, Join-Enforcement und `on_mod_tag_added`-Cleanup ergänzt.
- `cogs/tempvoice/interface.py` um den Button `🛡️ Tag-Filter` sowie eine Config-View mit drei Single-Selects und Save-Flow erweitert.
- Neuer Test `tests/test_tempvoice_tag_filter.py` deckt Persistenz, Min-Age-Block und Ragebaiter-Cleanup ab.
- Verifikation lokal grün: `pytest tests/test_tempvoice_tag_filter.py -v`, `pytest tests/test_tempvoice_core.py tests/test_tempvoice_lane_sorting.py -v`, `ruff check cogs/tempvoice/core.py cogs/tempvoice/interface.py tests/test_tempvoice_tag_filter.py`.

## Offen
- Kein Live-Discord-Smoke-Test in dieser Worker-Phase.
