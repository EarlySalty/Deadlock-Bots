# Twitch-Spam-Lernbuttons

## Zweck
Verdächtige Twitch-Spam-Alerts im Discord bekommen je nach AI-Urteil einen
Betreiber-Button: Bei einem Spam-Urteil kann der Fall als Spam korrigiert werden;
bei einem Harmlos-Urteil kann menschliches `clean`-Feedback bestätigt werden.

Der Button-Flow ist bewusst klein: Der Discord-Bot entscheidet nichts selbst, sondern leitet
den bestätigten Fall an die interne Twitch-API weiter. Spam-Korrekturen schreiben in die
bestehende Spam-Lernliste; Harmlos-Bestätigungen landen als `clean`-Feedback im Review-Log
und verändern den aktiven Filter nicht.

## Cross-Repo-Vertrag
Der Twitch-Bot hängt an direkte Alert-Posts ein Feld `spam_learning`:

```json
{
  "pattern": "aha, so sammelt man also viewer kappa",
  "pattern_type": "phrase",
  "source_message": "@MiracleGhost9 aha, so sammelt man also viewer Kappa",
  "source_channel": "miracleghost9",
  "reason": "Score 1: viewer + name",
  "safe_feedback_pattern": null
}
```

Ein Klick führt aus:

```text
POST http://127.0.0.1:8776/internal/twitch/v1/spam-learning
Header: X-Internal-Token: <token>
Body: {"verdict":"spam|safe", ...}
```

Bei einem AI-Urteil `safe` nutzt der Button `POST …/spam-learning/safe` mit
`{"pattern":"...","reason":"Manuelle Harmlos-Bestätigung (Discord)"}`.
Das schreibt eine `clean`-Zeile in `twitch_spam_review_decisions`, aber kein
Safe-Pattern in den aktiven Spam-Filter.

Token-Auflösung: `TWITCH_INTERNAL_API_TOKEN` → `MASTER_BROKER_TOKEN` →
`MAIN_BOT_INTERNAL_TOKEN`. Tokens werden nie geloggt.

## Live-Pfad
Der Rust-Changelog-Empfaenger liest `spam_learning`, erzeugt die Buttons und haelt
die zugehoerige Payload kurzzeitig im Speicher. Der Rust-Twitch-Bridge-Handler
nimmt den Button-Klick entgegen und sendet das Urteil an die Twitch-API.

Die Python-Implementierung ist Legacy-Paritaet fuer alte Changelog-Pfade; live
entscheidend ist der Rust-Bot.

## Bedienung
- `Spam lernen`: Muster wird als Spam gespeichert.
- `Als harmlos bestätigen`: menschliches `clean`-Feedback wird im Review-Log gespeichert.
- Nur Nutzer mit Moderations-/Adminrechten dürfen klicken.
- Bei Erfolg wird der geklickte Button deaktiviert und der gespeicherte Status angezeigt.

## Tests
- `cargo test -p dl-changelog spam_learning --lib` prüft Payload-Parsing und Button-Aufbau.
- `cargo test -p dl-bridges spam_learning --lib` prüft Button-IDs, Rechte und API-Weitergabe.
- `tests/test_changelog_spam_learning.py` hält den Legacy-Python-Pfad kompatibel.
