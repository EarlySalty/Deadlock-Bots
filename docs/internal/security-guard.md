# Security Guard

## Zweck
`SecurityGuard` schuetzt den Server vor Scam-Bursts, Mehrkanal-Spam und verdächtigen Finanz-/Referral-Nachrichten. Das Modul ist bewusst aggressiv gegen frische Accounts, geht bei etablierten Accounts aber vorsichtiger vor, weil dort eher ein gehackter Account als klassischer Wegwerf-Spam vermutet wird.

## Architektur
Der Cog haengt an `on_message` und pflegt pro User ein kurzes Nachrichtenfenster im Speicher. Es gibt drei Erkennungspfade:

- Mehrkanal-Burst fuer junge Accounts: mehrere Nachrichten in mehreren Kanaelen innerhalb eines Zeitfensters (`cogs/security_guard.py:403`, `cogs/security_guard.py:645`).
- Bild-Scam ueber mehrere Kanaele: Anhaenge werden multimodal durch die AI geprueft (`cogs/security_guard.py:464`, `cogs/security_guard.py:577`).
- Keyword plus AI fuer Einzelmessages: Schlagwoerter wie `telegram`, `profit`, `usdt` oder `promo code` triggern einen gezielten Scam-Check (`cogs/security_guard.py:493`, `cogs/security_guard.py:641`).

Die Textpruefung nutzt `AIConnector.generate_text`, die Bildpruefung `AIConnector.generate_multimodal`. Bei echten Treffern werden Nachrichten geloescht, Beweise gespiegelt und je nach Account-Alter Ban oder Timeout angewandt. Fuer nicht bestaetigte Bursts oder etablierte Accounts werden Mod-Proposals mit Buttons erzeugt (`ScamBanView`, `UnbanView`, `AppealView`).

## Konfiguration
Die Runtime-Konfig liegt statisch in `SECURITY_CONFIG` in `cogs/security_guard.py:46`.

- `REVIEW_CHANNEL_ID`: Beweis-/Review-Channel.
- `MOD_CHANNEL_ID`: Mod-Channel fuer Appeals, Unban und Scam-Proposals.
- `PUNISHMENT`: global `ban` oder `timeout`.
- `WINDOW_SECONDS`, `CHANNEL_THRESHOLD`, `MESSAGE_THRESHOLD`: Burst-Erkennung.
- `ACCOUNT_MAX_AGE_HOURS`: Grenze fuer "junger" Account.
- `ESTABLISHED_ACCOUNT_MIN_AGE_HOURS`, `ESTABLISHED_MIN_JOIN_HOURS`: Schwelle fuer den etablierten Sonderpfad.
- `AI_SCAM_PROVIDER`, `AI_SCAM_CONFIDENCE`: Text-AI.
- `AI_IMAGE_PROVIDER`, `AI_IMAGE_CONFIDENCE`: Bild-AI.
- `TIMEOUT_MINUTES`, `VIEW_TIMEOUT_SECONDS`: Timeout- und Button-Lebensdauer.

Env-Vars direkt im Cog gibt es hier nicht; Provider-Setup kommt indirekt aus `AIConnector`.

## Admin-Workflow
1. Mit `!security_diag` die aktiven Schwellen und Zielkanaele pruefen.
2. Bei Auto-Ban im Mod-Channel den Case und die gespiegelt geposteten Beweise pruefen.
3. Wenn es ein False Positive ist, ueber den `Unban`-Button im Mod-Post entsperren.
4. Bei etablierten Accounts landet stattdessen ein "Auto-Timeout"-Post im Mod-Channel. Dort koennen Mods per `Ban` oder `Timeout aufheben` entscheiden.
5. Appeals kommen per Modal rein und werden in den Mod-Channel weitergeleitet.

## Datenmodell
Persistiert wird in `security_guard_incidents` (`cogs/security_guard.py:341`).

Wichtige Felder:

- `case_id`, `guild_id`, `user_id`, `user_tag`
- `action`, `reason`
- `channel_count`, `message_count`, `attachment_count`, `keyword_hit`
- `messages_json` als gekuerzte Beweiszusammenfassung
- `created_at`

Zusätzlich lebt kurzfristiger Zustand nur im Speicher: `_message_history`, `_active_cases`, `_cases`.

## Wartung & Troubleshooting
- Wenn nichts passiert: Bot-Rechte auf `Ban Members`, `Moderate Members` und Message-Delete pruefen (`cogs/security_guard.py:944`, `cogs/security_guard.py:971`, `cogs/security_guard.py:997`).
- Wenn Bild-Checks nie triggern: `AIConnector` und MiniMax-Multimodal-Setup pruefen.
- Wenn es zu viele False Positives gibt: zuerst `AI_*_CONFIDENCE`, dann die Keyword-Liste und zuletzt Burst-Schwellen anpassen.
- Wenn keine Logs ankommen: `REVIEW_CHANNEL_ID` und `MOD_CHANNEL_ID` pruefen (`cogs/security_guard.py:1038`, `cogs/security_guard.py:1052`).
- Attachments werden absichtlich begrenzt und uebergroße Dateien nicht gespiegelt (`cogs/security_guard.py:1014`).

## Code-Referenz
- Haupt-Cog: `cogs/security_guard.py:267`
- Trigger-Logik: `cogs/security_guard.py:403`, `cogs/security_guard.py:645`
- AI-Checks: `cogs/security_guard.py:544`, `cogs/security_guard.py:577`
- Incident-Handling: `cogs/security_guard.py:691`
- Etablierter-Account-Pfad: `cogs/security_guard.py:732`
- Appeals/Unban: `cogs/security_guard.py:1066`, `cogs/security_guard.py:1117`
- Admin-Diagnose: `cogs/security_guard.py:1298`
