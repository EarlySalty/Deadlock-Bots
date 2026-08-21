# Twitch-Spam-Lernbuttons

## Zweck
Verdächtige Twitch-Spam-Alerts im Discord bekommen je nach AI-Urteil genau einen
Gegen-Override: AI `harmlos` → `Als Spam korrigieren`; AI `Spam` →
`Als harmlos korrigieren`.

Das Modell entscheidet und lernt automatisch. Der Discord-Bot entscheidet nichts
selbst, sondern leitet den menschlichen Gegen-Override an die interne Twitch-API
weiter. Spam-Korrekturen schreiben in die bestehende Spam-Lernliste; Harmlos-
Korrekturen übernehmen die vollständige Originalnachricht in die Safe-Liste und
entfernen das falsche Spam-Muster.

## Cross-Repo-Vertrag
Der Twitch-Bot hängt an direkte Alert-Posts ein Feld `spam_learning`:

```json
{
  "spam_learning": {
    "v": 2,
    "verdict": "spam",
    "ai_reason": "Viewer-Bot-Werbung",
    "learned": [{"table": "spam", "id": 123, "pattern": "eballo.com"}],
    "learn_pattern": null,
    "safe_feedback_pattern": "@miracleground9 aha, so sammelt man also viewer kappa"
  }
}
```

Bei AI `safe` führt ein Klick aus:

```text
POST http://127.0.0.1:8776/internal/twitch/v1/spam-learning
Header: X-Internal-Token: <token>
Body: {"verdict":"spam", ...}
```

Bei AI `spam` nutzt der Button mit der gelernten Spam-Row-ID:

```text
POST …/spam-learning/correct
Body: {"table":"spam","id":123}
```

Die API liest `source_message` aus der Spam-Row, schreibt daraus atomar ein
manuelles Safe-Pattern mit `source_channel=discord-correction`, legt zusätzlich
eine `clean`-Zeile in `twitch_spam_review_decisions` an und entfernt die falsche
Spam-Row. Falls keine Spam-Row-ID existiert, wird der vollständige kanonische
Nachrichten-Fallback — nur wenn er in die Discord-custom_id passt — über
`POST …/spam-learning/safe` gespeichert. Bei Überlänge gibt es bewusst keinen
Fallback-Button; ein gekürzter Text wird niemals als Safe gelernt.

Safe-Patterns werden im Rust-Filter vor Score, Moderationsaktion und LLM geladen.
Der Match ist ein exakter kanonisierter Volltext (NFKC/Homoglyphen, Whitespace,
Trim, Kleinschreibung); Teilstrings oder längere Nachrichten matchen nicht. Der
bestehende Hot-Reload aktualisiert die Liste spätestens nach 120 Sekunden.

Token-Auflösung: `TWITCH_INTERNAL_API_TOKEN` → `MASTER_BROKER_TOKEN` →
`MAIN_BOT_INTERNAL_TOKEN`. Tokens werden nie geloggt.

## Live-Pfad
Der Rust-Changelog-Empfaenger liest `spam_learning`, erzeugt die Buttons und haelt
die zugehoerige Payload kurzzeitig im Speicher. Der Rust-Twitch-Bridge-Handler
nimmt den Button-Klick entgegen und sendet das Urteil an die Twitch-API.

Die Python-Implementierung ist Legacy-Paritaet fuer alte Changelog-Pfade; live
entscheidend ist der Rust-Bot.

## Bedienung
- `Als Spam korrigieren`: das Muster wird als Spam gelernt.
- `Als harmlos korrigieren`: Safe-Pattern lernen, Clean-Audit schreiben und falsches
  Spam-Muster entfernen.
- Nur Nutzer mit Moderations-/Adminrechten dürfen klicken.
- Bei Erfolg wird der geklickte Button deaktiviert und der gespeicherte Status angezeigt.

## Tests
- `cargo test -p dl-changelog spam_learning --lib` prüft Payload-Parsing und Button-Aufbau.
- `cargo test -p dl-bridges spam_learning --lib` prüft Button-IDs, Rechte und API-Weitergabe.
- `tests/test_changelog_spam_learning.py` hält den Legacy-Python-Pfad kompatibel.
