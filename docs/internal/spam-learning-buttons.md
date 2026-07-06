# Twitch-Spam-Lernbuttons

## Zweck
Verdächtige Twitch-Spam-Alerts im Discord bekommen zwei Betreiber-Buttons:
`Spam lernen` und `Harmlos lernen`. Damit kann ein Mod ein gemeldetes Muster direkt als
positives oder negatives Beispiel speichern, ohne in die Datenbank zu gehen.

Der Button-Flow ist bewusst klein: Der Discord-Bot entscheidet nichts selbst, sondern leitet
den bestätigten Fall an die interne Twitch-API weiter. Dort wird in die bestehenden
Spam-/Safe-Lernlisten geschrieben.

## Cross-Repo-Vertrag
Der Twitch-Bot hängt an direkte Alert-Posts ein Feld `spam_learning`:

```json
{
  "pattern": "aha, so sammelt man also viewer kappa",
  "pattern_type": "phrase",
  "source_message": "@MiracleGhost9 aha, so sammelt man also viewer Kappa",
  "source_channel": "miracleghost9",
  "reason": "Score 1: viewer + name"
}
```

Ein Klick führt aus:

```text
POST http://127.0.0.1:8776/internal/twitch/v1/spam-learning
Header: X-Internal-Token: <token>
Body: {"verdict":"spam|safe", ...}
```

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
- `Harmlos lernen`: Muster wird als Safe-Muster gespeichert.
- Nur Nutzer mit Moderations-/Adminrechten dürfen klicken.
- Bei Erfolg werden beide Buttons deaktiviert und auf `Gelernt` gesetzt.

## Tests
- `cargo test -p dl-changelog spam_learning --lib` prüft Payload-Parsing und Button-Aufbau.
- `cargo test -p dl-bridges spam_learning --lib` prüft Button-IDs, Rechte und API-Weitergabe.
- `tests/test_changelog_spam_learning.py` hält den Legacy-Python-Pfad kompatibel.
