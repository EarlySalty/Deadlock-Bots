# AI-Moderator

## Zweck
Der AI-Moderator scannt einen dedizierten Chat-Channel automatisiert auf harte Policy-Verstöße und auf wiederholtes Ragebait. Ziel ist nicht "strenger Chat", sondern das gezielte Herausfiltern von NSFW/CSAM/Hass-Spitzen sowie das strukturierte Vorsortieren fuer menschliche Moderation.

## Architektur
Der Cog arbeitet event-getrieben ueber `on_message` (`cogs/ai_moderator.py:430`). Gescannt werden nur konfigurierte Channel-IDs; Bots, Webhooks und User mit `manage_messages` werden uebersprungen (`cogs/ai_moderator.py:431` bis `cogs/ai_moderator.py:444`).

Pipeline:

1. Nachricht und bis zu vier Bild-Anhaenge aufnehmen.
2. Erste Klassifikation via `AIConnector.generate_multimodal(...)` mit JSON-Ausgabe (`cogs/ai_moderator.py:640`, `cogs/ai_connector.py:341`).
3. Bei Unsicherheit oder mittlerer Konfidenz wird Kontext nachgeladen und ein zweiter Lauf gemacht (`cogs/ai_moderator.py:610` bis `cogs/ai_moderator.py:638`).
4. Je nach Verdict:
   - `ok`: nichts tun
   - `delete` plus hohe Konfidenz in harten Kategorien: Auto-Delete + Timeout (`cogs/ai_moderator.py:488` bis `cogs/ai_moderator.py:494`, `cogs/ai_moderator.py:1005`)
   - `delete` oder `propose` oberhalb Schwellwert: Mod-Proposal mit Accept/Deny-Buttons (`cogs/ai_moderator.py:499`, `cogs/ai_moderator.py:967`)
5. Ragebait wird separat gezaehlt. Ab vier Treffern in 120 Minuten eskaliert der Bot zu `persistent_ragebait` und setzt zusaetzlich ein Mod-Tag `ragebaiter` fuer 14 Tage (`cogs/ai_moderator.py:803`, `cogs/ai_moderator.py:838`).

Persistente Review-Buttons werden nach Reboot wieder registriert (`cogs/ai_moderator.py:388` bis `cogs/ai_moderator.py:414`).

## Konfiguration
Basis ist `AI_MODERATOR_CONFIG` in `cogs/ai_moderator.py:18`.

- `SCAN_CHANNEL_IDS`, `MOD_REVIEW_CHANNEL_ID`, `LOG_CHANNEL_ID`
- `AI_PROVIDER`, `AI_MODEL` aktuell default auf `minimax` und `MiniMax-M3`
- `AUTO_DELETE_CONFIDENCE`, `PROPOSE_CONFIDENCE`, `CONTEXT_ESCALATE_BETWEEN`
- `TIMEOUT_MINUTES`
- `RAGEBAIT_WINDOW_MINUTES`, `RAGEBAIT_ESCALATE_THRESHOLD`
- `MAX_IMAGES_PER_CHECK`, `MAX_PROMPT_CHARS`

Provider-Setup kommt aus `AIConnector`:

- MiniMax: `MINIMAX_TOKEN_PLAN_KEY` oder `MINIMAX_API_KEY` bzw. `MINMAX` (`cogs/ai_connector.py:101`)
- OpenAI-Fallback allgemein: `OPENAI_API_KEY` oder `DEADLOCK_OPENAI_KEY` (`cogs/ai_connector.py:56`)
- Gemini wird fuer dieses Modul nicht genutzt, ist aber im Connector verfuegbar.
- DB-Pfad: `DEADLOCK_DB_PATH` oder `DEADLOCK_DB_DIR` (`cogs/ai_moderator.py:195`)

## Admin-Workflow
1. Offene Vorschlaege im Mod-Review-Channel pruefen.
2. `Accept` loescht die Nachricht und setzt den Timeout; `Deny` verlangt eine Begruendung (`cogs/ai_moderator.py:507`, `cogs/ai_moderator.py:568`).
3. Das Aktionslog landet getrennt im Log-Channel inklusive Originalnachricht als Quote (`cogs/ai_moderator.py:1251`).
4. Wenn in einem `ragebaiter_free`-Voice/Text-Kontext nur ein leichter Treffer vorliegt, wird zuerst gewarnt; bei Wiederholung binnen 30 Minuten wird ein Proposal erstellt (`cogs/ai_moderator.py:855`).

## Datenmodell
Tabellen:

- `ai_moderation_cases` fuer alle Auto-Deletes, Proposals und Mod-Entscheidungen (`cogs/ai_moderator.py:80`)
- `ai_moderation_ragebait_hits` fuer das Rolling Window der Ragebait-Hits (`cogs/ai_moderator.py:104`)

Wichtige Felder in `ai_moderation_cases`:

- `case_id`, `guild_id`, `channel_id`, `message_id`, `user_id`
- `original_content`, `attachments_json`
- `ai_category`, `ai_confidence`, `ai_reason`, `ai_raw_json`
- `action`, `mod_id`, `mod_action_at`, `mod_deny_reason`
- `mod_review_message_id`, `log_message_id`
- `escalated_with_context`

## Wartung & Troubleshooting
- Wenn der Bot gar nicht reagiert: Channel-ID-Whitelist und `manage_messages`-Bypass pruefen.
- Wenn Bilder ignoriert werden: Nur echte `image/*`-Attachments werden beruecksichtigt (`cogs/ai_moderator.py:1120`).
- Wenn nach Reboot Buttons tot sind: offene Cases und `mod_review_message_id` pruefen (`cogs/ai_moderator.py:1452`).
- Wenn Timeouts fehlschlagen: Bot braucht `Moderate Members` (`cogs/ai_moderator.py:1367`).
- Wenn False Positives im rauen Chat zu hoch sind: zuerst `PROPOSE_CONFIDENCE` und `CONTEXT_ESCALATE_BETWEEN` justieren, nicht sofort Prompt oder Kategorien umbauen.

## Code-Referenz
- Haupt-Cog: `cogs/ai_moderator.py:361`
- Prompt/Config: `cogs/ai_moderator.py:18`, `cogs/ai_moderator.py:35`
- Event-Flow: `cogs/ai_moderator.py:430`
- Kontext-Eskalation: `cogs/ai_moderator.py:610`
- Ragebait-Handling: `cogs/ai_moderator.py:803`
- Proposal/Auto-Delete: `cogs/ai_moderator.py:967`, `cogs/ai_moderator.py:1005`
- Connector/Provider: `cogs/ai_connector.py:101`, `cogs/ai_connector.py:341`, `cogs/ai_connector.py:532`
