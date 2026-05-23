# AI-Onboarding-Pipeline

## Zweck
`AIOnboarding` erzeugt eine personalisierte Starter-Tour fuer neue Server-Mitglieder. Anders als das klassische Regel-/Welcome-Onboarding fragt diese Pipeline drei kurze Freitext-Antworten ab und laesst daraus eine kompakte Empfehlungsliste generieren. Das Modul ist technisch relevant, weil es UI, LLM-Fallbacks, Datenschutz und Persistenz kombiniert.

## Architektur
Die Pipeline wird nicht per eigenem Slash-Command gestartet, sondern von anderen Flows aus aufgerufen, vor allem aus `rules_channel.py`, `welcome_dm/dm_main.py` und im Testfall via `!aiob` aus `AIConnector` (`cogs/rules_channel.py:303`, `cogs/welcome_dm/dm_main.py:304`, `cogs/ai_connector.py:560`).

Ablauf:

1. `start_in_channel()` postet einen persistenten Start-Button `Los geht's` in DM, Thread oder Channel (`cogs/ai_onboarding.py:587`).
2. Button oeffnet `OnboardingQuestionsModal` mit drei Feldern: Interessen, Erwartungen, Schreibstil (`cogs/ai_onboarding.py:310`).
3. `generate_personalized_text()` baut Prompt aus `SERVER_CONTEXT`, den Antworten und einem Rollen-Context-Block (`cogs/ai_onboarding.py:517`).
4. LLM-Aufruf:
   - zuerst Gemini mit `DEADLOCK_GEMINI_MODEL` bzw. Default `gemini-2.0-flash`
   - danach OpenAI mit `DEADLOCK_ONBOARD_MODEL`, Default `gpt-5.2`
   - wenn beides fehlt: lokaler Text-Fallback (`cogs/ai_onboarding.py:549` bis `cogs/ai_onboarding.py:584`)
5. Antwort wird als Embed mit Quick-Action-Buttons ausgeliefert.
6. Optional kann der User direkt "Regeln gelesen" bestaetigen; dann wird die Onboarding-Complete-Rolle gesetzt (`cogs/ai_onboarding.py:257`).

Persistente Start-Views werden beim Cog-Load aus `kv_store` restauriert (`cogs/ai_onboarding.py:437` bis `cogs/ai_onboarding.py:515`).

## Konfiguration
Env-Vars:

- `DEADLOCK_ONBOARD_MODEL`: OpenAI-Modellname, Default `gpt-5.2`
- `DEADLOCK_ONBOARD_TOKENS`: Max Output Tokens, Default `700`
- `DEADLOCK_GEMINI_MODEL`: bevorzugtes Gemini-Modell
- indirekt ueber `AIConnector`: `GOOGLE_API_KEY` oder `GEMINI_API_KEY`, sowie `OPENAI_API_KEY` oder `DEADLOCK_OPENAI_KEY`

Im Code fest verdrahtet:

- `GUILD_ID`
- vier Channel-URLs fuer Quick Actions
- mehrere Rollen-IDs fuer Ping-/Onboarding-Kontext
- `STREAMING_KEYWORDS` fuer Streamer-Erkennung

## Admin-Workflow
1. Fuer einen manuellen Test `!aiob` verwenden; der Bot schickt den Start-Button per DM (`cogs/ai_connector.py:560`).
2. Wenn Onboarding nach Reboot nicht mehr klickbar ist, `kv_store` Namespace `ai_onboarding:persistent_views` und Cog-Load pruefen.
3. Bei Beschwerden ueber unpassende Vorschlaege zuerst `SERVER_CONTEXT`, Rollen-Mapping und Prompt-Felder pruefen, nicht sofort das Modell wechseln.
4. Wenn Datenschutz wichtig ist: Opt-out greift sowohl fuer View-Persistenz als auch Session-Logging.

## Datenmodell
Es gibt keine eigene Tabelle; die Pipeline nutzt `kv_store`.

Namespaces:

- `ai_onboarding:persistent_views`: `message_id -> {user_id, thread_id}` (`cogs/ai_onboarding.py:126`, `cogs/ai_onboarding.py:442`)
- `ai_onboarding:sessions`: `user_id -> {answers, llm, thread_id}` (`cogs/ai_onboarding.py:127`, `cogs/ai_onboarding.py:616`)

Die Session speichert Antworten, Thread-Bezug und LLM-Meta, aber nur wenn der User nicht in `privacy_core` opt-out ist.

## Wartung & Troubleshooting
- Wenn gar keine KI-Antwort kommt: zuerst `AIConnector` und Provider-Keys pruefen; danach auf Fallback-Ausgabe testen.
- Wenn Rollen-Hinweise falsch sind: `ROLE_*`-IDs und `_build_role_context_block()` pruefen (`cogs/ai_onboarding.py:158`).
- Wenn alte Buttons nach Reboot haengen: korrupten `kv_store`-Eintrag entfernen; der Loader bereinigt kaputte Payloads bereits defensiv.
- Die Antwortlaenge ist auf `MAX_OUTPUT_TOKENS` und ein relativ kompaktes Prompt-Format ausgelegt; fuer groessere Tours nicht einfach blind das Token-Limit erhoehen.

## Code-Referenz
- Haupt-Cog: `cogs/ai_onboarding.py:431`
- Modal/Buttons: `cogs/ai_onboarding.py:218`, `cogs/ai_onboarding.py:310`, `cogs/ai_onboarding.py:390`
- Persistenz: `cogs/ai_onboarding.py:442`
- LLM-Pipeline: `cogs/ai_onboarding.py:517`
- Einstieg aus Test-Command: `cogs/ai_connector.py:560`
