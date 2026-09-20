# Verbraucher und Quellen des Discord-Bot/Web-Pakets

Basis fb357052. Betrachtet werden produktive Aufrufer von dl-bot und dl-web sowie deren Crates. Die API-Namen `from_env` in injizierbaren Konstruktoren bleiben zur Kompatibilität bestehen; Bot/Web übergeben ausschließlich `dl_core::runtime_config::lookup`. Dieser Lookup hat eine feste Secret-Allowlist und typisierte TOML-Sektionen, keinen Betriebs-ENV-Fallback. Die exakte Zuordnung aller Felder und bisherigen Aliasnamen steht als explizite Match-Projektion in runtime_config.rs.

| TOML-Bereich | Tatsächlicher Verbraucher | Bisherige Defaults / Grenzen |
|---|---|---|
| services, storage, features.gateway, discord.guild_id | dl-core::Config, Bot/Web-Listener und Gateway | Bestehende Port-/Snapshotdefaults; produktive Identitäten müssen separat belegt werden. Keine Datenverschiebung. |
| runtime.start | dl-bot/main.rs, master.rs, mcp.rs; dl-broker::BrokerConfig | Owner, Befehlssynchronisierung, Presence, PID, MCP und Broker-Allowlisten/Fristen. Fehlende Optionswerte behalten Konstruktor-Defaults. Logging wird erst nach geprüfter Config initialisiert, ohne RUST_LOG-ENV. |
| runtime.bridges | SteamBotClient, TwitchApiClient, MatcherConfig, TurnierProposalService, WebsiteClient | Alle bisherigen operativen Konstruktorparameter, Matcher-Providerwahl eingeschlossen. Typisierte IP und tatsächlicher Twitch-Konstruktor gemeinsam geprüft (IPv4/IPv6). |
| runtime.dashboard, runtime.web | DashboardConfig::from_lookup, WebConfig::from_lookup, Webmain, brain.rs, insights_sync.rs | Öffentliche OAuth-Client-ID ist Config, Client-Secret Infisical. URLs/CORS/Auth-IDs/Cookies/Fristen/Dateipfade. Keine dynamischen DB-Einstellungen verlagert. |
| runtime.community | Botmain; ConciergeConfig, SurveyPulseConfig, LFG-Freitext, PlayerFinder, VoiceHint; dl-server-as-code::DesiredModelOptions | Einzelne bestehende Schalter, IDs, Listen und Fristen. LFG-Foren-Schalter im Regelableiter verwendet denselben Snapshot wie Botmain. Keine neuen Sammel-Abschalter. |
| runtime.community.recording_* | Botmain, RcloneArchive | Fehlender State-Pfad behält OS-XDG/HOME-Vertrag, anschließend wie zuvor deadlock-bots/scrim-recordings. Expliziter Pfad wird relativ zur Config aufgelöst. Rclone/Archiv behalten bei fehlendem Wert ihre vorherigen Defaults. Kein Kopieren/Umziehen bestehender Aufnahmen. |
| moderation.enforce, runtime.moderation | Botmain, moderation_channel, ActionPolicyConfig, BehaviorDetectorGlue | Exakter Maßnahmen-Schalter, Kanäle/Scanliste, Einladungs-Ausnahmen, Konfidenzen und Timeoutfristen. Kanalbezogene Fachsettings bleiben in DB. |
| concierge.timeout_seconds | Botmain → ConciergeConfig.ai_timeout | Bestehender tatsächlich genutzter Gesamtzeitrahmen. Keine Zusammenlegung separater Knowledge-Zeitlimits. |
| runtime.ai | Botmain, Dashboard::build_scrim_lagebild_ai, TransparencyConfig, BrainHandler | Vorhandene Modell-Pins/Provider-URLs, Transparenz und Brain-Betriebswerte. Secretwerte bleiben ausschließlich Infisical. |
| llm.default_provider, llm.use_cases, llm.fireworks.model | bestehende dl-ai::LlmProviderConfig und deren Providerfabrik | Explizite alte Overrides; ohne Override unveränderte Verbraucherdefaults: ModerationVerify/TurnierVorschlag/VoiceHint OpenAI, übrige Fireworks. Kein neuer Modellname, kein Katalogzugriff, keine automatische Auswahl. |

## Entfernte Behauptungen aus dem ursprünglichen TOML-WIP

Nicht angebundene features.onboarding/lfg/voice/concierge, generische discord.channels/roles und knowledge.ask_url/retrieval wurden entfernt. Konkrete bestehende Modulschalter/IDs stehen in den typisierten runtime-Sektionen. Der ausschließlich im WIP vorhandene latest_stable-Resolver mitsamt Katalogkandidaten, Modellfamilien-/Refresh-Optionen und nur dafür vorhandenen Tests ist entfernt. Keine ausschließlich dafür benötigte externe Abhängigkeit war vorhanden.

Knowledge bleibt am vorhandenen tatsächlichen Vertrag: Concierge-Unterpfad 8 Sekunden, gemeinsame AnswerEngine-Retrieval 20 Sekunden, Basisziel http://127.0.0.1:8896; die Consumer bauen ihren Retrieve-Pfad selbst. Die unverdrahteten WIP-Werte (Ask-Pfad, 7 Sekunden, Retrieval-Modus) wurden nie als Livewerte übernommen. Kein neues Knowledge-Verhalten.

## Restscan / getrennte Aufgaben

Bot/Web direkte ENV-Leser: OS-HOME/XDG für den bestehenden Aufnahmestandard, zentrale explizite Secret-Allowlist, neuer Steamproxy mit bestehendem Secretvertrag. Zentrale DB-DSN bleibt Infisical. `EnvFilter::try_from_default_env` ist im gemeinsam verwendeten init_tracing entfernt. Compile-time CARGO_MANIFEST_DIR/CARGO_PKG_VERSION sind Build-Metadaten. Test-ENV und isolierte Test-DB-Auswahl sind keine Betriebsquelle.

Eigenständige aktive Dienste/Timer aus demselben Repo: dl-knowledge, dl-brain-feeder, dl-verbinder einschließlich summary/auswertung, dl-insights-sync, dl-repostats und deadlock-twitch-invite-sync. Deren Einstiegspunkte/Wrapper werden als separates, gemeinsam zu reviewendes Paket migriert. dl-mcp ist bisher ohne Unitnachweis. Brain-Repo-eigene Timer gehören nicht zu diesem Paket. Infisicalbootstrap wird nicht als neuer Authweg umgebaut.

Noch kein Vollrollout: produktive Betriebsdatei, externe Startwrapper und Dienstübernahme fehlen. Prüfdefaults oder Beispiele dürfen diese Werte nicht ersetzen. Vollständige Abnahme schließt Nebenjobs und tatsächliche Startstrecke ein.
