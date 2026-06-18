# Scam-Revoke-Button (Conversation-Scam-Guard, Phase C)

## Zweck
Der Twitch-Bot (Rust) betreibt einen LLM-gestützten Conversation-Scam-Guard. Erkennt er
einen Scam, postet er die Aktion (Auto-Bann / Moderationsvorschlag) über den Master-Broker
als Discord-Embed. Dieser Teil macht das Embed rücknehmbar: ein „Rückgängig"-Button nimmt
eine Fehlentscheidung direkt aus Discord zurück (Entbannen + als Fehlalarm markieren, was
das Self-Learning des Guards korrigiert).

Dies ist die Python-Discord-Seite (Phase C) eines Cross-Repo-Features. Die emittierende
Seite (Phase A/B) liegt im Twitch-Bot-Repo (`tb-chat` Notifier + `tb-internal-api`
Revoke-Route).

## Cross-Repo-Vertrag
Der Twitch-Bot hängt an die Scam-Meldung einen `view_spec`:

```json
{ "type": "scam_revoke", "verdict_id": 123, "channel_login": "...",
  "chatter_login": "...", "action_taken": "banned|suggested|timed_out" }
```

Ein Klick führt aus:

```
POST http://127.0.0.1:8776/internal/twitch/v1/scam-guard/revoke
Body:   {"verdictId": <int>}
Header: X-Internal-Token: <token>
```

Server-Antwort 2xx = Erfolg. Der Server entbannt (sofern gebannt) und markiert das Verdict
als `overturned`; Vorschläge werden nur markiert (kein Unban).

## Integrationspunkte (Python)
- `service/master_broker.py::_parse_view_spec` — `scam_revoke` ist in der Typ-Allowlist;
  validiert `verdict_id > 0` sowie nicht-leere `channel_login` / `chatter_login` /
  `action_taken` (Fehler → 400 `bad_request`).
- `service/master_broker.py::_resolve_broker_view` — behandelt `scam_revoke` **inline**
  (wie `link_button`, kein Resolver-Chaining) und gibt `should_register=True` zurück, damit
  der Broker die View via `bot.add_view` als persistent registriert.
- `service/scam_revoke.py` — die View selbst (`ScamRevokeView`), der HTTP-Aufruf
  (`post_revoke`, DI-fähig über `session=`), die Factory (`build_scam_revoke_view`) und alle
  deutschen UI-Texte. Token-Auflösung identisch zu `service/ticket_diagnose.py`
  (`TWITCH_INTERNAL_API_TOKEN` → `MASTER_BROKER_TOKEN` → `MAIN_BOT_INTERNAL_TOKEN`).

## Designentscheidung: inline statt Resolver
`cogs/twitch/live_bridge.py::resolve_master_broker_view_spec` (für `twitch_live_tracking`)
wirft bei Fremdtypen `ValueError` und **kettet nicht** auf einen vorherigen Resolver. Würde
`scam_revoke` über diesen Callback laufen, bräuchte es fragiles Chaining. Deshalb baut der
Broker die View direkt inline.

## Persistenz-Caveat
Die View ist `timeout=None` mit stabiler `custom_id` pro Verdict (`scam-revoke:<id>`) und wird
zur Laufzeit via `bot.add_view` registriert. Es gibt **bewusst keinen** Rehydrate-Mechanismus:
Nach einem Bot-Neustart sind Buttons auf alten Nachrichten inert. Akzeptiert, weil die
dauerhaften Rücknahme-Pfade `!unban` (Twitch-Chat) und das Dashboard (Phase D) sind.

## Tests
- `tests/test_scam_revoke.py` — View-Aufbau, POST-Vertrag (URL/Payload/Header), Klick-Pfad
  (Erfolg deaktiviert + relabelt; Fehlschlag bleibt nutzbar + Hinweis).
- `tests/test_master_broker.py` — Validierung + Registrierung der View über den Send-Handler.
